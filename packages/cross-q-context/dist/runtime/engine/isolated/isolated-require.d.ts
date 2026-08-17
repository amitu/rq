/**
 * isolated-require — the in-isolate require() chain (ADR-010 §82, the dispatch point).
 *
 * The Safe-mode analogue of `createRequireFn` (ADR-005). It carries that chain's
 * tier *shape* but changes the last inch: instead of returning host-realm live
 * objects, it routes each package to its `safeModeClass` column —
 *   needs_bridge → the pre-installed in-isolate module global (Buffer, crypto, …)
 *   source_bundle / user package → host-bundled source, eval'd IN-ISOLATE
 *   impossible → the guided error (ADR-010 §77)
 *
 * Data flow (the only flow that works — an isolate has no module system, so
 * nothing `import`s sandbox-definitions inside the isolate): the HOST reads the
 * registry, bakes a `packageId → safeModeClass/reason` map into the bundle-require
 * callback's closure, and installs:
 *   - `__rq_bundleRequire(id)` — a host callback that, given a require id,
 *     returns `{ kind: 'bridge', global } | { kind: 'source', code } | throws`.
 *     Only copied data crosses (a small record or a thrown copied error).
 *   - REQUIRE_ISOLATE_SHIM — in-isolate JS building `globalThis.require` on top
 *     of that callback, with an in-isolate module cache and CommonJS eval for
 *     source bundles. The bundle eval happens INSIDE the isolate (HARD INVARIANT).
 */
import type { SafePackageResolver, ScriptPackageUnsupportedReason } from '../../index.js';
import type { SourceBundler } from './source-bundler.js';
/** What the host bundle-require callback returns to the in-isolate require. */
type BundleRequireResult = {
    readonly kind: 'bridge';
    readonly global: string;
} | {
    readonly kind: 'iife';
    readonly code: string;
    readonly globalName: string;
} | {
    readonly kind: 'source';
    readonly code: string;
} | {
    readonly kind: 'impossible';
    readonly reason: ScriptPackageUnsupportedReason;
    readonly packageId: string;
};
/**
 * Inputs the host needs to resolve a require id to a bundle-or-bridge result.
 * The engine supplies these per execution.
 */
export interface BundleRequireDeps {
    /** Host-side cache-aware bundler (SOURCE_BUNDLE). */
    readonly bundler: SourceBundler;
    /** Raw source for user-authored packages, keyed by name (ADR-087). */
    readonly userPackages: Record<string, string> | undefined;
    /** Package names blocked by the blacklist filter (ADR-087 Tier 3.5). */
    readonly blacklistedPackages: readonly string[];
    /**
     * Returns the BUILD-TIME IIFE string for a SOURCE_BUNDLE built-in package id
     * (from `VENDOR_IIFES`), or undefined if not present. Built-ins (chai, uuid,
     * moment, …) are bundled at build time — they are sandbox-node
     * devDependencies, NOT installed at runtime, so they CANNOT be resolved/bundled
     * at execution time (the desktop worker's node_modules has no chai). Reusing
     * the pre-generated IIFE (ADR-003's machinery, which ADR-010 §24 extends) is
     * the correct path: no runtime resolution, ships with the engine.
     */
    readonly vendorIife: (id: string) => string | undefined;
    /** Directory esbuild resolves a user package's own `require()`s from. */
    readonly userPackageResolveDir: string;
    /**
     * Resolver for user-INSTALLED npm packages (ADR-010 §85, Tier 4). Undefined on
     * clients without a package store (e.g. the in-process CLI posture). Safe mode
     * reads the entry PATH (`resolveEntryPath`) and bundles it — never the live
     * object `resolve()` returns (that would breach the HARD INVARIANT).
     */
    readonly resolver: SafePackageResolver | undefined;
    /**
     * Per-execution context id routing resolution to the right node_modules
     * (mirrors NodeSandbox's `${pre|post}-${entryId}`). Undefined ⇒ no resolution.
     */
    readonly contextId: string | undefined;
}
/**
 * A pre-built id→result map produced by `prebundleRequires` BEFORE the isolate
 * runs. The sync require callback reads from this — it never bundles inline,
 * because esbuild's only worker-safe API is async (`buildSync` deadlocks in the
 * Electron sandbox worker, RQ-3359).
 */
export type PrebundledRequires = ReadonlyMap<string, BundleRequireResult>;
/**
 * Pre-bundle pass (ADR-010 §82, RQ-3359). Runs BEFORE the isolate executes:
 * classifies each require id the script needs and, for the ones that require an
 * esbuild bundle, does it ASYNC (`build()`, never `buildSync` — the latter
 * deadlocks in the Electron sandbox worker). Returns an id→result map the sync
 * require callback then reads. Bridge/IIFE ids are resolved here too so the map
 * is the single thing the callback consults.
 *
 * A bundle failure (native `.node` addon, unresolvable import) maps to a guided
 * IMPOSSIBLE result baked into the map; a not-found id throws during classify and
 * is recorded as a deferred throw the callback re-raises (so the error surfaces
 * at the require call site, attributed, not at pre-bundle time).
 */
export declare function prebundleRequires(ids: readonly string[], deps: BundleRequireDeps): Promise<PrebundledRequires>;
/**
 * The host-side bundle-require callback body (SYNC — it backs an `ivm.Callback`).
 * Reads the pre-built map first; for bridge/IIFE ids it can also resolve inline
 * (no async needed). Anything that would require a runtime bundle but isn't in
 * the map (e.g. a dynamic require the pre-pass couldn't statically see, or one
 * whose bundle failed) throws the guided IMPOSSIBLE error. NEVER bundles inline.
 */
export declare function resolveRequire(id: string, deps: BundleRequireDeps, prebuilt: PrebundledRequires): BundleRequireResult;
/**
 * Statically extract the require() ids in a source string — the input to the
 * pre-bundle pass. Matches `require('x')` / `require("x")` string literals,
 * including ones containing escaped quotes (`require('it\\'s')`) — the literal is
 * parsed in full and the escape sequences are unescaped so the returned id is the
 * runtime string value. Dynamic / template-literal requires (`require(expr)`,
 * `require(\`${x}\`)`) are intentionally NOT matched: they cannot be statically
 * pre-bundled and correctly fail-and-map at the call site (Safe-mode scripts use
 * literal requires; the build-time editor types only complete literals).
 */
export declare function extractRequireIds(source: string): string[];
/**
 * In-isolate JS: builds `globalThis.require` on top of `__rq_bundleRequire`. Holds
 * an in-isolate module cache. Three result kinds, all eval'd/resolved INSIDE the
 * isolate (HARD INVARIANT — only host data strings cross in, never live objects):
 *   - `bridge` → return the named pre-installed in-isolate global (Buffer, …).
 *   - `iife`   → eval the build-time IIFE (assigns `var __name` on the isolate
 *                global) and return `globalThis[globalName]` (chai/uuid/…).
 *   - `source` → CommonJS-wrap + eval a runtime-bundled user package.
 */
export { REQUIRE_ISOLATE_SHIM } from './shims/require.shim.js';
