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
import type { SafeBridge } from '../safe-bridge-factory.js';
import type { LogEntry } from '../../../index.js';
/**
 * Builds the console bridge. `onLog` is the host-side sink (the engine wires it
 * to `StreamHandle.push`); `now` supplies the timestamp (injected so the engine
 * keeps ownership of `Date.now`). The bridge itself only ever sees copied
 * strings — `level` and `argsJson` — and returns nothing.
 */
export declare function createConsoleBridge(onLog: (log: LogEntry) => void, now: () => number): SafeBridge;
/**
 * In-isolate JS: defines `console` on top of `__rq_console`. Each method
 * serializes its args to JSON in-isolate (String() fallback for values JSON
 * can't encode), then hands the plain string to the host bridge.
 */
export declare const CONSOLE_ISOLATE_SHIM = "\n(() => {\n  const serialize = (args) => {\n    try { return JSON.stringify(args); }\n    catch { try { return JSON.stringify(args.map((a) => String(a))); } catch { return '[\"<unserializable>\"]'; } }\n  };\n  const mk = (level) => (...args) => globalThis.__rq_console(level, serialize(args));\n  globalThis.console = { log: mk('log'), warn: mk('warn'), error: mk('error'), info: mk('info') };\n})();\n";
