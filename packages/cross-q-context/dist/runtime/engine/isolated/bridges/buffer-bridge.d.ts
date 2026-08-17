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
import type { SafeBridge } from '../safe-bridge-factory.js';
/**
 * The host-side bridge installed into the isolate as `__rq_buffer`. The
 * in-isolate `Buffer` shim (BUFFER_ISOLATE_SHIM) calls it.
 */
export declare function createBufferBridge(): SafeBridge;
/**
 * The guest-realm half of this bridge lives in `@requestly/sandbox-engine` (ADR-217):
 * the shim text is identical on every host, only the host callback above differs.
 * Re-exported here so existing import sites are unchanged.
 */
export { BUFFER_ISOLATE_SHIM } from '../shims/buffer.shim.js';
