// Executor smoke — proves the ported isolate primitives (the execute pillar's foundation) work
// against a REAL QuickJS runtime, from inside cross-q-context. Exercises the host↔guest value
// marshaller round-trip + the bridge factory. Run: `node test/executor-smoke.mjs`. Exits non-zero
// on any failure so it works as a CI gate without a test framework.
import assert from 'node:assert/strict';

import asyncifyVariant from '@jitl/quickjs-singlefile-cjs-release-asyncify';
import { newQuickJSAsyncWASMModuleFromVariant } from 'quickjs-emscripten-core';

import { marshalToHandle, dumpHandle, createSafeBridge, isDebugEnabled } from '../dist/runtime/engine/index.js';

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

  console.log(`\nExecutor smoke OK — ${passed} checks. Isolate primitives run in QuickJS from cross-q-context.`);
} finally {
  ctx.dispose();
}
