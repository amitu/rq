/* tslint:disable */
/* eslint-disable */

/**
 * The importable source formats this engine build supports, as a JSON string array.
 */
export function formats(): string;

/**
 * Parse `content` of the given `format` into a Requestly `MappedItems` bundle.
 *
 * Returns a JSON string:
 * - success → `{ "ok": true, "mapped": <MappedItems>, "report": <Report> }`
 * - hard failure (unknown format / unparseable input) → `{ "ok": false, "error": <msg> }`
 *
 * Per-item losses (coercions, drops) are never errors — they ride inside `report`.
 * `format` is one of the ids from [`formats`] (currently `"curl"`, `"postman"`).
 */
export function parse(format: string, content: string, file_name: string): string;

/**
 * The engine's version (the crate version), for staleness/compat checks at the boundary.
 */
export function version(): string;
