/**
 * buffer-bridge — Safe-mode `Buffer` capability (NEEDS_BRIDGE, ADR-010 §34).
 *
 * The isolate has no Node `Buffer`. This bridge exposes the data-shaped subset
 * scripts and packages actually use — encode/decode between strings and bytes —
 * as a copy-in/copy-out host callback, with an in-isolate JS shim presenting the
 * familiar `Buffer` surface on top of it.
 *
 * HARD INVARIANT: only copied data crosses. Bytes cross as a real `Uint8Array`/
 * `ArrayBuffer` (ADR-012 follow-up): the QuickJS marshaller copies it via
 * `getArrayBuffer`/`newArrayBuffer` — a byte copy, never a live reference. The
 * in-realm shim sends `.buffer` and reads results as `new Uint8Array(arrayBuffer)`.
 * (This replaced the original `number[]` model, forced by an isolated-vm × Electron
 * TypedArray process-abort, RQ-3359, that does not exist under WASM.) No Node
 * `Buffer` instance (a host object) ever crosses the edge.
 */

import { Buffer } from 'node:buffer';

import { createSafeBridge } from '../safe-bridge-factory.js';

import type { SafeBridge } from '../safe-bridge-factory.js';

/** Encodings the bridge accepts — a bounded set, not arbitrary host input. */
const SUPPORTED_ENCODINGS = ['utf8', 'utf-8', 'hex', 'base64', 'base64url', 'ascii', 'latin1'] as const;
type BufferEncodingSubset = (typeof SUPPORTED_ENCODINGS)[number];

function isSupportedEncoding(enc: string): enc is BufferEncodingSubset {
  return (SUPPORTED_ENCODINGS as readonly string[]).includes(enc);
}

/**
 * Host side: decode a string in the given encoding to bytes, or re-encode bytes
 * to a string. Discriminated on `op` so a single bridge covers both directions
 * with one copied-data contract.
 */
type BufferOp =
  | { readonly op: 'encode'; readonly input: string; readonly encoding: string }
  | { readonly op: 'decode'; readonly bytes: Uint8Array; readonly encoding: string };

type BufferResult = { readonly bytes: Uint8Array } | { readonly text: string };

function bufferHandler(req: BufferOp): BufferResult {
  if (!isSupportedEncoding(req.encoding)) {
    // Static, bounded message — the encoding is echoed from a closed set.
    throw new Error('Unsupported Buffer encoding in Safe mode');
  }
  if (req.op === 'encode') {
    const buf = Buffer.from(req.input, req.encoding);
    // Cross out as a Uint8Array — the marshaller copies it to a guest ArrayBuffer.
    return { bytes: new Uint8Array(buf) };
  }
  // The marshalled bytes arrive as a Uint8Array; Buffer.from re-reads them.
  return { text: Buffer.from(req.bytes).toString(req.encoding) };
}

/**
 * The host-side bridge installed into the isolate as `__rq_buffer`. The
 * in-isolate `Buffer` shim (BUFFER_ISOLATE_SHIM) calls it.
 */
export function createBufferBridge(): SafeBridge {
  return createSafeBridge('__rq_buffer', bufferHandler);
}

/**
 * The guest-realm half of this bridge lives in `@requestly/sandbox-engine` (ADR-217):
 * the shim text is identical on every host, only the host callback above differs.
 * Re-exported here so existing import sites are unchanged.
 */
export { BUFFER_ISOLATE_SHIM } from '../shims/buffer.shim.js';
