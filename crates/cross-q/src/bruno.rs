//! Bruno `.bru` (v2 language) → Idealised Model.
//!
//! Bruno stores a collection as a **directory tree of `.bru` text files**, one per request
//! (plus `collection.bru`, `folder.bru`, `environments/*.bru`). This module parses a single
//! `.bru` **request** file — the core unit. Directory-tree assembly (folders, ordering by
//! `meta.seq`, collection/environment files) is a later step; here we turn one request file
//! into an IR [`Request`].
//!
//! The `.bru` v2 grammar (ref: `@usebruno/lang`, MIT) is a flat list of **blocks**:
//!
//! ```text
//! meta {            ← dictionary block: `key: value` lines
//!   name: Get user
//!   type: http
//!   seq: 1
//! }
//! get {             ← the block *name* is the HTTP method
//!   url: {{base}}/users/:id
//!   body: json      ← which body:* block is active
//!   auth: bearer    ← which auth:* block is active
//! }
//! body:json {       ← text block: content preserved verbatim (outdented 2 spaces)
//!   { "hi": true }
//! }
//! ```
//!
//! Blocks open with `name {` / `name:subtype {` at column 0 and close with `}` at column 0.
//! Inner content is always indented (≥2 spaces), so structural braces inside a JSON body
//! never collide with the block's own closing brace — that column-0 rule is the whole
//! tokenizer.
//!
//! Every block maps to a **first-class** IR field — nothing is shoved into `ext`. The blocks
//! that don't have an obvious HTTP home still have a real cross-category one: `docs` →
//! `description` (same as Postman/Insomnia), `settings` → [`RequestSettings`], `tests` →
//! post-response script (same slot as Postman's `test` event), and Bruno's `vars:*`/`assert`
//! → [`RequestBehavior`] (request-scoped var set/capture and response assertions — concepts
//! Hurl also makes first-class). Growing the IR to fit these is the point of adding Bruno:
//! it forces the superset to be honest.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use cq_model::{
    ApiKeyPlacement, Assertion, Auth, Body, Collection, HttpRequest, Item, KeyValue, Method,
    ModelHeader, PathVar, Protocol, Provenance, RecordMeta, Request, RequestBehavior,
    RequestSettings, Script, ScriptDialect, Scripts, SourceFormat, Url, Variable, Workspace,
};
use cq_report::{Phase, Report};

fn prov(locator: impl Into<String>) -> Provenance {
    Provenance {
        format: SourceFormat::Bruno,
        locator: locator.into(),
    }
}

/// One `name { … }` (or `name:subtype { … }`) block, with its inner lines kept verbatim.
struct Block<'a> {
    name: &'a str,
    subtype: Option<&'a str>,
    /// Lines strictly between the opening `{` line and the closing `}` line, verbatim.
    lines: Vec<&'a str>,
}

impl Block<'_> {
    /// The `name:subtype` tag (or just `name`), for matching.
    fn tag(&self) -> String {
        match self.subtype {
            Some(s) => format!("{}:{}", self.name, s),
            None => self.name.to_string(),
        }
    }

    /// Parse inner lines as a dictionary: `key: value`, one per line; a leading `~` on the
    /// key marks it disabled (Bruno's convention). Blank lines are skipped. Returns entries
    /// in file order as `(key, value, enabled)`.
    fn dict(&self) -> Vec<(String, String, bool)> {
        let mut out = Vec::new();
        for line in &self.lines {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let (enabled, rest) = match trimmed.strip_prefix('~') {
                Some(r) => (false, r),
                None => (true, trimmed),
            };
            // Split on the first ':' — values (URLs, JWTs) may contain further colons.
            if let Some((k, v)) = rest.split_once(':') {
                out.push((k.trim().to_string(), v.trim().to_string(), enabled));
            } else {
                out.push((rest.trim().to_string(), String::new(), enabled));
            }
        }
        out
    }

    /// First value for `key` in a dictionary block (ignoring the enabled flag).
    fn get(&self, key: &str) -> Option<String> {
        self.dict()
            .into_iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, v, _)| v)
    }

    /// Parse inner lines as a verbatim text block (body/script/docs): drop a single leading
    /// and trailing blank line, then outdent every line by 2 spaces (the inverse of Bruno's
    /// `indentString`). Preserves the authored content byte-for-byte otherwise.
    fn text(&self) -> String {
        let mut lines: &[&str] = &self.lines;
        if lines.first().is_some_and(|l| l.trim().is_empty()) {
            lines = &lines[1..];
        }
        if lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines = &lines[..lines.len() - 1];
        }
        lines
            .iter()
            .map(|l| l.strip_prefix("  ").unwrap_or(l))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Split a `.bru` document into its top-level blocks. Any content outside a block (stray
/// lines) is ignored — Bruno files are all-blocks.
fn tokenize(content: &str) -> Vec<Block<'_>> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        // A block header is at column 0 and ends in `{`.
        if let Some(header) = line.strip_suffix('{') {
            let header = header.trim();
            if !header.is_empty() && !line.starts_with(char::is_whitespace) {
                let (name, subtype) = match header.split_once(':') {
                    Some((n, s)) => (n.trim(), Some(s.trim())),
                    None => (header, None),
                };
                // Collect until a closing `}` at column 0.
                let mut inner = Vec::new();
                i += 1;
                while i < lines.len() && lines[i] != "}" {
                    inner.push(lines[i]);
                    i += 1;
                }
                blocks.push(Block {
                    name,
                    subtype,
                    lines: inner,
                });
            }
        }
        i += 1;
    }
    blocks
}

const METHODS: &[&str] = &[
    "get", "post", "put", "delete", "patch", "options", "head", "trace",
];

/// Parse a single `.bru` request file into an IR [`Request`].
pub fn parse_bru_request(content: &str, report: &mut Report) -> Result<Request, String> {
    let blocks = tokenize(content);
    if blocks.is_empty() {
        return Err("bruno: no blocks found (not a .bru file?)".into());
    }

    let meta = blocks.iter().find(|b| b.tag() == "meta");
    let name = meta
        .and_then(|m| m.get("name"))
        .unwrap_or_else(|| "request".into());
    let seq = meta.and_then(|m| m.get("seq"));

    // The verb block's name is the HTTP method; it also declares which body/auth are active.
    let verb = blocks
        .iter()
        .find(|b| b.subtype.is_none() && METHODS.contains(&b.name))
        .ok_or_else(|| "bruno: no HTTP method block (get/post/...) found".to_string())?;

    let method = Method::from(verb.name.to_ascii_uppercase());
    let url = verb.get("url").unwrap_or_default();
    let body_mode = verb.get("body"); // e.g. "json", "none", "multipartForm"
    let auth_mode = verb.get("auth"); // e.g. "bearer", "none", "inherit"

    let mut http = HttpRequest {
        method,
        url: Url::raw(url),
        ..HttpRequest::default()
    };

    // headers
    if let Some(b) = blocks.iter().find(|b| b.tag() == "headers") {
        http.headers = b
            .dict()
            .into_iter()
            .map(|(k, v, enabled)| KeyValue {
                enabled,
                ..KeyValue::new(k, v)
            })
            .collect();
    }

    // query params — `query` or `params:query`
    if let Some(b) = blocks
        .iter()
        .find(|b| b.tag() == "query" || b.tag() == "params:query")
    {
        http.query = b
            .dict()
            .into_iter()
            .map(|(k, v, enabled)| KeyValue {
                enabled,
                ..KeyValue::new(k, v)
            })
            .collect();
    }

    // path params — `params:path`
    if let Some(b) = blocks.iter().find(|b| b.tag() == "params:path") {
        http.path_variables = b
            .dict()
            .into_iter()
            .map(|(k, v, _)| PathVar {
                key: k,
                value: v,
                data_type: Default::default(),
                description: None,
            })
            .collect();
    }

    // body — driven by the verb's declared `body:` mode
    http.body = parse_body(&blocks, body_mode.as_deref(), report);

    // request `settings` (e.g. `encodeUrl`) → first-class RequestSettings
    if let Some(b) = blocks.iter().find(|b| b.tag() == "settings") {
        apply_settings(b, &mut http, report);
    }

    // scripts. Bruno splits post-response into `script:post-response` (setup) and `tests`
    // (assertions-as-JS); both are post-response JS, the same slot Postman's `test` event
    // maps to. Concatenate so neither is lost, `script` first.
    let mut scripts = Scripts::default();
    if let Some(b) = blocks.iter().find(|b| b.tag() == "script:pre-request") {
        scripts.pre_request = Some(bru_script(b.text()));
    }
    let post = ["script:post-response", "tests"]
        .iter()
        .filter_map(|tag| blocks.iter().find(|b| &b.tag() == tag).map(|b| b.text()))
        .collect::<Vec<_>>();
    if !post.is_empty() {
        scripts.post_response = Some(bru_script(post.join("\n\n")));
    }

    // request-scoped var operations + response assertions → first-class RequestBehavior
    let behavior = RequestBehavior {
        pre_request_vars: block_vars(&blocks, "vars:pre-request"),
        post_response_vars: block_vars(&blocks, "vars:post-response"),
        asserts: block_asserts(&blocks),
    };

    // auth — driven by the verb's declared `auth:` mode
    let auth = parse_auth(&blocks, auth_mode.as_deref());

    let mut rec = RecordMeta::new(format!("bru-{}", slug(&name)), name, SourceFormat::Bruno);
    rec.rank = seq;
    // `docs` is markdown documentation — the same concept as a Postman/Insomnia description.
    if let Some(b) = blocks.iter().find(|b| b.tag() == "docs") {
        let docs = b.text();
        if !docs.is_empty() {
            rec.description = Some(docs);
        }
    }

    Ok(Request {
        meta: rec,
        protocol: Protocol::Http(http),
        auth,
        scripts,
        examples: Vec::new(),
        depends_on: Vec::new(),
        behavior,
    })
}

/// Map a Bruno `settings` block onto [`RequestSettings`]. Known keys are promoted; an
/// unrecognised setting is surfaced (not silently swallowed).
fn apply_settings(b: &Block<'_>, http: &mut HttpRequest, report: &mut Report) {
    let mut settings = RequestSettings::default();
    for (k, v, _) in b.dict() {
        let on = v == "true";
        match k.as_str() {
            "encodeUrl" => settings.encode_url = on,
            other => report.dropped(
                Phase::Parse,
                prov(format!("settings.{other}")),
                format!("bruno setting `{other}` has no IR field yet"),
            ),
        }
    }
    http.settings = settings;
}

/// A Bruno `vars:*` block → IR [`Variable`]s (runtime scope), preserving the disabled flag.
fn block_vars(blocks: &[Block<'_>], tag: &str) -> Vec<Variable> {
    blocks
        .iter()
        .find(|b| b.tag() == tag)
        .map(|b| {
            b.dict()
                .into_iter()
                .map(|(key, value, enabled)| Variable {
                    key,
                    value,
                    initial: None,
                    scope: cq_model::Scope::Runtime,
                    data_type: Default::default(),
                    category: Default::default(),
                    enabled,
                    rank: None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The Bruno `assert` block → IR [`Assertion`]s. Each line is `expr: predicate`
/// (e.g. `res.status: eq 200`); a leading `~` disables it.
fn block_asserts(blocks: &[Block<'_>]) -> Vec<Assertion> {
    blocks
        .iter()
        .find(|b| b.tag() == "assert")
        .map(|b| {
            b.dict()
                .into_iter()
                .map(|(expr, predicate, enabled)| Assertion {
                    expr,
                    predicate,
                    enabled,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a `.bru` request file into a single-request [`Workspace`] (mirrors the cURL entry).
/// Directory-tree collections come later; this is the one-file path the converter hooks into.
pub fn parse_bruno(content: &str, report: &mut Report) -> Result<Workspace, String> {
    let request = parse_bru_request(content, report)?;
    let root = Collection {
        meta: RecordMeta::new("bru-root", "", SourceFormat::Bruno),
        items: vec![Item::Request(Box::new(request))],
        ..Collection::default()
    };
    Ok(Workspace {
        meta: RecordMeta::new("bru-workspace", "", SourceFormat::Bruno),
        cross_q: ModelHeader::for_source(SourceFormat::Bruno),
        collections: vec![root],
        environments: Vec::new(),
        packages: Vec::new(),
    })
}

fn bru_script(source: String) -> Script {
    Script {
        source,
        language: Default::default(),
        dialect: ScriptDialect::Bru,
    }
}

fn parse_body(blocks: &[Block<'_>], mode: Option<&str>, report: &mut Report) -> Option<Body> {
    let mode = mode?;
    if mode == "none" {
        return None;
    }
    match mode {
        "json" => raw_body(blocks, "body:json", "application/json"),
        "text" => raw_body(blocks, "body:text", "text/plain"),
        "xml" => raw_body(blocks, "body:xml", "application/xml"),
        "sparql" => raw_body(blocks, "body:sparql", "application/sparql-query"),
        "graphql" => {
            let query = blocks
                .iter()
                .find(|b| b.tag() == "body:graphql")
                .map(|b| b.text())
                .unwrap_or_default();
            let variables = blocks
                .iter()
                .find(|b| b.tag() == "body:graphql:vars")
                .map(|b| b.text())
                .unwrap_or_default();
            Some(Body::Graphql {
                query,
                variables,
                operation_name: None,
            })
        }
        "formUrlEncoded" | "form-urlencoded" => {
            let fields = blocks
                .iter()
                .find(|b| b.tag() == "body:form-urlencoded")
                .map(kv_fields)
                .unwrap_or_default();
            Some(Body::UrlEncoded { fields })
        }
        "multipartForm" | "multipart-form" => {
            let fields = blocks
                .iter()
                .find(|b| b.tag() == "body:multipart-form")
                .map(|b| {
                    kv_fields(b)
                        .into_iter()
                        .map(cq_model::FormField::Text)
                        .collect()
                })
                .unwrap_or_default();
            Some(Body::FormData { fields })
        }
        other => {
            // A body mode we don't model yet (file/grpc/ws). Don't fabricate a body; note it.
            report.dropped(
                Phase::Parse,
                prov(format!("body:{other}")),
                format!("bruno body mode `{other}` not yet modelled; body omitted"),
            );
            None
        }
    }
}

fn raw_body(blocks: &[Block<'_>], tag: &str, media_type: &str) -> Option<Body> {
    blocks.iter().find(|b| b.tag() == tag).map(|b| Body::Raw {
        text: b.text(),
        media_type: media_type.to_string(),
    })
}

fn kv_fields(b: &Block<'_>) -> Vec<KeyValue> {
    b.dict()
        .into_iter()
        .map(|(k, v, enabled)| KeyValue {
            enabled,
            ..KeyValue::new(k, v)
        })
        .collect()
}

fn parse_auth(blocks: &[Block<'_>], mode: Option<&str>) -> Option<Auth> {
    let mode = mode?;
    let block = |tag: &str| blocks.iter().find(|b| b.tag() == tag);
    let params = |tag: &str| -> BTreeMap<String, String> {
        block(tag)
            .map(|b| b.dict().into_iter().map(|(k, v, _)| (k, v)).collect())
            .unwrap_or_default()
    };
    Some(match mode {
        "none" => Auth::None,
        "inherit" => Auth::Inherit,
        "basic" => {
            let b = block("auth:basic");
            Auth::Basic {
                username: b.and_then(|b| b.get("username")).unwrap_or_default(),
                password: b.and_then(|b| b.get("password")).unwrap_or_default(),
            }
        }
        "bearer" => Auth::Bearer {
            token: block("auth:bearer")
                .and_then(|b| b.get("token"))
                .unwrap_or_default(),
            header_prefix: None,
        },
        "apikey" => {
            let b = block("auth:apikey");
            let placement = match b.and_then(|b| b.get("placement")).as_deref() {
                Some("query") => ApiKeyPlacement::Query,
                _ => ApiKeyPlacement::Header,
            };
            Auth::ApiKey {
                key: b.and_then(|b| b.get("key")).unwrap_or_default(),
                value: b.and_then(|b| b.get("value")).unwrap_or_default(),
                placement,
            }
        }
        "oauth2" => {
            let mut p = params("auth:oauth2");
            let grant = p.remove("grant_type").unwrap_or_default();
            Auth::OAuth2 { grant, params: p }
        }
        "oauth1" => Auth::OAuth1 {
            params: params("auth:oauth1"),
        },
        "digest" => Auth::Digest {
            params: params("auth:digest"),
        },
        "ntlm" => Auth::Ntlm {
            params: params("auth:ntlm"),
        },
        "awsv4" => Auth::AwsSigV4 {
            params: params("auth:awsv4"),
        },
        // Types the IR doesn't model (wsse, akamai, …) — keep the credential, don't strip it.
        other => Auth::Unknown {
            raw_type: other.to_string(),
            raw: Value::Object(
                params(&format!("auth:{other}"))
                    .into_iter()
                    .map(|(k, v)| (k, json!(v)))
                    .collect(),
            ),
        },
    })
}

/// A lowercase, hyphenated slug for deterministic ids (no randomness → byte-stable output).
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
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "request".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_report::{Fidelity, Report};

    fn parse(content: &str) -> Request {
        let mut report = Report::new(Fidelity::Lossless);
        parse_bru_request(content, &mut report).expect("parse .bru")
    }

    // A canonical v2 .bru exercising every block that has a first-class IR home. The point:
    // nothing lands in ext — each block maps to a real field.
    const FULL: &str = r#"meta {
  name: Create user
  type: http
  seq: 3
}

post {
  url: {{base}}/users/:id
  body: json
  auth: bearer
}

headers {
  content-type: application/json
  ~x-debug: 1
}

params:query {
  page: 1
}

params:path {
  id: 42
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
  ~res.body.name: eq John
}

script:pre-request {
  bru.setVar("x", 1);
}

tests {
  test("created", () => expect(res.getStatus()).to.eql(201));
}

docs {
  # Create user
  Creates a user.
}

settings {
  encodeUrl: false
}
"#;

    #[test]
    fn maps_every_block_to_first_class_no_ext() {
        let req = parse(FULL);

        // meta
        assert_eq!(req.meta.name, "Create user");
        assert_eq!(req.meta.rank.as_deref(), Some("3"));
        // docs → description (NOT ext)
        assert!(req
            .meta
            .description
            .as_deref()
            .unwrap()
            .contains("Creates a user."));
        // the whole point of "again ext!": nothing idiosyncratic was dumped
        assert!(
            req.meta.ext.is_empty(),
            "ext must be empty — every block has a home"
        );

        let Protocol::Http(http) = &req.protocol else {
            panic!("expected http");
        };
        assert_eq!(String::from(http.method.clone()), "POST");
        assert_eq!(http.url.raw, "{{base}}/users/:id");

        // headers incl. the `~`-disabled one
        assert_eq!(http.headers.len(), 2);
        assert_eq!(http.headers[0].key, "content-type");
        assert!(http.headers[0].enabled);
        assert_eq!(http.headers[1].key, "x-debug");
        assert!(!http.headers[1].enabled);

        // query + path vars
        assert_eq!(http.query[0].key, "page");
        assert_eq!(http.path_variables[0].key, "id");
        assert_eq!(http.path_variables[0].value, "42");

        // body verbatim (outdented)
        match http.body.as_ref().unwrap() {
            Body::Raw { text, media_type } => {
                assert_eq!(media_type, "application/json");
                assert_eq!(text, "{\n  \"name\": \"John\"\n}");
            }
            other => panic!("expected raw json body, got {other:?}"),
        }

        // settings → encode_url
        assert!(!http.settings.encode_url);

        // auth: verb declared `bearer`, so Bearer wins (basic/apikey blocks absent here)
        match req.auth.as_ref().unwrap() {
            Auth::Bearer { token, .. } => assert_eq!(token, "{{token}}"),
            other => panic!("expected bearer, got {other:?}"),
        }

        // vars → RequestBehavior (first-class, not ext)
        assert_eq!(req.behavior.pre_request_vars[0].key, "ts");
        assert_eq!(req.behavior.post_response_vars[0].key, "userId");
        assert_eq!(req.behavior.post_response_vars[0].value, "res.body.id");

        // asserts → RequestBehavior incl. the disabled one
        assert_eq!(req.behavior.asserts.len(), 2);
        assert_eq!(req.behavior.asserts[0].expr, "res.status");
        assert_eq!(req.behavior.asserts[0].predicate, "eq 201");
        assert!(req.behavior.asserts[0].enabled);
        assert!(!req.behavior.asserts[1].enabled);

        // scripts: pre-request, and post-response = script:post-response + tests concatenated
        assert!(req
            .scripts
            .pre_request
            .as_ref()
            .unwrap()
            .source
            .contains("bru.setVar"));
        let post = &req.scripts.post_response.as_ref().unwrap().source;
        assert!(post.contains("test(\"created\""));
    }

    #[test]
    fn auth_selected_by_verb_declaration() {
        // Only the auth the verb names is active, even when several auth blocks are present.
        let bru = r#"meta {
  name: X
  type: http
}
get {
  url: https://x.test
  auth: basic
}
auth:basic {
  username: admin
  password: secret
}
auth:bearer {
  token: unused
}
"#;
        let req = parse(bru);
        match req.auth.as_ref().unwrap() {
            Auth::Basic { username, password } => {
                assert_eq!(username, "admin");
                assert_eq!(password, "secret");
            }
            other => panic!("expected basic, got {other:?}"),
        }
    }

    #[test]
    fn json_braces_do_not_confuse_block_close() {
        // The column-0 `}` rule: nested JSON braces (indented) never end the block early.
        let bru = r#"meta {
  name: Nested
  type: http
}
post {
  url: https://x.test
  body: json
}
body:json {
  {
    "a": { "b": [1, 2, { "c": 3 }] }
  }
}
"#;
        let req = parse(bru);
        let Protocol::Http(http) = &req.protocol else {
            panic!()
        };
        match http.body.as_ref().unwrap() {
            Body::Raw { text, .. } => {
                assert!(text.contains("\"c\": 3"));
                assert!(text.starts_with('{') && text.trim_end().ends_with('}'));
            }
            _ => panic!("expected raw body"),
        }
    }
}
