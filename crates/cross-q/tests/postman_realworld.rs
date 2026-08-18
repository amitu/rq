//! Real-world **canonical** Postman corpus — the fidelity oracle.
//!
//! Unlike the postman-collection-transformer `examples/` (a non-canonical PLURAL-key
//! dialect, good only for crash-safety), these are collections exported by real API
//! providers in the wild: canonical singular-key v2.1 (Adyen, MIT) plus v2.0 and v1
//! (Postman's own newman examples, Apache-2.0). Fetched to pinned commits — **fails loud
//! if absent** (run `tests/corpus/fetch-realworld-corpus.sh`; CI runs it before tests).
//! See `tests/corpus/README.md`.
//!
//! Two properties are asserted, both stronger than "it parsed":
//!
//! 1. **No hollow parse** — every request in the source survives into `MappedItems`. On
//!    canonical input a real collection MUST yield its requests; equal request counts
//!    (source vs re-emitted) is the anti-hollow-parse guard the transformer corpus could
//!    never give us (it read near-empty and still "passed").
//! 2. **Bounded round-trip loss** — Postman → IR → Postman drops only keys on a documented
//!    allowlist, each with a rationale. Any *new* dropped key fails the test: that's the
//!    "don't silently ignore a field" gate. As gaps close, entries leave the allowlist.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/realworld")
}

/// Every object key appearing anywhere in a JSON tree.
fn keys(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            for (k, val) in m {
                out.insert(k.clone());
                keys(val, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| keys(x, out)),
        _ => {}
    }
}

/// Count request items (objects carrying a `request`, or v1 flat `requests[]` entries) in a
/// source or re-emitted collection — the structural fidelity signal.
fn count_requests(v: &Value) -> usize {
    fn rec(v: &Value, n: &mut usize) {
        match v {
            Value::Object(m) => {
                if m.contains_key("request") {
                    *n += 1;
                }
                // v1 flat shape: top-level `requests: [...]`
                if let Some(Value::Array(reqs)) = m.get("requests") {
                    *n += reqs.len();
                }
                if let Some(Value::Array(items)) = m.get("item") {
                    for it in items {
                        rec(it, n);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|x| rec(x, n)),
            _ => {}
        }
    }
    let mut n = 0;
    rec(v, &mut n);
    n
}

/// Keys we *knowingly* do not reproduce on the round-trip, each with why it's benign or a
/// tracked enrichment gap. A dropped key NOT in here fails the test (silent-loss guard).
const ALLOWED_DROPPED: &[(&str, &str)] = &[
    // Postman writes the enabled-default explicitly (`"disabled": false`); we omit it on
    // emit. Semantically identical — absent ≡ enabled.
    (
        "disabled",
        "explicit enabled-default; omitting is equivalent",
    ),
    // `url` object split arrays. We preserve `url.raw` (semantically complete) and re-emit
    // `query`/`variable`; `host`/`path` are redundant with `raw`.
    ("host", "redundant with url.raw"),
    ("path", "redundant with url.raw"),
    ("query", "carried in url.raw; re-emitted when non-empty"),
    // Empty `response: []` arrays: we don't emit the key when there are no saved responses
    // (absent ≡ empty). Non-empty responses ARE re-emitted verbatim.
    (
        "response",
        "only empty response arrays; non-empty ones round-trip verbatim",
    ),
    // Rich description-as-object `{content, type, version}`: we keep the text, drop the
    // type/version wrapper. The one genuine (minor) enrichment gap — tracked for promotion.
    (
        "content",
        "rich-description wrapper; text preserved, type/version not yet",
    ),
    (
        "version",
        "rich-description wrapper; text preserved, type/version not yet",
    ),
    (
        "description",
        "a description dropped where source used the object form",
    ),
];

#[test]
fn realworld_corpus_parses_without_hollow_loss() {
    let base = corpus_dir();
    assert!(
        base.exists(),
        "real-world corpus not fetched — run \
         `crates/cross-q/tests/corpus/fetch-realworld-corpus.sh` (pinned; not vendored — \
         see tests/corpus/README.md)"
    );

    let allowed: BTreeSet<&str> = ALLOWED_DROPPED.iter().map(|(k, _)| *k).collect();
    let mut files = 0usize;
    let mut req_total = 0usize;
    let mut dropped_all: BTreeSet<String> = BTreeSet::new();
    let mut errors: Vec<String> = Vec::new();

    for entry in walk(&base) {
        let name = entry
            .strip_prefix(&base)
            .unwrap_or(&entry)
            .to_string_lossy()
            .to_string();
        let content = fs::read_to_string(&entry).expect("read corpus file");
        let original: Value = serde_json::from_str(&content).expect("corpus file is valid JSON");

        // 1. parses into a non-empty MappedItems
        let out = cross_q::parse_to_mapped_items("postman", &content, &name);
        if out["ok"] != Value::Bool(true) {
            errors.push(format!(
                "{name}: parse failed: {}",
                out["error"].as_str().unwrap_or("<no message>")
            ));
            continue;
        }
        files += 1;

        // 2. no hollow parse — request counts match through IR
        let emitted = cross_q::postman_roundtrip(&content).expect("round-trip");
        let (ro, re) = (count_requests(&original), count_requests(&emitted));
        req_total += ro;
        if ro != re {
            errors.push(format!(
                "{name}: request count changed through round-trip: source={ro} re-emitted={re}"
            ));
        }

        // 3. round-trip loss is bounded to the allowlist — but ONLY for v2.1 sources.
        // We emit v2.1, so a v1/v2.0 source's structural keys (`folders`, `order`,
        // `rawModeData`, `preRequestScript`, …) legitimately become their v2.1 equivalents;
        // a raw key-diff there is apples-to-oranges (correct translation, not loss). Same-
        // dialect (v2.1 → v2.1) is the only fair field-coverage comparison.
        let is_v21 = original
            .get("info")
            .and_then(|i| i.get("schema"))
            .and_then(|s| s.as_str())
            .is_some_and(|s| s.contains("v2.1.0"));
        if is_v21 {
            let (mut ok, mut ek) = (BTreeSet::new(), BTreeSet::new());
            keys(&original, &mut ok);
            keys(&emitted, &mut ek);
            for k in ok.difference(&ek) {
                dropped_all.insert(k.clone());
            }
        }
    }

    // Pinned corpus ⇒ pinned size; `> 0` cannot tell a full fetch from a truncated one.
    const PINNED_COLLECTIONS: usize = 20;
    assert_eq!(
        files, PINNED_COLLECTIONS,
        "corpus has {files} collections, expected {PINNED_COLLECTIONS} — a partial fetch, or \
         the pin moved. Re-run tests/corpus/fetch-realworld-corpus.sh on a clean dir."
    );
    assert!(
        errors.is_empty(),
        "real-world corpus errors:\n  {}",
        errors.join("\n  ")
    );

    let unexpected: Vec<&String> = dropped_all
        .iter()
        .filter(|k| !allowed.contains(k.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "NEW round-trip field loss (not on the documented allowlist) — either promote to \
         first-class IR or preserve in ext, don't silently drop:\n  {:?}",
        unexpected
    );

    // Informational: allowlist entries no longer observed → tighten the allowlist.
    let stale: Vec<&str> = allowed
        .iter()
        .filter(|k| !dropped_all.contains(**k))
        .copied()
        .collect();
    eprintln!(
        "real-world corpus: {files} collections, {req_total} requests, all round-trip with \
         request-count parity. Bounded loss keys observed: {:?}. Stale allowlist entries \
         (safe to remove): {:?}",
        dropped_all, stale
    );
}

/// Recursively collect `*.json` files under `dir`.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else if p.extension().and_then(|x| x.to_str()) == Some("json") {
                out.push(p);
            }
        }
    }
    out
}
