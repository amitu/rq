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
/**
 * Static message for a per-message deadline overrun (`gr-static-error-messages`).
 * Both engines report the same string; the message index is what varies, and it
 * rides the structured `ScriptMessageError` rather than the message text.
 */
export const ON_MESSAGE_TIMEOUT_ERROR = 'On-message script exceeded the per-message execution timeout';
/**
 * Host-side `messageIndex` stamping (ADR-208 §9). Deliberately host-side: the
 * guest/vm realm producing the result is never trusted to remember which iteration
 * it is in, so a shim that forgot to stamp cannot silently un-correlate a whole
 * batch's assertions.
 */
export function stampMessageIndex(result, messageIndex) {
    return { ...result, messageIndex };
}
export function createBatchOutcome() {
    return {
        testResults: [],
        mutations: undefined,
        requestMutations: [],
        messageErrors: [],
        messagesCompleted: 0,
        killedByTimeout: false,
    };
}
/**
 * Assemble a batch's `ScriptExecutionResult`.
 *
 * `error` is deliberately NEVER set here. A message whose script threw is reported
 * on `messageErrors`, because `error` fails the entry and message 7 throwing must
 * not fail a connection that delivered 1-6 and 8-10 (ADR-208 §9). The one failure
 * that DOES fail the whole run — the script not compiling — never reaches this
 * function: no message ran, so there is no per-message granularity to report it at.
 */
export function buildBatchResult(outcome, mutationDiff, cookieMutations) {
    return {
        mutationDiff,
        logs: [],
        testResults: outcome.testResults,
        ...(cookieMutations.length > 0 ? { cookieMutations } : {}),
        ...(outcome.requestMutations.length > 0 ? { requestMutationDiff: { headers: outcome.requestMutations } } : {}),
        ...(outcome.messageErrors.length > 0 ? { messageErrors: outcome.messageErrors } : {}),
        messagesCompleted: outcome.messagesCompleted,
        // Forwarded, not dropped: the killed message sits OUTSIDE `messagesCompleted` (it
        // never reached a boundary) yet must not be retried, and this flag is the only thing
        // that distinguishes those two states for the drain. Discarding it here is what let
        // a runaway script's head message be re-queued forever.
        ...(outcome.killedByTimeout ? { killedByTimeout: true } : {}),
    };
}
