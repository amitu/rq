// @requestly/cross-q-context — the compat (transform) pillar.
//
// Thin, dependency-free wrapper over the WebAssembly core (cq-transform). Strings in,
// typed objects out. The WASM boundary speaks JSON strings; this wrapper parses and
// light-validates them.
//
// This is a drop-in replacement for the app's private @requestly/script-analysis: same
// `transformScript` / `batchTransformScripts` / `extractRequires` surface, same result
// shapes, same OXC-based Rust engine — now the open, monorepo-hosted one. The bundler vs
// node WASM build is selected by the package `imports` map (#engine), exactly like
// @requestly/cross-q's engine — bundlers/tests get engine/, the CLI/node get engine-node/.

import { extract_requires, transform } from '#engine';

const VALID_DIAGNOSTIC_KINDS = new Set(['Replacement', 'Warning', 'Error']);

function isSummary(value) {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof value.replacements === 'number' &&
    typeof value.warnings === 'number' &&
    typeof value.errors === 'number'
  );
}

function isDiagnostic(value) {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof value.kind === 'string' &&
    VALID_DIAGNOSTIC_KINDS.has(value.kind) &&
    typeof value.message === 'string'
  );
}

function isTransformResult(value) {
  if (typeof value !== 'object' || value === null) return false;
  if (typeof value.success !== 'boolean') return false;
  if (typeof value.code !== 'string') return false;
  if (!Array.isArray(value.diagnostics)) return false;
  if (!isSummary(value.summary)) return false;
  return value.diagnostics.every(isDiagnostic);
}

function isExtractRequiresResult(value) {
  if (typeof value !== 'object' || value === null) return false;
  if (!Array.isArray(value.requires)) return false;
  return value.requires.every(
    (req) =>
      typeof req === 'object' &&
      req !== null &&
      typeof req.rawId === 'string' &&
      typeof req.span === 'object' &&
      req.span !== null,
  );
}

/**
 * Transform a single script, replacing platform-specific API calls (pm.*, postman.*,
 * bru.*) with Requestly's rq.* API.
 *
 * @param {import('./index.js').TransformInput} input `{ source, platform }`
 * @returns {import('./index.js').TransformResult} the rewritten `code` + diagnostics
 */
export function transformScript(input) {
  const json = transform(input.source, input.platform);
  let parsed;
  try {
    parsed = JSON.parse(json);
  } catch {
    parsed = null;
  }
  if (!isTransformResult(parsed)) {
    return {
      success: false,
      code: input.source,
      diagnostics: [{ kind: 'Error', message: 'Invalid response from WASM transform engine' }],
      summary: { replacements: 0, warnings: 0, errors: 1 },
    };
  }
  return parsed;
}

/**
 * Batch transform multiple script pairs. Each entry has optional preRequest and
 * postResponse scripts. Returns per-entry results and an aggregated summary.
 *
 * @param {import('./index.js').BatchTransformInput} input `{ scripts, platform }`
 * @returns {import('./index.js').BatchTransformResult}
 */
export function batchTransformScripts(input) {
  const results = {};
  let totalReplacements = 0;
  let totalWarnings = 0;
  let totalErrors = 0;

  for (const [key, pair] of Object.entries(input.scripts)) {
    const entry = {
      preRequest: pair.preRequest
        ? transformScript({ source: pair.preRequest, platform: input.platform })
        : undefined,
      postResponse: pair.postResponse
        ? transformScript({ source: pair.postResponse, platform: input.platform })
        : undefined,
    };

    if (entry.preRequest) {
      totalReplacements += entry.preRequest.summary.replacements;
      totalWarnings += entry.preRequest.summary.warnings;
      totalErrors += entry.preRequest.summary.errors;
    }
    if (entry.postResponse) {
      totalReplacements += entry.postResponse.summary.replacements;
      totalWarnings += entry.postResponse.summary.warnings;
      totalErrors += entry.postResponse.summary.errors;
    }

    results[key] = entry;
  }

  return {
    results,
    summary: { replacements: totalReplacements, warnings: totalWarnings, errors: totalErrors },
  };
}

/**
 * Extract static `require()` calls from script source (ADR-084). Only string-literal
 * arguments are extracted — dynamic requires, template literals, and binary expressions
 * are skipped.
 *
 * @param {string} source the script text
 * @returns {ReadonlyArray<import('./index.js').ExtractedRequire>}
 */
export function extractRequires(source) {
  const json = extract_requires(source);
  let parsed;
  try {
    parsed = JSON.parse(json);
  } catch {
    parsed = null;
  }
  if (!isExtractRequiresResult(parsed)) {
    return [];
  }
  return parsed.requires;
}
