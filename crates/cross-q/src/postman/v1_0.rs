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

    // folder id → folder Value, for tree reconstruction.
    let mut folders_by_id: BTreeMap<String, &Value> = BTreeMap::new();
    if let Some(Value::Array(fs)) = root.get("folders") {
        for f in fs {
            if let Some(id) = shared::obj_str(f, "id") {
                folders_by_id.insert(id, f);
            }
        }
    }
    // Requests that live inside some folder (so they're not also placed at the root).
    let mut reqs_in_folders: HashSet<String> = HashSet::new();
    if let Some(Value::Array(fs)) = root.get("folders") {
        for f in fs {
            for id in id_order(f.get("order")) {
                reqs_in_folders.insert(id);
            }
        }
    }

    // Top-level requests first (`order`, excluding those inside folders) — matches the app.
    for id in id_order(root.get("order")) {
        if reqs_in_folders.contains(&id) || used.contains(&id) {
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
    // Then top-level folders (`folders_order`, else every folder) — reconstructed recursively.
    let top_folder_ids: Vec<String> = match root.get("folders_order") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => root
            .get("folders")
            .and_then(Value::as_array)
            .map(|fs| fs.iter().filter_map(|f| shared::obj_str(f, "id")).collect())
            .unwrap_or_default(),
    };
    for fid in &top_folder_ids {
        if let Some(f) = folders_by_id.get(fid) {
            let mut visited = HashSet::new();
            visited.insert(fid.clone());
            items.push(Item::Collection(Box::new(build_v1_folder(
                f,
                &format!("folder.{fid}"),
                &by_id,
                &folders_by_id,
                &mut used,
                report,
                visited,
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
        auth: super::v2_1::parse_auth(root.get("auth"), report, "auth"),
        headers: Vec::new(),
        scripts: shared::parse_scripts(shared::field(root, "event", "events")),
        variables: shared::parse_variables(root.get("variables")),
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

/// Reconstruct a v1 folder (and its subtree) → IR [`Collection`]. Children are its requests
/// (`order`) followed by its sub-folders (`folders_order`), recursively — matching the app's
/// `buildFolderNode`. `visited` guards against a cyclic `folders_order` graph.
#[allow(clippy::too_many_arguments)]
fn build_v1_folder<'a>(
    folder: &'a Value,
    floc: &str,
    by_id: &BTreeMap<String, &'a Value>,
    folders_by_id: &BTreeMap<String, &'a Value>,
    used: &mut HashSet<String>,
    report: &mut Report,
    visited: HashSet<String>,
) -> Collection {
    let fname = shared::obj_str(folder, "name").unwrap_or_else(|| "folder".to_string());
    let fid =
        shared::obj_str(folder, "id").unwrap_or_else(|| format!("pm-{}", shared::slugify(floc)));

    let mut children = Vec::new();
    // Requests first.
    for id in id_order(folder.get("order")) {
        if let Some(req) = by_id.get(&id) {
            used.insert(id.clone());
            children.push(Item::Request(Box::new(v1_request(
                req,
                report,
                &format!("{floc}.{id}"),
            ))));
        }
    }
    // Then sub-folders (cycle-guarded).
    for sub in id_order(folder.get("folders_order")) {
        if visited.contains(&sub) {
            continue;
        }
        if let Some(sf) = folders_by_id.get(&sub) {
            let mut v = visited.clone();
            v.insert(sub.clone());
            children.push(Item::Collection(Box::new(build_v1_folder(
                sf,
                &format!("{floc}.{sub}"),
                by_id,
                folders_by_id,
                used,
                report,
                v,
            ))));
        }
    }

    Collection {
        meta: shared::record_meta(fid, fname, floc, shared::description(folder)),
        // v1 folders carry a v2.1-style `auth` object when set.
        auth: super::v2_1::parse_auth(folder.get("auth"), report, &format!("{floc}.auth")),
        headers: Vec::new(),
        scripts: shared::parse_scripts(shared::field(folder, "event", "events")),
        variables: shared::parse_variables(folder.get("variables")),
        items: children,
    }
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
    // v1 requests carry a v2.1-style `auth` object (type + `<type>[]` params) when set; an
    // absent one means inherit (the app's `mapAuth(undefined)`). currentHelper/helperAttributes
    // are the legacy UI mirror and are NOT the auth source — matching the app.
    let auth = super::v2_1::parse_auth(req.get("auth"), report, &format!("{locator}.auth"));
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
        auth,
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
            // The app requires `data` to be an array; a null/absent `data` means no body
            // (an empty array is still a body — form/multipart with no rows).
            let Some(Value::Array(_)) = req.get("data") else {
                return None;
            };
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
