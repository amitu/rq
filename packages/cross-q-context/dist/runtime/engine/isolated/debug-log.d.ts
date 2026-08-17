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
/** Emit one trace line if debugging is enabled. Never throws. */
export declare function dlog(area: string, message: string, extra?: Record<string, unknown>): void;
/** Whether tracing is on (lets hot paths skip building log payloads entirely). */
export declare function isDebugEnabled(): boolean;
