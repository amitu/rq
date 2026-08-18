import { EntryType } from './_deps.js';
import type { EnvironmentVariables } from './_deps.js';
import type {
  ExecutionDirective,
  RequestHeaderMutation,
  ScriptExecutionContext,
  ScriptPhase,
  RawTestResult,
  VisualizerDirective,
} from './_deps.js';

import { createCookiesNamespace } from './cookies.js';
import type { CookieJarBridge } from './cookies.js';
import { createExecutionNamespace } from './execution.js';
import type { ExecutionDirectiveCollector } from './execution.js';
import type { AssertionLibs, RequestMutationCollector } from './requestResponse.js';
import { buildScriptMessage, buildScriptRequest, buildScriptResponse } from './requestResponse.js';
import type { RunRequestImpl } from './runRequest.js';
import { createSendRequest } from './sendRequest.js';
import { createVisualizer } from './visualizer.js';
import type { VisualizerCollector } from './visualizer.js';

// ---------------------------------------------------------------------------
// Raw mutation types — Layer 1 of ADR-053.
// Captures user intent as plain { value, type } entries.
// sandbox-definitions has zero dependency on @requestly/schemas.
// ---------------------------------------------------------------------------

export interface RawMutationEntry {
  value: string;
  type: 'string' | 'number' | 'boolean' | 'array';
}

export type RawScopeMutations = {
  global?: Record<string, RawMutationEntry | null>;
  environment?: Record<string, RawMutationEntry | null>;
  collection?: Record<string, RawMutationEntry | null>;
  runtime?: Record<string, RawMutationEntry | null>;
};

// ---------------------------------------------------------------------------
// Effective value — empty `localValue` means "no override", fall back to `syncValue`.
// Mirrors the helper in packages/variables/src/substitute-templates.ts so the
// script API and URL/param substitution read variables the same way (ADR-024).
// ---------------------------------------------------------------------------

// `localValue` is typed as possibly-absent and guarded, matching the Safe engine's `effective`
// (isolated-rq.ts). Every audited producer synthesizes `localValue: ''` so an absent one is not
// currently reachable, but the unguarded form returned `undefined` here while Safe returned
// `syncValue` — the same input reading differently per engine (RQ-5691). The guard removes the
// divergence rather than relying on every future producer to keep synthesizing the field.
function getEffectiveValue(data: { localValue?: string; syncValue: string }): string {
  return data.localValue !== undefined && data.localValue !== '' ? data.localValue : data.syncValue;
}

// ---------------------------------------------------------------------------
// Type restoration — variable values are stored as strings (syncValue/localValue),
// with the original type recorded alongside. `get` reads the string back and
// coerces to the recorded type so a script reads back what it set: a boolean
// stays a boolean, a number stays a number (RQ-1421). `type` is accepted as a
// plain string so both `RawMutationEntry.type` and `VariableData.type`
// (VariableDataType — string | number | boolean | secret) flow in without an
// enum import (sandbox-definitions must not depend on @requestly/schemas).
// ---------------------------------------------------------------------------

function coerceValueByType(value: string, type: string): string | number | boolean | unknown[] {
  switch (type) {
    case 'number': {
      // Empty string is not a number — return it as-is. `Number('')` is `0`,
      // which would silently fabricate a value; keep the empty string visible.
      if (value === '') return value;
      const n = Number(value);
      // Non-numeric string under a number type → return the raw string rather
      // than NaN/0, so a malformed value is visible instead of silently zeroed.
      return Number.isFinite(n) ? n : value;
    }
    case 'boolean':
      return value === 'true';
    case 'array': {
      // Arrays are stored JSON-encoded (ADR-192). Parse back to a real array so
      // scripts read what they set. Guard parse failure / non-array by returning
      // the raw string — a corrupt value stays visible rather than throwing.
      try {
        const parsed: unknown = JSON.parse(value);
        return Array.isArray(parsed) ? parsed : value;
      } catch {
        return value;
      }
    }
    default:
      return value;
  }
}

// ---------------------------------------------------------------------------
// Variable scope factory — creates get/set/unset/has/toObject closures.
// ---------------------------------------------------------------------------

function createVariableScope(
  contextVars: EnvironmentVariables,
  rawMutations: RawScopeMutations,
  scopeKey: keyof RawScopeMutations,
  options?: { readonly?: boolean },
) {
  return {
    // Returns `any`: sandbox scripts are dynamically typed, and the value's runtime
    // type (string/number/boolean) depends on how it was stored. The non-discriminated
    // union forced users to coerce at every use; `any` matches the scripting contract.
    // oxlint-disable-next-line @typescript-eslint/no-explicit-any
    get(key: string): any {
      const pending = rawMutations[scopeKey]?.[key];
      if (pending === null) return undefined;
      if (pending) return coerceValueByType(pending.value, pending.type);
      const existing = contextVars[key];
      if (!existing || existing.isEnabled === false) return undefined;
      return coerceValueByType(getEffectiveValue(existing), existing.type);
    },

    set(key: string, value: string | number | boolean | unknown[] | null | undefined): void {
      // A read-only scope (collectionVariables on a standalone request, no parent
      // collection) makes set/unset/clear a silent no-op — not a throw (RQ-4236).
      // Checked before the empty-key guard so a read-only write is a pure no-op
      // regardless of arguments. In-collection writes still persist.
      if (options?.readonly) return;
      if (!key) {
        // oxlint-disable-next-line custom/no-dynamic-error-message -- scopeKey is a bounded keyof RawScopeMutations union
        throw new Error(`${scopeKey} variable key must be a non-empty string`);
      }
      const scope = rawMutations[scopeKey] ?? (rawMutations[scopeKey] = {});
      // null/undefined CLEARS the variable (Postman runner parity, RQ-4780): `get`
      // returns undefined, not the truthy string "null" that String(value) would
      // produce (which silently defeats `if (!v) skipRequest()`). Same sentinel as unset.
      if (value == null) {
        scope[key] = null;
        return;
      }
      // Arrays are JSON-encoded and tagged `array` (ADR-192) so `get` restores a
      // real array. Scalars keep their recorded type (RQ-1421).
      if (Array.isArray(value)) {
        scope[key] = { value: JSON.stringify(value), type: 'array' };
      } else {
        scope[key] = {
          value: String(value),
          type: typeof value === 'number' ? 'number' : typeof value === 'boolean' ? 'boolean' : 'string',
        };
      }
    },

    unset(key: string): void {
      if (options?.readonly) return;
      if (!key) {
        // oxlint-disable-next-line custom/no-dynamic-error-message -- scopeKey is a bounded keyof RawScopeMutations union
        throw new Error(`${scopeKey} variable key must be a non-empty string`);
      }
      const scope = rawMutations[scopeKey] ?? (rawMutations[scopeKey] = {});
      scope[key] = null;
    },

    clear(): void {
      if (options?.readonly) return;
      const scope = rawMutations[scopeKey] ?? (rawMutations[scopeKey] = {});
      for (const key of Object.keys(contextVars)) {
        scope[key] = null;
      }
      for (const key of Object.keys(scope)) {
        if (scope[key] !== null) scope[key] = null;
      }
    },

    has(key: string): boolean {
      const pending = rawMutations[scopeKey]?.[key];
      if (pending === null) return false;
      if (pending) return true;
      const existing = contextVars[key];
      return !!existing && existing.isEnabled !== false;
    },

    /**
     * Serialization view of the scope — always string values, intentionally
     * asymmetric with `get()`. `get("n")` restores the recorded type (e.g. a
     * number), whereas `toObject().n` is the raw stored string. `toObject` is
     * for bulk inspection/serialization, where stringified values are wanted.
     */
    toObject(): Record<string, string> {
      const result: Record<string, string> = {};
      for (const [k, v] of Object.entries(contextVars)) {
        if (v.isEnabled === false) continue;
        const val = getEffectiveValue(v);
        if (val !== undefined) result[k] = val;
      }
      const pending = rawMutations[scopeKey];
      if (pending) {
        for (const [k, v] of Object.entries(pending)) {
          if (v === null) delete result[k];
          else result[k] = v.value;
        }
      }
      return result;
    },
  };
}

// ---------------------------------------------------------------------------
// rq namespace factory
// ---------------------------------------------------------------------------

/**
 * Creates the `rq` namespace object.
 *
 * This is the single source of truth for the rq scripting API.
 * Adding a new rq method = add it here, return it in the object.
 *
 * Takes VM dependencies as parameters — no `declare const`, no globals.
 * Works in Node.js, web workers, and browsers.
 */
export function createRqNamespace(
  executionState: {
    testResults: RawTestResult[];
    rawMutations: RawScopeMutations;
    requestMutations?: RequestHeaderMutation[];
    executionDirective?: ExecutionDirective;
    visualizerOutput?: VisualizerDirective;
  },
  libs: AssertionLibs,
  context: ScriptExecutionContext,
  eventName: ScriptPhase,
  cookieBridge?: CookieJarBridge,
  entryType?: EntryType,
  fetchImpl: typeof globalThis.fetch = globalThis.fetch,
  // Per-engine host round-trip for rq.execution.runRequest (ADR-169). `undefined`
  // for now; the engines (node-sandbox / quickjs) wire it in their own tasks.
  // When absent, `rq.execution.runRequest` is simply not present.
  runRequestImpl?: RunRequestImpl,
) {
  /**
   * rq.test — wraps a test function in a try/catch and records the result.
   */
  function test(name: string, testFn: () => void): void {
    try {
      testFn();
      executionState.testResults.push({ name, status: 'passed' });
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      executionState.testResults.push({ name, status: 'failed', error: message });
    }
  }

  /**
   * rq.test.skip — records a skipped test without executing anything.
   *
   * Accepts the same `(name, fn)` signature as `rq.test` for Postman parity
   * (`pm.test.skip(name, fn)`): the common migration idiom is toggling a test
   * active⇄skipped by adding/removing `.skip` on an existing `rq.test(name, fn)`.
   * The `fn` is intentionally never invoked — skipping means the body does not run.
   */
  test.skip = function skip(name: string, testFn?: () => void): void {
    // testFn is intentionally never invoked — skipping means the body does not
    // run. The param exists for pm.test.skip(name, fn) parity and to mirror
    // rq.test's signature in the generated editor types. Referenced here so it
    // surfaces as `testFn` (not `_testFn`) in the .d.ts without tripping
    // no-unused-vars.
    void testFn;
    executionState.testResults.push({ name, status: 'skipped' });
  };

  /**
   * rq.expect — Chai expect passthrough.
   */
  // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- chai.expect is unknown at the boundary; type narrowing happens at the consumer level
  const expect = libs.chai.expect as typeof import('chai').expect;

  /**
   * rq.info — execution metadata (ADR-049).
   * eventName is attached here from the phase parameter, not from the serialized context.
   * collectionId is excluded — internal only, not user-facing (ADR-053).
   */
  const { requestId, requestName, iteration, iterationCount, entryIndex, totalEntries } = context.info;
  const info = Object.freeze({
    requestId,
    requestName,
    iteration,
    iterationCount,
    entryIndex,
    totalEntries,
    eventName,
  });

  /**
   * Variable scope objects (ADR-053).
   */
  const environment = createVariableScope(context.environment, executionState.rawMutations, 'environment');
  const globals = createVariableScope(context.global, executionState.rawMutations, 'global');
  const isCollectionReadonly = context.info.collectionId === null;
  const collectionVariables = createVariableScope(
    context.collectionVariables,
    executionState.rawMutations,
    'collection',
    { readonly: isCollectionReadonly },
  );
  const variables = createVariableScope(context.variables, executionState.rawMutations, 'runtime');

  /**
   * rq.iterationData — read-only access to collection runner iteration data.
   * Built as a custom object (not createVariableScope) because iteration data
   * is structured input from CSV/JSON, not a mutable variable scope.
   * No set/unset — read-only by design.
   */
  const iterationData = {
    // Returns `any` for parity with the variable-scope getters, sparing scripts from coercing
    // at every use. (Runtime value is always `string | undefined` here — no number/boolean coercion.)
    // oxlint-disable-next-line @typescript-eslint/no-explicit-any
    get(key: string): any {
      const existing = context.iterationData[key];
      if (!existing || existing.isEnabled === false) return undefined;
      return getEffectiveValue(existing);
    },
    has(key: string): boolean {
      const existing = context.iterationData[key];
      return !!existing && existing.isEnabled !== false;
    },
    toObject(): Record<string, string> {
      const result: Record<string, string> = {};
      for (const [k, v] of Object.entries(context.iterationData)) {
        if (v.isEnabled === false) continue;
        const val = getEffectiveValue(v);
        if (val !== undefined) result[k] = val;
      }
      return result;
    },
  };

  /**
   * rq.request — curated request properties (ADR-054, ADR-136).
   * Protocol-specific shape dispatched via entryType.
   */
  const resolvedEntryType = entryType ?? EntryType.http;
  // Request header mutations (ADR-167) are collected here and surfaced to the host
  // via executionState.requestMutations, which the engine maps onto
  // ScriptExecutionResult.requestMutationDiff. Only meaningful in the pre-request
  // phase; the runtime ignores the diff for post-response scripts.
  if (executionState.requestMutations === undefined) executionState.requestMutations = [];
  const requestMutationCollector: RequestMutationCollector = { headers: executionState.requestMutations };
  const request = buildScriptRequest(
    context.request,
    resolvedEntryType,
    context.auth ? { auth: context.auth } : undefined,
    requestMutationCollector,
  );

  /**
   * rq.response — curated response properties with assertion chain (ADR-054, ADR-136).
   * null in pre-request phase; phase filtering removes the key entirely via PHASE_RESTRICTED.
   */
  const response = context.response ? buildScriptResponse(context.response, libs, resolvedEntryType) : null;

  /**
   * rq.vault — read-only access to vault secrets (ADR-022).
   * get/has/toObject read from pre-populated context.secrets.
   * set/unset throw — vault secrets cannot be modified from scripts.
   *
   * ADR-196 (RQ-3734, AC-008): when the "Allow scripts to access vault secrets"
   * device setting is off (`context.secretsAccessDisabled`), every accessor throws
   * an actionable error rather than returning empty — so the script fails loudly,
   * surfacing the message in the response/console instead of silently reading
   * `undefined`.
   */
  const assertVaultAccessEnabled = (): void => {
    if (context.secretsAccessDisabled === true) {
      throw new Error(
        'Vault access from scripts is disabled on this device. Turn on "Allow scripts to access vault secrets" in Settings → Vault to read vault secrets in scripts.',
      );
    }
  };
  const vault = {
    // Returns `any` for parity with the variable-scope getters, sparing scripts from coercing
    // at every use. (Runtime value is always `string | undefined` here — no number/boolean coercion.)
    // oxlint-disable-next-line @typescript-eslint/no-explicit-any
    get(key: string): any {
      assertVaultAccessEnabled();
      const existing = context.secrets[key];
      if (!existing || existing.isEnabled === false) return undefined;
      return getEffectiveValue(existing);
    },
    has(key: string): boolean {
      assertVaultAccessEnabled();
      const existing = context.secrets[key];
      return !!existing && existing.isEnabled !== false;
    },
    toObject(): Record<string, string> {
      assertVaultAccessEnabled();
      const result: Record<string, string> = {};
      for (const [k, v] of Object.entries(context.secrets)) {
        if (v.isEnabled === false) continue;
        const val = getEffectiveValue(v);
        if (val !== undefined) result[k] = val;
      }
      return result;
    },
  };

  /**
   * rq.cookies — per-host jar API gated by the pre-bound allowlist (ADR-105).
   * `cookieBridge` is supplied by the sandbox consumer. When omitted, an
   * inert bridge is used — every `jar(host)` call will short-circuit via
   * the allowlist check (empty allowlist ⇒ every host denied) so the
   * bridge never has to handle a real call.
   */
  const cookies = createCookiesNamespace({
    hostAllowlist: context.hostAllowlist,
    bridge: cookieBridge ?? INERT_COOKIE_BRIDGE,
  });

  /**
   * rq.sendRequest — issue an HTTP sub-request, Postman `pm.sendRequest`
   * parity (ADR-153). Wraps the injected `fetch`; dual callback+promise form.
   */
  const sendRequest = createSendRequest(fetchImpl);

  /**
   * rq.execution — collection-runner flow control (ADR-169). The namespace
   * writes a single ExecutionDirective onto this collector, which is a live
   * view onto executionState.executionDirective; the engine reads it back after
   * the script settles. Mirrors the requestMutationCollector wiring above.
   */
  const executionCollector: ExecutionDirectiveCollector = {
    get directive() {
      return executionState.executionDirective;
    },
    set directive(d) {
      executionState.executionDirective = d;
    },
  };

  /**
   * rq.visualizer — response visualizer (ADR-202). The namespace compiles the
   * Handlebars template in-guest at set() time and writes a single VisualizerOutput
   * onto this collector, a live view onto executionState.visualizerOutput the engine
   * reads back after the script settles. Mirrors the executionCollector wiring above.
   * Post-response-only via PHASE_RESTRICTED; the factory warns + no-ops otherwise.
   */
  const visualizerCollector: VisualizerCollector = {
    get output() {
      return executionState.visualizerOutput;
    },
    set output(v) {
      executionState.visualizerOutput = v;
    },
  };

  const rq = {
    test,
    expect,
    info,
    environment,
    globals,
    collectionVariables,
    variables,
    iterationData,
    request,
    response,
    vault,
    cookies,
    sendRequest,
    // rq.execution (ADR-169) — a real, supported namespace, so it lives in the
    // typed object literal (NOT the unsupported-stub Object.assign below) to
    // appear in the inferred return type and the editor autocomplete .d.ts.
    // Phase/availability shaping (skipRequest pre-request-only; runRequest only
    // when the engine wires runRequestImpl) is modeled by the factory's return type.
    execution: createExecutionNamespace(executionCollector, eventName, context.location ?? [], runRequestImpl),
    // rq.visualizer (ADR-202) — a real, supported namespace, so it lives in the
    // typed object literal to appear in the inferred return type and the editor
    // autocomplete .d.ts (FR-12). Available in BOTH the pre-request and post-response
    // phases (Postman parity) — the pre-request output is lifted onto the entry too,
    // last-writer-wins (ADR-202 "Amendment (2026-08-02)"); no phase gating.
    visualizer: createVisualizer(visualizerCollector, libs),
    // rq.message (ADR-208) — the message this on-message iteration is handling.
    // On-message-only via PHASE_RESTRICTED, which is now DERIVED from
    // PHASE_DESCRIPTORS[onMessage].exclusiveSurface rather than hand-listed.
    //
    // `null` outside on-message rather than absent, because the builder deletes the
    // key entirely in other phases (PHASE_RESTRICTED) — so the null is only ever
    // observed by the engine between construction and that deletion, never by a
    // script. The Safe engine mirrors the deletion with its own hand-written gate,
    // which the derivation does NOT reach; U-17 pins that absence separately.
    message: context.message ? buildScriptMessage(context.message, libs) : null,
    /** Whether this script is running in Safe mode (QuickJS-WASM) vs Developer mode (node:vm). */
    isSafeMode: false,
  };

  return rq;
}

/**
 * Placeholder bridge used when `createRqNamespace` is called without one.
 * Never dispatched under normal operation — `jar(host)` throws
 * `CookieJarHostDenied` for any host not in the (empty) allowlist, which
 * is the only state that reaches this bridge.
 */
const INERT_COOKIE_BRIDGE: CookieJarBridge = {
  list: () => [],
  upsert: () => {},
  remove: () => {},
  clear: () => {},
};
