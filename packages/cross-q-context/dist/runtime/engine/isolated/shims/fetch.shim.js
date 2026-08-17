/**
 * In-isolate shim for the Safe-mode `fetch` bridge.
 *
 * Lives here, not beside its host callback, because it is pure guest-realm JS
 * text with no host dependency — the half of the bridge that is identical on
 * every host. `@requestly/sandbox-node` re-exports it, so existing import sites
 * are unchanged. Keep this file free of imports.
 */
/**
 * In-isolate JS: a minimal `fetch` over `__rq_fetch`, returning a Response-like
 * object whose `.text()`/`.json()` resolve from the copied body string. Not a
 * full WHATWG Response — the data-shaped subset packages use. Identical for the
 * delegated and direct host paths — only the host side of the bridge moves.
 *
 * DECODES `bodyEncoding` (RQ-5401 §6). The fetcher classifies a binary response
 * body as base64 (ADR-153) and it rides the boundary as a flat string; without
 * decoding, `.text()` handed the guest the BASE64 TEXT rather than the content —
 * silently wrong, and indistinguishable from a server that really returned base64.
 * `.text()`/`.json()` now decode first, and `.arrayBuffer()` exposes the real bytes
 * (the only correct way to read a binary body — `.text()` on one is lossy by
 * definition, exactly as in WHATWG).
 *
 * `atob` and a UTF-8 `TextDecoder` come from `core-globals.ts`, which the isolate
 * installs for this reason; the shim body runs at CALL time, so their eval order
 * relative to this shim does not matter.
 */
export const FETCH_ISOLATE_SHIM = `
(() => {
  const call = globalThis.__rq_fetch;
  // base64 -> bytes. \`atob\` yields one char per byte (latin1), so char codes ARE
  // the bytes; a utf8 body needs no decode and is returned as-is by the callers.
  const toBytes = (b64) => {
    const bin = atob(b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return bytes;
  };
  const asText = (res) =>
    res.bodyEncoding === 'base64' ? new TextDecoder().decode(toBytes(res.body)) : res.body;
  globalThis.fetch = async (url, init) => {
    const opts = init || {};
    const headers = {};
    if (opts.headers) {
      if (typeof opts.headers.forEach === 'function') opts.headers.forEach((v, k) => { headers[k] = v; });
      else for (const k of Object.keys(opts.headers)) headers[k] = String(opts.headers[k]);
    }
    const req = {
      url: String(url),
      method: (opts.method || 'GET').toUpperCase(),
      headers,
      body: opts.body != null ? String(opts.body) : undefined,
    };
    const res = await call(req);
    return {
      status: res.status,
      statusText: res.statusText,
      ok: res.status >= 200 && res.status < 300,
      headers: { get: (k) => res.headers[String(k).toLowerCase()] ?? null, forEach: (fn) => { for (const k of Object.keys(res.headers)) fn(res.headers[k], k); } },
      text: async () => asText(res),
      json: async () => JSON.parse(asText(res)),
      arrayBuffer: async () =>
        (res.bodyEncoding === 'base64' ? toBytes(res.body) : new TextEncoder().encode(res.body)).buffer,
    };
  };
})();
`;
