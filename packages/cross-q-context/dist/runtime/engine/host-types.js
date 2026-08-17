// cross-q-context executor — HOST-side result types.
//
// The guest/rq.* API deals in low-level primitives (contract.ts: raw mutations, minimal results).
// The HOST inflates those into the persist-ready shapes a caller consumes — full VariableData, a
// scoped MutationDiff, the cookie mutation log, the rich ScriptExecutionResult. These mirror the
// app's `@requestly/shared-types/runtime` shapes so a host can consume execute()'s output directly;
// cross-q-context owns them (canonical), reusing the variable model + execution context it already
// defines.
export {};
