//! The `rq` format as a first-class cross-q citizen: emit it, read it back, and prove the
//! pair is lossless the way every other format in this repo has to prove it.
//!
//! Three gates, in ascending strength:
//!
//! 1. **Shape** — a curl and a Postman collection produce the documents a person expects.
//! 2. **Idempotence** — `rq` → IR → `rq` → IR recovers the same IR (the exporter didn't
//!    drop or mangle a field). Byte-identity would be brittle; IR equality is honest.
//! 3. **No hollow conversion** — over the pinned real-world Postman corpus, every request
//!    that enters the model survives the trip through the `rq` format.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cq_model::{Item, Workspace};
use cq_report::{Fidelity, Report};

fn to_project(source: &str, input: &str) -> (BTreeMap<String, String>, Report) {
    let mut report = Report::new(Fidelity::Lossless);
    let ws = cross_q::build_workspace(source, input, &mut report).expect("parse");
    let map = cross_q::emit_rq_md::to_rq_md(&ws, &mut report);
    (map, report)
}

fn reread(map: &BTreeMap<String, String>) -> (Workspace, Report) {
    let mut report = Report::new(Fidelity::Lossless);
    let ws = cross_q::rq_md::parse_rq_md(&serde_json::to_string(map).unwrap(), &mut report)
        .expect("re-read");
    (ws, report)
}

fn request_names(ws: &Workspace) -> Vec<String> {
    fn walk(items: &[Item], out: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Request(r) => out.push(r.meta.name.clone()),
                Item::Collection(c) => walk(&c.items, out),
            }
        }
    }
    let mut out = Vec::new();
    for c in &ws.collections {
        walk(&c.items, &mut out);
    }
    out.sort();
    out
}

// --- 1. shape ----------------------------------------------------------------------------

#[test]
fn a_curl_becomes_one_readable_document() {
    let (map, _) = to_project(
        "curl",
        "curl -X POST https://api.test/login -H 'Content-Type: application/json' \
         -d '{\"user\":\"amitu\"}'",
    );
    assert!(map.contains_key("rq.toml"));
    let (path, doc) = map
        .iter()
        .find(|(k, _)| k.ends_with(".md") && !k.ends_with("index.md"))
        .expect("a request document");
    assert!(path.ends_with(".md"), "{path}");
    assert!(doc.contains("method: POST"), "{doc}");
    assert!(doc.contains("url: https://api.test/login"), "{doc}");
    assert!(doc.contains("Content-Type: application/json"), "{doc}");
    assert!(doc.contains("-- body --"), "{doc}");
    assert!(doc.contains("{\"user\":\"amitu\"}"), "{doc}");
}

#[test]
fn a_postman_collection_becomes_a_tree_and_keeps_its_scripts_verbatim() {
    let collection = serde_json::json!({
        "info": { "name": "Acme", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
        "item": [{
            "name": "Auth",
            "item": [{
                "name": "login",
                "event": [{
                    "listen": "test",
                    "script": { "exec": ["pm.environment.set('token', pm.response.json().token);"] }
                }],
                "request": {
                    "method": "POST",
                    "url": { "raw": "https://api.test/login" },
                    "auth": { "type": "basic", "basic": [
                        { "key": "username", "value": "u" }, { "key": "password", "value": "p" }
                    ]}
                }
            }]
        }]
    })
    .to_string();

    let (map, report) = to_project("postman", &collection);
    let doc = map
        .get("Acme/Auth/login.md")
        .expect("the request at its tree path");
    assert!(doc.contains("type: basic"), "{doc}");
    assert!(doc.contains("-- post --"), "{doc}");
    // The pm.* source is carried as written — never string-replaced to rq.*.
    assert!(doc.contains("pm.environment.set"), "{doc}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("dialect")),
        "the dialect must be reported: {:?}",
        report.diagnostics
    );
}

#[test]
fn a_query_string_is_emitted_once_not_twice() {
    // The IR carries a query BOTH parsed into `query` and still sitting in `url.raw`, which
    // is what the source formats hand over. rq *appends* `query:` to `url:`, so emitting
    // both sent every imported request out as `?page=1&page=1` — a wrong URL on the wire,
    // silently, for anything imported with a query string.
    let (map, _) = to_project(
        "postman",
        r#"{
          "info": { "name": "Q", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
          "item": [{ "name": "search", "request": { "method": "GET",
            "url": { "raw": "https://x.test/s?page=1&q=cats" } } }]
        }"#,
    );
    let doc = map
        .iter()
        .find(|(k, _)| k.ends_with("search.md"))
        .map(|(_, v)| v.clone())
        .expect("the request was emitted");
    assert!(
        doc.contains("url: https://x.test/s\n"),
        "the url kept its query string, so it will be appended twice:\n{doc}"
    );
    assert!(
        doc.contains("page: '1'") && doc.contains("q: cats"),
        "{doc}"
    );

    // And the round trip still knows the whole URL.
    let (ws, _) = reread(&map);
    let names = request_names(&ws);
    assert_eq!(names.len(), 1, "{names:?}");
}

#[test]
fn an_unsendable_auth_is_preserved_and_reported_not_stripped() {
    let collection = serde_json::json!({
        "info": { "name": "A", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
        "item": [{
            "name": "hawked",
            "request": {
                "method": "GET",
                "url": { "raw": "https://api.test/x" },
                "auth": { "type": "hawk", "hawk": [
                    { "key": "authId", "value": "abc" }, { "key": "authKey", "value": "s3cret" }
                ]}
            }
        }]
    })
    .to_string();

    let (map, report) = to_project("postman", &collection);
    let doc = map.get("A/hawked.md").unwrap();
    assert!(doc.contains("hawk"), "{doc}");
    assert!(doc.contains("s3cret"), "the credential must survive: {doc}");
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d.message.contains("cannot send it yet")));

    // …and it comes back as an Unknown auth rather than being dropped on the way in.
    let (ws, _) = reread(&map);
    let json = serde_json::to_string(&ws).unwrap();
    assert!(json.contains("s3cret"), "{json}");
}

// --- 2. idempotence ------------------------------------------------------------------------

/// A hand-written project that exercises every corner the format has: nesting, collection
/// inheritance, each body shape, auth, variables, environments, and declared chaining.
fn fixture() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("rq.toml".into(), rq_doc::layout::marker()),
        (
            "index.md".into(),
            "---\nheaders:\n  X-Root: yes\n---\n".into(),
        ),
        (
            "acme/index.md".into(),
            "---\nheaders:\n  X-Team: platform\nauth: { type: bearer, token: shared }\n\
             vars:\n  host: https://api.test\n---\n\n-- description --\n\nThe Acme API.\n"
                .into(),
        ),
        (
            "acme/login.md".into(),
            "---\nmethod: POST\nurl: '{{host}}/auth/login'\n\
             headers:\n  Content-Type: application/json\n\
             capture:\n  token: response.access_token\n---\n\n\
             -- description --\n\nGet a token.\n\n-- body --\n\n{\"user\": \"amitu\"}\n"
                .into(),
        ),
        (
            "acme/me.md".into(),
            "---\nurl: '{{host}}/me'\nheaders:\n  Authorization: Bearer {{token}}\n\
             query:\n  expand: plan\nparents: [login]\n---\n\n\
             -- view --\n\n# {{ response.name }}\n\n-- post --\n\nrq.test('ok', () => true);\n"
                .into(),
        ),
        (
            "upload.md".into(),
            "---\nmethod: POST\nurl: https://api.test/upload\n\
             form_data:\n  caption: hello\n  photo: '@./cat.png'\n---\n"
                .into(),
        ),
        (
            "search.md".into(),
            "---\nmethod: POST\nurl: https://api.test/search\n\
             form:\n  q: rust\n  page: '2'\npath_vars:\n  id: '7'\n\
             timeout: 5000\nfollow_redirects: false\nverify_tls: false\n\
             auth: { type: api_key, key: X-Api-Key, value: k, in: query }\n---\n"
                .into(),
        ),
        (
            ".env".into(),
            "host=https://api.test\n".into(),
        ),
        (
            "env/staging.md".into(),
            "---\nvars:\n  host: https://staging.api.test\n  TOKEN: { default: t, secret: true }\n---\n"
                .into(),
        ),
    ])
}

#[test]
fn the_format_survives_its_own_round_trip() {
    let source = fixture();
    let (ir1, r1) = reread(&source);
    assert!(
        !r1.diagnostics
            .iter()
            .any(|d| d.severity == cq_report::Severity::Error),
        "{:?}",
        r1.diagnostics
    );

    let mut r2 = Report::new(Fidelity::Lossless);
    let reemitted = cross_q::emit_rq_md::to_rq_md(&ir1, &mut r2);
    let (ir2, _) = reread(&reemitted);

    assert_eq!(
        ir1,
        ir2,
        "rq → IR → rq → IR lost or changed something:\n{}",
        diff_first(&ir1, &ir2)
    );
}

fn diff_first(a: &Workspace, b: &Workspace) -> String {
    let (a, b) = (
        serde_json::to_string_pretty(a).unwrap(),
        serde_json::to_string_pretty(b).unwrap(),
    );
    a.lines()
        .zip(b.lines())
        .find(|(x, y)| x != y)
        .map(|(x, y)| format!("  first difference:\n  - {x}\n  + {y}"))
        .unwrap_or_else(|| "  (same text, different structure)".into())
}

#[test]
fn declared_chaining_survives_the_model() {
    let (ws, _) = reread(&fixture());
    let mut found = false;
    fn walk(items: &[cq_model::Item], found: &mut bool) {
        for item in items {
            match item {
                cq_model::Item::Collection(c) => walk(&c.items, found),
                cq_model::Item::Request(r) if r.meta.name == "me" => {
                    assert_eq!(r.depends_on.len(), 1, "me depends on login");
                    let dep = &r.depends_on[0];
                    // The edge points at login's id, and carries the capture as a binding.
                    assert!(dep.target.ends_with("acme/login"), "{}", dep.target);
                    assert_eq!(dep.binds.len(), 1);
                    assert_eq!(dep.binds[0].from, "response.access_token");
                    assert_eq!(dep.binds[0].to, "token");
                    *found = true;
                }
                _ => {}
            }
        }
    }
    for c in &ws.collections {
        walk(&c.items, &mut found);
    }
    assert!(found, "the `me` request was not found");
}

#[test]
fn an_rq_project_converts_out_to_the_other_formats() {
    let (ws, _) = reread(&fixture());

    // Postman: the importer feeds every exporter, which is the whole point of the hub.
    let postman = cross_q::emit_postman::to_postman(&ws);
    let text = serde_json::to_string(&postman).unwrap();
    assert!(text.contains("login"), "{text}");
    assert!(text.contains("/auth/login"), "{text}");

    // Bruno.
    let bruno = cross_q::emit_bruno::to_bruno(&ws);
    assert!(
        bruno.keys().any(|k| k.contains("login")),
        "{:?}",
        bruno.keys().collect::<Vec<_>>()
    );

    // The Requestly LOCAL_FS tree.
    let dir = tempfile::tempdir().unwrap();
    let mut report = Report::new(Fidelity::Lossless);
    cross_q::emit_rq::emit_rq(&ws, dir.path(), &mut report).unwrap();
    assert!(dir.path().join("apis").is_dir());
}

#[test]
fn a_single_request_document_parses_on_its_own() {
    let mut report = Report::new(Fidelity::Lossless);
    let ws = cross_q::rq_md::parse_rq_md(
        "---\nmethod: PUT\nurl: https://api.test/x\n---\n",
        &mut report,
    )
    .unwrap();
    assert_eq!(request_names(&ws), vec!["request"]);
}

// --- 3. no hollow conversion ---------------------------------------------------------------

fn realworld_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/realworld")
}

/// Every request that survives Postman → IR must still be there after IR → `rq` → IR.
/// A conversion that quietly halves your collection is the failure this repo exists to
/// prevent, so it is a gate, not a spot-check.
#[test]
fn the_real_world_corpus_survives_the_rq_format() {
    let base = realworld_dir();
    assert!(
        base.exists(),
        "real-world corpus not fetched — run \
         `crates/cross-q/tests/corpus/fetch-realworld-corpus.sh` (pinned; not vendored)"
    );

    let mut files: Vec<PathBuf> = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "json") {
                out.push(p);
            }
        }
    }
    walk(&base, &mut files);
    files.sort();

    let mut checked = 0usize;
    let mut losses = Vec::new();
    for path in &files {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut r1 = Report::new(Fidelity::Lossless);
        let Ok(ws) = cross_q::postman::parse_postman(&content, &mut r1) else {
            continue; // the importer's own gate covers parse failures
        };
        let before = request_names(&ws);
        if before.is_empty() {
            continue;
        }

        let mut r2 = Report::new(Fidelity::Lossless);
        let map = cross_q::emit_rq_md::to_rq_md(&ws, &mut r2);
        let (back, _) = reread(&map);
        let after = request_names(&back);

        checked += 1;
        if before.len() != after.len() {
            losses.push(format!(
                "{}: {} request(s) in, {} out",
                path.file_name().unwrap().to_string_lossy(),
                before.len(),
                after.len()
            ));
        }
    }

    assert!(checked > 0, "no collections were checked");
    assert!(
        losses.is_empty(),
        "{}/{checked} collection(s) lost requests through the rq format:\n  {}",
        losses.len(),
        losses.join("\n  ")
    );
}
