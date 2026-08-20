// END-TO-END: a Postman script → transform (Rust→WASM) → execute (QuickJS) → results.
// This is cross-q-context doing the whole job the OSS repo needs: take a foreign-dialect script and
// actually RUN it, returning the variable mutations + captured logs. Run: node test/execute-e2e.mjs
import assert from 'node:assert/strict';

import { transformScript } from '../src/index.js';
import { executeScript } from '../dist/runtime/engine/index.js';

let passed = 0;
const ok = (name) => {
  passed++;
  console.log(`  ✓ ${name}`);
};

// A minimal-but-valid script execution context (empty variable scopes + a bare request).
const context = {
  global: {},
  collectionVariables: {},
  environment: {},
  variables: {},
  iterationData: {},
  secrets: {},
  request: { url: 'https://example.com', method: 'GET', headers: [], queryParams: [], pathVariables: [], body: { contentType: 'none', formUrlEncoded: [], formData: [] }, contentType: 'none' },
  response: null,
  info: { requestId: 'r1', requestName: 'demo', iteration: 0, iterationCount: 1, entryIndex: 0, totalEntries: 1, collectionId: null },
  hostAllowlist: [],
};

// 1. Transform a Postman pre-request script to the rq.* dialect.
const postman = "pm.environment.set('token', 'abc123');\npm.globals.set('count', 2);\nconsole.log('hello from the sandbox');";
const t = transformScript({ source: postman, platform: 'postman' });
assert.equal(t.success, true, 'transform succeeded');
assert.ok(t.code.includes('rq.'), 'pm.* rewritten to rq.*');
ok(`transform: pm.* → rq.* (${t.code.split('\n')[0].slice(0, 48)}…)`);

// 2. Execute the transformed script in QuickJS.
const result = await executeScript({ script: t.code, phase: 'pre-request', context });

// 3. The script's variable writes come back as an inflated, persist-ready MutationDiff.
assert.ok(!result.error, `no execution error (got: ${result.error})`);
assert.ok(result.mutationDiff.environment, 'environment scope was mutated');
assert.equal(result.mutationDiff.environment.token.localValue, 'abc123', 'rq.environment.set persisted the value');
assert.equal(result.mutationDiff.environment.token.type, 'string', 'value type inferred');
ok('rq.environment.set → inflated MutationDiff (token=abc123)');
assert.ok(result.mutationDiff.global && result.mutationDiff.global.count, 'global scope was mutated');
assert.equal(result.mutationDiff.global.count.localValue, '2', 'rq.globals.set persisted');
ok('rq.globals.set → inflated MutationDiff (count=2)');

// 4. console.log was captured live via the console bridge.
assert.ok(result.logs.some((l) => Array.isArray(l.args) && l.args.some((a) => String(a).includes('hello from the sandbox'))), 'console.log captured');
ok('console.log captured through the console bridge');

// 5. Chai-backed rq.test / rq.expect (the require-chain) — a passing and a failing assertion.
const testScript = "pm.test('math works', () => { pm.expect(1 + 1).to.equal(2); });\npm.test('this fails', () => { pm.expect('a').to.equal('b'); });";
const tt = transformScript({ source: testScript, platform: 'postman' });
assert.equal(tt.success, true, 'test-script transform succeeded');
const tr = await executeScript({ script: tt.code, phase: 'post-response', context });
assert.ok(!tr.error, `no execution error (got: ${tr.error})`);
assert.equal(tr.testResults.length, 2, 'two test results');
const byName = Object.fromEntries(tr.testResults.map((t) => [t.name, t]));
assert.equal(byName['math works'].status, 'passed', 'passing assertion → passed');
assert.equal(byName['this fails'].status, 'failed', 'failing assertion → failed');
assert.ok(byName['this fails'].error, 'failed test carries an error message');
ok(`rq.test + rq.expect (chai via require-chain): 1 passed, 1 failed`);

// 6. Delegated fetch (rq.sendRequest) — the host provides the network backend; cross-q-context
//    marshals the request out and the response back, driving the async pump to settlement.
let captured = null;
const sendRequest = async (req) => {
  captured = req;
  return { status: 200, statusText: 'OK', headers: { 'content-type': 'application/json' }, body: '{"pong":true}', bodyEncoding: 'utf8' };
};
const fetchScript = "pm.sendRequest('https://api.example.com/ping', function (err, res) { pm.environment.set('fetchOutcome', err ? 'error' : ('ok:' + res.code)); });";
const ft = transformScript({ source: fetchScript, platform: 'postman' });
const fr = await executeScript({ script: ft.code, phase: 'pre-request', context, sendRequest });
assert.ok(!fr.error, `no execution error (got: ${fr.error})`);
assert.ok(captured, 'host sendRequest was called');
assert.equal(captured.url, 'https://api.example.com/ping', 'request url marshalled to host');
assert.equal(captured.method, 'GET', 'request method marshalled to host');
assert.equal(fr.mutationDiff.environment.fetchOutcome.localValue, 'ok:200', 'response delivered to the script callback');
ok('rq.sendRequest → delegated fetch → async response (status 200)');

// 7. Cookie jar (rq.cookies) — gated to the host allowlist; writes drain as cookie mutations.
const cookieScript = "const jar = rq.cookies.jar(); await jar.set('https://example.com/', 'sid', 'abc123'); rq.environment.set('cookieDone', 'yes');";
const cr = await executeScript({ script: cookieScript, phase: 'pre-request', context: { ...context, hostAllowlist: ['example.com'] } });
assert.ok(!cr.error, `no execution error (got: ${cr.error})`);
assert.ok(cr.cookieMutations && cr.cookieMutations.length > 0, 'cookie mutations drained');
assert.ok(cr.cookieMutations.some((m) => m.kind === 'upsert' && m.cookie.name === 'sid' && m.cookie.value === 'abc123'), 'sid cookie upserted for the allowed host');
assert.equal(cr.mutationDiff.environment.cookieDone.localValue, 'yes', 'script continued past the cookie write');
ok('rq.cookies.jar().set → allowlist-gated, drained as a cookie mutation');

// 8. Bruno's `require('axios')` facade over rq.sendRequest — request mapping (params, method,
//    JSON body), response shape ({data,status,statusText}), and axios's throw-on-non-2xx.
let axiosCaptured = null;
const axiosSend = async (req) => {
  axiosCaptured = req;
  return { status: 200, statusText: 'OK', headers: { 'content-type': 'application/json' }, body: '{"pong":true}', bodyEncoding: 'utf8' };
};
const axiosGet = "const axios = require('axios'); const r = await axios.get('https://api.example.com/ping', { params: { q: 1 } }); bru.setEnvVar('out', r.status + ':' + r.statusText + ':' + r.data.pong);";
const agr = await executeScript({ script: axiosGet, phase: 'pre-request', context, sendRequest: axiosSend });
assert.ok(!agr.error, `no execution error (got: ${agr.error})`);
assert.equal(axiosCaptured.url, 'https://api.example.com/ping?q=1', 'axios params → query string');
assert.equal(axiosCaptured.method, 'GET', 'axios.get → GET');
assert.equal(agr.mutationDiff.environment.out.localValue, '200:OK:true', 'axios response {status, statusText, data(json)}');
ok("require('axios').get → rq.sendRequest: params + JSON response mapped");

const axiosPost = "const axios = require('axios'); await axios.post('https://api.example.com/echo', { name: 'bruno' }); rq.environment.set('m', 'done');";
const apr = await executeScript({ script: axiosPost, phase: 'pre-request', context, sendRequest: axiosSend });
assert.ok(!apr.error, `no execution error (got: ${apr.error})`);
assert.equal(axiosCaptured.method, 'POST', 'axios.post → POST');
assert.ok(String(axiosCaptured.body?.raw ?? axiosCaptured.body ?? '').includes('"name":"bruno"'), 'axios object data → JSON raw body');
ok("require('axios').post → method + JSON body mapped");

const axios404 = async () => ({ status: 404, statusText: 'Not Found', headers: { 'content-type': 'application/json' }, body: '{"error":"nope"}', bodyEncoding: 'utf8' });
const axiosThrow = "const axios = require('axios'); try { await axios.get('https://api.example.com/missing'); bru.setEnvVar('caught', 'no'); } catch (e) { bru.setEnvVar('caught', 'yes:' + e.response.status + ':' + (e.isAxiosError === true) + ':' + e.response.data.error); }";
const atr = await executeScript({ script: axiosThrow, phase: 'pre-request', context, sendRequest: axios404 });
assert.ok(!atr.error, `script handled the rejection (got: ${atr.error})`);
assert.equal(atr.mutationDiff.environment.caught.localValue, 'yes:404:true:nope', 'axios rejects non-2xx with err.response{status,data}');
ok('axios rejects on non-2xx with err.response (isAxiosError)');

// 9. bru.interpolate({{var}}) — resolves across scopes (runtime + environment), leaves an
//    unresolved template literal (rq never fabricates an unprovided variable).
const interpScript =
  "bru.setEnvVar('host', 'api.example.com'); bru.setVar('id', '42'); bru.setEnvVar('out', bru.interpolate('https://{{host}}/u/{{id}}?x={{missing}}'));";
const ir = await executeScript({ script: interpScript, phase: 'pre-request', context });
assert.ok(!ir.error, `no execution error (got: ${ir.error})`);
assert.equal(
  ir.mutationDiff.environment.out.localValue,
  'https://api.example.com/u/42?x={{missing}}',
  'bru.interpolate resolves {{var}} across scopes and leaves unknowns literal',
);
ok('bru.interpolate resolves {{var}} across scopes');

console.log(`\nE2E OK — ${passed} checks. cross-q-context transformed a Postman script and RAN it in QuickJS — variables, console, chai-backed rq.test, delegated fetch, cookies, AND a Bruno axios facade — end to end.`);
