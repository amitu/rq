/**
 * VmPackageEvaluator — Evaluates user-authored package source code inside a VM context.
 *
 * Owns: CommonJS wrapping, vm.runInContext() execution, cycle detection (evaluation stack),
 * and error attribution (package name + import chain in error messages).
 *
 * Does NOT know about resolution, caching, or require(). Those live in createRequireFn().
 *
 * ADR-087 §VmPackageEvaluator
 */
import * as vm from 'node:vm';
/**
 * Evaluates user package source in a VM context. Returns the package's module.exports.
 * @param name — package name (without .js)
 * @param source — raw JavaScript source code
 */
export type VmPackageEvaluator = (name: string, source: string) => unknown;
/** Sentinel property on errors that have already been attributed to a package. */
import { PACKAGE_ERROR_SENTINEL } from '../isolated/package-error-sentinel.js';
export { PACKAGE_ERROR_SENTINEL };
/** Error subclass that carries the package-error sentinel for internal detection. */
declare class PackageError extends Error {
    readonly [PACKAGE_ERROR_SENTINEL] = true;
    constructor(message: string, options?: ErrorOptions);
}
/** Creates an error pre-marked with the sentinel so the evaluator passes it through unchanged. */
export declare function createPackageError(message: string, options?: ErrorOptions): PackageError;
/**
 * Creates a VmPackageEvaluator bound to the given VM context.
 *
 * The evaluator maintains an evaluation stack for cycle detection and produces
 * attributed error messages when package evaluation fails.
 *
 * Construction sequence (ADR-087):
 *   vmContext → vmEvaluator → requireFn → vmContext.require = requireFn
 */
export declare function createVmEvaluator(vmContext: vm.Context): VmPackageEvaluator;
