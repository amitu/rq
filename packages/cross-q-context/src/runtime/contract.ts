// cross-q-context — the scripting runtime CONTRACT primitives (self-contained, ADR-213 Layer 2).
//
// The low-level, model-free primitives every host shares. The request/response DATA MODEL is in
// `model.ts`; the composed execution types (`ScriptExecutionInput`/`Context`, `Sandbox`) that key
// off both live in `execution.ts`. This file imports NOTHING — cross-q-context is self-contained in
// the `rq` repo with zero dependency on the current app.

/** A serializable JSON value. Arrays/objects are `readonly` to match the app's `JsonValue` at the
 * consumption seam (a readonly value is safely assignable where a readonly one is expected). */
export type Json = null | boolean | number | string | readonly Json[] | { readonly [key: string]: Json };

/** Which script phase is running. `rq.response` is absent in `pre-request`; `on-message` runs per
 * inbound realtime message (WebSocket/Socket.IO/gRPC stream). */
export enum ScriptPhase {
  preRequest = 'pre-request',
  postResponse = 'post-response',
  onMessage = 'on-message',
}

/**
 * The sandbox engine. Published (WASM/browser) builds are `safe` only; an unrecognized value
 * resolves to `safe` (fail-closed). `developer` (`node:vm`) is a host-embedding, trusted-code
 * opt-in — never offered in a browser or a published build.
 */
export enum ScriptExecutionMode {
  safe = 'safe',
  developer = 'developer',
}

/** Immutable metadata about the run, surfaced to the script as `rq.info`. */
export interface ExecutionMetadata {
  requestId: string;
  requestName: string;
  iteration: number;
  iterationCount: number;
  entryIndex: number;
  totalEntries: number;
  collectionId: string | null;
}

/** The outcome of one `rq.test(...)`, as the guest records it. The host-side union with its
 * per-status fields is `TestResult` in `engine/host-types.ts`. */
export type TestStatus = 'passed' | 'failed' | 'skipped';
export interface RawTestResult {
  name: string;
  status: TestStatus;
  error?: string;
}

/** The `console.*` methods captured during execution. */
export enum LogLevel {
  log = 'log',
  warn = 'warn',
  error = 'error',
  info = 'info',
}

/** A `console.*` line captured during execution. */
export interface LogEntry {
  level: string;
  args: Json[];
  /** ms epoch when the line was captured (set by the console bridge). */
  timestamp?: number;
}

/**
 * The net variable changes per scope, **as the guest records them** — `key → new value` for a set,
 * `key → null` for unset/clear.
 *
 * This is the *pre-inflation* shape and it is not what a host receives. `execute()` returns the
 * inflated {@link MutationDiff} from `engine/host-types.ts` (scopes `global`/`runtime`, values as
 * full `VariableData`), which mirrors the app's own type so a host can consume it directly. Both
 * were once called `MutationDiff`; a host that implemented this one read an empty map and silently
 * dropped every `rq.variables.set`, which is why the raw one now says that it is raw.
 */
export interface RawMutationDiff {
  environment?: Record<string, Json>;
  globals?: Record<string, Json>;
  collectionVariables?: Record<string, Json>;
  variables?: Record<string, Json>;
}

/** A recorded change to the outgoing request's headers (`rq.request.headers.*`), tagged on `kind`. */
export type RequestHeaderMutation =
  | { kind: 'add'; name: string; value: string }
  | { kind: 'upsert'; name: string; value: string }
  | { kind: 'remove'; name: string }
  | { kind: 'clear' };

export interface RequestMutationDiff {
  headers?: readonly RequestHeaderMutation[];
}

/** A chaining directive drained from the run (`rq.execution.setNextRequest` / `skipRequest`). */
export type ExecutionDirective = { kind: 'set-next-request'; target: string | null } | { kind: 'skip-request' };

/**
 * The result of one execution **as the guest produces it**, before the host inflates it.
 *
 * Not what `execute()` returns: that is the richer {@link ScriptExecutionResult} in
 * `engine/host-types.ts`, which mirrors the app's type. See {@link RawMutationDiff}.
 */
export interface RawScriptExecutionResult {
  mutationDiff: RawMutationDiff;
  logs: LogEntry[];
  testResults: RawTestResult[];
  requestMutationDiff?: RequestMutationDiff;
  executionDirective?: ExecutionDirective;
  error?: string;
}

/** Feature flags a runtime component advertises. */
export type FeatureFlags = Record<string, boolean>;

/** Every runtime component advertises its capabilities. */
export interface RuntimeComponent {
  getFeatures(): Promise<FeatureFlags>;
}

/** A pull reader over a stream of events (a subset of the async-iterator protocol). */
export type StreamReadResult<T> = { done: false; value: T } | { done: true; value?: undefined };
export interface StreamReader<T> {
  read(): Promise<StreamReadResult<T>>;
  cancel?(): Promise<void>;
}

/** A touch of a deprecated Postman global (`globals`, `tv4`, …), streamed as it happens. */
export interface DeprecationSignal {
  identifier: string;
  /** Did a shim let the call execute (true), or did the access fail/no-op (false)? */
  shimmed: boolean;
}

/** Live events during execution, carrying the *raw* result. A host consuming `execute()` gets
 * `SandboxExecutionEvent` from `engine/host-types.ts`, whose terminal event carries the inflated
 * result. */
export type RawSandboxExecutionEvent =
  | { type: 'log'; log: LogEntry }
  | { type: 'deprecation'; signal: DeprecationSignal }
  | { type: 'result'; result: RawScriptExecutionResult };

/**
 * Live per-execution host callbacks (NOT serialized — marshaled across the host boundary). The
 * sandbox never imports a fetcher or a repository; the host injects these. Both are optional: absent
 * `sendRequest` means the guest uses its direct host-fetch path.
 */
export interface SandboxHostCallbacks {
  sendRequest?: (request: Json) => Promise<Json>;
  runRequest?: (descriptor: Json) => Promise<Json>;
}
