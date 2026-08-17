/**
 * ssrf-guard — SSRF protection for the sandbox `fetch` capability (RQ-3902 / RQ-3921).
 *
 * User-authored pre/post-request scripts get a `fetch`: the Developer engine
 * (`node-sandbox.ts`) injects the host `fetch` as a VM global, and the Safe
 * engine exposes one via `fetch-bridge.ts`. Both perform the network call in the
 * host (Node) realm, so an untrusted script can reach any endpoint the process
 * can — including the cloud metadata server (`169.254.169.254`,
 * `metadata.google.internal`) which returns the instance's service-account token.
 * On a server host (the scheduled-run-runner Cloud Run/GCE executor) that is a
 * full service-account credential-theft pivot (CWE-918, the RQ-3921 chain-breaker).
 *
 * This module wraps a base `fetch` so that, before every network hop (including
 * each redirect target), the request is validated:
 *   - only `http:` / `https:` schemes are allowed;
 *   - the cloud-metadata hostnames are blocked outright;
 *   - the hostname is resolved and EVERY resolved IP is checked — link-local
 *     (IMDS) is always blocked; the broader private ranges are blocked unless the
 *     policy opts in (`allowPrivateNetwork`, for desktop/CLI localhost testing).
 *
 * `createGuardedFetch`'s residual (originally logged as a follow-up under
 * RQ-3921): validation resolves DNS then hands the hostname to the base `fetch`,
 * which re-resolves — a sub-second DNS-rebinding flip between the two lookups is a
 * TOCTOU window that wrapper does not close, and redirect hops the transport
 * follows internally bypass it entirely.
 *
 * {@link createGuardedLookup} closes both **for hostname targets**, by moving the check to
 * the moment a name is resolved for a connection. The original note said this needed "an
 * undici dispatcher" and was therefore out of scope here; that was too pessimistic — a
 * Node `LookupFunction` is plain `node:dns`, so this module gains no transport dependency
 * and the caller wires it into whatever transport it owns.
 *
 * It is NOT a complete redirect control on its own: an IP-literal host never reaches a
 * custom `lookup`, because there is no name to resolve. A host that follows redirects
 * must ALSO check the peer address at connect time (pass {@link isAddressBlocked} to
 * `@requestly/fetcher`'s `HttpFetcherOptions.connectAddressGuard`). Note that
 * {@link assertUrlAllowed} does handle IP literals — URL-level validation is *stronger*
 * than the lookup for that one case, which is why the two are complementary rather than
 * redundant.
 */
import type { LookupFunction } from 'node:net';
/**
 * SSRF posture for a sandbox host.
 *
 * `allowPrivateNetwork: false` (server hosts, fail-closed default) blocks all
 * private/loopback/reserved ranges in addition to the always-blocked
 * link-local/metadata surface. `allowPrivateNetwork: true` (desktop/CLI) blocks
 * only the always-blocked surface, so scripts can still reach `localhost`/LAN
 * APIs — on those hosts the script runs on the user's own machine with the
 * user's own credentials, so there is no service-account token to steal.
 */
export interface SsrfPolicy {
    readonly allowPrivateNetwork: boolean;
}
/** Fail-closed posture for server-side hosts — blocks every private range. */
export declare const STRICT_SSRF_POLICY: SsrfPolicy;
/** Lenient posture for client hosts — preserves `localhost`/LAN access. */
export declare const CLIENT_SSRF_POLICY: SsrfPolicy;
/** Static, bounded reasons a request was refused — never interpolates untrusted input. */
export type SsrfBlockReason = 'scheme' | 'metadata-host' | 'blocked-address';
/**
 * Thrown when the guard refuses a request. The message is static
 * (`gr-static-error-messages`); the specific reason is a bounded discriminant.
 */
export declare class SsrfBlockedError extends Error {
    readonly reason: SsrfBlockReason;
    constructor(reason: SsrfBlockReason);
}
/** Whether a resolved IP address must be refused under the given policy. */
export declare function isAddressBlocked(ip: string, policy: SsrfPolicy): boolean;
/**
 * Validates a single URL against the policy: scheme allowlist, metadata-host
 * denylist, then resolved-IP checks. Throws {@link SsrfBlockedError} when refused.
 */
export declare function assertUrlAllowed(rawUrl: string, policy: SsrfPolicy): Promise<void>;
/**
 * A connect-time SSRF guard, shaped as a Node `LookupFunction` for undici's
 * connector (`Agent({ connect: { lookup } })` / `buildConnector({ lookup })`).
 *
 * ### Why this exists alongside `assertUrlAllowed`
 *
 * `assertUrlAllowed` validates a **URL**: it resolves the hostname, checks the
 * addresses, and then hands the *hostname* to the transport, which resolves it
 * again. Two consequences, both called out in this module's header as the
 * documented residual (RQ-3921):
 *
 *  1. **TOCTOU.** A name that answers a public address for the guard's lookup and
 *     a link-local one for the transport's defeats URL-level validation entirely.
 *  2. **Redirect hops are invisible.** When the transport follows a 3xx itself,
 *     hops 2..n never pass through any URL-level check — which is why hosts that
 *     wanted fail-closed redirects had to disable redirect following outright.
 *
 * Guarding at the lookup closes both **for hostname targets**. It does NOT cover a host
 * that is already an IP literal: Node's `net.connect` / `tls.connect` skip a custom
 * `lookup` entirely when there is no name to resolve, so a `Location:
 * http://169.254.169.254/…` hop would connect without this function ever being called.
 * A caller that follows redirects therefore needs a second, connect-level check on the
 * peer address — `@requestly/fetcher`'s `HttpFetcherOptions.connectAddressGuard` is that
 * seam, and `isAddressBlocked` is what to pass it. This function alone is not a complete
 * redirect control.
 *
 * For the hostname case it is strictly less machinery than the alternative: no
 * hand-rolled redirect loop, no second copy of Fetch redirect semantics (method
 * downgrade on 301/302/303, cross-origin credential stripping), and the
 * transport's own interceptors — Set-Cookie capture, redirect-hop capture — keep
 * working untouched.
 *
 * The header noted this needed "an undici dispatcher" and was therefore out of
 * scope for a platform-agnostic module. That turned out to be too pessimistic: a
 * `LookupFunction` is plain `node:dns`, so no transport dependency is introduced
 * here. The *caller* supplies it to its own transport.
 *
 * ### Behaviour
 *
 * Every resolved address is checked against the policy and the whole resolution
 * is refused if ANY of them is blocked — never a filtered subset. Returning only
 * the allowed addresses of a mixed answer would silently make a
 * partially-attacker-controlled DNS record connectable, which is the opposite of
 * fail-closed. An empty answer is also refused.
 *
 * The reported error is an `SsrfBlockedError` carrying `code: 'EACCES'`, so it
 * both discriminates for our own handlers and reads as a connect failure to the
 * transport rather than surfacing as an unhandled rejection.
 */
export declare function createGuardedLookup(policy: SsrfPolicy): LookupFunction;
/**
 * Wraps a base `fetch` with SSRF validation applied to the initial request and
 * to every redirect target. Honors the caller's `redirect` mode:
 *   - `'follow'` (default) — follows redirects manually, re-validating each hop
 *     and applying Fetch method/credential redirect semantics (see
 *     {@link nextRedirectInit});
 *   - `'manual'` — validates and returns the first (possibly redirect) response;
 *   - `'error'` — delegates to the base fetch (which rejects on redirect).
 */
export declare function createGuardedFetch(baseFetch: typeof fetch, policy: SsrfPolicy): typeof fetch;
