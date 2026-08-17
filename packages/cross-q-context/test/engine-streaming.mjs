// The FULL engine — QuickJsSandbox.execute() streaming a Postman script through the drop-in
// Sandbox interface: a StreamReader<SandboxExecutionEvent> with live log events + a terminal
// result. This is what the app consumes when it deletes its own engine. Run: node this-file.
import assert from 'node:assert/strict';

import { transformScript } from '../src/index.js';
import { QuickJsSandbox } from '../dist/runtime/engine/index.js';

let passed = 0;
const ok = (name) => {
  passed++;
  console.log(`  ✓ ${name}`);
};

const context = {
  global: {}, collectionVariables: {}, environment: {}, variables: {}, iterationData: {}, secrets: {},
  request: { url: 'https://example.com', method: 'GET', headers: [], queryParams: [], pathVariables: [], body: { contentType: 'none', formUrlEncoded: [], formData: [] }, contentType: 'none' },
  response: null,
  info: { requestId: 'r1', requestName: 'demo', iteration: 0, iterationCount: 1, entryIndex: 0, totalEntries: 1, collectionId: null },
  hostAllowlist: [],
};

const source = "console.log('streaming hi');\npm.environment.set('token', 'xyz');\npm.test('math', () => { pm.expect(2 + 2).to.equal(4); });";
const t = transformScript({ source, platform: 'postman' });
assert.equal(t.success, true, 'transform ok');

const sandbox = new QuickJsSandbox();
const reader = await sandbox.execute({
  script: t.code,
  phase: 'post-response',
  mode: 'safe',
  context,
  entryId: 'entry-1',
  entryType: 'http',
  blacklistedPackages: [],
});

// Drain the StreamReader — the app's consumption pattern.
const events = [];
let r;
while (!(r = await reader.read()).done) events.push(r.value);
ok(`drained ${events.length} streamed event(s) from the StreamReader`);

const logEvents = events.filter((e) => e.type === 'log');
assert.ok(logEvents.some((e) => e.log.args.some((a) => String(a).includes('streaming hi'))), 'log streamed live');
ok('console.log arrived as a live log event (not just in the result)');

const resultEvent = events.find((e) => e.type === 'result');
assert.ok(resultEvent, 'a terminal result event was streamed');
const result = resultEvent.result;
assert.ok(!result.error, `no error (got: ${result.error})`);
assert.equal(result.mutationDiff.environment.token.localValue, 'xyz', 'variable mutation in the result');
assert.equal(result.testResults.length, 1, 'one test result');
assert.equal(result.testResults[0].status, 'passed', 'chai assertion passed through the full engine');
ok('terminal result: inflated mutation + passing chai test');

// getFeatures() — part of the Sandbox/RuntimeComponent contract.
const features = await sandbox.getFeatures();
assert.equal(typeof features, 'object', 'getFeatures returns flags');
ok('Sandbox.getFeatures() responds');

console.log(`\nEngine streaming OK — ${passed} checks. The full QuickJsEngine streams a script through the drop-in Sandbox interface.`);
