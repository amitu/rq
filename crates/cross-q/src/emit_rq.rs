//! Emit a [`Workspace`] as a Requestly `LOCAL_FS` project tree.
//!
//! Targets the layout documented in `docs/FORMAT.md` §9 (schema `1.12.0`): a
//! `__requestly.json` marker, an `apis/` tree of one-concept-per-file request folders,
//! and `environments/`. Shapes that don't yet have an emitter (non-HTTP protocols, some
//! auth kinds) are recorded as `Dropped` diagnostics rather than written wrong.
//!
//! The IR→Requestly value mapping lives in [`crate::rq_shape`] and is shared with the
//! in-memory `MappedItems` emitter — one mapper, two serializations.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::{json, Value};

use cq_model::{Collection, Environment, HttpRequest, Item, Protocol, Request, Workspace};
use cq_report::{Diagnostic, Phase, Report, Severity};

use crate::rq_shape::{self, AuthMap};

fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    // serde_json::Map is a BTreeMap by default → sorted keys → byte-stable output.
    let mut s = serde_json::to_string_pretty(value).expect("json value serializes");
    s.push('\n');
    fs::write(path, s)
}

/// Make a filesystem-safe folder name from an entity name.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "request".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Emit the whole workspace under `out_dir`, creating it if needed.
pub fn emit_rq(ws: &Workspace, out_dir: &Path, report: &mut Report) -> io::Result<()> {
    fs::create_dir_all(out_dir)?;
    write_json(
        &out_dir.join("__requestly.json"),
        &json!({
            "$schema": rq_shape::schema_url("project"),
            "version": rq_shape::RQ_SCHEMA_VERSION,
            "include": [],
            "exclude": [],
        }),
    )?;

    let apis = out_dir.join("apis");
    fs::create_dir_all(&apis)?;
    for coll in &ws.collections {
        emit_collection(coll, &apis, report)?;
    }

    if !ws.environments.is_empty() {
        let envs = out_dir.join("environments");
        fs::create_dir_all(&envs)?;
        for env in &ws.environments {
            emit_environment(env, &envs)?;
        }
    }
    Ok(())
}

/// A collection with an empty name is the `apis/` root itself — its items are emitted
/// directly, without a wrapping folder.
fn emit_collection(coll: &Collection, parent: &Path, report: &mut Report) -> io::Result<()> {
    let dir = if coll.meta.name.is_empty() {
        parent.to_path_buf()
    } else {
        let d = parent.join(sanitize(&coll.meta.name));
        fs::create_dir_all(&d)?;
        write_json(
            &d.join("__metadata.json"),
            &json!({
                "$schema": rq_shape::schema_url("metadata"),
                "id": coll.meta.id,
                "name": coll.meta.name,
                "type": "collection",
                "rank": coll.meta.rank,
            }),
        )?;
        if !coll.variables.is_empty() {
            write_json(
                &d.join("__variables.json"),
                &rq_shape::variables_record(&coll.variables),
            )?;
        }
        d
    };

    for item in &coll.items {
        match item {
            Item::Request(req) => emit_request(req, &dir, report)?,
            Item::Collection(child) => emit_collection(child, &dir, report)?,
        }
    }
    Ok(())
}

fn emit_request(req: &Request, parent: &Path, report: &mut Report) -> io::Result<()> {
    let dir = parent.join(sanitize(&req.meta.name));
    fs::create_dir_all(&dir)?;
    match &req.protocol {
        Protocol::Http(http) => emit_http(req, http, &dir, report)?,
        Protocol::Graphql(_) => report.dropped(
            Phase::Emit,
            req.meta.source.clone(),
            format!(
                "GraphQL emit not yet implemented — request '{}' skipped",
                req.meta.name
            ),
        ),
    }
    Ok(())
}

fn emit_http(req: &Request, http: &HttpRequest, dir: &Path, report: &mut Report) -> io::Result<()> {
    write_json(
        &dir.join("__metadata.json"),
        &json!({
            "$schema": rq_shape::schema_url("metadata"),
            "id": req.meta.id,
            "name": req.meta.name,
            "type": "api",
            "entryType": "http",
            "url": http.url.raw,
            "method": String::from(http.method.clone()),
            "contentType": rq_shape::content_type_selector(&http.body),
            "rank": req.meta.rank,
        }),
    )?;

    if !http.headers.is_empty() {
        write_json(
            &dir.join("__headers.json"),
            &rq_shape::kvs_to_json(&http.headers),
        )?;
    }
    if !http.query.is_empty() {
        write_json(
            &dir.join("__query-params.json"),
            &rq_shape::kvs_to_json(&http.query),
        )?;
    }
    if !http.path_variables.is_empty() {
        write_json(
            &dir.join("__path-variables.json"),
            &rq_shape::path_vars_to_json(&http.path_variables),
        )?;
    }
    if let Some(body) = &http.body {
        write_json(&dir.join("__body.json"), &rq_shape::body_to_json(body))?;
    }
    if let Some(auth) = &req.auth {
        match rq_shape::auth_to_rq(auth) {
            AuthMap::Mapped(v) => {
                write_json(&dir.join("__auth.json"), &v)?;
            }
            AuthMap::NoAuth => {}
            AuthMap::Unsupported(desc) => report.dropped(
                Phase::Emit,
                req.meta.source.clone(),
                format!(
                    "auth kind not yet emittable to Requestly, dropped from '{}': {desc}",
                    req.meta.name
                ),
            ),
        }
    }
    if let Some(desc) = &req.meta.description {
        fs::write(dir.join("__README.md"), desc)?;
    }
    if req.scripts.pre_request.is_some() || req.scripts.post_response.is_some() {
        let sdir = dir.join("__scripts");
        fs::create_dir_all(&sdir)?;
        if let Some(s) = &req.scripts.pre_request {
            fs::write(sdir.join("__pre-request.ts"), &s.source)?;
        }
        if let Some(s) = &req.scripts.post_response {
            fs::write(sdir.join("__post-response.ts"), &s.source)?;
        }
    }

    report.push(Diagnostic::new(
        Severity::Ok,
        Phase::Emit,
        req.meta.source.clone(),
        format!("emitted http request '{}'", req.meta.name),
    ));
    Ok(())
}

fn emit_environment(env: &Environment, dir: &Path) -> io::Result<()> {
    let filename = if env.is_global {
        "__global.json".to_string()
    } else {
        format!("{}.json", sanitize(&env.meta.name))
    };
    write_json(
        &dir.join(filename),
        &json!({
            "$schema": rq_shape::schema_url("environment"),
            "id": env.meta.id,
            "name": env.meta.name,
            "variables": rq_shape::variables_record(&env.variables),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_model::{Auth, KeyValue, Method, ModelHeader, RecordMeta, SourceFormat, Url};
    use cq_report::Fidelity;

    fn one_request_ws() -> Workspace {
        let req = Request {
            meta: RecordMeta::new("r1", "issues", SourceFormat::Curl),
            protocol: Protocol::Http(HttpRequest {
                method: Method::Get,
                url: Url::raw("https://api.github.com/issues"),
                headers: vec![KeyValue::new("Accept", "application/json")],
                ..HttpRequest::default()
            }),
            auth: Some(Auth::Basic {
                username: "u".into(),
                password: "p".into(),
            }),
            scripts: Default::default(),
            examples: vec![],
            depends_on: vec![],
            behavior: Default::default(),
        };
        Workspace {
            meta: RecordMeta::new("ws", "", SourceFormat::Curl),
            cross_q: ModelHeader::for_source(SourceFormat::Curl),
            collections: vec![Collection {
                meta: RecordMeta::new("root", "", SourceFormat::Curl),
                items: vec![Item::Request(Box::new(req))],
                ..Collection::default()
            }],
            environments: vec![],
            packages: vec![],
        }
    }

    #[test]
    fn emits_project_and_request_tree() {
        let dir = tempfile::tempdir().unwrap();
        let mut report = Report::new(Fidelity::Lossless);
        emit_rq(&one_request_ws(), dir.path(), &mut report).unwrap();

        let marker = dir.path().join("__requestly.json");
        assert!(marker.exists());
        let marker_v: Value = serde_json::from_str(&fs::read_to_string(marker).unwrap()).unwrap();
        assert_eq!(marker_v["version"], json!("1.12.0"));

        let reqdir = dir.path().join("apis").join("issues");
        let meta: Value =
            serde_json::from_str(&fs::read_to_string(reqdir.join("__metadata.json")).unwrap())
                .unwrap();
        assert_eq!(meta["type"], json!("api"));
        assert_eq!(meta["entryType"], json!("http"));
        assert_eq!(meta["method"], json!("GET"));

        let headers: Value =
            serde_json::from_str(&fs::read_to_string(reqdir.join("__headers.json")).unwrap())
                .unwrap();
        assert_eq!(headers[0]["key"], json!("Accept"));
        assert_eq!(headers[0]["isEnabled"], json!(true));

        let auth: Value =
            serde_json::from_str(&fs::read_to_string(reqdir.join("__auth.json")).unwrap()).unwrap();
        assert_eq!(auth["type"], json!("basic_auth"));
        assert_eq!(auth["username"], json!("u"));

        assert_eq!(report.count(Severity::Ok), 1);
    }

    #[test]
    fn output_is_deterministic() {
        let ws = one_request_ws();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let mut r = Report::new(Fidelity::Lossless);
        emit_rq(&ws, a.path(), &mut r).unwrap();
        emit_rq(&ws, b.path(), &mut r).unwrap();
        let fa = fs::read_to_string(a.path().join("apis/issues/__metadata.json")).unwrap();
        let fb = fs::read_to_string(b.path().join("apis/issues/__metadata.json")).unwrap();
        assert_eq!(fa, fb);
    }
}
