/**
 * fetch-bridge — Safe-mode controlled `fetch` (NEEDS_BRIDGE, ADR-010 §34).
 *
 * The isolate has no `fetch`. This bridge exposes a controlled one as an ASYNC
 * copy-in/copy-out host callback: the isolate hands a serialized request
 * (method, url, headers, body), the host performs the request, and returns a
 * serialized response (status, headers, body). This unlocks axios-via-fetch and
 * got-via-fetch's data path (§5.2.4).
 *
 * TWO HOST PATHS (ADR-181/182, RQ-4312 — mirrors `run-request-bridge`):
 * - **Delegated** — when a `SendRequestHost` is injected, the handler hands the
 *   serialized request to `host.sendRequest(req)` and returns the envelope's
 *   response, so a script's `fetch` flows through the runtime's fetcher (the
 *   single egress chokepoint) rather than a direct host socket. An `ok:false`
 *   envelope becomes a thrown error the isolate shim rejects on.
 * - **Direct** — when no host is injected, the handler falls back to the
 *   platform's real `globalThis.fetch` (CLI / in-process parity, where localhost is
 *   legitimate). This is the historical behavior; desktop and the scheduled-run
 *   runner both inject a host now (ADR-204).
 *
 * HARD INVARIANT: only copied data crosses. The host never hands back a live
 * `Response`, `Headers`, or stream object — it drains the body to a string and
 * returns a plain serializable record. The live `SendRequestHost` stays host-side
 * and no reference (function / Response / host) ever enters the QuickJS realm.
 * `createSafeBridge`'s `Copyable` type constraint makes a non-copied handler a
 * COMPILE ERROR, and the paired `*.containment.test.ts` proves no host realm is
 * reachable through the bridge.
 *
 * The handler's I/O is typed with LOCAL `type` mirrors of the boundary shapes
 * (`FetchRequestData` / `FetchResponseData`) — exactly as `run-request-bridge`
 * declares its descriptor/envelope locally. This is required because
 * `createSafeBridge`'s `Copyable` constraint is `{ readonly [key: string]: Copyable }`,
 * which a `type` alias structurally satisfies but the equivalent `interface`
 * (`SerializedFetchRequest`/`SerializedFetchResponse` in shared-types) does NOT.
 * The local types are STRUCTURALLY identical to the imported interfaces and
 * assignable both ways with NO cast — a type-shape adapter, not a widening of the
 * safety contract.
 */
import { dlog } from './isolated/debug-log.js';
import { describeDelegationFailure } from './delegated-fetch.js';
import { createSafeBridge } from './isolated/safe-bridge-factory.js';
import { CLIENT_SSRF_POLICY, createGuardedFetch } from './ssrf-guard.js';
/**
 * Direct path: perform the request with an SSRF-guarded `fetch` (RQ-3902). The
 * isolate has no host realm, but the host still makes the actual network call,
 * so the metadata/internal-range denylist applies here too.
 */
async function directFetch(req, guardedFetch) {
    dlog('fetch', 'host fetch START (direct)', { method: req.method, url: req.url });
    const res = await guardedFetch(req.url, {
        method: req.method,
        headers: req.headers,
        body: req.body,
    });
    dlog('fetch', 'host fetch response received', { status: res.status });
    const headers = {};
    res.headers.forEach((value, key) => {
        headers[key] = value;
    });
    // Drain the body to a copied string — never hand back the live Response/stream.
    // The direct path decodes as text, so the copied body is always utf8.
    const body = await res.text();
    dlog('fetch', 'host fetch body drained → returning to guest', { bodyLen: body.length });
    return { status: res.status, statusText: res.statusText, headers, body, bodyEncoding: 'utf8' };
}
/**
 * The host-side async fetch bridge installed as `__rq_fetch`. When `host` is
 * provided (RQ-4312 slice A) the request is delegated to the runtime fetcher via
 * `host.sendRequest` — the single egress chokepoint, where egress policy is
 * enforced downstream (slice B). Otherwise it falls back to a direct
 * `globalThis.fetch` that is SSRF-guarded with `policy` (RQ-3902): the isolate
 * has no host realm, but the host still makes the call, so the metadata/internal-
 * range denylist applies. Defaults to the client posture; a server host passes
 * STRICT_SSRF_POLICY to also block private ranges on the direct path.
 */
export function createFetchBridge(host, policy = CLIENT_SSRF_POLICY) {
    const guardedFetch = createGuardedFetch(fetch, policy);
    // Copyable-in / Copyable-out: request → response. createSafeBridge's type
    // constraint makes a non-Copyable handler a COMPILE ERROR (the HARD INVARIANT).
    const handler = async (req) => {
        if (host === undefined) {
            return directFetch(req, guardedFetch);
        }
        dlog('fetch', 'host fetch START (delegated)', { method: req.method, url: req.url });
        // The local request type is assignable to the shared-types interface
        // (structurally identical) — no cast.
        const request = req;
        const envelope = await host.sendRequest(request);
        if (!envelope.ok) {
            dlog('fetch', 'delegated fetch envelope ok:false → rejecting guest fetch', {
                kind: envelope.error.kind,
                fetchKind: envelope.error.fetchKind,
                code: envelope.error.code,
            });
            // The factory marshals a thrown Error's MESSAGE across the edge, so the
            // guest `fetch` rejects with this message — meaning the message is the ONLY
            // channel a script can see. Compose it from the static message plus the
            // envelope's BOUNDED discriminants so the script author can tell a corporate
            // TLS-interception proxy (`tls_handshake: SELF_SIGNED_CERT_IN_CHAIN`) from a
            // dead port (`network: ECONNREFUSED`). Both parts are closed unions /
            // platform codes, never request data — `gr-static-error-messages` allows
            // bounded types.
            throw new Error(describeDelegationFailure(envelope.error));
        }
        // The host returns the interface response, assignable back to the local
        // copy-out type (structurally identical) — no cast.
        const response = envelope.response;
        dlog('fetch', 'delegated fetch response → returning to guest', { status: response.status });
        return response;
    };
    return createSafeBridge('__rq_fetch', handler, { async: true });
}
/**
 * The guest-realm half of this bridge lives in `@requestly/sandbox-engine` (ADR-217):
 * the shim text is identical on every host, only the host callback above differs.
 * Re-exported here so existing import sites are unchanged.
 */
export { FETCH_ISOLATE_SHIM } from './isolated/shims/fetch.shim.js';
