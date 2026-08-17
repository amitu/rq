import { hmac } from '@noble/hashes/hmac';
import { md5, sha1 } from '@noble/hashes/legacy';
import { sha256, sha384, sha512 } from '@noble/hashes/sha2';
/**
 * Browser host callback for the Safe-mode crypto bridge (ADR-204 §Decision item 4).
 *
 * This is the browser peer of `sandbox-node/src/isolated/bridges/crypto-bridge.ts`.
 * The **in-isolate shim is shared verbatim** — it is a pure string with no host
 * dependency — so the guest-visible `require('crypto')` API is literally the same
 * code on both surfaces. Only this host callback differs, and it must produce
 * BYTE-IDENTICAL output to the Node one or the shared shim silently lies.
 *
 * ## Why pure-JS and not WebCrypto
 *
 * The bridge contract is **synchronous**: the shim's `Hash.digest()` / `Hmac.digest()`
 * return a string directly. `SubtleCrypto.digest()` returns a Promise, so WebCrypto
 * cannot satisfy this shape without either (a) rewriting the contract async on both
 * surfaces, or (b) `Atomics.wait` + `SharedArrayBuffer` to block a worker on it,
 * which would require COOP/COEP headers across the whole web app to obtain a
 * primitive we can simply have in-process. ADR-204 chose pure-JS; both Postman and
 * Apidog ship pure-JS shims rather than host modules for the same reason.
 *
 * `@noble/hashes` is audited, zero-dependency, synchronous, and **already a direct
 * dependency of `modules/runtime` and `modules/sdk`** — so this adds no new
 * supply-chain surface.
 *
 * ## What is NOT pure-JS here
 *
 * Randomness deliberately is not. `crypto.getRandomValues` and `crypto.randomUUID`
 * are **synchronous** in browsers, so the CSPRNG stays the platform's — a hand-rolled
 * PRNG would be strictly worse and is the classic own-goal in this area.
 */
/** Mirrors `SUPPORTED_HASHES` in the Node bridge. The parity test asserts the two lists are equal. */
const SUPPORTED_HASHES = ['sha1', 'sha256', 'sha384', 'sha512', 'md5'];
const HASHERS = { sha1, sha256, sha384, sha512, md5 };
function isSupportedHash(algo) {
    return SUPPORTED_HASHES.includes(algo);
}
/**
 * `crypto.getRandomValues` rejects requests above 65536 bytes, and the Node bridge
 * independently caps at the same number — so this bound is both a policy limit and
 * a platform one.
 */
const MAX_RANDOM_BYTES = 65536;
/**
 * Hex-encode. Only ever runs on a DIGEST (≤64 bytes for SHA-512), never on the
 * input, so the readable form is used rather than a nibble-lookup table — the
 * lookup version needs an index assertion under `noUncheckedIndexedAccess` and
 * buys nothing measurable here.
 */
function toHex(bytes) {
    let out = '';
    for (const byte of bytes)
        out += byte.toString(16).padStart(2, '0');
    return out;
}
/**
 * Base64 via `btoa`. `btoa` takes a *binary string* (one char per byte), so the
 * bytes are widened one at a time — `String.fromCharCode(...bytes)` would blow the
 * argument limit on a large digest input and is a real crash, not a style nit.
 */
function toBase64(bytes) {
    let binary = '';
    for (const byte of bytes)
        binary += String.fromCharCode(byte);
    return btoa(binary);
}
function encode(bytes, outputEncoding) {
    return outputEncoding === 'base64' ? toBase64(bytes) : toHex(bytes);
}
/**
 * The browser crypto host callback. Same discriminated `op` switch as the Node
 * bridge; same static error messages (`gr-static-error-messages`).
 */
export function browserCryptoHandler(req) {
    switch (req.op) {
        case 'hash': {
            if (!isSupportedHash(req.algo))
                throw new Error('Unsupported hash algorithm in Safe mode');
            return { digest: encode(HASHERS[req.algo](req.data), req.outputEncoding) };
        }
        case 'hmac': {
            if (!isSupportedHash(req.algo))
                throw new Error('Unsupported HMAC algorithm in Safe mode');
            return { digest: encode(hmac(HASHERS[req.algo], req.key, req.data), req.outputEncoding) };
        }
        case 'randomBytes': {
            if (!Number.isInteger(req.size) || req.size < 0 || req.size > MAX_RANDOM_BYTES) {
                throw new Error('Invalid randomBytes size in Safe mode');
            }
            // Platform CSPRNG, synchronous in browsers. `getRandomValues` throws on a
            // zero-length... it does not, but it does reject >65536 — bounded above.
            return { bytes: crypto.getRandomValues(new Uint8Array(req.size)) };
        }
        case 'randomUUID':
            return { uuid: crypto.randomUUID() };
    }
}
