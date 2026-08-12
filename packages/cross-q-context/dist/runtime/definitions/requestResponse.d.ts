/**
 * Request/Response builders and assertion chain for the rq namespace.
 *
 * Exposes a curated allowlist of request/response properties to user scripts (ADR-054).
 * Protocol-specific interfaces per ADR-136: HTTP/GraphQL and gRPC each get their own
 * ScriptRequest/ScriptResponse shape with native properties and assertion chain.
 * Internal types (KeyValuePair metadata, HttpBody variants) are not leaked.
 */
import { EntryType } from './_deps.js';
import type { GrpcScriptResponse as GrpcScriptResponseData, GraphQLResponse, HttpResponse, ParsedGrpcRequest, ParsedGraphQLRequest, ParsedHttpRequest } from './_deps.js';
import type { GrpcStreamMessage, RequestHeaderMutation, ScriptMessageInput } from './_deps.js';
export interface AssertionLibs {
    chai: {
        expect: unknown;
    };
    lodash: {
        get: (obj: unknown, path: string) => unknown;
        isEqual: (a: unknown, b: unknown) => boolean;
    };
    ajv: unknown;
    /**
     * Handlebars compiler for `rq.visualizer.set()` (ADR-202). Delivered via the
     * `VENDOR_IIFES` vendor bundle: the Developer engine evals the Handlebars IIFE
     * in the VM realm and hands the resulting compiler in here (structurally the
     * same channel as `chai`/`lodash`/`ajv`). The Safe engine does not use this —
     * its hand-written shim requires Handlebars in-guest.
     */
    handlebars: {
        compile: (template: string) => (context?: unknown) => string;
    };
}
/**
 * Mutable header facade on `rq.request.headers` (ADR-167). Read accessors see the
 * script's own in-flight writes; mutators record ops onto the shared collector
 * AND update the working copy. Header names are matched case-insensitively.
 */
export interface ScriptRequestHeaders {
    add(header: {
        key: string;
        value: string;
    }): void;
    upsert(header: {
        key: string;
        value: string;
    }): void;
    remove(name: string): void;
    /**
     * Removes ALL headers (Postman `HeaderList.clear()` parity, RQ-3720). Takes no
     * argument; any value passed is ignored — Postman's `clear()` always clears the
     * whole list, never a single header.
     */
    clear(): void;
    has(name: string): boolean;
    get(name: string): string | undefined;
    all(): Record<string, string>;
}
export interface ScriptHttpRequest {
    readonly url: string;
    readonly method: string;
    /** Mutable header facade (ADR-167). Mutations apply pre-request only. */
    readonly headers: ScriptRequestHeaders;
    readonly queryParams: Readonly<Record<string, string>>;
    readonly body: string | undefined;
    /** SDK-style aliases (Postman `pm.request.addHeader/…`) — delegate to `headers.*`. */
    addHeader(header: {
        key: string;
        value: string;
    }): void;
    removeHeader(name: string): void;
    upsertHeader(header: {
        key: string;
        value: string;
    }): void;
    toJSON(): {
        url: string;
        method: string;
        headers: Record<string, string>;
        queryParams: Record<string, string>;
        body: string | undefined;
    };
}
/** Sink the header facade records mutations onto (ADR-167). */
export interface RequestMutationCollector {
    headers: RequestHeaderMutation[];
}
export interface StatusAssertions {
    readonly ok: void;
    readonly success: void;
    readonly accepted: void;
    readonly info: void;
    readonly redirection: void;
    readonly clientError: void;
    readonly badRequest: void;
    readonly unauthorized: void;
    readonly forbidden: void;
    readonly notFound: void;
    readonly rateLimited: void;
    readonly serverError: void;
    readonly error: void;
}
export interface HaveAssertions {
    status(expected: number | string): void;
    header(name: string): void;
    body(expected: string): void;
    jsonBody(): void;
    jsonBody(path: string): void;
    jsonBody(path: string, value: unknown): void;
    jsonSchema(schema: object, options?: object): void;
}
interface NegatedHttpResponseAssertions {
    readonly be: StatusAssertions;
    readonly have: HaveAssertions;
}
export interface ResponseAssertions {
    readonly be: StatusAssertions;
    readonly have: HaveAssertions;
    readonly not: NegatedHttpResponseAssertions;
}
/**
 * Read-only header facade on `rq.response.headers` (RQ-4233). A HYBRID: the wire
 * headers are present as own-enumerable string-keyed data properties (so bracket
 * access `headers['Content-Type']`, `Object.keys`, spread, and `JSON.stringify`
 * all keep working exactly as before this facade existed), PLUS non-enumerable
 * `get`/`has`/`all` methods for case-insensitive lookup (Postman parity for
 * `pm.response.headers.get()`). Mirrors the `rq.sendRequest` response-header
 * shape. The methods are non-enumerable so they never pollute `Object.keys` /
 * `JSON.stringify`. `all()` returns a fresh copy preserving original wire casing.
 */
export interface ScriptResponseHeaders {
    get(name: string): string | undefined;
    has(name: string): boolean;
    all(): Record<string, string>;
    [key: string]: string | ((name: string) => string | undefined) | ((name: string) => boolean) | (() => Record<string, string>) | undefined;
}
/**
 * Raw response body bytes exposed on `rq.response.stream` (Postman `pm.response.stream`
 * parity). A Buffer-like handle: `.toString(encoding)` re-encodes the bytes, `.length`
 * is the byte length. Backed by a real Node `Buffer` in the Developer engine and by the
 * Safe-engine `Buffer` shim (buffer-bridge) in the Safe engine — both satisfy this shape.
 * Typed as a minimal self-contained interface (not Node's `Buffer`) so the generated
 * editor `.d.ts` needs no `@types/node`.
 */
export interface ScriptResponseStream {
    toString(encoding?: string): string;
    readonly length: number;
    readonly [index: number]: number;
}
export interface ScriptHttpResponse {
    readonly status: number;
    readonly code: number;
    readonly statusText: string;
    readonly headers: ScriptResponseHeaders;
    readonly body: string;
    /**
     * The raw response body bytes as a Buffer-like handle (Postman `pm.response.stream`
     * parity). Lazily built from `body` + `bodyEncoding`, so scripts that never read it
     * pay nothing. Common use: `rq.response.stream.toString('base64')` to embed a binary
     * response (image/PDF) in a visualization.
     */
    readonly stream: ScriptResponseStream;
    /**
     * How `body` encodes the original bytes (ADR-153): `'base64'` for binary
     * responses (images, PDFs, ...), `'utf8'` for text. Absent on responses
     * persisted before ADR-153 — treat absent as `'utf8'`.
     */
    readonly bodyEncoding?: 'utf8' | 'base64';
    readonly time: number;
    readonly responseTime: number;
    readonly size: number;
    readonly to: ResponseAssertions;
    json(): any;
    text(): string;
    toJSON(): {
        status: number;
        statusText: string;
        headers: Record<string, string>;
        body: string;
        bodyEncoding?: 'utf8' | 'base64';
        time: number;
    };
}
export interface ScriptGrpcRequest {
    readonly url: string;
    readonly methodPath: string;
    readonly metadata: Readonly<Record<string, string>>;
    readonly message: string;
    readonly auth: Readonly<Record<string, unknown>> | null;
    toJSON(): {
        url: string;
        methodPath: string;
        metadata: Record<string, string>;
        message: string;
    };
}
export interface GrpcStatusAssertions {
    readonly ok: void;
    readonly success: void;
    readonly error: void;
}
export interface GrpcHaveAssertions {
    status(expected: number): void;
    metadata(name: string): void;
    trailer(name: string): void;
    message(expected: string): void;
    jsonMessage(): void;
    jsonMessage(path: string): void;
    jsonMessage(path: string, value: unknown): void;
    jsonSchema(schema: object, options?: object): void;
}
interface NegatedGrpcResponseAssertions {
    readonly be: GrpcStatusAssertions;
    readonly have: GrpcHaveAssertions;
}
export interface GrpcResponseAssertions {
    readonly be: GrpcStatusAssertions;
    readonly have: GrpcHaveAssertions;
    readonly not: NegatedGrpcResponseAssertions;
}
export interface ScriptGrpcResponse {
    readonly statusCode: number;
    readonly statusMessage: string;
    /** Read facade (RQ-4233) — get/has/all, case-insensitive. Matches HTTP `rq.response.headers`. */
    readonly metadata: ScriptResponseHeaders;
    /** Read facade (RQ-4233) — get/has/all, case-insensitive. Matches HTTP `rq.response.headers`. */
    readonly trailers: ScriptResponseHeaders;
    readonly messages: readonly GrpcStreamMessage[];
    readonly responseTime: number;
    readonly to: GrpcResponseAssertions;
    json(): unknown;
    text(): string;
    toJSON(): {
        statusCode: number;
        statusMessage: string;
        metadata: Record<string, string>;
        trailers: Record<string, string>;
        messages: GrpcStreamMessage[];
        responseTime: number;
    };
}
/** Union of all protocol-specific request types. Prefer `ScriptHttpRequest` or `ScriptGrpcRequest` when the protocol is known. */
export type ScriptRequest = ScriptHttpRequest | ScriptGrpcRequest;
/** Union of all protocol-specific response types. Prefer `ScriptHttpResponse` or `ScriptGrpcResponse` when the protocol is known. */
export type ScriptResponse = ScriptHttpResponse | ScriptGrpcResponse;
/**
 * Auth context extracted from the entry for gRPC script request building.
 * gRPC auth lives on the entry (`GrpcApiEntry.auth`), not the request.
 */
export interface GrpcBuildContext {
    auth: Record<string, unknown> | null;
}
export declare function buildScriptRequest(request: ParsedHttpRequest | ParsedGraphQLRequest | ParsedGrpcRequest, entryType: EntryType, grpcContext?: GrpcBuildContext, collector?: RequestMutationCollector): ScriptRequest;
export declare function buildScriptResponse(response: HttpResponse | GraphQLResponse | GrpcScriptResponseData, libs: AssertionLibs, entryType: EntryType): ScriptResponse;
/** Assertions on the message currently being handled. */
export interface MessageHaveAssertions {
    /** The message text includes `expected`. */
    body(expected: string): void;
    /**
     * The message parses as JSON. With a dotted `path`, that path exists; with a
     * `value` too, it equals `value`.
     */
    jsonBody(...args: [] | [string] | [string, unknown]): void;
}
export interface MessageBeAssertions {
    /** The message parses as JSON. */
    readonly json: undefined;
    /** The message is non-empty. */
    readonly present: undefined;
}
interface NegatedMessageAssertions {
    readonly be: MessageBeAssertions;
    readonly have: MessageHaveAssertions;
}
export interface MessageAssertions {
    readonly be: MessageBeAssertions;
    readonly have: MessageHaveAssertions;
    readonly not: NegatedMessageAssertions;
}
/**
 * One streamed message, as scripts see it (`rq.message`).
 *
 * `index` is the message's position in the CONNECTION, not in the drain batch —
 * batching is an implementation detail the user never observes, and the index is
 * what ties an assertion or an error back to the message they can see.
 */
export interface ScriptMessage {
    /** Zero-based position in the connection. Users see `msg index + 1`. */
    readonly index: number;
    /** Arrival time, epoch millis. */
    readonly timestamp: number;
    /** Raw payload as delivered by the transport. */
    readonly data: string;
    readonly to: MessageAssertions;
    /** Parsed payload. Throws if the message is not valid JSON. */
    json(): unknown;
    /** Raw payload — the symmetric peer of `json()`, matching `rq.response.text()`. */
    text(): string;
    toJSON(): {
        index: number;
        timestamp: number;
        data: string;
    };
}
/** Build the `rq.message` surface for one iteration of an on-message script. */
export declare function buildScriptMessage(message: ScriptMessageInput, libs: AssertionLibs): ScriptMessage;
export {};
