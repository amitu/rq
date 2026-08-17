import { VariableDataType } from '../model.js';
import type { VariableData } from '../model.js';
/** JS typeof strings (plus the `array` tag) a raw mutation carries. */
export type RawMutationType = 'string' | 'number' | 'boolean' | 'array';
/** Maps a script mutation's recorded JS type to the VariableDataType enum. */
export declare function toVariableDataType(type: RawMutationType): VariableDataType;
/** Constructs a default VariableData for a variable a script newly created. Host realm — crypto +
 * Date are reliable. */
export declare function createDefaultVariableData(value: string, type: RawMutationType): VariableData;
