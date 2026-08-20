//! # cq-wasm — the WebAssembly boundary for the cross-q import engine
//!
//! This crate is what ships as the **`@requestly/cross-q`** npm package. It exposes the
//! cross-q import engine across a JSON-string boundary — the same shape as
//! `@requestly/script-analysis`: strings in, a JSON string out, no live objects crossing
//! the FFI line.
//!
//! The host (the Requestly importer, behind its ADR-196 seam) calls [`parse`] with a
//! format id, the raw collection content, and a filename; it gets back a JSON string it
//! parses into `{ ok, mapped, report }` (or `{ ok: false, error }`). See
//! `docs/CONTEXT.md` and the integration plan for how this plugs into `dispatchParse`.

use wasm_bindgen::prelude::*;

/// Parse `content` of the given `format` into a Requestly `MappedItems` bundle.
///
/// Returns a JSON string:
/// - success → `{ "ok": true, "mapped": <MappedItems>, "report": <Report> }`
/// - hard failure (unknown format / unparseable input) → `{ "ok": false, "error": <msg> }`
///
/// Per-item losses (coercions, drops) are never errors — they ride inside `report`.
/// `format` is one of the ids from [`formats`] (currently `"curl"`, `"postman"`).
#[wasm_bindgen]
pub fn parse(format: &str, content: &str, file_name: &str) -> String {
    let value = cross_q::parse_to_mapped_items(format, content, file_name);
    // A serde_json::Value always serializes; fall back to a hand-built error string only
    // in the impossible case, so the boundary never panics.
    serde_json::to_string(&value).unwrap_or_else(|e| {
        format!("{{\"ok\":false,\"error\":\"failed to serialize result: {e}\"}}")
    })
}

/// The engine's version (the crate version), for staleness/compat checks at the boundary.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// The importable source formats this engine build supports, as a JSON string array.
#[wasm_bindgen]
pub fn formats() -> String {
    // Keep in sync with `cross_q::build_workspace`'s match arms.
    "[\"curl\",\"postman\",\"bruno\",\"openapi\"]".to_string()
}
