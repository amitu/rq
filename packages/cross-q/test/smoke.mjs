// Smoke test — proves the built WASM package actually parses through the JS wrapper in
// Node (the `#engine` -> engine-node path). Run: `node test/smoke.mjs`. Exits non-zero on
// any failure so it works as a CI gate without a test framework.

import assert from 'node:assert/strict';
import { parse, supportedFormats, version } from '../src/index.js';

let passed = 0;
const ok = (name) => {
  passed++;
  console.log(`  ✓ ${name}`);
};

// 1. formats + version resolve through the WASM boundary
const formats = supportedFormats();
assert.ok(Array.isArray(formats) && formats.includes('postman') && formats.includes('curl') && formats.includes('bruno'), 'formats');
ok(`supportedFormats() -> ${JSON.stringify(formats)}`);
assert.ok(typeof version() === 'string' && version().length > 0, 'version');
ok(`version() -> ${version()}`);

// 2. curl import
const curl = parse('curl', "curl -H 'Accept: application/json' https://api.example.com/v1/users", 'cmd.txt');
assert.equal(curl.ok, true, 'curl ok');
assert.equal(curl.mapped.requests[0].data.type, 'http', 'curl -> http request');
assert.equal(curl.mapped.requests[0].data.request.method, 'GET', 'curl method');
ok('parse(curl) -> MappedItems with an http request');

// 3. Postman import, incl. the RQ-3458 null/numeric-key case (must coerce, not fail)
const postman = JSON.stringify({
  info: { name: 'Nasty', _postman_id: 'abc', schema: 'https://schema.getpostman.com/json/collection/v2.1.0/collection.json' },
  item: [
    { name: 'folder', item: [
      { name: 'login', request: {
        method: 'POST', url: 'https://api.pay.test/login',
        header: [ { key: null, value: 'x' }, { key: 42, value: 'y' }, { key: 'Content-Type', value: 'application/json' } ],
        auth: { type: 'bearer', bearer: [ { key: 'token', value: '{{T}}' } ] },
      }, event: [ { listen: 'test', script: { exec: ["pm.test('ok', () => pm.response.to.have.status(200));"] } } ] },
    ]},
  ],
});
const res = parse('postman', postman, 'nasty.postman_collection.json');
assert.equal(res.ok, true, 'postman ok');

// tempId/parentId wiring: folder is a collection; login is parented to the folder
const coll = res.mapped.collections.find((c) => c.name === 'folder');
assert.ok(coll, 'folder collection present');
const login = res.mapped.requests.find((r) => r.name === 'login');
assert.ok(login, 'login request present');
assert.equal(login.parentId, coll.tempId, 'login parented to folder tempId');
ok('parse(postman) -> nested folder/request tempId wiring');

// RQ-3458: null key -> "", numeric key -> "42"; import did NOT fail
const headers = login.data.request.headers;
assert.equal(headers[0].key, '', 'null key coerced to ""');
assert.equal(headers[1].key, '42', 'numeric key coerced to "42"');
assert.equal(login.data.auth.type, 'bearer_token', 'bearer auth mapped');
const coerced = (res.report.diagnostics || []).filter((d) => d.severity === 'coerced');
assert.ok(coerced.length >= 2, 'coercions reported');
ok(`RQ-3458 keys coerced + reported (${coerced.length} coercions), import completed`);

// 3b. Postman v1.0.0 (the legacy flat shape) — proves version support through WASM, so
// swapping the engine into the app won't regress v1 collections.
const v1 = JSON.stringify({
  id: 'c', name: 'Legacy', order: ['r1'],
  requests: [{ id: 'r1', name: 'Top', method: 'GET', url: 'https://x.test/top', headers: 'Accept: application/json' }],
});
const rv1 = parse('postman', v1, 'v1.json');
assert.equal(rv1.ok, true, 'v1 ok');
const r1 = rv1.mapped.requests.find((r) => r.name === 'Top');
assert.ok(r1, 'v1 request present');
assert.equal(r1.data.request.headers[0].key, 'Accept', 'v1 header-string parsed');
ok('parse(postman v1.0.0) -> flat requests[] + header-string, via WASM');

// 3c. Bruno .bru (v2) — proves the text-DSL importer runs through WASM
const bru = `meta {
  name: Get user
  type: http
}
get {
  url: {{base}}/users/:id
  auth: bearer
}
auth:bearer {
  token: {{token}}
}
`;
const rbru = parse('bruno', bru, 'get-user.bru');
assert.equal(rbru.ok, true, 'bruno ok');
const bruReq = rbru.mapped.requests.find((r) => r.name === 'Get user');
assert.ok(bruReq, 'bruno request present');
assert.equal(bruReq.data.auth.type, 'bearer_token', 'bruno bearer auth mapped');
ok('parse(bruno .bru) -> request with bearer auth, via WASM');

// 4. unknown format is a soft error, not a throw
const bad = parse('insomnia', '{}', 'x.json');
assert.equal(bad.ok, false, 'unknown format -> ok:false');
assert.match(bad.error, /not_implemented/, 'not_implemented error');
ok('parse(unknown) -> soft { ok:false, error }');

console.log(`\n${passed} checks passed — @requestly/cross-q works in Node via WASM.`);
