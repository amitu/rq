import type { ExecutionDirective, RequestHeaderMutation, ScriptExecutionContext } from '../../index.js';
import type { TestResult } from '../host-types.js';
import type { VisualizerDirective } from '../../definitions/_deps.js';
import type { ScriptPhase } from '../../index.js';
import type { RawScopeMutations } from '../../index.js';
/**
 * Typed mutable VM scope state. All mutable outputs that scripts write to
 * during execution live in this single object.
 */
export interface ExecutionState {
    testResults: TestResult[];
    /** Raw variable mutations captured by scope methods (ADR-053 Layer 1). */
    rawMutations: RawScopeMutations;
    /** Request header mutations captured by `rq.request.headers.*` (ADR-167). */
    requestMutations: RequestHeaderMutation[];
    /** Flow-control directive captured by `rq.execution.setNextRequest` / `skipRequest` (ADR-169). */
    executionDirective?: ExecutionDirective;
    /** Visualizer intent captured by `rq.visualizer.set()` / `clear()` (ADR-202, FR-18). */
    visualizerOutput?: VisualizerDirective;
}
/**
 * Pure input to builder functions. Carries everything the builder needs
 * to construct the VM context and rq namespace.
 */
export interface SandboxBuildContext {
    context: ScriptExecutionContext;
    phase: ScriptPhase;
    vmRealm: {
        chai: {
            expect: unknown;
        };
    };
    host: Record<string, unknown>;
}
