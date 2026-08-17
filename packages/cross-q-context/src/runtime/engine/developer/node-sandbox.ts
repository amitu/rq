/**
 * NodeSandbox — Isolated script execution via node:vm
 *
 * Executes user scripts inside vm.createContext for isolation, streams logs
 * in real-time via StreamHandle, and enforces timeout via Promise.race.
 *
 * This is the core sandbox implementation extracted from desktop's SandboxService.
 * It has no RPC coupling — consumers (desktop, CLI, API server) add their own
 * transport wiring.
 */

import * as vm from 'node:vm';

import { ARRAY_METHODS_SHIM, CONVENIENCE_GLOBALS_SHIM } from '../../index.js';
import { StreamHandle } from '../stream-handle.js';

import { AsyncRegistry, SANDBOX_DEFAULT_TIMEOUT_MS } from '../index.js';
import { createConsoleMock } from './console-mock.js';
import { normalizeFromVm } from './realm-normalization.js';
import { buildDeprecationGlobals, buildRq, buildVmGlobals, createExecutionState, GLOBAL_NAMES } from './builder.js';
import { inflateMutations } from '../index.js';
import {
  buildBatchResult,
  createBatchOutcome,
  ON_MESSAGE_TIMEOUT_ERROR,
  stampMessageIndex,
} from '../index.js';
import { countScriptLines, parseScriptErrorLocation, scriptFilenameForPhase } from '../index.js';
import { VENDOR_IIFES } from '../index.js';
import { createRequireFn } from './require-builder.js';
import { createVmEvaluator } from './vm-package-evaluator.js';
import { toDelegatedFetch } from '../delegated-fetch.js';
import { CLIENT_SSRF_POLICY, createGuardedFetch } from '../ssrf-guard.js';
import type { SsrfPolicy } from '../ssrf-guard.js';

import { LogLevel, PHASE_DESCRIPTORS } from '../../index.js';
import {
  buildScriptMessage,
  createDeprecatedPostmanShims,
  createSendRequest,
  DEPRECATED_IDENTIFIERS,
  formatDeprecationMessage,
  SkipRequestSignal,
} from '../../index.js';

import type { FeatureFlags, ScriptExecutionInput, ScriptMessageInput, StreamReader } from '../../index.js';
import type {
  CookieJarMutation,
  Sandbox,
  SandboxExecutionEvent,
  ScriptExecutionResult,
  TestResult,
} from '../host-types.js';
import type { BatchOutcome } from '../index.js';
import type { AsyncGlobalName } from '../../index.js';
import type { ExecutionState } from './types.js';
import type {
  AssertionLibs,
  DeprecationEmit,
  PackageResolver,
  RunRequestImpl,
  ScriptSendRequest,
  SendRequestCallback,
  SendRequestInput,
} from '../../index.js';
import type { SandboxHostCallbacks } from '../../index.js';

/** Type predicate: checks that a value has the shape of a TestResult. */
function isTestResult(value: unknown): value is TestResult {
  return typeof value === 'object' && value !== null && 'name' in value && 'status' in value;
}

/** Realm-normalize test results from the VM context. */
function normalizeTestResults(results: TestResult[]): TestResult[] {
  const parsed: unknown = normalizeFromVm(results);
  return Array.isArray(parsed) ? parsed.filter(isTestResult) : [];
}

/**
 * Type predicate for `CookieJarMutation`. Discriminator is `kind` ∈
 * {'upsert', 'remove', 'clear'} per `packages/shared-types/src/runtime/scripts.ts`.
 */
function isCookieJarMutation(value: unknown): value is CookieJarMutation {
  if (typeof value !== 'object' || value === null || !('kind' in value) || !('host' in value)) {
    return false;
  }
  const kind = (value as { kind: unknown }).kind;
  return kind === 'upsert' || kind === 'remove' || kind === 'clear';
}

/**
 * Realm-normalize cookie mutations. The cookie objects (especially the nested
 * `expiry` discriminated union) are constructed in the VM realm and their
 * prototypes don't survive Cap'n Web structured-clone at the RPC boundary.
 * JSON roundtrip strips the prototypes (same pattern as testResults).
 */
function normalizeCookieMutations(mutations: readonly CookieJarMutation[]): readonly CookieJarMutation[] {
  const parsed: unknown = normalizeFromVm(mutations);
  return Array.isArray(parsed) ? parsed.filter(isCookieJarMutation) : [];
}

/**
 * ADR-192 (Slice 2, Developer engine only). `rq.<scope>.get()` restores an
 * `array`-typed variable via `JSON.parse` in the HOST realm, so the returned
 * array's prototype is the host `Array.prototype` — which is intentionally NOT
 * patched (no host leak). The `.first()/.last()` shim lives on the vm realm's
 * `Array.prototype`. Without bridging, `get(arr).last()` is `undefined` in
 * Developer mode while working in Safe mode — a silent engine divergence.
 *
 * This wraps each variable scope's `get` so an array result is re-created in the
 * vm realm (via `__rq_reviveArrayInRealm`, installed by ARRAY_METHODS_SHIM), so
 * the array a script sees inherits the patched prototype. Non-array results are
 * untouched. Scalars/objects are unaffected.
 */
function wrapScopeGetsForVmRealm(rq: Record<string, unknown>, vmContext: object): void {
  const reviveInRealm: unknown = Reflect.get(vmContext, '__rq_reviveArrayInRealm');
  if (typeof reviveInRealm !== 'function') return;
  // The reviver runs JSON.parse INSIDE the vm realm, so the array it returns
  // belongs to the vm realm (and inherits the patched .first()/.last()). We pass
  // the JSON string, not the host array, because structuredClone is unavailable
  // in a node:vm realm.
  const revive = (json: string): unknown[] => {
    const out: unknown = reviveInRealm(json);
    return Array.isArray(out) ? out : [];
  };
  for (const scopeKey of ['environment', 'globals', 'collectionVariables', 'variables'] as const) {
    const scope: unknown = rq[scopeKey];
    if (scope === null || typeof scope !== 'object') continue;
    const originalGet: unknown = Reflect.get(scope, 'get');
    if (typeof originalGet !== 'function') continue;
    const boundGet = (key: string): unknown => Reflect.apply(originalGet, scope, [key]);
    Reflect.set(scope, 'get', (key: string): unknown => {
      const result = boundGet(key);
      return Array.isArray(result) ? revive(JSON.stringify(result)) : result;
    });
  }
}

/**
 * Node.js sandbox execution engine.
 * Each execute() call creates a fresh vm context and runs the user script.
 * Timeout is enforced via Promise.race on input.timeoutMs.
 */
export class NodeSandbox implements Sandbox {
  private readonly resolver: PackageResolver | undefined;
  private readonly guardedFetch: typeof fetch;

  // The link-local/metadata surface is blocked regardless of policy (RQ-3902);
  // `ssrfPolicy` only decides whether the broader private ranges are also blocked.
  // Defaults to the client posture (allow localhost/LAN) since the common host is
  // desktop/CLI where the script runs on the user's own machine; server hosts
  // (scheduled-run-runner) pass STRICT_SSRF_POLICY to also block private ranges.
  constructor(resolver?: PackageResolver, options?: { readonly ssrfPolicy?: SsrfPolicy }) {
    this.resolver = resolver;
    this.guardedFetch = createGuardedFetch(globalThis.fetch, options?.ssrfPolicy ?? CLIENT_SSRF_POLICY);
  }

  getFeatures(): Promise<FeatureFlags> {
    return Promise.resolve({ isolatedVm: true, externalPackages: this.resolver !== undefined });
  }

  execute(
    input: ScriptExecutionInput,
    hostCallbacks?: SandboxHostCallbacks,
  ): Promise<StreamReader<SandboxExecutionEvent>> {
    const handle = new StreamHandle<SandboxExecutionEvent>();

    if (!input.script || input.script.trim() === '') {
      handle.push({ type: 'result', result: { mutationDiff: {}, logs: [], testResults: [] } });
      handle.end();
      return Promise.resolve(handle);
    }

    // Wrap fire-and-forget call so Sentry tracks the full Promise lifecycle.
    // The void discards the caller's reference but Sentry's startSpan
    // holds the span open until runScript resolves/rejects.
    // ADR-192: derive the verdict from runScript's return — it swallows a script
    // failure into an in-band `error` result and never re-throws, so it returns
    // `spanErrorResult()` on failure / plain on success (incl. skipRequest).
    void this.runScript(input, handle, hostCallbacks);
    return Promise.resolve(handle);
  }

  private async runScript(
    input: ScriptExecutionInput,
    handle: StreamHandle<SandboxExecutionEvent>,
    hostCallbacks?: SandboxHostCallbacks,
  ): Promise<void> {
    const executionState = createExecutionState();
    // Hoisted so the catch branch can drain partial cookie mutations even if
    // the failure happened after the bridge was constructed.
    let drainCookieMutations: (() => readonly CookieJarMutation[]) | undefined;

    // Per-execution async registry (ADR-219) — the single owner of "what counts
    // as pending work", shared with the Safe engine. Replaces RQ-5156's closure
    // counter and its two hand-placed `track()` calls: coverage is now a
    // property of the typed capability map below, so a new async surface that
    // is not registered does not compile. Per execution (never module state) so
    // concurrent executions in this worker cannot observe each other's count.
    const asyncRegistry = new AsyncRegistry<ReturnType<typeof setTimeout>>({
      timers: {
        scheduleTimer: (fn, ms) => setTimeout(fn, ms),
        cancelTimer: (timerHandle) => {
          clearTimeout(timerHandle);
        },
      },
      // Developer callbacks run host-side, so unlike Safe — where the callback
      // lives in the isolate and its throw is a guest promise rejection the host
      // cannot observe — the registry's own wrapper does catch this.
      onCallbackError: (error) => {
        handle.push({
          type: 'log',
          log: {
            level: LogLevel.error,
            args: [`Uncaught error in timer callback: ${error instanceof Error ? error.message : String(error)}`],
            timestamp: Date.now(),
          },
        });
      },
    });

    try {
      // Build console mock that streams log events
      const consoleMock = createConsoleMock((log) => {
        handle.push({ type: 'log', log });
      });

      // Deprecation chokepoint (RQ-3464). Single function so the console
      // warning and the analytics signal stay coupled and fire exactly once
      // per identifier (the once-guard lives in the proxy). The signal object
      // is constructed here in the host (Node) realm — never inside the VM
      // realm — so it needs no normalizeFromVm roundtrip and survives the RPC
      // boundary directly (ADR-034).
      const emitDeprecation: DeprecationEmit = (identifier, opts) => {
        handle.push({ type: 'deprecation', signal: { identifier, shimmed: opts.shimmed } });
        const policy = DEPRECATED_IDENTIFIERS[identifier];
        if (policy) {
          handle.push({
            type: 'log',
            log: { level: LogLevel.warn, args: [formatDeprecationMessage(identifier, policy)], timestamp: Date.now() },
          });
        }
      };

      // Build host object dynamically from GLOBAL_NAMES (parsed from globals.d.ts)
      const host: Record<string, unknown> = {};
      for (const name of GLOBAL_NAMES) {
        host[name] = (globalThis as Record<string, unknown>)[name];
      }
      // Override console with the streaming mock
      host['console'] = consoleMock;
      // `host['fetch']` is what buildVmGlobals injects into the context.
      //
      // DELEGATED (ADR-181/182, RQ-5318) when the host supplies a `sendRequest`
      // capability: route the script's `fetch` through the runtime fetcher, so it
      // inherits the OS trust store (`system-ca`), the user's CA / client
      // certificates and the fetcher's TLS handling. A bare `globalThis.fetch`
      // carries only Node's bundled Mozilla roots, which is why script requests
      // failed on TLS-intercepting corporate networks while ordinary requests
      // succeeded. The Safe engine gets this via its fetch bridge; this is the
      // Developer engine's equivalent.
      //
      // DIRECT fallback otherwise: the SSRF-guarded `globalThis.fetch` (RQ-3902),
      // so a script cannot reach the cloud metadata server / internal ranges. The
      // guard is not applied on the delegated path — egress there is the fetcher's
      // (and its host's) concern, matching the Safe engine's delegated branch.
      const baseFetch =
        hostCallbacks?.sendRequest !== undefined ? toDelegatedFetch(hostCallbacks.sendRequest as unknown as Parameters<typeof toDelegatedFetch>[0]) : this.guardedFetch;
      const trackedFetch: typeof fetch = (...args) => asyncRegistry.registerPromise(baseFetch(...args));

      // The capability map (ADR-219 / RQ-5671). Every global classified `'async'`
      // in GLOBAL_ASYNC_CLASS must appear here with a registry-backed
      // implementation: the `Record<AsyncGlobalName, …>` annotation makes a
      // missing entry a COMPILE ERROR, which is what replaces "remember to call
      // track()". Adding a global to GLOBAL_NAMES without classifying it fails to
      // compile one package earlier, in sandbox-definitions.
      //
      // Timer ids are the REGISTRY's, not Node's, on both the set and clear side —
      // they must pair, or a cleared timer's hold is never released.
      //
      // An interval re-arms after each fire, so its registry id changes while the
      // script still holds the id returned from `setInterval`. This map carries
      // that indirection: public id → the registry id of the currently-armed tick.
      // Both clears consult it, so `clearTimeout`/`clearInterval` stay
      // interchangeable as they are on the platform, and the public id is drawn
      // from the registry's own counter so the two kinds cannot collide.
      const activeIntervals = new Map<number, number>();
      type TimerCallback = (...callbackArgs: unknown[]) => void;

      const clearAnyTimer = (id?: number): void => {
        if (typeof id !== 'number') return;
        const armed = activeIntervals.get(id);
        activeIntervals.delete(id);
        asyncRegistry.clearTimer(armed ?? id);
      };

      const asyncGlobals: Record<AsyncGlobalName, unknown> = {
        fetch: trackedFetch,
        setTimeout: (fn: TimerCallback, ms?: number, ...args: unknown[]): number =>
          asyncRegistry.setTimer(() => fn(...args), ms ?? 0),
        // Holds, like every other timer. Measured against Postman 12.14.0: an
        // uncleared `setInterval` there holds the run open INDEFINITELY (5040+
        // ticks / 42min observed, no termination), because the app passes no
        // finite timeout to the sandbox so `Timerz`'s guard timer is never armed.
        // Holding matches that; our per-execution budget then bounds the runaway
        // case Postman leaves unbounded, and seal-and-warn keeps RQ-5156 intact.
        setInterval: (fn: TimerCallback, ms?: number, ...args: unknown[]): number => {
          const period = ms ?? 0;
          let publicId = 0;
          const tick = (): void => {
            const armed = asyncRegistry.setTimer(() => {
              // Cleared while this tick was in flight — do not fire, do not re-arm.
              if (!activeIntervals.has(publicId)) return;
              fn(...args);
              // Re-arm only if the callback did not clear this interval itself.
              if (activeIntervals.has(publicId)) tick();
            }, period);
            // The first arm defines the public id; later arms just re-point it.
            if (publicId === 0) publicId = armed;
            activeIntervals.set(publicId, armed);
          };
          tick();
          return publicId;
        },
        clearTimeout: clearAnyTimer,
        clearInterval: clearAnyTimer,
      };
      Object.assign(host, asyncGlobals);

      // Build context for the builder functions
      const ctx = {
        context: input.context,
        phase: input.phase,
        vmRealm: { chai: { expect: undefined as unknown } },
        host,
      };

      // Fresh vm context per execution — no state leakage.
      // Deprecation proxies are injected as bare VM globals (RQ-3464): the
      // deprecated identifiers (`globals`, `tv4`, `Backbone`, …) are absent
      // from the context today, so accessing them would otherwise throw
      // ReferenceError. The proxy makes the access observable via emitDeprecation.
      const vmContext = vm.createContext({
        ...buildVmGlobals(ctx),
        ...buildDeprecationGlobals(emitDeprecation),
      });

      // Evaluate Chai IIFE inside VM — creates __chai in the VM realm (ADR-002, ADR-005)
      vm.runInContext(VENDOR_IIFES.chai, vmContext);

      // Patch Array.prototype.first/.last INSIDE the VM realm (ADR-192, Slice 2) —
      // Postman parity. Run in-context (never a host-side prototype write) so the
      // patch cannot leak to the renderer host. Shared constant with the Safe engine.
      vm.runInContext(ARRAY_METHODS_SHIM, vmContext);

      // Inject require() function into VM context (ADR-005 §require() Implementation, ADR-087 §Resolution Order).
      // Packages are pre-evaluated in the host (Node.js) context at module load time.
      // The require function returns these pre-evaluated modules and caches per-execution.
      // The vmEvaluator enables user-authored package evaluation inside the VM context
      // when userPackages is supplied (ADR-088: forwarded from ExecutionPayload.userPackages).
      // When a PackageResolver is provided, it sits between user packages and Node built-ins (ADR-079).
      // contextId routes resolution to the correct per-script node_modules directory.
      const vmEvaluator = createVmEvaluator(vmContext);
      // contextIdPrefix comes from PHASE_DESCRIPTORS; this was a `preRequest ? 'pre' : 'post'`
      // ternary duplicated in both engines, so a new phase resolved require() against
      // the POST-RESPONSE node_modules.
      const contextId = `${PHASE_DESCRIPTORS[input.phase].contextIdPrefix}-${input.entryId}`;
      (vmContext as Record<string, unknown>)['require'] = createRequireFn(
        input.userPackages,
        vmEvaluator,
        this.resolver,
        contextId,
        input.blacklistedPackages,
        // RQ-5671 Phase 3: keep a require()'d built-in's async work visible to the
        // drain. Tier 5 hands over the REAL Node module, so without this
        // `require('timers').setTimeout(cb, 1000)` bypasses the registry-backed
        // globals entirely. Passing the same wrappers means the module and the
        // global are one surface, so ids interoperate across both.
        { registry: asyncRegistry, timers: asyncGlobals } as unknown as Parameters<typeof createRequireFn>[5],
      );

      // Vendor IIFEs consumed by the rq factory (libs): __lodash, __ajv, and (below)
      // __handlebars. `_` and `xml2Json` are NO LONGER installed here — both moved to
      // the shared CONVENIENCE_GLOBALS_SHIM (RQ-5613 `_`, RQ-5625 `xml2Json`) so both
      // engines install them from one source. The xml2js IIFE no longer needs an eager
      // eval — the shim's `require('xml2js')` self-resolves via the require chain (as
      // `require('crypto-js')`/`require('lodash')` already do).
      vm.runInContext(VENDOR_IIFES.lodash, vmContext);
      vm.runInContext(VENDOR_IIFES.ajv, vmContext);
      // Handlebars (ADR-202) — the response visualizer compiles its template with it
      // at rq.visualizer.set() time. Delivered via the same VENDOR_IIFES vendor-bundle
      // path as chai/lodash/ajv (creates `__handlebars` in the VM realm), then handed to
      // the rq factory in `libs` below. Internal (vendor-only): no user-facing require().
      vm.runInContext(VENDOR_IIFES.handlebars, vmContext);
      // Lazy convenience globals — bare `CryptoJS` (crypto-js, RQ-5512), `_` (lodash,
      // RQ-5613), and `xml2Json` (xml2js wrapper, RQ-5625). Shared verbatim with the
      // Safe engine so the two cannot drift. Runs AFTER `require` is injected above;
      // each accessor resolves nothing until a script touches the global, so the
      // bundle stays off the hot path for scripts that never use it.
      vm.runInContext(CONVENIENCE_GLOBALS_SHIM, vmContext);

      // Build rq namespace object and inject into VM context.
      // __chai, __lodash, __ajv are created by their respective IIFEs above.
      // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- VM realm boundary: vmContext is untyped, __chai structure known from Chai IIFE
      const chaiModule = (vmContext as Record<string, unknown>)['__chai'] as { expect: unknown };
      // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- VM realm boundary: __lodash structure known from lodash IIFE
      const lodashModule = (vmContext as Record<string, unknown>)['__lodash'] as {
        get: (obj: unknown, path: string) => unknown;
        isEqual: (a: unknown, b: unknown) => boolean;
      };
      const ajvModule = (vmContext as Record<string, unknown>)['__ajv'];
      // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- VM realm boundary: __handlebars structure known from the Handlebars IIFE (ADR-202)
      const handlebarsModule = (vmContext as Record<string, unknown>)['__handlebars'] as {
        compile: (template: string) => (context?: unknown) => string;
      };
      // Wrap the per-execution host callback into the engine-agnostic
      // RunRequestImpl that createRqNamespace expects (ADR-169). Absent when the
      // host doesn't supply runRequest → impl stays undefined → rq.execution.runRequest
      // is not present (createExecutionNamespace handles the absence). NodeSandbox's
      // host realm is reachable by design, so injecting a host fn here is consistent
      // with the Developer engine's posture (this is NOT the isolated path).
      const runRequestHost = hostCallbacks?.runRequest;
      const runRequestImpl: RunRequestImpl | undefined = runRequestHost
        ? (descriptor) => (runRequestHost as unknown as { runRequest(d: unknown): Promise<unknown> }).runRequest(descriptor) as ReturnType<RunRequestImpl>
        : undefined;
      const built = buildRq(
        executionState,
        { chai: chaiModule, lodash: lodashModule, ajv: ajvModule, handlebars: handlebarsModule },
        input.phase,
        input.context,
        input.entryType,
        runRequestImpl,
        trackedFetch,
      );
      drainCookieMutations = built.drainCookieMutations;

      // RQ-5156: count the WHOLE `rq.sendRequest` operation, not just its fetch.
      // The fetch promise resolves BEFORE `sendRequest`'s handler awaits
      // `raw.text()` and fires the user's callback (`sendRequest.ts`), so counting
      // only the fetch can reach zero while the body read is still pending and the
      // callback has never run. Rebuilding the entry over the same tracked fetch
      // and counting the OUTER promise closes that window: `fireCallback` queues
      // the callback microtask before this promise resolves, so the callback always
      // precedes the decrement. Same factory + same fetch as `buildRq` uses, so the
      // behavior is identical — only the counting differs.
      const sendRequestImpl = createSendRequest(trackedFetch);
      const trackedSendRequest: ScriptSendRequest = (requestInput: SendRequestInput, callback?: SendRequestCallback) =>
        asyncRegistry.registerPromise(
          callback === undefined ? sendRequestImpl(requestInput) : sendRequestImpl(requestInput, callback),
        );
      built.rq['sendRequest'] = trackedSendRequest;

      (vmContext as Record<string, unknown>)['rq'] = built.rq;

      // ADR-192 (Slice 2, Developer engine only): `rq.*.get()` JSON.parses in the
      // HOST realm, so an array it returns carries the host Array.prototype — not
      // the vm realm's patched one — and `.first()/.last()` would be undefined on
      // it. Wrap each variable scope's `get` so an array result is re-created in
      // the vm realm via the in-context reviver installed by ARRAY_METHODS_SHIM.
      // The Safe engine needs no equivalent (its parse already runs in-guest).
      wrapScopeGetsForVmRealm(built.rq, vmContext);

      // Runtime shims for the bounded core set of deprecated Postman identifiers
      // (ADR-156, Slice C / RQ-3465). `buildDeprecationGlobals` (above) seeded a
      // no-op `shimmed: false` proxy for ALL 13 registry identifiers. This block
      // OVERRIDES exactly four of them — `globals`, `environment`, `responseBody`,
      // `responseCode` — with delegating shims that fire `emitDeprecation` with
      // `shimmed: true` and actually execute against the now-built `rq`. This is
      // a deliberate last-write-wins on the same VM-context keys; it MUST run
      // AFTER `built.rq` is assigned (the shims delegate to it). The other ten
      // registry identifiers keep their Slice B no-op proxy. The value shims
      // (`responseBody`/`responseCode`) are lazy getter descriptors, so they are
      // transferred via Object.defineProperties + getOwnPropertyDescriptors to
      // preserve laziness — a plain spread would invoke the getter eagerly and
      // throw in the pre-request phase where `rq.response` is null.
      const postmanShims = createDeprecatedPostmanShims(built.rq, emitDeprecation);
      Object.defineProperties(vmContext, Object.getOwnPropertyDescriptors(postmanShims));

      // Wrap in async IIFE for top-level await support, with an in-realm try/catch
      // that captures the thrown error's `.stack` onto the error object itself
      // (RQ-4142). The stack is read INSIDE the vm realm — where V8's default
      // formatter runs — so it carries clean `<phase>-script.js:L:C` frames even
      // though the desktop sandbox Worker installs its own `Error.prepareStackTrace`
      // (Node source-map support), which strips vm frames from a host-side read.
      // Verified against the live desktop app. The leading newline after `try {`
      // puts the user's line 1 on physical line 2 — a uniform +1 offset with
      // unshifted columns (mirrors the QuickJS wrapper). The trailing newline
      // guards a final single-line comment in the user script.
      const wrappedScript = `(async () => { try {\n${input.script}\n} catch (__rqErr) { try { if (__rqErr && typeof __rqErr === 'object') { __rqErr.__rqStack = __rqErr.stack ? String(__rqErr.stack) : ''; } } catch (_e) { /* non-object throw */ } throw __rqErr; } })()`;

      // `filename` anchors stack frames to a phase-specific name
      // (`post-response-script.js`) instead of the default `evalmachine.<anonymous>`,
      // so `parseScriptErrorLocation` can find them and the surfaced frame names the
      // user's script (RQ-4142).
      const scriptFilename = scriptFilenameForPhase(input.phase);
      const script = new vm.Script(wrappedScript, { filename: scriptFilename });

      const timeoutMs = input.timeoutMs ?? SANDBOX_DEFAULT_TIMEOUT_MS;

      // ── On-message: one iteration per batch element (ADR-208 §7) ──
      // Same host-driven shape as the Safe engine: the compiled `vm.Script` is run
      // once per message, so the batch pays one compile rather than K, and the
      // iteration boundary is host-side. This engine's results already live host-side
      // in `executionState`, so "per-iteration emission" costs nothing here — what it
      // buys is that the two engines' loops are the same shape (Equivalence).
      if (input.messageBatch !== undefined) {
        const outcome = await this.runMessageBatch({
          input,
          batch: input.messageBatch,
          script,
          vmContext,
          executionState,
          rq: built.rq,
          libs: { chai: chaiModule, lodash: lodashModule, ajv: ajvModule, handlebars: handlebarsModule },
          pendingAsync: () => asyncRegistry.holdingCount(),
          timeoutMs,
        });
        handle.push({
          type: 'result',
          result: buildBatchResult(
            outcome,
            outcome.mutations ? inflateMutations(outcome.mutations, input.context) : {},
            normalizeCookieMutations(drainCookieMutations()),
          ),
        });
        handle.end();
        return;
      }

      // Run script in sandboxed context
      const scriptResult: unknown = script.runInContext(vmContext);
      const scriptPromise = scriptResult instanceof Promise ? scriptResult : Promise.resolve(scriptResult);

      // Enforce timeout via Promise.race — if timer wins, the catch block handles the error.
      // The script may continue running in the background, but the stream is closed.
      // NOTE (RQ-5156): "continues in the background" is now the TIMEOUT path only.
      // On normal completion the drain below waits for tracked async work first, so an
      // unawaited `rq.sendRequest(url, cb)` has its callback effects captured. ADR-153
      // §Consequences quotes this comment as its authority — keep the two in sync.
      // Shared budget for the script AND the RQ-5156 drain below — the drain
      // deliberately introduces no new timeout knob.
      const executionDeadline = Date.now() + timeoutMs;
      let timeoutId: ReturnType<typeof setTimeout> | undefined;
      const timeoutPromise = new Promise<never>((_, reject) => {
        timeoutId = setTimeout(() => reject(new Error('Script execution timed out')), timeoutMs);
      });

      try {
        await Promise.race([scriptPromise, timeoutPromise]);
      } finally {
        clearTimeout(timeoutId);
      }

      // RQ-5156: drain async work the script started but never awaited, so its
      // `rq.*` writes and logs land BEFORE `executionState` is serialized below.
      // Without this they arrive after the result was emitted and are discarded
      // silently — the pasted-Postman `rq.sendRequest(url, cb)` shape.
      //
      // Yield FIRST, then re-check: a callback that ran during the yield and
      // started another request has already re-incremented, so chains and nested
      // callbacks drain to arbitrary depth. Exits only after a full macrotask turn
      // passes with nothing outstanding.
      for (;;) {
        await new Promise((resolve) => setImmediate(resolve));
        if (asyncRegistry.holdingCount() === 0) break;
        if (Date.now() >= executionDeadline) break;
      }
      const unfinishedAsync = asyncRegistry.holdingCount();
      if (unfinishedAsync > 0) {
        // Seal and warn — never fail. A request that passes today must not start
        // failing because this hotfix ran out of budget waiting on background work.
        handle.push({
          type: 'log',
          log: {
            level: LogLevel.warn,
            args: [
              `Script finished with ${unfinishedAsync} background operation(s) still running; their results were not captured. Await them (e.g. \`await rq.sendRequest(...)\`) to include their effects.`,
            ],
            timestamp: Date.now(),
          },
        });
      }

      // Realm normalization — testResults objects are created inside the VM realm
      // and fail structured clone in Cap'n Web serialization (ADR-004 §POC-Caveat-1).
      const normalizedResults = normalizeTestResults(executionState.testResults);

      // Inflate raw mutations into proper MutationDiff (ADR-053 Layer 2)
      const mutationDiff = inflateMutations(executionState.rawMutations, input.context);

      // Script completed successfully
      const cookieMutations = normalizeCookieMutations(drainCookieMutations());
      const result: ScriptExecutionResult = {
        mutationDiff,
        logs: [],
        testResults: normalizedResults,
        ...(cookieMutations.length > 0 ? { cookieMutations } : {}),
        ...(executionState.requestMutations.length > 0
          ? { requestMutationDiff: { headers: executionState.requestMutations } }
          : {}),
        ...(executionState.executionDirective !== undefined
          ? { executionDirective: executionState.executionDirective }
          : {}),
        // Visualizer artifact from a post-response rq.visualizer.set() (ADR-202).
        // Absent when the script emitted none; the runtime lifts it onto EntryResult.
        ...(executionState.visualizerOutput !== undefined ? { visualizerOutput: executionState.visualizerOutput } : {}),
      };
      handle.push({ type: 'result', result });
      handle.end();
    } catch (err) {
      // rq.execution.skipRequest() throws SkipRequestSignal to abort the remaining
      // pre-request script (Postman parity, ADR-169). This is NOT an execution error —
      // the directive was already collected; surface a normal result carrying it.
      if (err instanceof SkipRequestSignal) {
        const normalizedResults = normalizeTestResults(executionState.testResults);
        const mutationDiff = inflateMutations(executionState.rawMutations, input.context);
        const cookieMutations = normalizeCookieMutations(drainCookieMutations?.() ?? []);
        handle.push({
          type: 'result',
          result: {
            mutationDiff,
            logs: [],
            testResults: normalizedResults,
            ...(cookieMutations.length > 0 ? { cookieMutations } : {}),
            ...(executionState.requestMutations.length > 0
              ? { requestMutationDiff: { headers: executionState.requestMutations } }
              : {}),
            ...(executionState.executionDirective !== undefined
              ? { executionDirective: executionState.executionDirective }
              : {}),
            // Visualizer is post-response-only, so this path (pre-request skipRequest)
            // never carries one; spread kept for symmetry with the other result paths.
            ...(executionState.visualizerOutput !== undefined
              ? { visualizerOutput: executionState.visualizerOutput }
              : {}),
          },
        });
        handle.end();
        return;
      }
      const message = err instanceof Error ? err.message : String(err);
      // Map a thrown user-script error back to the editor (RQ-4142). Prefer the
      // stack captured inside the vm realm (own `__rqStack` prop on the error),
      // which is immune to the sandbox Worker's `Error.prepareStackTrace` override;
      // fall back to the host-side stack. Non-user failures (timeout, package eval,
      // host) have no `<phase>-script.js` frame, so the parser returns undefined and
      // we surface the bare message as before.
      let capturedStack: string | undefined;
      if (err !== null && typeof err === 'object') {
        const stashed = Reflect.get(err, '__rqStack');
        if (typeof stashed === 'string' && stashed !== '') capturedStack = stashed;
      }
      const rawStack = capturedStack ?? (err instanceof Error ? err.stack : undefined);
      const errorLocation = parseScriptErrorLocation(
        rawStack,
        countScriptLines(input.script),
        scriptFilenameForPhase(input.phase),
        message,
      );
      // Realm normalization (same as success path above)
      const normalizedResults = normalizeTestResults(executionState.testResults);
      // Inflate even on error — partial mutations from before the error should propagate
      const mutationDiff = inflateMutations(executionState.rawMutations, input.context);
      // Drain even on error so any cookie mutations made before the throw propagate.
      // drainCookieMutations may be undefined if the throw happened before buildRq ran.
      const cookieMutations = normalizeCookieMutations(drainCookieMutations?.() ?? []);
      handle.push({
        type: 'result',
        result: {
          mutationDiff,
          logs: [],
          testResults: normalizedResults,
          ...(cookieMutations.length > 0 ? { cookieMutations } : {}),
          // Partial header mutations from before the throw still propagate (ADR-167).
          ...(executionState.requestMutations.length > 0
            ? { requestMutationDiff: { headers: executionState.requestMutations } }
            : {}),
          // A script may setNextRequest then throw — still report the directive (ADR-169).
          ...(executionState.executionDirective !== undefined
            ? { executionDirective: executionState.executionDirective }
            : {}),
          // A post-response script may visualizer.set() then throw — still report it (ADR-202).
          ...(executionState.visualizerOutput !== undefined
            ? { visualizerOutput: executionState.visualizerOutput }
            : {}),
          error: message,
          ...(errorLocation !== undefined ? { errorLocation } : {}),
        },
      });
      handle.end();
      return;
    } finally {
      // Seal on EVERY exit path — success, skipRequest, timeout, throw. A live
      // timer that outlives the execution would otherwise fire into a sealed
      // stream, and its `rq.*` writes would be silently discarded after the
      // result was emitted (exactly the class of bug RQ-5156 set out to close).
      asyncRegistry.seal();
    }
    // Clean run (or skipRequest directive) → plain return → derived OK.
    return;
  }

  /**
   * Run an on-message batch: one iteration per message, driven from the host
   * (ADR-208 §7, runtime 021 §Decision).
   *
   * The four obligations, and where each is discharged:
   *
   * - **Ordering** — a single sequential loop over the batch, awaited per element.
   * - **Coverage** — exactly one iteration per element; a throw is caught, recorded
   *   against its message, and the loop continues, so one message's failure cannot
   *   skip another.
   * - **Isolation** — a `try`/`catch` around each run, plus a reset of the
   *   per-iteration collectors at each boundary.
   * - **Equivalence** — everything that varies between iterations is `rq.message`
   *   and the re-armed budget; `messageIndex` is stamped host-side, in the shared
   *   helper both engines use.
   *
   * **This engine has no working per-message deadline, and that is a known
   * limitation rather than a tuning detail** (runtime 021 §Per-message deadline
   * AMENDMENT). `node:vm` cannot pre-empt CPU-bound guest code at all: a macrotask
   * timer cannot fire while the guest holds the thread, so a runaway iteration runs
   * unbounded and is reported as success. The per-message budget below is therefore
   * real only for iterations that yield (an `await`), and the batch bound catches an
   * overrun only once the iteration has finished on its own. Safe mode is the
   * default; closing this needs an engine change, not a test.
   */
  private async runMessageBatch(deps: {
    input: ScriptExecutionInput;
    batch: readonly ScriptMessageInput[];
    script: vm.Script;
    vmContext: object;
    executionState: ExecutionState;
    rq: Record<string, unknown>;
    libs: AssertionLibs;
    pendingAsync: () => number;
    timeoutMs: number;
  }): Promise<BatchOutcome> {
    const { input, batch, script, vmContext, executionState, rq, libs, pendingAsync, timeoutMs } = deps;
    const scriptFilename = scriptFilenameForPhase(input.phase);
    const scriptLines = countScriptLines(input.script);
    const outcome = createBatchOutcome();

    for (const message of batch) {
      // Re-point `rq.message` for this iteration. Rebuilt through the same factory
      // `createRqNamespace` uses, so the surface is identical to a single execution's.
      rq['message'] = buildScriptMessage(message, libs);

      const iterationStart = Date.now();
      // The failure is normalized AT the catch rather than stashed raw: a raw
      // `unknown` carried out of the try and stringified later is how an error object
      // turns into "[object Object]" in a user-facing message.
      let failure: { readonly message: string; readonly stack: string | undefined } | undefined;
      try {
        const scriptResult: unknown = script.runInContext(vmContext);
        const scriptPromise = scriptResult instanceof Promise ? scriptResult : Promise.resolve(scriptResult);
        // The per-message budget, re-armed here rather than shared by the batch.
        let timeoutId: ReturnType<typeof setTimeout> | undefined;
        const timeoutPromise = new Promise<never>((_, reject) => {
          timeoutId = setTimeout(() => reject(new Error(ON_MESSAGE_TIMEOUT_ERROR)), timeoutMs);
        });
        try {
          await Promise.race([scriptPromise, timeoutPromise]);
        } finally {
          clearTimeout(timeoutId);
        }
      } catch (err) {
        // Prefer the stack captured inside the vm realm (RQ-4142) — the host-side one
        // is stripped by the sandbox Worker's `Error.prepareStackTrace`.
        let capturedStack: string | undefined;
        if (err !== null && typeof err === 'object') {
          const stashed = Reflect.get(err, '__rqStack');
          if (typeof stashed === 'string' && stashed !== '') capturedStack = stashed;
        }
        failure = {
          message: err instanceof Error ? err.message : String(err),
          stack: capturedStack ?? (err instanceof Error ? err.stack : undefined),
        };
      }

      // RQ-5156 drain, per iteration: work this message's script started without
      // `await`ing must land before the next message's script can observe the same
      // collectors. The yield is a MACROTASK (`setImmediate`), which is also the
      // iteration boundary runtime 021 requires of this engine.
      const drainDeadline = iterationStart + timeoutMs;
      for (;;) {
        await new Promise((resolve) => setImmediate(resolve));
        if (pendingAsync() === 0) break;
        if (Date.now() >= drainDeadline) break;
      }

      // Collect this iteration's slice and reset the per-iteration collectors.
      // `rawMutations` is NOT reset: it accumulates across the batch (ADR-208 §6),
      // which is also what makes read-your-own-writes hold from message to message.
      for (const result of normalizeTestResults(executionState.testResults)) {
        outcome.testResults.push(stampMessageIndex(result, message.index));
      }
      executionState.testResults.length = 0;
      outcome.requestMutations.push(...executionState.requestMutations);
      executionState.requestMutations.length = 0;
      outcome.mutations = executionState.rawMutations;
      outcome.messagesCompleted += 1;

      if (failure !== undefined) {
        const errorLocation = parseScriptErrorLocation(failure.stack, scriptLines, scriptFilename, failure.message);
        outcome.messageErrors.push({
          messageIndex: message.index,
          error: failure.message,
          ...(errorLocation !== undefined ? { errorLocation } : {}),
        });
        // A timeout means the script is STILL RUNNING in this realm (nothing can kill
        // it), so the next iteration would race it over the same collectors. Abandon
        // the batch; the tail is re-queued from `messagesCompleted`.
        if (failure.message === ON_MESSAGE_TIMEOUT_ERROR) break;
        continue;
      }

      // The batch bound: host-side, NOT re-armed, checked at the boundary. An
      // iteration that overran its own budget without the timer firing (it finished
      // just after the race resolved, or it never yielded) abandons the batch here.
      if (Date.now() - iterationStart > timeoutMs) {
        outcome.messageErrors.push({ messageIndex: message.index, error: ON_MESSAGE_TIMEOUT_ERROR });
        break;
      }
    }
    return outcome;
  }
}
