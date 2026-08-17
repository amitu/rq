import type { ScriptExecutionResult } from './host-types.js';
import type { ScriptExecutionContext } from '../execution.js';
/** What one execute call needs: the (transformed) script, its phase, and the marshalled context. */
export interface ExecuteScriptInput {
    script: string;
    phase: string;
    context: ScriptExecutionContext;
    timeoutMs?: number;
}
/**
 * Run a (transformed) rq.* script safely in QuickJS and return its result. Self-contained: the OSS
 * caller supplies the script + a ScriptExecutionContext and gets back mutations / tests / logs.
 */
export declare function executeScript(input: ExecuteScriptInput): Promise<ScriptExecutionResult>;
