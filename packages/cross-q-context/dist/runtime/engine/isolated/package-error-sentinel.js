export const PACKAGE_ERROR_SENTINEL = Symbol('packageError');
export function isScriptPackageUnsupportedError(err) {
    return (err instanceof Error &&
        PACKAGE_ERROR_SENTINEL in err &&
        'unsupportedReason' in err &&
        'packageId' in err &&
        typeof Reflect.get(err, 'packageId') === 'string' &&
        typeof Reflect.get(err, 'unsupportedReason') === 'string');
}
/** Error subclass carrying the package-error sentinel for internal detection (relocated here off the
 * node:vm-tainted vm-package-evaluator so the Safe require chain stays Node-free). */
class PackageError extends Error {
    [PACKAGE_ERROR_SENTINEL] = true;
    constructor(message, options) {
        super(message, options);
        this.name = 'PackageError';
    }
}
export function createPackageError(message, options) {
    return new PackageError(message, options);
}
