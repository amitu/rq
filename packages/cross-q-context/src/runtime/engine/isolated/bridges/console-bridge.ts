/**
 * console-bridge — Safe-mode `console` streaming (NEEDS_BRIDGE, ADR-010 §34/§83).
 *
 * The console bridge goes through the SAME typed factory + lint rule + containment
 * test discipline as every other bridge — the HARD INVARIANT applies to EVERY
 * bridge (ADR-010, marshalled via the QuickJS factory per ADR-012).
 *
 * Behavior: the guest serializes its args to a JSON
 * string in-isolate (String() fallback for BigInt/circular), then calls the
 * fire-and-forget host callback with `(level, argsJson)`. The host parses the
 * string back into `LogEntry.args` and pushes a `log` event. Only copied
 * strings cross the edge — no live host reference, no value/exception returns.
 */

import { LogLevel } from '../../../index.js';

import { createIgnoredBridge } from '../safe-bridge-factory.js';

import type { SafeBridge } from '../safe-bridge-factory.js';
import type { LogEntry } from '../../../index.js';

/** The console levels streamed as `log` events, matching NodeSandbox's set. */
const CONSOLE_LEVELS: readonly LogLevel[] = [LogLevel.log, LogLevel.warn, LogLevel.error, LogLevel.info];

/** Maps a console method name to its LogLevel; `log` is the fallback. */
function levelFor(method: string): LogLevel {
  return CONSOLE_LEVELS.find((l) => l === method) ?? LogLevel.log;
}

/**
 * Builds the console bridge. `onLog` is the host-side sink (the engine wires it
 * to `StreamHandle.push`); `now` supplies the timestamp (injected so the engine
 * keeps ownership of `Date.now`). The bridge itself only ever sees copied
 * strings — `level` and `argsJson` — and returns nothing.
 */
export function createConsoleBridge(onLog: (log: LogEntry) => void, now: () => number): SafeBridge {
  return createIgnoredBridge('__rq_console', (level: string, argsJson: string): void => {
    let args: unknown[];
    try {
      const parsed: unknown = JSON.parse(argsJson);
      args = Array.isArray(parsed) ? parsed : [argsJson];
    } catch {
      // Non-serializable args were stringified in-isolate to a single-string
      // fallback (see CONSOLE_ISOLATE_SHIM).
      args = [argsJson];
    }
    // `args` is the guest's JSON-serialized console args parsed back — Json by construction.
    onLog({ level: levelFor(level), args: args as LogEntry['args'], timestamp: now() });
  });
}

/**
 * In-isolate JS: defines `console` on top of `__rq_console`. Each method
 * serializes its args to JSON in-isolate (String() fallback for values JSON
 * can't encode), then hands the plain string to the host bridge.
 */
export const CONSOLE_ISOLATE_SHIM = `
(() => {
  const serialize = (args) => {
    try { return JSON.stringify(args); }
    catch { try { return JSON.stringify(args.map((a) => String(a))); } catch { return '["<unserializable>"]'; } }
  };
  const mk = (level) => (...args) => globalThis.__rq_console(level, serialize(args));
  globalThis.console = { log: mk('log'), warn: mk('warn'), error: mk('error'), info: mk('info') };
})();
`;
