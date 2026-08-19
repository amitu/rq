/**
 * QuickJsSandbox — Safe-mode script execution in a QuickJS-WASM guest realm.
 *
 * The Safe-mode engine for Sandbox Safe Mode (sandbox-node ADR-007, superseded by
 * ADR-012). Unlike `NodeSandbox` (`node:vm`, the Developer engine), this runs each
 * script in a separate QuickJS runtime — a portable WASM interpreter with its own
 * heap and NO host realm. There is no host object graph to walk to, so RQ-2489's
 * constructor-walk escape (`x.constructor.constructor('return process')()`)
 * terminates in the guest's own (inert) globals instead of yielding the host
 * `process` — the boundary is structural, not patched.
 *
 * Resource limits are real: a `runtime.setInterruptHandler` op-count poll kills a
 * runaway CPU loop (spike-confirmed), and `runtime.setMemoryLimit` bounds runaway
 * memory while the host survives. The one limitation shared with isolated-vm is
 * that neither engine can interrupt a script blocked INSIDE a host call.
 *
 * WHY QuickJS-WASM (ADR-012): isolated-vm@7 cannot link from source against
 * Electron on Windows (12 unresolved V8/v8_inspector externals Electron's Windows
 * build strips). QuickJS-WASM has no native link step, so Safe mode becomes
 * buildable on every platform incl. Windows. The `Sandbox` contract, the `rq.*`
 * namespace, the host-side bundler, the dispatcher, the UI toggle, and the
 * observability surface are all engine-independent and unchanged.
 *
 * NOTE: this engine is exported via the `@requestly/sandbox-node/isolated` subpath.
 * That subpath name is retained for continuity (the dispatcher's lazy import path),
 * but it no longer means a native-addon quarantine — QuickJS-WASM inlines into any
 * bundle, so the subpath is now just "the Safe engine half" of the package.
 */

import {
  ARRAY_METHODS_SHIM,
  CONVENIENCE_GLOBALS_SHIM,
  DEPRECATED_IDENTIFIERS,
  formatDeprecationMessage,
} from '../index.js';
// Everything host-independent the engine needs now lives in its own package
// (ADR-217): the guest realm, the bridge factory + marshaller, the five
// platform-free bridges, and the engine's own vocabulary. Importing them from
// here — rather than from a sibling host package — is what lets the browser host
// (ADR-204) run this identical engine.
// Imported from their defining modules rather than from `./index.js`: that barrel also
// exports NodeSandbox, whose static `node:vm` import then rides along into every graph
// that reaches this engine — including the browser host, which this engine exists to
// be shared with. A barrel that re-enters its own package is how a Node-only
// dependency ends up in a browser bundle without anyone importing it.
import { AsyncRegistry } from './async-registry.js';
import { SANDBOX_DEFAULT_TIMEOUT_MS } from './constants.js';
import { createInMemoryCookieJarBridge } from './cookies.js';
import { inflateMutations } from './inflate-mutations.js';
import { createConsoleBridge } from './isolated/bridges/console-bridge.js';
import { DEPRECATION_ISOLATE_SHIM, createDeprecationBridge } from './isolated/bridges/deprecation-bridge.js';
import { RUN_REQUEST_ISOLATE_SHIM, createRunRequestBridge } from './isolated/bridges/run-request-bridge.js';
import { createTimerBridges } from './isolated/bridges/timer-bridge.js';
import { CORE_GLOBALS_SHIM } from './isolated/core-globals.js';
import { dlog } from './isolated/debug-log.js';
import { RQ_COLLECT_EXPR, RQ_ISOLATE_SHIM, RQ_ITERATION_RESET_EXPR } from './isolated/isolated-rq.js';
import { AXIOS_ISOLATE_SHIM } from './isolated/shims/axios.shim.js';
import { BRU_ISOLATE_SHIM } from './isolated/shims/bru.shim.js';
import { marshalToHandle } from './isolated/marshal.js';
import { pendingAsyncCalls } from './isolated/safe-bridge-factory.js';
import { ON_MESSAGE_TIMEOUT_ERROR, buildBatchResult, createBatchOutcome, stampMessageIndex } from './on-message-batch.js';
import { UserScriptError, countScriptLines, parseScriptErrorLocation, scriptFilenameForPhase } from './script-error-location.js';
import type { SafeBridge } from './isolated/safe-bridge-factory.js';
import type { BatchOutcome } from './on-message-batch.js';
import { LogLevel, ScriptPhase } from '../index.js';
import { StreamHandle } from './stream-handle.js';

// Direct from the Node-free module: `impossible-error` also imports
// `createPackageError` from `vm-package-evaluator` (`node:vm`), which would taint
// this file's graph (ADR-204).
import { isScriptPackageUnsupportedError } from './isolated/package-error-sentinel.js';

import type { DeprecationEmit, RawScopeMutations, SafePackageResolver } from '../index.js';
import type { BundleCache } from './isolated/source-bundler.js';
import type { QuickJSAsyncWASMModule, QuickJSAsyncContext, QuickJSHandle } from 'quickjs-emscripten-core';
import type {
  ExecutionDirective,
  FeatureFlags,
  LogEntry,
  RequestHeaderMutation,
  SandboxHostCallbacks,
  ScriptExecutionContext,
  ScriptExecutionInput,
  ScriptMessageInput,
  StreamReader,
} from '../index.js';
import type { VisualizerDirective } from '../definitions/_deps.js';
// The host-side result layer (inflated result + streaming event + Sandbox surface) lives here.
import type {
  Sandbox,
  SandboxExecutionEvent,
  ScriptExecutionResult,
  TestResult,
  SendRequestHost,
} from './host-types.js';

/**
 * The engine's HOST SEAM (ADR-204 — one engine, two hosts).
 *
 * Everything below this line is host-independent engine logic. These four things
 * are not, and are therefore injected rather than imported:
 *
 * - `createModule` — the QuickJS variant differs (`-cjs-` on Node, `-browser-` in
 *   a browser), pinned to the same version by a parity test.
 * - `valueBridgeFactories` — the four Node-backed capability bridges
 *   (`buffer`/`crypto`/`util`/`zlib`). Each host supplies its own callbacks; the
 *   in-isolate shims behind them are shared verbatim.
 * - `createFetchBridge` — Node's has a direct `globalThis.fetch` fallback; the
 *   browser's is delegated-only (no DNS API, CORS, and ADR-202 puts egress in the
 *   cloud).
 * - `isolateShims` — the eval ORDER is load-bearing (`Buffer` must exist before
 *   the `zlib` shim; the deprecation shim must run after the `rq` namespace).
 *   Passed as DATA so the two hosts cannot drift in ordering.
 *
 * `requireSupport` is deliberately a separate, partly-optional seam — see its
 * own docblock.
 */
export type CreateQuickJsModule = () => Promise<QuickJSAsyncWASMModule>;

/**
 * Per-execution `require()` resolution, injected so the two hosts can support
 * DIFFERENT TIERS of the same chain.
 *
 * This is not an all-or-nothing seam, and that matters: `rq.test()` / `rq.expect()`
 * are built on Chai, and Chai is loaded THROUGH require
 * (`globalThis.__rq_chai = globalThis.require('chai')`). A host with no require
 * chain has no assertions — which would make a web sandbox close to useless.
 *
 * So the browser supplies the tiers that are Node-free (the build-time vendor
 * IIFEs that carry Chai, and the `needs_bridge` module globals) while omitting the
 * SOURCE_BUNDLE tier, whose bundler needs `node:crypto` + `node:path`. Net effect
 * on web: `require('chai')` works, `require('lodash')` does not — which is exactly
 * the S1/S2 line in the Task Brief.
 */
export interface RequireResolver {
  /** Resolve one require id to a copyable record, or throw the guided error. */
  resolve(id: string): unknown;
}

export interface RequireSupport {
  /** The in-guest `require` shim, eval'd after the capability shims. */
  readonly isolateShim: string;
  /**
   * Prepare resolution for ONE execution. Node pre-bundles here (esbuild's
   * worker-safe API is async while the guest callback is sync); the browser has
   * nothing to pre-bundle and resolves straight from the vendor table.
   */
  prepare(input: ScriptExecutionInput): Promise<RequireResolver>;
}

export interface QuickJsHostConfig {
  readonly createModule: CreateQuickJsModule;
  readonly createRequireSupport: (deps: {
    readonly resolver?: SafePackageResolver;
    readonly bundleCache?: BundleCache;
  }) => RequireSupport;
  readonly valueBridgeFactories: readonly (() => SafeBridge)[];
  readonly createFetchBridge: (host: SendRequestHost | undefined) => SafeBridge;
  readonly isolateShims: readonly string[];
}

/** Per-runtime memory ceiling (bytes). Bounds an OOM script without killing the host. */
const ISOLATE_MEMORY_LIMIT_BYTES = 128 * 1024 * 1024;

/**
 * Interrupt budget: the op-count poll kills a runaway loop. QuickJS calls the
 * interrupt handler every N bytecode ops; we count calls and abort past this
 * ceiling. This is the CPU-time kill (spike-confirmed against `while(true){}`).
 * The wall-clock timeout (`input.timeoutMs`) is enforced in tandem.
 */
const INTERRUPT_OP_CEILING = 1_000_000;

/**
 * Memoized QuickJS-WASM module load. The WASM (base64-embedded in the asyncify
 * single-file variant) compiles once per process; every execution reuses the
 * module and only creates a fresh `runtime`/`context` (cheap). Memoizing the
 * promise — not the resolved value — means concurrent first-callers share one
 * compile.
 */
let quickJsModulePromise: Promise<QuickJSAsyncWASMModule> | undefined;
function getQuickJsModule(createModule: CreateQuickJsModule): Promise<QuickJSAsyncWASMModule> {
  if (quickJsModulePromise === undefined) {
    dlog('module', 'compiling QuickJS WASM module (cold)');
    quickJsModulePromise = createModule();
  } else {
    dlog('module', 'reusing memoized WASM module (warm)');
  }
  return quickJsModulePromise;
}

/**
 * Drop the memoized WASM module so the NEXT execution compiles a fresh one.
 * Called only after a timeout kill: an interrupt-killed asyncified frame leaves a
 * deferred host-ref free queued at the WASM-MODULE level (not the runtime level) —
 * leaking the killed runtime is not enough, because that orphaned free detonates
 * (as an unhandled `freeHostRef` rejection that crashes the worker) on the next
 * runtime created in the SAME module. Abandoning the module isolates the corruption
 * to the killed execution; the next `getQuickJsModule()` compiles a clean one
 * (~15ms cold / ~3ms warm — negligible, and only paid after a rare timeout).
 */
function resetQuickJsModule(): void {
  dlog('module', 'ABANDONING memoized WASM module (post-timeout-kill) — next run recompiles');
  quickJsModulePromise = undefined;
}

/** Shape the in-isolate rq shim serializes out (mirrors RQ_COLLECT_EXPR). */
interface CollectedFromIsolate {
  testResults?: TestResult[];
  mutations?: RawScopeMutations;
  requestMutations?: RequestHeaderMutation[];
  executionDirective?: ExecutionDirective;
  visualizerOutput?: VisualizerDirective;
}

/** Type guard validating a guest-emitted request header mutation (ADR-167). */
function isRequestHeaderMutation(value: unknown): value is RequestHeaderMutation {
  if (typeof value !== 'object' || value === null) return false;
  const v: Record<string, unknown> = { ...value };
  // `clear` carries no name/value (RQ-3720) — recognize it before the name check.
  if (v['kind'] === 'clear') return true;
  if (typeof v['name'] !== 'string') return false;
  if (v['kind'] === 'remove') return true;
  return (v['kind'] === 'add' || v['kind'] === 'upsert') && typeof v['value'] === 'string';
}

/** Type guard validating a guest-emitted flow-control directive (ADR-169). */
function isExecutionDirective(value: unknown): value is ExecutionDirective {
  if (typeof value !== 'object' || value === null) return false;
  const v: Record<string, unknown> = { ...value };
  if (v['kind'] === 'skip-request') return true;
  return v['kind'] === 'set-next-request' && (typeof v['target'] === 'string' || v['target'] === null);
}

/** Type guard validating a guest-emitted visualizer intent — a compiled/error output or a `clear()` marker (ADR-202, FR-18). */
function isVisualizerDirective(value: unknown): value is VisualizerDirective {
  if (typeof value !== 'object' || value === null) return false;
  const v: Record<string, unknown> = { ...value };
  // An explicit clear() drains as `{ kind: 'cleared' }` — distinct from absent so it
  // overrides an earlier phase's set() (FR-18c); the runtime strips it at the lift.
  if (v['kind'] === 'cleared') return true;
  if (v['kind'] === 'error') return typeof v['message'] === 'string';
  // `data` is any JsonValue (incl. null / false / 0), so assert presence, not truthiness.
  return v['kind'] === 'compiled' && typeof v['html'] === 'string' && 'data' in v;
}

/**
 * Parse the JSON the in-guest rq shim emits into its raw shape.
 *
 * Split out of `buildResult` because the on-message batch loop drains this ONCE
 * PER ITERATION and must accumulate the raw slices, inflating the mutations only
 * once at the end (ADR-208 §6) — inflating per iteration would both waste the
 * work and lose the accumulate-once contract.
 */
function parseCollected(collectedJson: string): CollectedFromIsolate {
  let collected: CollectedFromIsolate = {};
  try {
    const parsed: unknown = JSON.parse(collectedJson);
    if (parsed && typeof parsed === 'object') {
      // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- guest JSON from own RQ_COLLECT_EXPR shim; narrowing from unknown after typeof guard
      const obj = parsed as Record<string, unknown>;
      collected = {
        // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- array elements match TestResult by construction (RQ_COLLECT_EXPR shim)
        testResults: Array.isArray(obj['testResults']) ? (obj['testResults'] as TestResult[]) : [],
        mutations:
          obj['mutations'] && typeof obj['mutations'] === 'object'
            ? // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- narrowing from unknown after typeof guard
              (obj['mutations'] as Record<string, unknown>)
            : undefined,
        requestMutations: Array.isArray(obj['requestMutations'])
          ? obj['requestMutations'].filter(isRequestHeaderMutation)
          : [],
        executionDirective: isExecutionDirective(obj['executionDirective']) ? obj['executionDirective'] : undefined,
        visualizerOutput: isVisualizerDirective(obj['visualizerOutput']) ? obj['visualizerOutput'] : undefined,
      };
    }
  } catch {
    // No collection (e.g. empty script) — fall through to an empty result.
  }
  return collected;
}

/**
 * Inflate a parsed guest collection into a ScriptExecutionResult. The mutations
 * are raw (RawScopeMutations); the host inflates them to a MutationDiff with full
 * VariableData (ADR-053 Layer 2), reusing the same `inflateMutations` NodeSandbox
 * uses so both engines produce identical diffs.
 */
function buildResult(collectedJson: string, context: ScriptExecutionContext): ScriptExecutionResult {
  const collected = parseCollected(collectedJson);
  const testResults = collected.testResults ?? [];
  const mutationDiff = collected.mutations ? inflateMutations(collected.mutations, context) : {};
  const requestMutations = collected.requestMutations ?? [];
  const directive = collected.executionDirective;
  const visualizerOutput = collected.visualizerOutput;
  return {
    mutationDiff,
    logs: [],
    testResults,
    ...(requestMutations.length > 0 ? { requestMutationDiff: { headers: requestMutations } } : {}),
    ...(directive !== undefined ? { executionDirective: directive } : {}),
    ...(visualizerOutput !== undefined ? { visualizerOutput } : {}),
  };
}

/**
 * Eval an in-guest expression and read its result out as a copied string. Used
 * for the rq-collection drain (a single JSON string crosses — copied data,
 * nothing live). Disposes the result handle. Returns `'{}'` if the eval failed
 * or did not produce a string.
 */
function evalStringOut(ctx: QuickJSAsyncContext, expr: string): string {
  const r = ctx.evalCode(expr);
  if (r.error) {
    dlog('evalStringOut', 'eval returned error (returning {})', { expr: expr.slice(0, 40) });
    r.error.dispose();
    return '{}';
  }
  const dumped: unknown = ctx.dump(r.value);
  r.value.dispose();
  return typeof dumped === 'string' ? dumped : '{}';
}

/**
 * Set a copied string as a guest global and dispose the handle. Used to copy the
 * serialized ScriptExecutionContext in (a single string crosses — nothing live).
 */
function setStringGlobal(ctx: QuickJSAsyncContext, name: string, value: string): void {
  const handle = ctx.newString(value);
  ctx.setProp(ctx.global, name, handle);
  handle.dispose();
}

/**
 * The kill condition's mutable state, shared with the interrupt handler.
 *
 * Mutable rather than closed-over constants because the on-message batch loop
 * RE-ARMS both halves at each iteration boundary (runtime 021 §Per-message
 * deadline): `timeoutMs` is a per-MESSAGE budget, so five 120ms iterations under a
 * 200ms budget must all complete while one runaway iteration is still killed at
 * ~200ms. `tripped` is the only reliable kill signal — an interrupt kill unwinds
 * the guest frame without surfacing as a catchable exception or an eval error.
 */
interface TimeoutState {
  deadline: number;
  interruptCalls: number;
  tripped: boolean;
}

/**
 * Safe-mode sandbox engine. Each `execute()` call creates and disposes a fresh
 * QuickJS runtime (per-execution lifecycle — no state leakage, matching ADR-001's
 * fresh-context-per-run semantics). Implements the unchanged `Sandbox` interface,
 * so it is a drop-in for `NodeSandbox` behind the dispatcher (ADR-008).
 */
export class QuickJsEngine implements Sandbox {
  private readonly resolver: SafePackageResolver | undefined;
  private readonly bundleCache: BundleCache | undefined;

  // Constructor signature mirrors NodeSandbox so the eventual CLI migration
  // (deferred, TB §5.3.3) is a one-line swap. The engine must also run in-process
  // with no resolver (the CLI's posture). When a resolver IS provided, Tier 4 of
  // the in-guest require chain resolves user-installed npm packages to source
  // and bundles them (ADR-010 §85). When a `bundleCache` is provided (desktop),
  // SOURCE_BUNDLE output is persisted across executions (ADR-010 §78, R-5);
  // absent, the bundler uses its in-memory default (CLI / in-process posture).
  private readonly host: QuickJsHostConfig;

  constructor(
    resolver: SafePackageResolver | undefined,
    bundleCache: BundleCache | undefined,
    host: QuickJsHostConfig,
  ) {
    this.resolver = resolver;
    this.bundleCache = bundleCache;
    // Host-agnostic by construction: there is no default, so a host MUST be
    // supplied. `quickjs-sandbox.ts` supplies the Node one and keeps the
    // historical `QuickJsSandbox` constructor shape.
    this.host = host;
  }

  getFeatures(): Promise<FeatureFlags> {
    // NOTE: the `isolatedVm` FeatureFlags key is a pre-existing misnomer on
    // NodeSandbox (ADR-007 "Noted"). Reinterpreting/renaming it is a flagged
    // non-blocking follow-up; this engine reports it truthfully (it IS isolated)
    // plus externalPackages capability parity with NodeSandbox.
    return Promise.resolve({ isolatedVm: true, externalPackages: this.resolver !== undefined });
  }

  // `hostCallbacks.runRequest` (when present) is wired into the QuickJS isolate
  // as the `__rq_runRequest` bridge (ADR-169 §Safe-mode containment): it MUST go
  // through safe-bridge-factory (copy-in/copy-out), never a raw host reference —
  // so RQ-2489's host-realm escape stays structurally closed.
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
    // Per-execution QuickJS runtime + context. Created in the try so a
    // construction failure is reported on the stream rather than thrown into the
    // void wrapper. Disposed unconditionally in finally (QuickJS requires every
    // runtime/context be disposed — undisposed handles abort at teardown).
    let runtime: ReturnType<QuickJSAsyncWASMModule['newRuntime']> | undefined;
    let context: QuickJSAsyncContext | undefined;
    // Names of every host-backed function global installed into the guest. They
    // MUST be nulled before teardown — a host-fn handle still referenced by a
    // guest global aborts `JS_FreeRuntime` (gc_obj_list / freeHostRef) under this
    // quickjs-emscripten build. See `disposeGuest`.
    const installedGlobals: string[] = [];
    // Per-execution async registry (ADR-219) — the single owner of "what counts
    // as pending work". Declared outside the try so the `finally` can seal it on
    // EVERY exit path: a live host timer that outlives the execution would fire
    // into a disposed context. Node's timers are the injected delegations; the
    // registry itself is platform-neutral (ADR-217).
    // A throw inside a timer callback is reported, never fatal — the same
    // seal-and-warn posture RQ-5156 chose for unfinished async work, and the
    // parity target for postman-sandbox's `Timerz` onError (timers.js:200-201).
    // Two paths reach this: the registry's own `onCallbackError` (a host-side
    // callback, which in Safe mode only resolves a promise and cannot throw) and
    // `__rq_timerError` (the in-guest catch, which is the one that actually fires
    // for user script callbacks, since they run inside the isolate).
    const reportTimerCallbackError = (message: string): void => {
      handle.push({
        type: 'log',
        log: {
          level: LogLevel.error,
          args: [`Uncaught error in timer callback: ${message}`],
          timestamp: Date.now(),
        },
      });
    };
    const asyncRegistry = new AsyncRegistry<ReturnType<typeof setTimeout>>({
      timers: {
        scheduleTimer: (fn, ms) => setTimeout(fn, ms),
        cancelTimer: (timerHandle) => {
          clearTimeout(timerHandle);
        },
      },
      onCallbackError: (error) => {
        reportTimerCallbackError(error instanceof Error ? error.message : String(error));
      },
    });
    // Set when the interrupt handler kills a runaway script. A killed frame leaves
    // the asyncify machinery holding host-refs whose free is DEFERRED; disposing
    // the runtime then surfaces an unhandled `freeHostRef` rejection during a LATER
    // execution (runtimes share one memoized WASM module) — which would crash the
    // worker. On this path we LEAK the killed runtime instead of disposing it (the
    // Bruno-validated posture: a timeout is an exceptional event, the worker is
    // recycled periodically, and a leak is strictly safer than a worker crash).
    let killedByTimeout = false;
    // Stash for a typed `ScriptPackageUnsupportedError` thrown by `resolveRequire`
    // inside the require `newFunction`. A host throw inside a QuickJS `newFunction`
    // aborts the runtime (it does NOT propagate the typed error host-side), so the
    // require callback CATCHES the throw, records it here, and returns a guest error
    // the in-guest `require` shim re-throws as a catchable exception. The outer
    // catch reads this stash to lift the analytics classification onto
    // `errorDetails` (ADR-010 §87) — preserving the `Script Package Unsupported`
    // signal that the throw would otherwise lose.
    let requireImpossibleError: unknown;

    dlog('run', 'runScript ENTER', {
      phase: input.phase,
      entryId: input.entryId,
      scriptLen: input.script.length,
      timeoutMs: input.timeoutMs,
    });
    try {
      const QuickJS = await getQuickJsModule(this.host.createModule);
      dlog('run', 'module ready; creating runtime');
      runtime = QuickJS.newRuntime();
      runtime.setMemoryLimit(ISOLATE_MEMORY_LIMIT_BYTES);
      context = runtime.newContext();
      const ctx = context;
      dlog('run', 'runtime + context created');

      // ── Wire the guest global self-reference ──
      // QuickJS already exposes `globalThis`; alias `global` to it so user code
      // that reads `global` works. Nothing host-side crosses (the HARD INVARIANT)
      // — this aliases the guest's OWN global object to itself.
      ctx.setProp(ctx.global, 'global', ctx.global);

      // ── Install the Safe-mode host-capability surface (ADR-010) ──
      // Every bridge below is authored through the typed bridge-factory and
      // installs via the factory's marshalling wrapper, so only COPIED DATA
      // crosses the guest edge (the HARD INVARIANT). The bridge's `install(ctx)`
      // is the SOLE site a `ctx.newFunction`/`newAsyncifiedFunction` is born for a
      // capability bridge (enforced by `no-bridge-outside-factory`).
      const installBridge = (bridge: SafeBridge): void => {
        dlog('run', 'installing bridge', { name: bridge.name });
        bridge.install(ctx);
        installedGlobals.push(bridge.name);
      };

      // Console bridge — fire-and-forget; the guest serializes args to a JSON
      // string and the host pushes a `log` event. `now` is injected so the engine
      // keeps ownership of `Date.now`.
      installBridge(
        createConsoleBridge(
          (log: LogEntry) => handle.push({ type: 'log', log }),
          () => Date.now(),
        ),
      );

      // Deprecation bridge — Safe-mode parity with NodeSandbox's legacy Postman
      // identifiers (ADR-156). The chokepoint mirrors `node-sandbox.ts`
      // (structurally equivalent, same event shape): on a deprecated identifier's first access the guest
      // calls `__rq_deprecation`, and the host pushes a `deprecation` signal +
      // (when the identifier has a policy) a warn `log` with the static guidance
      // message. The signal object is built host-side, so it needs no realm
      // roundtrip and crosses the RPC boundary directly (ADR-034). The in-guest
      // shim (DEPRECATION_ISOLATE_SHIM) is eval'd below, AFTER the rq namespace.
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
      installBridge(createDeprecationBridge(emitDeprecation));

      // Value bridges (Buffer, crypto, util, zlib). Sync bridges install as a
      // `ctx.newFunction`. stream/process have no host callback (pure in-guest
      // shims, below).
      for (const make of this.host.valueBridgeFactories) {
        installBridge(make());
      }

      // Fetch bridge (ADR-181/182) — ALWAYS installed (the shim provides
      // `globalThis.fetch` unconditionally), but takes an optional `SendRequestHost`.
      // When the host supplies `sendRequest` (desktop and the scheduled-run runner),
      // a script's `fetch` is delegated through the runtime fetcher (the single egress
      // chokepoint); otherwise `undefined` flows through and the bridge keeps the
      // direct `globalThis.fetch` path (the CLI today). Installed explicitly — like
      // the console/runRequest bridges — because it is no longer a zero-arg factory.
      // Seam: cross-q-context's SandboxHostCallbacks is loosely typed (Json in/out) to stay
      // circular-import-free; the host provides the concrete SendRequestHost/RunRequestHost shape.
      installBridge(
        this.host.createFetchBridge(
          hostCallbacks?.sendRequest as unknown as Parameters<typeof this.host.createFetchBridge>[0],
        ),
      );

      // runRequest bridge (ADR-169) — installed ONLY when the host supplies the
      // `runRequest` callback (absent on the CLI / in-process posture). Like the
      // fetch bridge it is an async copy-in/copy-out `__rq_runRequest` callback
      // authored through the bridge-factory, so only copied data crosses the guest
      // edge — the live RunRequestHost stays host-side (RQ-2489 stays closed). The
      // host callback global is installed here with the other bridges; its in-guest
      // shim (RUN_REQUEST_ISOLATE_SHIM) is eval'd AFTER the rq shim, below, so
      // `globalThis.rq.execution` already exists to attach `runRequest` onto.
      if (hostCallbacks?.runRequest) {
        installBridge(
          createRunRequestBridge(hostCallbacks.runRequest as unknown as Parameters<typeof createRunRequestBridge>[0]),
        );
      }

      // Timer bridges (RQ-5154, ADR-219) — the isolate has no clock, so
      // `setTimeout` & co. in CORE_GLOBALS_SHIM await these. Only numbers cross;
      // the guest keeps its own id→callback table. Every timer holds the run
      // open, intervals included — matching Postman, bounded by our budget.
      for (const bridge of createTimerBridges(asyncRegistry, reportTimerCallbackError)) {
        installBridge(bridge);
      }

      // Cookie jar bridge (ADR-105). Sync host callback: the guest calls
      // __rq_cookies({op, host, ...}) and gets back copied data. The in-memory
      // bridge accumulates mutations; the engine drains them after the script.
      const cookieBridgeHandle = createInMemoryCookieJarBridge(
        input.context.cookieJarSeed as unknown as Parameters<typeof createInMemoryCookieJarBridge>[0],
      );
      const hostAllowlist = new Set((input.context.hostAllowlist ?? []).map((h: string) => h.toLowerCase()));
      const cookieFnHandle = ctx.newFunction('__rq_cookies', (argsHandle: QuickJSHandle) => {
        const raw: unknown = ctx.dump(argsHandle);
        if (!raw || typeof raw !== 'object') return marshalToHandle(ctx, { error: 'invalid cookie args' });
        // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- narrowed by typeof guard above
        const args = raw as Record<string, unknown>;
        const op = typeof args['op'] === 'string' ? args['op'] : '';
        const host = typeof args['host'] === 'string' ? args['host'] : '';
        if (!host || !hostAllowlist.has(host.toLowerCase())) {
          return marshalToHandle(ctx, { error: `CookieStore: programmatic access to "${host}" is denied.` });
        }
        if (op === 'list') {
          return marshalToHandle(ctx, { result: cookieBridgeHandle.bridge.list(host) });
        }
        if (op === 'upsert') {
          // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- guest cookie object validated by the createCookiesNamespace shim before crossing
          const cookie = args['cookie'] as Parameters<typeof cookieBridgeHandle.bridge.upsert>[1];
          cookieBridgeHandle.bridge.upsert(host, cookie);
          return marshalToHandle(ctx, { result: args['cookie'] });
        }
        if (op === 'remove') {
          const name = typeof args['name'] === 'string' ? args['name'] : '';
          const path = typeof args['path'] === 'string' ? args['path'] : '/';
          cookieBridgeHandle.bridge.remove(host, name, path);
          return marshalToHandle(ctx, { result: null });
        }
        if (op === 'clear') {
          cookieBridgeHandle.bridge.clear(host);
          return marshalToHandle(ctx, { result: null });
        }
        return marshalToHandle(ctx, { error: 'unknown cookie op' });
      });
      ctx.setProp(ctx.global, '__rq_cookies', cookieFnHandle);
      cookieFnHandle.dispose();
      installedGlobals.push('__rq_cookies');
      // Pass allowlist to the guest for the rq.cookies shim
      setStringGlobal(ctx, '__rq_hostAllowlist_json', JSON.stringify(input.context.hostAllowlist ?? []));
      // Pass the script phase to the guest for the rq.execution shim (ADR-169).
      // `phase` is NOT part of the copied ScriptExecutionContext (it lives on
      // ScriptExecutionInput.phase, mirroring the Developer engine's separate
      // `eventName` parameter), so it is threaded in as its own copied string —
      // 'pre-request' / 'post-response' — gating `rq.execution.skipRequest`.
      setStringGlobal(ctx, '__rq_phase', input.phase);

      // The require dispatch callback (ADR-010 §82). The host reads the registry
      // and resolves each require id to a bridge-global pointer or bundled source
      // (or throws the guided IMPOSSIBLE error). Only copied data crosses. This is
      // engine plumbing (not a capability bridge), so the engine file owns the
      // `ctx.newFunction` — the lint rule exempts this file (same exemption the
      // ivm-era engine had for `ivm.Callback`).
      // Built ONCE per execution: `prepare()` and `isolateShim` must come from the
      // same instance. Calling the factory twice would construct a throwaway
      // bundler on the Node host purely to read a string.
      const requireSupport = this.host.createRequireSupport({
        resolver: this.resolver,
        bundleCache: this.bundleCache,
      });
      const requireResolver = await requireSupport.prepare(input);

      const requireFn = ctx.newFunction('__rq_bundleRequire', (idHandle: QuickJSHandle) => {
        const id = ctx.getString(idHandle);
        dlog('require', 'guest require()', { id });
        try {
          const resolved = requireResolver.resolve(id);
          dlog('require', 'resolved ok', { id });
          return marshalToHandle(ctx, resolved);
        } catch (requireErr) {
          dlog('require', 'resolve threw → guest error', {
            id,
            msg: requireErr instanceof Error ? requireErr.message.slice(0, 60) : String(requireErr),
          });
          requireImpossibleError = requireErr;
          const message = requireErr instanceof Error ? requireErr.message : String(requireErr);
          return { error: ctx.newError(message) };
        }
      });
      ctx.setProp(ctx.global, '__rq_bundleRequire', requireFn);
      requireFn.dispose();
      installedGlobals.push('__rq_bundleRequire');

      // Copy the serializable ScriptExecutionContext IN as a JSON string and parse
      // it inside the guest. The boundary types already satisfy
      // `gr-serializable-boundary-data` (ADR-034), so JSON is lossless here; a
      // single copied string crosses the edge — no function or live reference.
      // Exposed as `__rq_context` for the in-guest rq shim to read.
      try {
        setStringGlobal(ctx, '__rq_context_json', JSON.stringify(input.context));
      } catch (copyErr) {
        // The context failed JSON serialization — a boundary violation to fix at
        // source, surfaced rather than silently dropped. Static message for Sentry
        // grouping; the underlying failure rides the cause chain
        // (gr-static-error-messages / gr-preserve-cause-chains).
        throw new Error('Failed to copy script context into the guest realm', { cause: copyErr });
      }

      // ── Eval the in-guest shims (ADR-010) ──
      // 0. Parse the copied context string into the object the rq shim reads.
      this.evalOrThrow(ctx, `globalThis.__rq_context = JSON.parse(globalThis.__rq_context_json);`, 'context-parse');
      // 1. Core globals — QuickJS ships fewer intrinsics than V8 (no TextEncoder/
      //    atob/EventTarget/queueMicrotask/timers); SOURCE_BUNDLE packages (Chai)
      //    and the capability shims depend on them. Pure in-guest JS.
      this.evalOrThrow(ctx, CORE_GLOBALS_SHIM, 'core-globals');
      // 1b. Array.prototype.first/.last — Postman parity (ADR-192, Slice 2). Eval'd
      //     IN-GUEST so the prototype patch cannot leak to the host (structurally
      //     impossible in QuickJS-WASM). No dependency on `globalThis.rq`, so its
      //     ordering vs the rq namespace is free. Shared constant with NodeSandbox.
      this.evalOrThrow(ctx, ARRAY_METHODS_SHIM, 'array-methods');
      // 2. Capability shims (console, process, Buffer, crypto, util, stream,
      //    zlib, fetch) — build the user-facing API on the installed callbacks.
      this.host.isolateShims.forEach((shim, i) => {
        this.evalOrThrow(ctx, shim, `capability-shim[${i}]`);
      });
      // 3. The require chain — `globalThis.require` over `__rq_bundleRequire`.
      this.evalOrThrow(ctx, requireSupport.isolateShim, 'require-chain');
      // 3b. Lazy convenience globals — bare `CryptoJS` over `require('crypto-js')`
      //     (RQ-5512) and bare `_` over `require('lodash')` (RQ-5613). MUST run
      //     AFTER step 3: each accessor resolves through `globalThis.require`.
      //     Shared verbatim with NodeSandbox so the two engines cannot drift.
      //     Nothing is bundled until a script touches the global, so a script that
      //     never uses one pays nothing. (`xml2Json` is NOT here — xml2js needs the
      //     Node built-in `events`, not yet requireable in Safe; deferred.)
      this.evalOrThrow(ctx, CONVENIENCE_GLOBALS_SHIM, 'convenience-globals');
      // 4. Chai (a SOURCE_BUNDLE package) → exposed as `__rq_chai` for the rq shim.
      this.evalOrThrow(ctx, `globalThis.__rq_chai = globalThis.require('chai');`, 'chai-load');
      // 4b. On-message only: the message the rq shim binds `rq.message` from
      //     (ADR-208 §9). Copied in as its own JSON string — like the context —
      //     so nothing live crosses. A batch re-points it per iteration via the
      //     shim's `__rq_setMessage`, so this seeds only the first (or, for a
      //     single-message execute, the only) iteration.
      if (input.phase === ScriptPhase.onMessage) {
        setStringGlobal(ctx, '__rq_message_json', JSON.stringify(input.context.message ?? null));
        this.evalOrThrow(ctx, `globalThis.__rq_message = JSON.parse(globalThis.__rq_message_json);`, 'message-parse');
      }
      // 5. The rq.* namespace over the copied context + Chai.
      this.evalOrThrow(ctx, RQ_ISOLATE_SHIM, 'rq-namespace');
      // See execute.ts — Bruno's globals, mapped onto the rq namespace.
      this.evalOrThrow(ctx, BRU_ISOLATE_SHIM, 'bru-compat');
      this.evalOrThrow(ctx, AXIOS_ISOLATE_SHIM, 'axios-facade');
      // 6. Legacy Postman deprecation identifiers (ADR-156 parity). MUST run AFTER
      //    step 5 — the `globals`/`environment`/`responseBody`/`responseCode` shims
      //    delegate to `globalThis.rq`. Installed unconditionally, matching
      //    NodeSandbox (which seeds all 14 identifiers for every execution).
      this.evalOrThrow(ctx, DEPRECATION_ISOLATE_SHIM, 'deprecation-shim');
      // 7. runRequest shim (ADR-169) — attaches `rq.execution.runRequest` over the
      //    `__rq_runRequest` bridge. MUST run AFTER step 5 so `globalThis.rq.execution`
      //    already exists (the shim no-ops if it doesn't). Only eval'd when the host
      //    supplied the callback (the bridge global is installed above in tandem).
      if (hostCallbacks?.runRequest) {
        this.evalOrThrow(ctx, RUN_REQUEST_ISOLATE_SHIM, 'run-request-shim');
      }

      // ── Timeout: interrupt-poll CPU kill + wall-clock deadline ──
      // The op-count interrupt kills a runaway loop; the wall-clock deadline backs
      // it for time spent between interrupt polls. (Neither can interrupt a script
      // blocked INSIDE a host call — same limitation as isolated-vm.)
      const timeoutMs = input.timeoutMs ?? SANDBOX_DEFAULT_TIMEOUT_MS;
      // `tripped` is the ONLY reliable kill signal: a runaway loop killed by the
      // interrupt handler unwinds the whole guest frame — the termination is NOT a
      // catchable JS exception (the in-guest try/catch never runs) and does NOT
      // surface on `evalCodeAsync`'s result through the async-IIFE wrapper (the IIFE
      // promise is left pending). So we record that the handler fired a stop and
      // treat that as the timeout, regardless of the eval result. (Confirmed
      // empirically — see the QuickJS run-loop notes, and the permanent guard in
      // `__tests__/interrupt-catchability.probe.test.ts`.)
      const timeout: TimeoutState = { deadline: Date.now() + timeoutMs, interruptCalls: 0, tripped: false };
      runtime.setInterruptHandler(() => {
        const stop = ++timeout.interruptCalls > INTERRUPT_OP_CEILING || Date.now() > timeout.deadline;
        if (stop) timeout.tripped = true;
        return stop;
      });

      const scriptFilename = scriptFilenameForPhase(input.phase);

      // ── On-message: one iteration per batch element (ADR-208 §7) ──
      // A batch is the only input shape that varies between an execute() over K
      // messages and K single executions, and the loop is driven from the HOST
      // rather than inside the guest. That is deliberate, and it is what discharges
      // runtime 021's four obligations at once: the host owns the iteration
      // boundary, so it can re-arm the per-message budget, drain the iteration's
      // results across the edge BEFORE the next one starts, and enforce a batch
      // bound the guest cannot skip. A guest-side loop cannot do the last two —
      // the host is blocked inside `evalCode` while the guest runs, and an
      // iteration whose overrun is spent inside a host call is never interruptible.
      if (input.messageBatch !== undefined) {
        const outcome = await this.runMessageBatch(ctx, runtime, input, input.messageBatch, timeout, timeoutMs);
        killedByTimeout = outcome.killedByTimeout;
        if (!killedByTimeout) runtime.removeInterruptHandler();
        handle.push({
          type: 'result',
          result: buildBatchResult(
            outcome,
            outcome.mutations ? inflateMutations(outcome.mutations, input.context) : {},
            cookieBridgeHandle.drainMutations(),
          ),
        });
        handle.end();
        dlog('run', 'runScript SUCCESS (batch)', {
          completed: outcome.messagesCompleted,
          of: input.messageBatch.length,
          killed: killedByTimeout,
        });
        return;
      }

      // ── Eval the script (MODULE mode + top-level try/catch) ──
      // Eval the user script in MODULE mode (`{ type: 'module' }`) so the user's
      // own top-level `await` is legal WITHOUT an async-IIFE wrapper. The wrapper
      // is a trap (confirmed empirically): an interrupt kill unwinds its frame
      // without surfacing the kill, AND the dead asyncify frame pins the shims'
      // host-refs so teardown emits an async `freeHostRef` rejection that crashes
      // the worker. Module mode avoids both — a kill surfaces as `r.error`
      // ("interrupted") and teardown is clean. We still wrap the body in a TOP-
      // LEVEL try/catch (NOT an IIFE) so a thrown OR async-rejected user error is
      // captured to `__rq_error` (a bare module rejection wouldn't surface on
      // `r.error`). The asyncify variant drives awaited HOST calls (e.g. the fetch
      // bridge) to completion during `evalCodeAsync`. Trailing newline before the
      // catch guards a final line comment in the user script.
      // ── Eval the script as an async IIFE + resolvePromise (the Bruno pattern) ──
      // Wrap the body in `(async () => { try { <script> } catch { __rq_error } })()` so
      // top-level `await` is legal. `evalCode` (sync, NOT `evalCodeAsync`) returns the
      // IIFE's guest promise handle. `resolvePromise` awaits it — and because every
      // async bridge uses the guest-promise + `executePendingJobs` pump pattern (not
      // asyncified WASM suspend), the promise chain advances correctly on each host-side
      // settlement. This avoids the module-mode teardown race (RQ-3359): `evalCodeAsync`
      // in module mode returned while asyncified frames were still suspended; here
      // `resolvePromise` doesn't return until the entire script (including all awaited
      // host calls) has settled.
      const wrappedScript = `(async () => { try {\n${input.script}\n} catch (e) { globalThis.__rq_error = (e && e.constructor && e.constructor.name && e.constructor.name !== 'Error' ? e.constructor.name + ': ' : '') + ((e && e.message) ? String(e.message) : String(e)); globalThis.__rq_stack = (e && e.stack) ? String(e.stack) : ''; } })()`;
      dlog('run', 'evalCode START (async IIFE + resolvePromise pattern)', { timeoutMs });
      const evalResult = ctx.evalCode(wrappedScript, scriptFilename);
      if (evalResult.error) {
        const runMessage = this.dumpErrorMessage(ctx, evalResult.error);
        // Syntax/compile errors surface here (the in-guest try/catch never runs).
        // Read the stack before disposing so we can point at the offending line too.
        const runStack = this.dumpErrorStack(ctx, evalResult.error);
        evalResult.error.dispose();
        dlog('run', 'evalCode sync error (syntax?)', { msg: runMessage.slice(0, 80) });
        const errorLocation = parseScriptErrorLocation(
          runStack,
          countScriptLines(input.script),
          scriptFilename,
          runMessage,
        );
        throw new UserScriptError(runMessage, errorLocation);
      }
      // The IIFE returned a guest promise. Drive it to settlement with a pump loop:
      // `executePendingJobs` advances the guest microtask chain (resolving the IIFE
      // promise for sync scripts) + the async yield lets host promises (fetch) settle
      // (their `.settled.then(executePendingJobs)` callback then advances the guest
      // await chain on the next pump). `resolvePromise` is NOT used — it deadlocks on
      // sync scripts (it waits for the job pump to fire, but nobody is pumping).
      // Bounded by the wall-clock deadline so a genuinely-hung host call doesn't
      // block forever.
      // RQ-5156: the loop also continues while a host bridge call is in flight or
      // guest jobs are queued, so work the script started WITHOUT `await`ing (the
      // `rq.sendRequest(url, cb)` shape) completes before the context is disposed.
      // Without this the IIFE settles immediately on such a script, the context is
      // torn down mid-flight, and the callback's `rq.*` writes vanish silently.
      // The deadline/interrupt break below still bounds the whole loop, and the
      // post-loop path is unchanged (a killed runtime must never be pumped).
      await this.pumpToSettlement(ctx, runtime, evalResult.value, timeout);
      evalResult.value.dispose();

      // Remove the interrupt handler immediately so it holds no closure over a
      // leaked runtime.
      runtime.removeInterruptHandler();

      // A timeout kill surfaces here via the tripped flag (the kill is not a
      // catchable guest exception). Do NOT touch the killed runtime further — no
      // job pump, no eval, no dispose: the killed frame's deferred host-ref free is
      // the landmine, and ANY further interaction (including `executePendingJobs`)
      // detonates it as an unhandled `freeHostRef` rejection that crashes the
      // worker (the runtimes share one memoized WASM module). Mark it leaked and
      // surface the timeout error; `finally` skips disposal.
      if (timeout.tripped) {
        killedByTimeout = true;
        dlog('run', 'TIMEOUT kill — leaking runtime + resetting module');
        throw new Error('Safe-mode script exceeded the execution timeout');
      }

      // Non-killed path: flush any remaining guest microtasks.
      dlog('run', 'draining pending jobs');
      runtime.executePendingJobs();

      // RQ-5156 seal-and-warn (parity with the Developer engine): the pump loop can
      // exit on the deadline with a host bridge call still in flight. Say so instead
      // of silently dropping its effects — but never fail the run over it, so a
      // request that passes today cannot start failing because of this drain.
      const unfinishedAsync = pendingAsyncCalls(ctx);
      if (unfinishedAsync > 0) {
        dlog('run', 'sealing with unfinished async bridge calls', { unfinishedAsync });
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

      // A thrown/rejected user error was captured to `__rq_error` by the top-level
      // catch. Surface it as the run error (matching isolated-vm's await-then-throw)
      // — UNLESS it is the rq.execution.skipRequest() abort. skipRequest() throws to
      // stop the rest of the pre-request script (Postman parity, ADR-169); the
      // directive was already collected on the guest global BEFORE the throw, so the
      // drain below picks it up. A `skip-request` directive means the throw was the
      // intended termination, not an error — surface a CLEAN result carrying the
      // directive (mirrors the Developer engine's `SkipRequestSignal` instanceof path).
      const userError = evalStringOut(ctx, `globalThis.__rq_error || ''`);
      // The thrown error's `.stack` (captured alongside `__rq_error` in the guest
      // catch) carries the file/line/column frames. Read it out via the same
      // copy-out helper and map it back to the user's editor (RQ-4142).
      const userStack = evalStringOut(ctx, `globalThis.__rq_stack || ''`);

      // Drain the in-guest rq collection as COPIED DATA (a JSON string crosses,
      // nothing live). Parse it host-side into the result event (ADR-010 SCOPE).
      const collectedJson = evalStringOut(ctx, RQ_COLLECT_EXPR);
      const result = buildResult(collectedJson, input.context);

      if (userError !== '' && result.executionDirective?.kind !== 'skip-request') {
        dlog('run', 'user script error captured', { message: userError.slice(0, 120) });
        const errorLocation = parseScriptErrorLocation(
          userStack,
          countScriptLines(input.script),
          scriptFilename,
          userError,
        );
        throw new UserScriptError(userError, errorLocation);
      }
      if (userError !== '') {
        dlog('run', 'skipRequest() abort — surfacing clean result with directive');
      }
      const cookieMutations = cookieBridgeHandle.drainMutations();
      if (cookieMutations.length > 0) {
        // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- extending result with optional cookieMutations field (matches NodeSandbox pattern)
        (result as unknown as Record<string, unknown>)['cookieMutations'] = cookieMutations;
      }
      dlog('run', 'pushing result + ending stream', { tests: result.testResults.length });
      handle.push({ type: 'result', result });
      handle.end();
      dlog('run', 'runScript SUCCESS');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      dlog('run', 'runScript CAUGHT error', { message: message.slice(0, 160) });
      // IMPOSSIBLE-tail (ADR-010 §87): a Safe-mode `require()` of an unsupported
      // package surfaces the guided error. Lift its typed classification onto
      // `errorDetails` so the runtime carries it as `EntryError.details` and the
      // client fires the `Script Package Unsupported` analytics event. Only copied,
      // serializable data crosses (the HARD INVARIANT) — `reason`/`packageId` are
      // strings.
      //
      // The typed error can arrive two ways: (a) thrown host-side during
      // pre-bundle (e.g. a not-found id `prebundleRequires` couldn't classify) —
      // that lands in `err`; or (b) thrown by `resolveRequire` INSIDE the require
      // `newFunction`, where a host throw can't propagate — that was caught and
      // stashed in `requireImpossibleError` (the run error `err` is then the
      // generic guest exception). Prefer the stash so the precise reason survives
      // either way.
      const impossibleError = isScriptPackageUnsupportedError(requireImpossibleError)
        ? requireImpossibleError
        : isScriptPackageUnsupportedError(err)
          ? err
          : undefined;
      const errorDetails = impossibleError
        ? {
            kind: 'script_package_unsupported',
            reason: impossibleError.unsupportedReason,
            packageId: impossibleError.packageId,
          }
        : undefined;
      // A thrown user-script error carries its mapped source location (RQ-4142);
      // other failure kinds (timeout, package-unsupported, host) do not.
      const errorLocation = err instanceof UserScriptError ? err.errorLocation : undefined;
      handle.push({
        type: 'result',
        result: {
          mutationDiff: {},
          logs: [],
          testResults: [],
          error: message,
          ...(errorDetails !== undefined ? { errorDetails } : {}),
          ...(errorLocation !== undefined ? { errorLocation } : {}),
        },
      });
      handle.end();
      return;
    } finally {
      // Tear down the guest — EXCEPT a timeout-killed runtime, which we leak (its
      // deferred host-ref free would crash a later execution; see `killedByTimeout`).
      // The normal/error paths dispose cleanly (per-execution lifecycle).
      dlog('teardown', 'finally ENTER', { killedByTimeout });
      // Seal FIRST, on every exit path including the timeout kill: a live host
      // timer that outlives this execution would fire into a disposed context.
      asyncRegistry.seal();
      if (killedByTimeout) {
        // Drop the interrupt handler so the leaked runtime holds no closure over
        // `this`/`deadline`, and ABANDON the shared WASM module so the killed
        // frame's module-level deferred host-ref free cannot detonate on the next
        // execution's runtime. We deliberately do NOT dispose the killed runtime.
        try {
          runtime?.removeInterruptHandler();
        } catch {
          /* leaked runtime — nothing to recover */
        }
        resetQuickJsModule();
      } else {
        this.disposeGuest(context, runtime, installedGlobals);
      }
    }
    // Clean run (or skipRequest directive) → plain return → derived OK.
    return;
  }

  /**
   * Drive a guest promise to settlement.
   *
   * `executePendingJobs` advances the guest microtask chain (resolving the IIFE
   * promise for sync scripts); the async yield lets host promises (fetch) settle,
   * whose `.settled.then(executePendingJobs)` callback then advances the guest
   * await chain on the next pump. `resolvePromise` is NOT used — it deadlocks on
   * sync scripts (it waits for the job pump to fire, but nobody is pumping).
   *
   * RQ-5156: the loop also continues while a host bridge call is in flight or guest
   * jobs are queued, so work the script started WITHOUT `await`ing (the
   * `rq.sendRequest(url, cb)` shape) completes before the context is disposed.
   * Without this the IIFE settles immediately on such a script, the context is torn
   * down mid-flight, and the callback's `rq.*` writes vanish silently.
   *
   * Bounded by the wall-clock deadline so a genuinely-hung host call cannot block
   * forever. ONE implementation, shared by the single-script path and every
   * on-message iteration — the Equivalence obligation (runtime 021 §Decision) is a
   * property of the two paths pumping identically, not of them looking similar.
   */
  private async pumpToSettlement(
    ctx: QuickJSAsyncContext,
    runtime: ReturnType<QuickJSAsyncWASMModule['newRuntime']>,
    promise: QuickJSHandle,
    timeout: TimeoutState,
  ): Promise<void> {
    dlog('run', 'pump loop START (driving guest promise to settlement)');
    let pumpCount = 0;
    while (ctx.getPromiseState(promise).type === 'pending' || pendingAsyncCalls(ctx) > 0 || runtime.hasPendingJob()) {
      if (timeout.tripped || Date.now() > timeout.deadline) {
        dlog('run', 'pump loop: deadline/interrupt — exiting', { pumpCount });
        break;
      }
      runtime.executePendingJobs();
      await new Promise((resolve) => setTimeout(resolve, 1));
      pumpCount += 1;
    }
    dlog('run', 'pump loop DONE', { state: ctx.getPromiseState(promise).type, pumpCount });
  }

  /**
   * Run an on-message batch: one iteration per message, driven from the host
   * (ADR-208 §7, runtime 021 §Decision).
   *
   * The user script is compiled ONCE into a guest function and called per message,
   * so the batch pays one compile rather than K — which is the amortization
   * `messageBatch` exists for — while the iteration boundary stays host-side.
   * The rq shim is NOT re-evaluated per iteration: the variable scopes' in-guest
   * `working` state is what makes read-your-own-writes hold across a batch, so
   * `rq.message` is re-pointed instead, via the shim's `__rq_setMessage`.
   *
   * The four obligations, and where each is discharged:
   *
   * - **Ordering** — a single sequential loop over the batch, awaited per element.
   * - **Coverage** — exactly one iteration per element; a throw is recorded and the
   *   loop continues, so no element is skipped by another's failure.
   * - **Isolation** — the guest wrapper's own top-level try/catch per call, plus a
   *   host-side reset of the per-iteration collectors at each boundary.
   * - **Equivalence** — everything that varies between iterations is `rq.message`
   *   and the re-armed budget; `messageIndex` is stamped host-side so a batch's
   *   test results are indistinguishable from K single executions'.
   *
   * Two bounds, deliberately different:
   *
   * 1. The **per-message deadline** is re-armed at each boundary (both halves of
   *    the kill condition — wall clock and op counter). A well-behaved batch runs;
   *    a runaway iteration is still killed, because it never reaches a boundary.
   * 2. The **batch bound** is host-side and NOT re-armed: an iteration that
   *    overran its own budget without being killed — the case an un-interruptible
   *    host call produces — abandons the batch at the boundary. This replaces the
   *    withdrawn in-guest re-throw (runtime 021 §Per-message deadline AMENDMENT):
   *    an interrupt kill is not a catchable guest exception, so there was never an
   *    exception for the guest to re-throw.
   *
   * Per-iteration draining is mandatory, not an optimisation: a kill unwinds the
   * whole guest frame, so messages 1-6 survive a kill during message 7 only
   * because their slices already crossed to the host.
   */
  private async runMessageBatch(
    ctx: QuickJSAsyncContext,
    runtime: ReturnType<QuickJSAsyncWASMModule['newRuntime']>,
    input: ScriptExecutionInput,
    batch: readonly ScriptMessageInput[],
    timeout: TimeoutState,
    timeoutMs: number,
  ): Promise<BatchOutcome> {
    const scriptFilename = scriptFilenameForPhase(input.phase);
    const scriptLines = countScriptLines(input.script);
    const outcome: BatchOutcome = createBatchOutcome();

    // Compile once. The wrapper mirrors the single-script path's exactly — same
    // leading newline (so the user's line 1 is physical line 2, the uniform +1
    // offset `parseScriptErrorLocation` assumes), same catch capturing message and
    // stack onto guest globals, same trailing newline guarding a final line comment.
    const defineExpr = `globalThis.__rq_runIteration = async () => { try {\n${input.script}\n} catch (e) { globalThis.__rq_error = (e && e.constructor && e.constructor.name && e.constructor.name !== 'Error' ? e.constructor.name + ': ' : '') + ((e && e.message) ? String(e.message) : String(e)); globalThis.__rq_stack = (e && e.stack) ? String(e.stack) : ''; } };`;
    const defined = ctx.evalCode(defineExpr, scriptFilename);
    if (defined.error) {
      // A syntax error fails the whole batch — there is no per-message granularity
      // to report it at, since no message ran.
      const message = this.dumpErrorMessage(ctx, defined.error);
      const stack = this.dumpErrorStack(ctx, defined.error);
      defined.error.dispose();
      dlog('run', 'batch: script failed to compile', { msg: message.slice(0, 80) });
      throw new UserScriptError(message, parseScriptErrorLocation(stack, scriptLines, scriptFilename, message));
    }
    defined.value.dispose();

    for (const message of batch) {
      // Re-point `rq.message` for this iteration (a copied JSON string crosses).
      setStringGlobal(ctx, '__rq_message_json', JSON.stringify(message));
      this.evalOrThrow(ctx, `globalThis.__rq_setMessage(JSON.parse(globalThis.__rq_message_json));`, 'set-message');

      // Re-arm BOTH halves of the kill condition at the boundary. The wall clock is
      // the binding half; the op counter is re-armed for the same reason and is
      // belt-and-braces at realistic batch sizes (runtime 021 §Per-message deadline).
      const iterationStart = Date.now();
      timeout.deadline = iterationStart + timeoutMs;
      timeout.interruptCalls = 0;

      dlog('run', 'batch iteration START', { index: message.index });
      const call = ctx.evalCode(`globalThis.__rq_runIteration()`, scriptFilename);
      if (call.error) {
        // The wrapper swallows user throws, so an error here is the CALL failing
        // (e.g. the guest realm is gone) — record it against this message and stop
        // rather than looping into the same failure K times.
        //
        // Note what this path reports: an error, and NO `messagesCompleted` increment and
        // no `killedByTimeout`. That combination is the drain's zero-progress case, so
        // `absorb()` abandons this head rather than re-queuing it, and counts it on
        // `dropped` as well as here. The double-tally is deliberate — see the reasoning at
        // the floor in `modules/runtime/src/core/on-message/drain.ts`.
        const message_ = this.dumpErrorMessage(ctx, call.error);
        call.error.dispose();
        outcome.messageErrors.push({ messageIndex: message.index, error: message_ });
        break;
      }
      await this.pumpToSettlement(ctx, runtime, call.value, timeout);
      call.value.dispose();

      if (timeout.tripped) {
        // Killed mid-iteration. NOTHING from this iteration crossed the edge, and
        // the runtime must not be touched again (its deferred host-ref free would
        // detonate on a later execution). Report the kill against THIS message and
        // return: everything from earlier iterations is already accumulated here.
        dlog('run', 'batch iteration KILLED — abandoning batch', { index: message.index });
        outcome.messageErrors.push({ messageIndex: message.index, error: ON_MESSAGE_TIMEOUT_ERROR });
        outcome.killedByTimeout = true;
        return outcome;
      }
      runtime.executePendingJobs();

      // Drain this iteration's slice ACROSS THE EDGE before the next one starts.
      //
      // `executionDirective` and `visualizerOutput` are deliberately NOT carried:
      // a flow-control directive raised from an on-message script is rejected rather
      // than collected (ADR-208 §Design), and dropping it here is that rejection at
      // its earliest point; `rq.visualizer` is post-response-only, so the guest
      // cannot produce one in this phase at all.
      const slice = parseCollected(evalStringOut(ctx, RQ_COLLECT_EXPR));
      for (const result of slice.testResults ?? []) {
        outcome.testResults.push(stampMessageIndex(result, message.index));
      }
      outcome.requestMutations.push(...(slice.requestMutations ?? []));
      // Latest snapshot wins: the guest accumulates across iterations, so the
      // newest snapshot is the whole batch so far. Assigned only when the guest
      // produced one, so a killed iteration cannot blank the earlier accumulation.
      if (slice.mutations !== undefined) outcome.mutations = slice.mutations;

      const iterationError = evalStringOut(ctx, `globalThis.__rq_error || ''`);
      if (iterationError !== '') {
        const iterationStack = evalStringOut(ctx, `globalThis.__rq_stack || ''`);
        const errorLocation = parseScriptErrorLocation(iterationStack, scriptLines, scriptFilename, iterationError);
        outcome.messageErrors.push({
          messageIndex: message.index,
          error: iterationError,
          ...(errorLocation !== undefined ? { errorLocation } : {}),
        });
      }
      this.evalOrThrow(ctx, RQ_ITERATION_RESET_EXPR, 'iteration-reset');
      outcome.messagesCompleted += 1;

      // The batch bound (2 above). Checked here, after the slice is safely across,
      // so an abandoned batch still delivers everything it ran.
      if (Date.now() - iterationStart > timeoutMs) {
        dlog('run', 'batch: iteration overran its budget — abandoning batch', { index: message.index });
        outcome.messageErrors.push({ messageIndex: message.index, error: ON_MESSAGE_TIMEOUT_ERROR });
        break;
      }
    }
    return outcome;
  }

  /**
   * Dispose the per-execution context + runtime. Two teardown hazards this guards,
   * both confirmed empirically (see the QuickJS teardown notes):
   *
   * 1. NORMAL path — a host-fn handle still referenced by a live guest global keeps
   *    the guest heap non-empty, aborting `JS_FreeRuntime`. So we first remove the
   *    interrupt handler (after a timeout kill it is still armed and would abort the
   *    null-out eval) and null every installed host-fn global, making the host-fn
   *    objects collectable so the heap empties.
   *
   * 2. TIMEOUT-KILL path — when the interrupt terminates a script mid-frame, that
   *    dead frame pins the host-refs the shims reference; nulling globals can't
   *    release them, so `dispose()` throws a leak-check assertion at the tail of the
   *    free (a SYNCHRONOUS throw — the WASM heap is still freed). We wrap each
   *    dispose in its own catch so that abort is contained and never escapes as an
   *    unhandled rejection that would crash the worker. The run result has already
   *    been pushed by the time we get here, so swallowing a teardown-only abort is
   *    correct (it is hygiene, not a meaningful error — the heap is freed regardless).
   */
  private disposeGuest(
    context: QuickJSAsyncContext | undefined,
    runtime: ReturnType<QuickJSAsyncWASMModule['newRuntime']> | undefined,
    installedGlobals: readonly string[],
  ): void {
    dlog('teardown', 'disposeGuest START', { globals: installedGlobals.length });
    try {
      runtime?.removeInterruptHandler();
      if (context && installedGlobals.length > 0) {
        const nullExpr = installedGlobals.map((n) => `globalThis[${JSON.stringify(n)}]=undefined;`).join('') + 'true';
        const r = context.evalCode(nullExpr);
        (r.error ?? r.value).dispose();
        dlog('teardown', 'host-fn globals nulled');
      }
    } catch (e) {
      // Null-out failure is non-fatal; the dispose calls below free the heap.
      dlog('teardown', 'null-out threw (non-fatal)', { msg: e instanceof Error ? e.message.slice(0, 80) : String(e) });
    }
    // Each dispose is independently guarded: a timeout-killed frame makes dispose
    // throw a leak-check assertion AFTER freeing the heap. Containing it here keeps
    // the worker alive (the abort would otherwise surface as an unhandled rejection).
    let disposeCorrupted = false;
    try {
      context?.dispose();
      dlog('teardown', 'context disposed');
    } catch (e) {
      disposeCorrupted = true;
      dlog('teardown', 'context.dispose threw (contained)', {
        msg: e instanceof Error ? e.message.slice(0, 80) : String(e),
      });
    }
    try {
      runtime?.dispose();
      dlog('teardown', 'runtime disposed — disposeGuest DONE');
    } catch (e) {
      disposeCorrupted = true;
      dlog('teardown', 'runtime.dispose threw (contained)', {
        msg: e instanceof Error ? e.message.slice(0, 80) : String(e),
      });
    }
    if (disposeCorrupted) {
      resetQuickJsModule();
    }
  }

  /**
   * Eval an in-guest setup shim, throwing a host error (with the guest message) if
   * it fails. Shim failures are engine/setup bugs, not user-script errors, so they
   * abort the run rather than producing a silent partial realm.
   */
  private evalOrThrow(ctx: QuickJSAsyncContext, code: string, label: string): void {
    dlog('shim', 'evaluating', { label, codeLen: code.length });
    const r = ctx.evalCode(code);
    if (r.error) {
      const message = this.dumpErrorMessage(ctx, r.error);
      r.error.dispose();
      dlog('shim', 'FAILED — Safe-mode realm setup failed', { label, message: message.slice(0, 200) });
      throw new Error('Safe-mode realm setup failed', { cause: new Error(message) });
    }
    r.value.dispose();
    dlog('shim', 'ok', { label });
  }

  /** Read a guest error handle's `.message` (or a dump fallback) as a host string. */
  private dumpErrorMessage(ctx: QuickJSAsyncContext, errorHandle: QuickJSHandle): string {
    const dumped: unknown = ctx.dump(errorHandle);
    if (dumped && typeof dumped === 'object' && 'message' in dumped) {
      const m = Reflect.get(dumped as object, 'message');
      if (typeof m === 'string') return m;
    }
    return typeof dumped === 'string' ? dumped : String(dumped);
  }

  /** Read a guest error handle's `.stack` as a host string, if it has one (RQ-4142). */
  private dumpErrorStack(ctx: QuickJSAsyncContext, errorHandle: QuickJSHandle): string | undefined {
    const dumped: unknown = ctx.dump(errorHandle);
    if (dumped && typeof dumped === 'object' && 'stack' in dumped) {
      const s = Reflect.get(dumped as object, 'stack');
      if (typeof s === 'string') return s;
    }
    return undefined;
  }
}
