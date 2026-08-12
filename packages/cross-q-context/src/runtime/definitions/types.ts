import { PHASE_DESCRIPTORS, ScriptPhase } from './_deps.js';

/**
 * Map from entry name to allowed phases. Entries not in this map are
 * available in all phases. Both the builder and codegen read from this map.
 *
 * DERIVED by inverting each phase's `exclusiveSurface` (ADR-208 §4) rather than
 * hand-written, so a phase and its exclusive surfaces are declared once. The
 * inversion must reproduce every key the hand-written map held:
 *
 * - `response: [postResponse]`
 * - `visualizer: [preRequest, postResponse]` — owned by ADR-202. Available in BOTH
 *   those phases (Postman parity: `pm.visualizer.set()` is callable in both),
 *   resolving last-writer-wins across the chain with the pre-request result lifted
 *   onto the entry in `modules/runtime/src/core/execute.ts` — see ADR-202
 *   "Amendment (2026-08-02)" / TB FR-18 / D-18, which reversed the original
 *   post-response-only restriction. It stays restricted (rather than dropping out of
 *   this map entirely) so it is genuinely ABSENT in on-message, where nothing lifts
 *   `visualizerOutput`: an ignored call is the outcome ADR-202 restricted against.
 *   Both phases list it and the inversion accumulates — the two-phase case the
 *   `??=` below exists for.
 * - `message: [onMessage]`
 *
 * Note this derivation does NOT reach the Safe engine's hand-written guest mirror
 * (`isolated-rq.ts`), which gates the same members with its own booleans. That
 * shim needs its own absence test; key-presence parity passes when it drifts.
 */
export const PHASE_RESTRICTED: Readonly<Partial<Record<string, readonly ScriptPhase[]>>> = Object.freeze(
  Object.values(ScriptPhase).reduce<Record<string, ScriptPhase[]>>((restricted, phase) => {
    for (const surface of PHASE_DESCRIPTORS[phase].exclusiveSurface) {
      // Accumulated rather than assigned: a surface exclusive to two phases
      // (`visualizer`) must list both, not have the second overwrite the first.
      (restricted[surface] ??= []).push(phase);
    }
    return restricted;
  }, {}),
);
