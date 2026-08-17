/**
 * Keep a `require()`d Node built-in's async work visible to the `AsyncRegistry`
 * (ADR-219, RQ-5671 Phase 3).
 *
 * Developer mode's require chain hands the script the REAL Node module
 * (`require-builder.ts` Tier 5), so anything async it starts is invisible to the
 * drain — `require('timers').setTimeout(cb, 1000)` bypasses the registry-backed
 * global entirely. Safe mode has no such hole: every `needs_bridge` module reaches
 * the host through a counted bridge.
 *
 * Which treatment each built-in gets is declared on its registry entry
 * (`NodeBuiltinPackage.developerAsync`), a REQUIRED field — so a new built-in
 * cannot be added without deciding, which is the standing guarantee that keeps
 * this from being a one-time sweep.
 */
import type { AsyncRegistry } from '../index.js';
import type { DeveloperAsyncTreatment } from '../../index.js';
/** The registry-backed timer surface, as injected over the script's globals. */
export interface RegistryTimerGlobals {
    readonly setTimeout: unknown;
    readonly setInterval: unknown;
    readonly clearTimeout: unknown;
    readonly clearInterval: unknown;
}
/**
 * Apply a built-in's declared Developer async treatment.
 *
 * `not-an-async-source` returns the module untouched — the justification for each
 * such entry lives on the registry entry, not here.
 */
export declare function applyDeveloperAsyncTreatment(mod: unknown, treatment: DeveloperAsyncTreatment, registry: AsyncRegistry<never>, timers: RegistryTimerGlobals): unknown;
