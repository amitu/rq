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
/** Sentinel property on errors that have already been attributed to a package. */
import { PACKAGE_ERROR_SENTINEL } from '../isolated/package-error-sentinel.js';
export { PACKAGE_ERROR_SENTINEL };
/** Error subclass that carries the package-error sentinel for internal detection. */
class PackageError extends Error {
    [PACKAGE_ERROR_SENTINEL] = true;
    constructor(message, options) {
        super(message, options);
        this.name = 'PackageError';
    }
}
/** Creates an error pre-marked with the sentinel so the evaluator passes it through unchanged. */
export function createPackageError(message, options) {
    return new PackageError(message, options);
}
/** Number of boilerplate lines the CommonJS wrapper adds before user code. */
const WRAPPER_LINE_OFFSET = 1;
/**
 * Adjusts a line number from a VM error to refer to the user's source code.
 * The CommonJS wrapper adds WRAPPER_LINE_OFFSET lines before user code.
 */
function adjustLineNumber(message) {
    return message.replace(/\bline (\d+)\b/g, (_match, lineStr) => {
        const adjusted = parseInt(lineStr, 10) - WRAPPER_LINE_OFFSET;
        return `line ${adjusted > 0 ? adjusted : 1}`;
    });
}
/**
 * Formats the import chain context for error messages.
 * - Empty stack: no context (direct call from script)
 * - Single parent: `(imported by 'parent')`
 * - Multiple parents: `(import chain: a → b)`
 */
function formatImportChain(evaluationStack) {
    if (evaluationStack.length === 0)
        return '';
    if (evaluationStack.length === 1)
        return ` (imported by '${evaluationStack[0]}')`;
    return ` (import chain: ${evaluationStack.join(' \u2192 ')})`;
}
/**
 * Creates a VmPackageEvaluator bound to the given VM context.
 *
 * The evaluator maintains an evaluation stack for cycle detection and produces
 * attributed error messages when package evaluation fails.
 *
 * Construction sequence (ADR-087):
 *   vmContext → vmEvaluator → requireFn → vmContext.require = requireFn
 */
export function createVmEvaluator(vmContext) {
    const evaluationStack = [];
    return function evaluate(name, source) {
        // Cycle detection: if the package is already being evaluated, we have a circular dependency.
        if (evaluationStack.includes(name)) {
            throw createPackageError(`Circular dependency detected: ${[...evaluationStack, name].join(' \u2192 ')}`);
        }
        evaluationStack.push(name);
        try {
            // Create a fresh module object for this package.
            const mod = { exports: {} };
            // Save/restore vmContext.module so nested evaluations don't clobber the outer package's module reference.
            const prevModule = vmContext['module'];
            vmContext['module'] = mod;
            try {
                // CommonJS wrapper: require is NOT passed as a parameter — it's already a VM global.
                // The wrapped code accesses require, rq, console, and all sandbox globals as free variables.
                const wrapped = `(function(module, exports) {\n${source}\n})(module, module.exports);`;
                vm.runInContext(wrapped, vmContext);
            }
            catch (err) {
                // If the error was already attributed by a nested evaluator call, re-throw as-is.
                // Only the innermost evaluator wraps the error — outer levels propagate it unchanged.
                if (err instanceof Error && PACKAGE_ERROR_SENTINEL in err) {
                    throw err;
                }
                const chain = formatImportChain(evaluationStack.slice(0, -1));
                const originalMessage = err instanceof Error ? err.message : String(err);
                const adjustedMessage = adjustLineNumber(originalMessage);
                throw new PackageError(`Error in package '${name}'${chain}: ${adjustedMessage}`, { cause: err });
            }
            finally {
                vmContext['module'] = prevModule;
            }
            return mod.exports;
        }
        finally {
            evaluationStack.pop();
        }
    };
}
