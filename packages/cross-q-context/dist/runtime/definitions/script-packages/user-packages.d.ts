/** File extension that discriminates user-authored packages from built-ins and npm packages. */
export declare const USER_PACKAGE_EXTENSION = ".js";
/** Check if a require ID refers to a user-authored package (ends with .js). */
export declare function isUserPackageRequire(id: string): boolean;
/** Convert a package name to its require ID (append .js). */
export declare function toUserPackageRequireId(name: string): string;
/** Extract the package name from a require ID (strip .js). Throws if the ID is not a user package require. */
export declare function extractPackageName(id: string): string;
