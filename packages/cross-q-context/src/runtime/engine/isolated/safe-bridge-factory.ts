/**
 * safe-bridge-factory — the SOLE construction path for every Safe-mode
 * (QuickJS-WASM) capability bridge (sandbox-node ADR-010, amended by ADR-012; TB
 * engineering item E-3).
 *
 * THE HARD INVARIANT (ADR-010 §16, re-expressed for QuickJS in ADR-012): every
 * Safe-mode bridge passes COPIED DATA both ways and never hands the guest realm a
 * live host reference. Under QuickJS the boundary is `QuickJSHandle`s: a host
 * function is installed via `ctx.newFunction` (sync) / `ctx.newAsyncifiedFunction`
 * (async), its arguments arrive as guest handles that the factory `dump`s to plain
 * data, and its return value is marshalled back to a fresh guest handle. The host
 * handler itself only ever sees `Copyable` data and returns `Copyable` data — it
 * never receives or returns a `QuickJSHandle`. (The ivm-era `ivm.Reference` /
 * `ivm.ExternalCopy` / sync-`ivm.Callback` forms are gone — see ADR-012.)
 *
 * This factory makes that invariant a COMPILE ERROR, not a runtime hope
 * (`gr-illegal-states-unrepresentable` / ADR-009): a bridge handler is typed to
 * accept and return only `Copyable` values — JSON-ish data, Dates, and binary as
 * `number[]`. A handler that takes or returns a function, a `QuickJSHandle`, or any
 * non-cloneable value does not type-check, so the leak-a-live-reference mistake
 * cannot be authored here. The handle-marshalling (`dump` in, `marshalToHandle`
 * out) lives entirely inside the factory; a bridge author never touches a handle.
 *
 * The author-time guarantee (this factory + the `no-bridge-outside-factory`
 * lint rule) composes with the runtime guarantee (ADR-011's adversarial escape
 * suite + the per-bridge containment tests). The factory prevents an unsafe
 * bridge from being written; the suite catches an escape at the boundary if one
 * ever slips through another path. See ADR-010 §"Two layers of enforcement".
 *
 * CLI-bundle safety: this module imports only the engine-neutral marshalling
 * helpers (no native addon). It lives under `src/isolated/` and is reached via the
 * `@requestly/sandbox-node/isolated` subpath; it is NEVER re-exported from the `.`
 * barrel (`src/index.ts`). (The native-addon quarantine relaxes to a hygiene
 * boundary under ADR-012 — WASM has no link step — but the subpath discipline
 * stays for clarity.)
 */

import { dlog } from './debug-log.js';
import { dumpHandle, marshalToHandle } from './marshal.js';

import type { QuickJSAsyncContext, QuickJSHandle } from 'quickjs-emscripten-core';

/**
 * The shapes structured clone (and therefore the realm boundary) can carry.
 * Deliberately EXCLUDES functions, symbols, and `QuickJSHandle` — those are the
 * live-reference forms the HARD INVARIANT forbids. A bridge handler whose
 * arguments or return value are not assignable to `Copyable` is a type error.
 *
 * `bigint` is intentionally absent: our JSON result/log path does not round-trip
 * it, so bridges must not depend on it crossing the edge (containment tests assert
 * this, ADR-010 §Step 10).
 *
 * BINARY DATA crosses as a real `Uint8Array`/`ArrayBuffer` (ADR-012 follow-up).
 * The marshaller (`marshal.ts`) carries it via QuickJS `newArrayBuffer` (out) /
 * `getArrayBuffer` (in) — a byte copy, never a live reference, so the HARD
 * INVARIANT holds. (This replaced the original `number[]` model, which was forced
 * by an isolated-vm × Electron TypedArray process-abort — RQ-3359 — that does not
 * exist under QuickJS-WASM. The in-realm shims pass `.buffer` out and read
 * `new Uint8Array(arrayBuffer)` in.) `QuickJSHandle`, functions, and symbols stay
 * OUT of `Copyable` — those are the live-reference forms a bridge must never cross.
 */
export type Copyable =
  | string
  | number
  | boolean
  | null
  | undefined
  | Date
  | Uint8Array
  | ArrayBuffer
  | readonly Copyable[]
  | { readonly [key: string]: Copyable };

/**
 * A bridge handler: a pure host-side function whose inputs and output are both
 * `Copyable`. The factory `dump`s the guest arguments to `Copyable` data, calls
 * the handler, and marshals the `Copyable` result back to a guest handle; the
 * handler never receives or returns a live guest/host reference.
 */
export type BridgeHandler<Args extends readonly Copyable[], Result extends Copyable> = (...args: Args) => Result;

/**
 * An async bridge handler — same copy-in/copy-out contract, returning a promise
 * of a `Copyable` (e.g. the `fetch` bridge). The factory installs it via
 * `ctx.newAsyncifiedFunction`, which suspends/resumes the WASM stack so the guest
 * sees the resolved copied value returned directly (no guest-promise plumbing).
 */
export type AsyncBridgeHandler<Args extends readonly Copyable[], Result extends Copyable> = (
  ...args: Args
) => Promise<Result>;

/**
 * A bridge token the engine installs into the guest realm. Engine-neutral: it
 * carries the global `name` and an `install(ctx)` closure that is the ONLY place a
 * `ctx.newFunction`/`newAsyncifiedFunction` is constructed for a capability bridge
 * (the `no-bridge-outside-factory` lint rule enforces this). `install` defines
 * `globalThis[name]` in the guest and disposes the function handle it created
 * (QuickJS requires every handle be disposed). The brand prevents an arbitrary
 * object from being passed where a `SafeBridge` is expected.
 */
export interface SafeBridge {
  readonly __safeBridge: true;
  /** The global name the bridge's user-facing accessor is installed under. */
  readonly name: string;
  /**
   * Installs the bridge into the guest realm: creates the host-backed function
   * via the factory's marshalling wrapper, sets it as `globalThis[name]`, and
   * disposes the created handle. Called once per execution by the engine.
   */
  readonly install: (ctx: QuickJSAsyncContext) => void;
}

/**
 * Set `globalThis[name] = fnHandle` in the guest, then dispose the handle. The
 * guest keeps its own reference once `setProp` copies it into the global object,
 * so the host handle is free to dispose (QuickJS handle discipline).
 */
function defineGlobal(ctx: QuickJSAsyncContext, name: string, fnHandle: QuickJSHandle): void {
  ctx.setProp(ctx.global, name, fnHandle);
  fnHandle.dispose();
}

/**
 * THE ONE BOUNDARY COERCION (ADR-010 HARD INVARIANT, ADR-012).
 *
 * The guest is untyped: `dumpHandle` yields `unknown[]`. A bridge handler is typed
 * to take `Copyable` args — at RUNTIME the dumped values ARE copyable JSON-ish data
 * (`dump` cannot produce a function/symbol/live reference), but TypeScript cannot
 * prove `unknown[]` satisfies the handler's specific `Args` (parameter position is
 * contravariant). This is the "truly unavoidable library boundary" `gr-no-unsafe-cast`
 * sanctions with a disable + ADR reference — and it is the SINGLE place a handler is
 * invoked with dumped args, so the whole factory's type-safety reduces to this one
 * audited call. `marshalToHandle` takes `unknown` and re-parses every branch, so the
 * RESULT direction needs no coercion.
 */
function invokeWithDumpedArgs(handler: (...args: never[]) => unknown, args: readonly unknown[]): unknown {
  // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion -- ADR-010/ADR-012: the sole guest→handler boundary; dumped guest values are Copyable at runtime (dump cannot emit a live reference), unprovable to TS.
  return handler(...(args as never[]));
}

/** Install a sync host function: dump guest arg handles, invoke the handler, marshal back. */
function installSync(ctx: QuickJSAsyncContext, name: string, handler: (...args: never[]) => unknown): void {
  const fn = ctx.newFunction(name, (...argHandles: QuickJSHandle[]) => {
    dlog('bridge', 'sync call', { name, argc: argHandles.length });
    return marshalToHandle(
      ctx,
      invokeWithDumpedArgs(
        handler,
        argHandles.map((h) => dumpHandle(ctx, h)),
      ),
    );
  });
  defineGlobal(ctx, name, fn);
}

/**
 * In-flight async-bridge counter, per guest context (RQ-5156).
 *
 * A script that starts a host call without `await`ing it — the pasted-Postman
 * `rq.sendRequest(url, cb)` shape, 97% of all `sendRequest` usage in the prod
 * corpus — leaves that call in flight when the top-level IIFE promise settles.
 * The engine's pump loop consults this count so it does not dispose the context
 * while a host call is still running and the guest callback has never been invoked
 * (the callback's `rq.*` writes would otherwise be discarded silently).
 *
 * Keyed by context so the state is PER EXECUTION: each `QuickJsSandbox` run owns
 * its own runtime/context, and concurrent executions in one worker must never
 * observe each other's count.
 */
const inFlightByContext = new WeakMap<QuickJSAsyncContext, { n: number }>();

function trackAsyncStart(ctx: QuickJSAsyncContext): void {
  const slot = inFlightByContext.get(ctx);
  if (slot === undefined) inFlightByContext.set(ctx, { n: 1 });
  else slot.n += 1;
}

function trackAsyncEnd(ctx: QuickJSAsyncContext): void {
  const slot = inFlightByContext.get(ctx);
  if (slot !== undefined && slot.n > 0) slot.n -= 1;
}

/**
 * How many async bridge calls are still in flight for this context. Read by the
 * engine's pump loop (`quickjs-sandbox.ts`) as one of its continue conditions.
 */
export function pendingAsyncCalls(ctx: QuickJSAsyncContext): number {
  return inFlightByContext.get(ctx)?.n ?? 0;
}

/**
 * Is it still safe to settle a guest promise from the host? (RQ-5156)
 *
 * A host call can outlive the execution: the engine seals the result and disposes
 * the context while the call is in flight (the drain gives up at the deadline).
 * Touching `ctx` afterwards throws `QuickJSUseAfterFree: Lifetime not alive` from
 * inside this fire-and-forget IIFE, which surfaces as a process-level unhandled
 * rejection — swallowed by the desktop worker's guard, but a real use-after-free
 * attempt and pure noise. Dropping the settlement is correct: the guest realm that
 * would have observed it no longer exists.
 */
function isSettleTargetAlive(ctx: QuickJSAsyncContext, promise: { readonly alive: boolean }): boolean {
  return ctx.alive && promise.alive;
}

/**
 * Install an async host function via the GUEST-PROMISE pattern (not asyncified).
 *
 * `newAsyncifiedFunction` suspends the WASM stack, which `evalCodeAsync` / the job
 * queue can't track — the run loop tears down the runtime while the host call is
 * still in flight (RQ-3359, confirmed on desktop). The guest-promise pattern (the
 * Bruno-validated approach) avoids this entirely: the bridge returns a GUEST promise
 * that the script `await`s normally, the host resolves it when the async op settles,
 * and `executePendingJobs` pumps the guest `await` chain forward. No WASM suspend,
 * no teardown race, and `resolvePromise` at the top level drives the whole chain.
 *
 * The handler is typed to return `unknown` so the async-arm union from the public
 * overload is accepted without a cast.
 */
function installAsync(ctx: QuickJSAsyncContext, name: string, handler: (...args: never[]) => unknown): void {
  const fn = ctx.newFunction(name, (...argHandles: QuickJSHandle[]) => {
    dlog('bridge', 'async call ENTER (guest-promise pattern)', { name, argc: argHandles.length });
    const args = argHandles.map((h) => dumpHandle(ctx, h));
    const promise = ctx.newPromise();
    // Counted for the whole life of the host call (RQ-5156) so the engine's pump
    // loop cannot tear the context down mid-flight.
    trackAsyncStart(ctx);
    void (async () => {
      try {
        try {
          const result = await invokeWithDumpedArgs(handler, args);
          if (!isSettleTargetAlive(ctx, promise)) {
            dlog('bridge', 'async call RESOLVED after teardown → dropping', { name });
            return;
          }
          dlog('bridge', 'async call RESOLVED → resolving guest promise', { name });
          const handle = marshalToHandle(ctx, result);
          promise.resolve(handle);
          handle.dispose();
        } catch (e) {
          if (!isSettleTargetAlive(ctx, promise)) {
            dlog('bridge', 'async call THREW after teardown → dropping', { name });
            return;
          }
          dlog('bridge', 'async call THREW → rejecting guest promise', {
            name,
            msg: e instanceof Error ? e.message.slice(0, 60) : String(e),
          });
          const errHandle = ctx.newError(e instanceof Error ? e.message : String(e));
          promise.reject(errHandle);
          errHandle.dispose();
        }
        void promise.settled.then(() => ctx.runtime.executePendingJobs());
      } finally {
        // `finally`, not a tail statement: if the reject path itself throws, an
        // un-decremented counter would stall the pump loop until the deadline.
        //
        // Decrementing here is safe even though the guest callback has NOT run
        // yet: settling the guest promise queues guest reaction jobs, so the pump
        // loop keeps iterating on `hasPendingJob()`. That is what lets the callback
        // — and any further bridge call it starts, which re-increments this counter
        // — complete before teardown.
        trackAsyncEnd(ctx);
      }
    })();
    return promise.handle;
  });
  defineGlobal(ctx, name, fn);
}

/** Install a fire-and-forget host function (no return crosses back). */
function installIgnored(ctx: QuickJSAsyncContext, name: string, handler: (...args: never[]) => unknown): void {
  const fn = ctx.newFunction(name, (...argHandles: QuickJSHandle[]) => {
    dlog('bridge', 'ignored call', { name, argc: argHandles.length });
    invokeWithDumpedArgs(
      handler,
      argHandles.map((h) => dumpHandle(ctx, h)),
    );
    // No return → guest sees `undefined`; no handle to dispose.
  });
  defineGlobal(ctx, name, fn);
}

/**
 * Construct a value-returning bridge.
 *
 * `handler` is type-constrained to copy-in/copy-out: TypeScript rejects a handler
 * that takes or returns anything outside `Copyable`. The factory `dump`s each
 * guest argument handle to `Copyable` data, calls the handler, and marshals the
 * `Copyable` result back to a fresh guest handle — so only data crosses the edge.
 *
 * Sync handlers install via `ctx.newFunction`. Async handlers (`{ async: true }`,
 * e.g. fetch) install via `ctx.newAsyncifiedFunction`: the asyncify variant
 * suspends the WASM stack while the host promise settles and resumes with the
 * copied resolved value returned directly to the guest (the Spike-A pattern —
 * no guest `newPromise`/`executePendingJobs` dance).
 *
 * @param name   global name the bridge installs under inside the guest realm
 * @param handler pure host function; arguments and return are `Copyable`
 * @param opts   `{ async: true }` for promise-returning handlers (e.g. fetch)
 */
export function createSafeBridge<Args extends readonly Copyable[], Result extends Copyable>(
  name: string,
  handler: BridgeHandler<Args, Result>,
): SafeBridge;
export function createSafeBridge<Args extends readonly Copyable[], Result extends Copyable>(
  name: string,
  handler: AsyncBridgeHandler<Args, Result>,
  opts: { async: true },
): SafeBridge;
export function createSafeBridge<Args extends readonly Copyable[], Result extends Copyable>(
  name: string,
  handler: BridgeHandler<Args, Result> | AsyncBridgeHandler<Args, Result>,
  opts?: { async: true },
): SafeBridge {
  // The async arm installs via `ctx.newAsyncifiedFunction` (the asyncify variant
  // suspends/resumes the WASM stack so the guest sees the resolved copied value
  // directly); the sync arm via `ctx.newFunction`. The typed handler is accepted by
  // the `(...args: never[])` install signature without a cast; the single audited
  // arg coercion lives in `invokeWithDumpedArgs`. Only copied data crosses.
  if (opts?.async) {
    return { __safeBridge: true, name, install: (ctx) => installAsync(ctx, name, handler) };
  }
  return { __safeBridge: true, name, install: (ctx) => installSync(ctx, name, handler) };
}

/**
 * Construct a fire-and-forget bridge: the guest calls it, nothing (not even an
 * exception) crosses back. Used by the `console` bridge — the guest serializes
 * its args to a copied string and the host consumes them with no return path.
 *
 * The handler returns `void` and takes only `Copyable` arguments, so — as with
 * `createSafeBridge` — no live reference can be authored across the edge. The
 * guest call evaluates to `undefined`.
 */
export function createIgnoredBridge<Args extends readonly Copyable[]>(
  name: string,
  handler: (...args: Args) => void,
): SafeBridge {
  // The typed handler is accepted by the `(...args: never[])` install signature
  // without a cast; the single audited arg coercion lives in `invokeWithDumpedArgs`.
  return { __safeBridge: true, name, install: (ctx) => installIgnored(ctx, name, handler) };
}
