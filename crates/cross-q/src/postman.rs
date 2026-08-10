//! Parse a Postman Collection into the Idealised Model.
//!
//! **Version-isolated.** Each Postman schema version is its own parser module, converging
//! only at the IR — never an `if version === …` branch inside one parser. The entry point
//! detects the version ([`detect`]) off `info.schema` (or a v1 structural signal) and
//! dispatches to the dedicated parser:
//!
//! - [`v1_0`] — v1.0.0: flat `requests[]` + `folders[]`/`order[]`, header-strings, `dataMode` bodies.
//! - [`v2_0`] — v2.0.0: the `item[]` tree; auth params are objects.
//! - [`v2_1`] — v2.1.0: the `item[]` tree; auth params are arrays.
//!
//! Version-agnostic leaf primitives (coercion, kv, bodies, scripts, auth building, the v2
//! tree walk) live in [`shared`], so the version modules compose rather than copy-paste.
//!
//! **Strict by default.** Version/structure detection fails loud (a v2-shaped doc with no
//! recognized `info.schema` is rejected, not guessed). cross-q is tolerant only where a
//! real reason forces it — the one such case is RQ-3458 key coercion (real exports emit
//! null/numeric keys; coerce the unambiguous, drop the ambiguous, and report both).

use serde_json::Value;

use cq_model::Workspace;
use cq_report::Report;

mod detect;
mod shared;
mod v1_0;
mod v2_0;
mod v2_1;

/// Parse a Postman collection JSON string into a [`Workspace`], detecting the schema
/// version and dispatching to the matching per-version parser.
pub fn parse_postman(input: &str, report: &mut Report) -> Result<Workspace, String> {
    let root: Value = serde_json::from_str(input).map_err(|e| format!("invalid JSON: {e}"))?;
    match detect::detect_version(&root)? {
        detect::PostmanVersion::V1_0 => Ok(v1_0::parse(&root, report)),
        detect::PostmanVersion::V2_0 => Ok(v2_0::parse(&root, report)),
        detect::PostmanVersion::V2_1 => Ok(v2_1::parse(&root, report)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_model::{Auth, Body, Item, Method, Protocol, ScriptDialect};
    use cq_report::{Fidelity, Severity};

    fn parse(s: &str) -> (Workspace, Report) {
        let mut r = Report::new(Fidelity::Lossless);
        let ws = parse_postman(s, &mut r).expect("parse");
        (ws, r)
    }

    const V21_SCHEMA: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

    // ---- v2.1 -------------------------------------------------------------------------

    #[test]
    fn v2_1_parses_a_simple_collection() {
        let json = format!(
            r#"{{
              "info": {{ "name": "GitHub", "_postman_id": "abc", "schema": "{V21_SCHEMA}" }},
              "item": [
                {{ "name": "list issues",
                  "request": {{
                    "method": "GET",
                    "url": {{ "raw": "https://api.github.com/issues?state=open",
                             "query": [{{ "key": "state", "value": "open" }}] }},
                    "header": [{{ "key": "Accept", "value": "application/json" }}],
                    "auth": {{ "type": "bearer", "bearer": [{{ "key": "token", "value": "{{{{T}}}}" }}] }}
                  }},
                  "event": [
                    {{ "listen": "test", "script": {{ "exec": ["pm.test('ok', () => pm.response.to.have.status(200));"] }} }}
                  ]
                }}
              ]
            }}"#
        );
        let (ws, _) = parse(&json);
        let coll = &ws.collections[0];
        assert_eq!(coll.meta.name, "GitHub");
        let Item::Request(req) = &coll.items[0] else {
            panic!("expected request")
        };
        assert_eq!(req.meta.name, "list issues");
        let Protocol::Http(http) = &req.protocol else {
            panic!()
        };
        assert_eq!(http.method, Method::Get);
        assert_eq!(http.query.len(), 1);
        assert_eq!(http.headers[0].key, "Accept");
        match req.auth.as_ref().unwrap() {
            Auth::Bearer { token, .. } => assert_eq!(token, "{{T}}"),
            other => panic!("expected bearer, got {other:?}"),
        }
        assert_eq!(
            req.scripts.post_response.as_ref().unwrap().dialect,
            ScriptDialect::Pm
        );
    }

    #[test]
    fn v2_1_tolerates_plural_key_spellings() {
        // Some Postman-published collections spell header/response/event as plural
        // (headers/responses/events) with the same value shape. We must recover that data,
        // not silently read the collection empty.
        let json = format!(
            r#"{{
              "info": {{ "name": "Plural", "schema": "{V21_SCHEMA}" }},
              "item": [
                {{ "name": "get",
                  "request": {{
                    "method": "GET",
                    "url": "https://x.test/u",
                    "headers": [{{ "key": "Accept", "value": "application/json" }}]
                  }},
                  "events": [
                    {{ "listen": "prerequest", "script": {{ "exec": ["pm.environment.set('t', 1);"] }} }}
                  ],
                  "responses": [
                    {{ "name": "200", "code": 200, "status": "OK", "body": "{{}}" }}
                  ]
                }}
              ]
            }}"#
        );
        let (ws, _) = parse(&json);
        let Item::Request(req) = &ws.collections[0].items[0] else {
            panic!("expected request")
        };
        let Protocol::Http(http) = &req.protocol else {
            panic!()
        };
        // plural `headers` recovered (not hollow)
        assert_eq!(http.headers.len(), 1);
        assert_eq!(http.headers[0].key, "Accept");
        // plural `events` → pre-request script
        assert!(req.scripts.pre_request.is_some(), "plural events not read");
        // plural `responses` → saved example
        assert_eq!(req.examples.len(), 1, "plural responses not read");
    }

    #[test]
    fn v2_folders_become_nested_collections() {
        let json = format!(
            r#"{{ "info": {{ "name": "W", "schema": "{V21_SCHEMA}" }},
              "item": [ {{ "name": "folder", "item": [
                {{ "name": "r1", "request": {{ "method": "GET", "url": "https://x.test/1" }} }} ] }} ] }}"#
        );
        let (ws, _) = parse(&json);
        let Item::Collection(folder) = &ws.collections[0].items[0] else {
            panic!("expected folder → collection")
        };
        assert_eq!(folder.meta.name, "folder");
        assert_eq!(folder.items.len(), 1);
    }

    #[test]
    fn v2_shaped_without_schema_marker_fails_loud() {
        // Strict by default: no info.schema → reject (matches the app), don't guess v2.1.
        let mut r = Report::new(Fidelity::Lossless);
        assert!(
            parse_postman(r#"{ "info": { "name": "NoMarker" }, "item": [] }"#, &mut r).is_err()
        );
    }

    #[test]
    fn rq_3458_null_and_numeric_keys_are_coerced_not_fatal() {
        let json = format!(
            r#"{{ "info": {{ "name": "Nasty", "schema": "{V21_SCHEMA}" }},
              "item": [ {{ "name": "bad-keys", "request": {{
                "method": "POST", "url": "https://x.test/submit",
                "header": [
                  {{ "key": null, "value": "v1" }},
                  {{ "key": 42, "value": "v2" }},
                  {{ "key": "ok", "value": "v3" }},
                  {{ "key": {{ "nested": true }}, "value": "v4" }}
                ] }} }} ] }}"#
        );
        let (ws, report) = parse(&json);
        let Item::Request(req) = &ws.collections[0].items[0] else {
            panic!()
        };
        let Protocol::Http(http) = &req.protocol else {
            panic!()
        };
        assert_eq!(http.headers.len(), 3);
        assert_eq!(http.headers[0].key, "");
        assert_eq!(http.headers[1].key, "42");
        assert_eq!(http.headers[2].key, "ok");
        assert!(report.count(Severity::Coerced) >= 2);
        assert_eq!(report.count(Severity::Dropped), 1);
    }

    #[test]
    fn v2_urlencoded_and_raw_json_bodies() {
        let json = format!(
            r#"{{ "info": {{ "name": "B", "schema": "{V21_SCHEMA}" }},
              "item": [
                {{ "name": "j", "request": {{ "method": "POST", "url": "https://x.test/j",
                  "body": {{ "mode": "raw", "raw": "{{\"a\":1}}", "options": {{ "raw": {{ "language": "json" }} }} }} }} }},
                {{ "name": "f", "request": {{ "method": "POST", "url": "https://x.test/f",
                  "body": {{ "mode": "urlencoded", "urlencoded": [{{ "key": "a", "value": "1" }}] }} }} }}
              ] }}"#
        );
        let (ws, _) = parse(&json);
        let items = &ws.collections[0].items;
        let Item::Request(j) = &items[0] else {
            panic!()
        };
        let Protocol::Http(jh) = &j.protocol else {
            panic!()
        };
        match jh.body.as_ref().unwrap() {
            Body::Raw { media_type, .. } => assert_eq!(media_type, "application/json"),
            other => panic!("expected raw json, got {other:?}"),
        }
        let Item::Request(f) = &items[1] else {
            panic!()
        };
        let Protocol::Http(fh) = &f.protocol else {
            panic!()
        };
        assert!(matches!(fh.body.as_ref().unwrap(), Body::UrlEncoded { .. }));
    }

    // ---- v2.0 -------------------------------------------------------------------------

    #[test]
    fn v2_0_auth_object_shape_is_mapped() {
        // v2.0 auth params are an OBJECT, not v2.1's array — the one real v2.0/v2.1 diff.
        let json = r#"{
          "info": { "name": "V20", "schema": "https://schema.getpostman.com/json/collection/v2.0.0/collection.json" },
          "item": [ { "name": "r", "request": {
            "method": "GET", "url": "https://x.test/",
            "auth": { "type": "bearer", "bearer": { "token": "T20" } } } } ]
        }"#;
        let (ws, _) = parse(json);
        let Item::Request(req) = &ws.collections[0].items[0] else {
            panic!()
        };
        match req.auth.as_ref().unwrap() {
            Auth::Bearer { token, .. } => assert_eq!(token, "T20"),
            other => panic!("expected bearer from v2.0 object shape, got {other:?}"),
        }
    }

    // ---- v1.0.0 -----------------------------------------------------------------------

    #[test]
    fn v1_flat_requests_folders_and_header_string() {
        let json = r#"{
          "id": "col1", "name": "Legacy V1",
          "order": ["r-top"],
          "folders_order": ["f1"],
          "folders": [ { "id": "f1", "name": "Folder A", "order": ["r-in"] } ],
          "requests": [
            { "id": "r-top", "name": "Top", "method": "GET", "url": "https://x.test/top",
              "headers": "Accept: application/json\nX-Foo: bar" },
            { "id": "r-in", "name": "InFolder", "method": "POST", "url": "https://x.test/in",
              "dataMode": "raw", "rawModeData": "{\"a\":1}",
              "tests": "tests['ok'] = responseCode.code === 200;" }
          ]
        }"#;
        let (ws, _) = parse(json);
        let coll = &ws.collections[0];
        assert_eq!(coll.meta.name, "Legacy V1");
        // items: folder (sub-collection) + top-level request, in that construction order.
        let folder = coll
            .items
            .iter()
            .find_map(|it| match it {
                Item::Collection(c) if c.meta.name == "Folder A" => Some(c),
                _ => None,
            })
            .expect("folder A present");
        let Item::Request(infolder) = &folder.items[0] else {
            panic!("folder request")
        };
        assert_eq!(infolder.meta.name, "InFolder");
        let Protocol::Http(ih) = &infolder.protocol else {
            panic!()
        };
        assert!(matches!(ih.body.as_ref().unwrap(), Body::Raw { .. }));
        assert!(infolder.scripts.post_response.is_some());

        let top = coll
            .items
            .iter()
            .find_map(|it| match it {
                Item::Request(r) if r.meta.name == "Top" => Some(r),
                _ => None,
            })
            .expect("top request present");
        let Protocol::Http(th) = &top.protocol else {
            panic!()
        };
        assert_eq!(th.method, Method::Get);
        // header-string parsed into two kv pairs
        assert_eq!(th.headers.len(), 2);
        assert_eq!(th.headers[0].key, "Accept");
        assert_eq!(th.headers[0].value, "application/json");
        assert_eq!(th.headers[1].key, "X-Foo");
    }

    #[test]
    fn v1_urlencoded_body() {
        let json = r#"{
          "id": "c", "name": "V1 Form", "order": ["r1"],
          "requests": [ { "id": "r1", "name": "form", "method": "POST", "url": "https://x.test/f",
            "dataMode": "urlencoded", "data": [ { "key": "a", "value": "1", "enabled": true } ] } ]
        }"#;
        let (ws, _) = parse(json);
        let Item::Request(req) = &ws.collections[0].items[0] else {
            panic!()
        };
        let Protocol::Http(h) = &req.protocol else {
            panic!()
        };
        match h.body.as_ref().unwrap() {
            Body::UrlEncoded { fields } => {
                assert_eq!(fields[0].key, "a");
                assert_eq!(fields[0].value, "1");
            }
            other => panic!("expected urlencoded, got {other:?}"),
        }
    }

    // ---- dispatch ---------------------------------------------------------------------

    #[test]
    fn not_a_collection_errors() {
        let mut r = Report::new(Fidelity::Lossless);
        assert!(parse_postman(r#"{"foo":"bar"}"#, &mut r).is_err());
    }
}
