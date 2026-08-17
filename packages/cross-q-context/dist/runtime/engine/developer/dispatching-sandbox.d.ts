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
import type { NodeSandbox } from './node-sandbox.js';
import type { BundleCache } from '../isolated/source-bundler.js';
import type { SafePackageResolver } from '../../index.js';
import type { SandboxHostCallbacks, ScriptExecutionInput, StreamReader } from '../../index.js';
import type { Sandbox, SandboxExecutionEvent } from '../host-types.js';
export declare class DispatchingSandbox implements Sandbox {
    private readonly developerEngine;
    private readonly resolver?;
    private readonly bundleCache?;
    constructor(developerEngine: NodeSandbox, resolver?: SafePackageResolver | undefined, bundleCache?: BundleCache | undefined);
    getFeatures(): ReturnType<Sandbox['getFeatures']>;
    execute(input: ScriptExecutionInput, hostCallbacks?: SandboxHostCallbacks): Promise<StreamReader<SandboxExecutionEvent>>;
}
