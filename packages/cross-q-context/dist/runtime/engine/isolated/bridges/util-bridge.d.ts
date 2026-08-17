/**
 * util-bridge — Safe-mode `util` subset (NEEDS_BRIDGE, ADR-010 §34).
 *
 * `util.inspect` / `util.format` are the data-shaped pieces packages reach for.
 * Both reduce to "string in → string out" once arguments are pre-stringified
 * in-isolate, so most of `util` is actually implementable as a pure in-isolate
 * shim with no host call. The one host-backed op is `inspect` (Node's formatter
 * is richer than anything we'd reimplement), exposed copy-in/copy-out.
 *
 * HARD INVARIANT: only copied data crosses — the isolate pre-serializes the
 * value to a JSON-safe form and the host returns a formatted string.
 */
import type { SafeBridge } from '../safe-bridge-factory.js';
/** The host-side util bridge installed as `__rq_util_inspect`. */
export declare function createUtilBridge(): SafeBridge;
/**
 * The guest-realm half of this bridge lives in `@requestly/sandbox-engine` (ADR-217):
 * the shim text is identical on every host, only the host callback above differs.
 * Re-exported here so existing import sites are unchanged.
 */
export { UTIL_ISOLATE_SHIM } from '../shims/util.shim.js';
