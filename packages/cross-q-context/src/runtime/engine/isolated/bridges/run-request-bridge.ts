/**
 * run-request-bridge — Safe-mode `rq.execution.runRequest` (ADR-169).
 *
 * The isolate cannot reach the host runner. This bridge exposes the
 * `runRequest` capability as an ASYNC copy-in/copy-out host callback: the guest
 * hands a serialized `RunRequestDescriptor` (plain JSON), the host runs the
 * child request's full pipeline OFF-PROCESS via the injected `RunRequestHost`,
 * and returns a serialized `RunRequestEnvelope` (plain JSON). Mirrors the fetch
 * bridge exactly — a serialized request in, a serialized result out.
 *
 * HARD INVARIANT (RQ-2489 stays closed — ADR-169 §Safe-mode containment): only
 * copied data crosses. The handler receives a `RunRequestDescriptor` and returns
 * a `RunRequestEnvelope`; the live `RunRequestHost` stays host-side and no
 * reference (function / Response / RunRequestHost) ever enters the QuickJS realm.
 * `createSafeBridge`'s `Copyable` type constraint makes a non-copied handler a
 * COMPILE ERROR, and the paired `*.containment.test.ts` proves no host realm is
 * reachable through the bridge. Identical containment to the fetch bridge.
 *
 * The handler's I/O is typed with LOCAL `type` mirrors of the boundary shapes
 * (`RunRequestDescriptorData` / `RunRequestEnvelopeData`) — exactly as the fetch
 * bridge declares `FetchRequest`/`FetchResult` locally. This is required because
 * `createSafeBridge`'s `Copyable` constraint is `{ readonly [key: string]: Copyable }`,
 * which a `type` alias structurally satisfies but an `interface` (the form
 * `RunRequestDescriptor`/`RunRequestEnvelope` take in shared-types) does NOT —
 * interfaces lack the implicit index signature. The local types are STRUCTURALLY
 * identical to the imported interfaces (every field genuinely `Copyable`; the
 * shared-types `run-request.contract.ts` proves `Serializable<RunRequestDescriptor>`
 * equals itself), and the two are assignable both ways with NO cast. So the
 * `Copyable` guarantee is fully preserved — this is a type-shape adapter, not a
 * widening of the safety contract.
 */

import { dlog } from '../debug-log.js';
import { createSafeBridge } from '../safe-bridge-factory.js';

import type { SafeBridge } from '../safe-bridge-factory.js';
import type {
  RunRequestDescriptor,
  RunRequestEnvelope,
  RunRequestErrorKind,
  RunRequestHost,
} from '../../../index.js';

/**
 * Local `type` mirror of `RunRequestDescriptor` (the copy-in shape). A `type`
 * alias structurally satisfies the factory's `Copyable` index-signature
 * constraint where the equivalent `interface` cannot; assignable both ways with
 * the shared-types `RunRequestDescriptor` interface (proven, no cast).
 */
type RunRequestDescriptorData = {
  readonly requestId: string;
  readonly variableOverrides?: Readonly<Record<string, string>>;
  readonly depth: number;
};

/** Local `type` mirror of `SerializedSubResponse` (inside the success envelope). */
type SerializedSubResponseData = {
  readonly code: number;
  readonly status: string;
  readonly headers: Readonly<Record<string, string>>;
  readonly responseBody: string;
  readonly responseTime: number;
};

/** Local `type` mirror of `RunRequestEnvelope` (the copy-out shape). */
type RunRequestEnvelopeData =
  | { readonly ok: true; readonly response: SerializedSubResponseData }
  | { readonly ok: false; readonly error: { readonly kind: RunRequestErrorKind; readonly message: string } };

/**
 * The host-side async runRequest bridge installed as `__rq_runRequest`. Unlike
 * `createFetchBridge` (which calls global `fetch`), this bridge is CONSTRUCTED
 * with the injected `RunRequestHost`: the handler closes over `host` and calls
 * `host.runRequest(descriptor)` host-side. The descriptor in and the envelope
 * out are both `Copyable` (plain JSON), so only copied data crosses the edge —
 * the live `RunRequestHost` never enters the guest.
 */
export function createRunRequestBridge(host: RunRequestHost): SafeBridge {
  // Copyable-in / Copyable-out: descriptor → envelope. createSafeBridge's type
  // constraint makes a non-Copyable handler a COMPILE ERROR (the HARD INVARIANT).
  const handler = async (descriptor: RunRequestDescriptorData): Promise<RunRequestEnvelopeData> => {
    dlog('runRequest', 'host runRequest START', { requestId: descriptor.requestId, depth: descriptor.depth });
    try {
      // The local descriptor type is assignable to the shared-types interface
      // (structurally identical) — no cast. The host returns the interface
      // envelope, assignable back to the local copy-out type — no cast.
      const descriptorForHost: RunRequestDescriptor = descriptor;
      const envelope: RunRequestEnvelope = await host.runRequest(descriptorForHost);
      dlog('runRequest', 'host runRequest result', { ok: envelope.ok });
      return envelope;
    } catch (e) {
      dlog('runRequest', 'host runRequest THREW', { msg: e instanceof Error ? e.message.slice(0, 100) : String(e) });
      throw e;
    }
  };
  return createSafeBridge('__rq_runRequest', handler, { async: true });
}

/**
 * In-isolate JS: attach `rq.execution.runRequest` over `__rq_runRequest`, building
 * the Postman-shaped response (`.code`/`.status`/`.headers.get()`/`.json()`/`.text()`)
 * from the copied envelope. A hand-written mirror of `createRunRequest`
 * (`@requestly/sandbox-definitions`) — the same 10-call cap, the same response
 * rehydration, the same kind-tagged errors. Parity is by construction +
 * enforced by the containment/parity test (it cannot import the TS factory, like
 * FETCH_ISOLATE_SHIM). Eval'd AFTER the rq shim so `globalThis.rq.execution`
 * already exists (it coexists with setNextRequest/skipRequest/location).
 */
export const RUN_REQUEST_ISOLATE_SHIM = `
(() => {
  const call = globalThis.__rq_runRequest;
  if (!call || !globalThis.rq || !globalThis.rq.execution) return;
  let callCount = 0;
  globalThis.rq.execution.runRequest = async (requestId, opts) => {
    if (typeof requestId !== 'string' || requestId.length === 0) {
      const e = new Error('rq.execution.runRequest: a non-empty requestId is required.'); e.kind = 'invalid_argument'; throw e;
    }
    callCount += 1;
    // The literal 10 is MAX_RUN_REQUEST_CALLS (@requestly/sandbox-definitions/runRequest) —
    // hand-inlined here because this shim is an in-isolate JS string that cannot import the
    // TS factory (same constraint as FETCH_ISOLATE_SHIM). Keep in lockstep with that constant.
    if (callCount > 10) { const e = new Error('rq.execution.runRequest: exceeded the 10-call limit per script.'); e.kind = 'cap_exceeded'; throw e; }
    const descriptor = { requestId, depth: 0 };
    if (opts && opts.variables) descriptor.variableOverrides = opts.variables;
    const envelope = await call(descriptor);
    if (!envelope.ok) { const e = new Error(envelope.error.message); e.kind = envelope.error.kind; throw e; }
    const r = envelope.response;
    const lookup = {}; for (const k of Object.keys(r.headers)) lookup[k.toLowerCase()] = r.headers[k];
    return {
      code: r.code, status: r.status, responseTime: r.responseTime,
      headers: Object.assign({ get: (n) => lookup[String(n).toLowerCase()] }, lookup),
      json: () => JSON.parse(r.responseBody), text: () => r.responseBody,
    };
  };
})();
`;
