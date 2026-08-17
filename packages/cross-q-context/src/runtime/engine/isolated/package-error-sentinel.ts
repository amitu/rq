/**
 * The `PackageError` brand + factory, in its own Node-free module.
 *
 * In the app this brand lived in `vm-package-evaluator.ts`, which imports `node:vm` — tainting the
 * whole Safe require chain with a Developer-engine dependency. cross-q-context relocates
 * `createPackageError` here (alongside the sentinel) so `isolated-require` / `impossible-error` pull
 * it from a genuinely Node-free module. Keep this file import-free except the reason type.
 */
import type { ScriptPackageUnsupportedReason } from '../../index.js';

export const PACKAGE_ERROR_SENTINEL = Symbol('packageError');

export interface ScriptPackageUnsupportedError extends Error {
  readonly [PACKAGE_ERROR_SENTINEL]: true;
  readonly unsupportedReason: ScriptPackageUnsupportedReason;
  readonly packageId: string;
}

export function isScriptPackageUnsupportedError(err: unknown): err is ScriptPackageUnsupportedError {
  return (
    err instanceof Error &&
    PACKAGE_ERROR_SENTINEL in err &&
    'unsupportedReason' in err &&
    'packageId' in err &&
    typeof Reflect.get(err, 'packageId') === 'string' &&
    typeof Reflect.get(err, 'unsupportedReason') === 'string'
  );
}

/** Error subclass carrying the package-error sentinel for internal detection (relocated here off the
 * node:vm-tainted vm-package-evaluator so the Safe require chain stays Node-free). */
class PackageError extends Error {
  readonly [PACKAGE_ERROR_SENTINEL] = true;

  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'PackageError';
  }
}

export function createPackageError(message: string, options?: ErrorOptions): PackageError {
  return new PackageError(message, options);
}
