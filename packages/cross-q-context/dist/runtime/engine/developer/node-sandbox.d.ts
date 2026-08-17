/**
 * NodeSandbox — Isolated script execution via node:vm
 *
 * Executes user scripts inside vm.createContext for isolation, streams logs
 * in real-time via StreamHandle, and enforces timeout via Promise.race.
 *
 * This is the core sandbox implementation extracted from desktop's SandboxService.
 * It has no RPC coupling — consumers (desktop, CLI, API server) add their own
 * transport wiring.
 */
import type { SsrfPolicy } from '../ssrf-guard.js';
import type { FeatureFlags, ScriptExecutionInput, StreamReader } from '../../index.js';
import type { Sandbox, SandboxExecutionEvent } from '../host-types.js';
import type { PackageResolver } from '../../index.js';
import type { SandboxHostCallbacks } from '../../index.js';
/**
 * Node.js sandbox execution engine.
 * Each execute() call creates a fresh vm context and runs the user script.
 * Timeout is enforced via Promise.race on input.timeoutMs.
 */
export declare class NodeSandbox implements Sandbox {
    private readonly resolver;
    private readonly guardedFetch;
    constructor(resolver?: PackageResolver, options?: {
        readonly ssrfPolicy?: SsrfPolicy;
    });
    getFeatures(): Promise<FeatureFlags>;
    execute(input: ScriptExecutionInput, hostCallbacks?: SandboxHostCallbacks): Promise<StreamReader<SandboxExecutionEvent>>;
    private runScript;
    /**
     * Run an on-message batch: one iteration per message, driven from the host
     * (ADR-208 §7, runtime 021 §Decision).
     *
     * The four obligations, and where each is discharged:
     *
     * - **Ordering** — a single sequential loop over the batch, awaited per element.
     * - **Coverage** — exactly one iteration per element; a throw is caught, recorded
     *   against its message, and the loop continues, so one message's failure cannot
     *   skip another.
     * - **Isolation** — a `try`/`catch` around each run, plus a reset of the
     *   per-iteration collectors at each boundary.
     * - **Equivalence** — everything that varies between iterations is `rq.message`
     *   and the re-armed budget; `messageIndex` is stamped host-side, in the shared
     *   helper both engines use.
     *
     * **This engine has no working per-message deadline, and that is a known
     * limitation rather than a tuning detail** (runtime 021 §Per-message deadline
     * AMENDMENT). `node:vm` cannot pre-empt CPU-bound guest code at all: a macrotask
     * timer cannot fire while the guest holds the thread, so a runaway iteration runs
     * unbounded and is reported as success. The per-message budget below is therefore
     * real only for iterations that yield (an `await`), and the batch bound catches an
     * overrun only once the iteration has finished on its own. Safe mode is the
     * default; closing this needs an engine change, not a test.
     */
    private runMessageBatch;
}
