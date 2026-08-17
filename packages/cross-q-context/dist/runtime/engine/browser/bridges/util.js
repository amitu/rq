import { inspect } from 'node-inspect-extracted';
/**
 * Browser host callback for the Safe-mode `util` bridge (ADR-204).
 *
 * Peer of `sandbox-node/src/isolated/bridges/util-bridge.ts`; the in-isolate shim
 * is shared verbatim (it implements `format` purely in-isolate and defers only
 * `inspect` to this host callback).
 *
 * ## Why a port of Node's implementation rather than a hand-rolled formatter
 *
 * `util.inspect`'s output is not a data value, but it IS user-visible — it is what
 * a script author reads when debugging. Hand-rolling it means guaranteed
 * divergence: Node's `reduceToSingleString` has non-obvious rules (the `compact: 3`
 * heuristic, the `breakLength` accounting that adds indentation + brace + a
 * 10-char fudge, and `groupArrayElements`' column alignment for arrays over six
 * entries). Reimplementing that from the outside produces something that looks
 * right on small inputs and drifts on real ones.
 *
 * `node-inspect-extracted` is Node's own implementation extracted for the web, so
 * parity here is STRUCTURAL rather than aspirational — the same argument ADR-204
 * makes for sharing one script engine, applied one layer down.
 *
 * ## Narrow value space
 *
 * The shared shim calls this with `JSON.stringify(value)`, so whatever arrives has
 * already been through a JSON round-trip: only `null`, booleans, numbers, strings,
 * arrays, and plain objects can reach here. Functions, symbols, `undefined`,
 * Dates-as-objects, Maps/Sets, and circular references cannot — the shim either
 * drops them or falls back to `String(value)` on a stringify throw.
 */
/** Options are pinned to the Node bridge's; the parity test asserts identical output. */
const INSPECT_OPTIONS = { depth: 4, breakLength: 120 };
export function browserUtilHandler(req) {
    let value;
    try {
        value = JSON.parse(req.json);
    }
    catch {
        // Same fallback as the Node bridge: a non-JSON payload is inspected as the
        // raw string it is, rather than throwing across the bridge.
        value = req.json;
    }
    return { text: inspect(value, INSPECT_OPTIONS) };
}
