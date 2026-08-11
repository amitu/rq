//! `cross-q-context` — the open-source `rq.*` scripting runtime (see `docs/CONTEXT.md`).
//!
//! A QuickJS sandbox that runs pre-request / post-response scripts against the `rq.*` API
//! (backward-compatible with Postman's `pm.*`), as one Rust core that compiles to a native
//! crate, a WASM npm package, and a PyPI wheel — so `rq.*` semantics are defined once and never
//! drift between a browser client, a CLI, and a CI runner.
//!
//! Status: **scaffold**. The engine boundary, the §6 wire contract, and a genuine (if partial)
//! `rq.*` surface work end-to-end — set/read variables across the four scopes, run `rq.test`
//! with a minimal `expect`, capture `console`, read the request/response, and emit a chaining
//! directive; the host gets a serializable [`ScriptExecutionResult`]. Still to come (tracked):
//! the full Chai `expect`, cookies, `sendRequest`, the response `.to` assertion tree, gRPC,
//! `runRequest`, async pumping, the `pm.*`/`bru.*` compat transforms, and WASM/PyPI packaging.

mod wire;

pub use wire::{
    ExecutionDirective, HeaderMutation, LogEntry, Mode, MutationDiff, Phase, ScriptContext,
    ScriptExecutionInput, ScriptExecutionResult, TestResult, TestStatus,
};

use rquickjs::{CatchResultExt, Context, Runtime};
use serde_json::Value;

/// The compat pillar (`docs/CONTEXT.md` §3): the OXC-based AST rewrite of platform script
/// dialects → `rq.*`, re-exported from the `cq-transform` crate. A Postman (`pm.*`/`postman.*`)
/// script is rewritten to `rq.*` **once, at import time**, and stored rewritten — so it runs
/// unchanged on the runtime below. (The Bruno phase lands per `docs/BRUNO-COMPAT.md`.)
pub mod compat {
    pub use cq_transform::full_transform as transform;
    pub use cq_transform::types::{Platform, TransformResult};
}

/// The `rq.*` shim installed into every fresh realm (see `rq_shim.js`).
const RQ_SHIM: &str = include_str!("rq_shim.js");

/// Guest-realm memory ceiling (`docs/CONTEXT.md` §4).
const MEMORY_LIMIT_BYTES: usize = 128 * 1024 * 1024;
/// Interrupt ceiling — the handler fires periodically; abort once it has been polled this many
/// times (a coarse op-count guard; a wall-clock deadline lands with async support).
const INTERRUPT_POLL_LIMIT: u64 = 5_000_000;

/// Run one script and return the serializable result (`docs/CONTEXT.md` §6). Never panics on
/// guest error — a thrown script surfaces as [`ScriptExecutionResult::error`], not a failure of
/// the call itself. Isolation per §4: a fresh runtime + realm, memory-capped, interrupt-guarded,
/// and starting from a bare ES realm (no ambient Node/Web authority).
pub fn execute(input: &ScriptExecutionInput) -> ScriptExecutionResult {
    match run(input) {
        Ok(result) => result,
        // A harness-level failure (couldn't even set up the realm) still returns a result shell
        // carrying the error, so the wire contract holds unconditionally.
        Err(err) => ScriptExecutionResult {
            error: Some(err),
            ..Default::default()
        },
    }
}

fn run(input: &ScriptExecutionInput) -> Result<ScriptExecutionResult, String> {
    let runtime = Runtime::new().map_err(|e| e.to_string())?;
    runtime.set_memory_limit(MEMORY_LIMIT_BYTES);
    let mut polls: u64 = 0;
    runtime.set_interrupt_handler(Some(Box::new(move || {
        polls += 1;
        polls > INTERRUPT_POLL_LIMIT
    })));

    let context = Context::full(&runtime).map_err(|e| e.to_string())?;

    let context_json = serde_json::to_string(&input.context).map_err(|e| e.to_string())?;
    let phase_str = match input.phase {
        Phase::PreRequest => "pre-request",
        Phase::PostResponse => "post-response",
    };

    let mut result = ScriptExecutionResult::default();

    context.with(|ctx| -> Result<(), String> {
        // Marshal context in as a single JSON string (§4).
        let globals = ctx.globals();
        globals
            .set("__RQ_CONTEXT_JSON", context_json)
            .map_err(|e| e.to_string())?;
        globals.set("__RQ_PHASE", phase_str).map_err(|e| e.to_string())?;

        // Install the rq.* namespace + reserved output channels.
        ctx.eval::<(), _>(RQ_SHIM)
            .catch(&ctx)
            .map_err(|e| format!("rq shim install failed: {e}"))?;

        // Run the user script wrapped in an async IIFE so `await` works (§5), with an in-guest
        // try/catch that captures both sync and awaited-rejection errors on `__rq_err`. A parse
        // error of the wrapper itself surfaces as the eval `Err` below. Either way it becomes the
        // result's `error`, not a hard failure — tests/logs/mutations up to the throw still return.
        let wrapped = format!(
            "globalThis.__rq_err = null; (async () => {{ try {{\n{}\n}} catch (e) {{ globalThis.__rq_err = String((e && e.message) || e); }} }})();",
            input.script
        );
        if let Err(err) = ctx.eval::<rquickjs::Value, _>(wrapped).catch(&ctx) {
            result.error = Some(err.to_string());
        }

        Ok(())
    })?;

    // Drain microtask queue (promise resolutions from sync code).
    while runtime.is_job_pending() {
        let _ = runtime.execute_pending_job();
    }

    // Drain the reserved output channels with one JSON.stringify.
    let drained: String = context
        .with(|ctx| -> Result<String, String> {
            ctx.eval::<String, _>(
                "JSON.stringify({ tests: __rq_tests, logs: __rq_logs, mut: __rq_mut, reqmut: __rq_reqmut, directive: __rq_directive.value, err: globalThis.__rq_err })",
            )
            .catch(&ctx)
            .map_err(|e| e.to_string())
        })?;

    apply_drain(&drained, &mut result)?;
    Ok(result)
}

/// Fold the guest's drained JSON into the typed result.
fn apply_drain(drained: &str, result: &mut ScriptExecutionResult) -> Result<(), String> {
    let v: Value = serde_json::from_str(drained).map_err(|e| e.to_string())?;

    if let Some(tests) = v.get("tests") {
        result.test_results = serde_json::from_value(tests.clone()).map_err(|e| e.to_string())?;
    }
    if let Some(logs) = v.get("logs") {
        result.logs = serde_json::from_value(logs.clone()).map_err(|e| e.to_string())?;
    }
    if let Some(mutobj) = v.get("mut").and_then(Value::as_object) {
        let take = |k: &str| -> serde_json::Map<String, Value> {
            mutobj.get(k).and_then(Value::as_object).cloned().unwrap_or_default()
        };
        result.mutation_diff = MutationDiff {
            environment: take("environment"),
            globals: take("globals"),
            collection: take("collection"),
            variables: take("runtime"),
        };
    }
    if let Some(reqmut) = v.get("reqmut") {
        result.request_header_mutations = serde_json::from_value(reqmut.clone()).unwrap_or_default();
    }
    if let Some(dir) = v.get("directive") {
        if !dir.is_null() {
            result.execution_directive = serde_json::from_value(dir.clone()).ok();
        }
    }
    // An in-guest error (sync throw or awaited rejection) wins over an empty result but not over a
    // harness/parse error already set.
    if result.error.is_none() {
        if let Some(err) = v.get("err").and_then(Value::as_str) {
            result.error = Some(err.to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(script: &str, phase: Phase) -> ScriptExecutionInput {
        ScriptExecutionInput {
            script: script.to_string(),
            phase,
            context: ScriptContext::default(),
            mode: Mode::Safe,
            timeout_ms: None,
        }
    }

    #[test]
    fn runs_a_test_sets_a_var_and_captures_a_log() {
        let mut inp = input(
            r#"
              console.log("hello", 42);
              rq.environment.set("token", "abc");
              rq.test("adds up", function () { rq.expect(1 + 1).to.equal(2); });
              rq.test("fails", function () { rq.expect(1).to.equal(2); });
              rq.test.skip("later");
            "#,
            Phase::PreRequest,
        );
        inp.context.environment.insert("base".into(), Value::String("x".into()));
        let r = execute(&inp);

        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
        assert_eq!(r.test_results.len(), 3);
        assert_eq!(r.test_results[0].status, TestStatus::Passed);
        assert_eq!(r.test_results[1].status, TestStatus::Failed);
        assert_eq!(r.test_results[2].status, TestStatus::Skipped);
        assert_eq!(
            r.mutation_diff.environment.get("token"),
            Some(&Value::String("abc".into()))
        );
        assert_eq!(r.logs.len(), 1);
        assert_eq!(r.logs[0].level, "log");
    }

    #[test]
    fn reads_context_variables_and_response_in_post_response() {
        let mut inp = input(
            r#"
              rq.test("reads env", function () { rq.expect(rq.environment.get("k")).to.equal("v"); });
              rq.test("reads status", function () { rq.expect(rq.response.code).to.equal(200); });
            "#,
            Phase::PostResponse,
        );
        inp.context.environment.insert("k".into(), Value::String("v".into()));
        inp.context.response = serde_json::json!({ "status": 200, "body": "{\"ok\":true}" });
        let r = execute(&inp);
        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
        assert!(r.test_results.iter().all(|t| t.status == TestStatus::Passed), "{:?}", r.test_results);
    }

    #[test]
    fn empty_variable_key_throws_like_the_reference() {
        let r = execute(&input(r#"rq.environment.set("", "x");"#, Phase::PreRequest));
        assert!(r.error.as_deref().unwrap_or("").contains("non-empty string"), "{:?}", r.error);
    }

    #[test]
    fn set_next_request_surfaces_as_a_directive() {
        let r = execute(&input(r#"rq.execution.setNextRequest("Login");"#, Phase::PreRequest));
        match r.execution_directive {
            Some(ExecutionDirective::SetNextRequest { target: Some(t) }) => assert_eq!(t, "Login"),
            other => panic!("expected set-next-request, got {other:?}"),
        }
    }

    #[test]
    fn pm_script_transformed_then_executed_end_to_end() {
        // The whole pipeline: a Postman-dialect script → compat transform → run on the runtime.
        let pm = r#"
          pm.environment.set("t", "abc");
          pm.test("reads it back", function () { pm.expect(pm.environment.get("t")).to.equal("abc"); });
        "#;
        let out = compat::transform(pm, compat::Platform::Postman);
        assert!(out.success, "transform failed: {:?}", out.diagnostics);
        assert!(out.code.contains("rq.environment.set"), "not rewritten: {}", out.code);
        assert!(!out.code.contains("pm.environment"), "pm.* left behind: {}", out.code);

        let r = execute(&input(&out.code, Phase::PreRequest));
        assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
        assert_eq!(r.test_results.len(), 1);
        assert_eq!(r.test_results[0].status, TestStatus::Passed);
        assert_eq!(
            r.mutation_diff.environment.get("t"),
            Some(&Value::String("abc".into()))
        );
    }

    #[test]
    fn skip_request_is_pre_request_only() {
        let ok = execute(&input("rq.execution.skipRequest();", Phase::PreRequest));
        assert!(matches!(ok.execution_directive, Some(ExecutionDirective::SkipRequest)));
        let bad = execute(&input("rq.execution.skipRequest();", Phase::PostResponse));
        assert!(bad.error.as_deref().unwrap_or("").contains("pre-request"));
    }

    #[test]
    fn response_assertion_tree() {
        let mut inp = input(
            r#"
              rq.test("ok", function () { rq.response.to.be.ok; });
              rq.test("status", function () { rq.response.to.have.status(200); });
              rq.test("not error", function () { rq.response.to.not.be.error; });
              rq.test("json path", function () { rq.response.to.have.jsonBody("a.b", 1); });
              rq.test("fails notFound", function () { rq.response.to.be.notFound; });
            "#,
            Phase::PostResponse,
        );
        inp.context.response = serde_json::json!({ "status": 200, "body": "{\"a\":{\"b\":1}}" });
        let r = execute(&inp);
        assert!(r.error.is_none(), "{:?}", r.error);
        let status = |n: &str| r.test_results.iter().find(|t| t.name == n).unwrap().status;
        assert_eq!(status("ok"), TestStatus::Passed);
        assert_eq!(status("status"), TestStatus::Passed);
        assert_eq!(status("not error"), TestStatus::Passed);
        assert_eq!(status("json path"), TestStatus::Passed);
        assert_eq!(status("fails notFound"), TestStatus::Failed);
    }

    #[test]
    fn request_header_facade_records_mutations() {
        let mut inp = input(
            r#"
              rq.request.headers.upsert({ key: "Authorization", value: "Bearer x" });
              rq.request.addHeader({ key: "X-Trace", value: "1" });
              rq.request.removeHeader("Cookie");
            "#,
            Phase::PreRequest,
        );
        inp.context.request =
            serde_json::json!({ "url": "https://x.test", "method": "GET", "headers": [{ "key": "Cookie", "value": "a=b" }] });
        let r = execute(&inp);
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(r.request_header_mutations.len(), 3);
        assert!(matches!(&r.request_header_mutations[0], HeaderMutation::Upsert { key, .. } if key == "Authorization"));
        assert!(matches!(&r.request_header_mutations[2], HeaderMutation::Remove { name } if name == "Cookie"));
    }

    #[test]
    fn async_await_is_supported() {
        let r = execute(&input(
            r#"
              const v = await Promise.resolve("hi");
              rq.environment.set("k", v);
              rq.test("awaited", function () { rq.expect(rq.environment.get("k")).to.equal("hi"); });
            "#,
            Phase::PreRequest,
        ));
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(r.test_results[0].status, TestStatus::Passed);
        assert_eq!(r.mutation_diff.environment.get("k"), Some(&Value::String("hi".into())));
    }
}
