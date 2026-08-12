/**
 * Pluggable filter contract for blocking package installation.
 * The installer calls the composed filter before every pnpm invocation.
 *
 * @see ADR-087 D-1 (PackageFilter interface)
 * @see ADR-087 D-2 (BlacklistEntry discriminated union)
 */
/**
 * Compose multiple filters into a single filter.
 * First rejection wins — remaining filters are not called.
 */
export function composeFilters(...filters) {
    return {
        check(packageName, version) {
            for (const filter of filters) {
                const result = filter.check(packageName, version);
                if (!result.allowed)
                    return result;
            }
            return { allowed: true };
        },
    };
}
