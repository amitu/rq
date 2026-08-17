//! The `rq` project → Idealised Model.
//!
//! The other direction of [`emit_rq_md`](crate::emit_rq_md): read a directory of Markdown
//! request documents into the model, so an `rq` collection converts to Postman, Bruno, or
//! the Requestly tree like anything else in the category.
//!
//! Input is the same virtual filesystem the Bruno importer takes — a JSON map of
//! `path → contents` — or a single request document. **The directory is the tree**: every
//! `*.md` is a request, every directory is a collection, an `index.md` is what its
//! directory shares, and nothing stores a parent id.

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
        if files
            .keys()
            .any(|k| k == layout::MARKER || k.ends_with(".md") || k == layout::DOTENV)
        {
            return Ok(parse_project(&files, report));
        }
    }
    // A single request document.
    let (doc, notes) = Document::parse(content)?;
    note_all(&notes, "request.md", report);
    let request = to_request(&doc, "request", "request.md", report);
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
    let mut root = Collection {
        meta: meta("rq-root", ""),
        ..Collection::default()
    };
    if let Some(content) = files.get(layout::COLLECTION_FILE) {
        apply_collection(content, layout::COLLECTION_FILE, &mut root, report);
    }
    root.items = build_tree(files, "", report);

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
                    is_global: false,
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
    // The always-on layer is the global environment, in the model's terms.
    if let Some(content) = files.get(layout::DOTENV) {
        environments.push(Environment {
            meta: meta("rq-env-global", ""),
            is_global: true,
            variables: dotenv_variables(content),
        });
    }

    let mut ws = workspace(root, environments);
    link_dependencies(&mut ws, report);
    ws
}

/// `KEY=value` lines, `#` comments, an optional `export ` — the dotenv everyone has.
fn dotenv_variables(content: &str) -> Vec<Variable> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.strip_prefix("export ").unwrap_or(line).split_once('='))
        .map(|(key, value)| Variable {
            key: key.trim().to_string(),
            value: value
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string(),
            initial: None,
            scope: cq_model::Scope::Global,
            data_type: VarType::String,
            category: VarCategory::Scoped,
            enabled: true,
            rank: None,
        })
        .collect()
}

/// The children of `rel`: every `*.md` is a request, every directory a collection.
fn build_tree(files: &BTreeMap<String, String>, rel: &str, report: &mut Report) -> Vec<Item> {
    let prefix = if rel.is_empty() {
        String::new()
    } else {
        format!("{rel}/")
    };

    let mut requests: Vec<(String, &String)> = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    for (path, content) in files {
        let Some(tail) = path.strip_prefix(&prefix) else {
            continue;
        };
        match tail.split_once('/') {
            Some((dir, _)) => {
                if !layout::is_reserved_dir(dir) && !dirs.iter().any(|d| d == dir) {
                    dirs.push(dir.to_string());
                }
            }
            None => {
                if layout::is_request_file(tail) {
                    if let Some(name) = layout::request_name(tail) {
                        requests.push((name.to_string(), content));
                    }
                }
            }
        }
    }
    requests.sort_by(|a, b| a.0.cmp(&b.0));
    dirs.sort();

    let mut items = Vec::new();
    for (name, content) in requests {
        let child_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let at = layout::request_path(&child_rel);
        match Document::parse(content) {
            Ok((doc, notes)) => {
                note_all(&notes, &at, report);
                items.push(Item::Request(Box::new(to_request(
                    &doc, &name, &child_rel, report,
                ))));
            }
            Err(e) => report.push(cq_report::Diagnostic::new(
                Severity::Error,
                Phase::Parse,
                provenance(&at),
                format!("{at}: {e}"),
            )),
        }
    }
    for name in dirs {
        let child_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let mut collection = Collection {
            meta: meta(&format!("rq-{child_rel}"), &name),
            ..Collection::default()
        };
        let doc_at = layout::collection_path(&child_rel);
        if let Some(content) = files.get(&doc_at) {
            apply_collection(content, &doc_at, &mut collection, report);
        }
        collection.items = build_tree(files, &child_rel, report);
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
