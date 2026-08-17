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
import type { QuickJSAsyncContext } from 'quickjs-emscripten-core';
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
export type Copyable = string | number | boolean | null | undefined | Date | Uint8Array | ArrayBuffer | readonly Copyable[] | {
    readonly [key: string]: Copyable;
};
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
export type AsyncBridgeHandler<Args extends readonly Copyable[], Result extends Copyable> = (...args: Args) => Promise<Result>;
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
 * How many async bridge calls are still in flight for this context. Read by the
 * engine's pump loop (`quickjs-sandbox.ts`) as one of its continue conditions.
 */
export declare function pendingAsyncCalls(ctx: QuickJSAsyncContext): number;
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
export declare function createSafeBridge<Args extends readonly Copyable[], Result extends Copyable>(name: string, handler: BridgeHandler<Args, Result>): SafeBridge;
export declare function createSafeBridge<Args extends readonly Copyable[], Result extends Copyable>(name: string, handler: AsyncBridgeHandler<Args, Result>, opts: {
    async: true;
}): SafeBridge;
/**
 * Construct a fire-and-forget bridge: the guest calls it, nothing (not even an
 * exception) crosses back. Used by the `console` bridge — the guest serializes
 * its args to a copied string and the host consumes them with no return path.
 *
 * The handler returns `void` and takes only `Copyable` arguments, so — as with
 * `createSafeBridge` — no live reference can be authored across the edge. The
 * guest call evaluates to `undefined`.
 */
export declare function createIgnoredBridge<Args extends readonly Copyable[]>(name: string, handler: (...args: Args) => void): SafeBridge;
