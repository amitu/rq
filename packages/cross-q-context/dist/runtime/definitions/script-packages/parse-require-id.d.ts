/**
 * Parses a require() specifier into package name and optional version.
 *
 * Handles:
 * - `'lodash'` → `{ packageName: 'lodash', version: undefined }`
 * - `'lodash@4.17.21'` → `{ packageName: 'lodash', version: '4.17.21' }`
 * - `'@faker-js/faker@9.0.0'` → `{ packageName: '@faker-js/faker', version: '9.0.0' }`
 * - `'@faker-js/faker'` → `{ packageName: '@faker-js/faker', version: undefined }`
 * - `'lodash/fp'` → `{ packageName: 'lodash', version: undefined }` (deep import)
 * - `'lodash@4.17.21/fp'` → `{ packageName: 'lodash', version: '4.17.21' }` (deep import + version)
 *
 * @see ADR-084 D-2
 */
export interface ParsedRequireId {
    /** The bare package name (e.g., `'lodash'`, `'@faker-js/faker'`). No subpath. */
    readonly packageName: string;
    /** The version specifier if present (e.g., `'4.17.21'`, `'^4.0.0'`), or `undefined`. */
    readonly version: string | undefined;
}
export declare function parseRequireId(rawId: string): ParsedRequireId;
