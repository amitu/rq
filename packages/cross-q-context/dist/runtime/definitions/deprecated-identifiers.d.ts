/**
 * Deprecated Postman identifier registry + warn-once proxy factory.
 *
 * When a user script touches a deprecated bare identifier at runtime (e.g.
 * `globals`, `environment`, `tv4`, `Backbone`), the sandbox warns ONCE per
 * identifier and emits a single analytics signal. The registry below is the
 * single source of truth for WHICH identifiers warn and WHAT they say.
 *
 * The "what" lives here (per ADR-041); the VM injection "how" lives in
 * `modules/sandbox-node`. This file has no VM, no client, no analytics imports.
 *
 * Slice B (RQ-3464): warn-and-no-op only. Slice C (RQ-3465) adds shims that
 * make the deprecated identifiers actually execute — it reuses the same `emit`
 * chokepoint and flips `shimmed` to true, so the warn-once guard still fires
 * exactly once.
 */
/**
 * Per-identifier deprecation policy. Discriminated union (ADR-009) so illegal
 * states — e.g. a "silent" identifier carrying a warning message — are
 * unrepresentable: the silent set is simply ABSENT from the registry, never a
 * policy variant.
 */
export type DeprecatedIdentifierPolicy = {
    /** Bare Postman identifier with a direct `rq.*` replacement. */
    kind: 'warn-and-suggest-rq';
    /** The `rq.*` replacement to point the user at (e.g. `rq.globals.get/set`). */
    replacement: string;
} | {
    /** Third-party global with a non-`rq` alternative, or none. */
    kind: 'warn-only-alternative';
    /** Suggested alternative (e.g. `require('ajv')`), or null when there is none. */
    alternative: string | null;
};
/**
 * Closed registry of deprecated identifiers → policy.
 *
 * SILENT / EXCLUDED by deliberate ABSENCE (do NOT add these — they are
 * SUPPORTED globals, and absence from this registry is the single source of
 * truth for "do not warn"):
 *   - `_`        — lodash, injected as a convenience global (supported).
 *   - `xml2Json` — xml2js wrapper, injected as a convenience global (supported).
 *   - `cheerio`  — available via `require('cheerio')` (supported).
 *   - `CryptoJS` — crypto-js, a built-in package installed as a lazy
 *                  convenience global by `CONVENIENCE_GLOBALS_SHIM` (supported
 *                  since RQ-5512; it was warn-only until crypto-js entered
 *                  `EXTERNAL_BUILTIN_PACKAGES`).
 * A future agent must NOT "complete" this list with those names.
 */
export declare const DEPRECATED_IDENTIFIERS: Readonly<Record<string, DeprecatedIdentifierPolicy>>;
/** All deprecated identifier names (the registry keys). */
export type DeprecatedIdentifier = keyof typeof DEPRECATED_IDENTIFIERS;
/**
 * Produces the deprecation warning message for an identifier + policy.
 *
 * The message is bounded: `identifier` and the policy fields come from the
 * closed `DEPRECATED_IDENTIFIERS` registry, so this yields a finite set of
 * distinct strings (satisfies `gr-static-error-messages`'s "bounded types OK").
 */
export declare function formatDeprecationMessage(identifier: string, policy: DeprecatedIdentifierPolicy): string;
/**
 * Callback fired exactly once, on the first access of a deprecated identifier.
 *
 * The VALUE it carries (identifier + shimmed) is serializable, but `emit`
 * itself is invoked in-process (same Node process as the proxy) — it never
 * crosses the RPC boundary. The boundary crossing happens downstream, when the
 * consumer turns the emitted signal into a stream event (ADR-034).
 */
export type DeprecationEmit = (identifier: string, opts: {
    shimmed: boolean;
}) => void;
/**
 * Shape of the infinite chainable no-op proxy: every property access and every
 * call returns another `DeprecationProxy`. Lets consumers (and tests) exercise
 * `globals.get('x')()` style chains without unsafe casts.
 */
export interface DeprecationProxy {
    (...args: unknown[]): DeprecationProxy;
    readonly [key: string]: DeprecationProxy;
}
/**
 * Creates an infinite chainable proxy that, on FIRST access only, calls the
 * single `emit` chokepoint and then no-ops. Any property access or function
 * call returns the same proxy, so a script using a deprecated identifier (e.g.
 * `globals.get('x')`) does not crash — it gets a warning + a no-op chainable.
 *
 * Mirrors `createUnsupportedStub` in `rqMethods.ts`. Both the console warning
 * and the analytics emit go through `emit` (the single chokepoint) — the
 * factory never warns directly. The `warned` closure guarantees exactly one
 * emit per proxy instance, even across mixed get/apply access.
 *
 * Slice B always emits `{ shimmed: false }`; Slice C will pass a real shim and
 * emit `{ shimmed: true }` from the same chokepoint.
 */
export declare function createDeprecationProxy(identifier: string, emit: DeprecationEmit): DeprecationProxy;
/**
 * The bounded core set of deprecated identifiers that receive a runtime shim
 * delegating to `rq.*` (ADR-156, Slice C / RQ-3465). Closed 4-element list:
 *
 *   - `globals`      → `rq.globals`        (namespace Proxy)
 *   - `environment`  → `rq.environment`    (namespace Proxy)
 *   - `responseBody` → `rq.response.text()` (value, lazy getter)
 *   - `responseCode` → `rq.response`        (the object; lazy getter)
 *
 * Every entry MUST also be a key in `DEPRECATED_IDENTIFIERS` so the shim and
 * its warn message stay in sync (asserted in tests). Adding a fifth identifier
 * requires meeting ADR-156's two-part criterion and amending that ADR — this
 * is not a list to "complete".
 */
export declare const SHIMMED_IDENTIFIERS: readonly ["globals", "environment", "responseBody", "responseCode"];
/** A member of the bounded core shim set. */
export type ShimmedIdentifier = (typeof SHIMMED_IDENTIFIERS)[number];
/**
 * Builds the four runtime shims for the bounded core set (ADR-156).
 *
 * Returns a single object whose keys are exactly `SHIMMED_IDENTIFIERS`:
 *   - `globals` / `environment` are namespace-Proxy VALUES.
 *   - `responseBody` / `responseCode` are defined as **lazy getter descriptors**
 *     (via `Object.defineProperty`), so the consumer transfers them onto the VM
 *     context with `Object.defineProperties(ctx, Object.getOwnPropertyDescriptors(shims))`
 *     and the getter laziness is preserved. The getters MUST stay lazy: reading
 *     `rq.response.text()` eagerly at factory time would throw in the
 *     pre-request phase where `rq.response` is `null`.
 *
 * Each shim fires the shared `emit` chokepoint with `{ shimmed: true }` exactly
 * once on first access (warn-once, per-identifier), then delegates to `rq.*`.
 * Value shims return `undefined` (never throw) when `rq.response` is `null`,
 * matching native `rq` phase-availability behavior.
 *
 * `rq` is `unknown` at this boundary — narrowed once via `asRqView`. No platform
 * assumption, no VM, no analytics import (`gr-modules-platform-agnostic`).
 */
export declare function createDeprecatedPostmanShims(rq: unknown, emit: DeprecationEmit): Record<ShimmedIdentifier, unknown>;
