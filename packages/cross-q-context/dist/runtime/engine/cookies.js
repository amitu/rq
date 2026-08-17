/**
 * Sandbox-node cookie jar bridge (Phase E.21 / ADR-105).
 *
 * `createInMemoryCookieJarBridge` returns a per-execution `CookieJarBridge`
 * backed by an in-memory `Map<host, ScriptCookie>` (for intra-execution
 * `list` / `get` consistency) plus an ordered mutation log. The mutation log
 * is drained by `node-sandbox` after the script finishes and emitted on the
 * `result` event; the runtime replays it against the app-scoped
 * `CookieRepository` via `RuntimeConfig.onCookieJarMutation`.
 */
function keyOf(cookie) {
    return `${cookie.name}\t${cookie.path}`;
}
/**
 * Optional seed: cookies pre-fetched per allowed host before script execution.
 * Lets `bridge.list/get/getAll` return cookies persisted by previous executions
 * or captured Set-Cookie responses (ADR-105 read-side). Mutations made by the
 * current script layer on top of the seed via the in-memory store.
 */
export function createInMemoryCookieJarBridge(seed) {
    const store = new Map();
    const mutations = [];
    if (seed) {
        for (const { host, cookies } of seed) {
            const lcHost = host.toLowerCase();
            const perHost = new Map();
            for (const c of cookies) {
                // ScriptCookieSnapshot is structurally identical to ScriptCookie's
                // documented public fields; copy through to avoid VM-realm pollution.
                const cookie = {
                    name: c.name,
                    value: c.value,
                    domain: c.domain,
                    path: c.path,
                    secure: c.secure,
                    httpOnly: c.httpOnly,
                    expiry: c.expiry,
                };
                perHost.set(keyOf(cookie), cookie);
            }
            store.set(lcHost, perHost);
        }
    }
    const bridge = {
        list(host) {
            const perHost = store.get(host);
            return perHost ? Array.from(perHost.values()) : [];
        },
        upsert(host, cookie) {
            let perHost = store.get(host);
            if (!perHost) {
                perHost = new Map();
                store.set(host, perHost);
            }
            perHost.set(keyOf(cookie), cookie);
            mutations.push({
                kind: 'upsert',
                host,
                cookie: {
                    name: cookie.name,
                    value: cookie.value,
                    domain: cookie.domain,
                    path: cookie.path,
                    secure: cookie.secure,
                    httpOnly: cookie.httpOnly,
                    expiry: cookie.expiry,
                },
            });
        },
        remove(host, name, path) {
            const perHost = store.get(host);
            if (perHost)
                perHost.delete(keyOf({ name, path }));
            mutations.push({ kind: 'remove', host, name, path });
        },
        clear(host) {
            store.delete(host);
            mutations.push({ kind: 'clear', host });
        },
    };
    return {
        bridge,
        drainMutations() {
            return mutations.slice();
        },
    };
}
