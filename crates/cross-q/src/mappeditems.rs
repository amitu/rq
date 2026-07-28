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
    let mut environments = Vec::new();

    for coll in &ws.collections {
        walk_collection(coll, None, &mut collections, &mut requests, report);
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
    if !environments.is_empty() {
        obj.insert("environments".into(), Value::Array(environments));
    }
    Value::Object(obj)
}

/// Walk a collection, appending its records to the per-kind bundles. A collection with an
/// empty name is the synthetic root — it produces no record, and its items inherit the
/// incoming parent (root → `parentId: null`).
fn walk_collection(
    coll: &Collection,
    parent_temp: Option<&str>,
    collections: &mut Vec<Value>,
    requests: &mut Vec<Value>,
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
        if let Some(auth) = &coll.auth {
            if let AuthMap::Mapped(v) = rq_shape::auth_to_rq(auth) {
                data.insert("auth".into(), v);
            }
        }
        if let Some(scripts) = rq_shape::scripts_object(&coll.scripts) {
            data.insert("scripts".into(), scripts);
        }

        let mut item = serde_json::Map::new();
        item.insert("tempId".into(), json!(temp));
        item.insert("parentId".into(), parent_ref(parent_temp));
        item.insert("name".into(), json!(coll.meta.name));
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
                }
            }
            Item::Collection(child) => {
                walk_collection(child, child_parent, collections, requests, report)
            }
        }
    }
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
    data.insert("type".into(), json!("http"));
    data.insert("request".into(), rq_shape::http_request_object(http));
    if let Some(auth) = &req.auth {
        match rq_shape::auth_to_rq(auth) {
            AuthMap::Mapped(v) => {
                data.insert("auth".into(), v);
            }
            AuthMap::NoAuth => {}
            AuthMap::Unsupported(desc) => report.dropped(
                Phase::Emit,
                req.meta.source.clone(),
                format!("auth kind dropped from '{}': {desc}", req.meta.name),
            ),
        }
    }
    if let Some(scripts) = rq_shape::scripts_object(&req.scripts) {
        data.insert("scripts".into(), scripts);
    }

    let mut item = serde_json::Map::new();
    item.insert("tempId".into(), json!(req.meta.id));
    item.insert("parentId".into(), parent_ref(parent_temp));
    item.insert("name".into(), json!(req.meta.name));
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
