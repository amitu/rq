/**
 * on-message-batch — the parts of the on-message batch loop that are the SAME in
 * both engines (ADR-208 §7, runtime 021 §Decision).
 *
 * The loop itself is necessarily engine-specific: one drives `ctx.evalCode` against
 * a QuickJS guest, the other `vm.Script.runInContext` against a `node:vm` realm.
 * What must NOT be engine-specific is the shape of what a batch produces and the
 * rules for reporting it — `messageIndex` stamping, the static timeout message, and
 * the "a failed message does not fail the run" rule. Those live here so the two
 * loops cannot drift on them, which is the Equivalence obligation's other half:
 * one `execute()` over K messages is indistinguishable from K executions, in
 * EITHER engine.
 */
import type { CookieJarMutation, MutationDiff, RequestHeaderMutation, ScriptExecutionResult, ScriptMessageError, TestResult } from '../index.js';
import type { RawScopeMutations } from '../index.js';
/**
 * Static message for a per-message deadline overrun (`gr-static-error-messages`).
 * Both engines report the same string; the message index is what varies, and it
 * rides the structured `ScriptMessageError` rather than the message text.
 */
export declare const ON_MESSAGE_TIMEOUT_ERROR = "On-message script exceeded the per-message execution timeout";
/**
 * Host-side `messageIndex` stamping (ADR-208 §9). Deliberately host-side: the
 * guest/vm realm producing the result is never trusted to remember which iteration
 * it is in, so a shim that forgot to stamp cannot silently un-correlate a whole
 * batch's assertions.
 */
export declare function stampMessageIndex(result: TestResult, messageIndex: number): TestResult;
/** What one on-message batch produced, accumulated host-side across its iterations. */
export interface BatchOutcome {
    readonly testResults: TestResult[];
    /**
     * Raw mutations for the batch. Mutations accumulate across iterations (in-guest
     * for the Safe engine, in `ExecutionState` for the Developer engine) and are
     * inflated ONCE, per ADR-208 §6's accumulate-and-emit-once contract.
     */
    mutations: RawScopeMutations | undefined;
    readonly requestMutations: RequestHeaderMutation[];
    readonly messageErrors: ScriptMessageError[];
    /**
     * How many messages reached an iteration boundary. The runtime re-queues
     * `batch.slice(messagesCompleted)`, so this is what makes an abandoned batch a
     * throughput cost rather than a coverage hole.
     */
    messagesCompleted: number;
    /**
     * Safe engine only: an iteration was interrupt-killed, so the QuickJS runtime is
     * unusable and must be leaked rather than disposed. The Developer engine leaves
     * this false — `node:vm` cannot interrupt guest code at all, which is the known
     * limitation recorded in runtime 021 §Per-message deadline rather than a state
     * this flag can represent.
     */
    killedByTimeout: boolean;
}
export declare function createBatchOutcome(): BatchOutcome;
/**
 * Assemble a batch's `ScriptExecutionResult`.
 *
 * `error` is deliberately NEVER set here. A message whose script threw is reported
 * on `messageErrors`, because `error` fails the entry and message 7 throwing must
 * not fail a connection that delivered 1-6 and 8-10 (ADR-208 §9). The one failure
 * that DOES fail the whole run — the script not compiling — never reaches this
 * function: no message ran, so there is no per-message granularity to report it at.
 */
export declare function buildBatchResult(outcome: BatchOutcome, mutationDiff: MutationDiff, cookieMutations: readonly CookieJarMutation[]): ScriptExecutionResult;
