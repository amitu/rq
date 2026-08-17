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

console.log(`\nE2E OK — ${passed} checks. cross-q-context transformed a Postman script and RAN it in QuickJS, end to end.`);
