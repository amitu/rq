/**
 * process-bridge — Safe-mode inert `process` + timers (NEEDS_BRIDGE, ADR-010 §34).
 *
 * Packages routinely touch `process.env`, `process.nextTick`, `process.platform`,
 * and `setTimeout`/`setInterval` for feature detection or deferral. Exposing the
 * REAL host `process` is exactly the escape RQ-2489 is about, so this bridge is
 * deliberately INERT: an empty `env`, fixed platform/version strings, and
 * microtask-based `nextTick`. Timers are in-isolate (the isolate's own event
 * loop), not host timers. There is no host capability behind it — pure in-isolate
 * JS — so there is no host callback. Registered as a bridge for install +
 * containment-test discipline.
 *
 * HARD INVARIANT: trivially held — nothing crosses the edge; the host `process`
 * is never referenced from in-isolate code.
 */
/**
 * In-isolate JS: an inert `process` global + `setTimeout`/`setInterval`/`queueMicrotask`.
 * `process.env` is an empty object (no host env leaks). Platform strings are
 * static and non-identifying.
 */
export declare const PROCESS_ISOLATE_SHIM = "\n(() => {\n  const process = {\n    env: {},\n    platform: 'sandbox',\n    arch: 'sandbox',\n    version: 'v0.0.0',\n    versions: { node: '0.0.0' },\n    argv: [],\n    pid: 0,\n    nextTick: (fn, ...args) => { Promise.resolve().then(() => fn(...args)); },\n    cwd: () => '/',\n    hrtime: () => [0, 0],\n    exit: () => { throw new Error('process.exit is not available in Safe mode'); },\n  };\n  globalThis.process = process;\n  globalThis.__rq_processModule = process;\n  // Timers run on the isolate's own loop \u2014 never host timers.\n  if (typeof globalThis.setTimeout !== 'function') {\n    globalThis.setTimeout = (fn, _ms, ...args) => { Promise.resolve().then(() => fn(...args)); return 0; };\n    globalThis.clearTimeout = () => {};\n    globalThis.setInterval = () => 0;\n    globalThis.clearInterval = () => {};\n  }\n  if (typeof globalThis.queueMicrotask !== 'function') {\n    globalThis.queueMicrotask = (fn) => { Promise.resolve().then(fn); };\n  }\n})();\n";
