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

console.log(`\nE2E OK — ${passed} checks. cross-q-context transformed a Postman script and RAN it in QuickJS — variables, console, AND chai-backed rq.test — end to end.`);
