/* tslint:disable */
/* eslint-disable */

/**
 * WASM entry point: extract static `require()` calls from script source (ADR-084).
 * Returns JSON: `{ "requires": [{ "raw_id": "lodash@4.17.21", "span": { "start": 0, "end": 25, "line": 1, "col": 0 } }] }`
 */
export function extract_requires(source: string): string;

export function transform(source: string, platform: string): string;
