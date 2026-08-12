/**
 * `rq.execution.runRequest()` — sandbox scripting surface (ADR-169, Postman
 * `pm.execution.runRequest` parity).
 *
 * Boundary-Protocol-Pure (ADR-169 Option B): the factory builds a serializable
 * `RunRequestDescriptor`, hands it to an injected host round-trip (`impl`), and
 * rehydrates the returned `RunRequestEnvelope` into a Postman-shaped response
 * (`.code`, `.status`, `.headers.get()`, `.json()`, `.text()`) — mirroring the
 * response shape `sendRequest.ts` exposes. Nothing live crosses the boundary;
 * the descriptor goes out, the envelope comes back.
 *
 * Promise-only (no callback form): Postman's `runRequest` returns a Promise.
 *
 * Error contract (gr-discriminated-errors-at-boundaries): a failure rejects with
 * a kind-tagged `RunRequestFailure` carrying the closed `RunRequestErrorKind`;
 * an HTTP 4xx/5xx is NOT a failure — the response is delivered like sendRequest.
 *
 * The factory takes its single dependency (`impl`) as a parameter — no globals —
 * so it stays platform-agnostic and unit-testable with a mock.
 */
// ---------------------------------------------------------------------------
// Errors (kind-tagged — gr-discriminated-errors-at-boundaries)
// ---------------------------------------------------------------------------
/** Thrown when runRequest fails at the boundary — kind-tagged (gr-discriminated-errors-at-boundaries). */
export class RunRequestFailure extends Error {
    kind = 'run-request-failure';
    reason;
    constructor(reason, message) {
        super(message);
        this.name = 'RunRequestFailure';
        this.reason = reason;
    }
}
// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------
/**
 * Max runRequest calls per script (Postman parity — 10).
 *
 * Same value (10), different scope from `MAX_RUN_REQUEST_CALLS_PER_ROOT` in
 * `modules/runtime/src/run-request/resolver.ts` — per-script (resets each phase)
 * vs per-root-chain (across the whole run). Defense-in-depth; do NOT dedupe into
 * one constant.
 */
export const MAX_RUN_REQUEST_CALLS = 10;
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
/**
 * Builds the Postman-shaped HeaderList from the serialized header record.
 * Mirrors `sendRequest.ts`'s `toHeaderList` (which folds a `Headers` instance);
 * here the source is already a plain record, so we fold it the same way —
 * lower-cased keys so `get()` and index access agree.
 */
function toHeaderList(headers) {
    const lookup = {};
    for (const [k, v] of Object.entries(headers)) {
        lookup[k.toLowerCase()] = v;
    }
    const list = {
        get(name) {
            return lookup[name.toLowerCase()];
        },
    };
    // Index access mirror — also lower-cased keys so [...] and get() agree.
    for (const [k, v] of Object.entries(lookup)) {
        list[k] = v;
    }
    return list;
}
/** Rehydrates the serialized child response into the Postman-shaped object. */
function rehydrate(r) {
    const headers = toHeaderList(r.headers);
    return {
        code: r.code,
        status: r.status,
        headers,
        responseTime: r.responseTime,
        // Lazy parse — throws SyntaxError on non-JSON, like Postman's / sendRequest's .json().
        json() {
            return JSON.parse(r.responseBody);
        },
        text() {
            return r.responseBody;
        },
    };
}
// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------
/**
 * Builds the `rq.execution.runRequest` callable. `impl` is the per-engine host
 * round-trip (injected by the engine that wires runRequest in). The call count
 * is tracked per factory instance (per-script), enforcing the Postman 10-call
 * cap.
 */
export function createRunRequest(impl) {
    let callCount = 0;
    return async function runRequest(requestId, opts) {
        if (typeof requestId !== 'string' || requestId.length === 0) {
            throw new RunRequestFailure('invalid_argument', 'rq.execution.runRequest: a non-empty requestId is required.');
        }
        callCount += 1;
        if (callCount > MAX_RUN_REQUEST_CALLS) {
            // Static message (no interpolation) so gr-static-error-messages holds and
            // no new lint-disable is needed; the "10" matches MAX_RUN_REQUEST_CALLS.
            throw new RunRequestFailure('cap_exceeded', 'rq.execution.runRequest: exceeded the 10-call limit per script.');
        }
        const descriptor = {
            requestId,
            ...(opts?.variables ? { variableOverrides: opts.variables } : {}),
            // Host stamps the authoritative depth; this is informational only (never trusted from the guest).
            depth: 0,
        };
        const envelope = await impl(descriptor);
        if (!envelope.ok) {
            throw new RunRequestFailure(envelope.error.kind, envelope.error.message);
        }
        return rehydrate(envelope.response);
    };
}
