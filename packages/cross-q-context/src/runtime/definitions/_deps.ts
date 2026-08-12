// The single dependency seam for the vendored rq.* API (ADR-213 Layer 2, step 3). Every file under
// `definitions/` imports what it used to pull from `@requestly/*` from HERE, so the rq.* API is
// self-contained. Most symbols re-export the contract/model/execution layers; a few rq.*-only leaf
// types (phase descriptors, visualizer, runRequest, the injected VariableResolver) are defined here.

export type {
  ExecutionDirective,
  ExecutionMetadata,
  FeatureFlags,
  Json,
  LogEntry,
  MutationDiff,
  RequestHeaderMutation,
  RequestMutationDiff,
  RuntimeComponent,
  SandboxExecutionEvent,
  SandboxHostCallbacks,
  ScriptExecutionResult,
  StreamReader,
  TestResult,
  TestStatus,
} from '../contract.js';
export { ScriptExecutionMode, ScriptPhase } from '../contract.js';

export type {
  EnvironmentVariables,
  FormDataKeyValue,
  GraphQLBody,
  GraphQLRequest,
  GraphQLResponse,
  GrpcRequest,
  GrpcScriptResponse,
  GrpcStreamMessage,
  HttpBody,
  HttpRequest,
  HttpResponse,
  KeyValue,
  ParsedGraphQLRequest,
  ParsedGrpcRequest,
  ParsedHttpRequest,
  ParsedKeyValue,
  PathVariable,
  ScriptMessageInput,
  VariableData,
  VariableDataType,
} from '../model.js';
export { AuthType, EntryType, GrpcMethodType, RawBodyContentType, RequestContentType, RequestMethod } from '../model.js';

export type { CookieJarSeed, Sandbox, ScriptExecutionContext, ScriptExecutionInput } from '../execution.js';

import type { Json } from '../contract.js';
import { ScriptPhase } from '../contract.js';

/** Alias matching the app's boundary name for a JSON value. */
export type JsonValue = Json;

// ── phase descriptors (from shared-types/runtime) ───────────────────────────────────────────
export enum ExecutionErrorPhase {
  preparation = 'preparation',
  preScript = 'pre-script',
  request = 'request',
  postScript = 'post-script',
  onMessageScript = 'on-message-script',
}

/** The pre/post/on-message script bodies a source (collection/request) carries. */
export interface CollectionScripts {
  preRequest?: string;
  postResponse?: string;
  onMessage?: string;
}

export interface PhaseDescriptor {
  readonly scriptsField: keyof CollectionScripts;
  readonly errorPhase: ExecutionErrorPhase;
  readonly scriptFilename: string;
  readonly contextIdPrefix: string;
  readonly dtsBasename: string;
  readonly exclusiveSurface: readonly string[];
}

export const PHASE_DESCRIPTORS: Readonly<Record<ScriptPhase, PhaseDescriptor>> = {
  [ScriptPhase.preRequest]: {
    scriptsField: 'preRequest',
    errorPhase: ExecutionErrorPhase.preScript,
    scriptFilename: 'pre-request-script.js',
    contextIdPrefix: 'pre',
    dtsBasename: 'pre-request',
    exclusiveSurface: ['visualizer'],
  },
  [ScriptPhase.postResponse]: {
    scriptsField: 'postResponse',
    errorPhase: ExecutionErrorPhase.postScript,
    scriptFilename: 'post-response-script.js',
    contextIdPrefix: 'post',
    dtsBasename: 'post-response',
    exclusiveSurface: ['response', 'visualizer'],
  },
  [ScriptPhase.onMessage]: {
    scriptsField: 'onMessage',
    errorPhase: ExecutionErrorPhase.onMessageScript,
    scriptFilename: 'on-message-script.js',
    contextIdPrefix: 'on-message',
    dtsBasename: 'on-message',
    exclusiveSurface: ['message'],
  },
};

// ── visualizer (ADR-202) ────────────────────────────────────────────────────────────────────
export type VisualizerOutput = { kind: 'compiled'; html: string; data: JsonValue } | { kind: 'error'; message: string };
export type VisualizerDirective = VisualizerOutput | { kind: 'cleared' };

// ── runRequest boundary (ADR-169) ───────────────────────────────────────────────────────────
export interface RunRequestDescriptor {
  requestId: string;
  variableOverrides?: Readonly<Record<string, string>>;
  depth: number;
}
export interface SerializedSubResponse {
  code: number;
  status: string;
  headers: Readonly<Record<string, string>>;
  responseBody: string;
  responseTime: number;
}
export type RunRequestErrorKind =
  | 'not_found'
  | 'cap_exceeded'
  | 'cycle_detected'
  | 'sub_execution_failed'
  | 'host_unavailable'
  | 'invalid_argument'
  | 'timeout';
export interface RunRequestError {
  kind: RunRequestErrorKind;
  message: string;
  requestId?: string;
}
export type RunRequestEnvelope = { ok: true; response: SerializedSubResponse } | { ok: false; error: RunRequestError };

/** Injected variable resolver ($guid, {{var}} substitution, …). Opaque here — its implementation
 * and full parameter/return model stay host-side; the rq.* API only holds and invokes it. */
export interface VariableResolver {
  resolve(...args: unknown[]): unknown;
}
