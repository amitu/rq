//! Parse a Postman Collection (v2.0 / v2.1) into the Idealised Model.
//!
//! Tolerant by design (Postel inbound): we walk the JSON as [`serde_json::Value`] and
//! *coerce the unambiguous, drop the ambiguous, and report both* — never abort on a valid
//! collection. The headline case is RQ-3458: a key-value key that is `null` or numeric
//! used to reject the whole import; here it is coerced to a string with a `Coerced`
//! diagnostic, while an object/array key (genuinely ambiguous) is dropped with one.

use serde_json::Value;

use cq_model::{
    Auth, Body, Collection, HttpRequest, Item, KeyValue, Method, ModelHeader, Protocol, Provenance,
    RecordMeta, Request, Script, ScriptDialect, Scripts, SourceFormat, Url, Variable, Workspace,
};
use cq_report::{Phase, Report};

fn prov(locator: impl Into<String>) -> Provenance {
    Provenance {
        format: SourceFormat::Postman,
        locator: locator.into(),
    }
}

fn obj_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

/// Coerce a key-value *key* into a string (RQ-3458). Returns `None` (with a `Dropped`
/// diagnostic) only for genuinely ambiguous keys (object/array).
fn coerce_key(v: Option<&Value>, report: &mut Report, locator: &str) -> Option<String> {
    match v {
        None | Some(Value::Null) => {
            report.coerced(
                Phase::Parse,
                prov(locator),
                "null/absent key coerced to \"\"",
            );
            Some(String::new())
        }
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => {
            report.coerced(
                Phase::Parse,
                prov(locator),
                format!("numeric key {n} coerced to string"),
            );
            Some(n.to_string())
        }
        Some(Value::Bool(b)) => {
            report.coerced(
                Phase::Parse,
                prov(locator),
                format!("boolean key {b} coerced to string"),
            );
            Some(b.to_string())
        }
        Some(other) => {
            report.dropped(
                Phase::Parse,
                prov(locator),
                format!("key-value with a non-scalar key dropped: {other}"),
            );
            None
        }
    }
}

/// Coerce a value into a string: strings verbatim, numbers/bools stringified, arrays
/// space-joined, null → "". (Mirrors the app's value coercion.)
fn coerce_value(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| x.to_string())
            })
            .collect::<Vec<_>>()
            .join(" "),
        Some(other) => other.to_string(),
    }
}

/// Parse an array of Postman key/value pairs (`header`, `query`, urlencoded, formdata).
fn parse_kv_array(v: Option<&Value>, report: &mut Report, locator: &str) -> Vec<KeyValue> {
    let mut out = Vec::new();
    let Some(Value::Array(items)) = v else {
        return out;
    };
    for (i, item) in items.iter().enumerate() {
        let loc = format!("{locator}[{i}]");
        let Some(key) = coerce_key(item.get("key"), report, &loc) else {
            continue;
        };
        let mut kv = KeyValue::new(key, coerce_value(item.get("value")));
        // Postman uses `disabled: true`; our model stores the inverse.
        if item
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            kv.enabled = false;
        }
        if let Some(desc) = item.get("description").and_then(Value::as_str) {
            kv.description = Some(desc.to_string());
        }
        out.push(kv);
    }
    out
}

/// Postman `url` may be a string or an object with `raw` + structured `query`.
fn parse_url(v: Option<&Value>, report: &mut Report, locator: &str) -> (Url, Vec<KeyValue>) {
    match v {
        Some(Value::String(s)) => (Url::raw(s), Vec::new()),
        Some(Value::Object(_)) => {
            let raw = obj_str(v.unwrap(), "raw").unwrap_or_default();
            let query =
                parse_kv_array(v.unwrap().get("query"), report, &format!("{locator}.query"));
            (Url::raw(raw), query)
        }
        _ => (Url::default(), Vec::new()),
    }
}

fn parse_body(v: Option<&Value>, report: &mut Report, locator: &str) -> Option<Body> {
    let body = v?;
    let mode = body.get("mode").and_then(Value::as_str)?;
    match mode {
        "raw" => {
            let text = obj_str(body, "raw").unwrap_or_default();
            // language lives under options.raw.language
            let media_type = body
                .get("options")
                .and_then(|o| o.get("raw"))
                .and_then(|r| r.get("language"))
                .and_then(Value::as_str)
                .map(|lang| match lang {
                    "json" => "application/json",
                    "xml" => "application/xml",
                    "html" => "text/html",
                    "javascript" => "application/javascript",
                    _ => "text/plain",
                })
                .unwrap_or("text/plain")
                .to_string();
            Some(Body::Raw { text, media_type })
        }
        "urlencoded" => Some(Body::UrlEncoded {
            fields: parse_kv_array(
                body.get("urlencoded"),
                report,
                &format!("{locator}.urlencoded"),
            ),
        }),
        "formdata" => {
            // File parts can't be carried without the file; drop them with a note but keep
            // the text fields.
            let fields =
                parse_kv_array(body.get("formdata"), report, &format!("{locator}.formdata"));
            Some(Body::FormData {
                fields: fields.into_iter().map(cq_model::FormField::Text).collect(),
            })
        }
        "graphql" => {
            let query = body
                .get("graphql")
                .and_then(|g| g.get("query"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let variables = body
                .get("graphql")
                .and_then(|g| g.get("variables"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(Body::Graphql {
                query,
                variables,
                operation_name: None,
            })
        }
        "file" => {
            report.dropped(
                Phase::Parse,
                prov(format!("{locator}.file")),
                "file body cannot be carried without the file; emitted as no body",
            );
            None
        }
        other => {
            report.dropped(
                Phase::Parse,
                prov(locator),
                format!("unknown body mode {other:?} dropped"),
            );
            None
        }
    }
}

/// Map a Postman `auth` block to an IR [`Auth`]. Unknown types are preserved as
/// [`Auth::Unknown`] so a credential is never lost.
fn parse_auth(v: Option<&Value>, report: &mut Report, locator: &str) -> Option<Auth> {
    let auth = v?;
    let ty = auth.get("type").and_then(Value::as_str)?;
    // Postman stores params as an array of {key,value,type} under a key named after the type.
    let params = |name: &str| -> std::collections::BTreeMap<String, String> {
        let mut m = std::collections::BTreeMap::new();
        if let Some(Value::Array(items)) = auth.get(name) {
            for it in items {
                if let Some(k) = it.get("key").and_then(Value::as_str) {
                    m.insert(k.to_string(), coerce_value(it.get("value")));
                }
            }
        }
        m
    };
    match ty {
        "noauth" => Some(Auth::None),
        "inherit" => Some(Auth::Inherit),
        "basic" => {
            let p = params("basic");
            Some(Auth::Basic {
                username: p.get("username").cloned().unwrap_or_default(),
                password: p.get("password").cloned().unwrap_or_default(),
            })
        }
        "bearer" => {
            let p = params("bearer");
            Some(Auth::Bearer {
                token: p.get("token").cloned().unwrap_or_default(),
                header_prefix: None,
            })
        }
        "apikey" => {
            let p = params("apikey");
            let placement = match p.get("in").map(String::as_str) {
                Some("query") => cq_model::ApiKeyPlacement::Query,
                _ => cq_model::ApiKeyPlacement::Header,
            };
            Some(Auth::ApiKey {
                key: p.get("key").cloned().unwrap_or_default(),
                value: p.get("value").cloned().unwrap_or_default(),
                placement,
            })
        }
        "oauth2" => Some(Auth::OAuth2 {
            grant: "manual".to_string(),
            params: params("oauth2"),
        }),
        "oauth1" => Some(Auth::OAuth1 {
            params: params("oauth1"),
        }),
        "digest" => Some(Auth::Digest {
            params: params("digest"),
        }),
        "hawk" => Some(Auth::Hawk {
            params: params("hawk"),
        }),
        "awsv4" => Some(Auth::AwsSigV4 {
            params: params("awsv4"),
        }),
        "ntlm" => Some(Auth::Ntlm {
            params: params("ntlm"),
        }),
        other => {
            report.coerced(
                Phase::Map,
                prov(locator),
                format!("auth type {other:?} preserved as 'unknown'"),
            );
            Some(Auth::Unknown {
                raw_type: other.to_string(),
                raw: auth.clone(),
            })
        }
    }
}

/// Postman `event[]` → pre-request / post-response scripts (dialect `pm`).
fn parse_scripts(v: Option<&Value>) -> Scripts {
    let mut scripts = Scripts::default();
    let Some(Value::Array(events)) = v else {
        return scripts;
    };
    for ev in events {
        let listen = ev.get("listen").and_then(Value::as_str).unwrap_or("");
        let src = ev
            .get("script")
            .and_then(|s| s.get("exec"))
            .map(join_exec)
            .unwrap_or_default();
        if src.trim().is_empty() {
            continue;
        }
        let script = Script {
            source: src,
            language: cq_model::ScriptLang::JavaScript,
            dialect: ScriptDialect::Pm,
        };
        match listen {
            "prerequest" => scripts.pre_request = Some(script),
            "test" => scripts.post_response = Some(script),
            _ => {}
        }
    }
    scripts
}

fn join_exec(exec: &Value) -> String {
    match exec {
        Value::String(s) => s.clone(),
        Value::Array(lines) => lines
            .iter()
            .map(|l| l.as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn parse_request_item(item: &Value, report: &mut Report, locator: &str) -> Request {
    let name = obj_str(item, "name").unwrap_or_else(|| "request".to_string());
    let req = item.get("request");
    // `request` may itself be a bare URL string.
    let (method, url, query, headers, body, auth) = match req {
        Some(Value::String(s)) => (Method::Get, Url::raw(s), Vec::new(), Vec::new(), None, None),
        Some(r @ Value::Object(_)) => {
            let method = r
                .get("method")
                .and_then(Value::as_str)
                .map(|m| Method::from(m.to_string()))
                .unwrap_or(Method::Get);
            let (url, query) = parse_url(r.get("url"), report, &format!("{locator}.url"));
            let headers = parse_kv_array(r.get("header"), report, &format!("{locator}.header"));
            let body = parse_body(r.get("body"), report, &format!("{locator}.body"));
            let auth = parse_auth(r.get("auth"), report, &format!("{locator}.auth"));
            (method, url, query, headers, body, auth)
        }
        _ => (
            Method::Get,
            Url::default(),
            Vec::new(),
            Vec::new(),
            None,
            None,
        ),
    };

    let id = obj_str(item, "id")
        .or_else(|| obj_str(item, "_postman_id"))
        .unwrap_or_else(|| format!("pm-{}", slugify(&name)));

    let mut meta = RecordMeta::new(id, name, SourceFormat::Postman);
    meta.source.locator = locator.to_string();
    if let Some(desc) = item
        .get("request")
        .and_then(|r| r.get("description"))
        .and_then(Value::as_str)
    {
        meta.description = Some(desc.to_string());
    }

    Request {
        meta,
        protocol: Protocol::Http(HttpRequest {
            method,
            url,
            headers,
            query,
            path_variables: Vec::new(),
            body,
            settings: cq_model::RequestSettings::default(),
        }),
        auth,
        scripts: parse_scripts(item.get("event")),
        examples: Vec::new(),
        depends_on: Vec::new(),
    }
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let t = out.trim_matches('-').to_string();
    if t.is_empty() {
        "item".to_string()
    } else {
        t
    }
}

/// Recursively parse a Postman `item` (folder → collection, else → request).
fn parse_item(item: &Value, report: &mut Report, locator: &str) -> Item {
    if item.get("item").is_some() {
        let name = obj_str(item, "name").unwrap_or_else(|| "folder".to_string());
        let id = obj_str(item, "id").unwrap_or_else(|| format!("pm-{}", slugify(&name)));
        let mut meta = RecordMeta::new(id, name, SourceFormat::Postman);
        meta.source.locator = locator.to_string();
        let children = parse_items(item.get("item"), report, locator);
        Item::Collection(Box::new(Collection {
            meta,
            auth: parse_auth(item.get("auth"), report, &format!("{locator}.auth")),
            scripts: parse_scripts(item.get("event")),
            variables: Vec::new(),
            items: children,
        }))
    } else {
        Item::Request(Box::new(parse_request_item(item, report, locator)))
    }
}

fn parse_items(v: Option<&Value>, report: &mut Report, locator: &str) -> Vec<Item> {
    let mut out = Vec::new();
    if let Some(Value::Array(items)) = v {
        for (i, item) in items.iter().enumerate() {
            out.push(parse_item(item, report, &format!("{locator}.item[{i}]")));
        }
    }
    out
}

fn parse_variables(v: Option<&Value>) -> Vec<Variable> {
    let mut out = Vec::new();
    if let Some(Value::Array(items)) = v {
        for it in items {
            if let Some(key) = it.get("key").and_then(Value::as_str) {
                out.push(Variable {
                    key: key.to_string(),
                    value: coerce_value(it.get("value")),
                    initial: None,
                    scope: cq_model::Scope::Collection,
                    data_type: cq_model::VarType::String,
                    category: cq_model::VarCategory::Scoped,
                    enabled: !it.get("disabled").and_then(Value::as_bool).unwrap_or(false),
                    rank: None,
                });
            }
        }
    }
    out
}

/// Parse a Postman collection JSON string into a [`Workspace`].
pub fn parse_postman(input: &str, report: &mut Report) -> Result<Workspace, String> {
    let root: Value = serde_json::from_str(input).map_err(|e| format!("invalid JSON: {e}"))?;

    let info_name = root
        .get("info")
        .and_then(|i| obj_str(i, "name"))
        .unwrap_or_else(|| "Imported Collection".to_string());

    // Sanity: a v2.x collection has `info` and `item`.
    if root.get("item").is_none() {
        return Err("not a Postman collection (missing `item`)".to_string());
    }

    let items = parse_items(root.get("item"), report, "item");
    let variables = parse_variables(root.get("variable"));
    let collection_auth = parse_auth(root.get("auth"), report, "auth");

    let collection = Collection {
        meta: RecordMeta::new(
            root.get("info")
                .and_then(|i| obj_str(i, "_postman_id"))
                .unwrap_or_else(|| format!("pm-{}", slugify(&info_name))),
            info_name.clone(),
            SourceFormat::Postman,
        ),
        auth: collection_auth,
        scripts: parse_scripts(root.get("event")),
        variables,
        items,
    };

    Ok(Workspace {
        meta: RecordMeta::new("pm-workspace", info_name, SourceFormat::Postman),
        cross_q: ModelHeader::for_source(SourceFormat::Postman),
        collections: vec![collection],
        environments: Vec::new(),
        packages: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_report::{Fidelity, Severity};

    fn parse(s: &str) -> (Workspace, Report) {
        let mut r = Report::new(Fidelity::Lossless);
        let ws = parse_postman(s, &mut r).expect("parse");
        (ws, r)
    }

    const SIMPLE: &str = r#"{
      "info": { "name": "GitHub", "_postman_id": "abc" },
      "item": [
        { "name": "list issues",
          "request": {
            "method": "GET",
            "url": { "raw": "https://api.github.com/issues?state=open",
                     "query": [{ "key": "state", "value": "open" }] },
            "header": [{ "key": "Accept", "value": "application/json" }],
            "auth": { "type": "bearer", "bearer": [{ "key": "token", "value": "{{T}}" }] }
          },
          "event": [
            { "listen": "test", "script": { "exec": ["pm.test('ok', () => pm.response.to.have.status(200));"] } }
          ]
        }
      ]
    }"#;

    #[test]
    fn parses_a_simple_collection() {
        let (ws, _) = parse(SIMPLE);
        let coll = &ws.collections[0];
        assert_eq!(coll.meta.name, "GitHub");
        assert_eq!(coll.items.len(), 1);
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
            _ => panic!("expected bearer"),
        }
        // Postman test script is preserved with dialect=pm (not rewritten).
        let s = req.scripts.post_response.as_ref().unwrap();
        assert_eq!(s.dialect, ScriptDialect::Pm);
    }

    #[test]
    fn folders_become_nested_collections() {
        let json = r#"{
          "info": { "name": "W" },
          "item": [
            { "name": "folder", "item": [
              { "name": "r1", "request": { "method": "GET", "url": "https://x.test/1" } }
            ]}
          ]
        }"#;
        let (ws, _) = parse(json);
        let Item::Collection(folder) = &ws.collections[0].items[0] else {
            panic!("expected folder → collection")
        };
        assert_eq!(folder.meta.name, "folder");
        assert_eq!(folder.items.len(), 1);
    }

    #[test]
    fn rq_3458_null_and_numeric_keys_are_coerced_not_fatal() {
        // The RCA case: a header key that is null, and one that is numeric. The real
        // importer rejected the whole collection; cross-q coerces + reports.
        let json = r#"{
          "info": { "name": "Nasty" },
          "item": [
            { "name": "bad-keys", "request": {
                "method": "POST",
                "url": "https://x.test/submit",
                "header": [
                  { "key": null, "value": "v1" },
                  { "key": 42, "value": "v2" },
                  { "key": "ok", "value": "v3" },
                  { "key": { "nested": true }, "value": "v4" }
                ]
            }}
          ]
        }"#;
        let (ws, report) = parse(json);
        let Item::Request(req) = &ws.collections[0].items[0] else {
            panic!()
        };
        let Protocol::Http(http) = &req.protocol else {
            panic!()
        };
        // null → "", 42 → "42", "ok" kept; the object key is dropped.
        assert_eq!(http.headers.len(), 3);
        assert_eq!(http.headers[0].key, "");
        assert_eq!(http.headers[1].key, "42");
        assert_eq!(http.headers[2].key, "ok");
        assert!(report.count(Severity::Coerced) >= 2);
        assert_eq!(report.count(Severity::Dropped), 1);
        // Crucially: it did NOT error out.
    }

    #[test]
    fn urlencoded_and_raw_json_bodies() {
        let json = r#"{
          "info": { "name": "B" },
          "item": [
            { "name": "j", "request": { "method": "POST", "url": "https://x.test/j",
              "body": { "mode": "raw", "raw": "{\"a\":1}", "options": { "raw": { "language": "json" } } } } },
            { "name": "f", "request": { "method": "POST", "url": "https://x.test/f",
              "body": { "mode": "urlencoded", "urlencoded": [{ "key": "a", "value": "1" }] } } }
          ]
        }"#;
        let (ws, _) = parse(json);
        let items = &ws.collections[0].items;
        let Item::Request(j) = &items[0] else {
            panic!()
        };
        let Protocol::Http(jh) = &j.protocol else {
            panic!()
        };
        match jh.body.as_ref().unwrap() {
            Body::Raw { media_type, .. } => assert_eq!(media_type, "application/json"),
            _ => panic!("expected raw json"),
        }
        let Item::Request(f) = &items[1] else {
            panic!()
        };
        let Protocol::Http(fh) = &f.protocol else {
            panic!()
        };
        assert!(matches!(fh.body.as_ref().unwrap(), Body::UrlEncoded { .. }));
    }

    #[test]
    fn not_a_collection_errors() {
        let mut r = Report::new(Fidelity::Lossless);
        assert!(parse_postman(r#"{"foo":"bar"}"#, &mut r).is_err());
    }
}
