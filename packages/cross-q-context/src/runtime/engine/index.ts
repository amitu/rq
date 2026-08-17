// cross-q-context — the safe QuickJS EXECUTOR (self-contained, zero app dep).
//
// The execute pillar: runs a (transformed) rq.* script inside a QuickJS isolate and returns the
// mutations / test results / logs. Ported from the app's sandbox-engine core, decoupled from the
// app. This barrel grows as the port lands (slice by slice); today it exposes the isolate
// primitives — the host↔guest value marshaller and the bridge factory every capability builds on.
//
// quickjs-emscripten-core is a TYPE-only import here (erased at build); the actual WASM variant is
// pulled in by the host layer, so consumers of these primitives take on no WASM weight.

// The execute entry — run a (transformed) rq.* script safely in QuickJS and get its result.
export { executeScript } from './execute.js';
export type { ExecuteScriptInput } from './execute.js';
export { createFetchBridge } from './fetch-bridge.js';
export type {
  FetchRequestData,
  FetchResponseData,
  SendRequestFn,
  BodyEncoding,
  SendRequestHost,
  SerializedFetchRequest,
  SerializedFetchResponse,
  SerializedFetchError,
  SerializedFetchEnvelope,
} from './host-types.js';
// SSRF guard (the direct-fetch path's egress denylist) + the delegated-fetch helpers.
export {
  createGuardedFetch,
  createGuardedLookup,
  isAddressBlocked,
  SsrfBlockedError,
  CLIENT_SSRF_POLICY,
  STRICT_SSRF_POLICY,
} from './ssrf-guard.js';
export type { SsrfPolicy } from './ssrf-guard.js';
export { describeDelegationFailure, toDelegatedFetch } from './delegated-fetch.js';

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

// The require chain — REQUIRE_ISOLATE_SHIM (guest require) over resolveRequire (the built-in /
// bridge / VENDOR_IIFES / SOURCE_BUNDLE tiers), the rollup source-bundler for user npm packages,
// the shim order, and the package-error sentinel (relocated Node-free off vm-package-evaluator).
export { REQUIRE_ISOLATE_SHIM, resolveRequire, extractRequireIds } from './isolated/isolated-require.js';
export type { BundleRequireDeps, PrebundledRequires } from './isolated/isolated-require.js';
export { createSourceBundler } from './isolated/source-bundler.js';
export type { BundleCache, SourceBundler } from './isolated/source-bundler.js';
export { ISOLATE_SHIMS } from './isolated/isolate-shim-order.js';
export { NEEDS_BRIDGE_MODULE_GLOBALS } from './isolated/needs-bridge-globals.js';
export { createImpossiblePackageError } from './isolated/impossible-error.js';
export {
  PACKAGE_ERROR_SENTINEL,
  isScriptPackageUnsupportedError,
  createPackageError,
} from './isolated/package-error-sentinel.js';
export type { ScriptPackageUnsupportedError } from './isolated/package-error-sentinel.js';

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

// Node-backed value bridges (Buffer/crypto/util/zlib) — the host halves of those capability shims,
// installed unconditionally by the full engine. And the push-based streaming result transport.
export { createBufferBridge } from './isolated/bridges/buffer-bridge.js';
export { createCryptoBridge } from './isolated/bridges/crypto-bridge.js';
export { createUtilBridge } from './isolated/bridges/util-bridge.js';
export { createZlibBridge } from './isolated/bridges/zlib-bridge.js';
export { StreamHandle } from './stream-handle.js';

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

// Host-side result processing (ADR-053 Layer 2 + ADR-105 cookies + ADR-208 on-message batching).
export { inflateMutations } from './inflate-mutations.js';
export { createDefaultVariableData, toVariableDataType } from './variable-data.js';
export type { RawMutationType } from './variable-data.js';
export { createInMemoryCookieJarBridge } from './cookies.js';
export type { CookieJarBridgeHandle } from './cookies.js';
export {
  ON_MESSAGE_TIMEOUT_ERROR,
  stampMessageIndex,
  createBatchOutcome,
  buildBatchResult,
} from './on-message-batch.js';
export type { BatchOutcome } from './on-message-batch.js';

// The host-side result type layer (inflated mutations, cookie family, rich result).
export type {
  MutationVariables,
  CollectionMutation,
  MutationDiff,
  TestResult,
  TestResultStatus,
  ScriptCookieSnapshot,
  ScriptCookie,
  CookieJarBridge,
  CookieJarSeed,
  CookieJarMutation,
  ScriptMessageError,
  ScriptExecutionResult,
} from './host-types.js';
