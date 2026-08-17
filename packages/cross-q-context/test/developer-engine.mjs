// The DEVELOPER engine — NodeSandbox (node:vm) running a script through the SAME streaming Sandbox
// interface as the safe engine, plus the DispatchingSandbox picking safe-vs-developer by mode.
// This is the two-engine choice the app offers, now in cross-q-context. Run: node this-file.
import assert from 'node:assert/strict';

import { transformScript } from '../src/index.js';
import { NodeSandbox, DispatchingSandbox } from '../dist/runtime/engine/index.js';

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

async function drain(reader) {
  const events = [];
  let r;
  while (!(r = await reader.read()).done) events.push(r.value);
  return events;
}

const source = "console.log('developer hi');\npm.environment.set('token', 'dev');\npm.test('math', () => { pm.expect(3 * 3).to.equal(9); });";
const t = transformScript({ source, platform: 'postman' });
const input = {
  script: t.code,
  phase: 'post-response',
  mode: 'developer',
  context,
  entryId: 'entry-1',
  entryType: 'http',
  blacklistedPackages: [],
};

// 1. NodeSandbox (node:vm) directly.
const dev = new NodeSandbox();
const events = await drain(await dev.execute(input));
const result = events.find((e) => e.type === 'result')?.result;
assert.ok(result && !result.error, `no error (got: ${result?.error})`);
assert.equal(result.mutationDiff.environment.token.localValue, 'dev', 'variable mutation via node:vm');
assert.equal(result.testResults[0].status, 'passed', 'chai assertion passed in the developer engine');
assert.ok(events.some((e) => e.type === 'log' && e.log.args.some((a) => String(a).includes('developer hi'))), 'log streamed');
ok('NodeSandbox (node:vm developer engine) runs a script end-to-end');

// 2. DispatchingSandbox routes by mode: developer → NodeSandbox, safe → (lazy) QuickJsSandbox.
const dispatcher = new DispatchingSandbox(new NodeSandbox());
const devResult = (await drain(await dispatcher.execute({ ...input, mode: 'developer' }))).find((e) => e.type === 'result')?.result;
assert.equal(devResult.mutationDiff.environment.token.localValue, 'dev', 'dispatcher → developer engine');
ok('DispatchingSandbox routes mode=developer to the node:vm engine');

const safeResult = (await drain(await dispatcher.execute({ ...input, mode: 'safe' }))).find((e) => e.type === 'result')?.result;
assert.equal(safeResult.mutationDiff.environment.token.localValue, 'dev', 'dispatcher → safe engine (lazy QuickJS)');
assert.equal(safeResult.testResults[0].status, 'passed', 'safe engine assertion passed via the dispatcher');
ok('DispatchingSandbox routes mode=safe to the lazily-loaded QuickJS engine');

console.log(`\nDeveloper engine OK — ${passed} checks. Both engines + the safe/developer picker run in cross-q-context.`);
