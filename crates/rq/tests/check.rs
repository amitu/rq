//! `rq check` and `rq fmt`.
//!
//! Every case here plants one specific defect and asserts it is reported — a checker is only
//! worth having if it fails on the thing it claims to catch, and "it printed nothing" is the
//! easiest possible bug to ship. The last two go the other way: a project that is fine must
//! stay quiet, or people learn to ignore the output.

mod support;

use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rq");

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        rq::project::init(dir.path()).unwrap();
        Fixture { dir }
    }

    fn write(&self, name: &str, contents: &str) {
        let path = self.dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(name)).unwrap()
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let out = Command::new(BIN)
            .args(args)
            .args(["--color=never"])
            .current_dir(self.dir.path())
            .env_remove("RQ_PROJECT")
            .env("NO_COLOR", "1")
            .output()
            .expect("running rq");
        (
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
            out.status.code().unwrap_or(-1),
        )
    }
}

fn path(dir: &Path, name: &str) -> std::path::PathBuf {
    dir.join(name)
}

#[test]
fn a_parent_that_does_not_exist_is_an_error() {
    let f = Fixture::new();
    f.write("login.md", "---\nurl: https://api.test/login\n---\n");
    // Renaming `login` to `signin` and forgetting this is the everyday version of this bug.
    f.write(
        "me.md",
        "---\nurl: https://api.test/me\nparents: [signin]\n---\n",
    );

    let (out, code) = f.run(&["check"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("parents: [signin]"), "{out}");
    assert!(out.contains("error"), "{out}");
}

#[test]
fn parents_that_loop_are_an_error_naming_the_loop() {
    let f = Fixture::new();
    f.write("a.md", "---\nurl: https://api.test/a\nparents: [b]\n---\n");
    f.write("b.md", "---\nurl: https://api.test/b\nparents: [a]\n---\n");

    let (out, code) = f.run(&["check"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("loops"), "{out}");
    assert!(out.contains("→"), "the loop should be spelled out:\n{out}");
}

#[test]
fn a_capture_path_that_can_never_match_is_an_error() {
    let f = Fixture::new();
    f.write(
        "login.md",
        "---\nurl: https://api.test/login\ncapture:\n  token: resposne.access_token\n---\n",
    );

    let (out, code) = f.run(&["check"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("resposne"), "{out}");
    assert!(
        out.contains("response"),
        "the hint should list the roots:\n{out}"
    );
}

#[test]
fn a_view_template_that_stopped_parsing_is_an_error() {
    let f = Fixture::new();
    f.write(
        "thing.md",
        "---\nurl: https://api.test/thing\n---\n\n-- view --\n\n{% for x in response %}{{ x }}\n",
    );

    // Without this you find out at the very end of a run, after spending the request.
    let (out, code) = f.run(&["check"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("`-- view --` does not parse"), "{out}");
}

#[test]
fn a_body_file_that_is_not_there_is_an_error() {
    let f = Fixture::new();
    f.write(
        "upload.md",
        "---\nmethod: POST\nurl: https://api.test/up\nfile: payload.bin\n---\n",
    );

    let (out, code) = f.run(&["check"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("payload.bin"), "{out}");
}

#[test]
fn a_variable_nothing_provides_is_a_warning_not_an_error() {
    let f = Fixture::new();
    f.write(
        "me.md",
        "---\nurl: https://api.test/me\nheaders:\n  Authorization: Bearer {{TOKEN}}\n---\n",
    );

    // A run does not fail on this — it notes it and sends `Bearer {{TOKEN}}` as written, which
    // comes back 401 and reads like a credentials problem. So: reported, but not an error,
    // because check must not disagree with what a run does.
    let (out, code) = f.run(&["check"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("TOKEN"), "{out}");
    assert!(out.contains("warning"), "{out}");

    // Unless you ask for it to matter, which is what CI wants.
    let (_, code) = f.run(&["check", "--strict"]);
    assert_eq!(code, 1);
}

#[test]
fn a_variable_the_environment_declares_is_not_a_warning() {
    let f = Fixture::new();
    f.write(
        "me.md",
        "---\nurl: '{{api}}/me'\nheaders:\n  Authorization: Bearer {{TOKEN}}\n---\n",
    );
    // Declared, not necessarily set: `{ env: TOKEN }` provides the name whether or not this
    // shell exports it, and a check that depended on that would say different things in CI.
    f.write(
        "env/live.md",
        "---\nvars:\n  api: https://api.test\n  TOKEN: { env: TOKEN, secret: true }\n---\n",
    );

    let (out, code) = f.run(&["check"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("nothing to report"), "{out}");

    // Naming a different environment narrows the question, and then it is missing again.
    f.write(
        "env/other.md",
        "---\nvars:\n  api: https://other.test\n---\n",
    );
    let (out, code) = f.run(&["check", "-e", "other", "--strict"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("TOKEN"), "{out}");
}

#[test]
fn form_fields_are_variables_and_captures_reach_dependents() {
    let f = Fixture::new();
    f.write(
        "login.md",
        "---\nmethod: POST\nurl: https://api.test/login\ncapture:\n  token: response.access_token\n---\n",
    );
    f.write(
        "post.md",
        "---\nmethod: POST\nurl: https://api.test/posts\nparents: [login]\n\
         headers:\n  Authorization: Bearer {{token}}\n---\n\n\
         -- form --\n\ntext: { label: \"Say something\" }\n\n\
         -- body --\n\n{\"text\": \"{{text}}\"}\n",
    );

    // `{{token}}` comes from the parent's capture, `{{text}}` from this request's own form.
    // Both are ordinary and neither is a finding.
    let (out, code) = f.run(&["check", "--strict"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("nothing to report"), "{out}");
}

#[test]
fn a_file_that_does_not_parse_is_reported_with_its_name() {
    let f = Fixture::new();
    f.write(
        "broken.md",
        "---\nurl: https://api.test/x\nheaders: [1, 2\n---\n",
    );

    let (out, code) = f.run(&["check"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("broken"), "{out}");
}

#[test]
fn findings_come_out_as_json_for_ci() {
    let f = Fixture::new();
    f.write(
        "me.md",
        "---\nurl: https://api.test/me?t={{TOKEN}}\nparents: [nope]\n---\n",
    );

    let (out, code) = f.run(&["check", "--json"]);
    assert_eq!(code, 1, "{out}");
    let v: serde_json::Value = serde_json::from_str(out.trim()).expect("valid JSON");
    assert_eq!(v["errors"], 1, "{v:#}");
    assert_eq!(v["warnings"], 1, "{v:#}");
    let levels: Vec<&str> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["level"].as_str().unwrap())
        .collect();
    assert_eq!(levels, vec!["error", "warning"], "worst first: {v:#}");
}

#[test]
fn a_healthy_project_says_so_and_says_nothing_else() {
    let f = Fixture::new();
    f.write("me.md", "---\nurl: https://api.test/me\n---\n");

    let (out, code) = f.run(&["check"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("nothing to report"), "{out}");
    assert!(!out.contains("warning"), "{out}");
}

// --- rq fmt ------------------------------------------------------------------------------

#[test]
fn fmt_rewrites_a_hand_edited_file_and_is_idempotent() {
    let f = Fixture::new();
    // Written the way a person types it: keys out of order, inconsistent quoting, section
    // spacing that does not match what rq writes.
    f.write(
        "me.md",
        "---\nurl:    \"https://api.test/me\"\nmethod: GET\n---\n-- description --\nWho am I\n",
    );

    let (out, code) = f.run(&["fmt"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("me"), "{out}");

    let once = f.read("me.md");
    assert!(once.starts_with("---\nmethod: GET\n"), "{once}");

    // Formatting twice must be formatting once, or `--check` can never be trusted.
    let (out, code) = f.run(&["fmt"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("already formatted"), "{out}");
    assert_eq!(f.read("me.md"), once);
}

#[test]
fn fmt_check_reports_without_writing() {
    let f = Fixture::new();
    let messy = "---\nurl:    \"https://api.test/me\"\nmethod: GET\n---\n";
    f.write("me.md", messy);

    let (out, code) = f.run(&["fmt", "--check"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("would change"), "{out}");
    assert_eq!(f.read("me.md"), messy, "--check wrote to the file");

    let (_, code) = f.run(&["fmt"]);
    assert_eq!(code, 0);
    let (_, code) = f.run(&["fmt", "--check"]);
    assert_eq!(code, 0, "formatting should settle after one pass");
}

#[test]
fn fmt_keeps_what_this_build_does_not_understand() {
    let f = Fixture::new();
    f.write(
        "me.md",
        "---\nurl: https://api.test/me\nfuture_key: keep me\n---\n\n\
         -- description --\n\nHello\n\n-- whatever --\n\nkeep this section too\n",
    );

    let (out, code) = f.run(&["fmt"]);
    assert_eq!(code, 0, "{out}");
    let after = f.read("me.md");
    // Formatting a file must never be how you discover this build dropped something.
    assert!(after.contains("future_key: keep me"), "{after}");
    assert!(after.contains("-- whatever --"), "{after}");
    assert!(after.contains("keep this section too"), "{after}");
}

#[test]
fn fmt_refuses_on_a_collection_rq_is_only_reading() {
    let dir = tempfile::tempdir().unwrap();
    let stub = support::Stub::start(0, |_| (200, "OK", String::new()));
    std::fs::write(
        path(dir.path(), "acme.postman_collection.json"),
        format!(
            r#"{{ "info": {{ "name": "Acme", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" }},
                 "item": [{{ "name": "health", "request": {{ "method": "GET", "url": {{ "raw": "{}/health" }} }} }}] }}"#,
            stub.base
        ),
    )
    .unwrap();

    let out = Command::new(BIN)
        .args(["fmt", "--color=never"])
        .current_dir(dir.path())
        .env_remove("RQ_PROJECT")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("rq import"), "{text}");
    // And the file it was reading is untouched.
    assert!(
        std::fs::read_to_string(path(dir.path(), "acme.postman_collection.json"))
            .unwrap()
            .contains("\"info\"")
    );
}
