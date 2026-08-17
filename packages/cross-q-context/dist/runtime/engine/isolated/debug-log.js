/**
 * debug-log — opt-in, zero-noise tracing for the QuickJS Safe-mode engine (RQ-3359).
 *
 * Gated behind the `RQ_SANDBOX_DEBUG` env var so it costs nothing in normal runs.
 * Set `RQ_SANDBOX_DEBUG=1` in the desktop sandbox worker's environment to emit a
 * timestamped, sequence-numbered trace of EVERY critical step (module load,
 * runtime/context lifecycle, each shim eval, the run loop, the async bridge drive,
 * teardown, and per-bridge calls) to stderr — so a hang/stall is pinpointed to the
 * exact step in the live worker logs, not guessed at.
 *
 * Lines look like:  [rq-sandbox 0007 +12ms] engine: shim:crypto evaled ok
 * - `0007` is a process-monotonic sequence (ordering survives interleaving).
 * - `+12ms` is the delta since the previous log line (spikes show where time goes).
 *
 * Deliberately writes to stderr via `process.stderr.write` (not `console.*`, which
 * the sandbox worker may have repurposed) and never throws — a logging failure
 * must never affect execution.
 *
 * `process` is reached through a guarded `globalThis` lookup rather than named
 * directly, because this package is platform-neutral (ADR-217) and compiles with
 * no Node typings: naming it would not type-check, and a host that has no
 * `process` — a browser — must degrade to a silent no-op rather than throw. The
 * two readers below narrow `unknown` at runtime instead of asserting a shape
 * (`gr-no-unsafe-cast`, `gr-parse-at-boundaries`) — `globalThis` genuinely is a
 * boundary here, since what lives on it is exactly what this package refuses to
 * assume.
 */
/** `process.env[name]`, or undefined on a host that has neither. Never throws. */
function readHostEnv(name) {
    const proc = Reflect.get(globalThis, 'process');
    if (typeof proc !== 'object' || proc === null)
        return undefined;
    const env = Reflect.get(proc, 'env');
    if (typeof env !== 'object' || env === null)
        return undefined;
    const value = Reflect.get(env, name);
    return typeof value === 'string' ? value : undefined;
}
/** `process.stderr.write`, or undefined on a host without one. Never throws. */
function hostStderrWrite() {
    const proc = Reflect.get(globalThis, 'process');
    if (typeof proc !== 'object' || proc === null)
        return undefined;
    const stderr = Reflect.get(proc, 'stderr');
    if (typeof stderr !== 'object' || stderr === null)
        return undefined;
    const write = Reflect.get(stderr, 'write');
    return typeof write === 'function' ? (chunk) => Reflect.apply(write, stderr, [chunk]) : undefined;
}
const DEBUG_FLAG = readHostEnv('RQ_SANDBOX_DEBUG');
const ENABLED = DEBUG_FLAG === '1' || DEBUG_FLAG === 'true';
let seq = 0;
let lastTs = 0;
/** Emit one trace line if debugging is enabled. Never throws. */
export function dlog(area, message, extra) {
    if (!ENABLED)
        return;
    try {
        const now = Date.now();
        const delta = lastTs === 0 ? 0 : now - lastTs;
        lastTs = now;
        seq += 1;
        const seqStr = String(seq).padStart(4, '0');
        const extraStr = extra ? ' ' + safeJson(extra) : '';
        hostStderrWrite()?.(`[rq-sandbox ${seqStr} +${delta}ms] ${area}: ${message}${extraStr}\n`);
    }
    catch {
        // Logging must never affect execution.
    }
}
/** Whether tracing is on (lets hot paths skip building log payloads entirely). */
export function isDebugEnabled() {
    return ENABLED;
}
/** JSON.stringify that never throws and bounds the output size. */
function safeJson(value) {
    try {
        const s = JSON.stringify(value, (_k, v) => (typeof v === 'bigint' ? v.toString() : v));
        return s.length > 300 ? s.slice(0, 300) + '…' : s;
    }
    catch {
        return '<unserializable>';
    }
}
