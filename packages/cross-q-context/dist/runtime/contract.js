// cross-q-context — the scripting runtime CONTRACT primitives (self-contained, ADR-213 Layer 2).
//
// The low-level, model-free primitives every host shares. The request/response DATA MODEL is in
// `model.ts`; the composed execution types (`ScriptExecutionInput`/`Context`, `Sandbox`) that key
// off both live in `execution.ts`. This file imports NOTHING — cross-q-context is self-contained in
// the `rq` repo with zero dependency on the current app.
/** Which script phase is running. `rq.response` is absent in `pre-request`; `on-message` runs per
 * inbound realtime message (WebSocket/Socket.IO/gRPC stream). */
export var ScriptPhase;
(function (ScriptPhase) {
    ScriptPhase["preRequest"] = "pre-request";
    ScriptPhase["postResponse"] = "post-response";
    ScriptPhase["onMessage"] = "on-message";
})(ScriptPhase || (ScriptPhase = {}));
/**
 * The sandbox engine. Published (WASM/browser) builds are `safe` only; an unrecognized value
 * resolves to `safe` (fail-closed). `developer` (`node:vm`) is a host-embedding, trusted-code
 * opt-in — never offered in a browser or a published build.
 */
export var ScriptExecutionMode;
(function (ScriptExecutionMode) {
    ScriptExecutionMode["safe"] = "safe";
    ScriptExecutionMode["developer"] = "developer";
})(ScriptExecutionMode || (ScriptExecutionMode = {}));
