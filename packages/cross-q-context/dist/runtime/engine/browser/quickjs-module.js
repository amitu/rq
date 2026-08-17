import asyncifyVariant from '@jitl/quickjs-singlefile-browser-release-asyncify';
import { newQuickJSAsyncWASMModuleFromVariant } from 'quickjs-emscripten-core';
/**
 * Browser host for the QuickJS-WASM Safe engine (ADR-204).
 *
 * This is the SAME engine `modules/sandbox-node` runs — deliberately, and that is
 * the whole point of the package. Script semantics must not fork by surface: V8 is
 * more permissive than QuickJS, so a script authored against a browser-native
 * engine could pass on web and fail on desktop, where Safe (QuickJS) is the
 * DEFAULT engine (`sandbox-node/src/dispatching-sandbox.ts`, the D-13 flip).
 *
 * The only difference from the Node host is the packaged variant: `-browser-`
 * here, `-cjs-` there, pinned to the same version. `variant-pin-parity.test.ts`
 * enforces that equality, because an accidental single-sided bump would silently
 * reintroduce exactly the engine skew this package exists to prevent.
 */
/**
 * Memoized QuickJS-WASM module load. Mirrors the Node host's `getQuickJsModule`:
 * the WASM (base64-embedded in the single-file asyncify variant) compiles once,
 * and every execution reuses the module, creating only a fresh runtime/context.
 *
 * The PROMISE is memoized rather than the resolved value, so concurrent
 * first-callers share one compile instead of racing two.
 *
 * Callers must reach this through a lazy `import()` — the variant inlines ~1.47MB
 * of base64 WASM, and ADR-204 requires it contribute ZERO bytes to the web
 * client's initial bundle (mirroring how `DispatchingSandbox` already lazy-imports
 * the Safe engine).
 */
let quickJsModulePromise;
export function getQuickJsModule() {
    quickJsModulePromise ??= newQuickJSAsyncWASMModuleFromVariant(asyncifyVariant);
    return quickJsModulePromise;
}
/**
 * Test seam — drops the memoized module so a test can assert cold-start behavior.
 * Not part of the production surface.
 */
export function resetQuickJsModuleForTesting() {
    quickJsModulePromise = undefined;
}
