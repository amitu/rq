// Vendored from the app's @requestly/variables — the executor's host-realm variable defaults.
// Maps a script mutation's recorded JS type onto the VariableDataType enum and builds a fresh
// VariableData for a newly-created variable (ADR-053).
import { VariableDataType } from '../model.js';
const TYPE_MAP = {
    string: VariableDataType.string,
    number: VariableDataType.number,
    boolean: VariableDataType.boolean,
    array: VariableDataType.array,
};
/** Maps a script mutation's recorded JS type to the VariableDataType enum. */
export function toVariableDataType(type) {
    return TYPE_MAP[type];
}
/** Constructs a default VariableData for a variable a script newly created. Host realm — crypto +
 * Date are reliable. */
export function createDefaultVariableData(value, type) {
    const now = new Date().toISOString();
    return {
        id: crypto.randomUUID(),
        syncValue: value,
        localValue: value,
        type: TYPE_MAP[type],
        isEnabled: true,
        isPersisted: false,
        rank: null,
        createdAt: now,
        updatedAt: now,
        createdBy: null,
        updatedBy: null,
    };
}
