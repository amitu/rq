import {
  createDeprecationProxy,
  createRqNamespace,
  DEPRECATED_IDENTIFIERS,
  GLOBAL_NAMES,
  PHASE_RESTRICTED,
} from '../../index.js';
import type { AssertionLibs, DeprecationEmit, RunRequestImpl } from '../../index.js';

import type { EntryType } from '../../index.js';
import type { ScriptExecutionContext, ScriptPhase } from '../../index.js';
import type { CookieJarMutation } from '../host-types.js';

import type { VariableResolver } from '../../definitions/_deps.js';

import { createInMemoryCookieJarBridge } from '../index.js';
import { registerDynamicVariables } from './dynamic-variables.js';
import type { ExecutionState, SandboxBuildContext } from './types.js';

/**
 * Creates a fresh ExecutionState for a single script execution.
 */
export function createExecutionState(): ExecutionState {
  return { testResults: [], rawMutations: {}, requestMutations: [] };
}

/**
 * Returns an object of globals for spreading into vm.createContext().
 * Names come from GLOBAL_NAMES — the single source of truth.
 */
export function buildVmGlobals(ctx: SandboxBuildContext): Record<string, unknown> {
  const globals: Record<string, unknown> = {};
  for (const name of GLOBAL_NAMES) {
    globals[name] = ctx.host[name];
  }
  return globals;
}

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
export function buildDeprecationGlobals(emit: DeprecationEmit): Record<string, unknown> {
  const globals: Record<string, unknown> = {};
  for (const identifier of Object.keys(DEPRECATED_IDENTIFIERS)) {
    globals[identifier] = createDeprecationProxy(identifier, emit);
  }
  return globals;
}

/**
 * Returns whether an entry is allowed for the given phase.
 */
function isEntryAllowed(entryName: string, phase: ScriptPhase): boolean {
  const allowedPhases = PHASE_RESTRICTED[entryName];
  if (!allowedPhases) return true;
  return allowedPhases.includes(phase);
}

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

export function buildRq(
  executionState: ExecutionState,
  libs: AssertionLibs,
  phase: ScriptPhase,
  context: ScriptExecutionContext,
  entryType?: EntryType,
  // Per-execution host round-trip for rq.execution.runRequest (ADR-169). When
  // supplied, the Developer engine threads it through to createRqNamespace so
  // rq.execution.runRequest is present; when undefined, runRequest is absent.
  runRequestImpl?: RunRequestImpl,
  // The `fetch` backing rq.sendRequest. The caller passes an SSRF-guarded fetch
  // (RQ-3902) so rq.sendRequest cannot reach the metadata server; defaults to the
  // raw host fetch only when a caller omits it (tests).
  fetchImpl: typeof fetch = globalThis.fetch,
  // The dynamic-variable resolvers ($guid/$randomInt/faker.*). INJECTED — cross-q-context stays
  // free of the faker catalog; the host (the app) passes its resolver. Empty ⇒ no dynamic vars.
  dynamicVariableResolvers: ReadonlyArray<VariableResolver> = [],
): BuildRqResult {
  // ADR-105: per-execution in-memory jar. `jar(host)` enforces the allowlist
  // in sandbox-definitions; the bridge only sees already-allowed hosts. The
  // handle tracks an ordered mutation log that node-sandbox drains into the
  // script result so the runtime can persist them to CookieRepository.
  // The optional `cookieJarSeed` from context lets `list/get/getAll` see
  // cookies persisted by prior executions or captured Set-Cookie responses.
  const cookieBridgeHandle = createInMemoryCookieJarBridge(context.cookieJarSeed as unknown as Parameters<typeof createInMemoryCookieJarBridge>[0]);
  const rq = createRqNamespace(
    executionState,
    libs,
    context,
    phase,
    cookieBridgeHandle.bridge,
    entryType,
    // fetchImpl precedes runRequestImpl positionally; pass the guarded fetch
    // (RQ-3902) so the trailing runRequestImpl lands correctly.
    fetchImpl,
    runRequestImpl,
  );
  const result: Record<string, unknown> = { ...rq };

  // Phase restriction: delete entries not allowed in this phase
  for (const entryName of Object.keys(result)) {
    if (!isEntryAllowed(entryName, phase)) {
      delete result[entryName];
    }
  }

  // ADR-055: Register dynamic variables ($guid, $randomInt, etc.) on the rq object.
  // Resolvers are constructed in-process (ADR-034 — never serialized across boundaries).
  registerDynamicVariables(result, dynamicVariableResolvers);

  return {
    rq: result,
    drainCookieMutations: () => cookieBridgeHandle.drainMutations(),
  };
}

/** Re-export for node-sandbox to build host object dynamically */
export { GLOBAL_NAMES };
