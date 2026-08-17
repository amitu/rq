/**
 * source-bundler — SOURCE_BUNDLE host-side bundling + in-isolate eval (ADR-010 §34/§82).
 *
 * Pure-JS packages run in Safe mode by being bundled host-side (Rollup WASM, CJS)
 * and eval'd INSIDE the isolate — never pre-evaluated host-side as a live object
 * the way ADR-005's `hostModules` does (that live object is the cross-realm
 * reference the HARD INVARIANT forbids). This is the "last inch" change from the
 * Developer-mode require path.
 *
 * Cache-aware (ADR-010 §78, 2026-06-12 decision): `createSourceBundler(cache?)`
 * returns a `bundleSource` that keys on a content hash and reuses a prior bundle.
 * The default cache is in-memory (v1); the `BundleCache` interface is the seam
 * where Slice 3 plugs the `ScriptPackageInstallerService` disk cache — no rewrite
 * of this module needed then.
 *
 * Uses `@rollup/wasm-node` (WASM variant, no native binary) instead of esbuild.
 * Rollup WASM's 548KB binary is base64-inlined at build time so this module can
 * be bundled into a single CJS file by esbuild — the property esbuild itself
 * lacks (it throws "cannot be bundled" because it spawns a native Go binary).
 * This unblocks the CLI single-file bundle for Safe mode (ADR-111).
 */
import { createHash } from 'node:crypto';
import path from 'node:path';
import commonjs from '@rollup/plugin-commonjs';
import json from '@rollup/plugin-json';
import nodeResolve from '@rollup/plugin-node-resolve';
import { rollup } from '@rollup/wasm-node';
import { dlog } from './debug-log.js';
/** Default in-memory cache — one per engine/bundler instance. */
class InMemoryBundleCache {
    store = new Map();
    get(key) {
        return this.store.get(key);
    }
    set(key, bundle) {
        this.store.set(key, bundle);
    }
}
function hashKey(...parts) {
    const h = createHash('sha256');
    for (const p of parts)
        h.update(p).update('\0');
    return h.digest('hex');
}
const OUTPUT_OPTIONS = { format: 'cjs', exports: 'auto' };
// The @rollup/plugin-* packages peer-depend on `rollup` (native), whose Plugin
// type diverges from @rollup/wasm-node's: native rollup exposes PluginContext.fs,
// wasm-node omits it. At runtime the plugins never use the fs field so behavior
// is identical — spike-verified across ms/qs/nanoid/crypto-js/lodash.
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- cross-package rollup type bridge
function createPlugins() {
    // The plugins' default-export types don't surface a call signature under NodeNext, though the
    // esModuleInterop default import IS the callable factory at runtime. Cast to call.
    return [
        nodeResolve({ preferBuiltins: true }),
        commonjs(),
        json(),
    ];
}
/**
 * Virtual-entry plugin for bundling raw user-authored package source. Creates a
 * virtual module from `source`; bare imports inside it resolve relative to
 * `resolveDir` (via a synthesized importer path) so `require('lodash')` in a
 * user package finds the per-context node_modules.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- rollup Plugin type bridge
function stdinPlugin(source, resolveDir) {
    const VIRTUAL_ID = '\0stdin-entry';
    return {
        name: 'stdin',
        resolveId(importee, importer) {
            if (importee === VIRTUAL_ID)
                return VIRTUAL_ID;
            if (importer === VIRTUAL_ID && !importee.startsWith('.') && !importee.startsWith('\0')) {
                // eslint-disable-next-line @typescript-eslint/no-explicit-any -- rollup PluginContext.resolve
                return this.resolve(importee, path.join(resolveDir, '_virtual_.js'), { skipSelf: true });
            }
            return null;
        },
        load(id) {
            if (id === VIRTUAL_ID)
                return source;
            return null;
        },
    };
}
/** Rollup → CJS bundle string. */
async function runBuild(inputOptions) {
    dlog('bundler', 'rollup build START', { input: JSON.stringify(inputOptions.input) });
    const bundle = await rollup(inputOptions);
    try {
        const { output } = await bundle.generate(OUTPUT_OPTIONS);
        const code = output[0].code;
        if (!code) {
            dlog('bundler', 'rollup produced NO output');
            throw new Error('SOURCE_BUNDLE produced no output');
        }
        dlog('bundler', 'rollup build OK', { bytes: code.length });
        return code;
    }
    finally {
        await bundle.close();
    }
}
/**
 * Create a cache-aware SOURCE_BUNDLE bundler.
 *
 * @param cache optional cache; defaults to a fresh in-memory cache (v1). Slice 3
 *              passes a disk-backed `BundleCache`.
 */
export function createSourceBundler(cache = new InMemoryBundleCache()) {
    return {
        async bundleSource(logicalName, source, resolveDir) {
            const key = hashKey('src', logicalName, source);
            const cached = cache.get(key);
            if (cached !== undefined)
                return cached;
            const bundle = await runBuild({
                input: '\0stdin-entry',
                plugins: [stdinPlugin(source, resolveDir), ...createPlugins()],
                onwarn: () => { },
            });
            cache.set(key, bundle);
            return bundle;
        },
        async bundleEntry(logicalName, entryPath) {
            const key = hashKey('entry', logicalName, entryPath);
            const cached = cache.get(key);
            if (cached !== undefined)
                return cached;
            const bundle = await runBuild({
                input: entryPath,
                plugins: createPlugins(),
                onwarn: () => { },
            });
            cache.set(key, bundle);
            return bundle;
        },
    };
}
