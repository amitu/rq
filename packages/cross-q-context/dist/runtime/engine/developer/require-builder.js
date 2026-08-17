import { builtinModules, createRequire } from 'node:module';
import { EXTERNAL_BUILTIN_PACKAGES, NODE_BUILTIN_PACKAGES, extractPackageName, isUserPackageRequire, parseRequireId, } from '../../index.js';
import { createPackageError } from './vm-package-evaluator.js';
import { applyDeveloperAsyncTreatment } from './registry-aware-builtins.js';
import { VENDOR_IIFES } from '../index.js';
// Node's real require — used to resolve Node built-ins that IIFE packages
// depend on at runtime. Use process.cwd() as the base — this module is always
// consumed in Node.js (Electron main, API server) and esbuild bundles it as
// CJS where import.meta.url is empty anyway.
const nodeRequire = createRequire(`file://${process.cwd()}/`);
// Safe Node built-ins available via require() in user scripts.
// Derived from NODE_BUILTIN_PACKAGES (single source of truth in sandbox-definitions).
// Both bare ('crypto') and node:-prefixed ('node:crypto') forms are supported.
// SECURITY: Only modules with no filesystem, network, process, or code execution
// side effects. Dangerous built-ins (child_process, fs, net, vm, etc.) are blocked.
// See ADR-005 for the full security policy and blocked list.
const ALLOWED_NODE_BUILTINS = new Set(NODE_BUILTIN_PACKAGES.flatMap((p) => [p.id, `node:${p.id}`]));
// Bare and `node:`-prefixed id → the module's declared Developer async treatment
// (ADR-219, RQ-5671 Phase 3). Tier 5 returns the REAL Node module, so a module
// that can start async work must be wrapped or its work is invisible to the drain.
const DEVELOPER_ASYNC_BY_ID = new Map(NODE_BUILTIN_PACKAGES.flatMap((p) => [
    [p.id, p.developerAsync],
    [`node:${p.id}`, p.developerAsync],
]));
// Blocked Node built-ins — dangerous modules NOT in the safe set.
// Used to give a specific "not allowed" error instead of the generic "not available".
const safeNodeBuiltinIds = new Set(NODE_BUILTIN_PACKAGES.map((p) => p.id));
const BLOCKED_NODE_BUILTINS = new Set(builtinModules.filter((m) => !m.startsWith('_') && !safeNodeBuiltinIds.has(m)).flatMap((m) => [m, `node:${m}`]));
// Pre-compute lookup map: package id → { iife, globalName }
const packageMap = new Map();
for (const pkg of EXTERNAL_BUILTIN_PACKAGES) {
    const iife = VENDOR_IIFES[pkg.id];
    if (iife) {
        packageMap.set(pkg.id, { iife, globalName: pkg.globalName });
    }
}
/**
 * Pre-evaluates all IIFE packages in the host (Node.js) context.
 * This runs once at module load — packages are evaluated in the full Node.js
 * environment where Buffer, process, ReadableStream, etc. are all available.
 * Results are cached and returned by the require() function injected into the VM.
 */
const hostModules = new Map();
for (const [id, pkg] of packageMap) {
    const originalRequire = globalThis['require'];
    try {
        // Evaluate in the global scope via indirect eval so `var __pkg = ...`
        // creates a property on globalThis (even in strict mode, indirect eval
        // runs in the global scope where var declarations become global properties).
        // The IIFE needs `require` for Node built-in dependencies.
        globalThis['require'] = nodeRequire;
        // Strip leading "use strict"; — in strict mode, var declarations in eval
        // don't create global properties. The IIFE internals are already strict.
        const iifeCode = pkg.iife.replace(/^"use strict";/, '');
        (0, eval)(iifeCode);
        const mod = globalThis[pkg.globalName];
        if (mod !== undefined) {
            hostModules.set(id, mod);
        }
    }
    catch (err) {
        // Package failed to evaluate — will be unavailable via require()
        // eslint-disable-next-line no-console -- Error observability: log failed package evaluation so failures are diagnosable
        console.error(`Failed to pre-evaluate built-in package "${id}"`, err);
    }
    finally {
        if (originalRequire === undefined) {
            delete globalThis['require'];
        }
        else {
            globalThis['require'] = originalRequire;
        }
    }
}
function mapResolverError(id, err) {
    const message = err instanceof Error ? err.message : String(err);
    const missingMatch = /Cannot find module '(@[^'/]+\/[^'/]+|[^'/]+)'/.exec(message);
    if (missingMatch) {
        const missingDep = missingMatch[1];
        return createPackageError(`Package '${id}' failed to load: missing dependency '${missingDep}'. Add '${missingDep}' to your script imports.`, { cause: err });
    }
    return createPackageError(`Package '${id}' cannot be used.`, { cause: err });
}
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
export function createRequireFn(userPackages, vmEvaluator, resolver, contextId, blacklistedPackages = [], asyncTreatment) {
    const cache = new Map();
    return function require(id) {
        if (typeof id !== 'string') {
            throw new Error('require() argument must be a string');
        }
        if (id === '') {
            throw new Error('Package name cannot be empty.');
        }
        // Tier 1: Return cached module (use has() not !== undefined to handle packages that export falsy values)
        if (cache.has(id)) {
            return cache.get(id);
        }
        // Tier 2: Check if it's a pre-evaluated built-in package
        const mod = hostModules.get(id);
        if (mod !== undefined) {
            cache.set(id, mod);
            return mod;
        }
        // Tier 3: User-authored packages — .js suffix gates this tier (ADR-087).
        //    Both userPackages and vmEvaluator must be present; createRequireFn()
        //    never touches vm.* — it delegates to the evaluator.
        if (isUserPackageRequire(id) && userPackages && vmEvaluator) {
            const name = extractPackageName(id);
            const source = userPackages[name];
            if (source !== undefined) {
                const value = vmEvaluator(name, source);
                cache.set(id, value);
                return value;
            }
            throw createPackageError(`Package '${name}' not found in project. Check the package name or create it in the Package Library.`);
        }
        // Tier 3.5: Blacklisted packages — blocked by the blacklist filter (ADR-087).
        if (blacklistedPackages.length > 0) {
            const parsed = parseRequireId(id);
            if (parsed.packageName && blacklistedPackages.includes(parsed.packageName)) {
                throw createPackageError(`Package '${parsed.packageName}' is not allowed.`);
            }
        }
        // Tier 4: PackageResolver — user-installed npm packages (ADR-079)
        // Returns the module object or undefined (fallthrough sentinel).
        // If the resolver throws, surface the error — it means the package was installed
        // but failed to load (e.g. missing peer dependency).
        if (resolver) {
            try {
                const resolved = resolver.resolve(id, contextId);
                if (resolved !== undefined) {
                    cache.set(id, resolved);
                    return resolved;
                }
            }
            catch (err) {
                throw mapResolverError(id, err);
            }
        }
        // Tier 5: Allow safe Node built-ins (IIFE package dependencies + user-facing modules).
        // SECURITY: Explicit allowlist derived from NODE_BUILTIN_PACKAGES — dangerous
        // modules (child_process, fs, net, vm, etc.) are blocked. See ADR-005.
        if (ALLOWED_NODE_BUILTINS.has(id)) {
            const builtinMod = nodeRequire(id);
            // Route the module's async work through the AsyncRegistry per its declared
            // treatment (ADR-219). Without this a script can bypass the registry-backed
            // globals entirely — `require('timers').setTimeout(cb, 1000)` was the live
            // instance. When no registry is supplied (older callers) the module is
            // returned untouched, which is the pre-Phase-3 behaviour.
            const treated = asyncTreatment === undefined
                ? builtinMod
                : applyDeveloperAsyncTreatment(builtinMod, DEVELOPER_ASYNC_BY_ID.get(id) ?? 'not-an-async-source', asyncTreatment.registry, asyncTreatment.timers);
            cache.set(id, treated);
            return treated;
        }
        // Tier 6a: Blocked Node built-in — give a specific security message.
        const bareId = id.startsWith('node:') ? id.slice(5) : id;
        if (BLOCKED_NODE_BUILTINS.has(id) || BLOCKED_NODE_BUILTINS.has(bareId)) {
            throw createPackageError(`Package '${bareId}' cannot be used.`);
        }
        // Tier 6b: Unknown package — throw descriptive error including the package name.
        throw createPackageError(`Package '${id}' not found.`);
    };
}
