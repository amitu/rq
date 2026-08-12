import type { InstallPackageSpec } from './install-types.js';
export type DuplicatePackageError = {
    readonly kind: 'version-conflict';
    readonly packageName: string;
    readonly specifiers: readonly string[];
} | {
    readonly kind: 'unversioned-vs-versioned';
    readonly packageName: string;
    readonly specifiers: readonly string[];
};
export interface ExtractedRequire {
    readonly rawId: string;
}
export type ValidateRequiresResult = {
    readonly ok: true;
    readonly value: readonly InstallPackageSpec[];
} | {
    readonly ok: false;
    readonly error: DuplicatePackageError;
};
export declare function validateRequires(requires: readonly ExtractedRequire[]): ValidateRequiresResult;
