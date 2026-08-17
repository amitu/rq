import type { MutationDiff } from './host-types.js';
import type { ScriptExecutionContext } from '../execution.js';
import type { RawScopeMutations } from '../definitions/rqMethods.js';
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
export declare function inflateMutations(rawMutations: RawScopeMutations, context: ScriptExecutionContext): MutationDiff;
