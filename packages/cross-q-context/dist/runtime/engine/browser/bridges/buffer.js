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
/** Mirrors `SUPPORTED_ENCODINGS` in the Node bridge; the parity test asserts the lists match. */
const SUPPORTED_ENCODINGS = ['utf8', 'utf-8', 'hex', 'base64', 'base64url', 'ascii', 'latin1'];
function isSupportedEncoding(enc) {
    return SUPPORTED_ENCODINGS.includes(enc);
}
const BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
/** Reverse lookup accepting BOTH alphabets, matching Node's leniency. */
const BASE64_LOOKUP = new Map();
for (let i = 0; i < BASE64_ALPHABET.length; i++)
    BASE64_LOOKUP.set(BASE64_ALPHABET[i] ?? '', i);
BASE64_LOOKUP.set('-', 62);
BASE64_LOOKUP.set('_', 63);
/**
 * Lenient base64 → bytes, matching `Buffer.from(str, 'base64')`.
 *
 * Skips anything outside the alphabet (including `=`, whitespace, newlines),
 * accepts the URL-safe alphabet, and drops a trailing group of fewer than 2
 * characters — which carries no whole byte. `atob` rejects all of this.
 */
function base64ToBytes(input) {
    const symbols = [];
    for (const char of input) {
        const value = BASE64_LOOKUP.get(char);
        if (value !== undefined)
            symbols.push(value);
    }
    const byteLength = Math.floor((symbols.length * 6) / 8);
    const out = new Uint8Array(byteLength);
    let outIndex = 0;
    let accumulator = 0;
    let bits = 0;
    for (const symbol of symbols) {
        accumulator = (accumulator << 6) | symbol;
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            out[outIndex++] = (accumulator >> bits) & 0xff;
        }
    }
    return out;
}
function bytesToBase64(bytes, urlSafe) {
    let out = '';
    for (let i = 0; i < bytes.length; i += 3) {
        const b0 = bytes[i] ?? 0;
        const b1 = bytes[i + 1];
        const b2 = bytes[i + 2];
        out += BASE64_ALPHABET[b0 >> 2];
        out += BASE64_ALPHABET[((b0 & 0x03) << 4) | ((b1 ?? 0) >> 4)];
        out += b1 === undefined ? (urlSafe ? '' : '=') : BASE64_ALPHABET[((b1 & 0x0f) << 2) | ((b2 ?? 0) >> 6)];
        out += b2 === undefined ? (urlSafe ? '' : '=') : BASE64_ALPHABET[b2 & 0x3f];
    }
    return urlSafe ? out.replace(/-/g, '-').replace(/\+/g, '-').replace(/\//g, '_') : out;
}
/** `Buffer.from(str, 'hex')` — parse pairs, stop at the first invalid one. */
function hexToBytes(input) {
    const out = new Uint8Array(Math.floor(input.length / 2));
    let count = 0;
    for (let i = 0; i + 1 < input.length; i += 2) {
        const byte = Number.parseInt(input.slice(i, i + 2), 16);
        if (Number.isNaN(byte) || !/^[0-9a-fA-F]{2}$/.test(input.slice(i, i + 2)))
            break;
        out[count++] = byte;
    }
    return out.slice(0, count);
}
function bytesToHex(bytes) {
    let out = '';
    for (const byte of bytes)
        out += byte.toString(16).padStart(2, '0');
    return out;
}
/** `latin1`/`ascii` encode: keep the low byte of each UTF-16 code unit. */
function toSingleByte(input) {
    const out = new Uint8Array(input.length);
    for (let i = 0; i < input.length; i++)
        out[i] = input.charCodeAt(i) & 0xff;
    return out;
}
function fromSingleByte(bytes, maskHighBit) {
    let out = '';
    for (const byte of bytes)
        out += String.fromCharCode(maskHighBit ? byte & 0x7f : byte);
    return out;
}
function encodeToBytes(input, encoding) {
    switch (encoding) {
        case 'utf8':
        case 'utf-8':
            return new TextEncoder().encode(input);
        case 'hex':
            return hexToBytes(input);
        case 'base64':
        case 'base64url':
            return base64ToBytes(input);
        // NOT a fallthrough bug: encoding with 'ascii' is identical to 'latin1' in
        // Node — the high-bit masking happens only on DECODE.
        case 'ascii':
        case 'latin1':
            return toSingleByte(input);
    }
}
function decodeToText(bytes, encoding) {
    switch (encoding) {
        case 'utf8':
        case 'utf-8':
            return new TextDecoder('utf-8').decode(bytes);
        case 'hex':
            return bytesToHex(bytes);
        case 'base64':
            return bytesToBase64(bytes, false);
        case 'base64url':
            return bytesToBase64(bytes, true);
        case 'ascii':
            return fromSingleByte(bytes, true);
        case 'latin1':
            return fromSingleByte(bytes, false);
    }
}
/** The browser Buffer host callback. Same shape and same static error as the Node bridge. */
export function browserBufferHandler(req) {
    if (!isSupportedEncoding(req.encoding)) {
        throw new Error('Unsupported Buffer encoding in Safe mode');
    }
    return req.op === 'encode'
        ? { bytes: encodeToBytes(req.input, req.encoding) }
        : { text: decodeToText(req.bytes, req.encoding) };
}
