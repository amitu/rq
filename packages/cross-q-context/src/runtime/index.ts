// The cross-q-context scripting runtime — public surface. Step 1 the contract, step 2 the data
// model, step 3 the composed execution types + the rq.* API (ADR-213 Layer 2).
export * from './contract.js';
export * from './model.js';
export * from './execution.js';
// The rq.* API (createRqNamespace, the guest shims, GLOBAL_NAMES, …) — the runtime pillar.
export * from './definitions/index.js';

// The host-side result types. `execute()` returns these, so importing `MutationDiff` from the
// runtime surface gives you the one you will actually be handed — not the guest's raw shape,
// which is `RawMutationDiff` in contract.ts. They are type-only, so this pulls in no engine code.
export type {
  MutationDiff,
  MutationVariables,
  CollectionMutation,
  ScriptExecutionResult,
  SandboxExecutionEvent,
  TestResult,
  TestResultStatus,
} from './engine/host-types.js';
