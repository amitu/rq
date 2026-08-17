//! The script host boundary, exercised end to end with a stub engine.
//!
//! `rq` ships no script runtime yet, so these tests stand in for one: a [`FakeEngine`]
//! returns the results a real engine would, and the assertions are on what reached the
//! **socket** and what came back on the run. That makes the seam a tested contract rather
//! than a hopeful shape — when cross-q-context's engine lands, it drops into the same slot
//! and these tests say whether the wiring still holds.

mod support;

use std::sync::Mutex;

use support::Stub;

use rq::project::{self, Project};
use rq::run::{self, RunOptions};
use rq::script::{
    ExecutionDirective, LogEntry, MutationDiff, RequestHeaderMutation, RequestMutationDiff,
    ScriptEngine, ScriptExecutionInput, ScriptExecutionResult, ScriptPhase, TestResult, TestStatus,
};

/// An engine that answers with whatever the test says, and records what it was asked.
struct FakeEngine {
    reply: Box<dyn Fn(&ScriptExecutionInput) -> ScriptExecutionResult + Send + Sync>,
    seen: Mutex<Vec<ScriptExecutionInput>>,
}

impl FakeEngine {
    fn new(
        reply: impl Fn(&ScriptExecutionInput) -> ScriptExecutionResult + Send + Sync + 'static,
    ) -> Self {
        Self {
            reply: Box::new(reply),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<ScriptExecutionInput> {
        self.seen.lock().unwrap().clone()
    }
}

impl ScriptEngine for FakeEngine {
    fn name(&self) -> &str {
        "fake"
    }

    fn execute(&self, input: &ScriptExecutionInput) -> anyhow::Result<ScriptExecutionResult> {
        self.seen.lock().unwrap().push(input.clone());
        Ok((self.reply)(input))
    }
}

fn passing(name: &str) -> TestResult {
    TestResult {
        name: name.into(),
        status: TestStatus::Passed,
        error: None,
    }
}

fn failing(name: &str, error: &str) -> TestResult {
    TestResult {
        name: name.into(),
        status: TestStatus::Failed,
        error: Some(error.into()),
    }
}

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        project::init(dir.path()).unwrap();
        Fixture { dir }
    }

    fn write(&self, rel: &str, contents: &str) {
        let path = self.dir.path().join(format!("{rel}.md"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
    }

    fn run(&self, name: &str, engine: &dyn ScriptEngine) -> run::Run {
        let project = Project::open(self.dir.path().to_path_buf()).unwrap();
        let target = project.resolve(name).unwrap();
        run::run(&project, target, &RunOptions::default(), engine).unwrap()
    }
}

// ---------------------------------------------------------------------------------------

#[test]
fn a_pre_request_script_changes_what_goes_on_the_wire() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write(
        "signed",
        &format!(
            "---\nurl: {}/x\nheaders:\n  X-Keep: yes\n  X-Drop: no\n---\n\n-- pre --\n\nsign();\n",
            stub.base
        ),
    );

    let engine = FakeEngine::new(|_| ScriptExecutionResult {
        request_mutation_diff: Some(RequestMutationDiff {
            headers: vec![
                RequestHeaderMutation::Upsert {
                    name: "X-Signature".into(),
                    value: "abc123".into(),
                },
                RequestHeaderMutation::Remove {
                    name: "X-Drop".into(),
                },
            ],
        }),
        ..ScriptExecutionResult::default()
    });

    let outcome = f.run("signed", &engine);
    assert!(outcome.target().response.is_some());

    let seen = stub.next();
    assert_eq!(seen.header("x-signature"), Some("abc123"));
    assert_eq!(seen.header("x-keep"), Some("yes"));
    assert_eq!(
        seen.header("x-drop"),
        None,
        "the script removed this header"
    );
}

#[test]
fn a_post_response_variable_reaches_the_next_request() {
    let stub = Stub::start(2, |req| match req.path.as_str() {
        "/auth/login" => (200, "OK", "{\"ok\":true}".into()),
        _ => (200, "OK", "{}".into()),
    });
    let f = Fixture::new();
    f.write(
        "login",
        &format!(
            "---\nmethod: POST\nurl: {}/auth/login\n---\n\n-- post --\n\nsetToken();\n",
            stub.base
        ),
    );
    f.write(
        "me",
        &format!(
            "---\nurl: {}/me\nheaders:\n  Authorization: Bearer {{{{token}}}}\nparents: [login]\n---\n",
            stub.base
        ),
    );

    // The engine sets a variable the way `rq.vars.set` would — into the same runtime layer
    // `capture:` writes to.
    let engine = FakeEngine::new(|input| {
        if input.phase == ScriptPhase::PostResponse {
            let mut vars = serde_json::Map::new();
            vars.insert("token".into(), serde_json::json!("from-script"));
            return ScriptExecutionResult {
                mutation_diff: MutationDiff {
                    environment: Some(vars),
                    ..MutationDiff::default()
                },
                ..ScriptExecutionResult::default()
            };
        }
        ScriptExecutionResult::default()
    });

    f.run("me", &engine);
    stub.next(); // login
    let me = stub.next();
    assert_eq!(me.header("authorization"), Some("Bearer from-script"));
}

#[test]
fn test_results_and_logs_land_on_the_step_and_set_the_exit_code() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write(
        "checked",
        &format!(
            "---\nurl: {}/x\n---\n\n-- post --\n\nassertions();\n",
            stub.base
        ),
    );

    let engine = FakeEngine::new(|_| ScriptExecutionResult {
        test_results: vec![
            passing("status is 200"),
            failing("has a token", "expected undefined to be a string"),
        ],
        logs: vec![LogEntry {
            level: "log".into(),
            args: vec![serde_json::json!("checking"), serde_json::json!(2)],
        }],
        ..ScriptExecutionResult::default()
    });

    let outcome = f.run("checked", &engine);
    assert_eq!(outcome.total_tests(), 2);
    assert_eq!(outcome.failed_tests(), 1);
    assert_eq!(outcome.target().tests[0].status, TestStatus::Passed);
    assert_eq!(outcome.target().logs[0].message(), "checking 2");
}

#[test]
fn skip_request_sends_nothing_and_says_so() {
    // The stub expects one request and will not get it — the assertion is the silence.
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write(
        "skipped",
        &format!(
            "---\nurl: {}/x\n---\n\n-- pre --\n\nrq.execution.skipRequest();\n",
            stub.base
        ),
    );

    let engine = FakeEngine::new(|_| ScriptExecutionResult {
        execution_directive: Some(ExecutionDirective::SkipRequest),
        ..ScriptExecutionResult::default()
    });

    let outcome = f.run("skipped", &engine);
    let step = outcome.target();
    assert!(step.skipped());
    assert!(step.response.is_none());
    assert!(
        step.notes.iter().any(|n| n.contains("skipRequest")),
        "{:?}",
        step.notes
    );
    assert!(
        outcome.view.is_none(),
        "nothing to render without a response"
    );
}

#[test]
fn set_next_request_is_refused_out_loud() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write(
        "redirected",
        &format!("---\nurl: {}/x\n---\n\n-- post --\n\nnext();\n", stub.base),
    );

    let engine = FakeEngine::new(|_| ScriptExecutionResult {
        execution_directive: Some(ExecutionDirective::SetNextRequest {
            target: Some("other".into()),
        }),
        ..ScriptExecutionResult::default()
    });

    let outcome = f.run("redirected", &engine);
    assert!(
        outcome
            .target()
            .notes
            .iter()
            .any(|n| n.contains("setNextRequest") && n.contains("parents:")),
        "{:?}",
        outcome.target().notes
    );
}

/// The half of chaining that has nothing to do with scripts: most real sessions are a
/// cookie, not a token in a JSON body.
#[test]
fn a_cookie_set_by_one_step_is_sent_on_the_next() {
    let stub = Stub::start_with_headers(2, |req| match req.path.as_str() {
        "/auth/login" => (
            200,
            "OK",
            "{\"ok\":true}".into(),
            vec![(
                "Set-Cookie".to_string(),
                "session=abc123; Path=/; HttpOnly".to_string(),
            )],
        ),
        _ => (200, "OK", "{}".into(), Vec::new()),
    });
    let f = Fixture::new();
    f.write(
        "login",
        &format!("---\nmethod: POST\nurl: {}/auth/login\n---\n", stub.base),
    );
    f.write(
        "me",
        &format!("---\nurl: {}/me\nparents: [login]\n---\n", stub.base),
    );

    let outcome = f.run("me", &rq::script::NoEngine);
    assert_eq!(outcome.steps.len(), 2);

    let login = stub.next();
    assert_eq!(login.header("cookie"), None, "nothing to send yet");
    let me = stub.next();
    assert_eq!(me.header("cookie"), Some("session=abc123"));
}

/// …and the jar is visible to a script, in the runtime's shape.
#[test]
fn the_jar_is_seeded_into_the_script_context() {
    let stub = Stub::start_with_headers(2, |req| match req.path.as_str() {
        "/auth/login" => (
            200,
            "OK",
            "{}".into(),
            vec![(
                "Set-Cookie".to_string(),
                "session=abc123; Path=/".to_string(),
            )],
        ),
        _ => (200, "OK", "{}".into(), Vec::new()),
    });
    let f = Fixture::new();
    f.write(
        "login",
        &format!("---\nmethod: POST\nurl: {}/auth/login\n---\n", stub.base),
    );
    f.write(
        "me",
        &format!(
            "---\nurl: {}/me\nparents: [login]\n---\n\n-- pre --\n\nreadJar();\n",
            stub.base
        ),
    );

    let engine = FakeEngine::new(|_| ScriptExecutionResult::default());
    f.run("me", &engine);

    let calls = engine.calls();
    let ctx = &calls.last().expect("the pre-request script ran").context;
    assert_eq!(ctx.cookie_jar_seed.len(), 1);
    assert_eq!(ctx.cookie_jar_seed[0].cookies[0]["key"], "session");
    assert_eq!(ctx.cookie_jar_seed[0].cookies[0]["value"], "abc123");
    assert!(
        ctx.host_allowlist.contains(&"127.0.0.1".to_string()),
        "{:?}",
        ctx.host_allowlist
    );
}

#[test]
fn the_context_handed_to_the_engine_matches_the_contract() {
    let stub = Stub::start(1, |_| (201, "Created", "{\"id\":7}".into()));
    let f = Fixture::new();
    f.write(
        "shaped",
        &format!(
            "---\nmethod: POST\nurl: {}/things\nquery:\n  expand: all\n\
             headers:\n  Content-Type: application/json\n\
             vars:\n  who: amitu\n  TOKEN: {{ default: s3cret, secret: true }}\n---\n\n\
             -- body --\n\n{{\"a\": 1}}\n\n-- post --\n\ncheck();\n",
            stub.base
        ),
    );

    let engine = FakeEngine::new(|_| ScriptExecutionResult::default());
    f.run("shaped", &engine);

    let calls = engine.calls();
    assert_eq!(calls.len(), 1, "only the post-response script exists");
    let ctx = &calls[0].context;

    // request, in model.ts's shape
    assert_eq!(ctx.request["method"], "POST");
    assert!(
        ctx.request["url"]
            .as_str()
            .unwrap()
            .ends_with("/things?expand=all"),
        "{}",
        ctx.request["url"]
    );
    assert_eq!(ctx.request["queryParams"][0]["key"], "expand");
    assert_eq!(ctx.request["body"]["raw"], "{\"a\": 1}");
    assert_eq!(ctx.request["contentType"], "application/json");

    // response, in model.ts's shape
    let response = ctx
        .response
        .as_ref()
        .expect("post-response gets a response");
    assert_eq!(response["status"], 201);
    assert_eq!(response["statusText"], "Created");
    assert_eq!(response["body"], "{\"id\":7}");
    assert!(response["headers"]["content-type"].is_string());

    // variables are bucketed by where the value came from, and secrets are flagged
    assert_eq!(ctx.variables["who"]["syncValue"], "amitu");
    assert_eq!(ctx.secrets["TOKEN"]["type"], "secret");
    assert_eq!(calls[0].phase, ScriptPhase::PostResponse);
    assert_eq!(ctx.info.request_name, "shaped");
    assert_eq!(ctx.info.total_entries, 1);
}

#[test]
fn the_shipped_build_reports_that_the_script_never_ran() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write(
        "scripted",
        &format!(
            "---\nurl: {}/x\n---\n\n-- post --\n\nrq.test('x', () => true);\n",
            stub.base
        ),
    );

    let outcome = f.run("scripted", &rq::script::NoEngine);
    assert!(
        outcome
            .target()
            .notes
            .iter()
            .any(|n| n.contains("NOT executed")),
        "{:?}",
        outcome.target().notes
    );
    assert_eq!(outcome.total_tests(), 0);
}

// --- the chain: collection scripts wrap their requests ------------------------------------

impl Fixture {
    fn write_collection(&self, rel: &str, contents: &str) {
        let path = self.dir.path().join(rel).join(project::COLLECTION_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
    }
}

/// ADR-061's sandwich: pre-request runs root → request, post-response runs request → root.
/// Getting this backwards would make a collection imported from the app behave differently
/// in the CLI, which is the drift this whole project exists to prevent.
#[test]
fn collection_scripts_wrap_the_request_in_both_directions() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write_collection(
        "acme",
        "---\n---\n\n-- pre --\n\nouterPre();\n\n-- post --\n\nouterPost();\n",
    );
    f.write_collection(
        "acme/v2",
        "---\n---\n\n-- pre --\n\ninnerPre();\n\n-- post --\n\ninnerPost();\n",
    );
    f.write(
        "acme/v2/ping",
        &format!(
            "---\nurl: {}/ping\n---\n\n-- pre --\n\nownPre();\n\n-- post --\n\nownPost();\n",
            stub.base
        ),
    );

    let engine = FakeEngine::new(|_| ScriptExecutionResult::default());
    f.run("acme/v2/ping", &engine);

    let order: Vec<String> = engine
        .calls()
        .iter()
        .map(|c| format!("{:?}:{}", c.phase, c.script.trim()))
        .collect();
    assert_eq!(
        order,
        vec![
            "PreRequest:outerPre();",
            "PreRequest:innerPre();",
            "PreRequest:ownPre();",
            "PostResponse:ownPost();",
            "PostResponse:innerPost();",
            "PostResponse:outerPost();",
        ]
    );
}

/// ADR-020: the request is re-prepared after every script, so a variable a collection's
/// pre-request script sets is substituted into the request that follows it.
#[test]
fn a_variable_set_by_a_collection_script_reaches_the_request_it_wraps() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write_collection("acme", "---\n---\n\n-- pre --\n\nsign();\n");
    f.write(
        "acme/ping",
        &format!(
            "---\nurl: {}/ping\nquery:\n  sig: '{{{{signature}}}}'\n---\n",
            stub.base
        ),
    );

    let engine = FakeEngine::new(|_| {
        let mut vars = serde_json::Map::new();
        vars.insert("signature".into(), serde_json::json!("computed-abc"));
        ScriptExecutionResult {
            mutation_diff: MutationDiff {
                variables: Some(vars),
                ..MutationDiff::default()
            },
            ..ScriptExecutionResult::default()
        }
    });

    f.run("acme/ping", &engine);
    assert_eq!(stub.next().path, "/ping?sig=computed-abc");
}

/// ADR-167: header mutations accumulate across the whole chain, in call order.
#[test]
fn header_mutations_accumulate_across_the_chain() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write_collection("acme", "---\n---\n\n-- pre --\n\ncollection();\n");
    f.write(
        "acme/ping",
        &format!(
            "---\nurl: {}/ping\n---\n\n-- pre --\n\nrequest();\n",
            stub.base
        ),
    );

    let engine = FakeEngine::new(|input| {
        let name = if input.script.contains("collection") {
            "X-From-Collection"
        } else {
            "X-From-Request"
        };
        ScriptExecutionResult {
            request_mutation_diff: Some(RequestMutationDiff {
                headers: vec![RequestHeaderMutation::Upsert {
                    name: name.into(),
                    value: "1".into(),
                }],
            }),
            ..ScriptExecutionResult::default()
        }
    });

    f.run("acme/ping", &engine);
    let seen = stub.next();
    assert_eq!(seen.header("x-from-collection"), Some("1"));
    assert_eq!(seen.header("x-from-request"), Some("1"));
}

/// ADR-169: a `skipRequest()` anywhere in the pre-request chain aborts the rest of it —
/// running later scripts for a request that will never be sent is how state gets mutated
/// for a call that didn't happen.
#[test]
fn a_skip_in_a_collection_script_aborts_the_rest_of_the_chain() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write_collection("acme", "---\n---\n\n-- pre --\n\nskipEverything();\n");
    f.write(
        "acme/ping",
        &format!(
            "---\nurl: {}/ping\n---\n\n-- pre --\n\nneverRuns();\n",
            stub.base
        ),
    );

    let engine = FakeEngine::new(|input| {
        if input.script.contains("skipEverything") {
            return ScriptExecutionResult {
                execution_directive: Some(ExecutionDirective::SkipRequest),
                ..ScriptExecutionResult::default()
            };
        }
        ScriptExecutionResult::default()
    });

    let outcome = f.run("acme/ping", &engine);
    assert!(outcome.target().skipped());
    let ran: Vec<String> = engine
        .calls()
        .iter()
        .map(|c| c.script.trim().to_string())
        .collect();
    assert_eq!(
        ran,
        vec!["skipEverything();"],
        "the request's own script must not run"
    );
}

/// Every request in the graph gets its chain — not just the one you named.
#[test]
fn every_request_in_the_dag_runs_its_own_scripts() {
    let stub = Stub::start(2, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write_collection("acme", "---\n---\n\n-- pre --\n\nshared();\n");
    f.write(
        "acme/login",
        &format!(
            "---\nmethod: POST\nurl: {}/login\n---\n\n-- post --\n\nloginPost();\n",
            stub.base
        ),
    );
    f.write(
        "acme/me",
        &format!(
            "---\nurl: {}/me\nparents: [login]\n---\n\n-- pre --\n\nmePre();\n",
            stub.base
        ),
    );

    let engine = FakeEngine::new(|_| ScriptExecutionResult::default());
    f.run("acme/me", &engine);

    let ran: Vec<String> = engine
        .calls()
        .iter()
        .map(|c| c.script.trim().to_string())
        .collect();
    assert_eq!(
        ran,
        vec![
            "shared();",    // login's inherited pre-request
            "loginPost();", // login's own post-response
            "shared();",    // me's inherited pre-request
            "mePre();",     // me's own
        ]
    );
}

/// …and with no engine, an unrun collection script is reported, not silently ignored.
#[test]
fn an_unrun_collection_script_is_reported_under_its_collection() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write_collection("acme", "---\n---\n\n-- pre --\n\ncollectionScript();\n");
    f.write("acme/ping", &format!("---\nurl: {}/ping\n---\n", stub.base));

    let outcome = f.run("acme/ping", &rq::script::NoEngine);
    assert!(
        outcome
            .target()
            .notes
            .iter()
            .any(|n| n.starts_with("acme:") && n.contains("NOT executed")),
        "{:?}",
        outcome.target().notes
    );
}
