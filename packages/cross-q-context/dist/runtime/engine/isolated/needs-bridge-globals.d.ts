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
export declare const NEEDS_BRIDGE_MODULE_GLOBALS: Readonly<Record<string, string>>;
