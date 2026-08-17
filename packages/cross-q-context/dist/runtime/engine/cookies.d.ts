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
import type { CookieJarBridge } from './host-types.js';
import type { CookieJarMutation, CookieJarSeed } from './host-types.js';
/**
 * Bridge handle with drain. Returned to `node-sandbox.runScript` so it can
 * drain the mutation log after the script completes and include it in the
 * `result` event.
 */
export interface CookieJarBridgeHandle {
    readonly bridge: CookieJarBridge;
    /** Returns the ordered mutation log and leaves the bridge's internal state intact. */
    drainMutations(): readonly CookieJarMutation[];
}
/**
 * Optional seed: cookies pre-fetched per allowed host before script execution.
 * Lets `bridge.list/get/getAll` return cookies persisted by previous executions
 * or captured Set-Cookie responses (ADR-105 read-side). Mutations made by the
 * current script layer on top of the seed via the in-memory store.
 */
export declare function createInMemoryCookieJarBridge(seed?: CookieJarSeed): CookieJarBridgeHandle;
