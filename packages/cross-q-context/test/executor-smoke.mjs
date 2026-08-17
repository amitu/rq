// Executor smoke — proves the ported isolate primitives (the execute pillar's foundation) work
// against a REAL QuickJS runtime, from inside cross-q-context. Exercises the host↔guest value
// marshaller round-trip + the bridge factory. Run: `node test/executor-smoke.mjs`. Exits non-zero
// on any failure so it works as a CI gate without a test framework.
import assert from 'node:assert/strict';

import asyncifyVariant from '@jitl/quickjs-singlefile-cjs-release-asyncify';
import { newQuickJSAsyncWASMModuleFromVariant } from 'quickjs-emscripten-core';

import {
  marshalToHandle,
  dumpHandle,
  createSafeBridge,
  isDebugEnabled,
  CORE_GLOBALS_SHIM,
  BUFFER_ISOLATE_SHIM,
  CRYPTO_ISOLATE_SHIM,
  FETCH_ISOLATE_SHIM,
  UTIL_ISOLATE_SHIM,
  ZLIB_ISOLATE_SHIM,
  RQ_ISOLATE_SHIM,
  CONSOLE_ISOLATE_SHIM,
  PROCESS_ISOLATE_SHIM,
  RUN_REQUEST_ISOLATE_SHIM,
  STREAM_ISOLATE_SHIM,
  DEPRECATION_ISOLATE_SHIM,
  createConsoleBridge,
  createTimerBridges,
  AsyncRegistry,
  SANDBOX_DEFAULT_TIMEOUT_MS,
} from '../dist/runtime/engine/index.js';

let passed = 0;
const ok = (name) => {
  passed++;
  console.log(`  ✓ ${name}`);
};

const mod = await newQuickJSAsyncWASMModuleFromVariant(asyncifyVariant);
const ctx = mod.newContext();
try {
  // 1. marshalToHandle → dumpHandle round-trips every Copyable shape through a real isolate.
  const cases = [
    ['string', 'hello'],
    ['number', 42],
    ['boolean', true],
    ['null', null],
    ['array', [1, 2, 3]],
    ['nested object', { a: 1, b: { c: [true, 'x'], d: null } }],
  ];
  for (const [label, value] of cases) {
    const h = marshalToHandle(ctx, value);
    const back = dumpHandle(ctx, h);
    h.dispose();
    assert.deepEqual(back, value, `round-trip ${label}`);
    ok(`marshal round-trip: ${label}`);
  }

  // 2. A host-marshalled object is readable by GUEST code (the real cross-edge path).
  const objH = marshalToHandle(ctx, { nums: [2, 4, 6], name: 'rq' });
  ctx.setProp(ctx.global, 'injected', objH);
  objH.dispose();
  const r = ctx.evalCode('injected.nums.reduce((a, b) => a + b, 0) + ":" + injected.name');
  assert.equal(r.error, undefined, 'guest reads injected object without error');
  assert.equal(ctx.dump(r.value), '12:rq', 'guest computed over marshalled data');
  r.value.dispose();
  ok('guest reads a host-marshalled object');

  // 3. The bridge factory constructs a SafeBridge (installation is exercised by the host layer).
  const bridge = createSafeBridge('probe', (x) => x * 2);
  assert.ok(bridge && typeof bridge === 'object', 'createSafeBridge returns a bridge');
  ok('bridge factory constructs a SafeBridge');

  // 4. debug-log helper is importable and inert by default.
  assert.equal(typeof isDebugEnabled(), 'boolean', 'isDebugEnabled returns a boolean');
  ok('debug-log helper importable');

  // 5. The ported guest-side realm strings are SYNTACTICALLY VALID QuickJS programs. Each is
  //    eval'd in a fresh isolate; a SyntaxError means the string-port corrupted the source. A
  //    ReferenceError (a shim reaching for a host global not yet installed) is expected here — the
  //    host layer installs those bridges before eval in the real assembly.
  const shims = [
    ['CORE_GLOBALS_SHIM', CORE_GLOBALS_SHIM],
    ['BUFFER_ISOLATE_SHIM', BUFFER_ISOLATE_SHIM],
    ['CRYPTO_ISOLATE_SHIM', CRYPTO_ISOLATE_SHIM],
    ['FETCH_ISOLATE_SHIM', FETCH_ISOLATE_SHIM],
    ['UTIL_ISOLATE_SHIM', UTIL_ISOLATE_SHIM],
    ['ZLIB_ISOLATE_SHIM', ZLIB_ISOLATE_SHIM],
    ['RQ_ISOLATE_SHIM', RQ_ISOLATE_SHIM],
    ['CONSOLE_ISOLATE_SHIM', CONSOLE_ISOLATE_SHIM],
    ['PROCESS_ISOLATE_SHIM', PROCESS_ISOLATE_SHIM],
    ['RUN_REQUEST_ISOLATE_SHIM', RUN_REQUEST_ISOLATE_SHIM],
    ['STREAM_ISOLATE_SHIM', STREAM_ISOLATE_SHIM],
    ['DEPRECATION_ISOLATE_SHIM', DEPRECATION_ISOLATE_SHIM],
  ];
  for (const [label, src] of shims) {
    assert.equal(typeof src, 'string', `${label} is a string`);
    assert.ok(src.length > 0, `${label} is non-empty`);
    const shimCtx = mod.newContext();
    try {
      const res = shimCtx.evalCode(src);
      if (res.error) {
        const err = shimCtx.dump(res.error);
        res.error.dispose();
        const name = (err && err.name) || String(err);
        assert.notEqual(name, 'SyntaxError', `${label} has no SyntaxError (got: ${JSON.stringify(err)})`);
      } else {
        res.value.dispose();
      }
    } finally {
      shimCtx.dispose();
    }
    ok(`guest-side realm string parses in QuickJS: ${label}`);
  }

  // 6. Capability bridges + async support are importable and construct.
  assert.equal(typeof createConsoleBridge, 'function', 'createConsoleBridge exported');
  assert.equal(typeof createTimerBridges, 'function', 'createTimerBridges exported');
  const logs = [];
  const consoleBridge = createConsoleBridge((e) => logs.push(e), () => 0);
  assert.ok(consoleBridge && typeof consoleBridge === 'object', 'console bridge constructs');
  ok('console bridge constructs');
  const registry = new AsyncRegistry();
  assert.ok(registry && typeof registry === 'object', 'AsyncRegistry constructs');
  ok('AsyncRegistry constructs');
  assert.equal(typeof SANDBOX_DEFAULT_TIMEOUT_MS, 'number', 'SANDBOX_DEFAULT_TIMEOUT_MS is a number');
  ok('constants importable');

  console.log(`\nExecutor smoke OK — ${passed} checks. Isolate primitives + guest realm + capability bridges run in QuickJS from cross-q-context.`);
} finally {
  ctx.dispose();
}
