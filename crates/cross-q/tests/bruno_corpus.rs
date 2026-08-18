//! Bruno directory importer, exercised against a real-world canonical collection
//! (usebruno's own `bruno-tests`, MIT, pinned — see `tests/corpus/`).
//!
//! **Fails loud if the corpus hasn't been fetched** (run
//! `tests/corpus/fetch-bruno-corpus.sh`; CI runs it before tests) — a corpus test that
//! silently skipped would be a false green.
//!
//! Asserts the two properties that matter for a directory importer:
//! 1. **No hollow parse** — every `.bru` *request* file in the tree becomes a request in the
//!    workspace (equal counts). A folder-walk that quietly lost requests would pass a mere
//!    "it didn't panic" check; this won't.
//! 2. **The tree is real** — folders come through as nested collections, and
//!    `environments/*.bru` come through as environments with variables.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cq_model::{Item, Workspace};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/bruno-testbench")
}

/// Read every file under `dir` into a virtual-FS map keyed by path relative to `dir` — the
/// exact shape the host passes across the WASM boundary.
fn read_tree(dir: &Path, base: &Path, map: &mut BTreeMap<String, String>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            read_tree(&p, base, map);
        } else if let Ok(content) = fs::read_to_string(&p) {
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
            map.insert(rel, content);
        }
    }
}

/// Count request items (nodes carrying an HTTP/GraphQL request) anywhere in a workspace.
fn count_requests(ws: &Workspace) -> usize {
    fn walk(items: &[Item], n: &mut usize) {
        for it in items {
            match it {
                Item::Request(_) => *n += 1,
                Item::Collection(c) => walk(&c.items, n),
            }
        }
    }
    let mut n = 0;
    for c in &ws.collections {
        walk(&c.items, &mut n);
    }
    n
}

#[test]
fn bruno_directory_import_has_no_hollow_loss() {
    let base = corpus_dir();
    assert!(
        base.exists(),
        "bruno corpus not fetched — run \
         `crates/cross-q/tests/corpus/fetch-bruno-corpus.sh` (pinned; not vendored — see \
         tests/corpus/README.md)"
    );

    let mut files = BTreeMap::new();
    read_tree(&base, &base, &mut files);

    // The source of truth for "no hollow parse": count request `.bru` files in the tree
    // (everything except folder.bru / collection.bru / bruno.json / non-.bru).
    let want_requests = files
        .keys()
        .filter(|p| {
            p.ends_with(".bru")
                && !p.ends_with("/folder.bru")
                && *p != "collection.bru"
                && !p.starts_with("environments/")
        })
        .count();
    // The corpus is pinned, so its size is a fact rather than a range. `> 0` was not enough:
    // a partial fetch (ours was throttled to 16 of 239 files once) leaves a smaller tree that
    // passes every assertion below — self-consistently, because they all compare the parse
    // against whatever happens to be on disk. Bump these two with the pin, never to match a
    // tree you didn't fetch on purpose.
    const PINNED_REQUESTS: usize = 223;
    const PINNED_ENVS: usize = 2;
    assert_eq!(
        want_requests, PINNED_REQUESTS,
        "corpus has {want_requests} request .bru files, expected {PINNED_REQUESTS} — a partial \
         fetch, or the pin moved. Delete tests/corpus/bruno-testbench and re-run the fetch \
         script; if the pin really changed, update PINNED_REQUESTS."
    );

    let want_envs = files
        .keys()
        .filter(|p| p.starts_with("environments/") && p.ends_with(".bru"))
        .count();

    assert_eq!(
        want_envs, PINNED_ENVS,
        "corpus has {want_envs} environment .bru files, expected {PINNED_ENVS} — see above"
    );

    let mut report = cq_report::Report::new(cq_report::Fidelity::Lossless);
    let ws = cross_q::bruno::parse_bruno_collection(&files, &mut report).expect("parse bruno dir");

    // 1. no hollow parse — every request file survived into the tree
    let got = count_requests(&ws);
    assert_eq!(
        got, want_requests,
        "request count mismatch: {want_requests} request .bru files but {got} in the workspace"
    );

    // 2. the tree is real — root has nested folders, environments came through
    let root = &ws.collections[0];
    let folders = root
        .items
        .iter()
        .filter(|i| matches!(i, Item::Collection(_)))
        .count();
    assert!(
        folders > 0,
        "expected nested folders in the collection tree"
    );
    assert_eq!(
        ws.environments.len(),
        want_envs,
        "environment count mismatch"
    );
    for env in &ws.environments {
        assert!(
            !env.variables.is_empty(),
            "environment `{}` parsed with no variables",
            env.meta.name
        );
    }

    eprintln!(
        "bruno corpus: {} requests across {} top-level folders, {} environments — no hollow loss",
        got,
        folders,
        ws.environments.len()
    );
}
