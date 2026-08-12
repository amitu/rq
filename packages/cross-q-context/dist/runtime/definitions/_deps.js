// The single dependency seam for the vendored rq.* API (ADR-213 Layer 2, step 3). Every file under
// `definitions/` imports what it used to pull from `@requestly/*` from HERE, so the rq.* API is
// self-contained. Most symbols re-export the contract/model/execution layers; a few rq.*-only leaf
// types (phase descriptors, visualizer, runRequest, the injected VariableResolver) are defined here.
export { ScriptExecutionMode, ScriptPhase } from '../contract.js';
export { AuthType, EntryType, GrpcMethodType, RawBodyContentType, RequestContentType, RequestMethod } from '../model.js';
import { ScriptPhase } from '../contract.js';
// ── phase descriptors (from shared-types/runtime) ───────────────────────────────────────────
export var ExecutionErrorPhase;
(function (ExecutionErrorPhase) {
    ExecutionErrorPhase["preparation"] = "preparation";
    ExecutionErrorPhase["preScript"] = "pre-script";
    ExecutionErrorPhase["request"] = "request";
    ExecutionErrorPhase["postScript"] = "post-script";
    ExecutionErrorPhase["onMessageScript"] = "on-message-script";
})(ExecutionErrorPhase || (ExecutionErrorPhase = {}));
export const PHASE_DESCRIPTORS = {
    [ScriptPhase.preRequest]: {
        scriptsField: 'preRequest',
        errorPhase: ExecutionErrorPhase.preScript,
        scriptFilename: 'pre-request-script.js',
        contextIdPrefix: 'pre',
        dtsBasename: 'pre-request',
        exclusiveSurface: ['visualizer'],
    },
    [ScriptPhase.postResponse]: {
        scriptsField: 'postResponse',
        errorPhase: ExecutionErrorPhase.postScript,
        scriptFilename: 'post-response-script.js',
        contextIdPrefix: 'post',
        dtsBasename: 'post-response',
        exclusiveSurface: ['response', 'visualizer'],
    },
    [ScriptPhase.onMessage]: {
        scriptsField: 'onMessage',
        errorPhase: ExecutionErrorPhase.onMessageScript,
        scriptFilename: 'on-message-script.js',
        contextIdPrefix: 'on-message',
        dtsBasename: 'on-message',
        exclusiveSurface: ['message'],
    },
};
