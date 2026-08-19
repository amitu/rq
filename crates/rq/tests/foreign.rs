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
    // `--no-console` only exists where a console does; `rq env`, `rq check` and `rq fmt`
    // would reject it.
    let extra: &[&str] = if matches!(args.first(), Some(&"env") | Some(&"check") | Some(&"fmt"))
        || args.get(2) == Some(&"check")
    {
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

// --- what `rq check` does with somebody else's collection --------------------------------

/// A Bruno collection: one good request, one file that does not parse.
fn bruno_tree(dir: &Path, base: &str) {
    let c = dir.join("collection");
    std::fs::create_dir_all(&c).unwrap();
    std::fs::write(
        c.join("bruno.json"),
        r#"{ "version": "1", "name": "demo", "type": "collection" }"#,
    )
    .unwrap();
    std::fs::write(
        c.join("ok.bru"),
        format!("meta {{\n  name: ok\n  type: http\n}}\nget {{\n  url: {base}/ok\n}}\n"),
    )
    .unwrap();
    // Someone hand-edited this one and lost a brace — the everyday `.bru` failure.
    std::fs::write(
        c.join("broken.bru"),
        "meta {\n  name: broken\n  type: http\n\nget {\n  url: https://x.test/broken\n",
    )
    .unwrap();
}

#[test]
fn check_names_the_source_file_that_would_not_parse() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(0, |_| (200, "OK", String::new()));
    bruno_tree(dir.path(), &stub.base);

    // rq does not validate `.bru` syntax itself — cross-q does, while reading. What matters
    // is that rq says so instead of loading one request, reporting "1 dropped", and calling
    // the project healthy.
    let (out, err, code) = run(dir.path(), &["--project", "./collection", "check"]);
    let text = format!("{out}{err}");
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("broken.bru"),
        "the file should be named:\n{text}"
    );
    assert!(!text.contains("nothing to report"), "{text}");
}

#[test]
fn check_does_not_cry_wolf_over_a_collection_that_merely_lost_detail() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(0, |_| (200, "OK", String::new()));
    std::fs::write(
        dir.path().join("acme.postman_collection.json"),
        postman(&stub.base),
    )
    .unwrap();

    // Everything the converter mentions is worth knowing, but a request that came through
    // with less than it had is a warning, not an error — otherwise every real collection
    // fails `check` and nobody runs it twice.
    let (out, err, code) = run(dir.path(), &["check"]);
    let text = format!("{out}{err}");
    assert_eq!(code, 0, "{text}");
    assert!(!text.contains("✗"), "{text}");
}

#[test]
fn a_corrupt_collection_says_which_file_and_why() {
    let dir = tempfile::tempdir().unwrap();
    // Truncated mid-array: the export was interrupted, or an editor mangled it.
    std::fs::write(
        dir.path().join("acme.postman_collection.json"),
        r#"{"info":{"name":"x","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},"item":["#,
    )
    .unwrap();

    let (out, err, code) = run(dir.path(), &["check"]);
    let text = format!("{out}{err}");
    assert_ne!(code, 0);
    // "no rq project found" would be true and useless with the file sitting right there.
    assert!(text.contains("acme.postman_collection.json"), "{text}");
    assert!(text.contains("JSON"), "{text}");
}

/// A foreign-dialect script, carried faithfully and **run**.
///
/// This test used to assert the opposite — that a `bru.*` script failed with `bru is not
/// defined` — and said in its own comment that landing the dialect support should break it.
/// It did. What it pins now is the whole path: read a Bruno collection in place, and its
/// script runs against the same engine an rq script does.
#[test]
fn a_foreign_dialect_script_runs() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1, |_| (200, "OK", "{\"ok\":true}".into()));
    let c = dir.path().join("collection");
    std::fs::create_dir_all(&c).unwrap();
    std::fs::write(
        c.join("bruno.json"),
        r#"{ "version": "1", "name": "d", "type": "collection" }"#,
    )
    .unwrap();
    std::fs::write(
        c.join("get.bru"),
        format!(
            "meta {{\n  name: get\n  type: http\n}}\nget {{\n  url: {}/thing\n}}\n\
             tests {{\n  test(\"ran\", function () {{ expect(res.getStatus()).to.equal(200); }});\n}}\n",
            stub.base
        ),
    )
    .unwrap();

    // The compiled-in engine, not the pinned-off one the other cases use.
    let out = Command::new(BIN)
        .args(["--project", "./collection", "r", "get", "--color=never", "--no-console"])
        .current_dir(dir.path())
        .env_remove("RQ_PROJECT")
        .env_remove("RQ_SCRIPT_ENGINE")
        .env("NO_COLOR", "1")
        .output()
        .expect("running rq");
    let code = out.status.code().unwrap_or(-1);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(stub.next().path, "/thing");
    assert!(text.contains("1/1 test(s) passed"), "{text}");
}
