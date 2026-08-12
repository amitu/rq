/**
 * Contract for resolving user-installed npm packages at require() time.
 * Injected into the sandbox's require chain between IIFE built-ins and Node built-ins.
 *
 * Implementations return host-realm module objects (not vm-context-allocated).
 * The resolver is NOT an RPC boundary — it runs in the same process as the sandbox.
 *
 * @see ADR-079 (PackageResolver interface, require chain extension)
 * @see ADR-080 (createRequire delivery strategy)
 */
export interface PackageResolver {
    /**
     * Resolve a package specifier to a module object.
     *
     * Used by the Developer engine (`node:vm`), which shares the host realm and can
     * accept a live host-realm module object. The Safe engine (isolated-vm) must NOT
     * use this — handing a live host object into the isolate violates the HARD
     * INVARIANT (ADR-010). Safe mode uses `SafePackageResolver.resolveEntryPath`.
     *
     * @param id - The raw require() argument (e.g., `'lodash'`, `'lodash/fp'`, `'@faker-js/faker'`)
     *             Version is NOT included — resolution happens against already-installed node_modules.
     * @param contextId - The package context directory (e.g., `'pre-sourceId'`). Identifies which
     *                     per-script node_modules to resolve from. Undefined if no context is available.
     * @returns The module object, or `undefined` if the package is not installed (fallthrough sentinel).
     */
    resolve(id: string, contextId: string | undefined): unknown;
}
/**
 * Safe-mode extension of the resolver contract (ADR-010 §85). Where `resolve`
 * returns a live host-realm object (which the isolate forbids), `resolveEntryPath`
 * returns the package's on-disk ENTRY PATH — SOURCE the isolated engine bundles
 * with esbuild and eval's in-isolate. Kept separate from `PackageResolver` so
 * Developer-only consumers (and their test mocks) need not implement it; a
 * resolver that can serve Safe mode implements BOTH.
 */
export interface SafePackageResolver extends PackageResolver {
    /**
     * Resolve a package specifier to its on-disk entry file path WITHOUT evaluating
     * it (the `require.resolve` analogue of `resolve`). The package is already
     * installed on disk in the per-context node_modules; this returns the path the
     * bundler reads from.
     *
     * CONTRACT (load-bearing for the HARD INVARIANT, ADR-010): implementations MUST
     * return ONLY an absolute path string or `undefined` — never a live host module
     * object, function, or any non-path value. The isolated engine bundles the file
     * at this path and eval's it inside the isolate; it relies on nothing live
     * crossing the boundary. (The isolate engine calls ONLY this method, never the
     * inherited `resolve()`, which intentionally returns live objects for Developer
     * mode.)
     *
     * @param id - The raw require() argument (version stripped internally).
     * @param contextId - The per-script context directory; undefined if unavailable.
     * @returns The absolute entry-file path, or `undefined` if the package is not installed.
     */
    resolveEntryPath(id: string, contextId: string | undefined): string | undefined;
}
