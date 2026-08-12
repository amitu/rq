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
/**
 * Surfaced as promise-rejection / callback `err` when the URL's host is not
 * in the pre-bound allowlist (ADR-105). The runtime wraps this in an
 * `EntryError` with the kebab-case `CookieRuntimeError` payload on `details`
 * — see `packages/shared-types/src/runtime/errors.ts`.
 */
export class CookieJarHostDenied extends Error {
    kind = 'cookie-jar-host-denied';
    reason = 'not_granted';
    host;
    url;
    constructor(url, host) {
        super(`CookieStore: programmatic access to "${host}" is denied.`);
        this.name = 'CookieJarHostDenied';
        this.host = host;
        this.url = url;
    }
}
/** Surfaced when `new URL(url)` throws. */
export class CookieJarInvalidUrl extends Error {
    kind = 'cookie-jar-invalid-url';
    url;
    constructor(url) {
        super(`CookieStore: invalid URL "${url}".`);
        this.name = 'CookieJarInvalidUrl';
        this.url = url;
    }
}
function hostFromUrl(url) {
    try {
        return new URL(url).hostname.toLowerCase();
    }
    catch {
        return null;
    }
}
function toScriptCookie(host, input) {
    return {
        name: input.name,
        value: input.value,
        domain: host,
        path: input.path ?? '/',
        secure: input.secure ?? false,
        httpOnly: input.httpOnly ?? false,
        expiry: input.expiry ?? { type: 'session' },
    };
}
function fireCallback(callback, err, result) {
    if (!callback)
        return;
    // Next-microtask so a synchronous throw inside the user's callback can't
    // tear through our sync bridge call-site. Matches Postman / Node async
    // conventions (callback is never invoked before the caller's `then()` can
    // attach).
    queueMicrotask(() => callback(err, result));
}
/**
 * Builds the `rq.cookies` namespace. `hostAllowlist` comes from
 * `ScriptExecutionContext.hostAllowlist` (pre-bound by the SDK at dispatch
 * time); `bridge` is supplied by the sandbox consumer.
 */
export function createCookiesNamespace(params) {
    const allowed = new Set(params.hostAllowlist.map((h) => h.toLowerCase()));
    const { bridge } = params;
    function resolveHost(url) {
        const host = hostFromUrl(url);
        if (host === null)
            return { error: new CookieJarInvalidUrl(url) };
        if (!allowed.has(host))
            return { error: new CookieJarHostDenied(url, host) };
        return { host };
    }
    const jar = {
        set(url, nameOrCookie, valueOrCallback, maybeCallback) {
            let input;
            let callback;
            if (typeof nameOrCookie === 'string') {
                // Value form: set(url, name, value, cb?)
                const value = typeof valueOrCallback === 'string' ? valueOrCallback : '';
                input = { name: nameOrCookie, value };
                callback = maybeCallback;
            }
            else {
                // Object form: set(url, cookieInput, cb?)
                input = nameOrCookie;
                callback = typeof valueOrCallback === 'function' ? valueOrCallback : undefined;
            }
            const resolved = resolveHost(url);
            if ('error' in resolved) {
                fireCallback(callback, resolved.error);
                return Promise.reject(resolved.error);
            }
            const cookie = toScriptCookie(resolved.host, input);
            bridge.upsert(resolved.host, cookie);
            fireCallback(callback, null, cookie);
            return Promise.resolve(cookie);
        },
        get(url, name, callback) {
            const resolved = resolveHost(url);
            if ('error' in resolved) {
                fireCallback(callback, resolved.error);
                return Promise.reject(resolved.error);
            }
            const value = bridge.list(resolved.host).find((c) => c.name === name)?.value;
            fireCallback(callback, null, value);
            return Promise.resolve(value);
        },
        getAll(url, callback) {
            const resolved = resolveHost(url);
            if ('error' in resolved) {
                fireCallback(callback, resolved.error);
                return Promise.reject(resolved.error);
            }
            const cookies = bridge.list(resolved.host);
            fireCallback(callback, null, cookies);
            return Promise.resolve(cookies);
        },
        unset(url, name, callback) {
            const resolved = resolveHost(url);
            if ('error' in resolved) {
                fireCallback(callback, resolved.error);
                return Promise.reject(resolved.error);
            }
            bridge.remove(resolved.host, name, '/');
            fireCallback(callback, null);
            return Promise.resolve();
        },
        clear(url, callback) {
            const resolved = resolveHost(url);
            if ('error' in resolved) {
                fireCallback(callback, resolved.error);
                return Promise.reject(resolved.error);
            }
            bridge.clear(resolved.host);
            fireCallback(callback, null);
            return Promise.resolve();
        },
    };
    return {
        jar() {
            return jar;
        },
    };
}
