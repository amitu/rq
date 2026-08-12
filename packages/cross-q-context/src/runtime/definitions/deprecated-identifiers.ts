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
export type DeprecatedIdentifierPolicy =
  | {
      /** Bare Postman identifier with a direct `rq.*` replacement. */
      kind: 'warn-and-suggest-rq';
      /** The `rq.*` replacement to point the user at (e.g. `rq.globals.get/set`). */
      replacement: string;
    }
  | {
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
export const DEPRECATED_IDENTIFIERS: Readonly<Record<string, DeprecatedIdentifierPolicy>> = {
  // Bare Postman globals with direct rq.* replacements.
  globals: { kind: 'warn-and-suggest-rq', replacement: 'rq.globals.get/set' },
  environment: { kind: 'warn-and-suggest-rq', replacement: 'rq.environment.get/set' },
  iteration: { kind: 'warn-and-suggest-rq', replacement: 'rq.info.iteration' },
  responseBody: { kind: 'warn-and-suggest-rq', replacement: 'rq.response.text()' },
  responseCode: { kind: 'warn-and-suggest-rq', replacement: 'rq.response.code' },
  responseHeaders: { kind: 'warn-and-suggest-rq', replacement: 'rq.response.headers' },
  responseCookies: { kind: 'warn-and-suggest-rq', replacement: 'rq.cookies' },
  responseTime: { kind: 'warn-and-suggest-rq', replacement: 'rq.response.responseTime' },
  tests: { kind: 'warn-and-suggest-rq', replacement: 'rq.test' },
  data: { kind: 'warn-and-suggest-rq', replacement: 'rq.iterationData.get' },
  request: { kind: 'warn-and-suggest-rq', replacement: 'rq.request' },

  // Third-party globals with non-rq alternatives (or none).
  tv4: { kind: 'warn-only-alternative', alternative: "require('ajv')" },
  Backbone: { kind: 'warn-only-alternative', alternative: null },
} as const;

/** All deprecated identifier names (the registry keys). */
export type DeprecatedIdentifier = keyof typeof DEPRECATED_IDENTIFIERS;

/**
 * Produces the deprecation warning message for an identifier + policy.
 *
 * The message is bounded: `identifier` and the policy fields come from the
 * closed `DEPRECATED_IDENTIFIERS` registry, so this yields a finite set of
 * distinct strings (satisfies `gr-static-error-messages`'s "bounded types OK").
 */
export function formatDeprecationMessage(identifier: string, policy: DeprecatedIdentifierPolicy): string {
  switch (policy.kind) {
    case 'warn-and-suggest-rq':
      return `${identifier} is deprecated — use ${policy.replacement}`;
    case 'warn-only-alternative':
      return policy.alternative === null
        ? `${identifier} is deprecated and not supported in Requestly`
        : `${identifier} is deprecated — use ${policy.alternative}`;
  }
}

/**
 * Callback fired exactly once, on the first access of a deprecated identifier.
 *
 * The VALUE it carries (identifier + shimmed) is serializable, but `emit`
 * itself is invoked in-process (same Node process as the proxy) — it never
 * crosses the RPC boundary. The boundary crossing happens downstream, when the
 * consumer turns the emitted signal into a stream event (ADR-034).
 */
export type DeprecationEmit = (identifier: string, opts: { shimmed: boolean }) => void;

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
export function createDeprecationProxy(identifier: string, emit: DeprecationEmit): DeprecationProxy {
  let warned = false;
  const fire = (): void => {
    if (warned) return;
    warned = true;
    emit(identifier, { shimmed: false });
  };
  // Callable target with the recursive index signature, built without casts.
  // The Proxy traps below intercept every get/apply and return `proxy`, so the
  // target body itself is never actually reached.
  const target: DeprecationProxy = Object.assign(function deprecated(): DeprecationProxy {
    return proxy;
  }, undefinedIndex());
  const handler: ProxyHandler<DeprecationProxy> = {
    get(_t, prop) {
      // User scripts access deprecated identifiers by string name only. Guard
      // against ALL symbol-keyed access: JS engines internally read well-known
      // symbols (Symbol.toStringTag during String/Object coercion and
      // console.log, Symbol.hasInstance during instanceof, etc.), and the
      // debugger/inspector introspects via symbols — none of those are the
      // user's code, so they must not fire the deprecation signal.
      if (typeof prop === 'symbol') return undefined;
      fire();
      return proxy;
    },
    apply() {
      fire();
      return proxy;
    },
  };
  const proxy: DeprecationProxy = new Proxy(target, handler);
  return proxy;
}

/** Empty object typed as the index half of DeprecationProxy (no enumerable keys). */
function undefinedIndex(): Record<string, DeprecationProxy> {
  return {};
}

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
export const SHIMMED_IDENTIFIERS = ['globals', 'environment', 'responseBody', 'responseCode'] as const;

/** A member of the bounded core shim set. */
export type ShimmedIdentifier = (typeof SHIMMED_IDENTIFIERS)[number];

/**
 * Minimal structural view of the fully-built `rq` object that the shims read.
 *
 * `rq` is typed `unknown` at the factory boundary (its concrete type is known
 * only to the builder; importing it here would couple this dependency-light
 * file to the builder). We narrow `unknown` to this view through a single
 * runtime-checked accessor (`asRqView`) rather than an unsafe `as`-cast to a
 * fabricated shape — satisfies `gr-no-unsafe-cast`.
 *
 * The three members are narrowed INDEPENDENTLY (RQ-3465). `globals` /
 * `environment` are present in BOTH the pre-request and post-response phases,
 * so their shims must NEVER be gated on `response`. `response` is `null` in the
 * pre-request phase by design — its absence only nulls out the
 * `responseBody` / `responseCode` value shims; it does not suppress the
 * `globals` / `environment` namespace shims. Earlier code bailed (returned
 * `null` for the whole view) when `response` was missing, which dropped the
 * `globals` / `environment` shims in pre-request → `globals.set` was undefined
 * → `TypeError`. The fields are therefore decoupled below.
 */
interface RqShimView {
  /** Non-null when `rq.globals` exposes the callable get/set surface. */
  readonly globals: unknown;
  /** Non-null when `rq.environment` exposes the callable get/set surface. */
  readonly environment: unknown;
  /** `null` in the pre-request phase; an object with `.code` / `.text()` otherwise. */
  readonly response: { readonly code: number; text(): string } | null;
}

/**
 * Narrows the `unknown` `rq` to `RqShimView`. `rq` is always the builder's
 * plain rq object (constructed in-process, never crossing a boundary), so this
 * is a safe structural narrowing — verified by the property probes rather than
 * asserted.
 *
 * Each member is narrowed INDEPENDENTLY (RQ-3465) — a missing `response` (the
 * pre-request phase) does NOT suppress `globals` / `environment`. The function
 * returns `null` only when `rq` is not an object at all; otherwise it returns a
 * view whose individual members may be undefined / null, and the per-shim
 * builders degrade those to `undefined` (never throw).
 */
function asRqView(rq: unknown): RqShimView | null {
  if (typeof rq !== 'object' || rq === null) return null;
  // Read members via Reflect.get so a missing key simply yields `undefined`
  // without a guard that gates the whole view. `response` is absent in the
  // pre-request phase — that must NOT suppress globals/environment.
  const response = Reflect.get(rq, 'response');
  const responseView = isResponseView(response) ? response : null;
  return {
    globals: Reflect.get(rq, 'globals'),
    environment: Reflect.get(rq, 'environment'),
    response: responseView,
  };
}

/** Runtime check that `rq.response` exposes the `.code` number + `.text()` method. */
function isResponseView(value: unknown): value is { readonly code: number; text(): string } {
  if (typeof value !== 'object' || value === null) return false;
  if (!('code' in value) || !('text' in value)) return false;
  const record: Record<string, unknown> = value;
  return typeof record['code'] === 'number' && typeof record['text'] === 'function';
}

/**
 * The five real methods exposed by `rq.globals` / `rq.environment`. A property
 * access whose name is in this set is a METHOD CALL (pass-through); any other
 * string property access is a VARIABLE READ resolved via `.get(name)` (and a
 * write via `.set(name, value)`). This is the access-semantics translation
 * (ADR-156 amendment 2026-06-15) — see `createNamespaceShim`.
 */
const NAMESPACE_METHODS: ReadonlySet<string> = new Set(['get', 'set', 'unset', 'has', 'toObject']);

/** Minimal structural view of an `rq.globals` / `rq.environment` namespace. */
interface VariableNamespace {
  get(name: string): unknown;
  set(name: string, value: unknown): void;
}

/**
 * Runtime-narrows the delegate to the get/set surface so bare reads/writes can
 * be resolved as variable access without an unsafe cast. Returns `null` when
 * the namespace does not expose callable `get`/`set` (e.g. unexpected rq shape).
 */
function asVariableNamespace(delegate: object): VariableNamespace | null {
  const get = Reflect.get(delegate, 'get');
  const set = Reflect.get(delegate, 'set');
  if (typeof get !== 'function' || typeof set !== 'function') return null;
  return {
    get: (name) => get.call(delegate, name),
    set: (name, value) => {
      set.call(delegate, name, value);
    },
  };
}

/**
 * Creates a namespace shim Proxy for `globals` / `environment` implementing the
 * ADR-156 (2026-06-15 amendment) ACCESS-SEMANTICS translation, not a plain
 * namespace passthrough. In legacy Postman, `globals.<name>` is a VARIABLE READ
 * (`globals.get('<name>')`) and `globals.<name> = v` a VARIABLE WRITE
 * (`globals.set('<name>', v)`) — NOT a property of an arbitrary-property object.
 * Because `rq.globals` exposes only the five methods get/set/unset/has/toObject,
 * a passthrough returns `undefined` for `globals.checkErrorWarning`, turning
 * `eval(globals.checkErrorWarning)` into a silent no-op → downstream
 * `ReferenceError` (the AirCanada symptom). The traps below fix that:
 *
 *   - `get(prop)`:
 *     - symbol / non-string prop, or one of the five real methods → return the
 *       real `delegate[prop]` (bound when a function) so `globals.get('k')`,
 *       `globals.set(...)`, iteration, etc. still work.
 *     - otherwise (a bare property read) → return `delegate.get(prop)`, i.e. the
 *       stored variable value. `eval(globals.checkErrorWarning)` therefore reads
 *       the stored function-source string. The trap is naturally recursive:
 *       nested `eval(globals.customizer)` inside an eval'd body hits the same
 *       proxy.
 *   - `set(prop, value)`: a non-method prop write delegates to
 *     `delegate.set(prop, value)` so `globals.foo = x` mutates the store at
 *     runtime too. Method names are not assignable — ignore and return true.
 *
 * On the FIRST access of any shape it fires `emit(identifier, { shimmed: true })`
 * exactly once (warn-once-per-identifier). The `fire()` once-guard lives outside
 * the per-prop traps, so repeated property gets do NOT re-emit. Mirrors the
 * `Symbol.toPrimitive` / `Symbol.iterator` guard of `createDeprecationProxy`.
 */
function createNamespaceShim(identifier: ShimmedIdentifier, target: unknown, emit: DeprecationEmit): unknown {
  let warned = false;
  const fire = (): void => {
    if (warned) return;
    warned = true;
    emit(identifier, { shimmed: true });
  };
  // The target may be undefined if the rq shape was unexpected; fall back to an
  // empty object so property access yields `undefined` rather than throwing.
  const delegate: object = typeof target === 'object' && target !== null ? target : {};
  const namespace = asVariableNamespace(delegate);
  const handler: ProxyHandler<object> = {
    get(_t, prop, receiver) {
      if (prop === Symbol.toPrimitive || prop === Symbol.iterator) return undefined;
      fire();
      // Symbols and the five real methods pass through to the real namespace
      // (functions bound to the delegate so `this` is correct).
      if (typeof prop !== 'string' || NAMESPACE_METHODS.has(prop)) {
        const value = Reflect.get(delegate, prop, receiver);
        return typeof value === 'function' ? value.bind(delegate) : value;
      }
      // Bare property read → resolve as a variable read of the stored value.
      return namespace === null ? undefined : namespace.get(prop);
    },
    set(_t, prop, value) {
      fire();
      // Method names are not assignable; silently ignore (return true so strict
      // mode does not throw). Any other string prop write is a variable write.
      if (typeof prop === 'string' && !NAMESPACE_METHODS.has(prop) && namespace !== null) {
        namespace.set(prop, value);
      }
      return true;
    },
    has(_t, prop) {
      return Reflect.has(delegate, prop);
    },
  };
  return new Proxy(delegate, handler);
}

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
export function createDeprecatedPostmanShims(rq: unknown, emit: DeprecationEmit): Record<ShimmedIdentifier, unknown> {
  const view = asRqView(rq);

  const shims: Record<ShimmedIdentifier, unknown> = {
    globals: createNamespaceShim('globals', view?.globals, emit),
    environment: createNamespaceShim('environment', view?.environment, emit),
    // Defined below as lazy getters; the placeholder `undefined` values are
    // overwritten by Object.defineProperty so a bare reference reads the getter.
    responseBody: undefined,
    responseCode: undefined,
  };

  let responseBodyWarned = false;
  Object.defineProperty(shims, 'responseBody', {
    enumerable: true,
    configurable: true,
    get(): unknown {
      if (!responseBodyWarned) {
        responseBodyWarned = true;
        emit('responseBody', { shimmed: true });
      }
      return view?.response == null ? undefined : view.response.text();
    },
  });

  let responseCodeWarned = false;
  Object.defineProperty(shims, 'responseCode', {
    enumerable: true,
    configurable: true,
    get(): unknown {
      if (!responseCodeWarned) {
        responseCodeWarned = true;
        emit('responseCode', { shimmed: true });
      }
      // Delegation target is the response OBJECT (ADR-156 authoritative table):
      // bare `responseCode` resolves to `rq.response`, so `responseCode.code`
      // yields the status number, matching Slice A's `responseCode.code →
      // rq.response.code` rewrite. `undefined` (not throw) when null pre-request.
      return view?.response == null ? undefined : view.response;
    },
  });

  return shims;
}
