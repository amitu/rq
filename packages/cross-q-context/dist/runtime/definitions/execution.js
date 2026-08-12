/**
 * `rq.execution` — sandbox scripting surface for collection-runner flow control
 * (ADR-169, Postman `pm.execution` parity).
 *
 * The namespace is a pure collector: `setNextRequest` / `skipRequest` write a
 * single discriminated `ExecutionDirective` onto an injected collector object,
 * which the engine drains after the script settles (mirrors the request-mutation
 * collector pattern in `rqMethods.ts`, ADR-167). `location` is a read-only view
 * of the current execution path.
 *
 * Phase gating (Postman parity): `setNextRequest` and `location` are present in
 * both phases; `skipRequest` is present ONLY in the pre-request phase — calling
 * it in post-response throws the same `TypeError` Postman gives
 * ("skipRequest is not a function").
 *
 * The factory takes its dependencies as parameters — no `declare const`, no
 * globals — so it stays platform-agnostic and unit-testable.
 */
import { ScriptPhase } from './_deps.js';
import { createRunRequest } from './runRequest.js';
/** Thrown by skipRequest() to abort the remaining pre-request script (Postman parity). Caught by the engine. */
export class SkipRequestSignal extends Error {
    kind = 'skip-request-signal';
    constructor() {
        super('rq.execution.skipRequest()');
        this.name = 'SkipRequestSignal';
    }
}
/**
 * Builds `rq.execution` (ADR-169). `setNextRequest` and `location` are always
 * present; `skipRequest` is present ONLY in the pre-request phase (Postman: it
 * is `is not a function` in post-response). `runRequest` is present in BOTH
 * phases — but ONLY when the engine supplies a `runRequestImpl` (engines that
 * don't wire it leave `rq.execution.runRequest` absent). The directive is
 * written onto the injected collector, drained by the engine after the script
 * settles.
 */
export function createExecutionNamespace(collector, phase, location, runRequestImpl) {
    const current = location.length > 0 ? location[location.length - 1] : undefined;
    // `location` is a real array carrying an extra read-only `.current` accessor,
    // then frozen. A plain array (so `Array.isArray` stays true, matching Postman)
    // intersected with `{ current }` is structurally a `ScriptExecutionLocation`,
    // so the annotation needs no cast (gr-no-unsafe-cast). Freezing in place keeps
    // the same object identity the type was inferred against.
    const locationArr = Object.assign([...location], { current });
    Object.freeze(locationArr);
    const ns = {
        setNextRequest(nameOrNull) {
            collector.directive = { kind: 'set-next-request', target: nameOrNull };
        },
        location: locationArr,
    };
    // runRequest: present in both phases (unlike skipRequest), but only when the
    // engine injects a host round-trip. Absent otherwise so engines that don't
    // wire it leave `rq.execution.runRequest` undefined.
    if (runRequestImpl) {
        ns.runRequest = createRunRequest(runRequestImpl);
    }
    // skipRequest: pre-request only. Sets the directive then throws the signal so
    // the rest of the pre-request script does not run (Postman parity). Absent in
    // post-response so calling it throws the SAME TypeError Postman gives
    // ("skipRequest is not a function").
    if (phase === ScriptPhase.preRequest) {
        return Object.assign(ns, {
            skipRequest() {
                collector.directive = { kind: 'skip-request' };
                throw new SkipRequestSignal();
            },
        });
    }
    return ns;
}
