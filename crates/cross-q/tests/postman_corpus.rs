//! Corpus test — run cross-q's Postman parser over Postman's own transformer example
//! collections (v1.0.0 / v2.0.0 / v2.1.0), fetched to a pinned SHA (see `tests/corpus/`).
//!
//! **Fails loud if the corpus hasn't been fetched** (run
//! `tests/corpus/fetch-postman-corpus.sh` once; CI runs it before tests) — a corpus test
//! that silently skipped would be a false green. This validates the parsers against
//! Postman's *own* definition of the format across all three versions — not just our own
//! fixtures.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/postman-transformer")
}

#[test]
fn parses_postman_transformer_corpus() {
    let base = corpus_dir();
    // Fail loud, never silently green: a corpus test that quietly skips tested nothing.
    // The corpus is fetched (pinned SHA), not vendored — run the fetch script once
    // (CI runs it before tests). See tests/corpus/README.md.
    assert!(
        base.exists(),
        "postman corpus not fetched — run `crates/cross-q/tests/corpus/fetch-postman-corpus.sh` \
         (pinned snapshot; not vendored — see tests/corpus/README.md)"
    );

    // Intentionally-malformed fixtures that are *allowed* to fail parsing (they exist to
    // exercise loud failure). Everything else must parse into a MappedItems object.
    let allowed_failures = ["malformed.json"];

    let mut total = 0usize;
    let mut ok = 0usize;
    let mut unexpected: Vec<String> = Vec::new();

    for ver in ["v1.0.0", "v2.0.0", "v2.1.0"] {
        let dir = base.join(ver);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            total += 1;

            let content = fs::read_to_string(&path).expect("read corpus fixture");
            // parse_to_mapped_items never panics — it returns { ok, mapped, report } or
            // { ok:false, error }. So this call itself is the crash-safety check.
            let out = cross_q::parse_to_mapped_items("postman", &content, &name);

            if out["ok"] == Value::Bool(true) {
                ok += 1;
                assert!(
                    out["mapped"].is_object(),
                    "{ver}/{name}: parsed ok but `mapped` is not an object"
                );
            } else if !allowed_failures.iter().any(|a| name == *a) {
                unexpected.push(format!(
                    "{ver}/{name}: {}",
                    out["error"].as_str().unwrap_or("<no error message>")
                ));
            }
        }
    }

    assert!(
        total > 0,
        "corpus dir present but contained no .json fixtures"
    );
    assert!(
        unexpected.is_empty(),
        "unexpected corpus parse failures ({ok}/{total} parsed ok):\n  {}",
        unexpected.join("\n  ")
    );
    eprintln!("postman corpus: {ok}/{total} parsed ok (allowed failures: {allowed_failures:?})");
}
