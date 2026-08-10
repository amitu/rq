//! Bruno `.bru` (v2 language) → Idealised Model.
//!
//! Bruno stores a collection as a **directory tree of `.bru` text files**, one per request
//! (plus `collection.bru`, `folder.bru`, `environments/*.bru`). This module handles both the
//! single `.bru` **request** file (the core unit) and the **whole directory**: the host
//! passes the tree as a virtual-FS map (`{ path: contents }`) — WASM has no filesystem, so
//! the directory walk lives in the host (browser/node) or the native CLI, and the map is one
//! string across the boundary. [`parse_bruno`] auto-detects map vs single file; the tree is
//! assembled by [`parse_bruno_collection`] (folders from directories, order by `meta.seq`,
//! `collection.bru`/`folder.bru` as inherited config, `environments/*.bru` as environments).
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

/// One dictionary entry, with the annotations Bruno can attach: a `~` disable flag, an
/// `@number`/`@boolean`/`@object`/`@string` type tag, an `@description(...)`, and (in env
/// files) a secret flag.
#[derive(Default)]
struct DictEntry {
    key: String,
    value: String,
    enabled: bool,
    /// Bruno type annotation: "number" | "boolean" | "object" | "string" (empty = string).
    data_type: String,
    description: Option<String>,
    secret: bool,
}

/// One `name { … }` / `name:subtype { … }` (dict/text) or `name:subtype [ … ]` (list) block.
struct Block<'a> {
    name: &'a str,
    subtype: Option<&'a str>,
    /// `[ … ]` list block (e.g. `vars:secret [ a, b ]`) rather than `{ … }`.
    is_list: bool,
    /// Lines strictly between the opening and closing delimiter lines, verbatim.
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

    /// Parse the block as a dictionary, honouring `~` (disabled), `@type`, and
    /// `@description(...)` / `'''…'''` multiline values. Entries are returned in file order.
    fn entries(&self) -> Vec<DictEntry> {
        let mut out = Vec::new();
        let mut pending_type = String::new();
        let mut pending_desc: Option<String> = None;
        let mut i = 0;
        while i < self.lines.len() {
            let raw = self.lines[i];
            let t = raw.trim();
            i += 1;
            if t.is_empty() {
                continue;
            }
            // `@description('''…''')` (may span lines) → attach to the next entry.
            if let Some(rest) = t.strip_prefix("@description(") {
                let (desc, consumed) = read_annotation(rest, &self.lines, i);
                pending_desc = Some(desc);
                i = consumed;
                continue;
            }
            // `@number` / `@boolean` / `@object` / `@string` type tag on the next entry.
            if let Some(ty) = t.strip_prefix('@') {
                pending_type = ty.to_string();
                continue;
            }
            let (enabled, rest) = match t.strip_prefix('~') {
                Some(r) => (false, r.trim_start()),
                None => (true, t),
            };
            let (key, valpart) = match rest.split_once(':') {
                Some((k, v)) => (k.trim().to_string(), v.trim()),
                None => (rest.trim().to_string(), ""),
            };
            // A `'''` opens a multiline value that runs until the next line that is `'''`.
            let value = if valpart == "'''" {
                let mut buf = Vec::new();
                while i < self.lines.len() && self.lines[i].trim() != "'''" {
                    buf.push(self.lines[i]);
                    i += 1;
                }
                i += 1; // consume the closing '''
                dedent(&buf)
            } else {
                strip_triple(valpart).to_string()
            };
            out.push(DictEntry {
                key,
                value,
                enabled,
                data_type: std::mem::take(&mut pending_type),
                description: pending_desc.take(),
                secret: false,
            });
        }
        out
    }

    /// Simple `(key, value, enabled)` view for blocks without annotations (auth params, …).
    fn dict(&self) -> Vec<(String, String, bool)> {
        self.entries()
            .into_iter()
            .map(|e| (e.key, e.value, e.enabled))
            .collect()
    }

    /// First value for `key` in a dictionary block (ignoring the enabled flag).
    fn get(&self, key: &str) -> Option<String> {
        self.entries()
            .into_iter()
            .find(|e| e.key == key)
            .map(|e| e.value)
    }

    /// Items of a `[ … ]` list block (comma- and/or newline-separated), e.g. `vars:secret`.
    fn list_items(&self) -> Vec<String> {
        self.lines
            .iter()
            .flat_map(|l| l.split(','))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
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

/// Dedent a multiline block by the smallest common leading-whitespace of its non-blank
/// lines (Bruno stores `'''…'''` dict values outdented to their own base indentation).
fn dedent(lines: &[&str]) -> String {
    let min = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            if l.len() >= min {
                &l[min..]
            } else {
                l.trim_start()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Strip surrounding `'''…'''` (single-line multiline) or `'…'` quoting from a dict value.
fn strip_triple(s: &str) -> &str {
    if let Some(inner) = s.strip_prefix("'''").and_then(|x| x.strip_suffix("'''")) {
        inner
    } else {
        s
    }
}

/// Read a `@description(…)` annotation body starting at `first` (the text after the `(`),
/// continuing across `lines` from index `i` until the closing `)`. Returns the description
/// text (triple/single quotes stripped, outdented) and the next line index to resume from.
fn read_annotation(first: &str, lines: &[&str], mut i: usize) -> (String, usize) {
    // Single-line: `@description('''text''')` or `@description('text')`.
    if let Some(body) = first.strip_suffix(')') {
        return (unquote(body).to_string(), i);
    }
    // Multiline: collect until a line whose trim ends with `''')` or `)`.
    let mut buf = Vec::new();
    while i < lines.len() {
        let l = lines[i];
        i += 1;
        let t = l.trim();
        if t == "''')" || t == ")" {
            break;
        }
        buf.push(l.strip_prefix("  ").unwrap_or(l));
    }
    (buf.join("\n"), i)
}

/// Strip `'''…'''` or `'…'` quoting from an annotation body.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("'''").and_then(|x| x.strip_suffix("'''")) {
        return inner;
    }
    if let Some(inner) = s.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')) {
        return inner;
    }
    s
}

/// Split a `.bru` document into its top-level blocks. A block header is at column 0 and ends
/// in `{` (dict/text) or `[` (list); it closes with the matching delimiter at column 0.
/// Content outside a block is ignored — Bruno files are all-blocks.
fn tokenize(content: &str) -> Vec<Block<'_>> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let opener = line
            .strip_suffix('{')
            .map(|h| (h, false, "}"))
            .or_else(|| line.strip_suffix('[').map(|h| (h, true, "]")));
        if let Some((header, is_list, close)) = opener {
            let header = header.trim();
            if !header.is_empty() && !line.starts_with(char::is_whitespace) {
                let (name, subtype) = match header.split_once(':') {
                    Some((n, s)) => (n.trim(), Some(s.trim())),
                    None => (header, None),
                };
                let mut inner = Vec::new();
                i += 1;
                while i < lines.len() && lines[i] != close {
                    inner.push(lines[i]);
                    i += 1;
                }
                blocks.push(Block {
                    name,
                    subtype,
                    is_list,
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

    // headers (may carry `@description(...)` annotations)
    if let Some(b) = blocks.iter().find(|b| b.tag() == "headers") {
        http.headers = entries_to_kvs(b);
    }

    // query params — `query` or `params:query`
    if let Some(b) = blocks
        .iter()
        .find(|b| b.tag() == "query" || b.tag() == "params:query")
    {
        http.query = entries_to_kvs(b);
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
        pre_request_vars: block_vars(&blocks, "vars:pre-request", cq_model::Scope::Runtime),
        post_response_vars: block_vars(&blocks, "vars:post-response", cq_model::Scope::Runtime),
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

/// A Bruno `vars:*` block → IR [`Variable`]s, preserving disabled flag and `@type` tag.
fn block_vars(blocks: &[Block<'_>], tag: &str, scope: cq_model::Scope) -> Vec<Variable> {
    blocks
        .iter()
        .find(|b| b.tag() == tag)
        .map(|b| {
            b.entries()
                .into_iter()
                .map(|e| var_from(e, scope))
                .collect()
        })
        .unwrap_or_default()
}

/// Build an IR [`Variable`] from a dict entry at a given scope, mapping Bruno's `@type` tag
/// and secret flag onto the IR's [`VarType`].
fn var_from(e: DictEntry, scope: cq_model::Scope) -> Variable {
    let data_type = if e.secret {
        cq_model::VarType::Secret
    } else {
        match e.data_type.as_str() {
            "number" => cq_model::VarType::Number,
            "boolean" => cq_model::VarType::Boolean,
            _ => cq_model::VarType::String,
        }
    };
    Variable {
        key: e.key,
        value: e.value,
        initial: None,
        scope,
        data_type,
        category: Default::default(),
        enabled: e.enabled,
        rank: None,
    }
}

/// A dict block → IR [`KeyValue`]s, carrying the disabled flag, `@description`, and (secret
/// entries) the `Secret` kind.
fn entries_to_kvs(b: &Block<'_>) -> Vec<KeyValue> {
    b.entries()
        .into_iter()
        .map(|e| KeyValue {
            enabled: e.enabled,
            description: e.description,
            kind: if e.secret {
                cq_model::KvKind::Secret
            } else {
                cq_model::KvKind::Text
            },
            ..KeyValue::new(e.key, e.value)
        })
        .collect()
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

/// Parse Bruno input into a [`Workspace`]. `content` is either:
///
/// - a **virtual-FS map** — a JSON object `{ "path": "file-contents", … }` describing a whole
///   collection directory (WASM has no filesystem, so the host walks the dir and passes the
///   tree as one string; the native CLI does the same walk); or
/// - a **single `.bru`** request file (raw text).
///
/// A `.bru` file is never valid JSON, so `content` that deserialises as a `{string: string}`
/// object is unambiguously the directory map; anything else is a single request.
pub fn parse_bruno(content: &str, report: &mut Report) -> Result<Workspace, String> {
    if let Ok(files) = serde_json::from_str::<BTreeMap<String, String>>(content) {
        if files
            .keys()
            .any(|k| k.ends_with(".bru") || k.ends_with("bruno.json"))
        {
            return parse_bruno_collection(&files, report);
        }
    }
    // Single request file.
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

/// Assemble a Bruno collection directory (a virtual-FS map) into a [`Workspace`]. The
/// directory *is* the tree: a dir with a `folder.bru` is a folder; every other `*.bru` is a
/// request; `bruno.json`/`collection.bru` describe the root; `environments/*.bru` are
/// environments. Siblings are ordered by their `meta.seq`.
pub fn parse_bruno_collection(
    files: &BTreeMap<String, String>,
    report: &mut Report,
) -> Result<Workspace, String> {
    // Root name from bruno.json.
    let name = files
        .get("bruno.json")
        .and_then(|c| serde_json::from_str::<Value>(c).ok())
        .and_then(|j| j.get("name").and_then(|n| n.as_str()).map(String::from))
        .unwrap_or_else(|| "bruno-collection".into());

    // Root collection carries collection.bru's inherited auth/headers/scripts/vars/docs.
    let mut root = Collection {
        meta: RecordMeta::new(
            format!("bru-{}", slug(&name)),
            name.clone(),
            SourceFormat::Bruno,
        ),
        ..Collection::default()
    };
    if let Some(c) = files.get("collection.bru") {
        let blocks = tokenize(c);
        apply_container(&blocks, &mut root);
        root.variables = collection_vars(&blocks);
    }

    // Environments (environments/*.bru).
    let mut environments = Vec::new();
    for (path, content) in files {
        if let Some(file) = path.strip_prefix("environments/") {
            if let Some(env_name) = file.strip_suffix(".bru") {
                environments.push(parse_environment(env_name, content));
            }
        }
    }

    // Build the folder tree from directory structure.
    root.items = build_tree(files, "", report);

    Ok(Workspace {
        meta: RecordMeta::new("bru-workspace", name, SourceFormat::Bruno),
        cross_q: ModelHeader::for_source(SourceFormat::Bruno),
        collections: vec![root],
        environments,
        packages: Vec::new(),
    })
}

/// Immediate children of directory `dir` (""=root), as ordered [`Item`]s. A child directory
/// with a `folder.bru` becomes a nested collection; a `*.bru` file becomes a request.
fn build_tree(files: &BTreeMap<String, String>, dir: &str, report: &mut Report) -> Vec<Item> {
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    let mut items: Vec<(Option<i64>, String, Item)> = Vec::new();
    let mut seen_dirs: std::collections::BTreeSet<String> = Default::default();

    for (path, content) in files {
        let Some(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        if rel.is_empty() || (dir.is_empty() && rel.starts_with("environments/")) {
            continue;
        }
        match rel.split_once('/') {
            // A nested directory: recurse once per unique child dir that has a folder.bru.
            Some((child, _)) => {
                let child_dir = format!("{prefix}{child}");
                if !seen_dirs.insert(child_dir.clone()) {
                    continue;
                }
                let folder_path = format!("{child_dir}/folder.bru");
                let (fname, seq) = match files.get(&folder_path) {
                    Some(fb) => folder_meta(fb).unwrap_or_else(|| (child.to_string(), None)),
                    None => (child.to_string(), None),
                };
                let mut folder = Collection {
                    meta: RecordMeta::new(
                        format!("bru-{}", slug(&child_dir)),
                        fname,
                        SourceFormat::Bruno,
                    ),
                    ..Collection::default()
                };
                if let Some(fb) = files.get(&folder_path) {
                    apply_container(&tokenize(fb), &mut folder);
                }
                folder.items = build_tree(files, &child_dir, report);
                items.push((seq, child.to_string(), Item::Collection(Box::new(folder))));
            }
            // A file directly in this dir.
            None => {
                if rel == "folder.bru" || (dir.is_empty() && rel == "collection.bru") {
                    continue;
                }
                if !rel.ends_with(".bru") {
                    continue;
                }
                match parse_bru_request(content, report) {
                    Ok(req) => {
                        let seq = req.meta.rank.as_deref().and_then(|s| s.parse::<i64>().ok());
                        items.push((seq, rel.to_string(), Item::Request(Box::new(req))));
                    }
                    Err(e) => report.dropped(Phase::Parse, prov(path.clone()), e),
                }
            }
        }
    }

    // Order by meta.seq (present first, ascending), then by name for stability.
    items.sort_by(|a, b| match (a.0, b.0) {
        (Some(x), Some(y)) => x.cmp(&y).then(a.1.cmp(&b.1)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.1.cmp(&b.1),
    });
    items.into_iter().map(|(_, _, item)| item).collect()
}

/// `(name, seq)` from a `folder.bru`'s `meta` block.
fn folder_meta(content: &str) -> Option<(String, Option<i64>)> {
    let blocks = tokenize(content);
    let meta = blocks.iter().find(|b| b.tag() == "meta")?;
    let name = meta.get("name")?;
    let seq = meta.get("seq").and_then(|s| s.parse::<i64>().ok());
    Some((name, seq))
}

/// Apply a container file's (`collection.bru`/`folder.bru`) shared, inheritable blocks —
/// `headers`, `auth { mode }` + `auth:<mode>`, `script:*`/`tests`, and `docs` — onto a
/// [`Collection`]. Its variables are filled separately by [`collection_vars`].
fn apply_container(blocks: &[Block<'_>], coll: &mut Collection) {
    if let Some(b) = blocks.iter().find(|b| b.tag() == "headers") {
        coll.headers = entries_to_kvs(b);
    }
    // Collection/folder auth is declared as `auth { mode: <type> }`, then `auth:<type> { … }`.
    if let Some(mode) = blocks
        .iter()
        .find(|b| b.tag() == "auth")
        .and_then(|b| b.get("mode"))
    {
        coll.auth = parse_auth(blocks, Some(&mode));
    }
    if let Some(b) = blocks.iter().find(|b| b.tag() == "script:pre-request") {
        coll.scripts.pre_request = Some(bru_script(b.text()));
    }
    let post = ["script:post-response", "tests"]
        .iter()
        .filter_map(|tag| blocks.iter().find(|b| &b.tag() == tag).map(|b| b.text()))
        .collect::<Vec<_>>();
    if !post.is_empty() {
        coll.scripts.post_response = Some(bru_script(post.join("\n\n")));
    }
    if let Some(b) = blocks.iter().find(|b| b.tag() == "docs") {
        let docs = b.text();
        if !docs.is_empty() {
            coll.meta.description = Some(docs);
        }
    }
}

/// Collection-level variables (`collection.bru`'s `vars:pre-request`/`vars:post-response`).
fn collection_vars(blocks: &[Block<'_>]) -> Vec<Variable> {
    let mut vars = block_vars(blocks, "vars:pre-request", cq_model::Scope::Collection);
    vars.extend(block_vars(
        blocks,
        "vars:post-response",
        cq_model::Scope::Collection,
    ));
    vars
}

/// Parse an `environments/<name>.bru` file into an [`Environment`]. Its `vars { … }` block
/// holds the values (with `@type` tags and `'''` multiline objects); `vars:secret [ … ]`
/// lists names whose values live outside the file — kept as empty secret variables.
fn parse_environment(name: &str, content: &str) -> cq_model::Environment {
    let blocks = tokenize(content);
    let mut variables = Vec::new();
    if let Some(b) = blocks.iter().find(|b| b.tag() == "vars" && !b.is_list) {
        variables.extend(
            b.entries()
                .into_iter()
                .map(|e| var_from(e, cq_model::Scope::Environment)),
        );
    }
    if let Some(b) = blocks
        .iter()
        .find(|b| b.tag() == "vars:secret" && b.is_list)
    {
        for key in b.list_items() {
            variables.push(Variable {
                key,
                value: String::new(),
                initial: None,
                scope: cq_model::Scope::Environment,
                data_type: cq_model::VarType::Secret,
                category: cq_model::VarCategory::Vault,
                enabled: true,
                rank: None,
            });
        }
    }
    cq_model::Environment {
        meta: RecordMeta::new(format!("bru-env-{}", slug(name)), name, SourceFormat::Bruno),
        is_global: false,
        variables,
    }
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

    fn parse_dir(files: &[(&str, &str)]) -> Workspace {
        let map: BTreeMap<String, String> = files
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let mut report = Report::new(Fidelity::Lossless);
        parse_bruno_collection(&map, &mut report).expect("parse dir")
    }

    #[test]
    fn directory_assembles_tree_ordered_by_seq_with_inheritance() {
        // A folder with two requests (seq out of order) + collection-level headers/auth.
        let files = &[
            ("bruno.json", r#"{"name":"My API","type":"collection"}"#),
            (
                "collection.bru",
                "headers {\n  x-team: platform\n}\n\nauth {\n  mode: bearer\n}\n\nauth:bearer {\n  token: {{t}}\n}\n",
            ),
            (
                "users/folder.bru",
                "meta {\n  name: Users\n  seq: 1\n}\n",
            ),
            (
                "users/second.bru",
                "meta {\n  name: Second\n  type: http\n  seq: 2\n}\nget {\n  url: https://x.test/2\n}\n",
            ),
            (
                "users/first.bru",
                "meta {\n  name: First\n  type: http\n  seq: 1\n}\nget {\n  url: https://x.test/1\n}\n",
            ),
        ];
        let ws = parse_dir(files);

        assert_eq!(ws.meta.name, "My API");
        let root = &ws.collections[0];
        // collection-level headers + auth are first-class on the root collection
        assert_eq!(root.headers[0].key, "x-team");
        assert!(matches!(root.auth, Some(Auth::Bearer { .. })));

        // one folder, two requests, ordered by seq
        let Item::Collection(users) = &root.items[0] else {
            panic!("expected a folder");
        };
        assert_eq!(users.meta.name, "Users");
        let names: Vec<&str> = users
            .items
            .iter()
            .map(|i| match i {
                Item::Request(r) => r.meta.name.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(
            names,
            vec!["First", "Second"],
            "requests ordered by meta.seq"
        );
    }

    #[test]
    fn environment_parses_typed_multiline_and_secret_vars() {
        use cq_model::VarType;
        let env = r#"vars {
  host: http://localhost
  @number
  count: 42
  @boolean
  live: true
  @object
  cfg: '''
    {"scope":"env"}
  '''
}
vars:secret [
  api_key,
  token
]
"#;
        let e = super::parse_environment("Local", env);
        let get = |k: &str| e.variables.iter().find(|v| v.key == k).unwrap();
        assert_eq!(get("host").data_type, VarType::String);
        assert_eq!(get("count").data_type, VarType::Number);
        assert_eq!(get("live").data_type, VarType::Boolean);
        // multiline object value preserved (outdented)
        assert_eq!(get("cfg").value, "{\"scope\":\"env\"}");
        // secret-list names → empty secret variables, value never in the file
        let secret = get("api_key");
        assert_eq!(secret.data_type, VarType::Secret);
        assert!(secret.value.is_empty());
    }

    #[test]
    fn header_description_annotation_is_first_class() {
        let bru = "meta {\n  name: X\n  type: http\n}\nget {\n  url: https://x.test\n}\nheaders {\n  @description('''auth token''')\n  authorization: {{t}}\n}\n";
        let req = parse(bru);
        let Protocol::Http(http) = &req.protocol else {
            panic!()
        };
        assert_eq!(http.headers[0].key, "authorization");
        assert_eq!(http.headers[0].description.as_deref(), Some("auth token"));
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
