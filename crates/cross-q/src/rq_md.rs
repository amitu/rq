//! The `rq` project → Idealised Model.
//!
//! The other direction of [`emit_rq_md`](crate::emit_rq_md): read a directory of Markdown
//! request documents into the model, so an `rq` collection converts to Postman, Bruno, or
//! the Requestly tree like anything else in the category.
//!
//! Input is the same virtual filesystem the Bruno importer takes — a JSON map of
//! `path → contents` — or a single `__metadata.md` document. **The directory is the tree**:
//! a folder holding `__metadata.md` is a request, every other folder is a collection, and
//! nothing stores a parent id.

use std::collections::BTreeMap;

use cq_model::{
    ApiKeyPlacement, Auth, Body, Collection, Dependency, Environment, FileRef, FormField,
    HttpRequest, Item, KeyValue, Method, ModelHeader, PathVar, Protocol, Provenance, RecordMeta,
    Request, Script, ScriptDialect, Scripts, SourceFormat, Url, VarBinding, VarCategory, VarType,
    Variable, Workspace,
};
use cq_report::{Phase, Report, Severity};
use rq_doc::{layout, AuthSpec, Document, StrMap};

/// Parse an `rq` project (a virtual-FS map) or a single request document.
pub fn parse_rq_md(content: &str, report: &mut Report) -> Result<Workspace, String> {
    if let Ok(files) = serde_json::from_str::<BTreeMap<String, String>>(content) {
        if files.keys().any(|k| {
            k == layout::MARKER
                || k.ends_with(layout::REQUEST_FILE)
                || k.ends_with(layout::COLLECTION_FILE)
        }) {
            return Ok(parse_project(&files, report));
        }
    }
    // A single request document.
    let (doc, notes) = Document::parse(content)?;
    note_all(&notes, layout::REQUEST_FILE, report);
    let request = to_request(&doc, "request", layout::REQUEST_FILE, report);
    Ok(workspace(
        Collection {
            meta: meta("rq-root", ""),
            items: vec![Item::Request(Box::new(request))],
            ..Collection::default()
        },
        Vec::new(),
    ))
}

/// Assemble a project directory into a [`Workspace`].
pub fn parse_project(files: &BTreeMap<String, String>, report: &mut Report) -> Workspace {
    let apis_prefix = format!("{}/", layout::APIS_DIR);

    let mut root = Collection {
        meta: meta("rq-root", ""),
        ..Collection::default()
    };
    if let Some(content) = files.get(&layout::collection_path("")) {
        apply_collection(content, &layout::collection_path(""), &mut root, report);
    }
    root.items = build_tree(files, &apis_prefix, "", report);

    let mut environments = Vec::new();
    for (path, content) in files {
        let Some(file) = path.strip_prefix(&format!("{}/", layout::ENVS_DIR)) else {
            continue;
        };
        let Some(name) = file.strip_suffix(".md") else {
            continue;
        };
        if file.contains('/') {
            continue;
        }
        match Document::parse(content) {
            Ok((doc, notes)) => {
                note_all(&notes, path, report);
                environments.push(Environment {
                    meta: meta(&format!("rq-env-{name}"), name),
                    is_global: name == layout::GLOBAL_ENV,
                    variables: variables(&doc, cq_model::Scope::Environment),
                });
            }
            Err(e) => report.push(cq_report::Diagnostic::new(
                Severity::Error,
                Phase::Parse,
                provenance(path),
                format!("{path}: {e}"),
            )),
        }
    }

    // `parents:` are paths; the model links by id. Resolve after the whole tree is known.
    let mut ws = workspace(root, environments);
    link_dependencies(&mut ws, report);
    ws
}

/// The immediate children of `dir`, in name order. A folder with a `__metadata.md` is a
/// request; anything else that holds files is a collection.
fn build_tree(
    files: &BTreeMap<String, String>,
    prefix: &str,
    rel: &str,
    report: &mut Report,
) -> Vec<Item> {
    let here = if rel.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}{rel}/")
    };

    let mut names: Vec<String> = Vec::new();
    for path in files.keys() {
        let Some(tail) = path.strip_prefix(&here) else {
            continue;
        };
        let Some((name, _)) = tail.split_once('/') else {
            continue;
        };
        if layout::is_reserved_dir(name) || names.iter().any(|n| n == name) {
            continue;
        }
        names.push(name.to_string());
    }
    names.sort();

    let mut items = Vec::new();
    for name in names {
        let child_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let request_at = format!("{prefix}{child_rel}/{}", layout::REQUEST_FILE);

        if let Some(content) = files.get(&request_at) {
            match Document::parse(content) {
                Ok((doc, notes)) => {
                    note_all(&notes, &request_at, report);
                    items.push(Item::Request(Box::new(to_request(
                        &doc, &name, &child_rel, report,
                    ))));
                }
                Err(e) => report.push(cq_report::Diagnostic::new(
                    Severity::Error,
                    Phase::Parse,
                    provenance(&request_at),
                    format!("{request_at}: {e}"),
                )),
            }
            continue;
        }

        let mut collection = Collection {
            meta: meta(&format!("rq-{child_rel}"), &name),
            ..Collection::default()
        };
        let doc_at = format!("{prefix}{child_rel}/{}", layout::COLLECTION_FILE);
        if let Some(content) = files.get(&doc_at) {
            apply_collection(content, &doc_at, &mut collection, report);
        }
        collection.items = build_tree(files, prefix, &child_rel, report);
        items.push(Item::Collection(Box::new(collection)));
    }
    items
}

fn apply_collection(content: &str, at: &str, collection: &mut Collection, report: &mut Report) {
    match Document::parse(content) {
        Ok((doc, notes)) => {
            note_all(&notes, at, report);
            collection.headers = key_values(&doc.front.headers);
            collection.auth = doc.front.auth.as_ref().map(auth);
            collection.variables = variables(&doc, cq_model::Scope::Collection);
            collection.scripts = scripts(&doc);
            collection.meta.description = doc.section("description").map(str::to_string);
        }
        Err(e) => report.push(cq_report::Diagnostic::new(
            Severity::Error,
            Phase::Parse,
            provenance(at),
            format!("{at}: {e}"),
        )),
    }
}

fn to_request(doc: &Document, name: &str, rel: &str, report: &mut Report) -> Request {
    let front = &doc.front;
    let url = front.url.clone().unwrap_or_default();
    if url.is_empty() {
        report.coerced(
            Phase::Map,
            provenance(rel),
            "the request has no `url:`; it was carried with an empty one",
        );
    }

    let http = HttpRequest {
        method: Method::from(front.method.clone().unwrap_or_else(|| "GET".into())),
        url: Url::raw(url),
        headers: key_values(&front.headers),
        query: key_values(&front.query),
        path_variables: front
            .path_vars
            .iter()
            .map(|(k, v)| PathVar {
                key: k.clone(),
                value: v.clone(),
                data_type: cq_model::ScalarType::default(),
                description: None,
            })
            .collect(),
        body: body(doc),
        settings: cq_model::RequestSettings {
            follow_redirects: front.follow_redirects.unwrap_or(true),
            verify_tls: front.verify_tls.unwrap_or(true),
            timeout_ms: front.timeout,
            ..cq_model::RequestSettings::default()
        },
    };

    let mut meta = meta(&format!("rq-{rel}"), name);
    meta.description = doc.section("description").map(str::to_string);

    // `parents:`/`capture:` are stashed on the node and turned into real edges once every
    // request in the workspace has an id (see `link_dependencies`).
    let depends_on = front
        .parents
        .iter()
        .map(|p| Dependency {
            target: p.clone(),
            binds: Vec::new(),
        })
        .collect();
    if !front.capture.is_empty() {
        let capture: serde_json::Map<String, serde_json::Value> = front
            .capture
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        meta.ext
            .insert(SourceFormat::Rq, serde_json::json!({ "capture": capture }));
    }

    Request {
        meta,
        protocol: Protocol::Http(http),
        auth: front.auth.as_ref().map(auth),
        scripts: scripts(doc),
        examples: Vec::new(),
        depends_on,
        behavior: Default::default(),
    }
}

/// Turn `parents:` paths into id edges, and each parent's `capture:` into the `binds` on
/// the dependency that reads it — the model's shape for the same fact.
fn link_dependencies(ws: &mut Workspace, report: &mut Report) {
    let mut by_rel: Vec<(String, String)> = Vec::new(); // (rel, id)
    let mut captures: Vec<(String, Vec<(String, String)>)> = Vec::new(); // (id, [(var, path)])
    collect(&ws.collections, &mut by_rel, &mut captures);

    fn collect(
        collections: &[Collection],
        by_rel: &mut Vec<(String, String)>,
        captures: &mut Vec<(String, Vec<(String, String)>)>,
    ) {
        for c in collections {
            for item in &c.items {
                match item {
                    Item::Collection(child) => {
                        collect(std::slice::from_ref(child.as_ref()), by_rel, captures)
                    }
                    Item::Request(r) => {
                        let rel = r.meta.id.trim_start_matches("rq-").to_string();
                        by_rel.push((rel, r.meta.id.clone()));
                        if let Some(bag) = r.meta.ext.get(&SourceFormat::Rq) {
                            if let Some(map) = bag.get("capture").and_then(|c| c.as_object()) {
                                captures.push((
                                    r.meta.id.clone(),
                                    map.iter()
                                        .map(|(k, v)| {
                                            (k.clone(), v.as_str().unwrap_or_default().to_string())
                                        })
                                        .collect(),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    let resolve = |rel_or_name: &str, from: &str| -> Option<String> {
        // A qualified path first, then a bare name resolved outward from the declaring
        // request — the same scoping rule `rq r` uses.
        if let Some((_, id)) = by_rel.iter().find(|(rel, _)| rel == rel_or_name) {
            return Some(id.clone());
        }
        let mut scope = from.rsplit_once('/').map(|(head, _)| head.to_string());
        while let Some(prefix) = scope.clone() {
            let candidate = format!("{prefix}/{rel_or_name}");
            if let Some((_, id)) = by_rel.iter().find(|(rel, _)| *rel == candidate) {
                return Some(id.clone());
            }
            scope = prefix.rsplit_once('/').map(|(head, _)| head.to_string());
        }
        by_rel
            .iter()
            .find(|(rel, _)| rel.rsplit('/').next() == Some(rel_or_name))
            .map(|(_, id)| id.clone())
    };

    fn walk(collections: &mut [Collection], f: &mut impl FnMut(&mut Request)) {
        for c in collections {
            for item in &mut c.items {
                match item {
                    Item::Collection(child) => walk(std::slice::from_mut(child.as_mut()), f),
                    Item::Request(r) => f(r),
                }
            }
        }
    }

    let mut unresolved: Vec<(String, String)> = Vec::new();
    walk(&mut ws.collections, &mut |r| {
        let from = r.meta.id.trim_start_matches("rq-").to_string();
        for dep in &mut r.depends_on {
            match resolve(&dep.target, &from) {
                Some(id) => {
                    dep.binds = captures
                        .iter()
                        .find(|(cid, _)| *cid == id)
                        .map(|(_, pairs)| {
                            pairs
                                .iter()
                                .map(|(var, path)| VarBinding {
                                    from: path.clone(),
                                    to: var.clone(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    dep.target = id;
                }
                None => unresolved.push((from.clone(), dep.target.clone())),
            }
        }
    });

    for (from, target) in unresolved {
        report.dropped(
            Phase::Map,
            provenance(&from),
            format!("`parents: [{target}]` names a request that isn't in this project"),
        );
    }
}

fn body(doc: &Document) -> Option<Body> {
    let front = &doc.front;
    if let Some(fields) = &front.form {
        return Some(Body::UrlEncoded {
            fields: key_values(fields),
        });
    }
    if let Some(fields) = &front.form_data {
        return Some(Body::FormData {
            fields: fields
                .iter()
                .map(|(k, v)| match v.strip_prefix('@') {
                    Some(path) => FormField::File(FileRef::Reference {
                        id: format!("rq-file-{k}"),
                        name: k.clone(),
                        path: path.to_string(),
                        size: 0,
                        source: String::new(),
                    }),
                    None => FormField::Text(key_value(k, v)),
                })
                .collect(),
        });
    }
    if let Some(path) = &front.file {
        return Some(Body::Binary {
            file: FileRef::Reference {
                id: "rq-body-file".into(),
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                path: path.clone(),
                size: 0,
                source: String::new(),
            },
        });
    }
    let text = doc.section("body").filter(|s| !s.trim().is_empty())?;
    Some(Body::Raw {
        text: text.to_string(),
        media_type: front.body_type.clone().unwrap_or_default(),
    })
}

/// Scripts come back with their dialect — `rq.*` unless the file says otherwise. The
/// converter never rewrites them; it records what they are.
fn scripts(doc: &Document) -> Scripts {
    let read = |name: &str| -> Option<Script> {
        doc.section(name)
            .filter(|s| !s.trim().is_empty())
            .map(|source| Script {
                source: source.to_string(),
                language: Default::default(),
                dialect: ScriptDialect::Rq,
            })
    };
    Scripts {
        pre_request: read("pre"),
        post_response: read("post"),
    }
}

fn variables(doc: &Document, scope: cq_model::Scope) -> Vec<Variable> {
    doc.front
        .vars
        .iter()
        .map(|(key, spec)| Variable {
            key: key.clone(),
            value: spec.default.clone().unwrap_or_default(),
            initial: None,
            scope,
            data_type: if spec.secret {
                VarType::Secret
            } else {
                VarType::String
            },
            category: VarCategory::Scoped,
            enabled: true,
            rank: None,
        })
        .collect()
}

fn auth(spec: &AuthSpec) -> Auth {
    match spec {
        AuthSpec::None => Auth::None,
        AuthSpec::Inherit => Auth::Inherit,
        AuthSpec::Basic { username, password } => Auth::Basic {
            username: username.clone(),
            password: password.clone(),
        },
        AuthSpec::Bearer { token, prefix } => Auth::Bearer {
            token: token.clone(),
            header_prefix: prefix.clone(),
        },
        AuthSpec::ApiKey {
            key,
            value,
            in_query,
        } => Auth::ApiKey {
            key: key.clone(),
            value: value.clone(),
            placement: if *in_query {
                ApiKeyPlacement::Query
            } else {
                ApiKeyPlacement::Header
            },
        },
        // An auth kind the CLI can't send is still a credential: carry it whole.
        AuthSpec::Other { kind, raw } => Auth::Unknown {
            raw_type: kind.clone(),
            raw: yaml_to_json(&rq_doc::Value::Mapping(raw.clone())),
        },
    }
}

fn yaml_to_json(v: &rq_doc::Value) -> serde_json::Value {
    match v {
        rq_doc::Value::Null => serde_json::Value::Null,
        rq_doc::Value::Bool(b) => serde_json::Value::Bool(*b),
        rq_doc::Value::Number(n) => n
            .as_u64()
            .map(serde_json::Value::from)
            .or_else(|| n.as_i64().map(serde_json::Value::from))
            .or_else(|| n.as_f64().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
        rq_doc::Value::String(s) => serde_json::Value::String(s.clone()),
        rq_doc::Value::Sequence(seq) => {
            serde_json::Value::Array(seq.iter().map(yaml_to_json).collect())
        }
        rq_doc::Value::Mapping(m) => serde_json::Value::Object(
            m.iter()
                .map(|(k, val)| {
                    (
                        k.as_str().unwrap_or_default().to_string(),
                        yaml_to_json(val),
                    )
                })
                .collect(),
        ),
        rq_doc::Value::Tagged(t) => yaml_to_json(&t.value),
    }
}

fn key_values(pairs: &StrMap) -> Vec<KeyValue> {
    pairs.iter().map(|(k, v)| key_value(k, v)).collect()
}

fn key_value(key: &str, value: &str) -> KeyValue {
    KeyValue::new(key, value)
}

fn meta(id: &str, name: &str) -> RecordMeta {
    RecordMeta::new(id, name, SourceFormat::Rq)
}

fn provenance(locator: &str) -> Provenance {
    Provenance {
        format: SourceFormat::Rq,
        locator: locator.to_string(),
    }
}

fn note_all(notes: &[rq_doc::Note], at: &str, report: &mut Report) {
    for note in notes {
        report.coerced(Phase::Parse, provenance(at), format!("{at}: {note}"));
    }
}

fn workspace(root: Collection, environments: Vec<Environment>) -> Workspace {
    Workspace {
        meta: meta("rq-workspace", ""),
        cross_q: ModelHeader::for_source(SourceFormat::Rq),
        collections: vec![root],
        environments,
        packages: Vec::new(),
    }
}
