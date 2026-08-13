//! Running a request: resolve the graph, inherit from the collections above it, substitute
//! variables, send, capture, render.

use anyhow::{bail, Context, Result};

use crate::doc::{AuthSpec, Document, VarSpec};
use crate::graph;
use crate::http::{self, Payload, Prepared, Response};
use crate::project::{Project, REQUEST_FILE};
use crate::render;
use crate::vars::{self, Vars};

#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    /// `--var key=value`, the highest-precedence layer.
    pub cli_vars: Vec<(String, String)>,
    /// `--environment name`, overriding the active one.
    pub environment: Option<String>,
    /// `--prompt`: ask for every declared variable.
    pub prompt: bool,
    pub interactive: bool,
    /// `--strict`: any note (unresolved variable, unexecuted script) fails the run.
    pub strict: bool,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub rel: String,
    pub name: String,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub response: Response,
    pub captured: Vec<(String, String)>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub steps: Vec<Step>,
    /// The rendered `-- view --` of the requested step, when it has one.
    pub view: Option<String>,
    /// The requested step's body, pretty-printed — what `--raw` shows instead of the view.
    pub raw: String,
    /// Every variable the requested step resolved: name, value (secrets masked), origin.
    pub vars: Vec<(String, String, String)>,
    pub notes: Vec<String>,
    pub secrets: Vec<String>,
}

impl Run {
    pub fn target(&self) -> &Step {
        self.steps.last().expect("a run always has a final step")
    }

    /// Non-zero when the requested step didn't come back 2xx — so `rq r` composes in CI.
    pub fn failed(&self) -> bool {
        !self.target().response.ok()
    }
}

/// Run `target` and everything it declares as a parent.
pub fn run(project: &Project, target: usize, opts: &RunOptions) -> Result<Run> {
    let order = graph::plan(project, target)?;

    // Environment layers are read once and shared by every step in the run.
    let env_name = opts
        .environment
        .clone()
        .or_else(|| project.active_env())
        .filter(|n| !n.is_empty());
    let mut env_layer: Vec<(String, String)> = Vec::new();
    let mut env_declared: Vec<(String, VarSpec)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    if let Some(name) = &env_name {
        let (doc, doc_notes) = project.load_env(name)?;
        notes.extend(doc_notes.into_iter().map(|n| format!("{name}.md: {n}")));
        env_declared = doc.front.vars.clone();
    }
    let mut global_declared: Vec<(String, VarSpec)> = Vec::new();
    if project.env_path(crate::project::GLOBAL_ENV).is_file()
        && env_name.as_deref() != Some(crate::project::GLOBAL_ENV)
    {
        let (doc, doc_notes) = project.load_env(crate::project::GLOBAL_ENV)?;
        notes.extend(doc_notes.into_iter().map(|n| format!("__global.md: {n}")));
        global_declared = doc.front.vars.clone();
    }
    // An environment's variables are values, not prompts: take their declared defaults
    // directly rather than asking for them.
    for (k, spec) in env_declared.iter().chain(global_declared.iter()) {
        if let Some(v) = env_value(spec) {
            env_layer.push((k.clone(), v));
        }
    }
    let env_secrets: Vec<String> = env_declared
        .iter()
        .chain(global_declared.iter())
        .filter(|(_, s)| s.secret)
        .map(|(k, _)| k.clone())
        .collect();

    let mut runtime: Vec<(String, String)> = Vec::new();
    let mut steps: Vec<Step> = Vec::new();
    let mut secrets: Vec<String> = Vec::new();

    for idx in order {
        let entry = &project.entries[idx];
        let (doc, doc_notes) = project.load(idx)?;
        let mut step_notes: Vec<String> = doc_notes.into_iter().map(|n| n.to_string()).collect();

        let inherited = Inherited::gather(project, idx, &mut step_notes)?;

        // Highest precedence first; `Vars` keeps the first value it is given.
        let mut v = Vars::new();
        v.layer("--var", opts.cli_vars.clone());
        v.layer("capture", runtime.clone());
        v.layer(
            env_name.as_deref().unwrap_or("environment"),
            env_layer.clone(),
        );
        for key in &env_secrets {
            v.mark_secret(key);
        }

        let mut declared = doc.front.vars.clone();
        declared.extend(inherited.declared.clone());
        vars::resolve_declared(&mut v, &declared, opts.prompt, opts.interactive)
            .with_context(|| entry.rel.clone())?;
        secrets.extend(v.secret_values());

        let prepared =
            prepare(&doc, &inherited, &v, &mut step_notes).with_context(|| entry.rel.clone())?;
        http::check_url(&prepared.url).with_context(|| entry.rel.clone())?;

        let response = http::send(&prepared).with_context(|| entry.rel.clone())?;
        let ctx = context_of(&prepared, &response, &v);

        let mut captured = Vec::new();
        for (key, path) in &doc.front.capture {
            match vars::extract(&ctx, path) {
                Some(value) => {
                    runtime.retain(|(k, _)| k != key);
                    runtime.push((key.clone(), value.clone()));
                    captured.push((key.clone(), value));
                }
                None => step_notes.push(format!(
                    "capture `{key}`: nothing at `{path}` in the response"
                )),
            }
        }

        // Scripts are the next slice of work; until then, say so on every run rather than
        // quietly sending a request whose pre-script never ran.
        for section in ["pre", "post"] {
            if doc.section(section).is_some_and(|s| !s.trim().is_empty()) {
                step_notes.push(format!(
                    "`-- {section} --` was NOT executed: this build has no script runtime yet"
                ));
            }
        }

        let is_target = idx == target;
        let step = Step {
            rel: entry.rel.clone(),
            name: entry.name.clone(),
            method: prepared.method.clone(),
            url: prepared.url.clone(),
            request_headers: prepared.headers.clone(),
            body: prepared.body.as_ref().map(|b| b.describe()),
            response,
            captured,
            notes: step_notes,
        };

        if is_target {
            let view = match doc.section("view").filter(|s| !s.trim().is_empty()) {
                Some(tpl) => Some(render::render_view(tpl, &ctx)?),
                None => None,
            };
            let raw = render::default_body(&step.response.body, step.response.json().as_ref());
            let resolved = v
                .iter()
                .map(|(k, val)| {
                    let shown = if v.is_secret(k) {
                        "***".to_string()
                    } else {
                        val.clone()
                    };
                    (
                        k.clone(),
                        shown,
                        v.origin(k).unwrap_or("unknown").to_string(),
                    )
                })
                .collect();
            steps.push(step);
            let mut run = Run {
                steps,
                view,
                raw,
                vars: resolved,
                notes,
                secrets,
            };
            run.secrets.sort();
            run.secrets.dedup();
            if opts.strict {
                enforce_strict(&run)?;
            }
            return Ok(run);
        }
        steps.push(step);
    }

    unreachable!("the plan always ends with the requested request")
}

fn enforce_strict(run: &Run) -> Result<()> {
    let mut all: Vec<&String> = run.notes.iter().collect();
    for s in &run.steps {
        all.extend(s.notes.iter());
    }
    if !all.is_empty() {
        bail!(
            "--strict: {} note(s) on this run\n  {}",
            all.len(),
            all.iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }
    Ok(())
}

/// An environment entry's value: an explicit default, or a bound process variable.
fn env_value(spec: &VarSpec) -> Option<String> {
    if let Some(name) = &spec.env {
        if let Ok(v) = std::env::var(name) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    spec.default.clone()
}

/// Everything a request picks up from the collections it sits inside.
#[derive(Debug, Default)]
pub struct Inherited {
    pub headers: Vec<(String, String)>,
    pub declared: Vec<(String, VarSpec)>,
    pub auth: Option<AuthSpec>,
    pub timeout: Option<u64>,
    pub follow_redirects: Option<bool>,
    pub verify_tls: Option<bool>,
}

impl Inherited {
    fn gather(project: &Project, idx: usize, notes: &mut Vec<String>) -> Result<Inherited> {
        let mut out = Inherited::default();
        // Outermost first, so a nearer collection's header overwrites a farther one's.
        for anc in project.ancestors(idx) {
            let Some((doc, doc_notes)) = project.load_collection(anc)? else {
                continue;
            };
            let rel = &project.entries[anc].rel;
            notes.extend(doc_notes.into_iter().map(|n| format!("{rel}: {n}")));
            for (k, v) in &doc.front.headers {
                out.headers.retain(|(ek, _)| !ek.eq_ignore_ascii_case(k));
                out.headers.push((k.clone(), v.clone()));
            }
            match &doc.front.auth {
                Some(AuthSpec::Inherit) | None => {}
                Some(a) => out.auth = Some(a.clone()),
            }
            out.timeout = doc.front.timeout.or(out.timeout);
            out.follow_redirects = doc.front.follow_redirects.or(out.follow_redirects);
            out.verify_tls = doc.front.verify_tls.or(out.verify_tls);
            // Nearest collection's declarations first.
            let mut declared = doc.front.vars.clone();
            declared.extend(std::mem::take(&mut out.declared));
            out.declared = declared;
        }
        Ok(out)
    }
}

/// Substitute everything and assemble the request that will go on the wire.
pub fn prepare(
    doc: &Document,
    inherited: &Inherited,
    v: &Vars,
    notes: &mut Vec<String>,
) -> Result<Prepared> {
    let front = &doc.front;
    let mut missing: Vec<String> = Vec::new();
    let mut sub = |text: &str| -> String {
        let s = vars::substitute(text, v);
        for m in s.missing {
            if !missing.contains(&m) {
                missing.push(m);
            }
        }
        s.text
    };

    let raw_url = front
        .url
        .clone()
        .ok_or_else(|| anyhow::anyhow!("the request has no `url:`"))?;
    let path_vars: Vec<(String, String)> = front
        .path_vars
        .iter()
        .map(|(k, val)| (k.clone(), sub(val)))
        .collect();
    let url = http::apply_path_vars(&sub(&raw_url), &path_vars);
    let mut query: Vec<(String, String)> = front
        .query
        .iter()
        .map(|(k, val)| (sub(k), sub(val)))
        .collect();

    let mut headers: Vec<(String, String)> = Vec::new();
    for (k, val) in inherited.headers.iter().chain(front.headers.iter()) {
        let k = sub(k);
        let val = sub(val);
        headers.retain(|(ek, _)| !ek.eq_ignore_ascii_case(&k));
        headers.push((k, val));
    }

    let auth = match &front.auth {
        Some(AuthSpec::Inherit) | None => inherited.auth.clone(),
        Some(AuthSpec::None) => None,
        Some(other) => Some(other.clone()),
    };
    if let Some(auth) = auth {
        apply_auth(&auth, &mut headers, &mut query, &mut sub, notes);
    }

    let body = build_payload(doc, &headers, &mut sub)?;

    for m in &missing {
        notes.push(format!(
            "`{{{{{m}}}}}` is unresolved and was sent as written"
        ));
    }

    Ok(Prepared {
        method: front.method.clone().unwrap_or_else(|| "GET".into()),
        url: http::with_query(&url, &query),
        headers,
        body,
        timeout_ms: front.timeout.or(inherited.timeout),
        follow_redirects: front
            .follow_redirects
            .or(inherited.follow_redirects)
            .unwrap_or(true),
        verify_tls: front.verify_tls.or(inherited.verify_tls).unwrap_or(true),
    })
}

fn apply_auth(
    auth: &AuthSpec,
    headers: &mut Vec<(String, String)>,
    query: &mut Vec<(String, String)>,
    sub: &mut impl FnMut(&str) -> String,
    notes: &mut Vec<String>,
) {
    let mut set_header = |name: &str, value: String| {
        // An explicit header in the file wins over generated auth: if you wrote it, you
        // meant it.
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name)) {
            headers.push((name.to_string(), value));
        }
    };
    match auth {
        AuthSpec::None | AuthSpec::Inherit => {}
        AuthSpec::Basic { username, password } => {
            let token = base64(format!("{}:{}", sub(username), sub(password)).as_bytes());
            set_header("Authorization", format!("Basic {token}"));
        }
        AuthSpec::Bearer { token, prefix } => {
            let token = sub(token);
            let value = match prefix {
                Some(p) if !p.is_empty() => format!("{} {token}", sub(p)),
                _ => token,
            };
            set_header("Authorization", value);
        }
        AuthSpec::ApiKey {
            key,
            value,
            in_query,
        } => {
            let (key, value) = (sub(key), sub(value));
            if *in_query {
                query.push((key, value));
            } else {
                set_header(&key, value);
            }
        }
        AuthSpec::Other { kind, .. } => notes.push(format!(
            "auth `{kind}` was not applied: this build can send basic, bearer and api_key"
        )),
    }
}

/// Assemble the body from whichever of the four sources the file uses. Using two is an
/// error — a request has one body, and guessing which one you meant is how data gets lost.
fn build_payload(
    doc: &Document,
    headers: &[(String, String)],
    sub: &mut impl FnMut(&str) -> String,
) -> Result<Option<Payload>> {
    let front = &doc.front;
    let section = doc.section("body").filter(|s| !s.trim().is_empty());
    let sources: Vec<&str> = [
        section.map(|_| "-- body --"),
        front.form.as_ref().map(|_| "form:"),
        front.form_data.as_ref().map(|_| "form_data:"),
        front.file.as_ref().map(|_| "file:"),
    ]
    .into_iter()
    .flatten()
    .collect();

    if sources.len() > 1 {
        bail!(
            "the request declares {} bodies ({}) — keep one",
            sources.len(),
            sources.join(", ")
        );
    }

    let content_type = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone());

    if let Some(text) = section {
        let media_type = front
            .body_type
            .as_deref()
            .map(media_type_of)
            .or(content_type)
            .unwrap_or_else(|| "text/plain".to_string());
        return Ok(Some(Payload::Text {
            text: sub(text),
            media_type,
        }));
    }
    if let Some(fields) = &front.form {
        return Ok(Some(Payload::Form(
            fields.iter().map(|(k, v)| (sub(k), sub(v))).collect(),
        )));
    }
    if let Some(fields) = &front.form_data {
        return Ok(Some(Payload::Multipart(
            fields.iter().map(|(k, v)| (sub(k), sub(v))).collect(),
        )));
    }
    if let Some(path) = &front.file {
        return Ok(Some(Payload::File {
            path: sub(path),
            media_type: front
                .body_type
                .as_deref()
                .map(media_type_of)
                .or(content_type)
                .unwrap_or_else(|| "application/octet-stream".to_string()),
        }));
    }
    Ok(None)
}

/// `body_type: json` → `application/json`. Anything already containing a slash is used
/// verbatim, so an unusual media type is never mangled.
pub fn media_type_of(short: &str) -> String {
    match short.trim().to_ascii_lowercase().as_str() {
        "json" => "application/json",
        "text" | "plain" => "text/plain",
        "html" => "text/html",
        "xml" => "application/xml",
        "js" | "javascript" => "application/javascript",
        "form" | "urlencoded" => "application/x-www-form-urlencoded",
        "graphql" => "application/json",
        "binary" => "application/octet-stream",
        other => {
            return if other.contains('/') {
                other.to_string()
            } else {
                format!("application/{other}")
            }
        }
    }
    .to_string()
}

/// The context a `-- view --` template and every `capture:` path read.
///
/// `response` is the parsed body, because that is what a template iterates. Status,
/// headers and the raw text sit alongside it.
pub fn context_of(req: &Prepared, resp: &Response, v: &Vars) -> serde_json::Value {
    let body = match resp.json() {
        Some(json) => json,
        None => serde_json::Value::String(resp.body.clone()),
    };
    let headers: serde_json::Map<String, serde_json::Value> = resp
        .headers
        .iter()
        .map(|(k, val)| {
            (
                k.to_ascii_lowercase(),
                serde_json::Value::String(val.clone()),
            )
        })
        .collect();
    let vars: serde_json::Map<String, serde_json::Value> = v
        .iter()
        .map(|(k, val)| (k.clone(), serde_json::Value::String(val.clone())))
        .collect();

    serde_json::json!({
        "response": body,
        "status": resp.status,
        "status_text": resp.status_text,
        "headers": headers,
        "body": resp.body,
        "time_ms": resp.elapsed.as_millis() as u64,
        "bytes": resp.bytes,
        "vars": vars,
        "request": {
            "method": req.method,
            "url": req.url,
        },
    })
}

/// A starter document for `rq init`-style creation.
pub fn scaffold(url: &str, method: &str) -> Document {
    let mut doc = Document::default();
    doc.front.method = Some(method.to_string());
    doc.front.url = Some(url.to_string());
    doc
}

pub fn request_path(project: &Project, rel: &str) -> std::path::PathBuf {
    project
        .root
        .join(crate::project::APIS_DIR)
        .join(rel)
        .join(REQUEST_FILE)
}

/// Standard base64. Twenty lines beats a dependency for one Authorization header.
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prep(src: &str, pairs: &[(&str, &str)]) -> (Prepared, Vec<String>) {
        let (doc, _) = Document::parse(src).unwrap();
        let mut v = Vars::new();
        v.layer("test", pairs.iter().map(|(k, val)| (*k, *val)));
        let mut notes = Vec::new();
        let prepared = prepare(&doc, &Inherited::default(), &v, &mut notes).unwrap();
        (prepared, notes)
    }

    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
    }

    #[test]
    fn substitutes_url_query_and_headers() {
        let src = "---\nurl: https://api.test/{{owner}}/x\nquery:\n  state: open\n\
                   headers:\n  Authorization: Bearer {{token}}\n---\n";
        let (p, notes) = prep(src, &[("owner", "acme"), ("token", "t0k")]);
        assert_eq!(p.url, "https://api.test/acme/x?state=open");
        assert_eq!(p.headers[0], ("Authorization".into(), "Bearer t0k".into()));
        assert!(notes.is_empty());
    }

    #[test]
    fn an_unresolved_variable_is_noted_and_left_intact() {
        let (p, notes) = prep("---\nurl: https://api.test/{{owner}}\n---\n", &[]);
        assert_eq!(p.url, "https://api.test/{{owner}}");
        assert!(notes[0].contains("owner"), "{notes:?}");
    }

    #[test]
    fn basic_auth_becomes_an_authorization_header() {
        let src = "---\nurl: https://api.test\nauth: { type: basic, username: u, password: '{{pw}}' }\n---\n";
        let (p, _) = prep(src, &[("pw", "pass")]);
        assert_eq!(
            p.headers[0],
            ("Authorization".into(), "Basic dTpwYXNz".into())
        );
    }

    #[test]
    fn a_bare_bearer_token_omits_the_prefix() {
        let src =
            "---\nurl: https://api.test\nauth: { type: bearer, token: t, prefix: null }\n---\n";
        let (p, _) = prep(src, &[]);
        assert_eq!(p.headers[0], ("Authorization".into(), "t".into()));
    }

    #[test]
    fn an_explicit_header_beats_generated_auth() {
        let src = "---\nurl: https://api.test\nheaders:\n  Authorization: mine\n\
                   auth: { type: bearer, token: t }\n---\n";
        let (p, _) = prep(src, &[]);
        assert_eq!(p.headers.len(), 1);
        assert_eq!(p.headers[0].1, "mine");
    }

    #[test]
    fn api_key_in_query_lands_on_the_url() {
        let src = "---\nurl: https://api.test\nauth: { type: api_key, key: k, value: v, in: query }\n---\n";
        let (p, _) = prep(src, &[]);
        assert_eq!(p.url, "https://api.test?k=v");
    }

    #[test]
    fn body_section_media_type_follows_the_content_type_header() {
        let src = "---\nurl: https://api.test\nmethod: POST\nheaders:\n  Content-Type: application/json\n---\n\n-- body --\n\n{\"a\": {{n}}}\n";
        let (p, _) = prep(src, &[("n", "1")]);
        match p.body.unwrap() {
            Payload::Text { text, media_type } => {
                assert_eq!(text, "{\"a\": 1}");
                assert_eq!(media_type, "application/json");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn two_bodies_are_an_error() {
        let src = "---\nurl: https://api.test\nform:\n  a: 1\n---\n\n-- body --\n\nx\n";
        let (doc, _) = Document::parse(src).unwrap();
        let err = prepare(&doc, &Inherited::default(), &Vars::new(), &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("declares 2 bodies"), "{err}");
    }

    #[test]
    fn media_type_shorthands_and_passthrough() {
        assert_eq!(media_type_of("json"), "application/json");
        assert_eq!(media_type_of("text/csv"), "text/csv");
        assert_eq!(media_type_of("vnd.api+json"), "application/vnd.api+json");
    }
}
