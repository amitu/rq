// cross-q-context — the scripting runtime CONTRACT (self-contained, ADR-213 Layer 2 migration).
//
// This is the canonical, dependency-free contract for executing a script: a pure function of a
// serializable input to a serializable output, so every host (a browser tab, a Node worker, a CLI,
// a future `rq` app) speaks the same `execute(input) → result`. It deliberately imports NOTHING —
// cross-q-context must be self-contained in the `rq` repo with zero dependency on the current app.
//
// Requestly's app currently expresses this contract inside `@requestly/shared-types` (woven through
// its `common`/`runtime` type graph). As the runtime migrates here, the app becomes a CONSUMER: it
// maps its richer internal types onto this contract at the seam. This file is the source of truth;
// extra app-only channels (cookies, visualizer, packages, on-message) layer on top without changing
// the core shape.

/** A serializable JSON value. */
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

/** Which script phase is running. `rq.response` is absent in `pre-request`. */
export enum ScriptPhase {
  preRequest = 'pre-request',
  postResponse = 'post-response',
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

/**
 * The serializable context handed to the guest (JSON-parsed inside the realm). Variable scopes are
 * `key → value` maps; `request`/`response` are opaque JSON the shim reads. `response` is null in the
 * pre-request phase.
 */
export interface ScriptExecutionContext {
  environment: Record<string, Json>;
  globals: Record<string, Json>;
  collectionVariables: Record<string, Json>;
  variables: Record<string, Json>;
  request: Json;
  response: Json | null;
  info: ExecutionMetadata;
}

/** One `execute` call's input. */
export interface ScriptExecutionInput {
  script: string;
  phase: ScriptPhase;
  mode: ScriptExecutionMode;
  context: ScriptExecutionContext;
  timeoutMs?: number;
}

/** The outcome of one `rq.test(...)`. */
export type TestStatus = 'passed' | 'failed' | 'skipped';
export interface TestResult {
  name: string;
  status: TestStatus;
  error?: string;
}

/** A `console.*` line captured during execution. */
export interface LogEntry {
  level: string;
  args: Json[];
}

/**
 * The net variable changes per scope — `key → new value` for a set, `key → null` for unset/clear.
 * Consumers that need type/secret fidelity inflate this host-side; the runtime emits the raw shape.
 */
export interface MutationDiff {
  environment?: Record<string, Json>;
  globals?: Record<string, Json>;
  collectionVariables?: Record<string, Json>;
  variables?: Record<string, Json>;
}

/** A recorded change to the outgoing request's headers (`rq.request.headers.*`), tagged on `op`. */
export type RequestHeaderMutation =
  | { op: 'add'; key: string; value: string }
  | { op: 'upsert'; key: string; value: string }
  | { op: 'remove'; name: string }
  | { op: 'clear' };

export interface RequestMutationDiff {
  headers?: RequestHeaderMutation[];
}

/** A chaining directive drained from the run (`rq.execution.setNextRequest` / `skipRequest`). */
export type ExecutionDirective = { kind: 'set-next-request'; target: string | null } | { kind: 'skip-request' };

/** The result of one `execute` call. */
export interface ScriptExecutionResult {
  mutationDiff: MutationDiff;
  logs: LogEntry[];
  testResults: TestResult[];
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
export interface StreamReader<T> {
  read(): Promise<{ done: false; value: T } | { done: true; value?: undefined }>;
  cancel?(): Promise<void>;
}

/** Live events emitted during execution: logs stream as they happen, the result is terminal. */
export type SandboxExecutionEvent = { type: 'log'; log: LogEntry } | { type: 'result'; result: ScriptExecutionResult };

/**
 * Live per-execution host callbacks (NOT serialized — marshaled across the host boundary). The
 * sandbox never imports a fetcher or a repository; the host injects these. Both are optional: absent
 * `sendRequest` means the guest uses its direct host-fetch path.
 */
export interface SandboxHostCallbacks {
  sendRequest?: (request: Json) => Promise<Json>;
  runRequest?: (descriptor: Json) => Promise<Json>;
}

/** The engine contract: execute a script, stream events, terminate with a result. */
export interface Sandbox extends RuntimeComponent {
  execute(input: ScriptExecutionInput, hostCallbacks?: SandboxHostCallbacks): Promise<StreamReader<SandboxExecutionEvent>>;
}
