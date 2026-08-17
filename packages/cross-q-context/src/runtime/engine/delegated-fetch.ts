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

import type { SendRequestHost, SerializedFetchError, SerializedFetchRequest } from './host-types.js';

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
export function describeDelegationFailure(error: SerializedFetchError): string {
  const parts: string[] = [];
  if (error.fetchKind !== undefined) parts.push(error.fetchKind);
  if (error.code !== undefined) parts.push(error.code);
  return parts.length === 0 ? error.message : `${error.message} (${parts.join(': ')})`;
}

/** Statuses that MUST carry a null body — `new Response(body, { status })` throws otherwise. */
const NULL_BODY_STATUSES: ReadonlySet<number> = new Set([101, 103, 204, 205, 304]);

/**
 * Builds the Error a delegated fetch rejects with, giving `message` an OWN
 * ENUMERABLE descriptor.
 *
 * Script logs are serialized out of the sandbox with `JSON.stringify`, and Node
 * makes `Error.prototype.message` non-enumerable — so a script author writing the
 * natural `console.error(err)` saw `{…,"cause":{}}` with the reason silently
 * erased. QuickJS already exposes an own-enumerable `message`, which is why the
 * Safe engine showed the reason and the Developer engine did not; this brings the
 * two to parity (RQ-5318).
 *
 * Only the message is redescribed — the Error is otherwise ordinary, so `throw` /
 * `instanceof` / stack capture are unaffected.
 */
function delegationError(message: string): Error {
  const error = new Error(message);
  Object.defineProperty(error, 'message', { value: message, enumerable: true, writable: true, configurable: true });
  return error;
}

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
export function toDelegatedFetch(host: SendRequestHost): typeof fetch {
  const delegated = async (input: Parameters<typeof fetch>[0], init?: RequestInit): Promise<Response> => {
    // Let the platform normalize the call: `Request` resolves the URL, upper-cases
    // the method, merges headers, and turns ANY `BodyInit` (string, URLSearchParams,
    // FormData, ArrayBuffer, Blob, …) into a readable body — including synthesising
    // the multipart `content-type` a hand-rolled `String(body)` would both mangle
    // and omit. Reading it back as text matches the boundary, whose `body` is a
    // string (ADR-034).
    const normalized = new Request(input, init);
    const headers: Record<string, string> = {};
    normalized.headers.forEach((value, key) => {
      headers[key] = value;
    });
    const requestBody = normalized.body === null ? undefined : await normalized.text();

    const request: SerializedFetchRequest = {
      url: normalized.url,
      method: normalized.method,
      headers,
      ...(requestBody !== undefined ? { body: requestBody } : {}),
    };

    const envelope = await host.sendRequest(request);
    if (!envelope.ok) {
      // Mirrors the isolate shim: the guest sees a rejected fetch whose message
      // carries the fetcher's bounded classification.
      throw delegationError(describeDelegationFailure(envelope.error));
    }

    const { status, statusText, headers: responseHeaders, body, bodyEncoding } = envelope.response;
    const decoded = bodyEncoding === 'base64' ? Buffer.from(body, 'base64') : body;
    return new Response(NULL_BODY_STATUSES.has(status) ? null : decoded, {
      status,
      statusText,
      headers: responseHeaders,
    });
  };
  return delegated as typeof fetch;
}
