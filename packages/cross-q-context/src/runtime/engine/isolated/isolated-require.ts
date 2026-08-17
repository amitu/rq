/**
 * isolated-require — the in-isolate require() chain (ADR-010 §82, the dispatch point).
 *
 * The Safe-mode analogue of `createRequireFn` (ADR-005). It carries that chain's
 * tier *shape* but changes the last inch: instead of returning host-realm live
 * objects, it routes each package to its `safeModeClass` column —
 *   needs_bridge → the pre-installed in-isolate module global (Buffer, crypto, …)
 *   source_bundle / user package → host-bundled source, eval'd IN-ISOLATE
 *   impossible → the guided error (ADR-010 §77)
 *
 * Data flow (the only flow that works — an isolate has no module system, so
 * nothing `import`s sandbox-definitions inside the isolate): the HOST reads the
 * registry, bakes a `packageId → safeModeClass/reason` map into the bundle-require
 * callback's closure, and installs:
 *   - `__rq_bundleRequire(id)` — a host callback that, given a require id,
 *     returns `{ kind: 'bridge', global } | { kind: 'source', code } | throws`.
 *     Only copied data crosses (a small record or a thrown copied error).
 *   - REQUIRE_ISOLATE_SHIM — in-isolate JS building `globalThis.require` on top
 *     of that callback, with an in-isolate module cache and CommonJS eval for
 *     source bundles. The bundle eval happens INSIDE the isolate (HARD INVARIANT).
 */

import {
  EXTERNAL_BUILTIN_PACKAGES,
  NODE_BUILTIN_PACKAGES,
  extractPackageName,
  isUserPackageRequire,
  parseRequireId,
} from '../../index.js';

import { builtinModules } from 'node:module';

import { createPackageError } from './package-error-sentinel.js';
import { NEEDS_BRIDGE_MODULE_GLOBALS } from './needs-bridge-globals.js';
import { classifyBundleFailure } from './classify-bundle-failure.js';
import { dlog } from './debug-log.js';
import { createImpossiblePackageError } from './impossible-error.js';

import type {
  ExternalBuiltinPackage,
  NodeBuiltinPackage,
  SafeModeClass,
  SafePackageResolver,
  ScriptPackageUnsupportedReason,
} from '../../index.js';
import type { SourceBundler } from './source-bundler.js';

/** What the host bundle-require callback returns to the in-isolate require. */
type BundleRequireResult =
  | { readonly kind: 'bridge'; readonly global: string }
  | { readonly kind: 'iife'; readonly code: string; readonly globalName: string }
  | { readonly kind: 'source'; readonly code: string }
  // A package whose bundle failed in the pre-pass (ADR-010 §87). Carries the
  // classified reason so `resolveRequire` throws the guided IMPOSSIBLE error with
  // a precise `reason` (consumed by the Script Package Unsupported analytics event)
  // instead of a blanket `other`. NEVER returned to the in-isolate require —
  // `resolveRequire` throws on this arm.
  | { readonly kind: 'impossible'; readonly reason: ScriptPackageUnsupportedReason; readonly packageId: string };

/** A package id → its safe-mode classification, baked from the registry host-side. */
interface ClassEntry {
  readonly safeModeClass: SafeModeClass;
  readonly reason?: ScriptPackageUnsupportedReason;
  /** For SOURCE_BUNDLE built-ins: the IIFE global name (e.g. `__chai`). */
  readonly globalName?: string;
}

/** Build the dispatch map once from the registries (both bare + node: forms). */
function buildClassMap(): ReadonlyMap<string, ClassEntry> {
  const map = new Map<string, ClassEntry>();
  // Widen from the `as const`-narrowed tuple literals to the interface so the
  // optional `impossibleReason` field is visible (no current package is
  // `impossible`, so the literals omit it).
  const externals: readonly ExternalBuiltinPackage[] = EXTERNAL_BUILTIN_PACKAGES;
  const nodeBuiltins: readonly NodeBuiltinPackage[] = NODE_BUILTIN_PACKAGES;
  for (const pkg of externals) {
    // External built-ins carry a globalName — they resolve via the build-time
    // IIFE (VENDOR_IIFES), not runtime esbuild (they are devDependencies, not
    // installed at runtime). See the source_bundle branch in resolveRequire.
    map.set(pkg.id, { safeModeClass: pkg.safeModeClass, reason: pkg.impossibleReason, globalName: pkg.globalName });
  }
  for (const pkg of nodeBuiltins) {
    // `globalName` is set only for the Node built-ins served by an in-isolate
    // polyfill IIFE (e.g. `events`, RQ-5625) — carrying it here lets the
    // source_bundle branch of resolveRequire resolve them via VENDOR_IIFES exactly
    // like an EXTERNAL package. The rest have no globalName and stay Node-native
    // (Developer) / IMPOSSIBLE (Safe).
    const entry: ClassEntry = {
      safeModeClass: pkg.safeModeClass,
      reason: pkg.impossibleReason,
      globalName: pkg.globalName,
    };
    map.set(pkg.id, entry);
    map.set(`node:${pkg.id}`, entry);
  }
  return map;
}

const CLASS_MAP = buildClassMap();

/**
 * Inputs the host needs to resolve a require id to a bundle-or-bridge result.
 * The engine supplies these per execution.
 */
export interface BundleRequireDeps {
  /** Host-side cache-aware bundler (SOURCE_BUNDLE). */
  readonly bundler: SourceBundler;
  /** Raw source for user-authored packages, keyed by name (ADR-087). */
  readonly userPackages: Record<string, string> | undefined;
  /** Package names blocked by the blacklist filter (ADR-087 Tier 3.5). */
  readonly blacklistedPackages: readonly string[];
  /**
   * Returns the BUILD-TIME IIFE string for a SOURCE_BUNDLE built-in package id
   * (from `VENDOR_IIFES`), or undefined if not present. Built-ins (chai, uuid,
   * moment, …) are bundled at build time — they are sandbox-node
   * devDependencies, NOT installed at runtime, so they CANNOT be resolved/bundled
   * at execution time (the desktop worker's node_modules has no chai). Reusing
   * the pre-generated IIFE (ADR-003's machinery, which ADR-010 §24 extends) is
   * the correct path: no runtime resolution, ships with the engine.
   */
  readonly vendorIife: (id: string) => string | undefined;
  /** Directory esbuild resolves a user package's own `require()`s from. */
  readonly userPackageResolveDir: string;
  /**
   * Resolver for user-INSTALLED npm packages (ADR-010 §85, Tier 4). Undefined on
   * clients without a package store (e.g. the in-process CLI posture). Safe mode
   * reads the entry PATH (`resolveEntryPath`) and bundles it — never the live
   * object `resolve()` returns (that would breach the HARD INVARIANT).
   */
  readonly resolver: SafePackageResolver | undefined;
  /**
   * Per-execution context id routing resolution to the right node_modules
   * (mirrors NodeSandbox's `${pre|post}-${entryId}`). Undefined ⇒ no resolution.
   */
  readonly contextId: string | undefined;
}

/**
 * A pre-built id→result map produced by `prebundleRequires` BEFORE the isolate
 * runs. The sync require callback reads from this — it never bundles inline,
 * because esbuild's only worker-safe API is async (`buildSync` deadlocks in the
 * Electron sandbox worker, RQ-3359).
 */
export type PrebundledRequires = ReadonlyMap<string, BundleRequireResult>;

/**
 * Classify a require id by tier WITHOUT bundling. Returns:
 *  - a terminal `BundleRequireResult` for bridge / IIFE built-ins (no async work), or
 *  - `{ needsBundle: 'source' | 'entry', ... }` when the id requires an async
 *    esbuild bundle (user-authored source, or an installed-package entry path), or
 *  - throws the guided IMPOSSIBLE error for blacklist / impossible / not-found.
 *
 * Both the pre-bundle pass (async) and the sync callback share this single
 * classification so tiering can't drift between them.
 */
type Classification =
  | { readonly settled: BundleRequireResult }
  | { readonly needsBundle: 'source'; readonly name: string; readonly source: string; readonly resolveDir: string }
  | { readonly needsBundle: 'entry'; readonly pkgName: string; readonly entryPath: string };

function classifyRequire(id: string, deps: BundleRequireDeps): Classification {
  if (typeof id !== 'string' || id === '') {
    throw new Error('require() argument must be a non-empty string');
  }

  // Tier: user-authored packages (.js suffix, ADR-087) — bundle source, eval in-isolate.
  if (isUserPackageRequire(id) && deps.userPackages) {
    const name = extractPackageName(id);
    const source = deps.userPackages[name];
    if (source !== undefined) {
      return { needsBundle: 'source', name, source, resolveDir: deps.userPackageResolveDir };
    }
    throw createPackageError(
      `Package '${name}' not found in project. Check the package name or create it in the Package Library.`,
    );
  }

  // Tier: blacklist (ADR-087 Tier 3.5).
  if (deps.blacklistedPackages.length > 0) {
    const parsed = parseRequireId(id);
    if (parsed.packageName && deps.blacklistedPackages.includes(parsed.packageName)) {
      throw createImpossiblePackageError(parsed.packageName, 'other');
    }
  }

  // Tier: classified built-ins — dispatch by safeModeClass.
  const entry = CLASS_MAP.get(id);
  if (entry) {
    if (entry.safeModeClass === 'impossible') {
      throw createImpossiblePackageError(id, entry.reason ?? 'other');
    }
    if (entry.safeModeClass === 'needs_bridge') {
      const global = NEEDS_BRIDGE_MODULE_GLOBALS[id];
      if (global) return { settled: { kind: 'bridge', global } };
      throw createImpossiblePackageError(id, 'other');
    }
    // source_bundle built-in: the BUILD-TIME IIFE (no runtime esbuild).
    const iife = deps.vendorIife(id);
    if (iife !== undefined && entry.globalName) {
      return { settled: { kind: 'iife', code: iife, globalName: entry.globalName } };
    }
    throw createImpossiblePackageError(id, 'other');
  }

  // Node builtin not in CLASS_MAP (e.g. child_process, fs, net) — impossible.
  // Checked before the resolver so builtins never fall through to "not found."
  const bareId = id.startsWith('node:') ? id.slice(5) : id;
  if (builtinModules.includes(bareId)) {
    throw createImpossiblePackageError(bareId, 'other');
  }

  // Tier 4: user-INSTALLED npm packages (ADR-010 §85). Resolve to the on-disk
  // entry PATH (never the live object); the bundle happens in the async pre-pass.
  if (deps.resolver && deps.contextId) {
    const parsed = parseRequireId(id);
    const pkgName = parsed.packageName ?? id;
    const entryPath = deps.resolver.resolveEntryPath(id, deps.contextId);
    if (entryPath !== undefined) {
      return { needsBundle: 'entry', pkgName, entryPath };
    }
    throw createPackageError(`Package '${pkgName}' not found. Install it via the Package Library.`);
  }

  // Truly unknown package — not found.
  throw createPackageError(`Package '${id}' not found.`);
}

/**
 * Pre-bundle pass (ADR-010 §82, RQ-3359). Runs BEFORE the isolate executes:
 * classifies each require id the script needs and, for the ones that require an
 * esbuild bundle, does it ASYNC (`build()`, never `buildSync` — the latter
 * deadlocks in the Electron sandbox worker). Returns an id→result map the sync
 * require callback then reads. Bridge/IIFE ids are resolved here too so the map
 * is the single thing the callback consults.
 *
 * A bundle failure (native `.node` addon, unresolvable import) maps to a guided
 * IMPOSSIBLE result baked into the map; a not-found id throws during classify and
 * is recorded as a deferred throw the callback re-raises (so the error surfaces
 * at the require call site, attributed, not at pre-bundle time).
 */
export async function prebundleRequires(ids: readonly string[], deps: BundleRequireDeps): Promise<PrebundledRequires> {
  const out = new Map<string, BundleRequireResult>();
  for (const id of new Set(ids)) {
    let classification: Classification;
    try {
      classification = classifyRequire(id, deps);
    } catch {
      // Classification threw (blacklist / impossible / not-found). Skip baking —
      // the sync callback re-runs classifyRequire and throws the SAME guided
      // error at the require call site (attributed to the right line).
      dlog('prebundle', 'classify threw → defer to require site', { id });
      continue;
    }
    if ('settled' in classification) {
      dlog('prebundle', 'settled (bridge/iife)', { id, kind: classification.settled.kind });
      out.set(id, classification.settled);
      continue;
    }
    const pkgId = classification.needsBundle === 'source' ? classification.name : classification.pkgName;
    dlog('prebundle', 'bundling', { id, kind: classification.needsBundle });
    try {
      const code =
        classification.needsBundle === 'source'
          ? await deps.bundler.bundleSource(classification.name, classification.source, classification.resolveDir)
          : await deps.bundler.bundleEntry(id, classification.entryPath);
      dlog('prebundle', 'bundled ok', { id, bytes: code.length });
      out.set(id, { kind: 'source', code });
    } catch (err) {
      dlog('prebundle', 'bundle FAILED → impossible', { id, reason: classifyBundleFailure(err) });
      // esbuild could not bundle it (fail-and-map, ADR-010 §87). Classify WHY from
      // the esbuild failure (native `.node` addon → native_addon, else other) and
      // bake a typed `impossible` entry so the sync callback throws the guided
      // error with a precise `reason` for the Script Package Unsupported analytics
      // event — not a blanket `other`. The classification is the signal (not a
      // silent swallow, `gr-no-silent-catch`); the original esbuild error rides
      // through as `cause` when the error is constructed in `resolveRequire`.
      out.set(id, { kind: 'impossible', reason: classifyBundleFailure(err), packageId: pkgId });
    }
  }
  return out;
}

/**
 * The host-side bundle-require callback body (SYNC — it backs an `ivm.Callback`).
 * Reads the pre-built map first; for bridge/IIFE ids it can also resolve inline
 * (no async needed). Anything that would require a runtime bundle but isn't in
 * the map (e.g. a dynamic require the pre-pass couldn't statically see, or one
 * whose bundle failed) throws the guided IMPOSSIBLE error. NEVER bundles inline.
 */
export function resolveRequire(id: string, deps: BundleRequireDeps, prebuilt: PrebundledRequires): BundleRequireResult {
  const baked = prebuilt.get(id);
  if (baked !== undefined) {
    // A baked `impossible` entry means the pre-pass bundle failed and classified
    // the reason — throw the guided error with that precise reason (ADR-010 §87)
    // rather than returning it to the in-isolate require (which only handles
    // bridge/iife/source).
    if (baked.kind === 'impossible') {
      throw createImpossiblePackageError(baked.packageId, baked.reason);
    }
    return baked;
  }

  const classification = classifyRequire(id, deps);
  if ('settled' in classification) return classification.settled;

  // Needs a bundle but it isn't pre-built (not statically extracted, or its
  // bundle failed). Fail-and-map to the guided error rather than bundling inline
  // (buildSync would deadlock the worker — RQ-3359).
  const name = classification.needsBundle === 'source' ? classification.name : classification.pkgName;
  throw createImpossiblePackageError(name, 'other');
}

/**
 * Statically extract the require() ids in a source string — the input to the
 * pre-bundle pass. Matches `require('x')` / `require("x")` string literals,
 * including ones containing escaped quotes (`require('it\\'s')`) — the literal is
 * parsed in full and the escape sequences are unescaped so the returned id is the
 * runtime string value. Dynamic / template-literal requires (`require(expr)`,
 * `require(\`${x}\`)`) are intentionally NOT matched: they cannot be statically
 * pre-bundled and correctly fail-and-map at the call site (Safe-mode scripts use
 * literal requires; the build-time editor types only complete literals).
 */
export function extractRequireIds(source: string): string[] {
  const ids: string[] = [];
  // Match a quoted literal allowing backslash-escapes inside: `\\.` consumes any
  // escaped char (incl. an escaped quote), `[^'"\\]` consumes ordinary chars.
  const re = /\brequire\(\s*(['"])((?:\\.|[^\\])*?)\1\s*\)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(source)) !== null) {
    const raw = m[2];
    if (!raw) continue;
    // Unescape so the id is the runtime value (e.g. `\\'` → `'`), matching what
    // the in-isolate `require()` actually receives.
    ids.push(raw.replace(/\\(.)/g, '$1'));
  }
  return ids;
}

/**
 * In-isolate JS: builds `globalThis.require` on top of `__rq_bundleRequire`. Holds
 * an in-isolate module cache. Three result kinds, all eval'd/resolved INSIDE the
 * isolate (HARD INVARIANT — only host data strings cross in, never live objects):
 *   - `bridge` → return the named pre-installed in-isolate global (Buffer, …).
 *   - `iife`   → eval the build-time IIFE (assigns `var __name` on the isolate
 *                global) and return `globalThis[globalName]` (chai/uuid/…).
 *   - `source` → CommonJS-wrap + eval a runtime-bundled user package.
 */
export { REQUIRE_ISOLATE_SHIM } from './shims/require.shim.js';
