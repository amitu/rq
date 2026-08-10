//! Postman version detection — mirrors the app's `detectPostmanVersion`: the authoritative
//! marker is the `info.schema` URL (it encodes v2.0 vs v2.1); v1.0.0 is a structural signal
//! (flat `requests[]`, no `info`, no `item[]`).
//!
//! Strict by default (matches the app's `detectPostmanVersion`): a v2-shaped doc with an
//! absent or unrecognized `info.schema` **fails loud** rather than being guessed as v2.1.
//! The published spec makes `info.schema` required; a silent baseline assumption is the
//! kind of guess that masks import regressions. (cross-q is tolerant only where a real
//! reason forces it — e.g. RQ-3458 key coercion — not here.)

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PostmanVersion {
    V1_0,
    V2_0,
    V2_1,
}

pub(super) fn detect_version(root: &Value) -> Result<PostmanVersion, String> {
    let schema = root
        .get("info")
        .and_then(|i| i.get("schema"))
        .and_then(Value::as_str);

    if let Some(s) = schema {
        if s.contains("v2.1.0") {
            return Ok(PostmanVersion::V2_1);
        }
        if s.contains("v2.0.0") {
            return Ok(PostmanVersion::V2_0);
        }
        // Marker present but unrecognized — fail loud (name it), don't guess.
        return Err(format!(
            "unsupported Postman collection version marker: {s:?} (supported: v1.0.0, v2.0.0, v2.1.0)"
        ));
    }

    // No `info.schema`. v1.0.0 is a positive structural signal: flat `requests[]`, no
    // `info` wrapper, no `item[]`.
    if root.get("requests").map(Value::is_array).unwrap_or(false)
        && root.get("info").is_none()
        && root.get("item").is_none()
    {
        return Ok(PostmanVersion::V1_0);
    }

    // A v2-shaped doc (has `item[]`) with no marker, or anything else: fail loud. No v2.1
    // baseline fallback — strict by default.
    Err("not a supported Postman collection: missing info.schema (v2 requires it) and no v1 requests[] signal".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_marker_dispatches_v2() {
        let v21 = json!({ "info": { "name": "C", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" }, "item": [] });
        assert_eq!(detect_version(&v21), Ok(PostmanVersion::V2_1));
        let v20 = json!({ "info": { "name": "C", "schema": "https://schema.postman.com/json/collection/v2.0.0/collection.json" }, "item": [] });
        assert_eq!(detect_version(&v20), Ok(PostmanVersion::V2_0));
    }

    #[test]
    fn v2_shaped_without_marker_fails_loud() {
        // Strict: a v2-shaped doc with no info.schema is rejected, not guessed as v2.1.
        let d = detect_version(&json!({ "info": { "name": "C" }, "item": [] }));
        assert!(d.is_err());
    }

    #[test]
    fn unknown_marker_fails_loud() {
        let d = detect_version(
            &json!({ "info": { "name": "C", "schema": "…/v9.9.9/collection.json" }, "item": [] }),
        );
        assert!(d.is_err());
    }

    #[test]
    fn v1_structural_signal() {
        let d = detect_version(&json!({ "id": "c", "name": "C", "requests": [], "order": [] }));
        assert_eq!(d, Ok(PostmanVersion::V1_0));
    }

    #[test]
    fn not_a_collection_errors() {
        assert!(detect_version(&json!({ "foo": "bar" })).is_err());
    }
}
