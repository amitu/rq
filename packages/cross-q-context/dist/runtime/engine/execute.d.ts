import type { ScriptExecutionResult, SendRequestFn } from './host-types.js';
import type { ScriptExecutionContext } from '../execution.js';
/** What one execute call needs: the (transformed) script, its phase, and the marshalled context. */
export interface ExecuteScriptInput {
    script: string;
    phase: string;
    context: ScriptExecutionContext;
    timeoutMs?: number;
    /** Host fetch backing rq.sendRequest / the guest `fetch`. Omit and those APIs are unavailable. */
    sendRequest?: SendRequestFn;
}
/**
 * Run a (transformed) rq.* script safely in QuickJS and return its result. Self-contained: the OSS
 * caller supplies the script + a ScriptExecutionContext and gets back mutations / tests / logs.
 */
export declare function executeScript(input: ExecuteScriptInput): Promise<ScriptExecutionResult>;
