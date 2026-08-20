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

// 4. A BRUNO script on the developer engine. Bruno has no transform — the source runs verbatim
// against the `bru`/`req` + bare `test`/`expect` runtime shim. Bruno's own scripts are node:vm-
// native (they `require('uuid'|'nanoid'|…)`), so the developer engine is exactly where they run;
// before the shim was wired here they died on `bru is not defined`.
// bru.setVar writes the RUNTIME scope (transient, like Postman's pm.variables) which is not
// persisted to mutationDiff — so its round-trip is asserted in-script via a test() instead.
const bruSource = [
  "bru.setEnvVar('token', 'bruno');",
  "bru.setVar('copy', bru.getEnvVar('token'));",
  "test('math', () => { expect(2 * 2).to.equal(4); });",
  "test('req url', () => { expect(req.getUrl()).to.equal('https://example.com'); });",
  "test('runtime var roundtrip', () => { expect(bru.getVar('copy')).to.equal('bruno'); });",
].join('\n');
const bruInput = {
  script: bruSource,
  phase: 'post-response',
  mode: 'developer',
  context,
  entryId: 'bru-1',
  entryType: 'http',
  blacklistedPackages: [],
};
const bruResult = (await drain(await new NodeSandbox().execute(bruInput))).find((e) => e.type === 'result')?.result;
assert.ok(bruResult && !bruResult.error, `no error (got: ${bruResult?.error})`);
assert.equal(bruResult.mutationDiff.environment.token.localValue, 'bruno', 'bru.setEnvVar via node:vm');
assert.equal(bruResult.testResults.length, 3, 'all three bare test() blocks ran');
assert.ok(bruResult.testResults.every((t) => t.status === 'passed'), 'bare test/expect + req.getUrl + bru.getVar roundtrip passed');
ok('NodeSandbox runs a BRUNO script (bru/req + bare test/expect) end-to-end');

// 5. The dispatcher (the app's actual entry) runs Bruno on the developer engine too.
const bruDispatched = (await drain(await new DispatchingSandbox(new NodeSandbox()).execute(bruInput))).find((e) => e.type === 'result')?.result;
assert.equal(bruDispatched.mutationDiff.environment.token.localValue, 'bruno', 'dispatcher → developer engine runs Bruno');
ok('DispatchingSandbox runs a Bruno script on the developer engine');

// 6. require('axios') resolves to the facade in the developer engine (the shim wraps the node:vm
// require). Behaviour (request/response/throw) is covered against a mock transport in execute-e2e;
// here we prove the dev-engine wiring — the facade and its methods are what require() returns.
const axiosLoadSrc =
  "const axios = require('axios'); bru.setEnvVar('t', [typeof axios, typeof axios.get, typeof axios.post, typeof axios.create].join(':'));";
const axiosLoad = (await drain(await new NodeSandbox().execute({ script: axiosLoadSrc, phase: 'post-response', mode: 'developer', context, entryId: 'ax-1', entryType: 'http', blacklistedPackages: [] }))).find((e) => e.type === 'result')?.result;
assert.ok(axiosLoad && !axiosLoad.error, `no error (got: ${axiosLoad?.error})`);
assert.equal(axiosLoad.mutationDiff.environment.t.localValue, 'function:function:function:function', "require('axios') → facade with get/post/create");
ok("require('axios') resolves to the facade on the developer engine");

// 7. Bruno inbuilt libraries nanoid + tv4 resolve via require() and run in BOTH engines. nanoid
// draws from crypto.getRandomValues (the Safe crypto bridge / real node crypto); tv4 is pure JS.
const libScript =
  "const { nanoid } = require('nanoid'); const tv4 = require('tv4'); const id = nanoid(); bru.setEnvVar('lib', typeof id + ':' + (id.length > 10) + ':' + tv4.validate({ n: 5 }, { type: 'object', properties: { n: { type: 'number' } } }));";
const libInput = { script: libScript, phase: 'post-response', mode: 'developer', context, entryId: 'lib-1', entryType: 'http', blacklistedPackages: [] };
const libDispatcher = new DispatchingSandbox(new NodeSandbox());
const devLib = (await drain(await libDispatcher.execute({ ...libInput, mode: 'developer' }))).find((e) => e.type === 'result')?.result;
assert.ok(devLib && !devLib.error, `no error (got: ${devLib?.error})`);
assert.equal(devLib.mutationDiff.environment.lib.localValue, 'string:true:true', 'nanoid + tv4 in the developer engine');
const safeLib = (await drain(await libDispatcher.execute({ ...libInput, mode: 'safe' }))).find((e) => e.type === 'result')?.result;
assert.ok(safeLib && !safeLib.error, `no error (got: ${safeLib?.error})`);
assert.equal(safeLib.mutationDiff.environment.lib.localValue, 'string:true:true', 'nanoid + tv4 in the safe engine');
ok('Bruno inbuilt libs nanoid + tv4 run via require() in both engines');

console.log(`\nDeveloper engine OK — ${passed} checks. Both engines + the safe/developer picker run in cross-q-context.`);
