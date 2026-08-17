// The host-side fetch bridge (`__rq_fetch`) — the delegated half of the guest FETCH_ISOLATE_SHIM.
//
// cross-q-context never performs the network call itself: it marshals the request out to a
// host-injected SendRequestFn and marshals the response back, so egress/SSRF policy stays entirely
// host-side (the app's runtime, the rq CLI's fetcher, …). If the host callback rejects, the guest
// `fetch` rejects with the error's message — so throw a bounded message, never request data.
import { createSafeBridge } from './isolated/safe-bridge-factory.js';
/** Build the `__rq_fetch` async bridge that delegates each request to `sendRequest`. */
export function createFetchBridge(sendRequest) {
    const handler = async (req) => sendRequest(req);
    return createSafeBridge('__rq_fetch', handler, { async: true });
}
