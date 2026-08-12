// Smoke test — proves the built WASM package transforms through the JS wrapper in Node
// (the `#engine` -> engine-node path). Run: `node test/smoke.mjs`. Exits non-zero on any
// failure so it works as a CI gate without a test framework.

import assert from 'node:assert/strict';
import { batchTransformScripts, extractRequires, transformScript } from '../src/index.js';

let passed = 0;
const ok = (name) => {
  passed++;
  console.log(`  ✓ ${name}`);
};

// 1. pm.* namespace rename + a recorded replacement
const t = transformScript({
  source: "pm.test('ok', () => pm.response.to.have.status(200));",
  platform: 'postman',
});
assert.equal(t.success, true, 'transform success');
assert.ok(t.code.includes('rq.'), 'pm.* rewritten to rq.*');
assert.ok(!t.code.includes('pm.'), 'no pm.* left');
assert.ok(t.summary.replacements > 0, 'replacements recorded');
ok(`transformScript(pm.*) -> ${JSON.stringify(t.summary)}`);

// 2. postman.* legacy alias
const legacy = transformScript({
  source: 'postman.setEnvironmentVariable("k", "v");',
  platform: 'postman',
});
assert.equal(legacy.success, true, 'legacy transform success');
assert.ok(legacy.code.includes('rq.'), 'postman.* rewritten to rq.*');
ok('transformScript(postman.*) -> rq.*');

// 3. batch pre/post pairs aggregate a summary
const batch = batchTransformScripts({
  platform: 'postman',
  scripts: {
    a: { preRequest: 'pm.environment.set("x", 1);', postResponse: "pm.test('t', () => {});" },
  },
});
assert.ok(batch.results.a.preRequest.success && batch.results.a.postResponse.success, 'batch entries transformed');
assert.ok(batch.summary.replacements >= 2, 'batch summary aggregated');
ok(`batchTransformScripts -> ${JSON.stringify(batch.summary)}`);

// 4. extractRequires pulls static string-literal require() ids (ADR-084)
const reqs = extractRequires("const _ = require('lodash@4.17.21'); const x = require(dynamic);");
assert.equal(reqs.length, 1, 'only the static require extracted');
assert.equal(reqs[0].rawId, 'lodash@4.17.21', 'raw id captured');
assert.ok(typeof reqs[0].span.start === 'number', 'span present');
ok(`extractRequires -> ${JSON.stringify(reqs.map((r) => r.rawId))}`);

console.log(`\n${passed} checks passed.`);
