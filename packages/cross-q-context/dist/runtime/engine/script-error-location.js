import { PHASE_DESCRIPTORS } from '../index.js';
/**
 * Source-location mapping for thrown user-script errors (RQ-4142).
 *
 * Both engines wrap the user script in an async IIFE whose body begins on a
 * fresh physical line (a leading newline after `{`/`try {`). That gives a
 * uniform contract: the user's line N lands on physical line N+1, with columns
 * unshifted. So every stack frame that references the script's synthetic
 * filename needs its line reduced by {@link WRAPPER_LINE_OFFSET} and nothing else.
 *
 * `parseScriptErrorLocation` turns the raw engine `.stack` into a
 * {@link ScriptErrorLocation}: the innermost user frame's line/column plus a
 * cleaned, offset-corrected stack string with the wrapper/host frames stripped.
 */
/**
 * Synthetic filename handed to a script compiler (`ctx.evalCode` /
 * `new vm.Script`) so thrown errors' stack frames anchor to the user's editor
 * instead of an anonymous eval realm. Phase-specific so the surfaced frame reads
 * `at post-response-script.js:3:1` — naming the script the user actually wrote,
 * not an internal `user-script.js` (RQ-4142).
 */
export function scriptFilenameForPhase(phase) {
    // The `switch` this replaces had a `default: 'user-script.js'`, so a new phase
    // silently reported frames against a filename the user never wrote.
    return PHASE_DESCRIPTORS[phase].scriptFilename;
}
/**
 * Physical→user line offset introduced by the async-IIFE wrapper's leading
 * newline. User line N sits on physical line N+1 in both engines, so we subtract
 * 1 from every reported line. Columns are unshifted (the newline resets them).
 */
export const WRAPPER_LINE_OFFSET = 1;
/** A stack line that is a frame (`    at ...`) rather than the header. */
const FRAME_LINE_RE = /^\s*at\s/;
/** Escape a string for literal use inside a RegExp. */
function escapeRegExp(value) {
    return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
/**
 * A thrown user-script error carrying its mapped source location, so an engine's
 * outer catch can surface the location on the result without re-deriving it.
 * Host-side only — never serialized (the runtime lifts the plain fields onto
 * `ScriptExecutionResult`/`EntryError` first).
 */
export class UserScriptError extends Error {
    errorLocation;
    constructor(message, errorLocation) {
        super(message);
        this.name = 'UserScriptError';
        this.errorLocation = errorLocation;
    }
}
/** Number of physical lines in the user's script — the frame-range upper bound. */
export function countScriptLines(script) {
    return script.split('\n').length;
}
/**
 * Parse an engine `.stack` into a {@link ScriptErrorLocation} anchored to the
 * user's script. Returns `undefined` when no frame maps to user code (e.g. an
 * error thrown entirely inside a host bridge, a timeout, or a `.stack` the engine
 * never populated) — the caller then surfaces the bare message, as before.
 *
 * @param rawStack       the thrown error's `.stack` (engine-native format)
 * @param scriptLineCount lines in the user script — frames outside `[1, n]` are
 *                        the wrapper's own (`})()` / `<eval>`) and are dropped
 * @param filename       the synthetic filename the script was compiled under
 *                        (see {@link scriptFilenameForPhase}); only frames
 *                        referencing it are kept
 * @param headerMessage  prepended as the stack header when the engine's stack has
 *                        none (QuickJS omits the `Name: message` line V8 includes)
 */
export function parseScriptErrorLocation(rawStack, scriptLineCount, filename, headerMessage) {
    if (rawStack === undefined || rawStack === '')
        return undefined;
    const frameRe = new RegExp(`${escapeRegExp(filename)}:(\\d+):(\\d+)`);
    const displayLines = [];
    let primary;
    for (const rawLine of rawStack.split('\n')) {
        const match = frameRe.exec(rawLine);
        if (match === null) {
            // Not a user-script frame. Keep genuine header lines (`TypeError: ...`);
            // drop foreign frames (node internals, host module) — they're noise.
            if (!FRAME_LINE_RE.test(rawLine) && rawLine.trim() !== '')
                displayLines.push(rawLine);
            continue;
        }
        const lineStr = match[1];
        const columnStr = match[2];
        if (lineStr === undefined || columnStr === undefined)
            continue;
        const line = parseInt(lineStr, 10) - WRAPPER_LINE_OFFSET;
        const column = parseInt(columnStr, 10);
        // Frames outside the user script are the async-IIFE wrapper's own — drop them.
        if (line < 1 || line > scriptLineCount)
            continue;
        if (primary === undefined)
            primary = { line, column };
        displayLines.push(rawLine.replace(frameRe, `${filename}:${line}:${column}`));
    }
    if (primary === undefined)
        return undefined;
    const hasHeader = displayLines.length > 0 && !FRAME_LINE_RE.test(displayLines[0] ?? '');
    if (!hasHeader && headerMessage !== undefined && headerMessage !== '') {
        displayLines.unshift(headerMessage);
    }
    return { line: primary.line, column: primary.column, stack: displayLines.join('\n') };
}
