/**
 * Browser host callback for the Safe-mode `zlib` bridge (ADR-204).
 *
 * Peer of `sandbox-node/src/isolated/bridges/zlib-bridge.ts`; the in-isolate shim
 * is shared verbatim.
 *
 * ## Why pure-JS rather than CompressionStream
 *
 * Same reason as `crypto`: the bridge contract is **synchronous** (the shim's
 * `gzipSync` returns bytes directly), and `CompressionStream` is a stream — async
 * by construction. `fflate` is zero-dependency, synchronous, and small.
 *
 * ## The trap this file exists to avoid
 *
 * Node's `deflateSync` emits a **zlib-wrapped** stream (RFC 1950 — 2-byte header +
 * Adler-32 trailer). fflate's identically-named `deflateSync` emits a **raw**
 * DEFLATE stream (RFC 1951). Mapping the two by name produces output that Node's
 * `inflateSync` rejects outright, and — worse — the failure only shows up when a
 * payload crosses surfaces, not in a browser-only round-trip. The correct pairing:
 *
 * | Node               | fflate        |
 * |--------------------|---------------|
 * | `gzipSync`         | `gzipSync`    |
 * | `gunzipSync`       | `gunzipSync`  |
 * | `deflateSync`      | `zlibSync`    |  <- not `deflateSync`
 * | `inflateSync`      | `unzlibSync`  |  <- not `inflateSync`
 *
 * ## What "parity" means here — and does not
 *
 * Unlike `crypto` and `Buffer`, byte-identical output is NOT the contract and is
 * not achievable: two conformant DEFLATE encoders may pick different block
 * splits and Huffman trees and both be correct. The real property is
 * **interoperability**, which is what the parity test asserts:
 *   - browser-compressed → Node-decompressed === original
 *   - Node-compressed    → browser-decompressed === original
 * A byte-equality test here would fail on a correct implementation.
 */
export type ZlibOp = {
    readonly op: 'gzip' | 'gunzip' | 'deflate' | 'inflate';
    readonly data: Uint8Array;
};
export type ZlibResult = {
    readonly bytes: Uint8Array;
};
export declare function browserZlibHandler(req: ZlibOp): ZlibResult;
