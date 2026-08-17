/**
 * Safe-mode classification for a built-in package (sandbox-node ADR-010 §10/§32).
 *
 * Drives how the in-isolate require chain (Safe mode / `IsolatedVmSandbox`)
 * resolves the package:
 * - `source_bundle` — pure JS; bundled host-side and eval'd inside the isolate
 *   with no host capability.
 * - `needs_bridge` — reaches a virtualizable capability (Buffer, crypto subset,
 *   util, stream, fetch, zlib) satisfied by an authored data-in/data-out bridge.
 * - `impossible` — needs a live OS socket, live fd, or native `.node` addon;
 *   fails with a guided error carrying a `ScriptPackageUnsupportedReason`.
 *
 * Developer mode (`node:vm` / `NodeSandbox`) ignores this field — it reaches the
 * host realm by design.
 */
export type SafeModeClass = 'source_bundle' | 'needs_bridge' | 'impossible';

/**
 * Bounded reason an `impossible` package cannot run in Safe mode
 * (sandbox-node ADR-010 §87). Carried on the guided error the require chain
 * throws and — in Slice 3 — surfaced as the machine-readable classification on
 * the `Script Package Unsupported` analytics event.
 *
 * Discriminated union, not a free string (`gr-discriminated-unions`): every
 * IMPOSSIBLE blocker maps to exactly one of these.
 */
export type ScriptPackageUnsupportedReason = 'native_addon' | 'live_socket' | 'live_fs' | 'asymmetric_crypto' | 'other';

/**
 * A safe Node.js built-in module exposed to user scripts via require().
 *
 * In DEVELOPER mode these delegate to Node's native require(). They appear in the
 * editor's require() autocomplete but NOT in the Packages UI dropdown (LibraryPicker).
 *
 * In SAFE mode there is no Node, so a `source_bundle` built-in must be served by a
 * pure-JS in-isolate IIFE. Most of the `source_bundle` Node built-ins here do not yet
 * ship that IIFE (a latent gap — `require('path')`/`url`/`assert`/… still throw the
 * IMPOSSIBLE guided error in Safe). The two optional fields below opt a specific
 * built-in into an in-isolate polyfill bundle; `events` is the first to use them
 * (RQ-5625, needed by xml2js → `xml2Json`). To fix another one, add its browserify
 * polyfill as a sandbox-node devDependency and set both fields.
 */
/**
 * How Developer mode keeps a `require()`d built-in's async work visible to the
 * `AsyncRegistry` (ADR-219, RQ-5671 Phase 3).
 *
 * Safe mode needs no equivalent: every `needs_bridge` module reaches the host
 * through a counted bridge, so its coverage is automatic. Developer's `require()`
 * hands the script the REAL Node module, so anything async it starts is invisible
 * unless wrapped here — the same enumerated-vs-structural gap RQ-5671 closed for
 * globals, one level down.
 *
 * This field is **required**, so a new built-in cannot be added without deciding.
 *
 * - `registry-timers` — the module IS the timer surface; serve the registry's own
 *   wrappers instead of Node's (`timers`).
 * - `callback-last` — one-shot callback-style async APIs; wrap so a hold is held
 *   until the callback fires (`crypto`, `zlib`).
 * - `not-an-async-source` — cannot start async work on its own in this sandbox.
 *   Justify per entry: pure sync (`path`, `assert`, …), or async-capable only when
 *   driven by something already covered (`stream` has no fs/net to pump it, and
 *   `util.promisify` merely wraps a function whose own class already applies).
 */
export type DeveloperAsyncTreatment = 'registry-timers' | 'callback-last' | 'not-an-async-source';

/** The async-classified global names (timers + fetch) — the Developer engine's async surface.
 * Defined here as a literal union (the app derives it from GLOBAL_NAMES in a codegen file). */
export type AsyncGlobalName = 'setTimeout' | 'setInterval' | 'clearTimeout' | 'clearInterval' | 'fetch';

export interface NodeBuiltinPackage {
  /** require() identifier (e.g., 'crypto') */
  readonly id: string;
  /** Display name for autocomplete */
  readonly name: string;
  /** One-line description for autocomplete */
  readonly description: string;
  /** Safe-mode resolution class (ADR-010). Drives the in-isolate require chain. */
  readonly safeModeClass: SafeModeClass;
  /**
   * How Developer mode keeps this module's async work registry-visible (ADR-219).
   * Required: adding a built-in forces the decision rather than defaulting to
   * "invisible".
   */
  readonly developerAsync: DeveloperAsyncTreatment;
  /** Why this package is unavailable in Safe mode — present iff safeModeClass is 'impossible'. */
  readonly impossibleReason?: ScriptPackageUnsupportedReason;
  /**
   * IIFE global name for the Safe-mode in-isolate bundle (e.g. `__events`). Present
   * iff this `source_bundle` built-in is served by a generated polyfill IIFE rather
   * than Node's native module. Pairs with `polyfillEntry`. Mirrors
   * `ExternalBuiltinPackage.globalName`.
   */
  readonly globalName?: string;
  /**
   * npm specifier the vendor-IIFE codegen bundles into the in-isolate polyfill
   * (e.g. `'events/'` — the trailing slash forces the npm `events` package over
   * Node's built-in of the same name). Present with `globalName`. The pinned
   * version lives in `modules/sandbox-node`'s devDependencies (where the codegen
   * resolves it), not here.
   */
  readonly polyfillEntry?: string;
}

/**
 * Type contract for an external built-in sandbox package (IIFE-bundled npm packages).
 *
 * Both the IIFE generator (sandbox-node) and codegen (sandbox-definitions)
 * consume this interface from the EXTERNAL_BUILTIN_PACKAGES registry.
 */
export interface ExternalBuiltinPackage {
  /** require() identifier (e.g., 'csv-parse/lib/sync') */
  readonly id: string;
  /** npm entry point for esbuild (e.g., 'csv-parse/lib/sync') */
  readonly entry: string;
  /** IIFE global name (e.g., '__csv_parse') */
  readonly globalName: string;
  /** Pinned version — must match devDependency in both package.json files */
  readonly version: string;
  /** @types/ package name for packages that don't ship own types (e.g., '@types/chai') */
  readonly typesPackage?: string;
  /** Pinned @types/ version — independent of runtime version (e.g., '5.2.3' for @types/chai) */
  readonly typesVersion?: string;
  /** Safe-mode resolution class (ADR-010). Drives the in-isolate require chain. */
  readonly safeModeClass: SafeModeClass;
  /** Why this package is unavailable in Safe mode — present iff safeModeClass is 'impossible'. */
  readonly impossibleReason?: ScriptPackageUnsupportedReason;
  /**
   * Internal (vendor-only) package: the IIFE is delivered into BOTH guests via
   * VENDOR_IIFES + the require chain exactly like a normal built-in, but the
   * user-facing codegen surface is suppressed — no `require('<id>')` overload in
   * `require.d.ts`, so it never appears in editor autocomplete or the Packages
   * dropdown. Used for impl dependencies the sandbox needs internally but users
   * should not `require` directly (e.g. Handlebars, an impl detail of the
   * response visualizer). See ADR-202 Decision 2 (the resolved open question).
   */
  readonly internal?: boolean;
}
