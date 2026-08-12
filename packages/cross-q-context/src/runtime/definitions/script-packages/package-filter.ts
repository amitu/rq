/**
 * Pluggable filter contract for blocking package installation.
 * The installer calls the composed filter before every pnpm invocation.
 *
 * @see ADR-087 D-1 (PackageFilter interface)
 * @see ADR-087 D-2 (BlacklistEntry discriminated union)
 */

// ---------------------------------------------------------------------------
// Filter Contract
// ---------------------------------------------------------------------------

export type PackageFilterResult = { readonly allowed: true } | { readonly allowed: false; readonly reason: string };

export interface PackageFilter {
  /**
   * Check whether a package is allowed to be installed.
   * Must be fast (<100ms) — the installer calls this before every pnpm invocation.
   */
  check(packageName: string, version: string | undefined): PackageFilterResult;
}

/**
 * Compose multiple filters into a single filter.
 * First rejection wins — remaining filters are not called.
 */
export function composeFilters(...filters: PackageFilter[]): PackageFilter {
  return {
    check(packageName, version) {
      for (const filter of filters) {
        const result = filter.check(packageName, version);
        if (!result.allowed) return result;
      }
      return { allowed: true };
    },
  };
}

// ---------------------------------------------------------------------------
// Blacklist Data Structure
// ---------------------------------------------------------------------------

export type BlacklistEntry = BlacklistEntryPackage | BlacklistEntryVersion;

export interface BlacklistEntryPackage {
  readonly scope: 'package';
  readonly packageName: string;
  readonly reason: string;
}

export interface BlacklistEntryVersion {
  readonly scope: 'version';
  readonly packageName: string;
  readonly version: string;
  readonly reason: string;
}
