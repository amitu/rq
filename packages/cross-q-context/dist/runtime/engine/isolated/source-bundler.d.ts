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
/**
 * The cache seam. v1 uses the in-memory default; Slice 3 supplies a disk-backed
 * implementation over `ScriptPackageInstallerService` without touching the
 * bundler. Keyed by a content hash of the source (collision-free for our use).
 */
export interface BundleCache {
    get(key: string): string | undefined;
    set(key: string, bundle: string): void;
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
/**
 * Create a cache-aware SOURCE_BUNDLE bundler.
 *
 * @param cache optional cache; defaults to a fresh in-memory cache (v1). Slice 3
 *              passes a disk-backed `BundleCache`.
 */
export declare function createSourceBundler(cache?: BundleCache): SourceBundler;
