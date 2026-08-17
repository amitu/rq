// Vendored from the app's @requestly/variables — the executor's host-realm variable defaults.
// Maps a script mutation's recorded JS type onto the VariableDataType enum and builds a fresh
// VariableData for a newly-created variable (ADR-053).
import { VariableDataType } from '../model.js';

import type { VariableData } from '../model.js';

/** JS typeof strings (plus the `array` tag) a raw mutation carries. */
export type RawMutationType = 'string' | 'number' | 'boolean' | 'array';

const TYPE_MAP: Record<RawMutationType, VariableDataType> = {
  string: VariableDataType.string,
  number: VariableDataType.number,
  boolean: VariableDataType.boolean,
  array: VariableDataType.array,
};

/** Maps a script mutation's recorded JS type to the VariableDataType enum. */
export function toVariableDataType(type: RawMutationType): VariableDataType {
  return TYPE_MAP[type];
}

/** Constructs a default VariableData for a variable a script newly created. Host realm — crypto +
 * Date are reliable. */
export function createDefaultVariableData(value: string, type: RawMutationType): VariableData {
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
