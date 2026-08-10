//! IR → Postman Collection **v2.1** JSON — the reverse of the v2.1 importer.
//!
//! Purpose: the **round-trip completeness check**. Postman → IR → Postman lets us diff the
//! re-emitted collection against the original and mechanically find any field we silently
//! drop (see `tests/postman_roundtrip.rs`). It aims for semantic equivalence (canonical
//! v2.1 output), not byte-identity.
//!
//! This is also the seed of cross-q's Postman *exporter* (a real product direction — a
//! converter must export to Postman, not only Requestly).

use serde_json::{json, Map, Value};

use cq_model::{Auth, Body, Collection, FormField, Item, Protocol, Request, Scripts};
use cq_model::{KeyValue, Url, Variable, Workspace};

const V21_SCHEMA: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

/// Emit a [`Workspace`] as a Postman v2.1 collection. Postman-sourced workspaces wrap
/// exactly one collection (`collections[0]`); we reconstruct that.
pub fn to_postman(ws: &Workspace) -> Value {
    let Some(root) = ws.collections.first() else {
        return json!({ "info": { "name": ws.meta.name, "schema": V21_SCHEMA }, "item": [] });
    };

    let mut info = Map::new();
    info.insert("name".into(), json!(root.meta.name));
    info.insert("_postman_id".into(), json!(root.meta.id));
    if let Some(desc) = &root.meta.description {
        info.insert("description".into(), json!(desc));
    }
    info.insert("schema".into(), json!(V21_SCHEMA));

    let mut out = Map::new();
    out.insert("info".into(), Value::Object(info));
    out.insert(
        "item".into(),
        Value::Array(root.items.iter().map(emit_item).collect()),
    );
    if let Some(a) = auth_value(&root.auth) {
        out.insert("auth".into(), a);
    }
    if let Some(ev) = event_array(&root.scripts) {
        out.insert("event".into(), ev);
    }
    if !root.variables.is_empty() {
        out.insert("variable".into(), variables_value(&root.variables));
    }
    Value::Object(out)
}

fn emit_item(item: &Item) -> Value {
    match item {
        Item::Collection(folder) => emit_folder(folder),
        Item::Request(req) => emit_request(req),
    }
}

fn emit_folder(folder: &Collection) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(folder.meta.id));
    m.insert("name".into(), json!(folder.meta.name));
    m.insert(
        "item".into(),
        Value::Array(folder.items.iter().map(emit_item).collect()),
    );
    if let Some(a) = auth_value(&folder.auth) {
        m.insert("auth".into(), a);
    }
    if let Some(ev) = event_array(&folder.scripts) {
        m.insert("event".into(), ev);
    }
    Value::Object(m)
}

fn emit_request(req: &Request) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(req.meta.id));
    m.insert("name".into(), json!(req.meta.name));
    let Protocol::Http(http) = &req.protocol else {
        // non-HTTP protocols aren't representable in a Postman v2.1 request; emit the name
        // shell (the round-trip check will flag the loss).
        return Value::Object(m);
    };

    let mut request = Map::new();
    request.insert("method".into(), json!(String::from(http.method.clone())));
    request.insert(
        "url".into(),
        url_value(&http.url, &http.query, &http.path_variables),
    );
    if !http.headers.is_empty() {
        request.insert("header".into(), headers_value(&http.headers));
    }
    if let Some(body) = &http.body {
        if let Some(b) = body_value(body) {
            request.insert("body".into(), b);
        }
    }
    if let Some(a) = auth_value(&req.auth) {
        request.insert("auth".into(), a);
    }
    if let Some(desc) = &req.meta.description {
        request.insert("description".into(), json!(desc));
    }
    m.insert("request".into(), Value::Object(request));
    if let Some(ev) = event_array(&req.scripts) {
        m.insert("event".into(), ev);
    }
    // Saved responses were captured verbatim on import — emit them back unchanged.
    let responses: Vec<Value> = req
        .examples
        .iter()
        .filter_map(|e| e.response.clone())
        .collect();
    if !responses.is_empty() {
        m.insert("response".into(), Value::Array(responses));
    }
    Value::Object(m)
}

fn url_value(url: &Url, query: &[KeyValue], path_vars: &[cq_model::PathVar]) -> Value {
    let mut m = Map::new();
    m.insert("raw".into(), json!(url.raw));
    if !query.is_empty() {
        m.insert("query".into(), headers_value(query));
    }
    if !path_vars.is_empty() {
        let vars: Vec<Value> = path_vars
            .iter()
            .map(|pv| {
                let mut o = Map::new();
                o.insert("key".into(), json!(pv.key));
                o.insert("value".into(), json!(pv.value));
                if let Some(d) = &pv.description {
                    o.insert("description".into(), json!(d));
                }
                Value::Object(o)
            })
            .collect();
        m.insert("variable".into(), Value::Array(vars));
    }
    Value::Object(m)
}

fn headers_value(kvs: &[KeyValue]) -> Value {
    Value::Array(
        kvs.iter()
            .map(|kv| {
                let mut m = Map::new();
                m.insert("key".into(), json!(kv.key));
                m.insert("value".into(), json!(kv.value));
                if !kv.enabled {
                    m.insert("disabled".into(), json!(true));
                }
                if let Some(d) = &kv.description {
                    m.insert("description".into(), json!(d));
                }
                Value::Object(m)
            })
            .collect(),
    )
}

fn body_value(body: &Body) -> Option<Value> {
    match body {
        Body::None => None,
        Body::Raw { text, media_type } => {
            let lang = media_type_to_language(media_type);
            Some(json!({
                "mode": "raw",
                "raw": text,
                "options": { "raw": { "language": lang } }
            }))
        }
        Body::UrlEncoded { fields } => Some(json!({
            "mode": "urlencoded",
            "urlencoded": headers_value(fields),
        })),
        Body::FormData { fields } => {
            let arr: Vec<Value> = fields
                .iter()
                .filter_map(|f| match f {
                    FormField::Text(kv) => {
                        let mut m = Map::new();
                        m.insert("key".into(), json!(kv.key));
                        m.insert("value".into(), json!(kv.value));
                        m.insert("type".into(), json!("text"));
                        Some(Value::Object(m))
                    }
                    FormField::File(_) => None,
                })
                .collect();
            Some(json!({ "mode": "formdata", "formdata": arr }))
        }
        Body::Graphql {
            query, variables, ..
        } => Some(json!({
            "mode": "graphql",
            "graphql": { "query": query, "variables": variables }
        })),
        Body::Binary { .. } => Some(json!({ "mode": "file" })),
    }
}

fn media_type_to_language(media_type: &str) -> &'static str {
    if media_type.contains("json") {
        "json"
    } else if media_type.contains("xml") {
        "xml"
    } else if media_type.contains("html") {
        "html"
    } else if media_type.contains("javascript") {
        "javascript"
    } else {
        "text"
    }
}

/// Reverse of the v2.1 auth import: params become an **array** of `{key,value}` under a
/// key named after the type. `None` (unspecified) emits nothing; `Auth::Inherit` →
/// `inherit`; `Auth::None` → `noauth`.
fn auth_value(auth: &Option<Auth>) -> Option<Value> {
    let auth = auth.as_ref()?;
    let arr = |ty: &str, pairs: Vec<(&str, &str)>| {
        let items: Vec<Value> = pairs
            .into_iter()
            .map(|(k, v)| json!({ "key": k, "value": v, "type": "string" }))
            .collect();
        json!({ "type": ty, ty: items })
    };
    Some(match auth {
        Auth::None => json!({ "type": "noauth" }),
        Auth::Inherit => json!({ "type": "inherit" }),
        Auth::Basic { username, password } => arr(
            "basic",
            vec![("username", username), ("password", password)],
        ),
        Auth::Bearer { token, .. } => arr("bearer", vec![("token", token)]),
        Auth::ApiKey {
            key,
            value,
            placement,
        } => {
            let in_ = match placement {
                cq_model::ApiKeyPlacement::Query => "query",
                cq_model::ApiKeyPlacement::Header => "header",
            };
            arr("apikey", vec![("key", key), ("value", value), ("in", in_)])
        }
        Auth::OAuth2 { params, .. }
        | Auth::OAuth1 { params }
        | Auth::Digest { params }
        | Auth::Hawk { params }
        | Auth::AwsSigV4 { params }
        | Auth::Ntlm { params }
        | Auth::JwtBearer { params, .. }
        | Auth::EdgeGrid { params } => {
            let ty = auth_type_name(auth);
            let items: Vec<Value> = params
                .iter()
                .map(|(k, v)| json!({ "key": k, "value": v, "type": "string" }))
                .collect();
            json!({ "type": ty, ty: items })
        }
        Auth::Unknown { raw, .. } => raw.clone(),
    })
}

fn auth_type_name(auth: &Auth) -> &'static str {
    match auth {
        Auth::OAuth2 { .. } => "oauth2",
        Auth::OAuth1 { .. } => "oauth1",
        Auth::Digest { .. } => "digest",
        Auth::Hawk { .. } => "hawk",
        Auth::AwsSigV4 { .. } => "awsv4",
        Auth::Ntlm { .. } => "ntlm",
        Auth::JwtBearer { .. } => "jwt",
        Auth::EdgeGrid { .. } => "edgegrid",
        _ => "noauth",
    }
}

fn event_array(scripts: &Scripts) -> Option<Value> {
    if scripts.is_empty() {
        return None;
    }
    let mut arr = Vec::new();
    if let Some(s) = &scripts.pre_request {
        arr.push(json!({
            "listen": "prerequest",
            "script": { "type": "text/javascript", "exec": s.source.split('\n').collect::<Vec<_>>() }
        }));
    }
    if let Some(s) = &scripts.post_response {
        arr.push(json!({
            "listen": "test",
            "script": { "type": "text/javascript", "exec": s.source.split('\n').collect::<Vec<_>>() }
        }));
    }
    Some(Value::Array(arr))
}

fn variables_value(vars: &[Variable]) -> Value {
    Value::Array(
        vars.iter()
            .map(|v| json!({ "key": v.key, "value": v.value, "type": "string" }))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use cq_report::{Fidelity, Report};

    // A canonical Postman v2.1 collection (singular `header`/`response`/`event`, per the
    // official schema) — round-trips through IR and back, preserving examples, url path
    // variables, and item ids. (The transformer `examples/` corpus uses a *non-canonical*
    // plural dialect and is not a valid oracle for canonical v2.1.)
    const CANONICAL: &str = r#"{
      "info": { "name": "C", "_postman_id": "col-1",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
      "item": [
        { "id": "req-1", "name": "get user",
          "request": {
            "method": "GET",
            "url": { "raw": "https://x.test/users/:id", "variable": [{ "key": "id", "value": "42" }] },
            "header": [{ "key": "Accept", "value": "application/json" }]
          },
          "response": [
            { "name": "200 OK", "code": 200, "status": "OK", "body": "{\"ok\":true}",
              "header": [{ "key": "X-Trace", "value": "abc" }] }
          ]
        }
      ]
    }"#;

    #[test]
    fn canonical_v21_roundtrip_preserves_examples_pathvars_ids() {
        let mut report = Report::new(Fidelity::Lossless);
        let ws = crate::postman::parse_postman(CANONICAL, &mut report).expect("parse");
        let out = super::to_postman(&ws);

        let item = &out["item"][0];
        assert_eq!(item["id"], serde_json::json!("req-1"), "item id preserved");
        // url path variable survived
        assert_eq!(
            item["request"]["url"]["variable"][0]["key"],
            serde_json::json!("id")
        );
        // saved response (example) survived verbatim, incl. nested fields
        assert_eq!(item["response"][0]["code"], serde_json::json!(200));
        assert_eq!(item["response"][0]["status"], serde_json::json!("OK"));
        assert_eq!(
            item["response"][0]["header"][0]["key"],
            serde_json::json!("X-Trace")
        );
    }
}
