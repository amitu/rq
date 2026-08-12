/**
 * `rq.cookies.jar()` — sandbox scripting surface (ADR-105).
 *
 * Mirrors Postman's `pm.cookies.jar()` 1:1: no host argument on `jar()`, each
 * method takes a URL (host is derived via `new URL().hostname`), every method
 * returns a Promise and also invokes an optional Node-style callback so the
 * two idioms interop (`await jar.set(...)` and
 * `jar.set(..., (err, cookie) => {...})` both work).
 *
 * Types live here only; the concrete sync bridge (`CookieJarBridge`) is
 * supplied by sandbox consumers (e.g. `modules/sandbox-node/src/cookies.ts`).
 * The bridge is called synchronously inside each method so mutations land in
 * the drain log even when user scripts fire-and-forget — the callback /
 * promise is notification-only. When true async completion tracking lands, no
 * API change is needed; only the internal timing of the bridge call moves.
 */
/** Cookie shape visible to user scripts. */
export interface ScriptCookie {
    readonly name: string;
    readonly value: string;
    readonly domain: string;
    readonly path: string;
    readonly secure: boolean;
    readonly httpOnly: boolean;
    readonly expiry: {
        readonly type: 'session';
    } | {
        readonly type: 'absolute';
        readonly date: string;
    };
}
/** Object form accepted by `jar.set(url, cookie, cb?)`. */
export interface ScriptCookieInput {
    readonly name: string;
    readonly value: string;
    readonly path?: string;
    readonly secure?: boolean;
    readonly httpOnly?: boolean;
    readonly expiry?: {
        readonly type: 'session';
    } | {
        readonly type: 'absolute';
        readonly date: string;
    };
}
/** Node-style callback: `(err, result)`. */
export type CookieCallback<T> = (err: Error | null, result?: T) => void;
/** Jar handle returned by `rq.cookies.jar()`. */
export interface ScriptCookieJar {
    set(url: string, name: string, value: string): Promise<ScriptCookie>;
    set(url: string, name: string, value: string, callback: CookieCallback<ScriptCookie>): Promise<ScriptCookie>;
    set(url: string, cookie: ScriptCookieInput): Promise<ScriptCookie>;
    set(url: string, cookie: ScriptCookieInput, callback: CookieCallback<ScriptCookie>): Promise<ScriptCookie>;
    get(url: string, name: string): Promise<string | undefined>;
    get(url: string, name: string, callback: CookieCallback<string | undefined>): Promise<string | undefined>;
    getAll(url: string): Promise<readonly ScriptCookie[]>;
    getAll(url: string, callback: CookieCallback<readonly ScriptCookie[]>): Promise<readonly ScriptCookie[]>;
    unset(url: string, name: string): Promise<void>;
    unset(url: string, name: string, callback: CookieCallback<void>): Promise<void>;
    clear(url: string): Promise<void>;
    clear(url: string, callback: CookieCallback<void>): Promise<void>;
}
/** `rq.cookies` namespace. */
export interface ScriptCookiesNamespace {
    jar(): ScriptCookieJar;
}
/**
 * Sync bridge between the sandbox and the persisted cookie jar. The jar
 * wrapper extracts `host` from the user-supplied URL and verifies the
 * allowlist before calling into the bridge — the bridge only ever sees
 * already-allowed hosts.
 */
export interface CookieJarBridge {
    list(host: string): readonly ScriptCookie[];
    upsert(host: string, cookie: ScriptCookie): void;
    remove(host: string, name: string, path: string): void;
    clear(host: string): void;
}
/**
 * Surfaced as promise-rejection / callback `err` when the URL's host is not
 * in the pre-bound allowlist (ADR-105). The runtime wraps this in an
 * `EntryError` with the kebab-case `CookieRuntimeError` payload on `details`
 * — see `packages/shared-types/src/runtime/errors.ts`.
 */
export declare class CookieJarHostDenied extends Error {
    readonly kind: "cookie-jar-host-denied";
    readonly reason: "not_granted";
    readonly host: string;
    readonly url: string;
    constructor(url: string, host: string);
}
/** Surfaced when `new URL(url)` throws. */
export declare class CookieJarInvalidUrl extends Error {
    readonly kind: "cookie-jar-invalid-url";
    readonly url: string;
    constructor(url: string);
}
/**
 * Builds the `rq.cookies` namespace. `hostAllowlist` comes from
 * `ScriptExecutionContext.hostAllowlist` (pre-bound by the SDK at dispatch
 * time); `bridge` is supplied by the sandbox consumer.
 */
export declare function createCookiesNamespace(params: {
    hostAllowlist: readonly string[];
    bridge: CookieJarBridge;
}): ScriptCookiesNamespace;
