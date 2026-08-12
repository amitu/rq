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

export function parseRequireId(rawId: string): ParsedRequireId {
  // Strip subpath: 'lodash/fp' → 'lodash', '@scope/pkg/sub' → '@scope/pkg'
  const bare = stripSubpath(rawId);

  if (bare.startsWith('@')) {
    // Scoped package: '@scope/name' or '@scope/name@version'
    const slashIndex = bare.indexOf('/');
    if (slashIndex === -1) {
      // Malformed scoped package — no slash after scope
      return { packageName: bare, version: undefined };
    }
    const afterScope = bare.substring(slashIndex + 1);
    const atIndex = afterScope.indexOf('@');
    if (atIndex === -1) {
      return { packageName: bare, version: undefined };
    }
    const packageName = bare.substring(0, slashIndex + 1 + atIndex);
    const version = afterScope.substring(atIndex + 1);
    return { packageName, version: version || undefined };
  }

  // Unscoped package: 'name' or 'name@version'
  const atIndex = bare.indexOf('@');
  if (atIndex === -1) {
    return { packageName: bare, version: undefined };
  }
  const packageName = bare.substring(0, atIndex);
  const version = bare.substring(atIndex + 1);
  return { packageName, version: version || undefined };
}

/**
 * Strips subpath from a require specifier, keeping only the package root.
 * - `'lodash/fp'` → `'lodash'`
 * - `'@scope/pkg/sub/path'` → `'@scope/pkg'`
 * - `'lodash'` → `'lodash'`
 */
function stripSubpath(id: string): string {
  if (id.startsWith('@')) {
    // Scoped: first slash is scope separator, second slash starts subpath
    const firstSlash = id.indexOf('/');
    if (firstSlash === -1) return id;
    const secondSlash = id.indexOf('/', firstSlash + 1);
    return secondSlash === -1 ? id : id.substring(0, secondSlash);
  }
  // Unscoped: first slash starts subpath
  const slashIndex = id.indexOf('/');
  return slashIndex === -1 ? id : id.substring(0, slashIndex);
}
