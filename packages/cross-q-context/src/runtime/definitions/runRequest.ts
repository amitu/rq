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

import type {
  RunRequestDescriptor,
  RunRequestEnvelope,
  RunRequestErrorKind,
  SerializedSubResponse,
} from './_deps.js';

// ---------------------------------------------------------------------------
// Host round-trip
// ---------------------------------------------------------------------------

/** The host round-trip: takes the descriptor, runs the child off-process, returns the envelope. Injected per engine. */
export type RunRequestImpl = (descriptor: RunRequestDescriptor) => Promise<RunRequestEnvelope>;

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/** Options accepted by rq.execution.runRequest(id, opts?). */
export interface RunRequestOptions {
  variables?: Readonly<Record<string, string>>;
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/**
 * Response headers exposed to scripts. Mirrors `sendRequest`'s `ScriptHeaderList`:
 * a case-insensitive `get(name)` plus index access, so both
 * `res.headers.get('content-type')` and `res.headers['content-type']` work.
 */
export interface RunRequestHeaderList {
  get(name: string): string | undefined;
  [key: string]: string | ((name: string) => string | undefined) | undefined;
}

/** Postman-shaped response returned to the script (rehydrated from SerializedSubResponse). Mirrors sendRequest's response shape. */
export interface RunRequestResponse {
  /** Numeric HTTP status, e.g. 200. */
  code: number;
  /** HTTP status text, e.g. "OK". */
  status: string;
  headers: RunRequestHeaderList;
  /** Round-trip time in milliseconds. */
  responseTime: number;
  /** Parsed JSON body. Throws (SyntaxError) on a non-JSON body, like Postman. */
  json(): unknown;
  /** Raw response body as text. */
  text(): string;
}

/** The `rq.execution.runRequest` callable (promise-only). */
export type ScriptRunRequest = (requestId: string, opts?: RunRequestOptions) => Promise<RunRequestResponse>;

// ---------------------------------------------------------------------------
// Errors (kind-tagged — gr-discriminated-errors-at-boundaries)
// ---------------------------------------------------------------------------

/** Thrown when runRequest fails at the boundary — kind-tagged (gr-discriminated-errors-at-boundaries). */
export class RunRequestFailure extends Error {
  readonly kind = 'run-request-failure' as const;
  readonly reason: RunRequestErrorKind;

  constructor(reason: RunRequestErrorKind, message: string) {
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
function toHeaderList(headers: Readonly<Record<string, string>>): RunRequestHeaderList {
  const lookup: Record<string, string> = {};
  for (const [k, v] of Object.entries(headers)) {
    lookup[k.toLowerCase()] = v;
  }
  const list: RunRequestHeaderList = {
    get(name: string): string | undefined {
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
function rehydrate(r: SerializedSubResponse): RunRequestResponse {
  const headers = toHeaderList(r.headers);
  return {
    code: r.code,
    status: r.status,
    headers,
    responseTime: r.responseTime,
    // Lazy parse — throws SyntaxError on non-JSON, like Postman's / sendRequest's .json().
    json(): unknown {
      return JSON.parse(r.responseBody);
    },
    text(): string {
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
export function createRunRequest(impl: RunRequestImpl): ScriptRunRequest {
  let callCount = 0;
  return async function runRequest(requestId: string, opts?: RunRequestOptions): Promise<RunRequestResponse> {
    if (typeof requestId !== 'string' || requestId.length === 0) {
      throw new RunRequestFailure('invalid_argument', 'rq.execution.runRequest: a non-empty requestId is required.');
    }
    callCount += 1;
    if (callCount > MAX_RUN_REQUEST_CALLS) {
      // Static message (no interpolation) so gr-static-error-messages holds and
      // no new lint-disable is needed; the "10" matches MAX_RUN_REQUEST_CALLS.
      throw new RunRequestFailure('cap_exceeded', 'rq.execution.runRequest: exceeded the 10-call limit per script.');
    }
    const descriptor: RunRequestDescriptor = {
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
