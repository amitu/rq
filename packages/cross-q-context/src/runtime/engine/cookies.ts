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

import type { CookieJarBridge, ScriptCookie } from './host-types.js';
import type { CookieJarMutation, CookieJarSeed } from './host-types.js';

function keyOf(cookie: Pick<ScriptCookie, 'name' | 'path'>): string {
  return `${cookie.name}\t${cookie.path}`;
}

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
export function createInMemoryCookieJarBridge(seed?: CookieJarSeed): CookieJarBridgeHandle {
  const store = new Map<string, Map<string, ScriptCookie>>();
  const mutations: CookieJarMutation[] = [];

  if (seed) {
    for (const { host, cookies } of seed) {
      const lcHost = host.toLowerCase();
      const perHost = new Map<string, ScriptCookie>();
      for (const c of cookies) {
        // ScriptCookieSnapshot is structurally identical to ScriptCookie's
        // documented public fields; copy through to avoid VM-realm pollution.
        const cookie: ScriptCookie = {
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

  const bridge: CookieJarBridge = {
    list(host: string): readonly ScriptCookie[] {
      const perHost = store.get(host);
      return perHost ? Array.from(perHost.values()) : [];
    },

    upsert(host: string, cookie: ScriptCookie): void {
      let perHost = store.get(host);
      if (!perHost) {
        perHost = new Map<string, ScriptCookie>();
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

    remove(host: string, name: string, path: string): void {
      const perHost = store.get(host);
      if (perHost) perHost.delete(keyOf({ name, path }));
      mutations.push({ kind: 'remove', host, name, path });
    },

    clear(host: string): void {
      store.delete(host);
      mutations.push({ kind: 'clear', host });
    },
  };

  return {
    bridge,
    drainMutations(): readonly CookieJarMutation[] {
      return mutations.slice();
    },
  };
}
