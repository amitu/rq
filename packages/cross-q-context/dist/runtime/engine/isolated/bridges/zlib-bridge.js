/**
 * zlib-bridge — Safe-mode `zlib` subset (NEEDS_BRIDGE, ADR-010 §34).
 *
 * Compression/decompression is a data-shaped capability: bytes in → bytes out.
 * The isolate has no zlib, so this bridge exposes gzip/gunzip/deflate/inflate as
 * copy-in/copy-out host callbacks.
 *
 * HARD INVARIANT: only copied data crosses, as a real `Uint8Array`/`ArrayBuffer`
 * (ADR-012 follow-up): the QuickJS marshaller copies it via `getArrayBuffer`/
 * `newArrayBuffer` — a byte copy, never a live reference; a gzip→gunzip cycle is
 * byte-faithful. (This replaced the original `number[]` model, forced by an
 * isolated-vm × Electron TypedArray abort, RQ-3359, that does not exist under
 * WASM.) No host stream object crosses; each call is one-shot (the sync variants).
 */
import { deflateSync, gunzipSync, gzipSync, inflateSync } from 'node:zlib';
import { createSafeBridge } from '../safe-bridge-factory.js';
function zlibHandler(req) {
    const input = Buffer.from(req.data);
    let out;
    switch (req.op) {
        case 'gzip':
            out = gzipSync(input);
            break;
        case 'gunzip':
            out = gunzipSync(input);
            break;
        case 'deflate':
            out = deflateSync(input);
            break;
        case 'inflate':
            out = inflateSync(input);
            break;
    }
    // Cross out as a Uint8Array — the marshaller copies it to a guest ArrayBuffer.
    return { bytes: new Uint8Array(out) };
}
/** The host-side zlib bridge installed as `__rq_zlib`. */
export function createZlibBridge() {
    return createSafeBridge('__rq_zlib', zlibHandler);
}
/**
 * The guest-realm half of this bridge lives in `@requestly/sandbox-engine` (ADR-217):
 * the shim text is identical on every host, only the host callback above differs.
 * Re-exported here so existing import sites are unchanged.
 */
export { ZLIB_ISOLATE_SHIM } from '../shims/zlib.shim.js';
