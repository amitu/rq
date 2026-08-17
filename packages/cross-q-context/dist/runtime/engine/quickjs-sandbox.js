/**
 * `QuickJsSandbox` — the **Node host** for the Safe engine (ADR-204).
 *
 * The engine itself now lives in `./engine.ts` and is host-agnostic: it takes a
 * `QuickJsHostConfig` and imports no `node:*`. This file is the thin Node half —
 * it supplies the CJS QuickJS variant, the four Node-backed capability bridges,
 * the Node fetch bridge (which keeps its direct `globalThis.fetch` fallback), and
 * the full `require()` chain including the SOURCE_BUNDLE tier.
 *
 * The class keeps its original name and constructor shape deliberately, so every
 * existing construction site — and all 33 test files — are unchanged. That is what
 * made the extraction verifiable: the whole suite passes with zero test edits.
 *
 * The browser half is `@requestly/sandbox-browser`, which supplies a different
 * config to the same engine.
 */
import asyncifyVariant from '@jitl/quickjs-singlefile-cjs-release-asyncify';
import { newQuickJSAsyncWASMModuleFromVariant } from 'quickjs-emscripten-core';
import { VENDOR_IIFES } from './vendor-codegen/vendor-iifes.js';
import { dlog } from './isolated/debug-log.js';
import { PHASE_DESCRIPTORS } from '../index.js';
import { QuickJsEngine } from './engine.js';
import { ISOLATE_SHIMS } from './isolated/isolate-shim-order.js';
import { createFetchBridge } from './fetch-bridge.js';
import { createBufferBridge } from './isolated/bridges/buffer-bridge.js';
import { createCryptoBridge } from './isolated/bridges/crypto-bridge.js';
import { createUtilBridge } from './isolated/bridges/util-bridge.js';
import { createZlibBridge } from './isolated/bridges/zlib-bridge.js';
import { REQUIRE_ISOLATE_SHIM, extractRequireIds, prebundleRequires, resolveRequire, } from './isolated/isolated-require.js';
import { createSourceBundler } from './isolated/source-bundler.js';
/** The Node-backed value bridges (Buffer/crypto/util/zlib) the engine installs unconditionally. */
const VALUE_BRIDGE_FACTORIES = { createBufferBridge, createCryptoBridge, createUtilBridge, createZlibBridge };
/**
 * Build-time IIFE lookup for SOURCE_BUNDLE built-ins. `VENDOR_IIFES` is generated
 * keyed by `ExternalBuiltinPackageId`; widen to a plain string index so the require
 * chain can look up by raw require id without an unsafe cast.
 */
const vendorIifeLookup = VENDOR_IIFES;
/**
 * Anchor for esbuild when bundling a USER package's own `require()`s — MODULE-
 * relative, never `process.cwd()` (the desktop sandbox worker's cwd is
 * `clients/desktop`, not the repo). SOURCE_BUNDLE built-ins do NOT use this — they
 * resolve via the build-time IIFE (`VENDOR_IIFES`), since they are devDependencies
 * absent from the runtime node_modules.
 */
const moduleAnchorDir = typeof __dirname !== 'undefined'
    ? __dirname
    : typeof import.meta !== 'undefined' && import.meta.dirname
        ? import.meta.dirname
        : process.cwd();
/** The Node host's config — everything the engine used to import directly. */
export const NODE_QUICKJS_HOST = {
    createModule: () => newQuickJSAsyncWASMModuleFromVariant(asyncifyVariant),
    createRequireSupport: ({ resolver, bundleCache }) => ({
        isolateShim: REQUIRE_ISOLATE_SHIM,
        prepare: async (input) => {
            const bundler = createSourceBundler(bundleCache);
            const requireDeps = {
                bundler,
                userPackages: input.userPackages,
                blacklistedPackages: input.blacklistedPackages ?? [],
                vendorIife: (id) => vendorIifeLookup[id],
                userPackageResolveDir: moduleAnchorDir,
                resolver,
                // Was a `preRequest ? 'pre' : 'post'` ternary, which labelled every other
                // phase 'post' — silently, with no compile error. The descriptor makes the
                // lookup total (on-message gets its own prefix).
                contextId: `${PHASE_DESCRIPTORS[input.phase].contextIdPrefix}-${input.entryId}`,
            };
            // Pre-bundle pass (RQ-3359): esbuild's only worker-safe API is async, but the
            // require callback is sync — so bundle everything the script needs BEFORE the
            // guest runs, into a map the sync callback reads.
            const requireIdSources = [input.script, ...Object.values(input.userPackages ?? {})];
            const requiredIds = requireIdSources.flatMap((src) => extractRequireIds(src));
            dlog('run', 'pre-bundling requires', { ids: requiredIds });
            const prebuilt = await prebundleRequires(requiredIds, requireDeps);
            dlog('run', 'pre-bundle complete', { resolved: prebuilt.size });
            return { resolve: (id) => resolveRequire(id, requireDeps, prebuilt) };
        },
    }),
    valueBridgeFactories: Object.values(VALUE_BRIDGE_FACTORIES),
    createFetchBridge: (host) => createFetchBridge(host),
    isolateShims: ISOLATE_SHIMS,
};
/**
 * The Safe engine, wired to the Node host. Constructor signature is unchanged from
 * before the ADR-204 extraction.
 */
export class QuickJsSandbox extends QuickJsEngine {
    constructor(resolver, bundleCache) {
        super(resolver, bundleCache, NODE_QUICKJS_HOST);
    }
}
