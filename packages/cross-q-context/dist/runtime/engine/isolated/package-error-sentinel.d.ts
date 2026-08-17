/**
 * The `PackageError` brand + factory, in its own Node-free module.
 *
 * In the app this brand lived in `vm-package-evaluator.ts`, which imports `node:vm` — tainting the
 * whole Safe require chain with a Developer-engine dependency. cross-q-context relocates
 * `createPackageError` here (alongside the sentinel) so `isolated-require` / `impossible-error` pull
 * it from a genuinely Node-free module. Keep this file import-free except the reason type.
 */
import type { ScriptPackageUnsupportedReason } from '../../index.js';
export declare const PACKAGE_ERROR_SENTINEL: unique symbol;
export interface ScriptPackageUnsupportedError extends Error {
    readonly [PACKAGE_ERROR_SENTINEL]: true;
    readonly unsupportedReason: ScriptPackageUnsupportedReason;
    readonly packageId: string;
}
export declare function isScriptPackageUnsupportedError(err: unknown): err is ScriptPackageUnsupportedError;
/** Error subclass carrying the package-error sentinel for internal detection (relocated here off the
 * node:vm-tainted vm-package-evaluator so the Safe require chain stays Node-free). */
declare class PackageError extends Error {
    readonly [PACKAGE_ERROR_SENTINEL] = true;
    constructor(message: string, options?: ErrorOptions);
}
export declare function createPackageError(message: string, options?: ErrorOptions): PackageError;
export {};
