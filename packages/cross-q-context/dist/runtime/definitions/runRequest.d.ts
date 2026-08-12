/**
 * `rq.execution.runRequest()` — sandbox scripting surface (ADR-169, Postman
 * `pm.execution.runRequest` parity).
 *
 * Boundary-Protocol-Pure (ADR-169 Option B): the factory builds a serializable
 * `RunRequestDescriptor`, hands it to an injected host round-trip (`impl`), and
 * rehydrates the returned `RunRequestEnvelope` into a Postman-shaped response
 * (`.code`, `.status`, `.headers.get()`, `.json()`, `.text()`) — mirroring the
 * response shape `sendRequest.ts` exposes. Nothing live crosses the boundary;
 * the descriptor goes out, the envelope comes back.
 *
 * Promise-only (no callback form): Postman's `runRequest` returns a Promise.
 *
 * Error contract (gr-discriminated-errors-at-boundaries): a failure rejects with
 * a kind-tagged `RunRequestFailure` carrying the closed `RunRequestErrorKind`;
 * an HTTP 4xx/5xx is NOT a failure — the response is delivered like sendRequest.
 *
 * The factory takes its single dependency (`impl`) as a parameter — no globals —
 * so it stays platform-agnostic and unit-testable with a mock.
 */
import type { RunRequestDescriptor, RunRequestEnvelope, RunRequestErrorKind } from './_deps.js';
/** The host round-trip: takes the descriptor, runs the child off-process, returns the envelope. Injected per engine. */
export type RunRequestImpl = (descriptor: RunRequestDescriptor) => Promise<RunRequestEnvelope>;
/** Options accepted by rq.execution.runRequest(id, opts?). */
export interface RunRequestOptions {
    variables?: Readonly<Record<string, string>>;
}
/**
 * Response headers exposed to scripts. Mirrors `sendRequest`'s `ScriptHeaderList`:
 * a case-insensitive `get(name)` plus index access, so both
 * `res.headers.get('content-type')` and `res.headers['content-type']` work.
 */
export interface RunRequestHeaderList {
    get(name: string): string | undefined;
    [key: string]: string | ((name: string) => string | undefined) | undefined;
}
/** Postman-shaped response returned to the script (rehydrated from SerializedSubResponse). Mirrors sendRequest's response shape. */
export interface RunRequestResponse {
    /** Numeric HTTP status, e.g. 200. */
    code: number;
    /** HTTP status text, e.g. "OK". */
    status: string;
    headers: RunRequestHeaderList;
    /** Round-trip time in milliseconds. */
    responseTime: number;
    /** Parsed JSON body. Throws (SyntaxError) on a non-JSON body, like Postman. */
    json(): unknown;
    /** Raw response body as text. */
    text(): string;
}
/** The `rq.execution.runRequest` callable (promise-only). */
export type ScriptRunRequest = (requestId: string, opts?: RunRequestOptions) => Promise<RunRequestResponse>;
/** Thrown when runRequest fails at the boundary — kind-tagged (gr-discriminated-errors-at-boundaries). */
export declare class RunRequestFailure extends Error {
    readonly kind: "run-request-failure";
    readonly reason: RunRequestErrorKind;
    constructor(reason: RunRequestErrorKind, message: string);
}
/**
 * Max runRequest calls per script (Postman parity — 10).
 *
 * Same value (10), different scope from `MAX_RUN_REQUEST_CALLS_PER_ROOT` in
 * `modules/runtime/src/run-request/resolver.ts` — per-script (resets each phase)
 * vs per-root-chain (across the whole run). Defense-in-depth; do NOT dedupe into
 * one constant.
 */
export declare const MAX_RUN_REQUEST_CALLS = 10;
/**
 * Builds the `rq.execution.runRequest` callable. `impl` is the per-engine host
 * round-trip (injected by the engine that wires runRequest in). The call count
 * is tracked per factory instance (per-script), enforcing the Postman 10-call
 * cap.
 */
export declare function createRunRequest(impl: RunRequestImpl): ScriptRunRequest;
