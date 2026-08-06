// @requestly/cross-q — public types.
//
// The MappedItems / Report shapes are the app's own contract (see
// @requestly/shared-types `MappedItems` and the importers' `UnsupportedFeatureWarning`).
// They're intentionally typed loosely here as JSON so this package stays dependency-free;
// the consuming app narrows them to its own types at the seam.

/** A JSON value. */
export type Json =
  | null
  | boolean
  | number
  | string
  | Json[]
  | { [key: string]: Json };

/** The Requestly `MappedItems` bundle (bulkCreate*Item records, keyed by kind). */
export interface MappedItems {
  collections?: Json[];
  requests?: Json[];
  examples?: Json[];
  environments?: Json[];
  [key: string]: Json | undefined;
}

/** One recorded conversion decision. */
export interface Diagnostic {
  severity: 'ok' | 'coerced' | 'dropped' | 'error';
  phase: 'parse' | 'map' | 'emit';
  provenance: { format: string; locator?: string };
  message: string;
  detail?: Json;
}

/** The conversion report — what mapped cleanly, was coerced, or was dropped. */
export interface Report {
  fidelity: 'round_trip' | 'lossless' | 'lossy' | 'degraded';
  diagnostics?: Diagnostic[];
}

/** Success: the collection parsed; per-item losses (if any) ride in `report`. */
export interface ParseOk {
  ok: true;
  mapped: MappedItems;
  report: Report;
}

/** Hard failure: unknown format, or input that couldn't be parsed at all. */
export interface ParseErr {
  ok: false;
  error: string;
}

export type ParseResult = ParseOk | ParseErr;

/** Parse a collection into the Requestly MappedItems shape. */
export function parse(format: string, content: string, fileName?: string): ParseResult;

/** The source formats this engine build can import. */
export function supportedFormats(): string[];

/** The engine (crate) version. */
export function version(): string;
