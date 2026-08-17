/**
 * Registers dynamic variables as `$`-prefixed methods directly on the rq object.
 * Iterates each resolver's catalog and assigns a method per variable.
 *
 * ADR-055: Eager registration — simple loop assignment, no Proxy.
 * Resolvers are constructed in-process (ADR-034 — never serialized).
 */
export function registerDynamicVariables(rq, resolvers) {
    for (const resolver of resolvers) {
        for (const meta of resolver.list()) {
            rq[meta.name] = (...args) => resolver.resolve(meta.name, ...args);
        }
    }
}
