//! Scripts, run by the engine `rq` actually ships with — and by the one it used to.
//!
//! Every case here runs TWICE where it can: once on the embedded QuickJS host compiled into
//! the binary, and once on the Node sidecar against a cross-q-context checkout. Identical
//! assertions on both. That is the point of the suite: the embedded host is new Rust code
//! around the package's own guest realm, and the way to know it kept the semantics is to run
//! the same script through the engine that already had them.
//!
//! The embedded engine always runs — it needs nothing installed. The sidecar leg needs `node`
//! and a built cross-q-context (`npm install` in `packages/cross-q-context`); absent those it
//! is skipped with a message, or fails if `RQ_REQUIRE_ENGINE=1` is set, which is what CI does.

mod support;

use std::path::PathBuf;
use std::process::Command;

use support::Stub;

const BIN: &str = env!("CARGO_BIN_EXE_rq");

/// Which engine a case is running on.
#[derive(Clone, Debug)]
enum Engine {
    /// Compiled in. Needs nothing on the machine, so it is never skipped.
    Embedded,
    /// `node` on a cross-q-context checkout — the engine the semantics come from.
    Sidecar(PathBuf),
}

impl Engine {
    fn label(&self) -> &str {
        match self {
            Engine::Embedded => "embedded",
            Engine::Sidecar(_) => "sidecar",
        }
    }
}

/// The two cases below run on the embedded engine ONLY, and the reason is worth recording:
/// rq's sidecar loads cross-q-context's *lean* `executeScript`, which wires `console` and the
/// cookie jar and nothing else — `require('crypto')` there answers "not available in the safe
/// sandbox". The embedded host wires the Node-builtin bridges the full `QuickJsEngine` has,
/// because a request script that cannot hash a payload or gunzip a body is not much of a
/// script. So this is the embedded engine being *more* capable than what it replaced, and
/// there is nothing to compare it against until the sidecar is moved onto the full engine.
fn embedded_only() -> Vec<Engine> {
    vec![Engine::Embedded]
}

/// Every engine available here. The embedded one always; the sidecar when the checkout is
/// usable, so the two are compared on any machine set up to do it.
fn engines() -> Vec<Engine> {
    let mut out = vec![Engine::Embedded];
    if let Some(package) = sidecar() {
        out.push(Engine::Sidecar(package));
    }
    out
}

/// Is the sidecar available? Returns the package path, or explains itself.
fn sidecar() -> Option<PathBuf> {
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
    engine: Engine,
}

impl Fixture {
    fn new(engine: Engine) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        rq::project::init(dir.path()).unwrap();
        Fixture { dir, engine }
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.dir.path().join(format!("{name}.md")), contents).unwrap();
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .args(["--color=never", "--no-console"])
            .current_dir(self.dir.path())
            .env_remove("RQ_PROJECT");
        match &self.engine {
            // Unset means "use the one compiled in" — the default a user gets.
            Engine::Embedded => cmd.env_remove("RQ_SCRIPT_ENGINE"),
            Engine::Sidecar(package) => cmd.env("RQ_SCRIPT_ENGINE", package),
        };
        let out = cmd.output().expect("running rq");
        (
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
            out.status.code().unwrap_or(-1),
        )
    }
}

fn case_a_post_response_script_really_runs(engine: Engine) {
    let stub = Stub::start(1, |_| (200, "OK", "{\"id\":7,\"name\":\"seven\"}".into()));
    let f = Fixture::new(engine.clone());
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

fn case_a_failing_assertion_fails_the_run(engine: Engine) {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new(engine.clone());
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

fn case_a_variable_a_script_sets_reaches_the_next_request(engine: Engine) {
    let stub = Stub::start(2, |req| match req.path.as_str() {
        "/login" => (200, "OK", "{\"token\":\"tok-from-script\"}".into()),
        _ => (200, "OK", "{}".into()),
    });
    let f = Fixture::new(engine.clone());
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

fn case_a_pre_request_script_can_change_the_request(engine: Engine) {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new(engine.clone());
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

fn case_a_script_that_throws_is_reported_without_losing_the_response(engine: Engine) {
    let stub = Stub::start(1, |_| (200, "OK", "{\"still\":\"here\"}".into()));
    let f = Fixture::new(engine.clone());
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

/// `crypto`, `Buffer`, `zlib` and `require` in the embedded host are Rust code, so this
/// asserts against digests computed outside the test entirely (shasum, openssl, base64) —
/// agreement between two engines is not correctness, and here there is only one engine that
/// can do this at all (see `embedded_only`).
fn case_the_node_builtins_are_the_same_builtins(engine: Engine) {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new(engine.clone());
    f.write(
        "builtins",
        &format!(
            "---\nurl: {}/x\n---\n\n-- post --\n\n\
             const crypto = require('crypto');\n\
             const zlib = require('zlib');\n\
             const _ = require('lodash');\n\
             rq.test('sha256', () => rq.expect(crypto.createHash('sha256').update('abc').digest('hex'))\n\
                 .to.equal('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'));\n\
             rq.test('md5', () => rq.expect(crypto.createHash('md5').update('abc').digest('hex'))\n\
                 .to.equal('900150983cd24fb0d6963f7d28e17f72'));\n\
             rq.test('hmac sha256', () => rq.expect(crypto.createHmac('sha256', 'key').update('abc').digest('hex'))\n\
                 .to.equal('9c196e32dc0175f86f4b1cb89289d6619de6bee699e4c378e68309ed97a1a6ab'));\n\
             rq.test('base64 out', () => rq.expect(Buffer.from('hello', 'utf8').toString('base64')).to.equal('aGVsbG8='));\n\
             rq.test('base64 in', () => rq.expect(Buffer.from('aGVsbG8=', 'base64').toString('utf8')).to.equal('hello'));\n\
             rq.test('hex round trip', () => rq.expect(Buffer.from('616263', 'hex').toString('utf8')).to.equal('abc'));\n\
             rq.test('gzip round trip', () => rq.expect(zlib.gunzipSync(zlib.gzipSync(Buffer.from('round trip'))).toString('utf8'))\n\
                 .to.equal('round trip'));\n\
             rq.test('deflate round trip', () => rq.expect(zlib.inflateSync(zlib.deflateSync(Buffer.from('squeeze'))).toString('utf8'))\n\
                 .to.equal('squeeze'));\n\
             rq.test('randomUUID shape', () => rq.expect(crypto.randomUUID()).to.match(/^[0-9a-f]{{8}}-[0-9a-f]{{4}}-4[0-9a-f]{{3}}-[89ab][0-9a-f]{{3}}-[0-9a-f]{{12}}$/));\n\
             rq.test('randomBytes length', () => rq.expect(crypto.randomBytes(16).length).to.equal(16));\n\
             rq.test('a vendored package', () => rq.expect(_.chunk([1,2,3,4], 2)).to.deep.equal([[1,2],[3,4]]));\n",
            stub.base
        ),
    );
    let (_, err, code) = f.run(&["r", "builtins"]);
    assert_eq!(code, 0, "on the {} engine:\n{err}", engine.label());
    assert!(
        err.contains("11/11 test(s) passed"),
        "on the {} engine:\n{err}",
        engine.label()
    );
}

/// A refused require says WHY — the guided message from the package's own classification,
/// which travels in the guest bundle — rather than "not found". `require('fs')` cannot work in
/// a sandbox, and saying so is the difference between a dead end and a decision.
fn case_a_refused_package_says_why(engine: Engine) {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new(engine.clone());
    f.write(
        "nope",
        &format!(
            "---\nurl: {}/x\n---\n\n-- post --\n\nconst fs = require('fs');\n",
            stub.base
        ),
    );
    let (_, err, _) = f.run(&["r", "nope"]);
    assert!(
        err.contains("cannot be used in Safe mode"),
        "on the {} engine, expected the guided refusal:\n{err}",
        engine.label()
    );
}

/// A Postman script, unmodified, running on rq's engine.
///
/// The collection is read in place, so this is the whole path a person takes: export from
/// Postman, point rq at the file, run it. The script says `pm.*`; the runtime speaks `rq.*`;
/// the file records `script_dialect: pm` and the transform reconciles the two at execution.
fn case_a_postman_script_runs_unmodified(engine: Engine) {
    let stub = Stub::start(1, |_| (200, "OK", "{\"id\":7}".into()));
    let f = Fixture::new(engine.clone());
    std::fs::write(
        f.dir.path().join("acme.postman_collection.json"),
        format!(
            r#"{{ "info": {{ "name": "Acme", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" }},
                 "item": [{{ "name": "thing", "request": {{ "method": "GET", "url": {{ "raw": "{}/thing" }} }},
                   "event": [{{ "listen": "test", "script": {{ "exec": [
                     "pm.environment.set('id', String(pm.response.json().id));",
                     "console.log('ran on', pm.response.code);",
                     "pm.test('is 200', function () {{ pm.expect(pm.response.code).to.eql(200); }});",
                     "pm.test('has an id', function () {{ pm.expect(pm.response.json().id).to.eql(7); }});"
                   ] }} }}] }}] }}"#,
            stub.base
        ),
    )
    .unwrap();

    let (_, err, code) = f.run(&["--project", "./acme.postman_collection.json", "r", "thing"]);
    assert_eq!(code, 0, "on the {} engine:\n{err}", engine.label());
    assert!(err.contains("2/2 test(s) passed"), "{err}");
    assert!(
        err.contains("ran on 200"),
        "console.log should reach the step:\n{err}"
    );
}

/// Postman v1.0 — no `pm` at all: `tests['x'] = expr`, `responseCode`, `responseBody`. These
/// are the collections that have been sitting in repositories since 2015, and they are the
/// reason the transform matters more than a `pm.` → `rq.` rename would.
fn case_a_postman_v1_script_runs_unmodified(engine: Engine) {
    let stub = Stub::start(1, |_| (200, "OK", "hello".into()));
    let f = Fixture::new(engine.clone());
    std::fs::write(
        f.dir.path().join("old.json"),
        format!(
            r#"{{ "id": "c1", "name": "Old", "requests": [
                 {{ "id": "r1", "name": "thing", "method": "GET", "url": "{}/thing",
                    "tests": "tests['is 200'] = responseCode.code === 200;\ntests['said hello'] = responseBody === 'hello';" }} ] }}"#,
            stub.base
        ),
    )
    .unwrap();

    let (_, err, code) = f.run(&["--from", "postman", "--project", "./old.json", "r", "thing"]);
    assert_eq!(code, 0, "on the {} engine:\n{err}", engine.label());
    assert!(err.contains("2/2 test(s) passed"), "{err}");
}

/// The dialect is a document field, so it works in a hand-written file too: paste a Postman
/// script into an rq request, say what it is, and it runs. Nothing about this is special to
/// having been converted.
fn case_script_dialect_is_something_you_can_write(engine: Engine) {
    let stub = Stub::start(1, |_| (200, "OK", "{\"ok\":true}".into()));
    let f = Fixture::new(engine.clone());
    f.write(
        "thing",
        &format!(
            "---\nurl: {}/thing\nscript_dialect: pm\n---\n\n-- post --\n\n\
             pm.test('pasted straight from Postman', function () {{ \
               pm.expect(pm.response.code).to.eql(200); }});\n",
            stub.base
        ),
    );

    let (_, err, code) = f.run(&["r", "thing"]);
    assert_eq!(code, 0, "on the {} engine:\n{err}", engine.label());
    assert!(err.contains("1/1 test(s) passed"), "{err}");
}

/// A Bruno collection's scripts, running unmodified.
///
/// Bruno is reconciled by a runtime shim rather than a source rewrite — `bru`/`req`/`res` are
/// objects, so they can simply exist — and this is the proof that the mapping reaches all the
/// way through: a variable set in `script:pre-request` becomes a header on the wire, and the
/// `tests {}` block's bare `test`/`expect` assert against the real response.
fn case_a_bruno_script_runs_unmodified(engine: Engine) {
    let stub = Stub::start(1, |_| (200, "OK", "{\"ok\":true}".into()));
    let f = Fixture::new(engine.clone());
    let c = f.dir.path().join("collection");
    std::fs::create_dir_all(&c).unwrap();
    std::fs::write(
        c.join("bruno.json"),
        r#"{ "version": "1", "name": "demo", "type": "collection" }"#,
    )
    .unwrap();
    std::fs::write(
        c.join("thing.bru"),
        format!(
            "meta {{\n  name: thing\n  type: http\n}}\n\
             get {{\n  url: {}/thing\n}}\n\
             script:pre-request {{\n  \
               bru.setVar(\"who\", \"amitu\");\n  \
               req.setHeader(\"X-From\", bru.getVar(\"who\"));\n}}\n\
             script:post-response {{\n  \
               bru.setEnvVar(\"last_status\", String(res.getStatus()));\n}}\n\
             tests {{\n  \
               test(\"status is 200\", function () {{ expect(res.getStatus()).to.equal(200); }});\n  \
               test(\"body came through\", function () {{ expect(res.getBody().ok).to.equal(true); }});\n\
             }}\n",
            stub.base
        ),
    )
    .unwrap();

    let (_, err, code) = f.run(&["--project", "./collection", "r", "thing"]);
    assert_eq!(code, 0, "on the {} engine:\n{err}", engine.label());
    assert!(err.contains("2/2 test(s) passed"), "{err}");
    // The pre-request script's header actually left the process.
    assert_eq!(stub.next().header("x-from"), Some("amitu"));
}

/// A Bruno API rq has not implemented **throws, by name**.
///
/// The alternative — returning `undefined` and carrying on — turns a missing feature into a
/// wrong answer three lines later, which is the single failure mode this project is organised
/// against. Better to stop at the call that cannot work.
fn case_an_unimplemented_bruno_api_says_so(engine: Engine) {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new(engine.clone());
    f.write(
        "thing",
        &format!(
            "---\nurl: {}/thing\nscript_dialect: bru\n---\n\n-- post --\n\n\
             bru.cwd();\n",
            stub.base
        ),
    );

    let (_, err, _) = f.run(&["r", "thing"]);
    assert!(
        err.contains("bru.cwd()") && err.contains("rather than continuing"),
        "on the {} engine, the failure should name the call:\n{err}",
        engine.label()
    );
}

// The drivers: each case, on every engine this machine can run.

#[test]
fn a_post_response_script_really_runs() {
    for engine in engines() {
        eprintln!(
            "── a_post_response_script_really_runs on the {} engine",
            engine.label()
        );
        case_a_post_response_script_really_runs(engine);
    }
}

#[test]
fn a_failing_assertion_fails_the_run() {
    for engine in engines() {
        eprintln!(
            "── a_failing_assertion_fails_the_run on the {} engine",
            engine.label()
        );
        case_a_failing_assertion_fails_the_run(engine);
    }
}

#[test]
fn a_variable_a_script_sets_reaches_the_next_request() {
    for engine in engines() {
        eprintln!(
            "── a_variable_a_script_sets_reaches_the_next_request on the {} engine",
            engine.label()
        );
        case_a_variable_a_script_sets_reaches_the_next_request(engine);
    }
}

#[test]
fn a_pre_request_script_can_change_the_request() {
    for engine in engines() {
        eprintln!(
            "── a_pre_request_script_can_change_the_request on the {} engine",
            engine.label()
        );
        case_a_pre_request_script_can_change_the_request(engine);
    }
}

#[test]
fn a_script_that_throws_is_reported_without_losing_the_response() {
    for engine in engines() {
        eprintln!(
            "── a_script_that_throws_is_reported_without_losing_the_response on the {} engine",
            engine.label()
        );
        case_a_script_that_throws_is_reported_without_losing_the_response(engine);
    }
}

#[test]
fn the_node_builtins_are_the_same_builtins() {
    for engine in embedded_only() {
        eprintln!(
            "── the_node_builtins_are_the_same_builtins on the {} engine",
            engine.label()
        );
        case_the_node_builtins_are_the_same_builtins(engine);
    }
}

#[test]
fn a_refused_package_says_why() {
    for engine in embedded_only() {
        eprintln!(
            "── a_refused_package_says_why on the {} engine",
            engine.label()
        );
        case_a_refused_package_says_why(engine);
    }
}

#[test]
fn a_postman_script_runs_unmodified() {
    for engine in engines() {
        eprintln!(
            "── a_postman_script_runs_unmodified on the {} engine",
            engine.label()
        );
        case_a_postman_script_runs_unmodified(engine);
    }
}

#[test]
fn a_postman_v1_script_runs_unmodified() {
    for engine in engines() {
        eprintln!(
            "── a_postman_v1_script_runs_unmodified on the {} engine",
            engine.label()
        );
        case_a_postman_v1_script_runs_unmodified(engine);
    }
}

#[test]
fn script_dialect_is_something_you_can_write() {
    for engine in engines() {
        eprintln!(
            "── script_dialect_is_something_you_can_write on the {} engine",
            engine.label()
        );
        case_script_dialect_is_something_you_can_write(engine);
    }
}

#[test]
fn a_bruno_script_runs_unmodified() {
    for engine in engines() {
        eprintln!(
            "── a_bruno_script_runs_unmodified on the {} engine",
            engine.label()
        );
        case_a_bruno_script_runs_unmodified(engine);
    }
}

#[test]
fn an_unimplemented_bruno_api_says_so() {
    for engine in engines() {
        eprintln!(
            "── an_unimplemented_bruno_api_says_so on the {} engine",
            engine.label()
        );
        case_an_unimplemented_bruno_api_says_so(engine);
    }
}
