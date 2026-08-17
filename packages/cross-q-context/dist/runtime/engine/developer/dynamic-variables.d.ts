import type { VariableResolver } from '../../definitions/_deps.js';
/**
 * Registers dynamic variables as `$`-prefixed methods directly on the rq object.
 * Iterates each resolver's catalog and assigns a method per variable.
 *
 * ADR-055: Eager registration — simple loop assignment, no Proxy.
 * Resolvers are constructed in-process (ADR-034 — never serialized).
 */
export declare function registerDynamicVariables(rq: Record<string, unknown>, resolvers: ReadonlyArray<VariableResolver>): void;
