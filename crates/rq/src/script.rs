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

/// Net variable changes per scope — `key → value` for a set, `key → null` for an unset.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationDiff {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub globals: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection_variables: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<Map<String, Value>>,
}

impl MutationDiff {
    /// Every scope's changes, in precedence order (narrowest last, so it wins).
    pub fn all(&self) -> impl Iterator<Item = (&String, &Value)> {
        [
            self.globals.as_ref(),
            self.collection_variables.as_ref(),
            self.environment.as_ref(),
            self.variables.as_ref(),
        ]
        .into_iter()
        .flatten()
        .flat_map(|m| m.iter())
    }

    pub fn is_empty(&self) -> bool {
        self.all().next().is_none()
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<RequestHeaderMutation>,
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
pub struct NoEngine;

impl ScriptEngine for NoEngine {
    fn name(&self) -> &str {
        "none"
    }

    fn execute(&self, input: &ScriptExecutionInput) -> anyhow::Result<ScriptExecutionResult> {
        Ok(ScriptExecutionResult {
            error: Some(format!(
                "`-- {} --` was NOT executed: this build has no script runtime yet",
                input.phase.section()
            )),
            ..ScriptExecutionResult::default()
        })
    }
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
            "mutationDiff": { "environment": { "token": "abc" }, "variables": { "gone": null } },
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
            result.mutation_diff.all().count(),
            2,
            "both scopes' changes are visible"
        );
        assert!(matches!(
            result.execution_directive,
            Some(ExecutionDirective::SkipRequest)
        ));
        let headers = result.request_mutation_diff.unwrap().headers;
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
        let out = NoEngine.execute(&input).unwrap();
        assert!(out.error.unwrap().contains("-- pre --"));
        assert!(out.test_results.is_empty());
    }
}
