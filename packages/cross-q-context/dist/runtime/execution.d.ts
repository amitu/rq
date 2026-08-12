import type { ExecutionMetadata, FeatureFlags, Json, RuntimeComponent, SandboxExecutionEvent, SandboxHostCallbacks, ScriptPhase, ScriptExecutionMode, StreamReader } from './contract.js';
import type { EnvironmentVariables, GraphQLResponse, GrpcScriptResponse, HttpResponse, ParsedGraphQLRequest, ParsedGrpcRequest, ParsedHttpRequest, ScriptMessageInput } from './model.js';
/** Read-side cookie seed for `rq.cookies.jar(host)` (ADR-105): pre-fetched cookies per allowed host. */
export interface CookieJarSeed {
    host: string;
    cookies: readonly Json[];
}
/**
 * The serializable context handed to the guest (JSON-parsed inside the realm). Variable scopes are
 * `key → VariableData` maps; `request`/`response` are the parsed model shapes; `response` is null in
 * the pre-request phase.
 */
export interface ScriptExecutionContext {
    global: EnvironmentVariables;
    collectionVariables: EnvironmentVariables;
    environment: EnvironmentVariables;
    variables: EnvironmentVariables;
    iterationData: EnvironmentVariables;
    secrets: EnvironmentVariables;
    request: ParsedHttpRequest | ParsedGraphQLRequest | ParsedGrpcRequest;
    response: HttpResponse | GraphQLResponse | GrpcScriptResponse | null;
    /** Set only in the on-message phase, re-set per iteration. */
    message?: ScriptMessageInput;
    info: ExecutionMetadata;
    /** Ordered names collection → folders → request, for `rq.execution.location`. */
    location?: readonly string[];
    /** True when the vault-access device setting is off — `rq.vault.*` throws instead of returning empty. */
    secretsAccessDisabled?: boolean;
    /** Hosts granted `rq.cookies.jar(host)` access (ADR-105). Empty = no grants. */
    hostAllowlist: readonly string[];
    cookieJarSeed?: readonly CookieJarSeed[];
    /** Entry-level auth for protocols where auth lives on the entry, not the request (gRPC, MQTT). */
    auth?: Readonly<Record<string, unknown>>;
}
/** One `execute` call's input. */
export interface ScriptExecutionInput {
    script: string;
    phase: ScriptPhase;
    mode: ScriptExecutionMode;
    context: ScriptExecutionContext;
    timeoutMs?: number;
}
/** The engine contract: execute a script, stream events, terminate with a result. */
export interface Sandbox extends RuntimeComponent {
    execute(input: ScriptExecutionInput, hostCallbacks?: SandboxHostCallbacks): Promise<StreamReader<SandboxExecutionEvent>>;
}
/** Feature-flag helper alias (kept for symmetry with the app). */
export type { FeatureFlags };
