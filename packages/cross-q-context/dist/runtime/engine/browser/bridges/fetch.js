// Defining module, not the engine barrel — see the note in browser/sandbox.ts.
import { createSafeBridge } from '../../isolated/safe-bridge-factory.js';
/**
 * Browser fetch bridge — **delegated only** (ADR-204; ADR-181/182 for the
 * delegation mechanism).
 *
 * ## Why this is a separate bridge rather than a reuse
 *
 * The Node bridge (`sandbox-node/src/isolated/bridges/fetch-bridge.ts`) supports
 * two paths: **delegated** (hand the request to `host.sendRequest`, the runtime's
 * single egress chokepoint) and **direct** (call the platform's real
 * `globalThis.fetch`, SSRF-guarded). The direct path is what makes that module
 * un-portable — its guard imports `node:dns/promises` and `node:net` to resolve a
 * hostname and classify the address before allowing egress.
 *
 * A browser cannot do that, and — more importantly — **should not need to**:
 *
 * 1. There is no DNS API to pre-resolve with, so the Node guard's central
 *    technique is simply unavailable.
 * 2. A direct browser `fetch` to an arbitrary API is CORS-blocked in the general
 *    case, so the direct path is not merely unguarded, it mostly does not work.
 * 3. ADR-202 already settled where a web send egresses: the client resolves and
 *    the **cloud replays**. A script's `fetch` egressing straight from the user's
 *    browser would quietly bypass that, losing the stable egress IP that is the
 *    entire point of the Cloud Agent.
 *
 * So rather than invent a browser SSRF guard for a path that should not exist,
 * the browser bridge has **no direct path at all**. Egress policy is enforced
 * downstream, where it already is: the relay and the cloud runner apply
 * `STRICT_SSRF_POLICY`.
 *
 * ## Making the invariant unrepresentable
 *
 * `host` is a **required** parameter here, unlike the Node bridge's optional one
 * (`gr-illegal-states-unrepresentable`). A browser fetch bridge without a host has
 * no meaningful behaviour, so it is a compile error rather than a runtime
 * fallback to something unsafe.
 *
 * ## Shared guest code
 *
 * The in-isolate shim is `FETCH_ISOLATE_SHIM`, reused verbatim from
 * `@requestly/sandbox-engine` — the guest-visible `fetch` is identical on both
 * surfaces. Only which host answers `__rq_fetch` differs.
 *
 * HARD INVARIANT: only copied data crosses. The host never returns a live
 * `Response`/`Headers`/stream — it returns a plain serializable record, and
 * `createSafeBridge`'s `Copyable` constraint makes anything else a compile error.
 */
/**
 * The delegation handler, exposed separately from the bridge so it is directly
 * testable. `SafeBridge` deliberately exposes only `{ name, install }` — there is
 * no handler to reach through it, by design — so without this seam the only way to
 * test the delegation behaviour would be to stand up a real QuickJS context.
 */
export function createBrowserFetchHandler(host) {
    return async (req) => {
        // Assignable to the shared-types interface both ways — no cast.
        const request = req;
        const envelope = await host.sendRequest(request);
        if (!envelope.ok) {
            // The factory marshals a thrown Error's MESSAGE across the edge, so the
            // guest's `fetch` rejects with this text — matching the Node bridge, whose
            // guest-facing behaviour is driven by the same unchanged shim.
            throw new Error(envelope.error.message);
        }
        const response = envelope.response;
        return response;
    };
}
export function createBrowserFetchBridge(host) {
    // `{ async: true }` installs via `ctx.newAsyncifiedFunction`, so the guest sees
    // the resolved value returned directly rather than a guest promise.
    return createSafeBridge('__rq_fetch', createBrowserFetchHandler(host), { async: true });
}
