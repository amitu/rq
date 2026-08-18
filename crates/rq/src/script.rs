//! The script host boundary — the Rust side of the `cross-q-context` contract.
//!
//! `rq` does not execute JavaScript. It **hosts** an engine that does: it builds the
//! serializable input a script runs against, hands it to a [`ScriptEngine`], and applies
//! what comes back. The types here mirror
//! `packages/cross-q-context/src/runtime/{contract,execution,model}.ts` field for field, so
//! the two sides marshal the same JSON.
//!
//! Nothing here evaluates code. Today the only implementation is [`NoEngine`], which
//! reports that a script was not run — loudly, on every run that has one, rather than
//! quietly doing nothing. When cross-q-context ships an engine, implementing this one trait
//! is the whole integration.
//!
//! Field names are `camelCase` on the wire because that is what the TypeScript contract
//! uses; Rust keeps its own spelling and lets serde bridge the two.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// ---------------------------------------------------------------------------------------
// Primitives (contract.ts)
// ---------------------------------------------------------------------------------------

/// Which phase is running. `rq.response` is absent in `pre-request`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptPhase {
    #[serde(rename = "pre-request")]
    PreRequest,
    #[serde(rename = "post-response")]
    PostResponse,
    #[serde(rename = "on-message")]
    OnMessage,
}

impl ScriptPhase {
    /// The document section this phase reads.
    pub fn section(&self) -> &'static str {
        match self {
            ScriptPhase::PreRequest => "pre",
            ScriptPhase::PostResponse => "post",
            ScriptPhase::OnMessage => "message",
        }
    }
}

/// The engine's isolation mode. The CLI is **safe only**: `developer` is `node:vm`, which is
/// not a security boundary, and a terminal client that ran untrusted collection scripts with
/// host access would be a liability rather than a feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScriptExecutionMode {
    Safe,
    Developer,
}

/// Immutable metadata about the run, surfaced to the script as `rq.info`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionMetadata {
    pub request_id: String,
    pub request_name: String,
    pub iteration: u32,
    pub iteration_count: u32,
    pub entry_index: u32,
    pub total_entries: u32,
    pub collection_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
}

/// The outcome of one `rq.test(...)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub status: TestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A `console.*` line captured during execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub level: String,
    #[serde(default)]
    pub args: Vec<Value>,
}

impl LogEntry {
    /// The line as a person would read it: JSON strings unquoted, everything else compact.
    pub fn message(&self) -> String {
        self.args
            .iter()
            .map(|a| match a {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A variable the engine set, in the shape it inflates them to.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutatedVariable {
    #[serde(default)]
    pub local_value: String,
    #[serde(default)]
    pub sync_value: String,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

impl MutatedVariable {
    /// The value to carry forward: the working copy, or the persisted one when there is no
    /// working copy.
    pub fn value(&self) -> &str {
        if self.local_value.is_empty() {
            &self.sync_value
        } else {
            &self.local_value
        }
    }
}

/// Collection-scope mutations, tagged with the collection they belong to.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionMutation {
    #[serde(default)]
    pub collection_id: String,
    #[serde(default)]
    pub variables: Map<String, Value>,
}

/// Net variable changes per scope: a value for a set, `null` for an unset.
///
/// This mirrors the *inflated* host-facing `MutationDiff` exported from
/// `@requestly/cross-q-context/runtime` — `global`/`runtime` scopes, values as `VariableData`.
/// The guest-side shape (`globals`/`variables`, raw JSON) is a different type, named
/// `RawMutationDiff`; a host that mirrored it would read an empty `variables` map and
/// silently drop every `rq.variables.set`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationDiff {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<CollectionMutation>,
    /// What `rq.variables.set(…)` writes — the run's own scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault: Option<Map<String, Value>>,
}

impl MutationDiff {
    /// Every scope's changes, widest first so the narrowest wins.
    pub fn all(&self) -> Vec<(String, Option<String>)> {
        let scopes = [
            self.global.as_ref(),
            self.collection.as_ref().map(|c| &c.variables),
            self.environment.as_ref(),
            self.runtime.as_ref(),
        ];
        let mut out = Vec::new();
        for scope in scopes.into_iter().flatten() {
            for (key, value) in scope {
                out.push((key.clone(), read_value(value)));
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.all().is_empty()
    }
}

/// A mutation is an inflated variable, or a bare scalar, or `null` for an unset. Accepting
/// all three keeps this working whichever shape the engine settles on.
fn read_value(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Object(_) => serde_json::from_value::<MutatedVariable>(value.clone())
            .ok()
            .map(|v| v.value().to_string()),
        other => Some(other.to_string()),
    }
}

/// A recorded change to the outgoing request's headers (`rq.request.headers.*`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RequestHeaderMutation {
    Add { name: String, value: String },
    Upsert { name: String, value: String },
    Remove { name: String },
    Clear,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RequestMutationDiff {
    /// Kept raw so one entry we can't read costs only that entry. A script that set three
    /// headers and mistyped one should still send the other two, and hear about the third.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<Value>,
}

impl RequestMutationDiff {
    /// The mutations we can act on, and a note for each we can't.
    pub fn parse(&self) -> (Vec<RequestHeaderMutation>, Vec<String>) {
        let mut usable = Vec::new();
        let mut problems = Vec::new();
        for raw in &self.headers {
            match serde_json::from_value::<RequestHeaderMutation>(raw.clone()) {
                Ok(mutation) => usable.push(mutation),
                Err(e) => {
                    problems.push(format!("a header change could not be applied ({e}): {raw}"))
                }
            }
        }
        (usable, problems)
    }
}

/// A chaining directive drained from the run.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ExecutionDirective {
    SetNextRequest { target: Option<String> },
    SkipRequest,
}

/// The result of one `execute` call.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptExecutionResult {
    #[serde(default)]
    pub mutation_diff: MutationDiff,
    #[serde(default)]
    pub logs: Vec<LogEntry>,
    #[serde(default)]
    pub test_results: Vec<TestResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_mutation_diff: Option<RequestMutationDiff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_directive: Option<ExecutionDirective>,
    /// A thrown error. The script did not finish; nothing after the throw ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------------------
// The execution input (execution.ts + model.ts)
// ---------------------------------------------------------------------------------------

/// A resolved variable at the runtime boundary — the app's `VariableData`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableData {
    pub local_value: String,
    pub sync_value: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_enabled: Option<bool>,
}

impl VariableData {
    pub fn new(value: impl Into<String>, secret: bool) -> Self {
        let value = value.into();
        Self {
            local_value: value.clone(),
            sync_value: value,
            kind: if secret { "secret" } else { "string" }.to_string(),
            is_enabled: Some(true),
        }
    }
}

pub type EnvironmentVariables = Map<String, Value>;

/// Read-side cookie seed for `rq.cookies.jar(host)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CookieJarSeed {
    pub host: String,
    pub cookies: Vec<Value>,
}

/// The serializable context handed to the guest. `request` and `response` are carried as
/// JSON built to `model.ts`'s shapes rather than re-declared here — the model is the app's
/// schema, and duplicating it in Rust would be a second source of truth to keep in step.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptExecutionContext {
    pub global: EnvironmentVariables,
    pub collection_variables: EnvironmentVariables,
    pub environment: EnvironmentVariables,
    pub variables: EnvironmentVariables,
    pub iteration_data: EnvironmentVariables,
    pub secrets: EnvironmentVariables,
    pub request: Value,
    pub response: Option<Value>,
    pub info: ExecutionMetadata,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub location: Vec<String>,
    pub host_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cookie_jar_seed: Vec<CookieJarSeed>,
}

/// One `execute` call's input.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptExecutionInput {
    pub script: String,
    pub phase: ScriptPhase,
    pub mode: ScriptExecutionMode,
    pub context: ScriptExecutionContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

// ---------------------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------------------

/// A script engine. One method, one serializable value in, one out — the same call every
/// host of cross-q-context makes.
pub trait ScriptEngine {
    /// What to call this engine in diagnostics.
    fn name(&self) -> &str;

    /// Run one script. `Err` is a *host* failure (the engine could not be reached); a script
    /// that threw comes back as `Ok` with `error` set, because that is a result, not a crash.
    fn execute(&self, input: &ScriptExecutionInput) -> anyhow::Result<ScriptExecutionResult>;
}

/// The engine this build ships with: none.
///
/// It exists so the seam is real rather than hypothetical — every script in a run goes
/// through the same path, and this one reports that it was not executed. `--strict` turns
/// that report into a failure. Silently skipping a pre-request script would send a request
/// the author never described.
#[derive(Default)]
pub struct NoEngine {
    /// Why there is no engine, so the run can say something useful instead of "no runtime".
    why: Option<String>,
}

impl NoEngine {
    pub fn because(why: impl Into<String>) -> NoEngine {
        NoEngine {
            why: Some(why.into()),
        }
    }
}

impl ScriptEngine for NoEngine {
    fn name(&self) -> &str {
        "none"
    }

    fn execute(&self, input: &ScriptExecutionInput) -> anyhow::Result<ScriptExecutionResult> {
        Ok(ScriptExecutionResult {
            error: Some(format!(
                "`-- {} --` was NOT executed: {}",
                input.phase.section(),
                self.why.as_deref().unwrap_or("there is no script engine")
            )),
            ..ScriptExecutionResult::default()
        })
    }
}

// ---------------------------------------------------------------------------------------
// The engine rq actually ships with
// ---------------------------------------------------------------------------------------

/// Runs scripts by handing them to cross-q-context, over a pipe.
///
/// The engine is JavaScript driving QuickJS-on-WASM; `rq` is a Rust binary. Rather than
/// reimplement the `rq.*` semantics in a second runtime — the drift this whole project
/// exists to avoid — the CLI runs the real engine in `node` and speaks the wire contract to
/// it. One process per script: scripts are short, and a fresh isolate per run is the
/// isolation story anyway.
///
/// The cost is honest and stated: scripts need `node` and a built cross-q-context. Without
/// them `rq` runs everything else exactly as before and says why the script didn't.
pub struct NodeEngine {
    node: PathBuf,
    runner: PathBuf,
    package: PathBuf,
}

impl NodeEngine {
    /// Find an engine to run, or explain what is missing.
    ///
    /// `RQ_SCRIPT_ENGINE` points at a cross-q-context checkout; otherwise we look beside the
    /// binary and up from the current directory, which covers running out of this repo.
    pub fn discover() -> Result<NodeEngine, String> {
        let node = which("node").ok_or_else(|| {
            "`node` is not on your PATH, and the script engine runs on it".to_string()
        })?;
        let runner = runner_path()?;
        let package = package_path()?;
        Ok(NodeEngine {
            node,
            runner,
            package,
        })
    }

    pub fn package(&self) -> &Path {
        &self.package
    }
}

impl ScriptEngine for NodeEngine {
    fn name(&self) -> &str {
        "cross-q-context"
    }

    fn execute(&self, input: &ScriptExecutionInput) -> anyhow::Result<ScriptExecutionResult> {
        let mut child = Command::new(&self.node)
            .arg(&self.runner)
            .arg(&self.package)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {}", self.node.display()))?;

        let payload = serde_json::to_vec(input)?;
        child
            .stdin
            .take()
            .expect("stdin was piped")
            .write_all(&payload)
            .context("writing the script input to the engine")?;

        let output = child.wait_with_output().context("running the engine")?;
        if !output.status.success() {
            anyhow::bail!(
                "the script engine failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        serde_json::from_slice(&output.stdout).with_context(|| {
            format!(
                "the engine's answer was not a result: {}",
                String::from_utf8_lossy(&output.stdout)
                    .chars()
                    .take(200)
                    .collect::<String>()
            )
        })
    }
}

/// The runner script: beside the installed binary, or in this repo.
fn runner_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("RQ_SCRIPT_RUNNER").map(PathBuf::from) {
        return path
            .is_file()
            .then_some(path.clone())
            .ok_or_else(|| format!("RQ_SCRIPT_RUNNER={} is not a file", path.display()));
    }
    let candidates = [
        // Running from a checkout, in either profile.
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runner/execute.mjs"),
    ];
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| "the script runner (runner/execute.mjs) was not found".to_string())
}

/// A built cross-q-context to run.
fn package_path() -> Result<PathBuf, String> {
    let built = |root: &Path| root.join("dist/runtime/engine/execute.js").is_file();

    if let Some(path) = std::env::var_os("RQ_SCRIPT_ENGINE").map(PathBuf::from) {
        return if built(&path) {
            Ok(path)
        } else {
            Err(format!(
                "RQ_SCRIPT_ENGINE={} has no dist/runtime/engine/execute.js — is it built?",
                path.display()
            ))
        };
    }

    // This repo's own copy, which is what a contributor and the tests use.
    let in_repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/cross-q-context");
    if built(&in_repo) {
        return Ok(in_repo);
    }
    Err("cross-q-context was not found; set RQ_SCRIPT_ENGINE to a built checkout".to_string())
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape is the contract's, not Rust's — if these names drift, the guest reads
    /// undefined and every script silently misbehaves.
    #[test]
    fn the_input_serializes_with_the_contracts_field_names() {
        let input = ScriptExecutionInput {
            script: "rq.test('x', () => true)".into(),
            phase: ScriptPhase::PostResponse,
            mode: ScriptExecutionMode::Safe,
            context: ScriptExecutionContext {
                info: ExecutionMetadata {
                    request_id: "r1".into(),
                    request_name: "login".into(),
                    total_entries: 2,
                    ..ExecutionMetadata::default()
                },
                ..ScriptExecutionContext::default()
            },
            timeout_ms: Some(5000),
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["phase"], "post-response");
        assert_eq!(json["mode"], "safe");
        assert_eq!(json["timeoutMs"], 5000);
        assert_eq!(json["context"]["info"]["requestName"], "login");
        assert_eq!(json["context"]["info"]["totalEntries"], 2);
        assert!(json["context"].get("collectionVariables").is_some());
        assert!(json["context"].get("hostAllowlist").is_some());
    }

    #[test]
    fn a_result_from_the_contracts_json_reads_back() {
        let raw = serde_json::json!({
            "mutationDiff": {
                "environment": { "token": { "localValue": "abc", "syncValue": "" } },
                "runtime": { "gone": null }
            },
            "logs": [{ "level": "log", "args": ["hello", 42] }],
            "testResults": [
                { "name": "200 OK", "status": "passed" },
                { "name": "has token", "status": "failed", "error": "expected undefined" }
            ],
            "requestMutationDiff": { "headers": [
                { "kind": "upsert", "name": "X-Sig", "value": "abc" },
                { "kind": "remove", "name": "X-Debug" }
            ]},
            "executionDirective": { "kind": "skip-request" }
        });
        let result: ScriptExecutionResult = serde_json::from_value(raw).unwrap();
        assert_eq!(result.test_results.len(), 2);
        assert_eq!(result.test_results[1].status, TestStatus::Failed);
        assert_eq!(result.logs[0].message(), "hello 42");
        assert_eq!(
            result.mutation_diff.all().len(),
            2,
            "both scopes' changes are visible"
        );
        assert!(matches!(
            result.execution_directive,
            Some(ExecutionDirective::SkipRequest)
        ));
        let (headers, problems) = result.request_mutation_diff.unwrap().parse();
        assert!(problems.is_empty(), "{problems:?}");
        assert!(
            matches!(&headers[0], RequestHeaderMutation::Upsert { name, .. } if name == "X-Sig")
        );
        assert!(matches!(&headers[1], RequestHeaderMutation::Remove { name } if name == "X-Debug"));
    }

    #[test]
    fn the_shipped_engine_reports_that_it_did_not_run() {
        let input = ScriptExecutionInput {
            script: "anything".into(),
            phase: ScriptPhase::PreRequest,
            mode: ScriptExecutionMode::Safe,
            context: ScriptExecutionContext::default(),
            timeout_ms: None,
        };
        let out = NoEngine::default().execute(&input).unwrap();
        assert!(out.error.unwrap().contains("-- pre --"));
        assert!(out.test_results.is_empty());
    }
}
