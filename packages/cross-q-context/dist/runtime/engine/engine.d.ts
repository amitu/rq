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
import type { SafeBridge } from './index.js';
import type { SafePackageResolver } from '../index.js';
import type { BundleCache } from './isolated/source-bundler.js';
import type { QuickJSAsyncWASMModule } from 'quickjs-emscripten-core';
import type { FeatureFlags, SandboxHostCallbacks, ScriptExecutionInput, StreamReader } from '../index.js';
import type { Sandbox, SandboxExecutionEvent, SendRequestHost } from './host-types.js';
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
/**
 * Safe-mode sandbox engine. Each `execute()` call creates and disposes a fresh
 * QuickJS runtime (per-execution lifecycle — no state leakage, matching ADR-001's
 * fresh-context-per-run semantics). Implements the unchanged `Sandbox` interface,
 * so it is a drop-in for `NodeSandbox` behind the dispatcher (ADR-008).
 */
export declare class QuickJsEngine implements Sandbox {
    private readonly resolver;
    private readonly bundleCache;
    private readonly host;
    constructor(resolver: SafePackageResolver | undefined, bundleCache: BundleCache | undefined, host: QuickJsHostConfig);
    getFeatures(): Promise<FeatureFlags>;
    execute(input: ScriptExecutionInput, hostCallbacks?: SandboxHostCallbacks): Promise<StreamReader<SandboxExecutionEvent>>;
    private runScript;
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
    private pumpToSettlement;
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
    private runMessageBatch;
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
    private disposeGuest;
    /**
     * Eval an in-guest setup shim, throwing a host error (with the guest message) if
     * it fails. Shim failures are engine/setup bugs, not user-script errors, so they
     * abort the run rather than producing a silent partial realm.
     */
    private evalOrThrow;
    /** Read a guest error handle's `.message` (or a dump fallback) as a host string. */
    private dumpErrorMessage;
    /** Read a guest error handle's `.stack` as a host string, if it has one (RQ-4142). */
    private dumpErrorStack;
}
