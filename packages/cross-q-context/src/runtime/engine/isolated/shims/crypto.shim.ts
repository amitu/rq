/**
 * In-isolate shim for the Safe-mode `crypto` bridge.
 *
 * Lives here, not beside its host callback, because it is pure guest-realm JS
 * text with no host dependency — the half of the bridge that is identical on
 * every host. `@requestly/sandbox-node` re-exports it, so existing import sites
 * are unchanged. Keep this file free of imports.
 */

/**
 * In-isolate JS: builds `require('crypto')` subset + global `getRandomValues` on
 * top of `__rq_crypto`. Hash/Hmac are one-shot builders that accumulate input
 * in-isolate then make a single bridged call on `.digest()`.
 */
export const CRYPTO_ISOLATE_SHIM = `
(() => {
  const call = globalThis.__rq_crypto;
  // Normalize any input to a Uint8Array. Binary crosses the edge as the underlying
  // ArrayBuffer (the marshaller copies it via getArrayBuffer/newArrayBuffer —
  // ADR-012), so callers send \`.buffer\` and read results as \`new Uint8Array(ab)\`.
  const toBytes = (data) => {
    if (data instanceof Uint8Array) return data;
    if (data && data.buffer instanceof ArrayBuffer) return new Uint8Array(data.buffer, data.byteOffset || 0, data.byteLength);
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (Array.isArray(data)) return Uint8Array.from(data);
    return new TextEncoder().encode(String(data));
  };
  const concatBytes = (chunks) => {
    let total = 0;
    for (const c of chunks) total += c.length;
    const out = new Uint8Array(total);
    let off = 0;
    for (const c of chunks) { out.set(c, off); off += c.length; }
    return out;
  };
  // Send a Uint8Array across as its ArrayBuffer (slice to the exact view window so
  // a subarray doesn't leak the whole backing buffer).
  const ab = (u8) => u8.buffer.slice(u8.byteOffset, u8.byteOffset + u8.byteLength);
  class Hash {
    constructor(algo) { this._algo = algo; this._chunks = []; }
    update(data) { this._chunks.push(toBytes(data)); return this; }
    digest(encoding) {
      return call({ op: 'hash', algo: this._algo, data: ab(concatBytes(this._chunks)), outputEncoding: encoding || 'hex' }).digest;
    }
  }
  class Hmac {
    constructor(algo, key) { this._algo = algo; this._key = toBytes(key); this._chunks = []; }
    update(data) { this._chunks.push(toBytes(data)); return this; }
    digest(encoding) {
      return call({ op: 'hmac', algo: this._algo, key: ab(this._key), data: ab(concatBytes(this._chunks)), outputEncoding: encoding || 'hex' }).digest;
    }
  }
  // Fill a TypedArray/Buffer view in place with CSPRNG bytes (the Web
  // getRandomValues + Node randomFillSync contract). Randomness comes from the
  // randomBytes bridge call (result is an ArrayBuffer); the write-back happens
  // in-realm.
  const fillInPlace = (typedArray, offset, size) => {
    const start = offset || 0;
    const len = size === undefined ? typedArray.byteLength - start : size;
    const bytes = new Uint8Array(call({ op: 'randomBytes', size: len }).bytes);
    new Uint8Array(typedArray.buffer, typedArray.byteOffset + start, len).set(bytes);
    return typedArray;
  };
  // Guided stubs for the crypto methods Safe mode does NOT support, so a script
  // that reaches for them gets a clear, actionable error instead of an opaque
  // "x is not a function" (ADR-010 §76/§87 — the asymmetric deferral). Static,
  // bounded messages (gr-static-error-messages). These throw IN-ISOLATE — no
  // bridge call, no boundary crossing.
  const ASYMMETRIC_MSG =
    "Asymmetric crypto (RS256/ES/PS — sign/verify, public/private key ops) isn't available in Safe mode. " +
    'Use a symmetric algorithm (e.g. HS256), or switch this request to Developer mode.';
  const SYMMETRIC_CIPHER_MSG =
    "Cipher streams (createCipheriv/createDecipheriv) aren't available in Safe mode. " +
    'Switch this request to Developer mode to use them.';
  const asymmetricStub = () => {
    throw new Error(ASYMMETRIC_MSG);
  };
  const cipherStub = () => {
    throw new Error(SYMMETRIC_CIPHER_MSG);
  };
  const cryptoModule = {
    createHash: (algo) => new Hash(algo),
    createHmac: (algo, key) => new Hmac(algo, key),
    // Real Node crypto.randomBytes returns a Buffer; wrap so toString('hex')
    // (the common JWT/token idiom) decodes instead of comma-joining the bytes.
    randomBytes: (size) => Buffer.from(new Uint8Array(call({ op: 'randomBytes', size }).bytes)),
    randomUUID: () => call({ op: 'randomUUID' }).uuid,
    // Node's in-place CSPRNG fill (uuid v9 Node build uses this).
    randomFillSync: (buf, offset, size) => fillInPlace(buf, offset, size),
    getRandomValues: (typedArray) => fillInPlace(typedArray),
    // Unsupported — guided throw rather than undefined (the migration footgun the
    // Slice-3.1 audit flagged: jwt.sign with RS256 etc.).
    sign: asymmetricStub,
    verify: asymmetricStub,
    createSign: asymmetricStub,
    createVerify: asymmetricStub,
    publicEncrypt: asymmetricStub,
    privateDecrypt: asymmetricStub,
    privateEncrypt: asymmetricStub,
    publicDecrypt: asymmetricStub,
    generateKeyPair: asymmetricStub,
    generateKeyPairSync: asymmetricStub,
    createCipheriv: cipherStub,
    createDecipheriv: cipherStub,
  };
  // webcrypto subset some packages reach for (crypto.webcrypto.getRandomValues).
  cryptoModule.webcrypto = { getRandomValues: (ta) => fillInPlace(ta) };
  globalThis.__rq_cryptoModule = cryptoModule;
  globalThis.getRandomValues = (typedArray) => fillInPlace(typedArray);
  // Some bundles read globalThis.crypto.getRandomValues (Web Crypto global).
  if (typeof globalThis.crypto === 'undefined') {
    globalThis.crypto = { getRandomValues: (ta) => fillInPlace(ta), randomUUID: () => call({ op: 'randomUUID' }).uuid };
  }
})();
`;
