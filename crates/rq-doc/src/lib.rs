//! The `rq` request document — one Markdown file per request.
//!
//! A request is a single hand-editable file: YAML frontmatter for the structured bits,
//! then named sections for everything that is prose or code.
//!
//! ```text
//! ---
//! method: GET
//! url: https://api.github.com/repos/{{owner}}/{{repo}}/issues
//! headers:
//!   Accept: application/vnd.github+json
//! query:
//!   state: open
//! vars:
//!   owner: { default: anthropics, prompt: "Repository owner" }
//! parents: []
//! ---
//!
//! -- description --
//! What this is, for your future self.
//!
//! -- view --
//! # {{ response | length }} open issues
//!
//! -- post --
//! rq.test('200 OK', () => rq.response.status === 200);
//! ```
//!
//! Two rules govern the reader, both inherited from the import engine's reliability
//! thesis (`docs/cross-q.md`):
//!
//! - **Parse tolerantly, coerce visibly.** A numeric header value becomes a string and
//!   says so; only genuinely ambiguous input (a mapping where a scalar belongs) is fatal.
//! - **Never drop silently.** Frontmatter keys and sections this build doesn't understand
//!   are preserved verbatim and re-emitted on write, so a newer file edited by an older
//!   `rq` comes back whole.

pub mod layout;

use std::fmt::Write as _;

/// Re-exported because `AuthSpec::Other` carries a YAML mapping verbatim — a caller that
/// builds or reads one needs these types, and should not have to pin the same YAML crate.
pub use serde_norway::{Mapping, Value};

/// A parsed request (or collection) document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub front: Front,
    /// Sections in file order, verbatim. Unknown names are carried, not dropped.
    pub sections: Vec<Section>,
}

/// One `-- name --` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub body: String,
}

/// The known section names, in the order `write` emits them.
pub const KNOWN_SECTIONS: &[&str] = &["description", "view", "body", "pre", "post", "form"];

/// The structured half of the document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Front {
    pub method: Option<String>,
    pub url: Option<String>,
    pub headers: StrMap,
    pub query: StrMap,
    pub path_vars: StrMap,
    pub vars: Vec<(String, VarSpec)>,
    /// `application/x-www-form-urlencoded` fields.
    pub form: Option<StrMap>,
    /// `multipart/form-data` fields. A value of `@path/to/file` is a file part.
    pub form_data: Option<StrMap>,
    /// A binary body read from this path.
    pub file: Option<String>,
    /// Media type for the `-- body --` section. Absent = infer from `Content-Type`.
    pub body_type: Option<String>,
    pub auth: Option<AuthSpec>,
    /// Declared dependencies — the requests that must run first (§ chaining).
    pub parents: Vec<String>,
    /// Declarative extraction from this request's response into variables the dependents
    /// read: `token: response.body.access_token`. The zero-JS half of chaining.
    pub capture: StrMap,
    pub timeout: Option<u64>,
    pub follow_redirects: Option<bool>,
    pub verify_tls: Option<bool>,
    /// Frontmatter keys this build doesn't know. Preserved verbatim on write.
    pub extra: Mapping,
}

/// A declared input to the request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VarSpec {
    pub default: Option<String>,
    pub prompt: Option<String>,
    /// Read from this process environment variable when set.
    pub env: Option<String>,
    /// Never echoed when prompted, masked in `--show request`.
    pub secret: bool,
    /// A run fails rather than sending an empty value.
    pub required: bool,
    pub description: Option<String>,
}

/// Request or collection auth. Kinds this build can't send are still parsed and preserved
/// — an unknown credential is reported, never stripped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthSpec {
    None,
    Inherit,
    Basic {
        username: String,
        password: String,
    },
    Bearer {
        token: String,
        prefix: Option<String>,
    },
    ApiKey {
        key: String,
        value: String,
        in_query: bool,
    },
    Other {
        kind: String,
        raw: Mapping,
    },
}

/// An ordered string map. Ordered because a request's headers are read by humans in the
/// order they wrote them, and because a stable order makes the file diff cleanly.
pub type StrMap = Vec<(String, String)>;

/// A non-fatal parse note. Same contract as the converter's diagnostics: if `rq` made a
/// decision about your file, the decision is on the record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note(pub String);

impl std::fmt::Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------

impl Document {
    /// Parse a document, collecting non-fatal notes. Fatal errors are malformed YAML, a
    /// missing frontmatter fence, or a value whose shape can't be coerced.
    pub fn parse(text: &str) -> Result<(Document, Vec<Note>), String> {
        let mut notes = Vec::new();
        let (front_text, rest) = split_frontmatter(text)?;

        let map: Mapping = if front_text.trim().is_empty() {
            Mapping::new()
        } else {
            match serde_norway::from_str::<Value>(front_text) {
                Ok(Value::Mapping(m)) => m,
                Ok(Value::Null) => Mapping::new(),
                Ok(_) => return Err("frontmatter must be a mapping of keys to values".into()),
                Err(e) => return Err(format!("frontmatter is not valid YAML: {e}")),
            }
        };

        let front = Front::from_mapping(map, &mut notes)?;
        let sections = parse_sections(rest, &mut notes);
        Ok((Document { front, sections }, notes))
    }

    /// The request in a few words: the first non-empty line of `-- description --`.
    ///
    /// One definition, used by every place that has to name a request to a person — the
    /// project list, the console, a form's title — so they cannot describe the same request
    /// differently.
    pub fn summary(&self) -> Option<String> {
        self.section("description")?
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            // A heading is still a summary; the `#` is markup, not part of the words.
            .map(|line| line.trim_start_matches('#').trim().to_string())
            .filter(|line| !line.is_empty())
    }

    pub fn section(&self, name: &str) -> Option<&str> {
        self.sections
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.body.as_str())
    }

    pub fn set_section(&mut self, name: &str, body: impl Into<String>) {
        let body = body.into();
        match self.sections.iter_mut().find(|s| s.name == name) {
            Some(s) => s.body = body,
            None => self.sections.push(Section {
                name: name.to_string(),
                body,
            }),
        }
    }

    /// Serialize back to the on-disk form. Known sections come first in `KNOWN_SECTIONS`
    /// order; unknown ones follow in the order they were read.
    pub fn write(&self) -> String {
        let mut out = String::from("---\n");
        let map = self.front.to_mapping();
        if !map.is_empty() {
            let yaml = serde_norway::to_string(&Value::Mapping(map))
                .unwrap_or_else(|e| format!("# frontmatter could not be serialized: {e}\n"));
            out.push_str(&yaml);
        }
        out.push_str("---\n");

        let mut ordered: Vec<&Section> = Vec::new();
        for known in KNOWN_SECTIONS {
            if let Some(s) = self.sections.iter().find(|s| s.name == *known) {
                ordered.push(s);
            }
        }
        for s in &self.sections {
            if !KNOWN_SECTIONS.contains(&s.name.as_str()) {
                ordered.push(s);
            }
        }

        for s in ordered {
            let _ = write!(out, "\n-- {} --\n\n{}\n", s.name, s.body.trim_end());
        }
        out
    }
}

/// Split `---\n…\n---\n` off the front. The fence is required: a request file without
/// frontmatter has no URL, and guessing one from prose is exactly the kind of magic this
/// format refuses.
fn split_frontmatter(text: &str) -> Result<(&str, &str), String> {
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let body = body.trim_start_matches([' ', '\t', '\n', '\r']);
    let after = body
        .strip_prefix("---\n")
        .or_else(|| body.strip_prefix("---\r\n"))
        .ok_or_else(|| {
            "missing frontmatter: the file must start with a `---` line (see docs/RQ-FORMAT.md)"
                .to_string()
        })?;

    let mut offset = 0usize;
    for line in after.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.trim_end() == "---" {
            return Ok((&after[..offset], &after[offset + line.len()..]));
        }
        offset += line.len();
    }
    Err("unterminated frontmatter: no closing `---` line".into())
}

/// Read `-- name --` blocks. Text before the first marker is tolerated and reported —
/// people paste prose above the sections, and losing it would be worse than a note.
fn parse_sections(text: &str, notes: &mut Vec<Note>) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut current: Option<(String, String)> = None;
    let mut preamble = String::new();

    for line in text.lines() {
        match section_marker(line) {
            Some(name) => {
                if let Some((n, b)) = current.take() {
                    sections.push(Section {
                        name: n,
                        body: trim_block(&b),
                    });
                }
                current = Some((name, String::new()));
            }
            None => match current.as_mut() {
                Some((_, body)) => {
                    body.push_str(line);
                    body.push('\n');
                }
                None => {
                    preamble.push_str(line);
                    preamble.push('\n');
                }
            },
        }
    }
    if let Some((n, b)) = current {
        sections.push(Section {
            name: n,
            body: trim_block(&b),
        });
    }

    if !preamble.trim().is_empty() {
        notes.push(Note(
            "text before the first `-- section --` was read as the description".into(),
        ));
        let body = trim_block(&preamble);
        match sections.iter_mut().find(|s| s.name == "description") {
            Some(existing) => existing.body = format!("{body}\n\n{}", existing.body),
            None => sections.insert(
                0,
                Section {
                    name: "description".into(),
                    body,
                },
            ),
        }
    }

    let mut seen: Vec<String> = Vec::new();
    sections.retain(|s| {
        if seen.contains(&s.name) {
            notes.push(Note(format!(
                "duplicate `-- {} --` section; the later one was dropped",
                s.name
            )));
            false
        } else {
            seen.push(s.name.clone());
            true
        }
    });
    sections
}

/// `-- name --` on a line of its own. Deliberately strict so a markdown rule (`---`) or an
/// em-dash sentence is never mistaken for a section boundary.
fn section_marker(line: &str) -> Option<String> {
    let t = line.trim();
    let inner = t.strip_prefix("--")?.strip_suffix("--")?.trim();
    if inner.is_empty()
        || !inner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(inner.to_ascii_lowercase())
}

fn trim_block(s: &str) -> String {
    s.trim_matches('\n').trim_end().to_string()
}

// ---------------------------------------------------------------------------------------
// Frontmatter → Front
// ---------------------------------------------------------------------------------------

/// Known frontmatter keys, in the order `to_mapping` emits them.
const KNOWN_KEYS: &[&str] = &[
    "method",
    "url",
    "headers",
    "query",
    "path_vars",
    "form",
    "form_data",
    "file",
    "body_type",
    "auth",
    "vars",
    "capture",
    "parents",
    "timeout",
    "follow_redirects",
    "verify_tls",
];

impl Front {
    fn from_mapping(mut map: Mapping, notes: &mut Vec<Note>) -> Result<Front, String> {
        let mut front = Front {
            method: take_str(&mut map, "method", notes)?,
            url: take_str(&mut map, "url", notes)?,
            headers: take_strmap(&mut map, "headers", notes)?,
            query: take_strmap(&mut map, "query", notes)?,
            path_vars: take_strmap(&mut map, "path_vars", notes)?,
            form: take_opt_strmap(&mut map, "form", notes)?,
            form_data: take_opt_strmap(&mut map, "form_data", notes)?,
            file: take_str(&mut map, "file", notes)?,
            body_type: take_str(&mut map, "body_type", notes)?,
            auth: take_auth(&mut map, notes)?,
            vars: take_vars(&mut map, notes)?,
            capture: take_strmap(&mut map, "capture", notes)?,
            parents: take_str_seq(&mut map, "parents", notes)?,
            timeout: take_u64(&mut map, "timeout", notes)?,
            follow_redirects: take_bool(&mut map, "follow_redirects", notes)?,
            verify_tls: take_bool(&mut map, "verify_tls", notes)?,
            extra: Mapping::new(),
        };

        // Whatever is left is unknown to this build: keep it, and say so once.
        for (k, v) in map {
            if let Some(name) = k.as_str() {
                notes.push(Note(format!(
                    "unknown frontmatter key `{name}` — kept as-is{}",
                    suggest(name)
                )));
            }
            front.extra.insert(k, v);
        }
        Ok(front)
    }

    fn to_mapping(&self) -> Mapping {
        let mut m = Mapping::new();
        if let Some(v) = &self.method {
            m.insert("method".into(), v.as_str().into());
        }
        if let Some(v) = &self.url {
            m.insert("url".into(), v.as_str().into());
        }
        insert_strmap(&mut m, "headers", &self.headers);
        insert_strmap(&mut m, "query", &self.query);
        insert_strmap(&mut m, "path_vars", &self.path_vars);
        if let Some(f) = &self.form {
            m.insert("form".into(), strmap_value(f));
        }
        if let Some(f) = &self.form_data {
            m.insert("form_data".into(), strmap_value(f));
        }
        if let Some(v) = &self.file {
            m.insert("file".into(), v.as_str().into());
        }
        if let Some(v) = &self.body_type {
            m.insert("body_type".into(), v.as_str().into());
        }
        if let Some(a) = &self.auth {
            m.insert("auth".into(), a.to_value());
        }
        if !self.vars.is_empty() {
            let mut vm = Mapping::new();
            for (k, spec) in &self.vars {
                vm.insert(k.as_str().into(), spec.to_value());
            }
            m.insert("vars".into(), Value::Mapping(vm));
        }
        insert_strmap(&mut m, "capture", &self.capture);
        if !self.parents.is_empty() {
            m.insert(
                "parents".into(),
                Value::Sequence(self.parents.iter().map(|p| p.as_str().into()).collect()),
            );
        }
        if let Some(v) = self.timeout {
            m.insert("timeout".into(), v.into());
        }
        if let Some(v) = self.follow_redirects {
            m.insert("follow_redirects".into(), v.into());
        }
        if let Some(v) = self.verify_tls {
            m.insert("verify_tls".into(), v.into());
        }
        for (k, v) in &self.extra {
            m.insert(k.clone(), v.clone());
        }
        m
    }

    /// The declared spec for a variable, if any.
    pub fn var(&self, key: &str) -> Option<&VarSpec> {
        self.vars.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// A cheap did-you-mean over the known keys: one edit away, or a known key with the
/// separator written the other way (`pathVars` / `path-vars`).
fn suggest(name: &str) -> String {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect::<String>()
    };
    let n = norm(name);
    for k in KNOWN_KEYS {
        if norm(k) == n || edit_distance_1(&n, &norm(k)) {
            return format!(" (did you mean `{k}`?)");
        }
    }
    String::new()
}

/// True when `a` and `b` are within one insert/delete/substitute. Not a general Levenshtein
/// — just enough to catch a typo without a dependency.
fn edit_distance_1(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (mut i, mut j, mut diffs) = (0usize, 0usize, 0usize);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        diffs += 1;
        if diffs > 1 {
            return false;
        }
        match a.len().cmp(&b.len()) {
            std::cmp::Ordering::Greater => i += 1,
            std::cmp::Ordering::Less => j += 1,
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    diffs + (a.len() - i) + (b.len() - j) <= 1
}

// --- typed readers ----------------------------------------------------------------------

fn take(map: &mut Mapping, key: &str) -> Option<Value> {
    map.remove(Value::from(key))
}

/// Coerce a YAML scalar to a string. Numbers and booleans are accepted (and noted) because
/// `per_page: 5` and `secure: true` are what people actually write; a mapping or sequence
/// where a scalar belongs is genuinely ambiguous and fails.
fn scalar(v: &Value, at: &str, notes: &mut Vec<Note>) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => {
            notes.push(Note(format!(
                "{at}: number `{n}` read as the string \"{n}\""
            )));
            Ok(n.to_string())
        }
        Value::Bool(b) => {
            notes.push(Note(format!(
                "{at}: boolean `{b}` read as the string \"{b}\""
            )));
            Ok(b.to_string())
        }
        Value::Null => {
            notes.push(Note(format!("{at}: null read as an empty string")));
            Ok(String::new())
        }
        _ => Err(format!("{at}: expected a scalar, found a collection")),
    }
}

fn take_str(map: &mut Mapping, key: &str, notes: &mut Vec<Note>) -> Result<Option<String>, String> {
    match take(map, key) {
        None => Ok(None),
        Some(v) => Ok(Some(scalar(&v, key, notes)?)),
    }
}

fn take_u64(map: &mut Mapping, key: &str, notes: &mut Vec<Note>) -> Result<Option<u64>, String> {
    match take(map, key) {
        None => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| format!("{key}: expected a non-negative whole number, found `{n}`"))
            .map(Some),
        Some(v) => {
            let s = scalar(&v, key, notes)?;
            s.trim()
                .parse::<u64>()
                .map(Some)
                .map_err(|_| format!("{key}: expected a number, found \"{s}\""))
        }
    }
}

fn take_bool(map: &mut Mapping, key: &str, notes: &mut Vec<Note>) -> Result<Option<bool>, String> {
    match take(map, key) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(b)),
        Some(v) => {
            let s = scalar(&v, key, notes)?;
            match s.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => Ok(Some(true)),
                "false" | "no" | "off" | "0" => Ok(Some(false)),
                _ => Err(format!("{key}: expected true or false, found \"{s}\"")),
            }
        }
    }
}

fn take_strmap(map: &mut Mapping, key: &str, notes: &mut Vec<Note>) -> Result<StrMap, String> {
    Ok(take_opt_strmap(map, key, notes)?.unwrap_or_default())
}

fn take_opt_strmap(
    map: &mut Mapping,
    key: &str,
    notes: &mut Vec<Note>,
) -> Result<Option<StrMap>, String> {
    let Some(v) = take(map, key) else {
        return Ok(None);
    };
    match v {
        Value::Null => Ok(Some(Vec::new())),
        Value::Mapping(m) => {
            let mut out = Vec::new();
            for (k, val) in m {
                let name = match &k {
                    Value::String(s) => s.clone(),
                    other => scalar(other, &format!("{key}: key"), notes)?,
                };
                let at = format!("{key}.{name}");
                out.push((name, scalar(&val, &at, notes)?));
            }
            Ok(Some(out))
        }
        // `headers: ["Accept: text/plain"]` — a shape people try; accept it rather than
        // rejecting a file we can plainly understand.
        Value::Sequence(seq) => {
            let mut out = Vec::new();
            for (i, item) in seq.iter().enumerate() {
                let s = scalar(item, &format!("{key}[{i}]"), notes)?;
                match s.split_once(':') {
                    Some((k, val)) => out.push((k.trim().to_string(), val.trim().to_string())),
                    None => {
                        return Err(format!("{key}[{i}]: expected `name: value`, found \"{s}\""))
                    }
                }
            }
            notes.push(Note(format!("{key}: list form read as a mapping")));
            Ok(Some(out))
        }
        _ => Err(format!("{key}: expected a mapping of names to values")),
    }
}

fn take_str_seq(
    map: &mut Mapping,
    key: &str,
    notes: &mut Vec<Note>,
) -> Result<Vec<String>, String> {
    match take(map, key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Sequence(seq)) => seq
            .iter()
            .enumerate()
            .map(|(i, v)| scalar(v, &format!("{key}[{i}]"), notes))
            .collect(),
        // `parents: login` — a single dependency needs no brackets.
        Some(v) => Ok(vec![scalar(&v, key, notes)?]),
    }
}

fn take_vars(map: &mut Mapping, notes: &mut Vec<Note>) -> Result<Vec<(String, VarSpec)>, String> {
    let Some(v) = take(map, "vars") else {
        return Ok(Vec::new());
    };
    let Value::Mapping(m) = v else {
        if matches!(v, Value::Null) {
            return Ok(Vec::new());
        }
        return Err("vars: expected a mapping of variable names to specs".into());
    };
    let mut out = Vec::new();
    for (k, val) in m {
        let name = match &k {
            Value::String(s) => s.clone(),
            other => scalar(other, "vars: key", notes)?,
        };
        // `per_page: 10` is the natural way to write a default, and every value here is a
        // string by the time it is used. Reporting that coercion on every run would train
        // people to ignore notes, which is how the notes that matter get missed.
        let notes = &mut Vec::new();
        let at = format!("vars.{name}");
        let spec = match val {
            Value::Mapping(mut sm) => VarSpec {
                default: take_str(&mut sm, "default", notes)?,
                prompt: take_str(&mut sm, "prompt", notes)?,
                env: take_str(&mut sm, "env", notes)?,
                secret: take_bool(&mut sm, "secret", notes)?.unwrap_or(false),
                required: take_bool(&mut sm, "required", notes)?.unwrap_or(false),
                description: take_str(&mut sm, "description", notes)?,
            },
            // `owner: anthropics` — the shorthand for "just a default".
            other => VarSpec {
                default: Some(scalar(&other, &at, notes)?),
                ..VarSpec::default()
            },
        };
        out.push((name, spec));
    }
    Ok(out)
}

fn take_auth(map: &mut Mapping, notes: &mut Vec<Note>) -> Result<Option<AuthSpec>, String> {
    let Some(v) = take(map, "auth") else {
        return Ok(None);
    };
    match v {
        Value::Null => Ok(Some(AuthSpec::None)),
        Value::String(s) => match s.as_str() {
            "none" => Ok(Some(AuthSpec::None)),
            "inherit" => Ok(Some(AuthSpec::Inherit)),
            other => Err(format!(
                "auth: \"{other}\" needs its fields — write `auth: {{ type: {other}, … }}`"
            )),
        },
        Value::Mapping(mut m) => {
            let kind = take_str(&mut m, "type", notes)?
                .ok_or_else(|| "auth: missing `type`".to_string())?
                .to_ascii_lowercase();
            let spec = match kind.as_str() {
                "none" => AuthSpec::None,
                "inherit" => AuthSpec::Inherit,
                "basic" | "basic_auth" => AuthSpec::Basic {
                    username: take_str(&mut m, "username", notes)?.unwrap_or_default(),
                    password: take_str(&mut m, "password", notes)?.unwrap_or_default(),
                },
                "bearer" | "bearer_token" => AuthSpec::Bearer {
                    token: take_str(&mut m, "token", notes)?.unwrap_or_default(),
                    prefix: match take(&mut m, "prefix") {
                        // An explicit `prefix: null` means "send the bare token" — the
                        // tri-state the Idealised Model is careful to keep (§6).
                        Some(Value::Null) => None,
                        Some(v) => Some(scalar(&v, "auth.prefix", notes)?),
                        None => Some("Bearer".to_string()),
                    },
                },
                "api_key" | "apikey" => {
                    let placement = take_str(&mut m, "in", notes)?.unwrap_or_default();
                    AuthSpec::ApiKey {
                        key: take_str(&mut m, "key", notes)?.unwrap_or_default(),
                        value: take_str(&mut m, "value", notes)?.unwrap_or_default(),
                        in_query: matches!(placement.as_str(), "query" | "query_param"),
                    }
                }
                other => {
                    notes.push(Note(format!(
                        "auth: `{other}` is preserved but this build cannot send it"
                    )));
                    let mut raw = m.clone();
                    raw.insert("type".into(), other.into());
                    m = Mapping::new();
                    AuthSpec::Other {
                        kind: other.to_string(),
                        raw,
                    }
                }
            };
            for (k, _) in &m {
                if let Some(name) = k.as_str() {
                    notes.push(Note(format!("auth: unknown field `{name}` ignored")));
                }
            }
            Ok(Some(spec))
        }
        _ => Err("auth: expected a mapping or one of `none` / `inherit`".into()),
    }
}

// --- writers -----------------------------------------------------------------------------

fn strmap_value(m: &StrMap) -> Value {
    let mut out = Mapping::new();
    for (k, v) in m {
        out.insert(k.as_str().into(), v.as_str().into());
    }
    Value::Mapping(out)
}

fn insert_strmap(m: &mut Mapping, key: &str, v: &StrMap) {
    if !v.is_empty() {
        m.insert(key.into(), strmap_value(v));
    }
}

impl VarSpec {
    fn to_value(&self) -> Value {
        // The shorthand form when a default is all there is.
        if self.prompt.is_none()
            && self.env.is_none()
            && !self.secret
            && !self.required
            && self.description.is_none()
        {
            return self.default.clone().unwrap_or_default().into();
        }
        let mut m = Mapping::new();
        if let Some(v) = &self.default {
            m.insert("default".into(), v.as_str().into());
        }
        if let Some(v) = &self.prompt {
            m.insert("prompt".into(), v.as_str().into());
        }
        if let Some(v) = &self.env {
            m.insert("env".into(), v.as_str().into());
        }
        if self.secret {
            m.insert("secret".into(), true.into());
        }
        if self.required {
            m.insert("required".into(), true.into());
        }
        if let Some(v) = &self.description {
            m.insert("description".into(), v.as_str().into());
        }
        Value::Mapping(m)
    }
}

impl AuthSpec {
    fn to_value(&self) -> Value {
        let mut m = Mapping::new();
        match self {
            AuthSpec::None => return "none".into(),
            AuthSpec::Inherit => return "inherit".into(),
            AuthSpec::Basic { username, password } => {
                m.insert("type".into(), "basic".into());
                m.insert("username".into(), username.as_str().into());
                m.insert("password".into(), password.as_str().into());
            }
            AuthSpec::Bearer { token, prefix } => {
                m.insert("type".into(), "bearer".into());
                m.insert("token".into(), token.as_str().into());
                match prefix {
                    Some(p) if p != "Bearer" => {
                        m.insert("prefix".into(), p.as_str().into());
                    }
                    Some(_) => {}
                    None => {
                        m.insert("prefix".into(), Value::Null);
                    }
                }
            }
            AuthSpec::ApiKey {
                key,
                value,
                in_query,
            } => {
                m.insert("type".into(), "api_key".into());
                m.insert("key".into(), key.as_str().into());
                m.insert("value".into(), value.as_str().into());
                if *in_query {
                    m.insert("in".into(), "query".into());
                }
            }
            AuthSpec::Other { raw, .. } => return Value::Mapping(raw.clone()),
        }
        Value::Mapping(m)
    }
}

// ---------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const PITCH: &str = r#"---
method: GET
url: https://api.github.com/repos/{{owner}}/{{repo}}/issues
headers:
  Accept: application/vnd.github+json
  Authorization: Bearer {{GH_TOKEN}}
query:
  state: open
  per_page: 5
vars:
  owner: { default: anthropics, prompt: "Repository owner" }
  repo: { default: claude-code, prompt: "Repository name" }
  GH_TOKEN: { env: GH_TOKEN, secret: true, required: true }
parents: []
---

-- description --

List open issues for a GitHub repository.

-- view --

# {{ response | length }} open issues

-- post --

rq.test('200 OK', () => rq.response.status === 200);
"#;

    #[test]
    fn parses_the_pitch_document() {
        let (doc, notes) = Document::parse(PITCH).unwrap();
        assert_eq!(doc.front.method.as_deref(), Some("GET"));
        assert_eq!(doc.front.headers.len(), 2);
        assert_eq!(doc.front.headers[0].0, "Accept");
        assert_eq!(doc.front.query[1], ("per_page".into(), "5".into()));
        assert_eq!(doc.front.vars.len(), 3);
        assert_eq!(
            doc.front.var("owner").unwrap().prompt.as_deref(),
            Some("Repository owner")
        );
        assert!(doc.front.var("GH_TOKEN").unwrap().secret);
        assert!(doc.front.parents.is_empty());
        assert!(doc.section("description").unwrap().starts_with("List open"));
        assert!(doc.section("post").unwrap().contains("rq.test"));
        // Nothing to report: `per_page: 5` is how a person writes a number, and it becomes
        // "5" exactly.
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn round_trips_through_write() {
        let (doc, _) = Document::parse(PITCH).unwrap();
        let (again, _) = Document::parse(&doc.write()).unwrap();
        assert_eq!(doc.front.url, again.front.url);
        assert_eq!(doc.front.headers, again.front.headers);
        assert_eq!(doc.front.vars, again.front.vars);
        assert_eq!(doc.sections, again.sections);
    }

    #[test]
    fn unknown_keys_and_sections_survive() {
        let src = "---\nurl: https://x.test\nfuture_key: keep-me\n---\n\n-- lore --\n\nhello\n";
        let (doc, notes) = Document::parse(src).unwrap();
        assert!(notes.iter().any(|n| n.0.contains("future_key")));
        let out = doc.write();
        assert!(out.contains("future_key: keep-me"), "{out}");
        assert!(out.contains("-- lore --"), "{out}");
    }

    #[test]
    fn suggests_a_near_miss_key() {
        let (_, notes) = Document::parse("---\nurl: https://x.test\nheders: {}\n---\n").unwrap();
        assert!(
            notes.iter().any(|n| n.0.contains("did you mean `headers`")),
            "{notes:?}"
        );
    }

    #[test]
    fn markdown_rules_are_not_section_markers() {
        assert_eq!(section_marker("-- view --"), Some("view".into()));
        assert_eq!(section_marker("  -- pre --  "), Some("pre".into()));
        assert_eq!(section_marker("---"), None);
        assert_eq!(section_marker("-- not a marker --"), None);
        assert_eq!(section_marker("a -- view --"), None);
    }

    #[test]
    fn bearer_prefix_is_tri_state() {
        let bare = "---\nurl: u\nauth: { type: bearer, token: t, prefix: null }\n---\n";
        let (doc, _) = Document::parse(bare).unwrap();
        assert_eq!(
            doc.front.auth,
            Some(AuthSpec::Bearer {
                token: "t".into(),
                prefix: None
            })
        );
        // …and survives a write/read cycle rather than collapsing to the default.
        let (again, _) = Document::parse(&doc.write()).unwrap();
        assert_eq!(doc.front.auth, again.front.auth);
    }

    #[test]
    fn unknown_auth_is_preserved_not_stripped() {
        let src = "---\nurl: u\nauth: { type: hawk, id: abc, key: secret }\n---\n";
        let (doc, notes) = Document::parse(src).unwrap();
        assert!(notes.iter().any(|n| n.0.contains("cannot send")));
        assert!(doc.write().contains("hawk"));
        assert!(doc.write().contains("secret"));
    }

    #[test]
    fn missing_frontmatter_is_fatal() {
        let err = Document::parse("just some markdown\n").unwrap_err();
        assert!(err.contains("missing frontmatter"), "{err}");
    }

    #[test]
    fn preamble_becomes_the_description() {
        let (doc, notes) = Document::parse("---\nurl: u\n---\n\nloose prose\n").unwrap();
        assert_eq!(doc.section("description"), Some("loose prose"));
        assert!(notes.iter().any(|n| n.0.contains("description")));
    }
}

// ---------------------------------------------------------------------------------------
// The `-- form --` section
// ---------------------------------------------------------------------------------------

/// One field of a request's input form.
///
/// A form is the same idea as `vars:` — values the request needs — declared for *typing
/// into* rather than for defaulting. That is the whole distinction: `vars:` says where a
/// value comes from when you don't supply one, a form says this request expects you to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormField {
    pub name: String,
    /// What to call it on screen. Defaults to the field name.
    pub label: Option<String>,
    pub default: Option<String>,
    pub required: bool,
    /// Read without echo, masked on screen.
    pub secret: bool,
    /// Expected to hold more than one line. The console shows it taller; the terminal
    /// prompt reads until a blank line.
    pub multiline: bool,
    /// A line of explanation under the field.
    pub help: Option<String>,
}

impl FormField {
    pub fn title(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
}

impl Document {
    /// Parse the `-- form --` section. Same shape as `vars:`, because they are the same
    /// kind of thing: a mapping of name → spec, or name → default for the short form.
    ///
    /// A malformed form is an error rather than an empty one: silently showing no fields
    /// for a request that needs three is worse than refusing to open it.
    pub fn form(&self) -> Result<Vec<FormField>, String> {
        let Some(body) = self.section("form").filter(|s| !s.trim().is_empty()) else {
            return Ok(Vec::new());
        };
        let value: Value = serde_norway::from_str(body)
            .map_err(|e| format!("`-- form --` is not valid YAML: {e}"))?;
        let Value::Mapping(map) = value else {
            return Err("`-- form --` must be a mapping of field names to specs".into());
        };

        let mut notes = Vec::new();
        let mut fields = Vec::new();
        for (key, spec) in map {
            let name = match &key {
                Value::String(s) => s.clone(),
                other => scalar(other, "form: field name", &mut notes)?,
            };
            let at = format!("form.{name}");
            let field = match spec {
                Value::Mapping(mut m) => FormField {
                    label: take_str(&mut m, "label", &mut notes)?,
                    default: take_str(&mut m, "default", &mut notes)?,
                    required: take_bool(&mut m, "required", &mut notes)?.unwrap_or(false),
                    secret: take_bool(&mut m, "secret", &mut notes)?.unwrap_or(false),
                    multiline: take_bool(&mut m, "multiline", &mut notes)?.unwrap_or(false),
                    help: take_str(&mut m, "help", &mut notes)?,
                    name,
                },
                // `text: "hello"` — the short form, a default and nothing else.
                other => FormField {
                    default: Some(scalar(&other, &at, &mut notes)?),
                    name,
                    ..FormField::default()
                },
            };
            fields.push(field);
        }
        Ok(fields)
    }

    /// The form's fields as declared variables, so the terminal prompt path and the console
    /// form ask for exactly the same things.
    pub fn form_vars(&self) -> Result<Vec<(String, VarSpec)>, String> {
        Ok(self
            .form()?
            .into_iter()
            .map(|f| {
                let title = f.title().to_string();
                (
                    f.name.clone(),
                    VarSpec {
                        default: f.default,
                        prompt: Some(title),
                        env: None,
                        secret: f.secret,
                        required: f.required,
                        description: f.help,
                    },
                )
            })
            .collect())
    }
}

#[cfg(test)]
mod form_tests {
    use super::*;

    const DOC: &str = "---\nurl: https://api.test/posts\nmethod: POST\n---\n\n\
        -- form --\n\n\
        text: { label: \"What's happening?\", multiline: true, required: true }\n\
        reply_to: { label: \"Reply to\", help: \"a post id, or leave empty\" }\n\
        draft: false\n";

    #[test]
    fn a_form_declares_fields_in_file_order() {
        let (doc, _) = Document::parse(DOC).unwrap();
        let form = doc.form().unwrap();
        assert_eq!(
            form.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            vec!["text", "reply_to", "draft"]
        );
        assert_eq!(form[0].title(), "What's happening?");
        assert!(form[0].multiline && form[0].required);
        assert_eq!(form[1].help.as_deref(), Some("a post id, or leave empty"));
        // The short form is a default and nothing else.
        assert_eq!(form[2].default.as_deref(), Some("false"));
        assert_eq!(form[2].title(), "draft");
    }

    #[test]
    fn a_form_becomes_the_same_declared_variables_the_prompt_path_uses() {
        let (doc, _) = Document::parse(DOC).unwrap();
        let vars = doc.form_vars().unwrap();
        assert_eq!(vars[0].0, "text");
        assert_eq!(vars[0].1.prompt.as_deref(), Some("What's happening?"));
        assert!(vars[0].1.required);
    }

    #[test]
    fn no_form_section_is_no_fields_not_an_error() {
        let (doc, _) = Document::parse("---\nurl: u\n---\n").unwrap();
        assert!(doc.form().unwrap().is_empty());
    }

    #[test]
    fn a_malformed_form_refuses_rather_than_showing_nothing() {
        let (doc, _) =
            Document::parse("---\nurl: u\n---\n\n-- form --\n\njust a sentence\n").unwrap();
        let err = doc.form().unwrap_err();
        assert!(err.contains("mapping"), "{err}");
    }

    #[test]
    fn a_form_round_trips_through_write() {
        let (doc, _) = Document::parse(DOC).unwrap();
        let (again, _) = Document::parse(&doc.write()).unwrap();
        assert_eq!(doc.form().unwrap(), again.form().unwrap());
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;

    #[test]
    fn the_summary_is_the_first_line_of_the_description() {
        let src = "---\nurl: u\n---\n\n-- description --\n\nWrite a post.\n\nAnd more detail\nbelow it.\n";
        let (doc, _) = Document::parse(src).unwrap();
        assert_eq!(doc.summary().as_deref(), Some("Write a post."));
    }

    #[test]
    fn a_heading_is_still_a_summary() {
        let src = "---\nurl: u\n---\n\n-- description --\n\n# Compose\n\nwords\n";
        let (doc, _) = Document::parse(src).unwrap();
        assert_eq!(doc.summary().as_deref(), Some("Compose"));
    }

    #[test]
    fn no_description_is_no_summary() {
        let (doc, _) = Document::parse("---\nurl: u\n---\n").unwrap();
        assert_eq!(doc.summary(), None);
        let (blank, _) = Document::parse("---\nurl: u\n---\n\n-- description --\n\n\n").unwrap();
        assert_eq!(blank.summary(), None);
    }
}
