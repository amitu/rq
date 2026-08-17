import { GLOBAL_NAMES } from '../../index.js';
import type { AssertionLibs, DeprecationEmit, RunRequestImpl } from '../../index.js';
import type { EntryType } from '../../index.js';
import type { ScriptExecutionContext, ScriptPhase } from '../../index.js';
import type { CookieJarMutation } from '../host-types.js';
import type { VariableResolver } from '../../definitions/_deps.js';
import type { ExecutionState, SandboxBuildContext } from './types.js';
/**
 * Creates a fresh ExecutionState for a single script execution.
 */
export declare function createExecutionState(): ExecutionState;
/**
 * Returns an object of globals for spreading into vm.createContext().
 * Names come from GLOBAL_NAMES — the single source of truth.
 */
export declare function buildVmGlobals(ctx: SandboxBuildContext): Record<string, unknown>;
/**
 * Returns a record of warn-once deprecation proxies — one per identifier in
 * DEPRECATED_IDENTIFIERS — to spread into vm.createContext() as VM globals.
 *
 * These identifiers (`globals`, `environment`, `tv4`, `Backbone`, …) are bare
 * Postman globals absent from the VM context today, so accessing them currently
 * throws ReferenceError. Injecting a proxy makes the access observable: on first
 * touch it fires `emit` (which the caller wires to a deprecation stream event +
 * a console warning) and then no-ops chainably so the script does not crash.
 *
 * The silent set (`_`, `xml2Json`, `cheerio`) is deliberately absent from
 * DEPRECATED_IDENTIFIERS, so no proxy is created for them.
 */
export declare function buildDeprecationGlobals(emit: DeprecationEmit): Record<string, unknown>;
/**
 * Builds the rq namespace object by calling createRqNamespace from
 * @requestly/sandbox-definitions. Phase restriction: removes entries
 * not allowed in the current phase.
 *
 * Returns both the rq object and a `drainCookieMutations` accessor — the
 * caller (node-sandbox) drains after script execution and emits the list on
 * the final `result` event (ADR-105 Step 21 persistence half).
 */
export interface BuildRqResult {
    rq: Record<string, unknown>;
    drainCookieMutations: () => readonly CookieJarMutation[];
}
export declare function buildRq(executionState: ExecutionState, libs: AssertionLibs, phase: ScriptPhase, context: ScriptExecutionContext, entryType?: EntryType, runRequestImpl?: RunRequestImpl, fetchImpl?: typeof fetch, dynamicVariableResolvers?: ReadonlyArray<VariableResolver>): BuildRqResult;
/** Re-export for node-sandbox to build host object dynamically */
export { GLOBAL_NAMES };
