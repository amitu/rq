export type { ExecutionDirective, ExecutionMetadata, FeatureFlags, Json, LogEntry, MutationDiff, RequestHeaderMutation, RequestMutationDiff, RuntimeComponent, SandboxExecutionEvent, SandboxHostCallbacks, ScriptExecutionResult, StreamReader, TestResult, TestStatus, } from '../contract.js';
export { ScriptExecutionMode, ScriptPhase } from '../contract.js';
export type { EnvironmentVariables, FormDataKeyValue, GraphQLBody, GraphQLRequest, GraphQLResponse, GrpcRequest, GrpcScriptResponse, GrpcStreamMessage, HttpBody, HttpRequest, HttpResponse, KeyValue, ParsedGraphQLRequest, ParsedGrpcRequest, ParsedHttpRequest, ParsedKeyValue, PathVariable, ScriptMessageInput, VariableData, VariableDataType, } from '../model.js';
export { AuthType, EntryType, GrpcMethodType, RawBodyContentType, RequestContentType, RequestMethod } from '../model.js';
export type { CookieJarSeed, Sandbox, ScriptExecutionContext, ScriptExecutionInput } from '../execution.js';
import type { Json } from '../contract.js';
import { ScriptPhase } from '../contract.js';
/** Alias matching the app's boundary name for a JSON value. */
export type JsonValue = Json;
export declare enum ExecutionErrorPhase {
    preparation = "preparation",
    preScript = "pre-script",
    request = "request",
    postScript = "post-script",
    onMessageScript = "on-message-script"
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
export declare const PHASE_DESCRIPTORS: Readonly<Record<ScriptPhase, PhaseDescriptor>>;
export type VisualizerOutput = {
    kind: 'compiled';
    html: string;
    data: JsonValue;
} | {
    kind: 'error';
    message: string;
};
export type VisualizerDirective = VisualizerOutput | {
    kind: 'cleared';
};
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
export type RunRequestErrorKind = 'not_found' | 'cap_exceeded' | 'cycle_detected' | 'sub_execution_failed' | 'host_unavailable' | 'invalid_argument' | 'timeout';
export interface RunRequestError {
    kind: RunRequestErrorKind;
    message: string;
    requestId?: string;
}
export type RunRequestEnvelope = {
    ok: true;
    response: SerializedSubResponse;
} | {
    ok: false;
    error: RunRequestError;
};
/** Host capability that runs a saved request on a script's behalf (`rq.execution.runRequest`). The
 * sandbox never performs the sub-run itself — the host injects this and the run-request bridge
 * marshals the descriptor/envelope across the isolate edge. */
export interface RunRequestHost {
    runRequest(descriptor: RunRequestDescriptor): Promise<RunRequestEnvelope>;
}
/** Where a script error points, resolved to the user's editor coordinates (internal wrapper frames
 * removed, offsets corrected). All fields absent when no user-script frame could be anchored. */
export interface ScriptErrorLocation {
    /** 1-based line of the innermost user-script frame. */
    line?: number;
    /** 1-based column of the innermost user-script frame. */
    column?: number;
    /** Full multi-line stack for display — ready to render verbatim; do not re-parse. */
    stack?: string;
}
/** Injected variable resolver ($guid, {{var}} substitution, …). Opaque here — its implementation
 * and full parameter/return model stay host-side; the rq.* API only holds and invokes it. */
export interface VariableResolver {
    resolve(...args: unknown[]): unknown;
    /** The catalog this resolver provides — the developer engine registers a `$name()` per entry. */
    list(): ReadonlyArray<{
        readonly name: string;
    }>;
}
