//! Scripts, run by the engine `rq` actually ships with.
//!
//! Everything else stands a stub in for cross-q-context. This one runs it: real QuickJS,
//! real `rq.*`, real assertions — the only way to know the wire contract between a Rust
//! host and a JavaScript engine is the one both sides believe in.
//!
//! **Needs `node` and a built cross-q-context with its dependencies installed** (`npm
//! install` in `packages/cross-q-context`). Absent those it skips with a message rather
//! than failing, because they are not something a `cargo test` can fetch — but set
//! `RQ_REQUIRE_ENGINE=1` and the skip becomes a failure, which is what CI should do once
//! this is wired there.

mod support;

use std::path::PathBuf;
use std::process::Command;

use support::Stub;

const BIN: &str = env!("CARGO_BIN_EXE_rq");

/// Is there an engine to test? Returns the package path, or explains itself.
fn engine() -> Option<PathBuf> {
    let package = std::env::var_os("RQ_SCRIPT_ENGINE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/cross-q-context")
        });
    let installed = package
        .join("node_modules/quickjs-emscripten-core")
        .is_dir();
    let built = package.join("dist/runtime/engine/execute.js").is_file();
    let node = std::env::split_paths(&std::env::var_os("PATH")?).any(|d| d.join("node").is_file());

    if node && built && installed {
        return Some(package);
    }
    let why = if !node {
        "`node` is not on PATH"
    } else if !built {
        "cross-q-context is not built"
    } else {
        "cross-q-context's dependencies are not installed (npm install)"
    };
    assert!(
        std::env::var("RQ_REQUIRE_ENGINE").is_err(),
        "RQ_REQUIRE_ENGINE is set but {why}"
    );
    eprintln!("skipping: {why}");
    None
}

struct Fixture {
    dir: tempfile::TempDir,
    package: PathBuf,
}

impl Fixture {
    fn new(package: PathBuf) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        rq::project::init(dir.path()).unwrap();
        Fixture { dir, package }
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.dir.path().join(format!("{name}.md")), contents).unwrap();
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(BIN)
            .args(args)
            .args(["--color=never", "--no-console"])
            .current_dir(self.dir.path())
            .env("RQ_SCRIPT_ENGINE", &self.package)
            .env_remove("RQ_PROJECT")
            .output()
            .expect("running rq");
        (
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
            out.status.code().unwrap_or(-1),
        )
    }
}

#[test]
fn a_post_response_script_really_runs() {
    let Some(package) = engine() else { return };
    let stub = Stub::start(1, |_| (200, "OK", "{\"id\":7,\"name\":\"seven\"}".into()));
    let f = Fixture::new(package);
    f.write(
        "thing",
        &format!(
            "---\nurl: {}/thing\n---\n\n-- post --\n\n\
             console.log('the id is', JSON.parse(rq.response.body).id);\n\
             rq.test('came back ok', () => {{ if (rq.response.status !== 200) throw new Error('no'); }});\n\
             rq.test('has a name', () => {{ if (!JSON.parse(rq.response.body).name) throw new Error('nameless'); }});\n",
            stub.base
        ),
    );

    let (_, narration, code) = f.run(&["r", "thing"]);
    assert_eq!(code, 0, "{narration}");
    assert!(narration.contains("2/2 test(s) passed"), "{narration}");
    assert!(
        narration.contains("the id is 7"),
        "console.log: {narration}"
    );
}

#[test]
fn a_failing_assertion_fails_the_run() {
    let Some(package) = engine() else { return };
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new(package);
    f.write(
        "checked",
        &format!(
            "---\nurl: {}/x\n---\n\n-- post --\n\n\
             rq.test('this one fails', () => {{ throw new Error('on purpose'); }});\n",
            stub.base
        ),
    );

    let (_, narration, code) = f.run(&["r", "checked"]);
    assert_eq!(code, 1, "a failed assertion must fail the run: {narration}");
    assert!(narration.contains("on purpose"), "{narration}");
}

#[test]
fn a_variable_a_script_sets_reaches_the_next_request() {
    let Some(package) = engine() else { return };
    let stub = Stub::start(2, |req| match req.path.as_str() {
        "/login" => (200, "OK", "{\"token\":\"tok-from-script\"}".into()),
        _ => (200, "OK", "{}".into()),
    });
    let f = Fixture::new(package);
    f.write(
        "login",
        &format!(
            "---\nmethod: POST\nurl: {}/login\n---\n\n-- post --\n\n\
             rq.variables.set('token', JSON.parse(rq.response.body).token);\n",
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

    let (_, narration, code) = f.run(&["r", "me"]);
    assert_eq!(code, 0, "{narration}");
    stub.next();
    // The whole point: `rq.variables.set` in one request reached the wire of the next.
    assert_eq!(
        stub.next().header("authorization"),
        Some("Bearer tok-from-script")
    );
}

#[test]
fn a_pre_request_script_can_change_the_request() {
    let Some(package) = engine() else { return };
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new(package);
    f.write(
        "signed",
        &format!(
            "---\nurl: {}/x\n---\n\n-- pre --\n\n\
             rq.request.headers.upsert({{ key: 'X-Signature', value: 'abc' + 123 }});\n",
            stub.base
        ),
    );

    let (_, narration, code) = f.run(&["r", "signed"]);
    assert_eq!(code, 0, "{narration}");
    assert_eq!(stub.next().header("x-signature"), Some("abc123"));
}

#[test]
fn a_script_that_throws_is_reported_without_losing_the_response() {
    let Some(package) = engine() else { return };
    let stub = Stub::start(1, |_| (200, "OK", "{\"still\":\"here\"}".into()));
    let f = Fixture::new(package);
    f.write(
        "broken",
        &format!(
            "---\nurl: {}/x\n---\n\n-- post --\n\nthisIsNotDefined();\n",
            stub.base
        ),
    );

    let (result, narration, _) = f.run(&["r", "broken"]);
    assert!(
        narration.to_lowercase().contains("not defined") || narration.contains("ReferenceError"),
        "the error should say what broke: {narration}"
    );
    assert!(
        result.contains("still"),
        "the response survives a broken script: {result}"
    );
}
