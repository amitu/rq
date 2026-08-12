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
export {};
