//! Idealised Model → `rq` documents.
//!
//! `rq curl --save-as` and `rq import` both land here: cross-q parses whatever you have
//! (a curl command, a Postman collection, a Bruno tree) into the Idealised Model, and this
//! writes that model out as one Markdown file per request.
//!
//! It keeps the converter's promise: anything that can't be carried into the `.md` form is
//! a note on the way out, never a silent drop.

use std::path::{Path, PathBuf};

use anyhow::Result;
use cq_model::{
    Auth, Body, Collection, FileRef, FormField, Item, KeyValue, KvKind, Protocol, Request,
    ScriptDialect, VarType, Variable, Workspace,
};
use serde_norway::{Mapping, Value};

use crate::doc::{AuthSpec, Document, VarSpec};
use crate::project::{self, save_document, COLLECTION_FILE, REQUEST_FILE};

#[derive(Debug, Default)]
pub struct Emitted {
    /// Slash-separated paths below `apis/`, in the order written.
    pub requests: Vec<String>,
    pub environments: Vec<String>,
    pub notes: Vec<String>,
}

/// A request built but not yet written: dependencies are resolved once every request has
/// its final path, because `parents:` refers to requests by path and the model refers to
/// them by id.
struct Pending {
    id: String,
    rel: String,
    path: PathBuf,
    doc: Document,
    deps: Vec<cq_model::Dependency>,
}

/// Write a whole workspace into a project rooted at `root`.
pub fn emit(ws: &Workspace, root: &Path) -> Result<Emitted> {
    let mut out = Emitted::default();
    let mut pending: Vec<Pending> = Vec::new();
    let apis = root.join(project::APIS_DIR);

    for collection in &ws.collections {
        // A workspace's root collection is usually unnamed — its children land directly
        // under `apis/` rather than in an "Untitled" folder.
        if collection.meta.name.trim().is_empty() {
            emit_items(&collection.items, &apis, "", &mut out, &mut pending)?;
            write_collection_doc(collection, &apis, &mut out)?;
        } else {
            emit_collection(collection, &apis, "", &mut out, &mut pending)?;
        }
    }

    resolve_dependencies(&mut pending, &mut out);
    for item in &pending {
        save_document(&item.path, &item.doc)?;
        out.requests.push(item.rel.clone());
    }

    for env in &ws.environments {
        let name = if env.is_global {
            project::GLOBAL_ENV.to_string()
        } else {
            file_name(&env.meta.name, "environment")
        };
        let path = root.join(project::ENVS_DIR).join(format!("{name}.md"));
        let mut doc = Document::default();
        doc.front.vars = env.variables.iter().filter_map(var_spec).collect();
        if let Some(d) = env
            .meta
            .description
            .as_ref()
            .filter(|d| !d.trim().is_empty())
        {
            doc.set_section("description", d.clone());
        }
        save_document(&path, &doc)?;
        out.environments.push(name);
    }

    if !ws.packages.is_empty() {
        out.notes.push(format!(
            "{} reusable script package(s) were not written: this build has no script runtime",
            ws.packages.len()
        ));
    }
    Ok(out)
}

/// Turn the model's `depends_on` (ids) into the document's `parents:` (paths), and its
/// `binds` into a `capture:` on the request that produces the value.
fn resolve_dependencies(pending: &mut [Pending], out: &mut Emitted) {
    let by_id: Vec<(String, String)> = pending
        .iter()
        .map(|p| (p.id.clone(), p.rel.clone()))
        .collect();
    let lookup = |id: &str| -> Option<String> {
        by_id
            .iter()
            .find(|(pid, _)| pid == id)
            .map(|(_, rel)| rel.clone())
    };

    let mut captures: Vec<(String, String, String)> = Vec::new(); // (parent id, var, path)
    for item in pending.iter_mut() {
        for dep in &item.deps {
            match lookup(&dep.target) {
                Some(rel) => item.doc.front.parents.push(rel),
                None => out.notes.push(format!(
                    "{}: depends on a request that isn't in this import ({})",
                    item.rel, dep.target
                )),
            }
            for bind in &dep.binds {
                captures.push((dep.target.clone(), bind.to.clone(), bind.from.clone()));
            }
        }
    }
    for (parent_id, var, from) in captures {
        if let Some(parent) = pending.iter_mut().find(|p| p.id == parent_id) {
            parent.doc.front.capture.push((var, from));
        }
    }
}

fn emit_collection(
    collection: &Collection,
    parent_dir: &Path,
    prefix: &str,
    out: &mut Emitted,
    pending: &mut Vec<Pending>,
) -> Result<()> {
    let name = file_name(&collection.meta.name, "collection");
    let dir = unique_dir(parent_dir, &name);
    let rel = join_rel(prefix, &dir_name(&dir));
    std::fs::create_dir_all(&dir)?;
    write_collection_doc(collection, &dir, out)?;
    emit_items(&collection.items, &dir, &rel, out, pending)
}

fn emit_items(
    items: &[Item],
    dir: &Path,
    prefix: &str,
    out: &mut Emitted,
    pending: &mut Vec<Pending>,
) -> Result<()> {
    for item in items {
        match item {
            Item::Collection(c) => emit_collection(c, dir, prefix, out, pending)?,
            Item::Request(r) => emit_request(r, dir, prefix, out, pending)?,
        }
    }
    Ok(())
}

fn write_collection_doc(collection: &Collection, dir: &Path, out: &mut Emitted) -> Result<()> {
    let mut doc = Document::default();
    doc.front.headers = kv_pairs(&collection.headers, out);
    doc.front.auth = collection.auth.as_ref().map(|a| auth_spec(a, out));
    doc.front.vars = collection.variables.iter().filter_map(var_spec).collect();
    if let Some(d) = collection
        .meta
        .description
        .as_ref()
        .filter(|d| !d.trim().is_empty())
    {
        doc.set_section("description", d.clone());
    }
    add_scripts(&mut doc, &collection.scripts, &collection.meta.name, out);

    // Nothing to say → no file. An empty `__collection.md` is noise in a diff.
    if doc.front.headers.is_empty()
        && doc.front.auth.is_none()
        && doc.front.vars.is_empty()
        && doc.sections.is_empty()
    {
        return Ok(());
    }
    save_document(&dir.join(COLLECTION_FILE), &doc)
}

fn emit_request(
    req: &Request,
    parent_dir: &Path,
    prefix: &str,
    out: &mut Emitted,
    pending: &mut Vec<Pending>,
) -> Result<()> {
    let name = file_name(&req.meta.name, "request");
    let dir = unique_dir(parent_dir, &name);
    std::fs::create_dir_all(&dir)?;
    let rel = join_rel(prefix, &dir_name(&dir));

    let mut doc = Document::default();
    match &req.protocol {
        Protocol::Http(http) => {
            doc.front.method = Some(String::from(http.method.clone()));
            doc.front.url = Some(http.url.raw.clone());
            doc.front.headers = kv_pairs(&http.headers, out);
            doc.front.query = kv_pairs(&http.query, out);
            doc.front.path_vars = http
                .path_variables
                .iter()
                .map(|p| (p.key.clone(), p.value.clone()))
                .collect();
            if !http.settings.follow_redirects {
                doc.front.follow_redirects = Some(false);
            }
            if !http.settings.verify_tls {
                doc.front.verify_tls = Some(false);
            }
            doc.front.timeout = http.settings.timeout_ms;
            if let Some(body) = &http.body {
                apply_body(&mut doc, body, &rel, out);
            }
        }
        Protocol::Graphql(gql) => {
            doc.front.method = Some(String::from(gql.method.clone()));
            doc.front.url = Some(gql.url.raw.clone());
            doc.front.headers = kv_pairs(&gql.headers, out);
            doc.front.body_type = Some("graphql".into());
            let payload = serde_json::json!({
                "query": gql.query,
                "variables": gql.variables,
                "operationName": gql.operation_name,
            });
            doc.set_section(
                "body",
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            );
            out.notes.push(format!(
                "{rel}: a GraphQL request was written as its JSON POST body"
            ));
        }
    }

    doc.front.auth = req.auth.as_ref().map(|a| auth_spec(a, out));
    if let Some(d) = req
        .meta
        .description
        .as_ref()
        .filter(|d| !d.trim().is_empty())
    {
        doc.set_section("description", d.clone());
    }
    add_scripts(&mut doc, &req.scripts, &rel, out);

    if !req.examples.is_empty() {
        out.notes.push(format!(
            "{rel}: {} saved response example(s) were not written",
            req.examples.len()
        ));
    }
    if req.meta.disabled {
        out.notes.push(format!(
            "{rel}: was disabled in the source; written as active"
        ));
    }

    pending.push(Pending {
        id: req.meta.id.clone(),
        rel,
        path: dir.join(REQUEST_FILE),
        doc,
        deps: req.depends_on.clone(),
    });
    Ok(())
}

fn apply_body(doc: &mut Document, body: &Body, rel: &str, out: &mut Emitted) {
    match body {
        Body::None => {}
        Body::Raw { text, media_type } => {
            if !media_type.is_empty() {
                doc.front.body_type = Some(media_type.clone());
            }
            doc.set_section("body", text.clone());
        }
        Body::UrlEncoded { fields } => {
            doc.front.form = Some(kv_pairs(fields, out));
        }
        Body::FormData { fields } => {
            let mut pairs = Vec::new();
            for field in fields {
                match field {
                    FormField::Text(kv) => {
                        if kv.enabled {
                            pairs.push((kv.key.clone(), kv.value.clone()));
                        }
                    }
                    FormField::File(file) => match file {
                        FileRef::Reference { name, path, .. } => {
                            pairs.push((name.clone(), format!("@{path}")));
                        }
                        FileRef::Content { name, .. } => {
                            out.notes.push(format!(
                                "{rel}: inline file part `{name}` has no path and was skipped"
                            ));
                        }
                    },
                }
            }
            doc.front.form_data = Some(pairs);
        }
        Body::Binary { file } => match file {
            FileRef::Reference { path, .. } => doc.front.file = Some(path.clone()),
            FileRef::Content { name, .. } => out.notes.push(format!(
                "{rel}: inline binary body `{name}` has no path and was skipped"
            )),
        },
        Body::Graphql {
            query,
            variables,
            operation_name,
        } => {
            doc.front.body_type = Some("graphql".into());
            let payload = serde_json::json!({
                "query": query,
                "variables": variables,
                "operationName": operation_name,
            });
            doc.set_section(
                "body",
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            );
        }
    }
}

/// Scripts are carried **verbatim** with their dialect noted. A `pm.*` script is not
/// rewritten to `rq.*` on the way in: a textual rename imports clean and throws at
/// runtime, which is the one failure mode this project refuses to ship.
fn add_scripts(doc: &mut Document, scripts: &cq_model::Scripts, rel: &str, out: &mut Emitted) {
    for (section, script) in [
        ("pre", scripts.pre_request.as_ref()),
        ("post", scripts.post_response.as_ref()),
    ] {
        let Some(script) = script.filter(|s| !s.source.trim().is_empty()) else {
            continue;
        };
        doc.set_section(section, script.source.clone());
        if !matches!(script.dialect, ScriptDialect::Rq) {
            out.notes.push(format!(
                "{rel}: `-- {section} --` is written in the {:?} dialect and was kept verbatim",
                script.dialect
            ));
        }
    }
}

fn kv_pairs(pairs: &[KeyValue], out: &mut Emitted) -> Vec<(String, String)> {
    let mut kept = Vec::new();
    for kv in pairs {
        if !kv.enabled {
            out.notes.push(format!(
                "`{}` was disabled in the source and was dropped",
                kv.key
            ));
            continue;
        }
        if kv.kind == KvKind::File {
            out.notes.push(format!(
                "`{}` is a file field outside a form body and was written as text",
                kv.key
            ));
        }
        kept.push((kv.key.clone(), kv.value.clone()));
    }
    kept
}

fn var_spec(v: &Variable) -> Option<(String, VarSpec)> {
    if v.key.is_empty() {
        return None;
    }
    Some((
        v.key.clone(),
        VarSpec {
            default: Some(v.value.clone()),
            secret: v.data_type == VarType::Secret,
            description: None,
            ..VarSpec::default()
        },
    ))
}

/// Auth kinds `rq` can send become first-class; the rest are preserved verbatim under
/// their own type so the credential survives the trip.
fn auth_spec(auth: &Auth, out: &mut Emitted) -> AuthSpec {
    match auth {
        Auth::None => AuthSpec::None,
        Auth::Inherit => AuthSpec::Inherit,
        Auth::Basic { username, password } => AuthSpec::Basic {
            username: username.clone(),
            password: password.clone(),
        },
        Auth::Bearer {
            token,
            header_prefix,
        } => AuthSpec::Bearer {
            token: token.clone(),
            prefix: header_prefix.clone(),
        },
        Auth::ApiKey {
            key,
            value,
            placement,
        } => AuthSpec::ApiKey {
            key: key.clone(),
            value: value.clone(),
            in_query: matches!(placement, cq_model::ApiKeyPlacement::Query),
        },
        other => {
            let json = serde_json::to_value(other).unwrap_or(serde_json::Value::Null);
            let kind = json
                .get("kind")
                .and_then(|k| k.as_str())
                .unwrap_or("unknown")
                .to_string();
            let raw = match json_to_yaml(&json) {
                Value::Mapping(mut m) => {
                    // The document's key for the discriminant is `type`, not `kind`.
                    m.remove(Value::from("kind"));
                    m.insert("type".into(), kind.as_str().into());
                    m
                }
                _ => Mapping::new(),
            };
            out.notes.push(format!(
                "auth `{kind}` was preserved but this build cannot send it"
            ));
            AuthSpec::Other { kind, raw }
        }
    }
}

fn json_to_yaml(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => n
            .as_u64()
            .map(Value::from)
            .or_else(|| n.as_i64().map(Value::from))
            .or_else(|| n.as_f64().map(Value::from))
            .unwrap_or(Value::Null),
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(a) => Value::Sequence(a.iter().map(json_to_yaml).collect()),
        serde_json::Value::Object(o) => {
            let mut m = Mapping::new();
            for (k, val) in o {
                m.insert(k.as_str().into(), json_to_yaml(val));
            }
            Value::Mapping(m)
        }
    }
}

/// A filesystem-safe name, falling back to a stable placeholder rather than failing.
fn file_name(raw: &str, fallback: &str) -> String {
    project::slug_path(raw)
        .map(|s| s.replace('/', "-"))
        .unwrap_or_else(|_| fallback.to_string())
}

/// Two requests can share a name in a source collection; two directories cannot.
fn unique_dir(parent: &Path, name: &str) -> PathBuf {
    let mut candidate = parent.join(name);
    let mut n = 2;
    while candidate.exists() {
        candidate = parent.join(format!("{name}-{n}"));
        n += 1;
    }
    candidate
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn join_rel(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

/// Guess the source format for `rq import`, the way a person would: by looking.
pub fn detect_format(path: &Path, content: &str) -> Option<&'static str> {
    if path.extension().is_some_and(|e| e == "bru") {
        return Some("bruno");
    }
    let head = content.trim_start();
    if head.starts_with("curl ") {
        return Some("curl");
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(head) {
        if json.get("info").is_some() || json.get("_postman_id").is_some() {
            return Some("postman");
        }
    }
    if head.contains("\nget {") || head.starts_with("meta {") || head.contains("\npost {") {
        return Some("bruno");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Project;
    use cq_report::{Fidelity, Report};

    fn import(input: &str, source: &str) -> (tempfile::TempDir, Emitted) {
        let dir = tempfile::tempdir().unwrap();
        project::init(dir.path()).unwrap();
        let mut report = Report::new(Fidelity::Lossless);
        let ws = cross_q::build_workspace(source, input, &mut report).unwrap();
        let emitted = emit(&ws, dir.path()).unwrap();
        (dir, emitted)
    }

    #[test]
    fn a_curl_becomes_a_readable_request() {
        let (dir, out) = import(
            "curl -X POST https://api.test/login -H 'Content-Type: application/json' \
             -d '{\"user\":\"amitu\"}'",
            "curl",
        );
        assert_eq!(out.requests.len(), 1);
        let path = dir
            .path()
            .join(project::APIS_DIR)
            .join(&out.requests[0])
            .join(REQUEST_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("method: POST"), "{text}");
        assert!(text.contains("url: https://api.test/login"), "{text}");
        assert!(text.contains("Content-Type: application/json"), "{text}");
        assert!(text.contains("-- body --"), "{text}");
        assert!(text.contains("{\"user\":\"amitu\"}"), "{text}");
        // …and it parses back as a project.
        let project = Project::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(project.requests().count(), 1);
    }

    #[test]
    fn a_postman_collection_becomes_a_tree_with_scripts_kept_verbatim() {
        let collection = serde_json::json!({
            "info": { "name": "Acme", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
            "item": [
                {
                    "name": "Auth",
                    "item": [{
                        "name": "login",
                        "event": [{
                            "listen": "test",
                            "script": { "exec": ["pm.environment.set('token', pm.response.json().token);"] }
                        }],
                        "request": {
                            "method": "POST",
                            "url": { "raw": "https://api.test/login" },
                            "auth": { "type": "basic", "basic": [
                                { "key": "username", "value": "u" }, { "key": "password", "value": "p" }
                            ]}
                        }
                    }]
                }
            ]
        })
        .to_string();
        let (dir, out) = import(&collection, "postman");
        assert_eq!(out.requests, vec!["Acme/Auth/login"]);
        let text = std::fs::read_to_string(
            dir.path()
                .join(project::APIS_DIR)
                .join("Acme/Auth/login")
                .join(REQUEST_FILE),
        )
        .unwrap();
        assert!(text.contains("type: basic"), "{text}");
        assert!(text.contains("-- post --"), "{text}");
        // The pm.* source is carried as written, with a note — never rewritten to rq.*.
        assert!(text.contains("pm.environment.set"), "{text}");
        assert!(
            out.notes.iter().any(|n| n.contains("dialect")),
            "{:?}",
            out.notes
        );
    }

    #[test]
    fn detects_the_obvious_formats() {
        assert_eq!(
            detect_format(Path::new("x.json"), "{\"info\": {}, \"item\": []}"),
            Some("postman")
        );
        assert_eq!(
            detect_format(Path::new("x.txt"), "curl https://x.test"),
            Some("curl")
        );
        assert_eq!(detect_format(Path::new("x.bru"), "get {\n}"), Some("bruno"));
        assert_eq!(detect_format(Path::new("x.txt"), "nonsense"), None);
    }
}
