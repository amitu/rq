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
import { lookup } from 'node:dns/promises';
import { isIP } from 'node:net';
/** Fail-closed posture for server-side hosts — blocks every private range. */
export const STRICT_SSRF_POLICY = { allowPrivateNetwork: false };
/** Lenient posture for client hosts — preserves `localhost`/LAN access. */
export const CLIENT_SSRF_POLICY = { allowPrivateNetwork: true };
/** Hostnames that must never be reachable regardless of resolved IP or policy. */
const BLOCKED_HOSTNAMES = new Set(['metadata.google.internal', 'metadata.goog', 'metadata']);
const MAX_REDIRECTS = 20;
/** Headers stripped when a redirect crosses to a different origin. */
const CROSS_ORIGIN_SENSITIVE_HEADERS = ['authorization', 'cookie', 'proxy-authorization'];
/** Content headers dropped when a redirect downgrades the method to GET. */
const BODY_HEADERS = ['content-type', 'content-length', 'content-encoding', 'content-language', 'content-location'];
/**
 * Thrown when the guard refuses a request. The message is static
 * (`gr-static-error-messages`); the specific reason is a bounded discriminant.
 */
export class SsrfBlockedError extends Error {
    reason;
    constructor(reason) {
        super('Request blocked by the sandbox SSRF guard');
        this.name = 'SsrfBlockedError';
        this.reason = reason;
    }
}
function ipv4ToUint32(ip) {
    const parts = ip.split('.');
    if (parts.length !== 4)
        return null;
    let value = 0;
    for (const part of parts) {
        if (!/^\d{1,3}$/.test(part))
            return null;
        const octet = Number(part);
        if (octet > 255)
            return null;
        value = value * 256 + octet;
    }
    return value >>> 0;
}
function inRange(value, base, prefix) {
    const baseValue = ipv4ToUint32(base);
    if (baseValue === null)
        return false;
    const mask = prefix === 0 ? 0 : (0xffffffff << (32 - prefix)) >>> 0;
    return (value & mask) === (baseValue & mask);
}
/** Link-local / IMDS — 169.254.0.0/16. Always blocked. */
function isIpv4LinkLocal(value) {
    return inRange(value, '169.254.0.0', 16);
}
/** Loopback / private / reserved ranges — blocked only under the strict policy. */
function isIpv4Private(value) {
    return (inRange(value, '0.0.0.0', 8) || // "this host"
        inRange(value, '10.0.0.0', 8) || // RFC1918
        inRange(value, '100.64.0.0', 10) || // CGNAT (RFC6598)
        inRange(value, '127.0.0.0', 8) || // loopback
        inRange(value, '172.16.0.0', 12) || // RFC1918
        inRange(value, '192.0.0.0', 24) || // IETF protocol assignments
        inRange(value, '192.0.2.0', 24) || // TEST-NET-1
        inRange(value, '192.168.0.0', 16) || // RFC1918
        inRange(value, '198.18.0.0', 15) || // benchmarking
        inRange(value, '198.51.100.0', 24) || // TEST-NET-2
        inRange(value, '203.0.113.0', 24) || // TEST-NET-3
        inRange(value, '240.0.0.0', 4) // reserved / broadcast
    );
}
/**
 * Expands any IPv6 literal into its 8 16-bit hextets, handling `::` compression
 * and a trailing embedded IPv4 dotted-quad. Returns null if not a valid IPv6
 * literal. This mirrors what the WHATWG URL parser stores in `url.hostname`,
 * which normalizes e.g. `::ffff:169.254.169.254` to `::ffff:a9fe:a9fe` — the
 * naive "substring after the last colon" approach missed that form entirely.
 */
function parseIpv6(input) {
    let str = input.toLowerCase().trim();
    const zone = str.indexOf('%');
    if (zone !== -1)
        str = str.slice(0, zone); // drop scope id (fe80::1%eth0)
    if (!str.includes(':'))
        return null;
    // A trailing embedded IPv4 dotted-quad (::ffff:1.2.3.4) → two hex groups.
    const dotIdx = str.indexOf('.');
    if (dotIdx !== -1) {
        const colonIdx = str.lastIndexOf(':', dotIdx);
        if (colonIdx === -1)
            return null;
        const v4 = ipv4ToUint32(str.slice(colonIdx + 1));
        if (v4 === null)
            return null;
        const hi = ((v4 >>> 16) & 0xffff).toString(16);
        const lo = (v4 & 0xffff).toString(16);
        str = `${str.slice(0, colonIdx + 1)}${hi}:${lo}`;
    }
    const parts = str.split('::');
    if (parts.length > 2)
        return null;
    const toHextets = (segment) => {
        if (segment === '')
            return [];
        const out = [];
        for (const group of segment.split(':')) {
            if (!/^[0-9a-f]{1,4}$/.test(group))
                return null;
            out.push(Number.parseInt(group, 16));
        }
        return out;
    };
    const head = toHextets(parts[0] ?? '');
    if (head === null)
        return null;
    if (parts.length === 1)
        return head.length === 8 ? head : null;
    const tail = toHextets(parts[1] ?? '');
    if (tail === null)
        return null;
    const gap = 8 - head.length - tail.length;
    if (gap < 1)
        return null; // `::` must stand for ≥1 zero group
    return [...head, ...new Array(gap).fill(0), ...tail];
}
function hextetsMatch(h, prefix) {
    return prefix.every((value, i) => h[i] === value);
}
/**
 * Classifies an IPv6 address (given as 8 hextets). The three IPv4-carrying /96
 * prefixes — IPv4-mapped `::ffff:0:0/96`, IPv4-compatible `::/96` (covers `::`
 * and `::1`), and NAT64 `64:ff9b::/96` — are decided purely by their embedded
 * IPv4 so no dotted/hex/compressed spelling can slip past the IPv4 rules.
 */
function isIpv6HextetsBlocked(h, allowPrivateNetwork) {
    const isMapped = hextetsMatch(h, [0, 0, 0, 0, 0, 0xffff]);
    const isCompat = hextetsMatch(h, [0, 0, 0, 0, 0, 0]); // ::/96 (incl. ::, ::1)
    const isNat64 = hextetsMatch(h, [0x0064, 0xff9b, 0, 0, 0, 0]);
    if (isMapped || isCompat || isNat64) {
        const v4 = (((h[6] ?? 0) << 16) | (h[7] ?? 0)) >>> 0;
        if (isIpv4LinkLocal(v4))
            return true;
        return allowPrivateNetwork ? false : isIpv4Private(v4);
    }
    // AWS IMDSv6 (fd00:ec2::254) — metadata endpoint, always blocked.
    if (hextetsMatch(h, [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x254]))
        return true;
    // fe80::/10 — link-local. Always blocked.
    if (((h[0] ?? 0) & 0xffc0) === 0xfe80)
        return true;
    if (allowPrivateNetwork)
        return false;
    // fc00::/7 — unique-local (private).
    if (((h[0] ?? 0) & 0xfe00) === 0xfc00)
        return true;
    return false;
}
/** Whether a resolved IP address must be refused under the given policy. */
export function isAddressBlocked(ip, policy) {
    const kind = isIP(ip);
    if (kind === 4) {
        const value = ipv4ToUint32(ip);
        if (value === null)
            return true; // unparseable → fail closed
        if (isIpv4LinkLocal(value))
            return true;
        return policy.allowPrivateNetwork ? false : isIpv4Private(value);
    }
    if (kind === 6) {
        const hextets = parseIpv6(ip);
        if (hextets === null)
            return true; // unparseable → fail closed
        return isIpv6HextetsBlocked(hextets, policy.allowPrivateNetwork);
    }
    return true; // not an IP → fail closed
}
/**
 * Validates a single URL against the policy: scheme allowlist, metadata-host
 * denylist, then resolved-IP checks. Throws {@link SsrfBlockedError} when refused.
 */
export async function assertUrlAllowed(rawUrl, policy) {
    const url = new URL(rawUrl);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') {
        throw new SsrfBlockedError('scheme');
    }
    const hostname = url.hostname.replace(/^\[|\]$/g, '').toLowerCase();
    if (BLOCKED_HOSTNAMES.has(hostname)) {
        throw new SsrfBlockedError('metadata-host');
    }
    // Host is already an IP literal — validate directly, no DNS.
    if (isIP(hostname) !== 0) {
        if (isAddressBlocked(hostname, policy))
            throw new SsrfBlockedError('blocked-address');
        return;
    }
    // Resolve and validate EVERY address the hostname maps to.
    const resolved = await lookup(hostname, { all: true, verbatim: true });
    if (resolved.length === 0)
        throw new SsrfBlockedError('blocked-address');
    for (const { address } of resolved) {
        if (isAddressBlocked(address, policy))
            throw new SsrfBlockedError('blocked-address');
    }
}
/**
 * Computes the `RequestInit` for the next redirect hop, applying the Fetch
 * redirect semantics the manual loop must reproduce (the platform would apply
 * them itself under `redirect: 'follow'`):
 *   - 301/302 on a non-GET/HEAD method, and any 303, downgrade to a bodyless GET
 *     (method → GET, body + content headers dropped); 307/308 preserve both;
 *   - a cross-origin hop strips `Authorization` / `Cookie` /
 *     `Proxy-Authorization` so credentials are never forwarded to a new origin.
 */
function nextRedirectInit(init, status, fromUrl, toUrl) {
    const next = { ...init };
    const headers = new Headers(init?.headers ?? undefined);
    const method = (next.method ?? 'GET').toUpperCase();
    const downgrades = status === 303 || ((status === 301 || status === 302) && method !== 'GET' && method !== 'HEAD');
    if (downgrades) {
        next.method = 'GET';
        delete next.body;
        for (const header of BODY_HEADERS)
            headers.delete(header);
    }
    if (new URL(fromUrl).origin !== new URL(toUrl).origin) {
        for (const header of CROSS_ORIGIN_SENSITIVE_HEADERS)
            headers.delete(header);
    }
    next.headers = headers;
    return next;
}
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
export function createGuardedLookup(policy) {
    return (hostname, options, callback) => {
        void (async () => {
            try {
                // Force `all` so every address is inspected, regardless of what the caller
                // asked for; the requested shape is restored when reporting back. `family`
                // and `hints` are preserved so the transport's own preferences still apply.
                const addresses = await lookup(hostname, { ...options, all: true });
                if (addresses.length === 0) {
                    callback(blockedLookupError(), []);
                    return;
                }
                for (const { address } of addresses) {
                    if (isAddressBlocked(address, policy)) {
                        callback(blockedLookupError(), []);
                        return;
                    }
                }
                if (options.all === true) {
                    callback(null, addresses);
                    return;
                }
                const first = addresses[0];
                if (first === undefined) {
                    callback(blockedLookupError(), []);
                    return;
                }
                callback(null, first.address, first.family);
            }
            catch (err) {
                // A genuine resolution failure (NXDOMAIN, timeout) is reported as-is: it is
                // the transport's to classify, and reshaping it into a block would mislabel
                // an ordinary typo as a security refusal.
                callback(err instanceof Error ? err : blockedLookupError(), []);
            }
        })();
    };
}
/** An `SsrfBlockedError` shaped as an errno error so a transport treats it as a connect failure. */
function blockedLookupError() {
    const err = new SsrfBlockedError('blocked-address');
    err.code = 'EACCES';
    return err;
}
/**
 * Wraps a base `fetch` with SSRF validation applied to the initial request and
 * to every redirect target. Honors the caller's `redirect` mode:
 *   - `'follow'` (default) — follows redirects manually, re-validating each hop
 *     and applying Fetch method/credential redirect semantics (see
 *     {@link nextRedirectInit});
 *   - `'manual'` — validates and returns the first (possibly redirect) response;
 *   - `'error'` — delegates to the base fetch (which rejects on redirect).
 */
export function createGuardedFetch(baseFetch, policy) {
    const guarded = async (input, init) => {
        const startUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const redirectMode = init?.redirect ?? 'follow';
        if (redirectMode !== 'follow') {
            await assertUrlAllowed(startUrl, policy);
            return baseFetch(input, init);
        }
        // Manual redirect following so each hop's target is re-validated. `currentInit`
        // carries the Fetch method/credential adjustments across hops.
        let currentUrl = startUrl;
        let currentInit = init;
        for (let hop = 0; hop <= MAX_REDIRECTS; hop++) {
            await assertUrlAllowed(currentUrl, policy);
            const response = await baseFetch(currentUrl, { ...currentInit, redirect: 'manual' });
            const location = response.status >= 300 && response.status < 400 ? response.headers.get('location') : null;
            if (location === null)
                return response;
            const nextUrl = new URL(location, currentUrl).href;
            currentInit = nextRedirectInit(currentInit, response.status, currentUrl, nextUrl);
            currentUrl = nextUrl;
        }
        throw new SsrfBlockedError('blocked-address');
    };
    return guarded;
}
