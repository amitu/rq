/**
 * Browser host callback for the Safe-mode `Buffer` bridge (ADR-204).
 *
 * Peer of `sandbox-node/src/isolated/bridges/buffer-bridge.ts`. The in-isolate
 * `Buffer` shim is shared verbatim; only this host callback differs — and because
 * this bridge is pure data conversion, its output must be BYTE-IDENTICAL to Node's.
 *
 * ## Node encoding semantics that are easy to get wrong
 *
 * These are not pedantry — each one is a real behavioural difference from the
 * obvious browser primitive, and each is pinned by a parity test:
 *
 * - **`base64` decoding is LENIENT in Node, strict in `atob`.** Node ignores
 *   characters outside the alphabet, tolerates missing padding, and accepts the
 *   URL-safe alphabet under plain `'base64'`. `atob` throws on all three. Using
 *   `atob` directly would turn input Node accepts into a thrown error.
 * - **`ascii` is not "7-bit" on the way in.** Encoding a string with `'ascii'` is
 *   identical to `'latin1'` (low byte kept); only DECODING masks the high bit.
 *   Treating it as symmetric corrupts bytes ≥ 0x80 in one direction.
 * - **`hex` truncates at the first invalid pair** rather than throwing, and an odd
 *   trailing nibble is dropped.
 * - **`base64url` output carries no padding** and uses `-`/`_`.
 */
export type BufferOp = {
    readonly op: 'encode';
    readonly input: string;
    readonly encoding: string;
} | {
    readonly op: 'decode';
    readonly bytes: Uint8Array;
    readonly encoding: string;
};
export type BufferResult = {
    readonly bytes: Uint8Array;
} | {
    readonly text: string;
};
/** The browser Buffer host callback. Same shape and same static error as the Node bridge. */
export declare function browserBufferHandler(req: BufferOp): BufferResult;
