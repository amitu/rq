/**
 * crypto-bridge — Safe-mode crypto subset (NEEDS_BRIDGE, ADR-010 §34/§76).
 *
 * Exposes the data-shaped crypto operations the npm long-tail needs — hashing,
 * HMAC (the jsonwebtoken HS256 case, byte-identical to host), and random bytes —
 * as copy-in/copy-out host callbacks, with an in-isolate shim presenting both
 * `require('crypto')` and a global `getRandomValues`.
 *
 * SCOPE (ADR-010 §76): symmetric only. Asymmetric sign/verify (RS256/ES/PS) is
 * IMPOSSIBLE in v1 — those route to the guided-error path (impossible-error.ts),
 * not this bridge. Adding them later is a new factory-produced bridge, not a
 * relaxation of the copy invariant (§88).
 *
 * HARD INVARIANT: only copied data crosses. Binary I/O crosses as a real
 * `Uint8Array`/`ArrayBuffer` (ADR-012 follow-up): the QuickJS marshaller copies it
 * via `getArrayBuffer` (in) / `newArrayBuffer` (out) — a byte copy, never a live
 * reference. The in-realm shim sends `.buffer` (an ArrayBuffer) and reads results
 * back as `new Uint8Array(arrayBuffer)`. (This replaced the original `number[]`
 * model, forced by an isolated-vm × Electron TypedArray process-abort, RQ-3359 —
 * a class that does not exist under WASM. HMAC-SHA256 byte-parity holds either
 * way.) Digests come back as strings; random bytes as a `Uint8Array`. No host
 * `Hash`/`Hmac` object crosses — each call is one-shot data-in/data-out.
 */
import type { SafeBridge } from '../safe-bridge-factory.js';
/** The host-side crypto bridge installed as `__rq_crypto`. */
export declare function createCryptoBridge(): SafeBridge;
/**
 * The guest-realm half of this bridge lives in `@requestly/sandbox-engine` (ADR-217):
 * the shim text is identical on every host, only the host callback above differs.
 * Re-exported here so existing import sites are unchanged.
 */
export { CRYPTO_ISOLATE_SHIM } from '../shims/crypto.shim.js';
