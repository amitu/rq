/**
 * DispatchingSandbox — the single dispatch point that selects the script engine
 * per execution (sandbox-node ADR-008).
 *
 * Two engines coexist behind the one `Sandbox` interface: the QuickJS-WASM Safe
 * engine (`QuickJsSandbox`, ADR-012 — the default) and the retained `node:vm`
 * Developer engine (`NodeSandbox`, ADR-001). This thin dispatcher reads the
 * per-execution `mode` flag (ADR-009) at exactly one place — satisfying Risk R-4
 * (Developer can never silently fall back to Safe, and vice-versa) — and routes:
 *
 *   - `ScriptExecutionMode.developer` → the injected `NodeSandbox`
 *   - everything else (`default`) → the isolated Safe engine
 *
 * The `default` arm is deliberately FAIL-CLOSED (a considered departure from the
 * repo's `assertNever`-in-default norm, ADR-009): the explicit `safe` value AND
 * any unrecognized value (a corrupt or future-unknown mode that slips a boundary)
 * both resolve to the MORE contained engine — never to host access, never a throw.
 *
 * Engine wiring: the Developer engine is INJECTED (constructor dependency), and
 * the Safe engine is reached through a lazy
 * `await import('../quickjs-sandbox.js')` inside the Safe arm — so the
 * Safe engine + its QuickJS deps load only when Safe mode actually runs, not at
 * dispatcher construction. (Under isolated-vm this lazy import also kept the native
 * addon off the CLI static graph; with QuickJS-WASM that quarantine is moot — the
 * engine inlines into any bundle — but the lazy import is retained as a load-cost
 * optimization. ADR-008/012.)
 */
import { ScriptExecutionMode } from '../../index.js';
import { dlog } from '../index.js';
/**
 * Module-level cache for the resolved isolated Safe engine. Resolved once per
 * process on the first Safe execution and shared across every dispatcher
 * instance (the desktop runtime may construct more than one dispatcher — e.g.
 * per-collection / parallel runner runs). Caching at module scope pays the
 * dynamic-import cost once regardless of instance count (ADR-008 plan-review
 * resolution).
 */
let cachedSafeEngine;
async function resolveSafeEngine(resolver, bundleCache) {
    if (cachedSafeEngine !== undefined) {
        dlog('dispatch', 'reusing cached Safe engine');
        return cachedSafeEngine;
    }
    // Lazy import of the Safe-engine subpath (ADR-012). The engine is now WASM —
    // fully bundled into the consumer (no runtime native-addon resolution); the lazy
    // import defers loading it (and its QuickJS deps) until the first Safe execution.
    dlog('dispatch', 'lazy-importing Safe engine (first Safe run)');
    const { QuickJsSandbox } = await import('../quickjs-sandbox.js');
    // The resolver lets the Safe engine resolve user-INSTALLED npm packages to
    // source and bundle them (ADR-010 §85, Tier 4); the bundleCache persists
    // SOURCE_BUNDLE output across executions (ADR-010 §78). Both are SAME single
    // per-process dependencies the dispatcher was constructed with; the
    // per-execution contextId is threaded through `input`, not baked here, so
    // caching the engine is sound.
    //
    // `cachedSafeEngine` binds to the FIRST call's args. This is sound because the
    // dispatcher is constructed exactly ONCE per process (desktop: a single
    // module-scope `new DispatchingSandbox(...)` in SandboxService.ts), so there is
    // only ever one (resolver, bundleCache) pair. A test that constructs multiple
    // dispatchers with different caches must reset this module (or assert via a
    // fresh `QuickJsSandbox` directly) rather than relying on the cache.
    const engine = new QuickJsSandbox(resolver, bundleCache);
    cachedSafeEngine = engine;
    return engine;
}
export class DispatchingSandbox {
    developerEngine;
    resolver;
    bundleCache;
    // The Developer engine is injected, never constructed here — this keeps the
    // dispatcher engine-agnostic and means a `NodeSandbox`-only construction never
    // names the isolated module in its static import graph. The optional resolver
    // is forwarded to the lazily-constructed Safe engine for installed-package
    // resolution (ADR-010 §85); pass the SAME resolver the Developer engine got.
    constructor(developerEngine, resolver, bundleCache) {
        this.developerEngine = developerEngine;
        this.resolver = resolver;
        this.bundleCache = bundleCache;
    }
    getFeatures() {
        // Capabilities are reported by the Developer engine; the Safe engine reports
        // parity capabilities (both implement the same contract). Mode-specific
        // feature negotiation is not in Slice 1's scope.
        return this.developerEngine.getFeatures();
    }
    execute(input, hostCallbacks) {
        dlog('dispatch', 'execute', { mode: input.mode, entryId: input.entryId });
        switch (input.mode) {
            case ScriptExecutionMode.developer:
                return this.developerEngine.execute(input, hostCallbacks);
            // Fail-closed: `safe` AND any unrecognized value run the isolated engine.
            // The Safe engine accepts-but-ignores hostCallbacks for now; real bridge
            // wiring is task 2.5 (ADR-169 §Safe-mode containment).
            default:
                return resolveSafeEngine(this.resolver, this.bundleCache).then((engine) => engine.execute(input, hostCallbacks));
        }
    }
}
