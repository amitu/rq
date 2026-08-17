/**
 * In-isolate shim for the Safe-mode `zlib` bridge.
 *
 * Lives here, not beside its host callback, because it is pure guest-realm JS
 * text with no host dependency — the half of the bridge that is identical on
 * every host. `@requestly/sandbox-node` re-exports it, so existing import sites
 * are unchanged. Keep this file free of imports.
 */
/**
 * In-isolate JS: builds the `zlib` subset (sync gzip/gunzip/deflate/inflate) over
 * `__rq_zlib`, accepting `Uint8Array`/string and returning `Uint8Array`. Bytes
 * cross the edge as an `ArrayBuffer` (copied via the marshaller — ADR-012).
 */
export declare const ZLIB_ISOLATE_SHIM = "\n(() => {\n  const call = globalThis.__rq_zlib;\n  // Normalize input to an ArrayBuffer to send across (the marshaller copies it).\n  const toAb = (data) => {\n    if (data instanceof ArrayBuffer) return data;\n    if (data instanceof Uint8Array) return data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength);\n    if (data && data.buffer instanceof ArrayBuffer) return data.buffer.slice(data.byteOffset || 0, (data.byteOffset || 0) + data.byteLength);\n    if (Array.isArray(data)) return Uint8Array.from(data).buffer;\n    return new TextEncoder().encode(String(data)).buffer;\n  };\n  // Return a Buffer (real Node zlib returns Buffer) so toString(enc) decodes\n  // correctly \u2014 a plain Uint8Array's toString() comma-joins the bytes. Buffer is\n  // installed before this shim (ISOLATE_SHIMS order).\n  const mk = (op) => (data) => Buffer.from(new Uint8Array(call({ op, data: toAb(data) }).bytes));\n  globalThis.__rq_zlibModule = {\n    gzipSync: mk('gzip'), gunzipSync: mk('gunzip'),\n    deflateSync: mk('deflate'), inflateSync: mk('inflate'),\n  };\n})();\n";
