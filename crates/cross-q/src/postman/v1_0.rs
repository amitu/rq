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
    // Record ids already emitted as tempIds (folders AND requests). Malformed exports can reuse one
    // `id` across siblings, or reference the same folder/request twice (via `folders_order` or a
    // repeated `order` entry); the app then builds each reference into its own record. A tempId must
    // stay globally unique or the bulkCreate import collides and the reconstructed tree is ambiguous
    // (children can't tell which parent instance they belong to). Duplicates are disambiguated
    // against this set; unique ids are untouched, so well-formed exports keep their source ids
    // verbatim (roundtrip-stable).
    let mut record_ids: HashSet<String> = HashSet::new();
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

    // Top-level requests first (`order`, excluding those inside folders) — matches the app, which
    // builds one record per `order` entry. A repeated entry therefore yields a repeated record
    // (disambiguated tempId); only folder-owned requests are skipped.
    for (i, id) in id_order(root.get("order")).into_iter().enumerate() {
        if reqs_in_folders.contains(&id) {
            continue;
        }
        if let Some(req) = by_id.get(&id) {
            used.insert(id.clone());
            items.push(Item::Request(Box::new(v1_request(
                req,
                &mut record_ids,
                report,
                &format!("order.{i}.{id}"),
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
    for (i, fid) in top_folder_ids.iter().enumerate() {
        if let Some(f) = folders_by_id.get(fid) {
            let mut visited = HashSet::new();
            visited.insert(fid.clone());
            items.push(Item::Collection(Box::new(build_v1_folder(
                f,
                // Index-qualified so two references to the same folder id get distinct locators
                // (the locator is what disambiguates a duplicate tempId below).
                &format!("folder.{i}.{fid}"),
                &by_id,
                &folders_by_id,
                &mut used,
                &mut record_ids,
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
                    &mut record_ids,
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
    record_ids: &mut HashSet<String>,
    report: &mut Report,
    visited: HashSet<String>,
) -> Collection {
    let fname = shared::obj_str(folder, "name").unwrap_or_else(|| "folder".to_string());
    let source_fid =
        shared::obj_str(folder, "id").unwrap_or_else(|| format!("pm-{}", shared::slugify(floc)));
    // Keep the source id when unique; on a collision fall back to the (unique) locator so the
    // record tempId stays globally unique. Well-formed exports never collide, so this is a no-op
    // for them.
    let fid = if record_ids.insert(source_fid.clone()) {
        source_fid
    } else {
        format!("pm-{}", shared::slugify(floc))
    };

    let mut children = Vec::new();
    // Requests first (one record per `order` entry, disambiguated tempId on a repeat).
    for (i, id) in id_order(folder.get("order")).into_iter().enumerate() {
        if let Some(req) = by_id.get(&id) {
            used.insert(id.clone());
            children.push(Item::Request(Box::new(v1_request(
                req,
                record_ids,
                report,
                &format!("{floc}.{i}.{id}"),
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
                record_ids,
                report,
                v,
            ))));
        }
    }

    Collection {
        // v1 folder description is a plain string kept verbatim (incl. "" — the app keeps an
        // empty string too, unlike the v2 object-form which drops empty).
        meta: shared::record_meta(fid, fname, floc, shared::obj_str(folder, "description")),
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

fn v1_request(
    req: &Value,
    record_ids: &mut HashSet<String>,
    report: &mut Report,
    locator: &str,
) -> Request {
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
    let body = v1_body(req, &headers);

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

    let source_id =
        shared::obj_str(req, "id").unwrap_or_else(|| format!("pm-{}", shared::slugify(&name)));
    // Same global-uniqueness rule as folders: keep the source id when unique, else fall back to the
    // (unique) locator so a request built from a repeated `order` entry — or one inside a folder
    // reconstructed twice — gets its own tempId instead of colliding.
    let id = if record_ids.insert(source_id.clone()) {
        source_id
    } else {
        format!("pm-{}", shared::slugify(locator))
    };
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

/// v1 body: `dataMode` selects `rawModeData` (raw) or `data[]` (urlencoded/params). Mirrors the
/// app's `v1Body` normalisation: a raw body is produced when `dataMode == "raw"` OR when
/// `rawModeData` is a string and `dataMode` is absent — including an empty raw string (still a
/// body). v1 carries no editor `language`, so a raw body's media type is inferred from the
/// Content-Type header (RQ-4140), exactly as the app's `mapBody` does post-normalisation.
fn v1_body(req: &Value, headers: &[KeyValue]) -> Option<Body> {
    let mode = req.get("dataMode").and_then(Value::as_str);
    let raw_mode_data = req.get("rawModeData").and_then(Value::as_str);

    if mode == Some("raw") || (raw_mode_data.is_some() && mode.is_none()) {
        let text = raw_mode_data
            .or_else(|| req.get("data").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        return Some(Body::Raw {
            text,
            media_type: shared::raw_media_type_from_headers(headers),
        });
    }

    match mode {
        Some("urlencoded") | Some("params") => {
            // The app requires `data` to be an array; a null/absent `data` means no body
            // (an empty array is still a body — form/multipart with no rows).
            let Some(Value::Array(_)) = req.get("data") else {
                return None;
            };
            let fields = v1_data_kv(req.get("data"));
            if mode == Some("params") {
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
