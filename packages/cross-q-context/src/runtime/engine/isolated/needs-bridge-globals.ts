/**
 * The `needs_bridge` module-global table, in its own import-free module.
 *
 * Lifted out of the `bridges` barrel (which pulls the four Node-backed handlers)
 * so the browser host can reach it — it is one of the two require tiers the
 * browser supports. See ADR-204.
 */
/**
 * The in-isolate module globals each `needs_bridge` Node built-in resolves to,
 * keyed by require id. The require chain (`isolated-require.ts`) returns the
 * named global after the shims have been eval'd. `buffer` resolves to the
 * `Buffer` global itself; the others to their `__rq_*Module` global.
 */
export const NEEDS_BRIDGE_MODULE_GLOBALS: Readonly<Record<string, string>> = {
  buffer: 'Buffer',
  crypto: '__rq_cryptoModule',
  'node:crypto': '__rq_cryptoModule',
  util: '__rq_utilModule',
  'node:util': '__rq_utilModule',
  stream: '__rq_streamModule',
  'node:stream': '__rq_streamModule',
  zlib: '__rq_zlibModule',
  'node:zlib': '__rq_zlibModule',
  // Built by CORE_GLOBALS_SHIM from the same functions as the guest's timer
  // globals, so the module and the globals are one surface. This was
  // `__rq_processModule` — the PROCESS shim — so `require('timers').setTimeout`
  // was not a function in Safe mode (found by the RQ-5671 Phase 3 parity test).
  timers: '__rq_timersModule',
  'node:timers': '__rq_timersModule',
};
