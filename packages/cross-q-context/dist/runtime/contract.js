// cross-q-context — the scripting runtime CONTRACT (self-contained, ADR-213 Layer 2 migration).
//
// This is the canonical, dependency-free contract for executing a script: a pure function of a
// serializable input to a serializable output, so every host (a browser tab, a Node worker, a CLI,
// a future `rq` app) speaks the same `execute(input) → result`. It deliberately imports NOTHING —
// cross-q-context must be self-contained in the `rq` repo with zero dependency on the current app.
//
// Requestly's app currently expresses this contract inside `@requestly/shared-types` (woven through
// its `common`/`runtime` type graph). As the runtime migrates here, the app becomes a CONSUMER: it
// maps its richer internal types onto this contract at the seam. This file is the source of truth;
// extra app-only channels (cookies, visualizer, packages, on-message) layer on top without changing
// the core shape.
/** Which script phase is running. `rq.response` is absent in `pre-request`. */
export var ScriptPhase;
(function (ScriptPhase) {
    ScriptPhase["preRequest"] = "pre-request";
    ScriptPhase["postResponse"] = "post-response";
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
