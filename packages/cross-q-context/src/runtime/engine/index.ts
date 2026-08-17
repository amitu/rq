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

// Guest-side realm setup — JS-as-strings eval'd inside the isolate to build the sandbox globals.
// The core globals + the Node capability shims (Buffer/crypto/fetch/util/zlib) + the Safe-mode
// rq.* shim (the guest twin of the Developer-mode createRqNamespace). Zero app coupling.
export { CORE_GLOBALS_SHIM } from './isolated/core-globals.js';
export { BUFFER_ISOLATE_SHIM } from './isolated/shims/buffer.shim.js';
export { CRYPTO_ISOLATE_SHIM } from './isolated/shims/crypto.shim.js';
export { FETCH_ISOLATE_SHIM } from './isolated/shims/fetch.shim.js';
export { UTIL_ISOLATE_SHIM } from './isolated/shims/util.shim.js';
export { ZLIB_ISOLATE_SHIM } from './isolated/shims/zlib.shim.js';
export { RQ_ISOLATE_SHIM, RQ_ITERATION_RESET_EXPR, RQ_COLLECT_EXPR } from './isolated/isolated-rq.js';
export type { InIsolateCollected } from './isolated/isolated-rq.js';

// Capability bridges — the host-side halves that back the guest shims (console/process/fetch's
// run-request/streams/deprecations/timers). Each pairs a createXBridge() (or a constant table)
// with its *_ISOLATE_SHIM guest string. Installed by the host layer before the realm evals.
export { createConsoleBridge, CONSOLE_ISOLATE_SHIM } from './isolated/bridges/console-bridge.js';
export { PROCESS_ISOLATE_SHIM } from './isolated/bridges/process-bridge.js';
export { createRunRequestBridge, RUN_REQUEST_ISOLATE_SHIM } from './isolated/bridges/run-request-bridge.js';
export { STREAM_ISOLATE_SHIM } from './isolated/bridges/stream-bridge.js';
export {
  createDeprecationBridge,
  DEPRECATION_ISOLATE_SHIM,
  WARN_ONLY_IDENTIFIERS,
  DEPRECATION_SHIMMED_IDENTIFIERS,
} from './isolated/bridges/deprecation-bridge.js';
export { createTimerBridges } from './isolated/bridges/timer-bridge.js';

// Async lifecycle + support.
export { AsyncRegistry } from './async-registry.js';
export type { TimerDelegations, AsyncRegistryOptions, SettleFn } from './async-registry.js';
export { SANDBOX_DEFAULT_TIMEOUT_MS } from './constants.js';
export {
  scriptFilenameForPhase,
  parseScriptErrorLocation,
  countScriptLines,
  UserScriptError,
  WRAPPER_LINE_OFFSET,
} from './script-error-location.js';
