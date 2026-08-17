import { VariableDataType } from '../model.js';
import { createDefaultVariableData, toVariableDataType } from './variable-data.js';
/**
 * Inflates raw script mutations into a proper MutationDiff with full VariableData.
 * This is Layer 2 of ADR-053 — runs in the host realm (Node.js) after script execution.
 *
 * Rules:
 * - Existing variable → clone original VariableData, update only localValue
 * - New variable → createDefaultVariableData() with inferred type
 * - Runtime scope new variable → omit syncValue (transient, not persisted)
 * - null entry → null in diff (delete sentinel — ADR-053)
 * - Collection scope → wrap in CollectionMutation with collectionId
 */
export function inflateMutations(rawMutations, context) {
    const diff = {};
    if (rawMutations.global) {
        diff.global = inflateScope(rawMutations.global, context.global, false);
    }
    if (rawMutations.environment) {
        diff.environment = inflateScope(rawMutations.environment, context.environment, false);
    }
    if (rawMutations.runtime) {
        diff.runtime = inflateScope(rawMutations.runtime, context.variables, true);
    }
    // Layer 1 (createVariableScope) no-ops set/unset/clear when collectionId is null
    // (RQ-4236), so rawMutations.collection should never be populated without a valid
    // collectionId. The collectionId guard is kept as a defensive check.
    if (rawMutations.collection && context.info.collectionId) {
        diff.collection = {
            collectionId: context.info.collectionId,
            variables: inflateScope(rawMutations.collection, context.collectionVariables, false),
        };
    }
    return diff;
}
function inflateScope(rawEntries, contextVars, isRuntimeScope) {
    const result = {};
    for (const [key, entry] of Object.entries(rawEntries)) {
        if (entry === null) {
            // Unset — null sentinel means "delete this key"
            result[key] = null;
            continue;
        }
        const existing = contextVars[key];
        if (existing) {
            // Existing variable — clone and update localValue. syncValue is preserved
            // (server-persisted value stays unchanged). The type follows the value the
            // script set, so re-typing a variable (e.g. set("x", 1) on a previously
            // string var) reads back as that type instead of a string (RQ-1421).
            // Secret-typed variables keep their type — a script set must not downgrade
            // a secret to a plain string/number/boolean.
            const nextType = existing.type === VariableDataType.secret ? existing.type : toVariableDataType(entry.type);
            result[key] = { ...existing, localValue: entry.value, type: nextType };
        }
        else {
            // New variable — construct with defaults
            const newVar = createDefaultVariableData(entry.value, entry.type);
            if (isRuntimeScope) {
                // Runtime variables are transient — clear syncValue
                newVar.syncValue = '';
            }
            result[key] = newVar;
        }
    }
    return result;
}
