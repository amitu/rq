//! The host wire contract (`docs/CONTEXT.md` §6): a pure function of a serializable input to a
//! serializable output, so every host (Rust, WASM/JS, Python) calls the same `execute(input) →
//! result` with no live objects crossing the boundary.
//!
//! This is the initial, honest subset — the scopes, tests, logs, request-header mutations and
//! execution directive that the scaffolded runtime actually produces. Cookies, the vault, gRPC,
//! and `runRequest` are declared in the spec and land as the runtime grows.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which script phase is running. `rq.response` is absent in `pre-request`; `skipRequest()` is
/// pre-request only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    PreRequest,
    PostResponse,
}

/// The sandbox engine. Published (WASM/PyPI) builds are `Safe` only; unrecognized → `Safe`
/// (fail-closed). `Developer` (`node:vm`) is host-embedding only and not offered here yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Safe,
}

/// The serializable script context handed to the guest (JSON-parsed inside the realm). Variable
/// scopes are `key → value` maps; `request`/`response`/`info` are opaque JSON the shim reads.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScriptContext {
    #[serde(default)]
    pub environment: serde_json::Map<String, Value>,
    #[serde(default)]
    pub globals: serde_json::Map<String, Value>,
    #[serde(default, rename = "collectionVariables")]
    pub collection_variables: serde_json::Map<String, Value>,
    #[serde(default)]
    pub variables: serde_json::Map<String, Value>,
    #[serde(default)]
    pub request: Value,
    #[serde(default)]
    pub response: Value,
    #[serde(default)]
    pub info: Value,
}

/// One `execute` call's input (`docs/CONTEXT.md` §6).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptExecutionInput {
    pub script: String,
    pub phase: Phase,
    #[serde(default)]
    pub context: ScriptContext,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default, rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
}

/// A `rq.test(...)` outcome.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub status: TestStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
}

/// A `console.*` line captured during execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub level: String,
    pub args: Vec<Value>,
}

/// The net variable changes per scope — each is `key → new value` for `set`, and `key → null`
/// for `unset`/`clear`. Inflated host-side into a full diff by consumers that need type/secret
/// fidelity; the runtime emits the raw shape.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MutationDiff {
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub environment: serde_json::Map<String, Value>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub globals: serde_json::Map<String, Value>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty", rename = "collection")]
    pub collection: serde_json::Map<String, Value>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty", rename = "runtime")]
    pub variables: serde_json::Map<String, Value>,
}

/// A chaining directive (`setNextRequest` / `skipRequest`) drained from the run.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ExecutionDirective {
    SetNextRequest { target: Option<String> },
    SkipRequest,
}

/// The result of one `execute` call (`docs/CONTEXT.md` §6).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScriptExecutionResult {
    #[serde(rename = "mutationDiff")]
    pub mutation_diff: MutationDiff,
    pub logs: Vec<LogEntry>,
    #[serde(rename = "testResults")]
    pub test_results: Vec<TestResult>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "executionDirective")]
    pub execution_directive: Option<ExecutionDirective>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
