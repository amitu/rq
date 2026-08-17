/**
 * marshal — the copy-in/copy-out boundary between the host and the QuickJS guest
 * realm (sandbox-node ADR-010 HARD INVARIANT, ADR-012).
 *
 * Two directions, both copy-only (nothing live crosses):
 * - `dumpHandle(ctx, h)` — a guest `QuickJSHandle` → plain host `Copyable` data.
 *   Delegates to QuickJS `ctx.dump`, which recursively reads the guest value into
 *   a detached JS value. The host NEVER retains the handle.
 * - `marshalToHandle(ctx, value)` — host `Copyable` data → a fresh guest
 *   `QuickJSHandle`. Recursively builds guest primitives/arrays/objects; the
 *   caller (the factory / engine) owns and disposes the returned handle.
 *
 * Only the `safe-bridge-factory` and the engine import this — bridge authors
 * never touch a handle (the factory wraps both directions for them).
 *
 * MARSHALLING GAP (spike memo §"Marshalling gap"): QuickJS `dump` drops
 * `undefined` object properties and does not round-trip a raw `Date`. That gap is
 * irrelevant on the bridge path — bridge I/O is `Copyable` JSON-ish data + binary
 * as `number[]` — and the context copy-in / rq collection use the JSON-string
 * path (a single string crosses), not this handle marshaller. `marshalToHandle`
 * DOES emit `undefined` and serializes `Date` to an ISO string so a bridge that
 * returns either is faithful in the out-direction.
 */
import type { QuickJSAsyncContext, QuickJSHandle } from 'quickjs-emscripten-core';
/**
 * Read a guest handle out as plain host data. `ctx.dump` recursively detaches the
 * value (strings, numbers, booleans, null, arrays, plain objects) — no guest
 * reference is retained. The handle is owned by the caller (the factory disposes
 * the argument handles QuickJS hands it); this function does not dispose it.
 *
 * Returns `unknown` because the guest value is untyped at the boundary; the
 * factory's typed `BridgeHandler` signature is what constrains it to `Copyable`
 * for the bridge author. Callers parse, never cast (gr-parse-at-boundaries).
 */
export declare function dumpHandle(ctx: QuickJSAsyncContext, handle: QuickJSHandle): unknown;
/**
 * Build a fresh guest handle from host `Copyable` data. Recursive: primitives map
 * to the matching guest primitive; arrays/objects are built element-by-element
 * with each child handle disposed after it is set into its parent (QuickJS handle
 * discipline — every created handle must be disposed, and a child is owned by its
 * parent once `setProp` copies it in).
 *
 * The returned top-level handle is OWNED BY THE CALLER and must be disposed (the
 * factory returns it straight to the QuickJS function machinery, which disposes
 * it; the engine disposes the handles it builds directly).
 */
export declare function marshalToHandle(ctx: QuickJSAsyncContext, value: unknown): QuickJSHandle;
