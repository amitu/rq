//! Reading somebody else's collection, in place.
//!
//! `rq` can run a Postman export or a Bruno tree without converting it first: cross-q turns
//! it into the rq project it describes, in memory, and the tree is built from that. These
//! tests are about the two claims that makes — that the requests really run, and that
//! **nothing is written**, because reading a file someone dropped in a folder must not leave
//! anything behind in that folder.

mod support;

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use support::Stub;

const BIN: &str = env!("CARGO_BIN_EXE_rq");

/// A Postman v2.1 collection pointed at `base`.
fn postman(base: &str) -> String {
    format!(
        r#"{{
  "info": {{ "name": "Acme", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" }},
  "item": [
    {{ "name": "health", "request": {{ "method": "GET", "url": {{ "raw": "{base}/health" }} }} }},
    {{ "name": "users", "item": [
      {{ "name": "list users", "request": {{ "method": "GET", "url": {{ "raw": "{base}/users?page=1" }},
         "header": [{{ "key": "Accept", "value": "application/json" }}] }} }}
    ]}}
  ]
}}"#
    )
}

fn run(dir: &Path, args: &[&str]) -> (String, String, i32) {
    // `--no-console` only exists where a console does; `rq env` would reject it.
    let extra: &[&str] = if args.first() == Some(&"env") {
        &["--color=never"]
    } else {
        &["--color=never", "--no-console"]
    };
    let out = Command::new(BIN)
        .args(args)
        .args(extra)
        .current_dir(dir)
        .env_remove("RQ_PROJECT")
        .env("RQ_SCRIPT_ENGINE", "/nonexistent/cross-q-context")
        .env("NO_COLOR", "1")
        .output()
        .expect("running rq");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// Every path under `dir`, so a test can prove the run added nothing.
fn tree(dir: &Path) -> BTreeSet<String> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeSet<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            }
            out.insert(p.strip_prefix(base).unwrap().to_string_lossy().to_string());
        }
    }
    let mut out = BTreeSet::new();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn a_postman_file_is_a_project_without_being_converted_first() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1, |_| (200, "OK", "{\"ok\":true}".into()));
    let file = dir.path().join("acme.postman_collection.json");
    std::fs::write(&file, postman(&stub.base)).unwrap();
    let before = tree(dir.path());

    // Bare listing: no rq.toml anywhere, and it still finds the collection sitting here.
    let (out, err, code) = run(dir.path(), &["l"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("health"), "{out}");
    assert!(out.contains("list-users"), "{out}");
    assert!(
        err.contains("as postman"),
        "the run should say what it read and as what:\n{err}"
    );

    // And it runs.
    let (_, err, code) = run(dir.path(), &["r", "health"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(stub.next().path, "/health");

    // The whole point: the folder is exactly as we found it.
    assert_eq!(
        tree(dir.path()),
        before,
        "reading a collection wrote something into the directory"
    );
}

#[test]
fn a_query_string_goes_out_once() {
    // The converter used to leave the query in `url:` *and* emit a `query:` block, and rq
    // appends the latter — so this went out as `?page=1&page=1`. It reached the socket
    // wrong, which is the only place it could be seen.
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1, |_| (200, "OK", "[]".into()));
    std::fs::write(
        dir.path().join("acme.postman_collection.json"),
        postman(&stub.base),
    )
    .unwrap();

    let (_, err, code) = run(dir.path(), &["r", "list-users"]);
    assert_eq!(code, 0, "{err}");
    let seen = stub.next();
    assert_eq!(seen.path, "/users?page=1", "the query was sent twice");
    assert_eq!(seen.header("accept"), Some("application/json"));
}

#[test]
fn a_named_file_beats_whatever_is_in_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(0, |_| (200, "OK", String::new()));
    std::fs::write(
        dir.path().join("one.postman_collection.json"),
        postman(&stub.base),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("two.postman_collection.json"),
        postman(&stub.base).replace("\"health\"", "\"heartbeat\""),
    )
    .unwrap();

    // Two candidates and no way to choose: say so, and name them, rather than picking one.
    let (_, err, code) = run(dir.path(), &["l"]);
    assert_ne!(code, 0);
    assert!(err.contains("one.postman_collection.json"), "{err}");
    assert!(err.contains("two.postman_collection.json"), "{err}");

    // Naming one resolves it.
    let (out, err, code) = run(dir.path(), &["l", "two.postman_collection.json"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("heartbeat"), "{out}");
}

#[test]
fn a_curl_file_is_a_collection_too() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1, |_| (200, "OK", "hi".into()));
    std::fs::write(
        dir.path().join("calls.txt"),
        format!(
            "curl -X POST {}/login -d '{{\"u\":\"amitu\"}}'\n",
            stub.base
        ),
    )
    .unwrap();

    let (out, err, code) = run(dir.path(), &["l"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("as curl"), "{err}");
    assert!(out.contains("POST"), "{out}");

    let name = out
        .lines()
        .find(|l| l.contains("POST"))
        .and_then(|l| {
            l.split_whitespace()
                .find(|w| !w.starts_with('├') && !w.starts_with('└'))
        })
        .expect("a request name in the listing")
        .to_string();
    let (_, err, code) = run(dir.path(), &["r", &name]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(stub.next().body, "{\"u\":\"amitu\"}");
}

#[test]
fn an_unreadable_file_is_not_mistaken_for_a_collection() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.json"), r#"{"just":"some json"}"#).unwrap();

    // JSON that says nothing about being a collection is not one — the error is still the
    // ordinary "no project here", not a confusing parse failure.
    let (_, err, code) = run(dir.path(), &["l"]);
    assert_ne!(code, 0);
    assert!(err.contains("no rq project"), "{err}");
}

#[test]
fn the_format_can_be_forced_when_the_file_does_not_say() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(0, |_| (200, "OK", String::new()));
    // A Postman **v1** collection: perfectly readable, but it carries neither `info` nor
    // `_postman_id`, so looking at it proves nothing and rq should not guess.
    let v1 = format!(
        r#"{{ "id": "c1", "name": "Old", "requests": [
             {{ "id": "r1", "name": "health", "method": "GET", "url": "{}/health" }} ] }}"#,
        stub.base
    );
    std::fs::write(dir.path().join("mystery.json"), v1).unwrap();

    let (_, err, code) = run(dir.path(), &["l"]);
    assert_ne!(code, 0, "{err}");

    let (out, err, code) = run(dir.path(), &["l", "mystery.json", "--from", "postman"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("health"), "{out}");
}

#[test]
fn a_collection_read_in_place_has_nowhere_to_save_an_active_environment() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(0, |_| (200, "OK", String::new()));
    std::fs::write(
        dir.path().join("acme.postman_collection.json"),
        postman(&stub.base),
    )
    .unwrap();
    let before = tree(dir.path());

    let (_, err, code) = run(dir.path(), &["env", "switch", "staging"]);
    assert_ne!(code, 0);
    assert!(
        err.contains("rq import") || err.contains("no environment"),
        "the refusal should point somewhere useful:\n{err}"
    );
    assert_eq!(tree(dir.path()), before, "an `.rq/` was left behind");
}
