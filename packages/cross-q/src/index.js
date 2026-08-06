// @requestly/cross-q — the import engine.
//
// Thin, dependency-free wrapper over the WebAssembly core (cq-wasm). Strings in, typed
// objects out. Mirrors the shape of @requestly/script-analysis: the WASM boundary speaks
// JSON strings; this wrapper parses and light-validates them.
//
// The bundler vs node WASM build is selected by the package `imports` map (#engine),
// exactly like @requestly/script-analysis's #wasm-binding — bundlers/tests get pkg/,
// the CLI/node get pkg-node/.

import * as engine from '#engine';

/**
 * Parse a collection into the Requestly MappedItems shape.
 *
 * @param {string} format  one of `supportedFormats()` (e.g. "postman", "curl")
 * @param {string} content the raw collection text
 * @param {string} [fileName] optional source filename (used only for diagnostics)
 * @returns {ParseResult} `{ ok: true, mapped, report }` or `{ ok: false, error }`
 */
export function parse(format, content, fileName = '') {
  const raw = engine.parse(format, content, fileName);
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (e) {
    return { ok: false, error: `engine returned invalid JSON: ${e.message}` };
  }
  if (typeof parsed !== 'object' || parsed === null || typeof parsed.ok !== 'boolean') {
    return { ok: false, error: 'engine returned an unexpected shape' };
  }
  return parsed;
}

/** The source formats this engine build can import. @returns {string[]} */
export function supportedFormats() {
  try {
    return JSON.parse(engine.formats());
  } catch {
    return [];
  }
}

/** The engine (crate) version, for staleness/compat checks. @returns {string} */
export function version() {
  return engine.version();
}
