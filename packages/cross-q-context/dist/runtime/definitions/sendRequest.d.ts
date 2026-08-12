/**
 * `rq.sendRequest()` — sandbox scripting surface (ADR-153, Postman `pm.sendRequest` parity).
 *
 * Wraps the already-injected `fetch` VM global (see GLOBAL_NAMES) so a user
 * script can issue an HTTP sub-request and use the response in the same
 * execution. `fetch` is passed in (default `globalThis.fetch`) so the module
 * stays platform-agnostic and unit-testable with a mock.
 *
 * Dual form (D-11): every call returns a Promise AND invokes an optional
 * Node-style callback, so `await rq.sendRequest(...)` and
 * `rq.sendRequest(..., (err, res) => {})` both work — mirroring `cookies.ts`.
 *
 * Error contract (TB EC-15/EC-16): a transport/network failure rejects /
 * fires the callback with a `kind`-tagged `SendRequestError`; an HTTP 4xx/5xx
 * is NOT an error — `err` is null and the response is delivered. The raw
 * `fetch` `TypeError` is wrapped, never allowed to escape bare.
 */
/** Headers accepted on the request: object form OR Postman's array form. */
export type SendRequestHeaders = Record<string, string> | ReadonlyArray<{
    key: string;
    value: string;
    disabled?: boolean;
}>;
/** Request body — discriminated on `mode` (raw | urlencoded in v1). */
export type SendRequestBody = {
    mode: 'raw';
    raw: string;
} | {
    mode: 'urlencoded';
    urlencoded: ReadonlyArray<{
        key: string;
        value: string;
        disabled?: boolean;
    }>;
};
/** Config form of the request. */
export interface SendRequestConfig {
    url: string;
    method?: string;
    header?: SendRequestHeaders;
    body?: SendRequestBody;
}
/** A bare URL string or a full config. */
export type SendRequestInput = string | SendRequestConfig;
/**
 * Response headers exposed to scripts. Mirrors Postman's `HeaderList`: a
 * case-insensitive `get(name)` plus index access, so both
 * `res.headers.get('content-type')` (pasted Postman scripts) and
 * `res.headers['content-type']` (new scripts) work.
 */
export interface ScriptHeaderList {
    get(name: string): string | undefined;
    [key: string]: string | ((name: string) => string | undefined) | undefined;
}
/** Postman-shaped response object (ADR-153 §API Signature). */
export interface SendRequestResponse {
    /** Numeric HTTP status, e.g. 200. */
    code: number;
    /** HTTP status text, e.g. "OK". */
    status: string;
    headers: ScriptHeaderList;
    /** Round-trip time in milliseconds. */
    responseTime: number;
    /** Parsed JSON body. Throws (SyntaxError) on a non-JSON body, like Postman. */
    json(): unknown;
    /** Raw response body as text. */
    text(): string;
}
/** Node-style callback: `(err, response)`. */
export type SendRequestCallback = (err: SendRequestErrors | null, response?: SendRequestResponse) => void;
/** The `rq.sendRequest` callable (dual form). */
export interface ScriptSendRequest {
    (input: SendRequestInput): Promise<SendRequestResponse>;
    (input: SendRequestInput, callback: SendRequestCallback): Promise<SendRequestResponse>;
}
/** Surfaced when the request config has no usable URL. */
export declare class SendRequestInvalidArgs extends Error {
    readonly kind: "send-request-invalid-args";
    constructor();
}
/** Surfaced when the underlying fetch fails at the transport level (EC-15). */
export declare class SendRequestError extends Error {
    readonly kind: "send-request-network-error";
    readonly url: string;
    constructor(url: string, cause: unknown);
}
/** Discriminated union of every error `rq.sendRequest` can surface. */
export type SendRequestErrors = SendRequestInvalidArgs | SendRequestError;
/**
 * Builds the `rq.sendRequest` callable. `fetchImpl` defaults to the injected
 * `fetch` global; tests pass a mock.
 */
export declare function createSendRequest(fetchImpl?: typeof globalThis.fetch): ScriptSendRequest;
