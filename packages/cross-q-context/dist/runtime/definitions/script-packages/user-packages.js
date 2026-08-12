/** File extension that discriminates user-authored packages from built-ins and npm packages. */
export const USER_PACKAGE_EXTENSION = '.js';
/** Check if a require ID refers to a user-authored package (ends with .js). */
export function isUserPackageRequire(id) {
    return id.endsWith(USER_PACKAGE_EXTENSION);
}
/** Convert a package name to its require ID (append .js). */
export function toUserPackageRequireId(name) {
    return `${name}${USER_PACKAGE_EXTENSION}`;
}
/** Extract the package name from a require ID (strip .js). Throws if the ID is not a user package require. */
export function extractPackageName(id) {
    if (!isUserPackageRequire(id)) {
        throw new Error('Cannot extract package name from non-user-package require ID');
    }
    return id.slice(0, -USER_PACKAGE_EXTENSION.length);
}
