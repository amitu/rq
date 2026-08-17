// cross-q-context — the safe QuickJS EXECUTOR (self-contained, zero app dep).
//
// The execute pillar: runs a (transformed) rq.* script inside a QuickJS isolate and returns the
// mutations / test results / logs. Ported from the app's sandbox-engine core, decoupled from the
// app. This barrel grows as the port lands (slice by slice); today it exposes the isolate
// primitives — the host↔guest value marshaller and the bridge factory every capability builds on.
//
// quickjs-emscripten-core is a TYPE-only import here (erased at build); the actual WASM variant is
// pulled in by the host layer, so consumers of these primitives take on no WASM weight.

export { dumpHandle, marshalToHandle } from './isolated/marshal.js';
export { createSafeBridge, createIgnoredBridge, pendingAsyncCalls } from './isolated/safe-bridge-factory.js';
export type { Copyable, SafeBridge, BridgeHandler, AsyncBridgeHandler } from './isolated/safe-bridge-factory.js';
export { dlog, isDebugEnabled } from './isolated/debug-log.js';
