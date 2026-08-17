import type { QuickJSAsyncWASMModule } from 'quickjs-emscripten-core';
export declare function getQuickJsModule(): Promise<QuickJSAsyncWASMModule>;
/**
 * Test seam — drops the memoized module so a test can assert cold-start behavior.
 * Not part of the production surface.
 */
export declare function resetQuickJsModuleForTesting(): void;
