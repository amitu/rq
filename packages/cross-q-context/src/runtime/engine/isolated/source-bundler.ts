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

import type { InputOptions, OutputOptions } from '@rollup/wasm-node';

/**
 * The cache seam. v1 uses the in-memory default; Slice 3 supplies a disk-backed
 * implementation over `ScriptPackageInstallerService` without touching the
 * bundler. Keyed by a content hash of the source (collision-free for our use).
 */
export interface BundleCache {
  get(key: string): string | undefined;
  set(key: string, bundle: string): void;
}

/** Default in-memory cache — one per engine/bundler instance. */
class InMemoryBundleCache implements BundleCache {
  private readonly store = new Map<string, string>();

  get(key: string): string | undefined {
    return this.store.get(key);
  }

  set(key: string, bundle: string): void {
    this.store.set(key, bundle);
  }
}

/**
 * A bundler: turns pure-JS package source into a CJS bundle string, cached.
 *
 * Both methods are ASYNC. Because the in-isolate require callback is synchronous,
 * every bundle a script needs is produced by a PRE-BUNDLE PASS before the isolate
 * runs (see `prebundleRequires`); the sync callback then only reads the resulting
 * id→code map.
 */
export interface SourceBundler {
  /**
   * Bundle raw source (a user-authored package, ADR-087) to a self-contained CJS
   * string. `logicalName` labels the sourcefile + namespaces the cache key.
   * `resolveDir` anchors where Rollup resolves the source's own bare `require()`s.
   */
  bundleSource(logicalName: string, source: string, resolveDir: string): Promise<string>;

  /**
   * Bundle an INSTALLED npm package (ADR-010 §85) from its on-disk entry-file
   * path to a self-contained CJS string. Rollup walks the package's dependency
   * tree from `entryPath`. Rejects if Rollup can't bundle it (e.g. native `.node`
   * addon) — the require chain maps the rejection to a guided IMPOSSIBLE error.
   */
  bundleEntry(logicalName: string, entryPath: string): Promise<string>;
}

function hashKey(...parts: string[]): string {
  const h = createHash('sha256');
  for (const p of parts) h.update(p).update('\0');
  return h.digest('hex');
}

const OUTPUT_OPTIONS: OutputOptions = { format: 'cjs', exports: 'auto' };

// The @rollup/plugin-* packages peer-depend on `rollup` (native), whose Plugin
// type diverges from @rollup/wasm-node's: native rollup exposes PluginContext.fs,
// wasm-node omits it. At runtime the plugins never use the fs field so behavior
// is identical — spike-verified across ms/qs/nanoid/crypto-js/lodash.
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- cross-package rollup type bridge
function createPlugins(): Array<any> {
  // The plugins' default-export types don't surface a call signature under NodeNext, though the
  // esModuleInterop default import IS the callable factory at runtime. Cast to call.
  return [
    (nodeResolve as unknown as (opts: { preferBuiltins: boolean }) => unknown)({ preferBuiltins: true }),
    (commonjs as unknown as () => unknown)(),
    (json as unknown as () => unknown)(),
  ];
}

/**
 * Virtual-entry plugin for bundling raw user-authored package source. Creates a
 * virtual module from `source`; bare imports inside it resolve relative to
 * `resolveDir` (via a synthesized importer path) so `require('lodash')` in a
 * user package finds the per-context node_modules.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any -- rollup Plugin type bridge
function stdinPlugin(source: string, resolveDir: string): any {
  const VIRTUAL_ID = '\0stdin-entry';
  return {
    name: 'stdin',
    resolveId(importee: string, importer: string | undefined) {
      if (importee === VIRTUAL_ID) return VIRTUAL_ID;
      if (importer === VIRTUAL_ID && !importee.startsWith('.') && !importee.startsWith('\0')) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- rollup PluginContext.resolve
        return (this as any).resolve(importee, path.join(resolveDir, '_virtual_.js'), { skipSelf: true });
      }
      return null;
    },
    load(id: string) {
      if (id === VIRTUAL_ID) return source;
      return null;
    },
  };
}

/** Rollup → CJS bundle string. */
async function runBuild(inputOptions: InputOptions & { onwarn?: () => void }): Promise<string> {
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
  } finally {
    await bundle.close();
  }
}

/**
 * Create a cache-aware SOURCE_BUNDLE bundler.
 *
 * @param cache optional cache; defaults to a fresh in-memory cache (v1). Slice 3
 *              passes a disk-backed `BundleCache`.
 */
export function createSourceBundler(cache: BundleCache = new InMemoryBundleCache()): SourceBundler {
  return {
    async bundleSource(logicalName: string, source: string, resolveDir: string): Promise<string> {
      const key = hashKey('src', logicalName, source);
      const cached = cache.get(key);
      if (cached !== undefined) return cached;
      const bundle = await runBuild({
        input: '\0stdin-entry',
        plugins: [stdinPlugin(source, resolveDir), ...createPlugins()],
        onwarn: () => {},
      });
      cache.set(key, bundle);
      return bundle;
    },

    async bundleEntry(logicalName: string, entryPath: string): Promise<string> {
      const key = hashKey('entry', logicalName, entryPath);
      const cached = cache.get(key);
      if (cached !== undefined) return cached;
      const bundle = await runBuild({
        input: entryPath,
        plugins: createPlugins(),
        onwarn: () => {},
      });
      cache.set(key, bundle);
      return bundle;
    },
  };
}
