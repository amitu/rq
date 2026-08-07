//! Postman version detection — mirrors the app's `detectPostmanVersion`: the authoritative
//! marker is the `info.schema` URL (it encodes v2.0 vs v2.1); v1.0.0 is a structural signal
//! (flat `requests[]`, no `info`, no `item[]`).
//!
//! Divergence from the app (deliberate, tolerant): where the app *fails loud* on a
//! v2-shaped doc with an absent/unknown `info.schema`, cross-q defaults it to v2.1 (its
//! tolerant ethos). This is a known parity item — the equivalence gate will flag the
//! "missing marker" fixtures, and we decide then whether to match the app's fail-loud.

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
        // Marker present but unrecognized — fall through to structural (tolerant).
    }

    // v2-shaped (has `item[]`): default to v2.1 when the marker is absent/unknown.
    if root.get("item").is_some() {
        return Ok(PostmanVersion::V2_1);
    }

    // v1.0.0: flat `requests[]`, no `info` wrapper, no `item[]` (positive v1 signal).
    if root.get("requests").map(Value::is_array).unwrap_or(false)
        && root.get("info").is_none()
        && root.get("item").is_none()
    {
        return Ok(PostmanVersion::V1_0);
    }

    Err("not a Postman collection (no info.schema, no item[], no requests[])".to_string())
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
    fn v2_shaped_without_marker_defaults_v2_1() {
        let d = detect_version(&json!({ "info": { "name": "C" }, "item": [] }));
        assert_eq!(d, Ok(PostmanVersion::V2_1));
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
