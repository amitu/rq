/**
 * Shared helpers for a script `fetch` that is DELEGATED to the host's runtime
 * fetcher rather than performed with a bare `globalThis.fetch` (ADR-181/182).
 *
 * Both engines need this and neither can own it:
 *   - Safe (QuickJS) consumes it through `isolated/bridges/fetch-bridge.ts`, which
 *     rebuilds a Response-*like* object inside the isolate (no host realm there).
 *   - Developer (`node-sandbox.ts`) consumes `toDelegatedFetch` directly, because
 *     it has a real host realm and can hand the script a genuine `Response`.
 *
 * WHY DELEGATE AT ALL: a bare `globalThis.fetch` carries only Node's bundled
 * Mozilla roots. It never sees the host's OS trust store (`system-ca`) or the
 * user's configured CA / client certificates, which the fetcher merges per
 * request. On a TLS-intercepting corporate network that makes every script
 * request fail while ordinary requests succeed (RQ-5318).
 */
import type { SendRequestHost, SerializedFetchError } from './host-types.js';
/**
 * Renders a delegated-fetch failure into a single string.
 *
 * Appends the envelope's bounded discriminants to the static message: the
 * fetcher's own `fetchKind` (a closed union — `dns` / `tls_handshake` /
 * `proxy_unreachable` / …) and `code` (an OpenSSL/libuv identifier such as
 * `SELF_SIGNED_CERT_IN_CHAIN`). Neither is request data, so nothing untrusted is
 * interpolated (`gr-static-error-messages` permits bounded types).
 *
 * This is the ONLY channel a script has for the reason: a rejected `fetch`
 * surfaces its Error's message and nothing else. Degrades cleanly — with neither
 * field present the output is exactly the static message.
 */
export declare function describeDelegationFailure(error: SerializedFetchError): string;
/**
 * Adapts a `SendRequestHost` into a `fetch`-shaped function for the Developer
 * engine's VM global.
 *
 * Unlike the Safe engine's in-isolate shim, this returns a REAL `Response`: the
 * Developer engine has a host realm, so the script keeps full WHATWG semantics
 * instead of the data-shaped subset. It also decodes `bodyEncoding`, so a binary
 * body arrives as bytes rather than the base64 string the isolate shim currently
 * surfaces.
 */
export declare function toDelegatedFetch(host: SendRequestHost): typeof fetch;
