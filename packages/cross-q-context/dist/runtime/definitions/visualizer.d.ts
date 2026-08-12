/**
 * `rq.visualizer` — sandbox scripting surface for the response visualizer
 * (ADR-202, Postman `pm.visualizer` parity).
 *
 * The namespace is a pure collector, modelled on `createExecutionNamespace` +
 * `ExecutionDirectiveCollector` (execution.ts): `set(template, data)` compiles
 * the Handlebars template EAGERLY against a JSON snapshot of `data` and writes a
 * single discriminated `VisualizerDirective` onto an injected collector, which the
 * engine drains after the script settles. `set()` overwrites the slot (FR-03
 * "last call wins" is free); `clear()` writes a `{ kind: 'cleared' }` marker so a
 * later phase's clear overrides an earlier `set()` (FR-18c) — the runtime strips the
 * marker at the entry lift, disabling the Visualize action (FR-16).
 *
 * Two in-guest guards keep every non-JSON / bad-template outcome contained so it
 * never aborts the post-response script (ADR-202 Decision 1 / Decision 3, FR-10):
 * a `JSON.stringify` throw (circular reference / BigInt) and a Handlebars compile
 * or render throw are both caught and recorded as `{ kind: 'error', message }`.
 *
 * Phase gating (ADR-202): the visualizer is post-response-only. It is also in
 * `PHASE_RESTRICTED`, so the engines make it genuinely absent in the pre-request
 * phase; this factory is additionally phase-guarded so that, if constructed
 * outside post-response, `set`/`clear` no-op and emit a console warning rather
 * than compiling or collecting anything.
 *
 * The factory takes its dependencies as parameters — no `declare const`, no
 * globals — so it stays platform-agnostic and unit-testable.
 */
import type { JsonValue } from './_deps.js';
import type { VisualizerDirective } from './_deps.js';
/** Sink the visualizer namespace records its compiled/error output — or a `clear()` marker — onto (ADR-202). Mirrors ExecutionDirectiveCollector. */
export interface VisualizerCollector {
    output?: VisualizerDirective;
}
/** Minimal Handlebars surface the visualizer needs (compile → render). Satisfied by the vendor IIFE. */
export interface VisualizerLibs {
    handlebars: {
        compile: (template: string) => (context?: unknown) => string;
    };
}
export interface RqVisualizerNamespace {
    /** Set the response visualization from a Handlebars template + optional data (Postman parity). */
    set(template: string, data?: JsonValue): void;
    /** Clear the current visualization — returns the Visualize action to disabled (FR-16). */
    clear(): void;
}
/**
 * The single JS-in-HTML global the compiled `html` embeds the data snapshot on
 * (ADR-202 FR-04c). The render surface's `pm.getData`/`rq.getData` shims read it
 * (RQ-4996 / ADR-203). Kept as a named constant so both engines embed the same key.
 */
export declare const VISUALIZER_DATA_GLOBAL = "__rq_viz_data__";
/**
 * Builds `rq.visualizer` (ADR-202; pre-request support added 2026-08-02, ADR-202
 * "Amendment (2026-08-02)" / TB FR-18). Available in BOTH the pre-request and
 * post-response phases (Postman parity — `pm.visualizer.set()` is callable in both).
 * `set()` snapshots `data`, compiles the template eagerly and records a `compiled` /
 * `error` output; `clear()` empties the slot. The pre-request and post-response
 * outputs feed a single per-entry slot, last-writer-wins — the runtime lifts both
 * (`modules/runtime/src/core/execute.ts`).
 */
export declare function createVisualizer(collector: VisualizerCollector, libs: VisualizerLibs): RqVisualizerNamespace;
