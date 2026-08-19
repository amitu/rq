//! Running a request: resolve the graph, inherit from the collections above it, substitute
//! variables, send, capture, render.

use anyhow::{bail, Context, Result};

use crate::cookies::Jar;
use crate::doc::{AuthSpec, Document, VarSpec};
use crate::graph;
use crate::http::{self, Payload, Prepared, Response};
use crate::project::Project;
use crate::render;
use crate::script::{
    self, LogEntry, RequestHeaderMutation, ScriptEngine, ScriptExecutionResult, ScriptPhase,
    TestResult, TestStatus,
};
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
    /// Per-script wall clock handed to the engine. `None` = the engine's own default.
    pub script_timeout_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub rel: String,
    pub name: String,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
    pub body: Option<String>,
    /// The request body as text, for the console's request pane. `None` when there is no
    /// body, or when it is a file whose bytes belong on the wire and not on your screen.
    pub request_body: Option<String>,
    /// `None` when a pre-request script called `rq.execution.skipRequest()` — the step ran,
    /// the request deliberately did not.
    pub response: Option<Response>,
    pub captured: Vec<(String, String)>,
    /// `rq.test(...)` outcomes, in the order the script declared them.
    pub tests: Vec<TestResult>,
    /// `console.*` from the scripts on this step.
    pub logs: Vec<LogEntry>,
    pub notes: Vec<String>,
}

impl Step {
    pub fn skipped(&self) -> bool {
        self.response.is_none()
    }

    pub fn failed_tests(&self) -> usize {
        self.tests
            .iter()
            .filter(|t| t.status == TestStatus::Failed)
            .count()
    }
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
    /// The links the rendered view offers — what `--follow` and the console navigate by.
    pub fn links(&self) -> Vec<crate::render::Link> {
        self.view
            .as_deref()
            .map(|view| crate::render::markdown(view).links)
            .unwrap_or_default()
    }

    pub fn target(&self) -> &Step {
        self.steps.last().expect("a run always has a final step")
    }

    /// The requested step didn't come back 2xx. Only `--fail` turns this into an exit code:
    /// a 404 you asked for is a result, not an error.
    pub fn failed(&self) -> bool {
        self.target().response.as_ref().is_none_or(|r| !r.ok())
    }

    /// Failing `rq.test(...)` assertions across every step. **These do set the exit code**,
    /// without a flag: an assertion that fails silently in CI is the whole reason people
    /// stop trusting a test runner.
    pub fn failed_tests(&self) -> usize {
        self.steps.iter().map(Step::failed_tests).sum()
    }

    pub fn total_tests(&self) -> usize {
        self.steps.iter().map(|s| s.tests.len()).sum()
    }
}

/// Run `target` and everything it declares as a parent, hosting `engine` for any
/// `-- pre --` / `-- post --` script along the way.
pub fn run(
    project: &Project,
    target: usize,
    opts: &RunOptions,
    engine: &dyn ScriptEngine,
) -> Result<Run> {
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
    // An environment's variables are values, not prompts: take their declared defaults
    // directly rather than asking for them.
    for (key, spec) in env_declared.iter() {
        if let Some(value) = env_value(spec) {
            env_layer.push((key.clone(), value));
        }
    }
    // The always-on layer: `.env`, the file every project already has one of.
    let dotenv = project.dotenv();
    let env_secrets: Vec<String> = env_declared
        .iter()
        .filter(|(_, s)| s.secret)
        .map(|(k, _)| k.clone())
        .collect();

    let mut runtime: Vec<(String, String)> = Vec::new();
    let mut steps: Vec<Step> = Vec::new();
    let mut secrets: Vec<String> = Vec::new();
    // One jar for the whole run, never written to disk (see `cookies`).
    let mut jar = Jar::new();
    let total_entries = order.len() as u32;

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
        v.layer(".env", dotenv.clone());
        for key in &env_secrets {
            v.mark_secret(key);
        }

        // A `-- form --` field is a declared variable that expects to be typed in, so it
        // goes first: the form is the most specific statement about what this request
        // wants, and it is what the console fills in.
        let mut declared = doc
            .form_vars()
            .map_err(|e| anyhow::anyhow!("{}: {e}", entry.rel))?;
        declared.extend(doc.front.vars.clone());
        declared.extend(inherited.declared.clone());
        vars::resolve_declared(&mut v, &declared, opts.prompt, opts.interactive)
            .with_context(|| entry.rel.clone())?;
        secrets.extend(v.secret_values());

        let meta = script::ExecutionMetadata {
            request_id: entry.rel.clone(),
            request_name: entry.name.clone(),
            iteration: 1,
            iteration_count: 1,
            entry_index: steps.len() as u32,
            total_entries,
            collection_id: project.entries[idx]
                .parent
                .map(|p| project.entries[p].rel.clone()),
        };
        let mut tests: Vec<TestResult> = Vec::new();
        let mut logs: Vec<LogEntry> = Vec::new();

        // --- the pre-request chain ----------------------------------------------------
        //
        // Collection scripts run outermost first, then the request's own (ADR-061's
        // "sandwich": pre-request root→request, post-response request→root). Header
        // mutations accumulate across the chain in call order (ADR-167), the request is
        // re-prepared after every script so a variable one sets reaches the next one and
        // the request itself (ADR-020), and a `skipRequest()` aborts the rest of the chain
        // rather than letting later scripts run for a request that will never be sent
        // (ADR-169).
        let mut prepare_notes = Vec::new();
        let mut prepared =
            prepare(&doc, &inherited, &v, &mut prepare_notes).with_context(|| entry.rel.clone())?;
        let mut header_mutations: Vec<RequestHeaderMutation> = Vec::new();
        let mut skip = false;

        for (label, source) in pre_chain(&inherited, &doc, &mut step_notes) {
            let input = script::ScriptExecutionInput {
                script: source,
                phase: ScriptPhase::PreRequest,
                mode: script::ScriptExecutionMode::Safe,
                context: build_context(&prepared, None, &v, &jar, meta.clone()),
                timeout_ms: opts.script_timeout_ms,
            };
            let result = execute(engine, &input, &label);
            if let Some(diff) = &result.request_mutation_diff {
                let (usable, problems) = diff.parse();
                header_mutations.extend(usable);
                step_notes.extend(problems.into_iter().map(|p| format!("{label}: {p}")));
            }
            skip = absorb(
                &result,
                ScriptPhase::PreRequest,
                &label,
                &mut v,
                &mut runtime,
                &mut tests,
                &mut logs,
                &mut step_notes,
            );

            prepare_notes.clear();
            prepared = prepare(&doc, &inherited, &v, &mut prepare_notes)
                .with_context(|| entry.rel.clone())?;
            apply_header_mutations(&mut prepared, &header_mutations);

            if skip {
                break;
            }
        }
        step_notes.append(&mut prepare_notes);
        if !header_mutations.is_empty() {
            step_notes.push(format!(
                "the pre-request chain changed {} header(s)",
                header_mutations.len()
            ));
        }

        http::check_url(&prepared.url).with_context(|| entry.rel.clone())?;

        // Variables a pre-request script set are only visible if we re-resolve what they
        // touched, so re-substitute against the updated runtime layer before sending.
        let response = if skip {
            step_notes.push("the pre-request script called skipRequest(): not sent".into());
            None
        } else {
            if let Some(cookie) = jar.header_for(&prepared.full_url()) {
                if !prepared
                    .headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("cookie"))
                {
                    prepared.headers.push(("Cookie".into(), cookie));
                }
            }
            let response = http::send(&prepared).with_context(|| entry.rel.clone())?;
            jar.ingest(&prepared.full_url(), &response.headers);
            Some(response)
        };

        let ctx = response
            .as_ref()
            .map(|r| context_of(&prepared, r, &v))
            .unwrap_or(serde_json::Value::Null);

        // --- the post-response chain --------------------------------------------------
        // Reversed (ADR-061): the request's own script first, then outward through its
        // collections — so a collection wraps its requests rather than merely preceding
        // them.
        if let Some(resp) = response.as_ref() {
            for (label, source) in post_chain(&inherited, &doc, &mut step_notes) {
                let input = script::ScriptExecutionInput {
                    script: source,
                    phase: ScriptPhase::PostResponse,
                    mode: script::ScriptExecutionMode::Safe,
                    context: build_context(&prepared, Some(resp), &v, &jar, meta.clone()),
                    timeout_ms: opts.script_timeout_ms,
                };
                let result = execute(engine, &input, &label);
                absorb(
                    &result,
                    ScriptPhase::PostResponse,
                    &label,
                    &mut v,
                    &mut runtime,
                    &mut tests,
                    &mut logs,
                    &mut step_notes,
                );
            }
        }

        let mut captured = Vec::new();
        if response.is_some() {
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
        }

        let is_target = idx == target;
        let mut step = Step {
            rel: entry.rel.clone(),
            name: entry.name.clone(),
            method: prepared.method.clone(),
            url: prepared.full_url(),
            request_headers: prepared.headers.clone(),
            body: prepared.body.as_ref().map(|b| b.describe()),
            request_body: prepared.body.as_ref().and_then(|b| b.preview()),
            response,
            captured,
            tests,
            logs,
            notes: step_notes,
        };

        if is_target {
            let view = match doc.section("view").filter(|s| !s.trim().is_empty()) {
                Some(tpl) if !step.skipped() => match render::render_view(tpl, &ctx) {
                    Ok(rendered) => Some(rendered),
                    // A view is written for the shape a *successful* response has. When the
                    // request failed, the template failing too is a consequence, not the
                    // story — say what actually went wrong and show the body, rather than
                    // reporting an undefined field and hiding the 401 that caused it.
                    Err(e) if !step.response.as_ref().is_some_and(|r| r.ok()) => {
                        step.notes.push(format!(
                            "the -- view -- was not rendered because the response was {}: {}",
                            step.response
                                .as_ref()
                                .map(|r| r.status.to_string())
                                .unwrap_or_else(|| "missing".into()),
                            e
                        ));
                        None
                    }
                    Err(e) => return Err(e),
                },
                _ => None,
            };
            let raw = match &step.response {
                Some(r) => render::default_body(&r.body, r.json().as_ref()),
                None => String::new(),
            };
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
    /// Each enclosing collection's own scripts, **outermost first**.
    pub scripts: Vec<CollectionScripts>,
    pub declared: Vec<(String, VarSpec)>,
    pub auth: Option<AuthSpec>,
    pub timeout: Option<u64>,
    pub follow_redirects: Option<bool>,
    pub verify_tls: Option<bool>,
}

/// One collection's `-- pre --` / `-- post --`, with the name to report them under.
#[derive(Clone, Debug)]
pub struct CollectionScripts {
    pub label: String,
    pub pre: Option<String>,
    pub post: Option<String>,
}

/// The pre-request chain: every enclosing collection outermost first, then the request.
fn pre_chain(
    inherited: &Inherited,
    doc: &Document,
    notes: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut chain: Vec<(String, String)> = inherited
        .scripts
        .iter()
        .filter_map(|c| c.pre.clone().map(|s| (c.label.clone(), s)))
        .collect();
    if let Some(own) = reconcile(doc, "pre", notes) {
        chain.push(("this request".to_string(), own));
    }
    chain
}

/// The post-response chain: the request first, then outward — ADR-061's sandwich, so a
/// collection's script *wraps* its requests instead of merely preceding them.
fn post_chain(
    inherited: &Inherited,
    doc: &Document,
    notes: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut chain: Vec<(String, String)> = Vec::new();
    if let Some(own) = reconcile(doc, "post", notes) {
        chain.push(("this request".to_string(), own));
    }
    chain.extend(
        inherited
            .scripts
            .iter()
            .rev()
            .filter_map(|c| c.post.clone().map(|s| (c.label.clone(), s))),
    );
    chain
}

fn section(doc: &Document, name: &str) -> Option<String> {
    doc.section(name)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

/// A script section, reconciled to the `rq.*` API the runtime speaks.
///
/// A collection converted from Postman carries its scripts **verbatim** with the dialect
/// recorded — renaming someone's code textually imports clean and throws later. So the rename
/// happens here instead, on the way to the engine, every run, by the same transform the app
/// uses (`cq-transform`, OXC-based: it parses the script and rewrites identifiers in scope
/// rather than string-replacing `pm.` and hoping).
///
/// `pm.*`, the legacy `postman.setEnvironmentVariable(…)`, and v1's `tests['x'] = expr` /
/// `responseCode` / `responseBody` all come out as `rq.*`. A dialect with no transform (Bruno
/// today) is left alone and says so, which is better than a half-translation.
fn reconcile(doc: &Document, name: &str, notes: &mut Vec<String>) -> Option<String> {
    let source = section(doc, name)?;
    let dialect = doc
        .front
        .script_dialect
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .unwrap_or("rq")
        .to_ascii_lowercase();

    match dialect.as_str() {
        "rq" | "raw" => Some(source),
        "pm" | "postman" => {
            let result =
                cq_transform::full_transform(&source, cq_transform::types::Platform::Postman);
            if !result.success {
                notes.push(format!(
                    "`-- {name} --` is a {dialect} script and could not be reconciled to rq.*; \
                     it ran as written"
                ));
                return Some(source);
            }
            if result.summary.errors > 0 {
                for d in result
                    .diagnostics
                    .iter()
                    .filter(|d| matches!(d.kind, cq_transform::types::DiagnosticKind::Error))
                {
                    notes.push(format!("`-- {name} --`: {}", d.message));
                }
            }
            Some(result.code)
        }
        // Bruno needs no rewrite: the runtime speaks `bru`/`req`/`res` natively, because its
        // API is objects and calls all the way down. Postman is rewritten instead because its
        // v1 forms are syntax — `tests['ok'] = expr` is an assignment no object can intercept.
        "bru" | "bruno" => Some(source),
        other => {
            notes.push(format!(
                "`-- {name} --` is written in the `{other}` dialect, which rq cannot translate \
                 yet — it ran as written, and will fail if it uses `{other}.*`"
            ));
            Some(source)
        }
    }
}

impl Inherited {
    fn gather(project: &Project, idx: usize, notes: &mut Vec<String>) -> Result<Inherited> {
        let mut out = Inherited::default();

        // The project-wide collection first: it is the outermost thing there is.
        let root = project
            .root_collection()?
            .map(|(doc, notes)| (String::from("apis"), doc, notes));
        let ancestors = project.ancestors(idx).into_iter().map(|anc| {
            let rel = project.entries[anc].rel.clone();
            (anc, rel)
        });

        // Outermost first, so a nearer collection's header overwrites a farther one's.
        for (doc, doc_notes, rel) in root
            .into_iter()
            .map(|(rel, doc, notes)| (doc, notes, rel))
            .chain(ancestors.filter_map(|(anc, rel)| {
                project
                    .load_collection(anc)
                    .ok()
                    .flatten()
                    .map(|(doc, notes)| (doc, notes, rel))
            }))
        {
            let rel = &rel;
            notes.extend(doc_notes.into_iter().map(|n| format!("{rel}: {n}")));
            for (k, v) in &doc.front.headers {
                out.headers.retain(|(ek, _)| !ek.eq_ignore_ascii_case(k));
                out.headers.push((k.clone(), v.clone()));
            }
            match &doc.front.auth {
                Some(AuthSpec::Inherit) | None => {}
                Some(a) => out.auth = Some(a.clone()),
            }
            let (pre, post) = (
                reconcile(&doc, "pre", notes),
                reconcile(&doc, "post", notes),
            );
            if pre.is_some() || post.is_some() {
                out.scripts.push(CollectionScripts {
                    label: rel.clone(),
                    pre,
                    post,
                });
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
            "`{{{{{m}}}}}` has no value; it was left as written"
        ));
    }

    Ok(Prepared {
        method: front.method.clone().unwrap_or_else(|| "GET".into()),
        url,
        query,
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
            let (username, password) = (sub(username), sub(password));
            if usable(&username) || usable(&password) {
                let token = base64(format!("{username}:{password}").as_bytes());
                set_header("Authorization", format!("Basic {token}"));
            } else {
                notes.push(unset("basic", "username and password"));
            }
        }
        AuthSpec::Bearer { token, prefix } => {
            let token = sub(token);
            if !usable(&token) {
                notes.push(unset("bearer", "token"));
                return;
            }
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
            if !usable(&value) {
                notes.push(unset("api_key", "value"));
                return;
            }
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

/// Is this credential worth sending? An empty one is not, and neither is one that is still
/// a `{{template}}` — a collection can declare `Bearer {{GH_TOKEN}}` for everyone and stay
/// usable by someone who hasn't set a token, instead of turning every public request into
/// a 401.
fn usable(credential: &str) -> bool {
    !credential.trim().is_empty() && !credential.contains("{{")
}

fn unset(kind: &str, what: &str) -> String {
    format!("auth `{kind}` was not sent: its {what} resolved to nothing")
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

// ---------------------------------------------------------------------------------------
// The script seam
// ---------------------------------------------------------------------------------------

/// Build the serializable context a script runs against. `request` and `response` are
/// shaped to cross-q-context's `model.ts`, and the variable scopes are bucketed by where
/// each value actually came from — the origins `Vars` already tracks — so `rq.environment`
/// and `rq.variables` mean in the CLI what they mean in the app.
pub fn build_context(
    req: &Prepared,
    resp: Option<&Response>,
    v: &Vars,
    jar: &Jar,
    info: script::ExecutionMetadata,
) -> script::ScriptExecutionContext {
    let mut ctx = script::ScriptExecutionContext {
        info,
        request: request_json(req),
        response: resp.map(response_json),
        host_allowlist: jar.hosts(),
        cookie_jar_seed: jar
            .hosts()
            .into_iter()
            .map(|host| script::CookieJarSeed {
                cookies: jar.seed_for(&host),
                host,
            })
            .collect(),
        ..script::ScriptExecutionContext::default()
    };

    for (key, value) in v.iter() {
        let secret = v.is_secret(key);
        let data = serde_json::to_value(script::VariableData::new(value.clone(), secret))
            .unwrap_or(serde_json::Value::Null);
        let scope = match v.origin(key).unwrap_or("") {
            "__global" | "global" => &mut ctx.global,
            o if o.starts_with("collection") => &mut ctx.collection_variables,
            "--var" | "capture" | "prompt" | "script" => &mut ctx.variables,
            "default" => &mut ctx.variables,
            _ => &mut ctx.environment,
        };
        scope.insert(key.clone(), data);
        if secret {
            ctx.secrets.insert(
                key.clone(),
                serde_json::to_value(script::VariableData::new(value.clone(), true))
                    .unwrap_or(serde_json::Value::Null),
            );
        }
    }
    ctx
}

fn request_json(req: &Prepared) -> serde_json::Value {
    let kv = |pairs: &[(String, String)]| -> Vec<serde_json::Value> {
        pairs
            .iter()
            .map(|(k, val)| serde_json::json!({ "key": k, "value": val }))
            .collect()
    };
    let (content_type, body) = match &req.body {
        Some(Payload::Text { text, media_type }) => (
            media_type.clone(),
            serde_json::json!({
                "contentType": media_type,
                "raw": text,
                "rawContentType": media_type,
                "formUrlEncoded": [],
                "formData": [],
            }),
        ),
        Some(Payload::Form(fields)) => (
            "application/x-www-form-urlencoded".to_string(),
            serde_json::json!({
                "contentType": "application/x-www-form-urlencoded",
                "formUrlEncoded": kv(fields),
                "formData": [],
            }),
        ),
        Some(Payload::Multipart(fields)) => (
            "multipart/form-data".to_string(),
            serde_json::json!({
                "contentType": "multipart/form-data",
                "formUrlEncoded": [],
                "formData": fields
                    .iter()
                    .map(|(k, val)| serde_json::json!({ "key": k, "value": val, "type": "text" }))
                    .collect::<Vec<_>>(),
            }),
        ),
        Some(Payload::File { path, media_type }) => (
            media_type.clone(),
            serde_json::json!({
                "contentType": media_type,
                "binary": { "name": path, "path": path },
                "formUrlEncoded": [],
                "formData": [],
            }),
        ),
        None => (
            "none".to_string(),
            serde_json::json!({ "contentType": "none", "formUrlEncoded": [], "formData": [] }),
        ),
    };

    serde_json::json!({
        "url": req.full_url(),
        "method": req.method,
        "headers": kv(&req.headers),
        "queryParams": kv(&req.query),
        "pathVariables": [],
        "contentType": content_type,
        "body": body,
    })
}

fn response_json(resp: &Response) -> serde_json::Value {
    let headers: serde_json::Map<String, serde_json::Value> = resp
        .headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), serde_json::Value::String(v.clone())))
        .collect();
    serde_json::json!({
        "status": resp.status,
        "statusText": resp.status_text,
        "headers": headers,
        "body": resp.body,
        "time": resp.elapsed.as_millis() as u64,
        "size": resp.bytes,
    })
}

/// Replay the accumulated `rq.request.headers.*` changes onto a freshly prepared request.
/// Order matters — the diff is a log of what the chain did, replayed in the same sequence.
fn apply_header_mutations(prepared: &mut Prepared, mutations: &[RequestHeaderMutation]) {
    for mutation in mutations {
        match mutation {
            RequestHeaderMutation::Add { name, value } => {
                prepared.headers.push((name.clone(), value.clone()));
            }
            RequestHeaderMutation::Upsert { name, value } => {
                prepared
                    .headers
                    .retain(|(k, _)| !k.eq_ignore_ascii_case(name));
                prepared.headers.push((name.clone(), value.clone()));
            }
            RequestHeaderMutation::Remove { name } => {
                prepared
                    .headers
                    .retain(|(k, _)| !k.eq_ignore_ascii_case(name));
            }
            RequestHeaderMutation::Clear => prepared.headers.clear(),
        }
    }
}

/// Fold one script result into the run: variables into the same runtime layer `capture:`
/// writes to, tests and logs onto the step, everything else onto the record. Returns
/// whether the script asked to skip the request.
#[allow(clippy::too_many_arguments)]
fn absorb(
    result: &ScriptExecutionResult,
    phase: ScriptPhase,
    label: &str,
    v: &mut Vars,
    runtime: &mut Vec<(String, String)>,
    tests: &mut Vec<TestResult>,
    logs: &mut Vec<LogEntry>,
    notes: &mut Vec<String>,
) -> bool {
    for (key, value) in result.mutation_diff.all() {
        runtime.retain(|(k, _)| *k != key);
        // `None` is an unset: dropping it above was the whole job.
        if let Some(value) = value {
            runtime.push((key.clone(), value.clone()));
            // Also into this step's own set, so the next script in the chain sees it and
            // the re-prepared request substitutes it.
            v.set(key, value, "script");
        }
    }
    tests.extend(result.test_results.iter().cloned());
    logs.extend(result.logs.iter().cloned());

    if let Some(error) = &result.error {
        notes.push(format!("{label}: {error}"));
    }
    match &result.execution_directive {
        Some(script::ExecutionDirective::SkipRequest) => {
            return matches!(phase, ScriptPhase::PreRequest)
        }
        Some(script::ExecutionDirective::SetNextRequest { target }) => notes.push(format!(
            "setNextRequest({}) was ignored: rq walks the graph a request declares with \
             `parents:`, so there is no linear run order to redirect",
            target.as_deref().unwrap_or("null")
        )),
        None => {}
    }
    false
}

/// Run one script, turning a broken *engine* into a reported non-execution rather than a
/// failed run.
///
/// The request itself worked; only the script didn't. Losing the response — and the rest of
/// the chain — because the engine could not be loaded would be a worse answer than saying
/// so and carrying on. `--strict` still turns it into a failure for anyone who wants that.
fn execute(
    engine: &dyn ScriptEngine,
    input: &script::ScriptExecutionInput,
    _label: &str,
) -> ScriptExecutionResult {
    match engine.execute(input) {
        Ok(result) => result,
        Err(e) => ScriptExecutionResult {
            error: Some(format!(
                "`-- {} --` was NOT executed: {e:#}",
                input.phase.section()
            )),
            ..ScriptExecutionResult::default()
        },
    }
}

/// The variables that exist before any request is prepared: the command line, the active
/// environment, and `.env`.
///
/// A form's `default: '{{me}}'` is resolved against these — the value has to come from
/// somewhere, and "somewhere" is whatever the project already knows before you type.
pub fn ambient_vars(project: &Project, opts: &RunOptions) -> Vars {
    let mut vars = Vars::new();
    vars.layer("--var", opts.cli_vars.clone());

    let env_name = opts
        .environment
        .clone()
        .or_else(|| project.active_env())
        .filter(|n| !n.is_empty());
    if let Some(name) = &env_name {
        if let Ok((doc, _)) = project.load_env(name) {
            let values: Vec<(String, String)> = doc
                .front
                .vars
                .iter()
                .filter_map(|(key, spec)| env_value(spec).map(|v| (key.clone(), v)))
                .collect();
            for (key, spec) in &doc.front.vars {
                if spec.secret {
                    vars.mark_secret(key);
                }
            }
            vars.layer(name, values);
        }
    }
    vars.layer(".env", project.dotenv());
    vars
}

/// Follow a link out of a finished run: resolve `name?a=b` against the project and run it,
/// with the link's own variables layered on top of the ones the run already had.
///
/// A link is navigation, not a new session — the environment, the CLI variables and the
/// engine all carry over, so following `[#1287](rq:issue?number=1287)` differs from the
/// page you were on by exactly the thing the link said.
pub fn follow(
    project: &Project,
    link: &crate::render::Link,
    opts: &RunOptions,
    engine: &dyn ScriptEngine,
) -> Result<Run> {
    let (name, vars) = crate::render::parse_target(&link.target);
    let target = project
        .resolve(&name)
        .with_context(|| format!("link [{}] → {}", link.number, link.target))?;

    let mut opts = opts.clone();
    for (key, value) in vars {
        opts.cli_vars.retain(|(k, _)| *k != key);
        opts.cli_vars.push((key, value));
    }
    run(project, target, &opts, engine)
}

/// A starter document for `rq init`-style creation.
pub fn scaffold(url: &str, method: &str) -> Document {
    let mut doc = Document::default();
    doc.front.method = Some(method.to_string());
    doc.front.url = Some(url.to_string());
    doc
}

pub fn request_path(project: &Project, rel: &str) -> std::path::PathBuf {
    project.root.join(rq_doc::layout::request_path(rel))
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
        assert_eq!(p.full_url(), "https://api.test/acme/x?state=open");
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
        assert_eq!(p.full_url(), "https://api.test?k=v");
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
