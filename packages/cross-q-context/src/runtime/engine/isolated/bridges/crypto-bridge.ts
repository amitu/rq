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

import { createHash, createHmac, randomBytes, randomUUID } from 'node:crypto';

import { createSafeBridge } from '../safe-bridge-factory.js';

import type { SafeBridge } from '../safe-bridge-factory.js';

const SUPPORTED_HASHES = ['sha1', 'sha256', 'sha384', 'sha512', 'md5'] as const;
type HashAlgo = (typeof SUPPORTED_HASHES)[number];

function isSupportedHash(algo: string): algo is HashAlgo {
  return (SUPPORTED_HASHES as readonly string[]).includes(algo);
}

/**
 * One-shot crypto operations, discriminated on `op`. All I/O is copied data;
 * binary fields are `Uint8Array` (marshalled as a copied ArrayBuffer — ADR-012).
 */
type CryptoOp =
  | { readonly op: 'hash'; readonly algo: string; readonly data: Uint8Array; readonly outputEncoding: 'hex' | 'base64' }
  | {
      readonly op: 'hmac';
      readonly algo: string;
      readonly key: Uint8Array;
      readonly data: Uint8Array;
      readonly outputEncoding: 'hex' | 'base64';
    }
  | { readonly op: 'randomBytes'; readonly size: number }
  | { readonly op: 'randomUUID' };

type CryptoResult = { readonly digest: string } | { readonly bytes: Uint8Array } | { readonly uuid: string };

const MAX_RANDOM_BYTES = 65536;

function cryptoHandler(req: CryptoOp): CryptoResult {
  switch (req.op) {
    case 'hash': {
      if (!isSupportedHash(req.algo)) throw new Error('Unsupported hash algorithm in Safe mode');
      return { digest: createHash(req.algo).update(req.data).digest(req.outputEncoding) };
    }
    case 'hmac': {
      if (!isSupportedHash(req.algo)) throw new Error('Unsupported HMAC algorithm in Safe mode');
      return {
        digest: createHmac(req.algo, req.key).update(req.data).digest(req.outputEncoding),
      };
    }
    case 'randomBytes': {
      if (!Number.isInteger(req.size) || req.size < 0 || req.size > MAX_RANDOM_BYTES) {
        throw new Error('Invalid randomBytes size in Safe mode');
      }
      // randomBytes returns a Buffer (a Uint8Array) — cross it out directly; the
      // marshaller copies it to a guest ArrayBuffer.
      return { bytes: new Uint8Array(randomBytes(req.size)) };
    }
    case 'randomUUID':
      return { uuid: randomUUID() };
  }
}

/** The host-side crypto bridge installed as `__rq_crypto`. */
export function createCryptoBridge(): SafeBridge {
  return createSafeBridge('__rq_crypto', cryptoHandler);
}

/**
 * The guest-realm half of this bridge lives in `@requestly/sandbox-engine` (ADR-217):
 * the shim text is identical on every host, only the host callback above differs.
 * Re-exported here so existing import sites are unchanged.
 */
export { CRYPTO_ISOLATE_SHIM } from '../shims/crypto.shim.js';
