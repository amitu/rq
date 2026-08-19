//! Idealised Model → the `rq` project: one Markdown file per request.
//!
//! The target the `rq` CLI reads. `rq curl --save-as` and `rq import` land here too — the
//! CLI owns no conversion of its own, it writes what this emits.
//!
//! Output is a virtual filesystem (`path → contents`), like the Bruno emitter: the caller
//! decides whether that becomes files on disk, a zip, or an object in a browser. Nothing
//! here touches the filesystem, so the emitter works unchanged in WASM.
//!
//! It keeps the converter's promise: anything the `.md` form can't carry is a diagnostic
//! on the way out, never a silent drop.

use std::collections::BTreeMap;

use cq_model::{
    ApiKeyPlacement, Auth, Body, Collection, Environment, FileRef, FormField, Item, KeyValue,
    KvKind, Protocol, Provenance, Request, ScriptDialect, Scripts, SourceFormat, VarType, Variable,
    Workspace,
};
use cq_report::{Phase, Report};
use rq_doc::layout;
use rq_doc::{AuthSpec, Document, Mapping, Value, VarSpec};

/// Emit a workspace as an `rq` project. Keys are paths relative to the project root.
pub fn to_rq_md(ws: &Workspace, report: &mut Report) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    out.insert(layout::MARKER.to_string(), layout::marker());

    let mut pending: Vec<Pending> = Vec::new();
    let mut taken: Vec<String> = Vec::new();

    for collection in &ws.collections {
        // A workspace's root collection is usually unnamed — its children land directly
        // under `apis/` rather than inside an "Untitled" folder.
        if collection.meta.name.trim().is_empty() {
            if let Some(doc) = collection_doc(collection, report) {
                out.insert(layout::collection_path(""), doc.write());
            }
            emit_items(
                &collection.items,
                "",
                &mut taken,
                &mut pending,
                &mut out,
                report,
            );
        } else {
            emit_collection(collection, "", &mut taken, &mut pending, &mut out, report);
        }
    }

    resolve_dependencies(&mut pending, report);
    for item in pending {
        out.insert(layout::request_path(&item.rel), item.doc.write());
    }

    for env in &ws.environments {
        // The global environment becomes the always-on layer, and that is a `.env` — the
        // file every project already has one of.
        if env.is_global {
            let mut lines = String::from("# The always-on variable layer.\n");
            for var in &env.variables {
                if !var.key.is_empty() {
                    lines.push_str(&format!("{}={}\n", var.key, var.value));
                }
            }
            out.insert(layout::DOTENV.to_string(), lines);
            continue;
        }
        let name = layout::slug_segment(&env.meta.name, "environment");
        out.insert(
            layout::environment_path(&name),
            environment_doc(env).write(),
        );
    }

    if !ws.packages.is_empty() {
        report.dropped(
            Phase::Emit,
            provenance("packages"),
            format!(
                "{} reusable script package(s) were not written: the rq format has no home \
                 for them yet",
                ws.packages.len()
            ),
        );
    }
    out
}

/// A request built but not yet placed: dependencies resolve once every request has its
/// final path, because `parents:` refers to requests by path and the model by id.
struct Pending {
    id: String,
    rel: String,
    doc: Document,
    deps: Vec<cq_model::Dependency>,
}

fn emit_collection(
    collection: &Collection,
    prefix: &str,
    taken: &mut Vec<String>,
    pending: &mut Vec<Pending>,
    out: &mut BTreeMap<String, String>,
    report: &mut Report,
) {
    let rel = unique(
        prefix,
        &layout::slug_segment(&collection.meta.name, "collection"),
        taken,
    );
    let doc = collection_doc(collection, report);
    let has_doc = doc.is_some();
    if let Some(doc) = doc {
        out.insert(layout::collection_path(&rel), doc.write());
    }

    let before = pending.len();
    emit_items(&collection.items, &rel, taken, pending, out, report);

    // A directory only exists on disk because a file is in it. An empty collection with
    // nothing to say would vanish — say so rather than lose the node.
    if !has_doc && pending.len() == before {
        report.dropped(
            Phase::Emit,
            provenance(&rel),
            "an empty collection with no settings has no file to live in",
        );
    }
}

fn emit_items(
    items: &[Item],
    prefix: &str,
    taken: &mut Vec<String>,
    pending: &mut Vec<Pending>,
    out: &mut BTreeMap<String, String>,
    report: &mut Report,
) {
    for item in items {
        match item {
            Item::Collection(c) => emit_collection(c, prefix, taken, pending, out, report),
            Item::Request(r) => emit_request(r, prefix, taken, pending, report),
        }
    }
}

fn collection_doc(collection: &Collection, report: &mut Report) -> Option<Document> {
    let mut doc = Document::default();
    doc.front.headers = kv_pairs(&collection.headers, &collection.meta.name, report);
    doc.front.auth = collection
        .auth
        .as_ref()
        .map(|a| auth_spec(a, &collection.meta.name, report));
    doc.front.vars = collection.variables.iter().filter_map(var_spec).collect();
    if let Some(d) = collection
        .meta
        .description
        .as_ref()
        .filter(|d| !d.trim().is_empty())
    {
        doc.set_section("description", d.clone());
    }
    add_scripts(&mut doc, &collection.scripts, &collection.meta.name, report);

    // Nothing to say → no file. An empty `__collection.md` is noise in a diff.
    if doc.front.headers.is_empty()
        && doc.front.auth.is_none()
        && doc.front.vars.is_empty()
        && doc.sections.is_empty()
    {
        return None;
    }
    Some(doc)
}

fn environment_doc(env: &Environment) -> Document {
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
    doc
}

fn emit_request(
    req: &Request,
    prefix: &str,
    taken: &mut Vec<String>,
    pending: &mut Vec<Pending>,
    report: &mut Report,
) {
    let rel = unique(
        prefix,
        &layout::slug_segment(&req.meta.name, "request"),
        taken,
    );
    let mut doc = Document::default();

    match &req.protocol {
        Protocol::Http(http) => {
            doc.front.method = Some(String::from(http.method.clone()));
            doc.front.headers = kv_pairs(&http.headers, &rel, report);
            doc.front.query = kv_pairs(&http.query, &rel, report);
            // The IR carries the query BOTH ways — parsed into `query`, and still present in
            // `url.raw`, because that is what the source formats hand over. rq's `query:`
            // block is *appended* to `url:` (RQ-FORMAT), so emitting both sent every
            // imported request out as `?page=1&page=1`. The parsed pairs are the ones with
            // names, so they win and the raw copy is dropped.
            doc.front.url = Some(if doc.front.query.is_empty() {
                http.url.raw.clone()
            } else {
                strip_query(&http.url.raw)
            });
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
                apply_body(&mut doc, body, &rel, report);
            }
        }
        Protocol::Graphql(gql) => {
            doc.front.method = Some(String::from(gql.method.clone()));
            doc.front.url = Some(gql.url.raw.clone());
            doc.front.headers = kv_pairs(&gql.headers, &rel, report);
            doc.front.body_type = Some("graphql".into());
            doc.set_section(
                "body",
                graphql_body(&gql.query, &gql.variables, gql.operation_name.as_deref()),
            );
            report.coerced(
                Phase::Emit,
                provenance(&rel),
                "a GraphQL request was written as its JSON POST body",
            );
        }
    }

    doc.front.auth = req.auth.as_ref().map(|a| auth_spec(a, &rel, report));
    if let Some(d) = req
        .meta
        .description
        .as_ref()
        .filter(|d| !d.trim().is_empty())
    {
        doc.set_section("description", d.clone());
    }
    add_scripts(&mut doc, &req.scripts, &rel, report);

    if !req.examples.is_empty() {
        report.dropped(
            Phase::Emit,
            provenance(&rel),
            format!(
                "{} saved response example(s) were not written: the rq format has no home \
                 for them yet",
                req.examples.len()
            ),
        );
    }
    if req.meta.disabled {
        report.coerced(
            Phase::Emit,
            provenance(&rel),
            "the request was disabled in the source and was written as active",
        );
    }

    pending.push(Pending {
        id: req.meta.id.clone(),
        rel,
        doc,
        deps: req.depends_on.clone(),
    });
}

/// Turn the model's `depends_on` (ids) into the document's `parents:` (paths), and its
/// `binds` into a `capture:` on the request that produces the value.
fn resolve_dependencies(pending: &mut [Pending], report: &mut Report) {
    let by_id: Vec<(String, String)> = pending
        .iter()
        .map(|p| (p.id.clone(), p.rel.clone()))
        .collect();

    let mut captures: Vec<(String, String, String)> = Vec::new(); // (parent id, var, path)
    for item in pending.iter_mut() {
        for dep in &item.deps {
            match by_id.iter().find(|(id, _)| *id == dep.target) {
                Some((_, rel)) => item.doc.front.parents.push(rel.clone()),
                None => report.dropped(
                    Phase::Emit,
                    provenance(&item.rel),
                    format!(
                        "`depends_on` points at a request that isn't in this workspace ({})",
                        dep.target
                    ),
                ),
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

fn apply_body(doc: &mut Document, body: &Body, rel: &str, report: &mut Report) {
    match body {
        Body::None => {}
        Body::Raw { text, media_type } => {
            if !media_type.is_empty() {
                doc.front.body_type = Some(media_type.clone());
            }
            doc.set_section("body", text.clone());
        }
        Body::UrlEncoded { fields } => {
            doc.front.form = Some(kv_pairs(fields, rel, report));
        }
        Body::FormData { fields } => {
            let mut pairs = Vec::new();
            for field in fields {
                match field {
                    FormField::Text(kv) if kv.enabled => {
                        pairs.push((kv.key.clone(), kv.value.clone()))
                    }
                    FormField::Text(_) => {}
                    FormField::File(FileRef::Reference { name, path, .. }) => {
                        pairs.push((name.clone(), format!("@{path}")));
                    }
                    FormField::File(FileRef::Content { name, .. }) => report.dropped(
                        Phase::Emit,
                        provenance(rel),
                        format!("inline file part `{name}` has no path and was skipped"),
                    ),
                }
            }
            doc.front.form_data = Some(pairs);
        }
        Body::Binary { file } => match file {
            FileRef::Reference { path, .. } => doc.front.file = Some(path.clone()),
            FileRef::Content { name, .. } => report.dropped(
                Phase::Emit,
                provenance(rel),
                format!("inline binary body `{name}` has no path and was skipped"),
            ),
        },
        Body::Graphql {
            query,
            variables,
            operation_name,
        } => {
            doc.front.body_type = Some("graphql".into());
            doc.set_section(
                "body",
                graphql_body(query, variables, operation_name.as_deref()),
            );
        }
    }
}

/// `https://x/y?a=1#frag` → `https://x/y#frag`. Only the query goes; a fragment is not a
/// query parameter and rq does not synthesise one.
fn strip_query(raw: &str) -> String {
    let Some(q) = raw.find('?') else {
        return raw.to_string();
    };
    match raw[q..].find('#') {
        Some(h) => format!("{}{}", &raw[..q], &raw[q + h..]),
        None => raw[..q].to_string(),
    }
}

fn graphql_body(query: &str, variables: &str, operation_name: Option<&str>) -> String {
    let payload = serde_json::json!({
        "query": query,
        "variables": variables,
        "operationName": operation_name,
    });
    serde_json::to_string_pretty(&payload).unwrap_or_default()
}

/// Scripts are carried **verbatim** with their dialect noted. A `pm.*` script is not
/// rewritten to `rq.*` on the way through: a textual rename imports clean and throws at
/// run time, which is the one failure this project refuses to ship.
fn add_scripts(doc: &mut Document, scripts: &Scripts, rel: &str, report: &mut Report) {
    for (section, script) in [
        ("pre", scripts.pre_request.as_ref()),
        ("post", scripts.post_response.as_ref()),
    ] {
        let Some(script) = script.filter(|s| !s.source.trim().is_empty()) else {
            continue;
        };
        doc.set_section(section, script.source.clone());
        if !matches!(script.dialect, ScriptDialect::Rq) {
            report.coerced(
                Phase::Emit,
                provenance(rel),
                format!(
                    "`-- {section} --` is written in the {:?} dialect and was kept verbatim \
                     (rq's runtime reconciles it; the converter never renames a script)",
                    script.dialect
                ),
            );
        }
    }
}

fn kv_pairs(pairs: &[KeyValue], at: &str, report: &mut Report) -> Vec<(String, String)> {
    let mut kept = Vec::new();
    for kv in pairs {
        if !kv.enabled {
            report.dropped(
                Phase::Emit,
                provenance(at),
                format!(
                    "`{}` was disabled in the source; the rq format has no disabled flag",
                    kv.key
                ),
            );
            continue;
        }
        if kv.kind == KvKind::File {
            report.coerced(
                Phase::Emit,
                provenance(at),
                format!(
                    "`{}` is a file field outside a form body and was written as text",
                    kv.key
                ),
            );
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
            ..VarSpec::default()
        },
    ))
}

/// Auth kinds `rq` can send become first-class; the rest are preserved verbatim under
/// their own type so the credential survives the trip.
fn auth_spec(auth: &Auth, at: &str, report: &mut Report) -> AuthSpec {
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
            in_query: matches!(placement, ApiKeyPlacement::Query),
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
                    // The document spells the discriminant `type`, not `kind`.
                    m.remove(Value::from("kind"));
                    m.insert("type".into(), kind.as_str().into());
                    m
                }
                _ => Mapping::new(),
            };
            report.coerced(
                Phase::Emit,
                provenance(at),
                format!("auth `{kind}` was preserved verbatim; the rq CLI cannot send it yet"),
            );
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

/// Two entities can share a name in a source collection; two directories cannot.
fn unique(prefix: &str, name: &str, taken: &mut Vec<String>) -> String {
    let base = if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while taken.contains(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    taken.push(candidate.clone());
    candidate
}

fn provenance(locator: &str) -> Provenance {
    Provenance {
        format: SourceFormat::Rq,
        locator: locator.to_string(),
    }
}
