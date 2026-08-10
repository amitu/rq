//! Idealised Model → Bruno `.bru` (v2). The reverse of [`crate::bruno`].
//!
//! A Bruno collection is a directory tree, so [`to_bruno`] returns a **virtual-FS map**
//! `{ path: contents }` — the same shape the importer consumes — which the host (or the
//! native CLI) writes out as files. [`emit_request`] produces one request `.bru`; the rest
//! reconstruct `bruno.json`, `collection.bru`/`folder.bru`, and `environments/*.bru`.
//!
//! Scripts are written **verbatim by dialect** (see the README's "Scripts & JS dialects"):
//! cross-q never rewrites `pm.`→`bru.`. A script whose dialect isn't Bruno's native `bru`
//! still round-trips through here unchanged; reconciling dialects is `cross-q-context`'s job.
//!
//! The fidelity contract is **semantic idempotence**: `.bru` → IR → `.bru` → IR recovers the
//! same IR (see `tests/bruno_roundtrip.rs`), so the exporter is proven not to drop a field
//! rather than merely to produce plausible text.

use std::collections::BTreeMap;
use std::fmt::Write;

use cq_model::{
    Auth, Body, Collection, Environment, FormField, Item, KeyValue, Protocol, Request,
    RequestBehavior, Scripts, VarType, Variable, Workspace,
};

/// Emit a [`Workspace`] as a Bruno collection: a map of relative path → file contents.
pub fn to_bruno(ws: &Workspace) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let root = match ws.collections.first() {
        Some(c) => c,
        None => return files,
    };

    files.insert("bruno.json".into(), bruno_json(&root.meta.name));
    let coll = emit_container(root);
    if !coll.trim().is_empty() {
        files.insert("collection.bru".into(), coll);
    }
    for env in &ws.environments {
        files.insert(
            format!("environments/{}.bru", env.meta.name),
            emit_environment(env),
        );
    }
    emit_items(&root.items, "", &mut files);
    files
}

/// Write a collection's children into `files` under directory `dir` ("" = root).
fn emit_items(items: &[Item], dir: &str, files: &mut BTreeMap<String, String>) {
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    for (i, item) in items.iter().enumerate() {
        match item {
            Item::Request(req) => {
                files.insert(
                    format!("{prefix}{}.bru", slug(&req.meta.name)),
                    emit_request(req),
                );
            }
            Item::Collection(folder) => {
                let child = format!("{prefix}{}", slug(&folder.meta.name));
                let mut fb = format!(
                    "meta {{\n  name: {}\n  seq: {}\n}}\n",
                    folder.meta.name,
                    folder
                        .meta
                        .rank
                        .clone()
                        .unwrap_or_else(|| (i + 1).to_string())
                );
                let body = emit_container(folder);
                if !body.trim().is_empty() {
                    fb.push('\n');
                    fb.push_str(&body);
                }
                files.insert(format!("{child}/folder.bru"), fb);
                emit_items(&folder.items, &child, files);
            }
        }
    }
}

/// Emit one request `.bru`.
pub fn emit_request(req: &Request) -> String {
    let mut s = String::new();
    // meta
    let _ = writeln!(s, "meta {{\n  name: {}\n  type: http", req.meta.name);
    if let Some(seq) = &req.meta.rank {
        let _ = writeln!(s, "  seq: {seq}");
    }
    s.push_str("}\n\n");

    let Protocol::Http(http) = &req.protocol else {
        return s;
    };

    // verb block: method + url + declared body/auth modes
    let method = String::from(http.method.clone()).to_ascii_lowercase();
    let _ = writeln!(
        s,
        "{method} {{\n  url: {}\n  body: {}",
        http.url.raw,
        body_mode(&http.body)
    );
    if let Some(mode) = auth_mode(&req.auth) {
        let _ = writeln!(s, "  auth: {mode}");
    }
    s.push_str("}\n");

    if !http.query.is_empty() {
        s.push('\n');
        s.push_str(&kv_block("params:query", &http.query));
    }
    if !http.path_variables.is_empty() {
        let pv: Vec<KeyValue> = http
            .path_variables
            .iter()
            .map(|p| KeyValue::new(p.key.clone(), p.value.clone()))
            .collect();
        s.push('\n');
        s.push_str(&kv_block("params:path", &pv));
    }
    if !http.headers.is_empty() {
        s.push('\n');
        s.push_str(&kv_block("headers", &http.headers));
    }
    if let Some(block) = auth_block(&req.auth) {
        s.push('\n');
        s.push_str(&block);
    }
    if let Some(block) = body_block(&http.body) {
        s.push('\n');
        s.push_str(&block);
    }
    emit_behavior(&mut s, &req.behavior);
    emit_scripts(&mut s, &req.scripts);
    if let Some(desc) = &req.meta.description {
        s.push('\n');
        s.push_str(&text_block("docs", desc));
    }
    s
}

/// Emit the shared, inheritable blocks of a `collection.bru`/`folder.bru` (headers, auth,
/// vars, scripts, docs) — everything except the folder's own `meta` block.
fn emit_container(coll: &Collection) -> String {
    let mut s = String::new();
    if !coll.headers.is_empty() {
        s.push_str(&kv_block("headers", &coll.headers));
        s.push('\n');
    }
    if let Some(mode) = auth_mode(&coll.auth) {
        let _ = writeln!(s, "auth {{\n  mode: {mode}\n}}");
        if let Some(block) = auth_block(&coll.auth) {
            s.push('\n');
            s.push_str(&block);
            s.push('\n');
        }
    }
    // Collection-level variables are emitted as `vars:pre-request` (Bruno's model).
    if !coll.variables.is_empty() {
        s.push_str(&vars_block("vars:pre-request", &coll.variables));
        s.push('\n');
    }
    emit_scripts(&mut s, &coll.scripts);
    if let Some(desc) = &coll.meta.description {
        s.push_str(&text_block("docs", desc));
        s.push('\n');
    }
    s
}

fn emit_environment(env: &Environment) -> String {
    let mut s = String::new();
    let plain: Vec<&Variable> = env
        .variables
        .iter()
        .filter(|v| v.data_type != VarType::Secret)
        .collect();
    let secret: Vec<&Variable> = env
        .variables
        .iter()
        .filter(|v| v.data_type == VarType::Secret)
        .collect();
    if !plain.is_empty() {
        s.push_str(&vars_block(
            "vars",
            &plain.into_iter().cloned().collect::<Vec<_>>(),
        ));
    }
    if !secret.is_empty() {
        s.push_str("vars:secret [\n");
        for (i, v) in secret.iter().enumerate() {
            let comma = if i + 1 < secret.len() { "," } else { "" };
            let _ = writeln!(s, "  {}{comma}", v.key);
        }
        s.push_str("]\n");
    }
    s
}

fn emit_behavior(s: &mut String, b: &RequestBehavior) {
    if !b.pre_request_vars.is_empty() {
        s.push('\n');
        s.push_str(&vars_block("vars:pre-request", &b.pre_request_vars));
    }
    if !b.post_response_vars.is_empty() {
        s.push('\n');
        s.push_str(&vars_block("vars:post-response", &b.post_response_vars));
    }
    if !b.asserts.is_empty() {
        s.push_str("\nassert {\n");
        for a in &b.asserts {
            let dis = if a.enabled { "" } else { "~" };
            let _ = writeln!(s, "  {dis}{}: {}", a.expr, a.predicate);
        }
        s.push_str("}\n");
    }
}

fn emit_scripts(s: &mut String, scripts: &Scripts) {
    if let Some(sc) = &scripts.pre_request {
        s.push('\n');
        s.push_str(&text_block("script:pre-request", &sc.source));
    }
    if let Some(sc) = &scripts.post_response {
        s.push('\n');
        s.push_str(&text_block("script:post-response", &sc.source));
    }
}

// ---------------------------------------------------------------------------------------
// Block writers
// ---------------------------------------------------------------------------------------

/// A `key: value` dictionary block, with `~` for disabled and `@description(...)` for a
/// description — the inverse of [`crate::bruno`]'s `entries`.
fn kv_block(name: &str, kvs: &[KeyValue]) -> String {
    let mut s = format!("{name} {{\n");
    for kv in kvs {
        if let Some(desc) = &kv.description {
            let _ = writeln!(s, "  @description('''{desc}''')");
        }
        let dis = if kv.enabled { "" } else { "~" };
        let _ = writeln!(s, "  {dis}{}: {}", kv.key, kv.value);
    }
    s.push_str("}\n");
    s
}

/// A `vars:*` block, emitting the `@number`/`@boolean` type tag and `~` disabled flag.
fn vars_block(name: &str, vars: &[Variable]) -> String {
    let mut s = format!("{name} {{\n");
    for v in vars {
        match v.data_type {
            VarType::Number => s.push_str("  @number\n"),
            VarType::Boolean => s.push_str("  @boolean\n"),
            _ => {}
        }
        let dis = if v.enabled { "" } else { "~" };
        let _ = writeln!(s, "  {dis}{}: {}", v.key, v.value);
    }
    s.push_str("}\n");
    s
}

/// A verbatim text block (body/script/docs): indent every content line by 2 spaces — the
/// inverse of the importer's outdent.
fn text_block(name: &str, content: &str) -> String {
    let mut s = format!("{name} {{\n");
    for line in content.split('\n') {
        if line.is_empty() {
            s.push('\n');
        } else {
            let _ = writeln!(s, "  {line}");
        }
    }
    s.push_str("}\n");
    s
}

fn body_mode(body: &Option<Body>) -> &'static str {
    match body {
        None | Some(Body::None) => "none",
        Some(Body::Raw { media_type, .. }) => {
            if media_type.contains("json") {
                "json"
            } else if media_type.contains("xml") {
                "xml"
            } else if media_type.contains("sparql") {
                "sparql"
            } else {
                "text"
            }
        }
        Some(Body::Graphql { .. }) => "graphql",
        Some(Body::UrlEncoded { .. }) => "formUrlEncoded",
        Some(Body::FormData { .. }) => "multipartForm",
        Some(Body::Binary { .. }) => "file",
    }
}

fn body_block(body: &Option<Body>) -> Option<String> {
    match body {
        None | Some(Body::None) => None,
        Some(Body::Raw { text, media_type }) => {
            let tag = format!(
                "body:{}",
                body_mode(&Some(Body::Raw {
                    text: text.clone(),
                    media_type: media_type.clone()
                }))
            );
            Some(text_block(&tag, text))
        }
        Some(Body::Graphql {
            query, variables, ..
        }) => {
            let mut s = text_block("body:graphql", query);
            if !variables.is_empty() {
                s.push('\n');
                s.push_str(&text_block("body:graphql:vars", variables));
            }
            Some(s)
        }
        Some(Body::UrlEncoded { fields }) => Some(kv_block("body:form-urlencoded", fields)),
        Some(Body::FormData { fields }) => {
            let kvs: Vec<KeyValue> = fields
                .iter()
                .filter_map(|f| match f {
                    FormField::Text(kv) => Some(kv.clone()),
                    FormField::File(_) => None,
                })
                .collect();
            Some(kv_block("body:multipart-form", &kvs))
        }
        Some(Body::Binary { .. }) => None,
    }
}

/// The `auth:` mode string a request/collection declares, or `None` when auth is unspecified
/// (so the importer recovers `None`, not `Some(Auth::None)`).
fn auth_mode(auth: &Option<Auth>) -> Option<&str> {
    Some(match auth.as_ref()? {
        Auth::None => "none",
        Auth::Inherit => "inherit",
        Auth::Basic { .. } => "basic",
        Auth::Bearer { .. } => "bearer",
        Auth::ApiKey { .. } => "apikey",
        Auth::OAuth2 { .. } => "oauth2",
        Auth::OAuth1 { .. } => "oauth1",
        Auth::Digest { .. } => "digest",
        Auth::Ntlm { .. } => "ntlm",
        Auth::AwsSigV4 { .. } => "awsv4",
        Auth::Unknown { raw_type, .. } => raw_type,
        // Dialects Bruno has no native block for — carried, but flagged by the round-trip.
        Auth::JwtBearer { .. } => "jwt",
        Auth::Hawk { .. } => "hawk",
        Auth::EdgeGrid { .. } => "edgegrid",
    })
}

/// The `auth:<type> { … }` block for a concrete auth, or `None` for none/inherit/unspecified.
fn auth_block(auth: &Option<Auth>) -> Option<String> {
    let auth = auth.as_ref()?;
    let dict = |tag: &str, pairs: &[(&str, &str)]| {
        let mut s = format!("{tag} {{\n");
        for (k, v) in pairs {
            let _ = writeln!(s, "  {k}: {v}");
        }
        s.push_str("}\n");
        s
    };
    let params_block = |tag: &str, params: &std::collections::BTreeMap<String, String>| {
        let mut s = format!("{tag} {{\n");
        for (k, v) in params {
            let _ = writeln!(s, "  {k}: {v}");
        }
        s.push_str("}\n");
        s
    };
    Some(match auth {
        Auth::None | Auth::Inherit => return None,
        Auth::Basic { username, password } => dict(
            "auth:basic",
            &[("username", username), ("password", password)],
        ),
        Auth::Bearer { token, .. } => dict("auth:bearer", &[("token", token)]),
        Auth::ApiKey {
            key,
            value,
            placement,
        } => {
            let p = match placement {
                cq_model::ApiKeyPlacement::Query => "query",
                cq_model::ApiKeyPlacement::Header => "header",
            };
            dict(
                "auth:apikey",
                &[("key", key), ("value", value), ("placement", p)],
            )
        }
        Auth::OAuth2 { grant, params } => {
            let mut m = params.clone();
            if !grant.is_empty() {
                m.insert("grant_type".into(), grant.clone());
            }
            params_block("auth:oauth2", &m)
        }
        Auth::OAuth1 { params } => params_block("auth:oauth1", params),
        Auth::Digest { params } => params_block("auth:digest", params),
        Auth::Ntlm { params } => params_block("auth:ntlm", params),
        Auth::AwsSigV4 { params } => params_block("auth:awsv4", params),
        Auth::JwtBearer { params, .. } => params_block("auth:jwt", params),
        Auth::Hawk { params } => params_block("auth:hawk", params),
        Auth::EdgeGrid { params } => params_block("auth:edgegrid", params),
        Auth::Unknown { raw_type, raw } => {
            let mut s = format!("auth:{raw_type} {{\n");
            if let Some(obj) = raw.as_object() {
                for (k, v) in obj {
                    let val = v
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| v.to_string());
                    let _ = writeln!(s, "  {k}: {val}");
                }
            }
            s.push_str("}\n");
            s
        }
    })
}

fn bruno_json(name: &str) -> String {
    format!("{{\n  \"version\": \"1\",\n  \"name\": \"{name}\",\n  \"type\": \"collection\"\n}}\n")
}

/// A lowercase, hyphenated slug for filenames (deterministic → byte-stable output).
fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let t = out.trim_matches('-').to_string();
    if t.is_empty() {
        "request".into()
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use cq_report::{Fidelity, Report};

    // Every first-class block in one request. The exporter must be IR-idempotent on it:
    // parse → IR1 → emit → parse → IR2, with IR1 == IR2.
    const FULL: &str = r#"meta {
  name: Create user
  type: http
  seq: 2
}

post {
  url: {{base}}/users/:id
  body: json
  auth: bearer
}

params:query {
  page: 1
}

params:path {
  id: 42
}

headers {
  content-type: application/json
  ~x-debug: 1
}

auth:bearer {
  token: {{token}}
}

body:json {
  {
    "name": "John"
  }
}

vars:pre-request {
  ts: {{$timestamp}}
}

vars:post-response {
  userId: res.body.id
}

assert {
  res.status: eq 201
  ~res.body.ok: eq true
}

script:pre-request {
  bru.setVar("x", 1);
}

script:post-response {
  test("created", () => expect(res.getStatus()).to.eql(201));
}

docs {
  # Create user
  Creates a user.
}
"#;

    fn parse(s: &str) -> cq_model::Request {
        let mut r = Report::new(Fidelity::Lossless);
        crate::bruno::parse_bru_request(s, &mut r).expect("parse")
    }

    #[test]
    fn full_request_is_ir_idempotent() {
        let ir1 = parse(FULL);
        let text = super::emit_request(&ir1);
        let ir2 = parse(&text);
        assert_eq!(
            ir1, ir2,
            "re-emitted .bru did not recover the same IR:\n{text}"
        );
    }

    #[test]
    fn emitted_bru_carries_the_essentials() {
        // Guard against a vacuous idempotence (e.g. both sides empty): assert the text really
        // contains the fields, so the exporter is doing work.
        let ir = parse(FULL);
        let out = super::emit_request(&ir);
        for needle in [
            "meta {",
            "post {",
            "auth: bearer",
            "auth:bearer {",
            "params:path {",
            "body:json {",
            "vars:post-response {",
            "assert {",
            "~res.body.ok: eq true",
            "script:pre-request {",
            "docs {",
        ] {
            assert!(
                out.contains(needle),
                "emitted .bru missing `{needle}`:\n{out}"
            );
        }
    }
}
