//! Postman Collection **v1.0.0** parser — the genuinely-different topology. v1 has no
//! `info` wrapper and no `item[]` tree; instead a flat `requests[]` array, `folders[]`
//! whose `order[]` lists member request ids, and a top-level `order[]` for requests
//! directly under the collection. Headers are a newline-delimited **string**, bodies are
//! `dataMode` + `rawModeData`/`data[]`, and scripts are the `preRequestScript`/`tests`
//! **strings** (not a v2 `event[]`).
//!
//! It reuses the leaf primitives in [`super::shared`] (coercion, header-string parsing,
//! kv, script wrapping, the final IR request assembly) — only the v1-specific shape lives
//! here, so nothing from the v2 parser is copy-pasted.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use cq_model::{
    Body, Collection, FormField, HttpRequest, Item, KeyValue, Method, ModelHeader, RecordMeta,
    Request, Scripts, SourceFormat, Url, Workspace,
};
use cq_report::Report;

use super::shared;

pub(super) fn parse(root: &Value, report: &mut Report) -> Workspace {
    let name = shared::obj_str(root, "name").unwrap_or_else(|| "Imported Collection".to_string());

    // id → request Value
    let mut by_id: BTreeMap<String, &Value> = BTreeMap::new();
    if let Some(Value::Array(reqs)) = root.get("requests") {
        for r in reqs {
            if let Some(id) = shared::obj_str(r, "id") {
                by_id.insert(id, r);
            }
        }
    }

    let mut used: HashSet<String> = HashSet::new();
    let mut items: Vec<Item> = Vec::new();

    // Folders → sub-collections. Order folders by `folders_order` when present.
    let folders: Vec<&Value> = match root.get("folders") {
        Some(Value::Array(fs)) => order_by(fs, root.get("folders_order")),
        _ => Vec::new(),
    };
    for (fi, folder) in folders.iter().enumerate() {
        let fname = shared::obj_str(folder, "name").unwrap_or_else(|| "folder".to_string());
        let floc = format!("folders[{fi}]");
        // Unique fallback id from position (duplicate folder names must not collide).
        let fid = shared::obj_str(folder, "id")
            .unwrap_or_else(|| format!("pm-{}", shared::slugify(&floc)));
        let mut fitems = Vec::new();
        for id in id_order(folder.get("order")) {
            if let Some(req) = by_id.get(&id) {
                used.insert(id.clone());
                fitems.push(Item::Request(Box::new(v1_request(
                    req,
                    report,
                    &format!("{floc}.{id}"),
                ))));
            }
        }
        items.push(Item::Collection(Box::new(Collection {
            meta: shared::record_meta(fid, fname, &floc, shared::obj_str(folder, "description")),
            auth: None,
            headers: Vec::new(),
            scripts: Scripts::default(),
            variables: Vec::new(),
            items: fitems,
        })));
    }

    // Top-level `order[]` → requests directly under the collection.
    for id in id_order(root.get("order")) {
        if used.contains(&id) {
            continue;
        }
        if let Some(req) = by_id.get(&id) {
            used.insert(id.clone());
            items.push(Item::Request(Box::new(v1_request(
                req,
                report,
                &format!("order.{id}"),
            ))));
        }
    }

    // Orphans — any request referenced by no order[]. Keep them (append at root) rather
    // than silently drop.
    if let Some(Value::Array(reqs)) = root.get("requests") {
        for (i, r) in reqs.iter().enumerate() {
            let id = shared::obj_str(r, "id").unwrap_or_default();
            if id.is_empty() || !used.contains(&id) {
                items.push(Item::Request(Box::new(v1_request(
                    r,
                    report,
                    &format!("requests[{i}]"),
                ))));
            }
        }
    }

    let collection = Collection {
        meta: shared::record_meta(
            shared::obj_str(root, "id").unwrap_or_else(|| format!("pm-{}", shared::slugify(&name))),
            name.clone(),
            "",
            shared::obj_str(root, "description"),
        ),
        auth: None,
        headers: Vec::new(),
        scripts: Scripts::default(),
        variables: Vec::new(),
        items,
    };

    Workspace {
        meta: RecordMeta::new("pm-workspace", name, SourceFormat::Postman),
        cross_q: ModelHeader::for_source(SourceFormat::Postman),
        collections: vec![collection],
        environments: Vec::new(),
        packages: Vec::new(),
    }
}

/// Order elements of `arr` (folder objects) by an `ids` list of their `id`s, appending any
/// not listed. Falls back to array order when `ids` is absent.
fn order_by<'a>(arr: &'a [Value], ids: Option<&Value>) -> Vec<&'a Value> {
    let order = id_order(ids);
    if order.is_empty() {
        return arr.iter().collect();
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for id in &order {
        if let Some(el) = arr
            .iter()
            .find(|e| shared::obj_str(e, "id").as_deref() == Some(id))
        {
            out.push(el);
            seen.insert(id.clone());
        }
    }
    for el in arr {
        if let Some(id) = shared::obj_str(el, "id") {
            if !seen.contains(&id) {
                out.push(el);
            }
        } else {
            out.push(el);
        }
    }
    out
}

/// Read an `order`-style array of id strings.
fn id_order(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

fn v1_request(req: &Value, report: &mut Report, locator: &str) -> Request {
    let name = shared::obj_str(req, "name").unwrap_or_else(|| "request".to_string());
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .map(|m| Method::from(m.to_string()))
        .unwrap_or(Method::Get);
    let raw_url = shared::obj_str(req, "url").unwrap_or_default();
    let query = shared::query_from_raw(&raw_url);
    let url = Url::raw(raw_url);
    let headers = req
        .get("headers")
        .and_then(Value::as_str)
        .map(shared::parse_header_string)
        .unwrap_or_default();
    let body = v1_body(req, report, locator);

    // v1 scripts are strings on the request.
    let mut scripts = Scripts::default();
    if let Some(s) = req.get("preRequestScript").and_then(Value::as_str) {
        if !s.trim().is_empty() {
            scripts.pre_request = Some(shared::pm_script(s.to_string()));
        }
    }
    if let Some(s) = req.get("tests").and_then(Value::as_str) {
        if !s.trim().is_empty() {
            scripts.post_response = Some(shared::pm_script(s.to_string()));
        }
    }

    let id = shared::obj_str(req, "id").unwrap_or_else(|| format!("pm-{}", shared::slugify(&name)));
    let http = HttpRequest {
        method,
        url,
        headers,
        query,
        path_variables: Vec::new(),
        body,
        settings: cq_model::RequestSettings::default(),
    };
    shared::http_request(
        shared::NodeMeta {
            id,
            name,
            description: shared::obj_str(req, "description"),
        },
        locator,
        http,
        None,
        scripts,
    )
}

/// v1 body: `dataMode` selects `rawModeData` (raw) or `data[]` (urlencoded/params).
fn v1_body(req: &Value, _report: &mut Report, _locator: &str) -> Option<Body> {
    let mode = req.get("dataMode").and_then(Value::as_str)?;
    match mode {
        "raw" => {
            let text = req
                .get("rawModeData")
                .and_then(Value::as_str)
                .or_else(|| req.get("data").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string();
            if text.is_empty() {
                None
            } else {
                Some(Body::Raw {
                    text,
                    media_type: "text/plain".to_string(),
                })
            }
        }
        "urlencoded" | "params" => {
            let fields = v1_data_kv(req.get("data"));
            if mode == "params" {
                Some(Body::FormData {
                    fields: fields.into_iter().map(FormField::Text).collect(),
                })
            } else {
                Some(Body::UrlEncoded { fields })
            }
        }
        _ => None,
    }
}

/// v1 `data[]` is an array of `{ key, value, enabled }`.
fn v1_data_kv(v: Option<&Value>) -> Vec<KeyValue> {
    let mut out = Vec::new();
    if let Some(Value::Array(items)) = v {
        for it in items {
            let key = shared::coerce_value(it.get("key"));
            let mut kv = KeyValue::new(key, shared::coerce_value(it.get("value")));
            if !it.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
                kv.enabled = false;
            }
            out.push(kv);
        }
    }
    out
}
