/**
 * deprecation-bridge — Safe-mode legacy Postman deprecation identifiers
 * (ADR-156 parity for the QuickJS engine; mirrors NodeSandbox's Developer-mode
 * deprecation machinery).
 *
 * Developer mode (`node:vm`, `node-sandbox.ts`) injects host-realm `Proxy`
 * objects + lazy getter descriptors for the 13 `DEPRECATED_IDENTIFIERS` (4 of
 * them — `globals` / `environment` / `responseBody` / `responseCode` — real
 * `rq.*` shims; the other 9 warn-once chainable no-ops). The Safe engine
 * CANNOT inject a host `Proxy`/getter into the QuickJS guest: that is a live host
 * reference, which both is impossible to copy across the boundary and would
 * violate the HARD INVARIANT (ADR-010 §16 / ADR-012). So the identical shims are
 * rebuilt INSIDE the guest over the guest's own `globalThis.rq`, with a
 * fire-and-forget `__rq_deprecation` callback for the warn-once analytics/log
 * chokepoint. This is the same two-part shape as the console bridge: a
 * `createIgnoredBridge` host callback + an in-guest shim string.
 *
 * The shared registry (`@requestly/sandbox-definitions/deprecated-identifiers`)
 * stays the single source of truth for WHICH identifiers warn, WHAT they say, and
 * WHICH are shimmed. The two identifier lists embedded in the in-guest shim below
 * are kept identical to that registry by a parity assertion in the engine test
 * (`quickjs-sandbox.test.ts`).
 */
import type { SafeBridge } from '../safe-bridge-factory.js';
import type { DeprecationEmit } from '../../../index.js';
/**
 * The 9 warn-only identifiers — every registry key that is NOT one of the four
 * real shims. DERIVED from the registry (the single source of truth) so the
 * in-guest shim's list can never drift from it: add an identifier to
 * `DEPRECATED_IDENTIFIERS` (or move one in/out of `SHIMMED_IDENTIFIERS`) and this
 * recomputes. Both the shim string (below) and the engine test consume these two
 * exported constants, so there is no second hand-maintained copy to drift.
 */
export declare const WARN_ONLY_IDENTIFIERS: readonly string[];
/** The four real `rq.*`-delegating shims, re-exported as a plain string[] for the shim/test. */
export declare const DEPRECATION_SHIMMED_IDENTIFIERS: readonly string[];
/**
 * Host bridge: the guest calls `__rq_deprecation(identifier, shimmed)` on the
 * FIRST access of a deprecated identifier; the host turns it into the engine's
 * `emit(identifier, { shimmed })` chokepoint (which pushes the `deprecation`
 * signal + warn `log`). Only copied strings/booleans cross the edge — no live
 * reference, no return path (fire-and-forget).
 */
export declare function createDeprecationBridge(emit: DeprecationEmit): SafeBridge;
/**
 * In-guest JS: builds all 14 deprecated identifiers as globals over
 * `globalThis.rq` and the installed `globalThis.__rq_deprecation` callback. Eval'd
 * by the engine AFTER the rq-namespace shim (the 4 real shims delegate to `rq.*`).
 *
 * Achieves BEHAVIORAL parity with `deprecated-identifiers.ts` via a different
 * (guest-realm IIFE) implementation — the host realm builds JS `Proxy` objects +
 * `Object.defineProperty` descriptors and injects them into the vm context, which
 * is impossible across the QuickJS boundary, so the same observable behavior is
 * rebuilt inside the guest:
 * - `globals` / `environment` — access-semantics namespace proxy: a bare read
 *   `globals.<name>` resolves to `rq.globals.get('<name>')`, a bare write
 *   `globals.<name> = v` to `rq.globals.set('<name>', v)`; the five real methods
 *   (`get/set/unset/has/toObject`) pass through to the real namespace, bound.
 * - `responseBody` / `responseCode` — lazy getters delegating to
 *   `rq.response.text()` / `rq.response`; `undefined` (never throw) when
 *   `rq.response` is null OR malformed. The malformed-shape guard mirrors
 *   Developer mode's `isResponseView` (`deprecated-identifiers.ts`): the response
 *   counts only when `.code` is a number AND `.text` is a function, so a partial
 *   response yields `undefined` in BOTH engines (true parity, not just null parity).
 * - the other 10 — warn-once infinite chainable no-op proxies (any get/apply
 *   returns the proxy, so legacy chains like `tests['x'] = …` never crash).
 *
 * The two identifier lists are INTERPOLATED from the exported `WARN_ONLY_IDENTIFIERS`
 * / `DEPRECATION_SHIMMED_IDENTIFIERS` constants (both derived from the registry),
 * so the shim's lists are the SAME values the engine test asserts on — they cannot
 * silently diverge from `DEPRECATED_IDENTIFIERS`.
 *
 * Warn-once is enforced per-identifier in-guest (a `fired` closure), so repeated
 * access does not re-emit. Symbol-keyed access (engine/debugger internals:
 * `Symbol.toPrimitive`, `Symbol.iterator`, `Symbol.toStringTag`, …) never fires
 * the emit — matching the host proxies' symbol guard.
 */
export declare const DEPRECATION_ISOLATE_SHIM: string;
