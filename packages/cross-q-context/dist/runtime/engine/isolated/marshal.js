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
import { dlog } from './debug-log.js';
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
export function dumpHandle(ctx, handle) {
    // A guest `ArrayBuffer` — possibly NESTED inside an object/array bridge arg, e.g.
    // `{ op: 'hash', data: <ArrayBuffer> }` — must be read out as a host `Uint8Array`
    // via `getArrayBuffer`. `ctx.dump` flattens an ArrayBuffer to a `{0:..,1:..}`
    // object (losing the binary shape) AND recurses into containers, so we CANNOT
    // dump a container whole if it might hold a buffer. We therefore recurse
    // ourselves: ArrayBuffer → Uint8Array (a byte copy, no live ref); array/object →
    // walk member-by-member; primitives → `ctx.dump` (cheap, no buffers possible).
    // `getArrayBuffer` throws on a non-ArrayBuffer handle, so detection is gated by
    // `constructor.name === 'ArrayBuffer'`.
    if (isArrayBufferHandle(ctx, handle)) {
        const ab = ctx.getArrayBuffer(handle);
        // `.slice()` makes a COPY (not a view) — `ab.value` is a host view over WASM
        // heap memory that `ab.dispose()` invalidates; copying out keeps the HARD
        // INVARIANT (no live WASM-backed reference escapes to the handler).
        const bytes = new Uint8Array(ab.value).slice();
        ab.dispose();
        dlog('marshal', 'dump: guest ArrayBuffer → host Uint8Array', { len: bytes.length });
        return bytes;
    }
    if (ctx.typeof(handle) === 'object') {
        // We `ctx.dump` the container once to read its KEYS + array/object shape, then
        // re-read each member via `getProp` + recursive `dumpHandle` so a nested
        // ArrayBuffer is caught. (The shallow dump also flattens any nested buffer into
        // a throwaway `{0:..}` — wasted work proportional to nested-buffer size, but
        // bridge args are small: crypto keys/data + buffers ≤ 64 KB. A
        // getOwnPropertyNames-only key read would avoid it but the 0.32 API returns a
        // non-trivial wrapper; not worth the complexity for these payload sizes.)
        const dumpedShallow = ctx.dump(handle);
        if (dumpedShallow === null)
            return null;
        if (Array.isArray(dumpedShallow)) {
            return dumpedShallow.map((_, i) => {
                const child = ctx.getProp(handle, i);
                try {
                    return dumpHandle(ctx, child);
                }
                finally {
                    child.dispose();
                }
            });
        }
        if (typeof dumpedShallow === 'object') {
            const out = {};
            for (const key of Object.keys(dumpedShallow)) {
                const child = ctx.getProp(handle, key);
                try {
                    out[key] = dumpHandle(ctx, child);
                }
                finally {
                    child.dispose();
                }
            }
            return out;
        }
        return dumpedShallow;
    }
    return ctx.dump(handle);
}
/** True iff the guest handle is an `ArrayBuffer` (so `getArrayBuffer` is safe). */
function isArrayBufferHandle(ctx, handle) {
    if (ctx.typeof(handle) !== 'object')
        return false;
    const ctorHandle = ctx.getProp(handle, 'constructor');
    try {
        if (ctx.typeof(ctorHandle) === 'undefined')
            return false;
        const nameHandle = ctx.getProp(ctorHandle, 'name');
        try {
            return ctx.typeof(nameHandle) === 'string' && ctx.getString(nameHandle) === 'ArrayBuffer';
        }
        finally {
            nameHandle.dispose();
        }
    }
    finally {
        ctorHandle.dispose();
    }
}
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
export function marshalToHandle(ctx, value) {
    // Accepts `unknown`: the factory hands us a handler's return value, which is
    // `Copyable` by the public contract but `unknown` at this internal boundary
    // (the guest is untyped). Every branch below runtime-checks the shape, so this
    // PARSES the value rather than trusting a downcast (gr-parse-at-boundaries /
    // gr-no-unsafe-cast). A non-Copyable value (function, symbol) falls through to
    // the plain-object branch and marshals as an empty object — it cannot smuggle a
    // live reference across (the HARD INVARIANT holds structurally).
    if (value === undefined)
        return ctx.undefined;
    if (value === null)
        return ctx.null;
    switch (typeof value) {
        case 'string':
            return ctx.newString(value);
        case 'number':
            return ctx.newNumber(value);
        case 'boolean':
            return value ? ctx.true : ctx.false;
        case 'bigint':
            // bigint does not round-trip our JSON path; stringify defensively.
            return ctx.newString(value.toString());
        case 'function':
        case 'symbol':
            // Non-Copyable — never cross a live reference. Marshal as undefined.
            return ctx.undefined;
        default:
            break;
    }
    if (value instanceof Date) {
        // Date does not round-trip through `dump`; serialize to ISO so the out-bound
        // value is faithful and JSON-shaped (a bridge consumer reconstructs if needed).
        return ctx.newString(value.toISOString());
    }
    // Binary crosses as a real guest `ArrayBuffer` (ADR-012 follow-up): `ctx.dump`
    // flattens a TypedArray to a `{0:..,1:..}` object, so binary CANNOT ride the
    // generic object/array branches — it goes through `newArrayBuffer`, which copies
    // the bytes into the guest (copied data, HARD INVARIANT holds; no live reference).
    // MUST precede the array/object branches (a TypedArray is not `Array.isArray` but
    // would otherwise hit the plain-object branch and be flattened). `newArrayBuffer`
    // accepts a `Uint8Array` or an `ArrayBuffer`; the in-realm shim reads it back as
    // `new Uint8Array(arrayBuffer)`.
    if (value instanceof Uint8Array) {
        dlog('marshal', 'out: host Uint8Array → guest ArrayBuffer', { len: value.length });
        return ctx.newArrayBuffer(value);
    }
    if (value instanceof ArrayBuffer) {
        dlog('marshal', 'out: host ArrayBuffer → guest ArrayBuffer', { len: value.byteLength });
        return ctx.newArrayBuffer(value);
    }
    if (Array.isArray(value)) {
        const arr = ctx.newArray();
        value.forEach((item, i) => {
            const child = marshalToHandle(ctx, item);
            ctx.setProp(arr, i, child);
            child.dispose();
        });
        return arr;
    }
    // Plain object — marshal own enumerable entries (each value re-parsed).
    const obj = ctx.newObject();
    for (const [key, item] of Object.entries(value)) {
        const child = marshalToHandle(ctx, item);
        ctx.setProp(obj, key, child);
        child.dispose();
    }
    return obj;
}
