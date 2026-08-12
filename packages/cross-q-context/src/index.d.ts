// @requestly/cross-q-context — public types (compat / transform pillar).
//
// The result shapes mirror the app's private @requestly/script-analysis exactly, so this
// package is a drop-in replacement at the import seam. The consuming app narrows them to
// its own types where needed.

/** Diagnostic severity kind from the WASM transform engine. */
export type DiagnosticKind = 'Replacement' | 'Warning' | 'Error';

/** Byte-offset location information for a diagnostic. */
export interface SpanInfo {
  readonly start: number;
  readonly end: number;
  readonly line: number;
  readonly col: number;
}

/** A single diagnostic produced by the transform engine. */
export interface Diagnostic {
  readonly kind: DiagnosticKind;
  readonly message: string;
  readonly span?: SpanInfo;
}

/** Summary counts from a transform operation. */
export interface Summary {
  readonly replacements: number;
  readonly warnings: number;
  readonly errors: number;
}

/** Discriminated union result from a single script transform. */
export interface TransformResult {
  /** `true` when the source parsed; `false` on a hard parse error (code echoes input). */
  readonly success: boolean;
  /** The rewritten source (or the original, unchanged, on failure). */
  readonly code: string;
  readonly diagnostics: readonly Diagnostic[];
  readonly summary: Summary;
}

/** Supported source platforms for script transformation. */
export type Platform = 'postman';

/** Input for a single script transform. */
export interface TransformInput {
  readonly source: string;
  readonly platform: Platform;
}

/** A pair of pre-request and post-response scripts. */
export interface ScriptPair {
  readonly preRequest?: string;
  readonly postResponse?: string;
}

/** Input for batch transforming multiple script pairs. */
export interface BatchTransformInput {
  readonly scripts: Readonly<Record<string, ScriptPair>>;
  readonly platform: Platform;
}

/** Result for a single script pair in a batch. */
export interface ScriptPairResult {
  readonly preRequest?: TransformResult;
  readonly postResponse?: TransformResult;
}

/** Result of a batch transform operation. */
export interface BatchTransformResult {
  readonly results: Readonly<Record<string, ScriptPairResult>>;
  readonly summary: Summary;
}

/** A single static `require()` call extracted from script source (ADR-084). */
export interface ExtractedRequire {
  /** The raw string argument (e.g., `'lodash@4.17.21'`, `'@faker-js/faker'`). */
  readonly rawId: string;
  /** Byte-offset span of the entire `require('...')` call expression. */
  readonly span: SpanInfo;
}

/**
 * Transform a single script, replacing platform-specific API calls (pm.*, postman.*)
 * with Requestly's rq.* API.
 */
export function transformScript(input: TransformInput): TransformResult;

/**
 * Batch transform multiple script pairs. Each entry has optional preRequest and
 * postResponse scripts. Returns per-entry results and an aggregated summary.
 */
export function batchTransformScripts(input: BatchTransformInput): BatchTransformResult;

/**
 * Extract static `require()` calls from script source (ADR-084). Only string-literal
 * arguments are extracted — dynamic requires, template literals, and binary expressions
 * are skipped.
 */
export function extractRequires(source: string): readonly ExtractedRequire[];
