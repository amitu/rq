// cross-q-context — the safe QuickJS EXECUTE entry (the execute pillar, slice 3).
//
// Assembles the ported isolate pieces into a working run: load QuickJS, build the guest realm
// (context + core globals + the rq.* namespace shim), eval the (already-transformed) script, and
// collect the results the guest recorded — test outcomes, raw variable mutations (inflated
// host-side into a persist-ready MutationDiff), request-header mutations, chaining directive, and
// the visualization. Console output is captured live via the console bridge.
//
// This first cut runs pre-request / post-response scripts that use rq.* variables, request/response
// access, and console. The require-chain (chai-backed rq.test/rq.expect), the SSRF-guarded fetch,
// timers, cookies, and on-message batching are wired in follow-up slices; the pieces they need are
// already ported (bridges/inflate/cookies), this entry just doesn't assemble them yet.

import asyncifyVariant from '@jitl/quickjs-singlefile-cjs-release-asyncify';
import { newQuickJSAsyncWASMModuleFromVariant } from 'quickjs-emscripten-core';

import { createConsoleBridge, CONSOLE_ISOLATE_SHIM } from './isolated/bridges/console-bridge.js';
import { PROCESS_ISOLATE_SHIM } from './isolated/bridges/process-bridge.js';
import { CORE_GLOBALS_SHIM } from './isolated/core-globals.js';
import { BUFFER_ISOLATE_SHIM } from './isolated/shims/buffer.shim.js';
import { CRYPTO_ISOLATE_SHIM } from './isolated/shims/crypto.shim.js';
import { UTIL_ISOLATE_SHIM } from './isolated/shims/util.shim.js';
import { ZLIB_ISOLATE_SHIM } from './isolated/shims/zlib.shim.js';
import { FETCH_ISOLATE_SHIM } from './isolated/shims/fetch.shim.js';
import { createFetchBridge } from './fetch-bridge.js';
import { createInMemoryCookieJarBridge } from './cookies.js';
import { createTimerBridges } from './isolated/bridges/timer-bridge.js';
import { AsyncRegistry } from './async-registry.js';
import { pendingAsyncCalls } from './isolated/safe-bridge-factory.js';
import { LogLevel } from '../contract.js';
import { RQ_ISOLATE_SHIM, RQ_COLLECT_EXPR } from './isolated/isolated-rq.js';
import { AXIOS_ISOLATE_SHIM } from './isolated/shims/axios.shim.js';
import { BRU_ISOLATE_SHIM } from './isolated/shims/bru.shim.js';
import { REQUIRE_ISOLATE_SHIM } from './isolated/shims/require.shim.js';
import { VENDOR_IIFES } from './vendor-codegen/vendor-iifes.js';
import { marshalToHandle } from './isolated/marshal.js';
import { inflateMutations } from './inflate-mutations.js';
import { SANDBOX_DEFAULT_TIMEOUT_MS } from './constants.js';
import { EXTERNAL_BUILTIN_PACKAGES } from '../definitions/builtInPackages/index.js';

import type { ScriptExecutionResult, SendRequestFn } from './host-types.js';
import type { LogEntry, RequestHeaderMutation } from '../contract.js';
import type { ScriptExecutionContext } from '../execution.js';
import type { RawScopeMutations } from '../definitions/rqMethods.js';
import type { QuickJSAsyncContext, QuickJSAsyncWASMModule } from 'quickjs-emscripten-core';

/** What one execute call needs: the (transformed) script, its phase, and the marshalled context. */
export interface ExecuteScriptInput {
  script: string;
  phase: string;
  context: ScriptExecutionContext;
  timeoutMs?: number;
  /** Host fetch backing rq.sendRequest / the guest `fetch`. Omit and those APIs are unavailable. */
  sendRequest?: SendRequestFn;
}

const RUNTIME_MEMORY_LIMIT = 128 * 1024 * 1024;

let modulePromise: Promise<QuickJSAsyncWASMModule> | undefined;
/** Compile the QuickJS WASM once; reuse the module (fresh runtime/context per execution). */
function getModule(): Promise<QuickJSAsyncWASMModule> {
  // The singlefile variant's default-export typing doesn't line up with the loader's param type,
  // though it is the exact runtime shape the loader expects (mirrors the app's NODE_QUICKJS_HOST).
  modulePromise ??= newQuickJSAsyncWASMModuleFromVariant(
    asyncifyVariant as unknown as Parameters<typeof newQuickJSAsyncWASMModuleFromVariant>[0],
  );
  return modulePromise;
}

function setStringGlobal(ctx: QuickJSAsyncContext, name: string, value: string): void {
  const handle = ctx.newString(value);
  ctx.setProp(ctx.global, name, handle);
  handle.dispose();
}

// require() resolution — the built-in tier only (chai/lodash/moment/… served from the build-time
// VENDOR_IIFES bundles keyed by registry globalName). Bridge-backed builtins (crypto) and the
// user-package SOURCE_BUNDLE tier are follow-ups; unknown ids resolve to `impossible`.
const GLOBAL_NAME_BY_ID: ReadonlyMap<string, string> = new Map(
  EXTERNAL_BUILTIN_PACKAGES.map((p) => [p.id, p.globalName]),
);
type RequireResolution =
  | { kind: 'iife'; code: string; globalName: string }
  | { kind: 'impossible'; id: string };
function resolveRequireId(id: string): RequireResolution {
  const code = (VENDOR_IIFES as Record<string, string | undefined>)[id];
  const globalName = GLOBAL_NAME_BY_ID.get(id) ?? (id === 'events' ? '__events' : undefined);
  if (code && globalName) return { kind: 'iife', code, globalName };
  return { kind: 'impossible', id };
}

/** Eval guest source; throw with the guest error message on failure (assembly steps must succeed). */
function evalOrThrow(ctx: QuickJSAsyncContext, code: string, label: string): void {
  const r = ctx.evalCode(code);
  if (r.error) {
    const err = ctx.dump(r.error);
    r.error.dispose();
    throw new Error(`cross-q-context executor: ${label} failed: ${JSON.stringify(err)}`);
  }
  r.value.dispose();
}

/** Eval an expression expected to return a string (the collect drain); '{}' on any error. */
function evalStringOut(ctx: QuickJSAsyncContext, expr: string): string {
  const r = ctx.evalCode(expr);
  if (r.error) {
    r.error.dispose();
    return '{}';
  }
  const dumped: unknown = ctx.dump(r.value);
  r.value.dispose();
  return typeof dumped === 'string' ? dumped : '{}';
}

interface CollectedFromIsolate {
  testResults?: ScriptExecutionResult['testResults'];
  mutations?: RawScopeMutations;
  requestMutations?: readonly RequestHeaderMutation[];
  executionDirective?: ScriptExecutionResult['executionDirective'];
  visualizerOutput?: ScriptExecutionResult['visualizerOutput'];
}

/**
 * Run a (transformed) rq.* script safely in QuickJS and return its result. Self-contained: the OSS
 * caller supplies the script + a ScriptExecutionContext and gets back mutations / tests / logs.
 */
export async function executeScript(input: ExecuteScriptInput): Promise<ScriptExecutionResult> {
  const QuickJS = await getModule();
  const runtime = QuickJS.newRuntime();
  runtime.setMemoryLimit(RUNTIME_MEMORY_LIMIT);
  const ctx = runtime.newContext();
  const logs: LogEntry[] = [];
  // Host-function globals we install — nulled before dispose so QuickJS can free their HostRefs
  // (skipping this leaves a dangling ref that throws at runtime teardown).
  const installedGlobals: string[] = [];

  // Wall-clock + op-count interrupt so a runaway script can't hang the host.
  const timeoutMs = input.timeoutMs ?? SANDBOX_DEFAULT_TIMEOUT_MS;
  const deadline = Date.now() + timeoutMs;
  let opCount = 0;
  runtime.setInterruptHandler(() => (opCount += 1) > 1_000_000 || Date.now() > deadline);

  // Per-execution async registry backing the guest timers (setTimeout/clearTimeout, and the
  // callback dispatch that rq.sendRequest uses). Sealed on every exit so no host timer outlives
  // the isolate.
  const timerError = (message: string): void => {
    logs.push({ level: LogLevel.error, args: [`Uncaught error in timer callback: ${message}`], timestamp: Date.now() });
  };
  const asyncRegistry = new AsyncRegistry<ReturnType<typeof setTimeout>>({
    timers: {
      scheduleTimer: (fn, ms) => setTimeout(fn, ms),
      cancelTimer: (handle) => clearTimeout(handle),
    },
    onCallbackError: (error) => timerError(error instanceof Error ? error.message : String(error)),
  });

  try {
    ctx.setProp(ctx.global, 'global', ctx.global);

    const consoleBridge = createConsoleBridge((entry) => logs.push(entry), () => Date.now());
    consoleBridge.install(ctx);
    installedGlobals.push(consoleBridge.name);

    // The require dispatch callback the REQUIRE_ISOLATE_SHIM calls: id → bundle resolution.
    const requireFn = ctx.newFunction('__rq_bundleRequire', (idHandle) => {
      const id = String(ctx.dump(idHandle));
      const res = resolveRequireId(id);
      if (res.kind === 'impossible') {
        return { error: ctx.newString(`require('${id}') is not available in the safe sandbox`) };
      }
      return marshalToHandle(ctx, res);
    });
    ctx.setProp(ctx.global, '__rq_bundleRequire', requireFn);
    requireFn.dispose();
    installedGlobals.push('__rq_bundleRequire');

    // Timers — the guest setTimeout/clearTimeout (also how rq.sendRequest dispatches its callback).
    for (const bridge of createTimerBridges(asyncRegistry, timerError)) {
      bridge.install(ctx);
      installedGlobals.push(bridge.name);
    }

    // Delegated fetch — only when the host supplies a sendRequest backend. Adapt the simple
    // callback into the envelope-returning SendRequestHost the full bridge expects.
    if (input.sendRequest) {
      const fn = input.sendRequest;
      const fetchBridge = createFetchBridge({
        sendRequest: async (req) => {
          try {
            return { ok: true, response: await fn(req) };
          } catch (e) {
            return { ok: false, error: { kind: 'network', message: e instanceof Error ? e.message : String(e) } };
          }
        },
      });
      fetchBridge.install(ctx);
      installedGlobals.push(fetchBridge.name);
    }

    // Cookie jar (ADR-105). Sync host callback: the guest calls __rq_cookies({op, host, …}) and
    // gets copied data; the in-memory jar accumulates mutations, drained after the script. Access
    // is gated to the host allowlist (rq.cookies.jar(host) throws otherwise). The execution
    // context's seed shape matches the jar's structurally (Json[] vs ScriptCookieSnapshot[]).
    const cookieJar = createInMemoryCookieJarBridge(
      input.context.cookieJarSeed as unknown as Parameters<typeof createInMemoryCookieJarBridge>[0],
    );
    const cookieAllowlist = new Set((input.context.hostAllowlist ?? []).map((h) => h.toLowerCase()));
    const cookieFn = ctx.newFunction('__rq_cookies', (argsHandle) => {
      const raw: unknown = ctx.dump(argsHandle);
      if (!raw || typeof raw !== 'object') return marshalToHandle(ctx, { error: 'invalid cookie args' });
      const args = raw as Record<string, unknown>;
      const op = typeof args['op'] === 'string' ? args['op'] : '';
      const host = typeof args['host'] === 'string' ? args['host'] : '';
      if (!host || !cookieAllowlist.has(host.toLowerCase())) {
        return marshalToHandle(ctx, { error: `CookieStore: programmatic access to "${host}" is denied.` });
      }
      if (op === 'list') return marshalToHandle(ctx, { result: cookieJar.bridge.list(host) });
      if (op === 'upsert') {
        cookieJar.bridge.upsert(host, args['cookie'] as Parameters<typeof cookieJar.bridge.upsert>[1]);
        return marshalToHandle(ctx, { result: args['cookie'] });
      }
      if (op === 'remove') {
        const name = typeof args['name'] === 'string' ? args['name'] : '';
        const path = typeof args['path'] === 'string' ? args['path'] : '/';
        cookieJar.bridge.remove(host, name, path);
        return marshalToHandle(ctx, { result: null });
      }
      if (op === 'clear') {
        cookieJar.bridge.clear(host);
        return marshalToHandle(ctx, { result: null });
      }
      return marshalToHandle(ctx, { error: 'unknown cookie op' });
    });
    ctx.setProp(ctx.global, '__rq_cookies', cookieFn);
    cookieFn.dispose();
    installedGlobals.push('__rq_cookies');

    // Copy the context + phase + cookie-jar allowlist in as strings (nothing live crosses).
    setStringGlobal(ctx, '__rq_context_json', JSON.stringify(input.context));
    setStringGlobal(ctx, '__rq_phase', input.phase);
    setStringGlobal(ctx, '__rq_hostAllowlist_json', JSON.stringify(input.context.hostAllowlist ?? []));

    // Build the guest realm: parse the context, install core globals, then the rq.* namespace.
    evalOrThrow(ctx, 'globalThis.__rq_context = JSON.parse(globalThis.__rq_context_json);', 'context-parse');
    evalOrThrow(ctx, CORE_GLOBALS_SHIM, 'core-globals');
    // Capability shims — define console + the Node builtins (Buffer/crypto/util/zlib) as guest
    // globals. Each is lazy over its host bridge; only `console` is bridge-backed in this cut, so a
    // script that actually calls Buffer/crypto without the (unwired) backing bridge will throw.
    evalOrThrow(ctx, CONSOLE_ISOLATE_SHIM, 'console-shim');
    evalOrThrow(ctx, PROCESS_ISOLATE_SHIM, 'process-shim');
    evalOrThrow(ctx, BUFFER_ISOLATE_SHIM, 'buffer-shim');
    evalOrThrow(ctx, CRYPTO_ISOLATE_SHIM, 'crypto-shim');
    evalOrThrow(ctx, UTIL_ISOLATE_SHIM, 'util-shim');
    evalOrThrow(ctx, ZLIB_ISOLATE_SHIM, 'zlib-shim');
    if (input.sendRequest) evalOrThrow(ctx, FETCH_ISOLATE_SHIM, 'fetch-shim');
    // The require chain, then load chai (the real thing, from VENDOR_IIFES) so rq.test/rq.expect
    // assert for real, then build the rq.* namespace over it.
    evalOrThrow(ctx, REQUIRE_ISOLATE_SHIM, 'require-chain');
    evalOrThrow(ctx, 'globalThis.__rq_chai = globalThis.require("chai");', 'chai-load');
    evalOrThrow(ctx, RQ_ISOLATE_SHIM, 'rq-namespace');
    // Bruno's globals over the rq namespace, so a `.bru` script runs unmodified. Last,
    // because it maps onto `globalThis.rq`.
    evalOrThrow(ctx, BRU_ISOLATE_SHIM, 'bru-compat');
    evalOrThrow(ctx, AXIOS_ISOLATE_SHIM, 'axios-facade');

    // Eval the user script wrapped in an async IIFE with a top-level catch that records the error.
    const wrapped = `(async () => { try {\n${input.script}\n} catch (e) { globalThis.__rq_error = (e && e.message) ? String(e.message) : String(e); globalThis.__rq_stack = (e && e.stack) ? String(e.stack) : ''; } })()`;
    const evalResult = ctx.evalCode(wrapped, `script-${input.phase}.js`);
    if (evalResult.error) {
      const err = ctx.dump(evalResult.error);
      evalResult.error.dispose();
      const message = err && typeof err === 'object' && 'message' in err ? String((err as { message: unknown }).message) : String(err);
      return { mutationDiff: {}, logs, testResults: [], error: message };
    }
    // Drive the async IIFE to settlement: advance the guest microtask chain while its promise is
    // pending, host async calls (fetch) are in flight, or jobs remain — yielding to the host event
    // loop each turn so a delegated fetch can resolve. Bounded by the same deadline as the script.
    const promise = evalResult.value;
    let pumped = 0;
    while (
      ctx.getPromiseState(promise).type === 'pending' ||
      pendingAsyncCalls(ctx) > 0 ||
      runtime.hasPendingJob()
    ) {
      if (opCount > 1_000_000 || Date.now() > deadline) break;
      runtime.executePendingJobs();
      await new Promise((resolve) => setTimeout(resolve, 1));
      pumped += 1;
      if (pumped > 100_000) break;
    }
    evalResult.value.dispose();

    // Any error the guest's top-level catch recorded.
    const userError = evalStringOut(ctx, "globalThis.__rq_error || ''");

    // Drain everything the guest recorded in one JSON string.
    const collected = JSON.parse(evalStringOut(ctx, RQ_COLLECT_EXPR)) as CollectedFromIsolate;
    const mutationDiff = collected.mutations ? inflateMutations(collected.mutations, input.context) : {};

    const result: ScriptExecutionResult = {
      mutationDiff,
      logs,
      testResults: collected.testResults ?? [],
    };
    if (collected.requestMutations && collected.requestMutations.length > 0) {
      result.requestMutationDiff = { headers: collected.requestMutations };
    }
    if (collected.executionDirective) result.executionDirective = collected.executionDirective;
    if (collected.visualizerOutput) result.visualizerOutput = collected.visualizerOutput;
    const cookieMutations = cookieJar.drainMutations();
    if (cookieMutations.length > 0) result.cookieMutations = cookieMutations;
    if (userError && result.executionDirective?.kind !== 'skip-request') result.error = userError;
    return result;
  } finally {
    // Teardown (mirrors the app's disposeGuest): drop the interrupt handler, null the installed
    // host-function globals so their HostRefs are freed, then dispose context before runtime.
    // Each step guarded — a teardown throw must not mask the result.
    try {
      asyncRegistry.seal();
      runtime.removeInterruptHandler();
      if (installedGlobals.length > 0) {
        const nullExpr = installedGlobals.map((n) => `globalThis[${JSON.stringify(n)}]=undefined;`).join('') + 'true';
        const r = ctx.evalCode(nullExpr);
        (r.error ?? r.value).dispose();
      }
    } catch {
      /* non-fatal — the disposes below free the heap */
    }
    try {
      ctx.dispose();
    } catch {
      /* contained */
    }
    try {
      runtime.dispose();
    } catch {
      /* contained */
    }
  }
}
