import type { VmPackageEvaluator } from './vm-package-evaluator.js';
import type { PackageResolver } from '../../index.js';
import type { RegistryTimerGlobals } from './registry-aware-builtins.js';
import type { AsyncRegistry } from '../index.js';
/**
 * Creates a require() function to inject into the VM context.
 *
 * Resolution order (7-tier chain):
 * 1. Per-execution cache → hit? return
 * 2. External built-in packages (IIFE-bundled, pre-evaluated in host context)
 * 3. User-authored packages (.js suffix — delegated to vmEvaluator for VM-realm execution)
 * 3.5. Blacklisted packages — blocked by ADR-087 blacklist filter, "not allowed" error
 * 4. PackageResolver (user-installed npm packages) → resolve() !== undefined? cache + return
 * 5. Allowed Node built-ins (13 safe modules — see NODE_BUILTIN_PACKAGES)
 * 6. Error with descriptive message
 *
 * ADR-005 §require() Implementation + ADR-087 §Resolution Order
 * ADR-079 §PackageResolver interface, require chain extension
 *
 * @param userPackages — optional map of package name → raw JS source (from custom package library)
 * @param vmEvaluator — optional evaluator that runs source inside the VM context (CommonJS wrapping, cycle detection)
 * @param resolver — optional PackageResolver for user-installed npm packages (ADR-079)
 * @param contextId — optional context identifier routing resolution to the correct per-script node_modules
 * @param asyncTreatment — the per-execution `AsyncRegistry` plus the registry-backed timer
 *   globals, used to keep a `require()`d built-in's async work visible to the drain
 *   (ADR-219, RQ-5671 Phase 3). Omit only where no registry exists; the built-ins
 *   are then returned raw, which is the pre-Phase-3 behaviour.
 */
export declare function createRequireFn(userPackages?: Record<string, string>, vmEvaluator?: VmPackageEvaluator, resolver?: PackageResolver, contextId?: string, blacklistedPackages?: string[], asyncTreatment?: {
    readonly registry: AsyncRegistry<never>;
    readonly timers: RegistryTimerGlobals;
}): (id: string) => unknown;
