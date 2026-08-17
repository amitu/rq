/**
 * Wire shape, identical to the Node bridge's `CryptoOp` / `CryptoResult`. Kept
 * structurally in sync by `__tests__/crypto.parity.test.ts`, which drives BOTH
 * handlers with the same inputs and compares outputs byte-for-byte.
 */
export type CryptoOp = {
    readonly op: 'hash';
    readonly algo: string;
    readonly data: Uint8Array;
    readonly outputEncoding: 'hex' | 'base64';
} | {
    readonly op: 'hmac';
    readonly algo: string;
    readonly key: Uint8Array;
    readonly data: Uint8Array;
    readonly outputEncoding: 'hex' | 'base64';
} | {
    readonly op: 'randomBytes';
    readonly size: number;
} | {
    readonly op: 'randomUUID';
};
export type CryptoResult = {
    readonly digest: string;
} | {
    readonly bytes: Uint8Array;
} | {
    readonly uuid: string;
};
/**
 * The browser crypto host callback. Same discriminated `op` switch as the Node
 * bridge; same static error messages (`gr-static-error-messages`).
 */
export declare function browserCryptoHandler(req: CryptoOp): CryptoResult;
