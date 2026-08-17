//! Emit a [`Workspace`] as an in-memory Requestly **`MappedItems`** bundle — the shape
//! `@requestly/importers` returns from every `parseX` and hands to `sdk.import.fromMappedItems`.
//!
//! This is the contract a WASM build of cross-q binds to behind the app's ADR-196 seam
//! (see `~/bs/strategy/integration/…`). `MappedItems` is:
//! `{ collections?, requests?, examples?, environments? }`, each a `bulkCreate*Item`
//! (`{ tempId, parentId, name, description?, rank?, data }`). Uses the shared IR→Requestly
//! mapper in [`crate::rq_shape`], so it can never diverge from the `LOCAL_FS` emitter.

use serde_json::{json, Value};

use cq_model::{Collection, Environment, HttpRequest, Item, Protocol, Request, Workspace};
use cq_report::{Diagnostic, Phase, Report, Severity};

use crate::rq_shape::{self, AuthMap};

/// Build the Requestly `MappedItems` JSON bundle from a workspace, recording emit-phase
/// diagnostics for anything that can't be mapped.
pub fn to_mapped_items(ws: &Workspace, report: &mut Report) -> Value {
    let mut collections = Vec::new();
    let mut requests = Vec::new();
    let mut examples = Vec::new();
    let mut environments = Vec::new();
    let mut cookies = Vec::new();

    for coll in &ws.collections {
        walk_collection(
            coll,
            None,
            &mut collections,
            &mut requests,
            &mut examples,
            &mut cookies,
            report,
        );
    }
    for env in &ws.environments {
        environments.push(environment_item(env));
    }

    // Only include non-empty bundles (matches how the app omits absent kinds).
    let mut obj = serde_json::Map::new();
    if !collections.is_empty() {
        obj.insert("collections".into(), Value::Array(collections));
    }
    if !requests.is_empty() {
        obj.insert("requests".into(), Value::Array(requests));
    }
    if !examples.is_empty() {
        obj.insert("examples".into(), Value::Array(examples));
    }
    if !environments.is_empty() {
        obj.insert("environments".into(), Value::Array(environments));
    }
    // Saved-response cookies ride `mapped.cookies` (device-local jar, never bulk.create —
    // ADR-105/106), deduped last-write-wins across the whole tree.
    let cookies = rq_shape::dedupe_cookies(cookies);
    if !cookies.is_empty() {
        obj.insert("cookies".into(), Value::Array(cookies));
    }
    Value::Object(obj)
}

/// Walk a collection, appending its records to the per-kind bundles. A collection with an
/// empty name is the synthetic root — it produces no record, and its items inherit the
/// incoming parent (root → `parentId: null`).
#[allow(clippy::too_many_arguments)]
fn walk_collection(
    coll: &Collection,
    parent_temp: Option<&str>,
    collections: &mut Vec<Value>,
    requests: &mut Vec<Value>,
    examples: &mut Vec<Value>,
    cookies: &mut Vec<Value>,
    report: &mut Report,
) {
    let this_temp: Option<String> = if coll.meta.name.is_empty() {
        parent_temp.map(str::to_string)
    } else {
        let temp = coll.meta.id.clone();
        let mut data = serde_json::Map::new();
        if !coll.variables.is_empty() {
            data.insert(
                "variables".into(),
                rq_shape::variables_record(&coll.variables),
            );
        }
        // Requestly convention: a collection always carries an auth (unspecified →
        // `inherit`), applied here in the reverse converter — see `requestly_auth_value`.
        if let Some(v) = rq_shape::requestly_auth_value(&coll.auth) {
            data.insert("auth".into(), v);
        }
        // A collection-level auth kind with no Requestly equivalent (e.g. edgegrid) falls back
        // to inherit AND is flagged advanced_auth, matching the app's mapper.
        if let Some(a) = &coll.auth {
            if let AuthMap::Unsupported(desc) = rq_shape::auth_to_rq(a) {
                report.warn(
                    Severity::Coerced,
                    Phase::Emit,
                    coll.meta.source.clone(),
                    "advanced_auth",
                    format!("collection auth kind '{desc}' has no Requestly equivalent; → inherit"),
                );
            }
        }
        if let Some(scripts) = rq_shape::scripts_object(&coll.scripts) {
            data.insert("scripts".into(), scripts);
        }

        let mut item = serde_json::Map::new();
        item.insert("tempId".into(), json!(temp));
        item.insert("parentId".into(), parent_ref(parent_temp));
        item.insert(
            "name".into(),
            json!(rq_shape::truncate_name(
                &coll.meta.name,
                rq_shape::MAX_NAME_LENGTH
            )),
        );
        if let Some(d) = &coll.meta.description {
            item.insert("description".into(), json!(d));
        }
        if let Some(r) = &coll.meta.rank {
            item.insert("rank".into(), json!(r));
        }
        if !data.is_empty() {
            item.insert("data".into(), Value::Object(data));
        }
        collections.push(Value::Object(item));
        Some(temp)
    };

    let child_parent = this_temp.as_deref();
    for it in &coll.items {
        match it {
            Item::Request(req) => {
                if let Some(r) = request_item(req, child_parent, report) {
                    requests.push(r);
                    // Saved responses ride alongside as examples, parented to this request.
                    if let Protocol::Http(parent_http) = &req.protocol {
                        for (i, ex) in req.examples.iter().enumerate() {
                            // Saved-response cookies resolve against the example's own request
                            // URL (its `originalRequest`), else the parent request's URL.
                            if let Some(resp) = &ex.response {
                                let url = ex
                                    .request
                                    .as_ref()
                                    .map(|h| h.url.raw.as_str())
                                    .unwrap_or(parent_http.url.raw.as_str());
                                cookies.extend(rq_shape::cookies_from_response(resp, url));
                            }
                            examples.push(example_item(
                                ex,
                                &req.meta.id,
                                i,
                                parent_http,
                                &req.auth,
                            ));
                        }
                    }
                }
            }
            Item::Collection(child) => walk_collection(
                child,
                child_parent,
                collections,
                requests,
                examples,
                cookies,
                report,
            ),
        }
    }
}

/// A saved response → Requestly `BulkCreateExampleItem`: `{ tempId, parentId (the request),
/// name, data }` where `data` is the example's request entry plus the mapped `response`.
fn example_item(
    ex: &cq_model::Example,
    parent_request: &str,
    index: usize,
    parent_http: &HttpRequest,
    parent_auth: &Option<cq_model::Auth>,
) -> Value {
    let mut data = serde_json::Map::new();
    data.insert("type".into(), json!("http"));
    // The example's request is its saved `originalRequest`; when absent, it inherits the
    // parent request (url/method/auth) — matching the app's `buildExamples` fallback.
    let (http, auth) = match &ex.request {
        Some(h) => (h, &ex.auth),
        None => (parent_http, parent_auth),
    };
    data.insert("request".into(), rq_shape::http_request_object(http));
    if let Some(a) = rq_shape::requestly_auth_value(auth) {
        data.insert("auth".into(), a);
    }
    if let Some(resp) = ex.response.as_ref().and_then(map_response) {
        data.insert("response".into(), resp);
    }

    let mut item = serde_json::Map::new();
    item.insert("tempId".into(), json!(ex.meta.id));
    item.insert("parentId".into(), json!(parent_request));
    // `resp.name ?? "Example N"`, trailing whitespace/dots trimmed (a trailing '.' trips the
    // local-FS sanitizer), fall back again if that empties it, then cap the length — matching
    // the app's `buildExamples`.
    let fallback = || format!("Example {}", index + 1);
    let raw = if ex.meta.name.is_empty() {
        fallback()
    } else {
        ex.meta.name.clone()
    };
    let trimmed = raw.trim_end_matches(|c: char| c.is_whitespace() || c == '.');
    let name = if trimmed.is_empty() {
        fallback()
    } else {
        trimmed.to_string()
    };
    item.insert(
        "name".into(),
        json!(rq_shape::truncate_name(&name, rq_shape::MAX_NAME_LENGTH)),
    );
    item.insert("data".into(), Value::Object(data));
    Value::Object(item)
}

/// Map a Postman saved-response object → Requestly `HttpResponse`
/// (`{ body, headers, status, statusText, time }`), or `None` for a response-less example
/// (no body and no headers — ADR-073), matching the app's `mapPostmanResponse`.
fn map_response(resp: &Value) -> Option<Value> {
    let mut headers = serde_json::Map::new();
    if let Some(Value::Array(hs)) = resp.get("header") {
        for h in hs {
            if let Some(k) = h.get("key").and_then(Value::as_str) {
                let val = h.get("value").and_then(Value::as_str).unwrap_or("");
                headers.insert(k.to_string(), json!(val));
            }
        }
    }
    let body = resp.get("body").and_then(Value::as_str);
    if body.is_none() && headers.is_empty() {
        return None;
    }
    Some(json!({
        "body": body.unwrap_or(""),
        "headers": Value::Object(headers),
        "status": resp.get("code").and_then(Value::as_u64).unwrap_or(0),
        "statusText": resp.get("status").and_then(Value::as_str).unwrap_or(""),
        "time": 0,
    }))
}

fn request_item(req: &Request, parent_temp: Option<&str>, report: &mut Report) -> Option<Value> {
    let http: &HttpRequest = match &req.protocol {
        Protocol::Http(h) => h,
        Protocol::Graphql(_) => {
            report.dropped(
                Phase::Emit,
                req.meta.source.clone(),
                format!(
                    "GraphQL request '{}' not yet emittable to MappedItems",
                    req.meta.name
                ),
            );
            return None;
        }
    };

    let mut data = serde_json::Map::new();
    // A GraphQL body is its own Requestly entry type (`graphql`), not an HTTP body.
    if let Some(cq_model::Body::Graphql {
        query,
        variables,
        operation_name,
    }) = &http.body
    {
        data.insert("type".into(), json!("graphql"));
        data.insert(
            "request".into(),
            rq_shape::graphql_request_object(http, query, variables, operation_name.as_deref()),
        );
    } else {
        data.insert("type".into(), json!("http"));
        data.insert("request".into(), rq_shape::http_request_object(http));
    }
    // Requestly convention: a request always carries an auth (unspecified → `inherit`).
    // An auth kind with no Requestly equivalent falls back to `inherit` (matching the app's
    // `mapAuth` default), recorded as a coercion rather than dropped.
    match &req.auth {
        None => {
            data.insert("auth".into(), json!({ "type": "inherit" }));
        }
        Some(auth) => match rq_shape::auth_to_rq(auth) {
            AuthMap::Mapped(v) => {
                data.insert("auth".into(), v);
            }
            AuthMap::NoAuth => {
                data.insert("auth".into(), json!({ "type": "no_auth" }));
            }
            AuthMap::Unsupported(desc) => {
                report.warn(
                    Severity::Coerced,
                    Phase::Emit,
                    req.meta.source.clone(),
                    "advanced_auth",
                    format!(
                        "auth kind '{desc}' on '{}' has no Requestly equivalent; → inherit",
                        req.meta.name
                    ),
                );
                data.insert("auth".into(), json!({ "type": "inherit" }));
            }
        },
    }

    // Advisory warnings mirroring the app's mapper warnings — surfaced as `warningKind`-tagged
    // diagnostics the WASM shim aggregates into UnsupportedFeatureKind warnings. cross-q makes
    // the same lossy decision (coerce a non-standard method to GET, keep a form-data file
    // reference the user must re-attach); these flag it so the advisory matches. (binary_body is
    // flagged at parse time, where the file body is dropped.)
    let src_method = String::from(http.method.clone());
    if !matches!(
        src_method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    ) {
        report.warn(
            Severity::Coerced,
            Phase::Emit,
            req.meta.source.clone(),
            "unsupported_http_method",
            format!(
                "HTTP method '{src_method}' on '{}' coerced to GET",
                req.meta.name
            ),
        );
    }
    if let Some(cq_model::Body::FormData { fields }) = &http.body {
        if fields
            .iter()
            .any(|f| matches!(f, cq_model::FormField::File(_)))
        {
            report.warn(
                Severity::Coerced,
                Phase::Emit,
                req.meta.source.clone(),
                "file_reference",
                format!("form-data on '{}' references local files", req.meta.name),
            );
        }
    }
    if let Some(scripts) = rq_shape::scripts_object(&req.scripts) {
        data.insert("scripts".into(), scripts);
    }

    let mut item = serde_json::Map::new();
    item.insert("tempId".into(), json!(req.meta.id));
    item.insert("parentId".into(), parent_ref(parent_temp));
    item.insert(
        "name".into(),
        json!(rq_shape::truncate_name(
            &req.meta.name,
            rq_shape::MAX_NAME_LENGTH
        )),
    );
    if let Some(d) = &req.meta.description {
        item.insert("description".into(), json!(d));
    }
    if let Some(r) = &req.meta.rank {
        item.insert("rank".into(), json!(r));
    }
    item.insert("data".into(), Value::Object(data));

    report.push(Diagnostic::new(
        Severity::Ok,
        Phase::Emit,
        req.meta.source.clone(),
        format!("mapped request '{}'", req.meta.name),
    ));
    Some(Value::Object(item))
}

fn environment_item(env: &Environment) -> Value {
    json!({
        "tempId": env.meta.id,
        "name": env.meta.name,
        "isGlobal": env.is_global,
        "variables": rq_shape::variables_record(&env.variables),
    })
}

fn parent_ref(parent_temp: Option<&str>) -> Value {
    match parent_temp {
        Some(p) => json!(p),
        None => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_model::{
        Auth, KeyValue, Method, ModelHeader, Protocol, RecordMeta, Request, SourceFormat, Url,
    };
    use cq_report::Fidelity;

    fn http_req(id: &str, name: &str) -> Request {
        Request {
            meta: RecordMeta::new(id, name, SourceFormat::Postman),
            protocol: Protocol::Http(HttpRequest {
                method: Method::Get,
                url: Url::raw("https://api.example.com/x"),
                headers: vec![KeyValue::new("Accept", "application/json")],
                ..HttpRequest::default()
            }),
            auth: Some(Auth::Bearer {
                token: "{{T}}".into(),
                header_prefix: Some("Bearer".into()),
            }),
            scripts: Default::default(),
            examples: vec![],
            depends_on: vec![],
            behavior: Default::default(),
        }
    }

    #[test]
    fn named_collection_wires_parent_child_tempids() {
        let ws = Workspace {
            meta: RecordMeta::new("ws", "W", SourceFormat::Postman),
            cross_q: ModelHeader::for_source(SourceFormat::Postman),
            collections: vec![Collection {
                meta: RecordMeta::new("c1", "GitHub", SourceFormat::Postman),
                items: vec![Item::Request(Box::new(http_req("r1", "issues")))],
                ..Collection::default()
            }],
            environments: vec![],
            packages: vec![],
        };
        let mut report = Report::new(Fidelity::Lossless);
        let m = to_mapped_items(&ws, &mut report);

        assert_eq!(m["collections"][0]["tempId"], json!("c1"));
        assert_eq!(m["collections"][0]["parentId"], Value::Null);
        let req = &m["requests"][0];
        assert_eq!(req["tempId"], json!("r1"));
        assert_eq!(
            req["parentId"],
            json!("c1"),
            "request parent is the collection tempId"
        );
        assert_eq!(req["data"]["type"], json!("http"));
        assert_eq!(req["data"]["request"]["method"], json!("GET"));
        assert_eq!(req["data"]["request"]["headers"][0]["key"], json!("Accept"));
        assert_eq!(req["data"]["auth"]["type"], json!("bearer_token"));
    }

    #[test]
    fn empty_root_collection_is_transparent() {
        // curl-style: an unnamed root collection => request lands at root (parentId null),
        // and no collection record is produced.
        let ws = Workspace {
            meta: RecordMeta::new("ws", "", SourceFormat::Curl),
            cross_q: ModelHeader::for_source(SourceFormat::Curl),
            collections: vec![Collection {
                meta: RecordMeta::new("root", "", SourceFormat::Curl),
                items: vec![Item::Request(Box::new(http_req("r1", "issues")))],
                ..Collection::default()
            }],
            environments: vec![],
            packages: vec![],
        };
        let mut report = Report::new(Fidelity::Lossless);
        let m = to_mapped_items(&ws, &mut report);
        assert!(
            m.get("collections").is_none(),
            "no collection record for the synthetic root"
        );
        assert_eq!(m["requests"][0]["parentId"], Value::Null);
    }

    #[test]
    fn nested_folder_chains_parent_ids() {
        let inner = Collection {
            meta: RecordMeta::new("f1", "folder", SourceFormat::Postman),
            items: vec![Item::Request(Box::new(http_req("r1", "r")))],
            ..Collection::default()
        };
        let ws = Workspace {
            meta: RecordMeta::new("ws", "W", SourceFormat::Postman),
            cross_q: ModelHeader::for_source(SourceFormat::Postman),
            collections: vec![Collection {
                meta: RecordMeta::new("c1", "root-coll", SourceFormat::Postman),
                items: vec![Item::Collection(Box::new(inner))],
                ..Collection::default()
            }],
            environments: vec![],
            packages: vec![],
        };
        let mut report = Report::new(Fidelity::Lossless);
        let m = to_mapped_items(&ws, &mut report);
        // two collections; the request's parent is the inner folder.
        assert_eq!(m["collections"].as_array().unwrap().len(), 2);
        assert_eq!(m["requests"][0]["parentId"], json!("f1"));
    }

    #[test]
    fn is_deterministic() {
        let ws = Workspace {
            meta: RecordMeta::new("ws", "W", SourceFormat::Postman),
            cross_q: ModelHeader::for_source(SourceFormat::Postman),
            collections: vec![Collection {
                meta: RecordMeta::new("c1", "GitHub", SourceFormat::Postman),
                items: vec![Item::Request(Box::new(http_req("r1", "issues")))],
                ..Collection::default()
            }],
            environments: vec![],
            packages: vec![],
        };
        let mut r = Report::new(Fidelity::Lossless);
        let a = serde_json::to_string(&to_mapped_items(&ws, &mut r)).unwrap();
        let b = serde_json::to_string(&to_mapped_items(&ws, &mut r)).unwrap();
        assert_eq!(a, b);
    }
}
