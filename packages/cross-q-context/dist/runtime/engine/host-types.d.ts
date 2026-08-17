import type { VariableData, EnvironmentVariables } from '../model.js';
import type { LogEntry, RequestMutationDiff, ExecutionDirective } from '../contract.js';
import type { VisualizerDirective, ScriptErrorLocation } from '../definitions/_deps.js';
/** Per-key net change for a scope — `VariableData` for a set, `null` to delete. */
export type MutationVariables = Record<string, VariableData | null>;
/** Collection-scope mutations, tagged with the collection they belong to. */
export interface CollectionMutation {
    collectionId: string;
    variables: MutationVariables;
}
/** The inflated mutation diff produced host-side from the guest's raw mutations. Every value is a
 * full `VariableData` (or null to delete) — ready to persist, unlike the guest's raw key→value. */
export interface MutationDiff {
    global?: MutationVariables;
    environment?: MutationVariables;
    collection?: CollectionMutation;
    runtime?: MutationVariables;
    vault?: MutationVariables;
}
export type TestResultStatus = 'passed' | 'failed' | 'skipped';
export type TestResult = {
    status: 'passed';
    name: string;
    messageIndex?: number;
} | {
    status: 'failed';
    name: string;
    error: string;
    messageIndex?: number;
} | {
    status: 'skipped';
    name: string;
    messageIndex?: number;
};
/** A cookie as it crosses the isolate edge (JSON-serializable). */
export interface ScriptCookieSnapshot {
    name: string;
    value: string;
    domain: string;
    path: string;
    secure: boolean;
    httpOnly: boolean;
    expiry: {
        type: 'session';
    } | {
        type: 'absolute';
        date: string;
    };
}
/** A cookie as the in-memory jar bridge holds it (structurally identical to the snapshot). */
export type ScriptCookie = ScriptCookieSnapshot;
/** The per-execution cookie-jar bridge the host installs; the guest's `rq.cookies.jar(host)` calls it. */
export interface CookieJarBridge {
    list(host: string): readonly ScriptCookie[];
    upsert(host: string, cookie: ScriptCookie): void;
    remove(host: string, name: string, path: string): void;
    clear(host: string): void;
}
/** Read-side seed for the jar: pre-fetched cookies per allowed host. */
export type CookieJarSeed = readonly {
    host: string;
    cookies: readonly ScriptCookieSnapshot[];
}[];
/** A recorded cookie-jar change, drained after execution for the host to persist. */
export type CookieJarMutation = {
    kind: 'upsert';
    host: string;
    cookie: ScriptCookieSnapshot;
} | {
    kind: 'remove';
    host: string;
    name: string;
    path: string;
} | {
    kind: 'clear';
    host: string;
};
/** How a fetch response body is encoded in the flat `body` string (ADR-153): base64 for binary. */
export type BodyEncoding = 'utf8' | 'base64';
/** The request the guest hands out for the host to perform (copy-in; nothing live crosses). A
 * `type` (not `interface`) so it structurally satisfies the bridge's Copyable index-signature. */
export type FetchRequestData = {
    readonly url: string;
    readonly method: string;
    readonly headers: {
        readonly [key: string]: string;
    };
    readonly body?: string;
};
/** The response the host hands back (copy-out). `type` for the same Copyable reason. */
export type FetchResponseData = {
    readonly status: number;
    readonly statusText: string;
    readonly headers: {
        readonly [key: string]: string;
    };
    readonly body: string;
    readonly bodyEncoding: BodyEncoding;
};
/** The host capability that actually performs a sandbox script's fetch (rq.sendRequest / the guest
 * `fetch`). cross-q-context delegates every request out to this — it makes no network call itself,
 * so egress/SSRF policy stays with the host. A rejection's message is the only channel the script
 * sees, so throw a bounded, non-request-derived message. */
export type SendRequestFn = (request: FetchRequestData) => Promise<FetchResponseData>;
export type SerializedFetchRequest = FetchRequestData;
export type SerializedFetchResponse = FetchResponseData;
export type FetchErrorKind = string;
export type FetchDelegationErrorKind = string;
/** Kind-tagged error a delegated fetch returns; `fetchKind`/`code` sharpen the boundary category. */
export interface SerializedFetchError {
    kind: FetchDelegationErrorKind;
    message: string;
    fetchKind?: FetchErrorKind;
    code?: string;
}
export type SerializedFetchEnvelope = {
    ok: true;
    response: SerializedFetchResponse;
} | {
    ok: false;
    error: SerializedFetchError;
};
/** The host fetcher the full engine delegates to (envelope-returning, app-compatible). */
export interface SendRequestHost {
    sendRequest(request: SerializedFetchRequest): Promise<SerializedFetchEnvelope>;
}
/** An error from one iteration of an on-message batch, tagged with the message it came from. */
export interface ScriptMessageError {
    readonly messageIndex: number;
    readonly error: string;
    readonly errorLocation?: ScriptErrorLocation;
}
/** What one execute() yields: the inflated mutations, streamed logs (collected), test outcomes, and
 * the optional side-channels (request-header mutations, chaining directive, visualization, errors,
 * cookie mutations, and on-message batch stats). */
export interface ScriptExecutionResult {
    mutationDiff: MutationDiff;
    logs: LogEntry[];
    testResults: TestResult[];
    cookieMutations?: readonly CookieJarMutation[];
    requestMutationDiff?: RequestMutationDiff;
    executionDirective?: ExecutionDirective;
    visualizerOutput?: VisualizerDirective;
    error?: string;
    errorDetails?: unknown;
    errorLocation?: ScriptErrorLocation;
    messageErrors?: readonly ScriptMessageError[];
    messagesCompleted?: number;
    killedByTimeout?: boolean;
}
export type { VariableData, EnvironmentVariables };
