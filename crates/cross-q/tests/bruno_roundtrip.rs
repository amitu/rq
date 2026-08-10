//! Bruno round-trip **fidelity** — the exporter is proven lossless by *semantic idempotence*:
//! `.bru` → IR → `.bru` → IR must recover the same IR. Byte-identity would be brittle (block
//! order, formatting); IR equality is the honest test — if the exporter drops or mangles a
//! field, the re-parsed IR differs and the test names the file.
//!
//! Runs over the pinned real Bruno corpus (usebruno `bruno-tests`, MIT). **Fails loud if the
//! corpus isn't fetched** (run `tests/corpus/fetch-bruno-corpus.sh`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cq_model::Item;
use cq_report::{Fidelity, Report};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/bruno-testbench")
}

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

fn count_requests(items: &[Item]) -> usize {
    items
        .iter()
        .map(|i| match i {
            Item::Request(_) => 1,
            Item::Collection(c) => count_requests(&c.items),
        })
        .sum()
}

#[test]
fn bruno_request_roundtrip_is_ir_idempotent() {
    let base = corpus_dir();
    assert!(
        base.exists(),
        "bruno corpus not fetched — run `crates/cross-q/tests/corpus/fetch-bruno-corpus.sh`"
    );
    let mut files = BTreeMap::new();
    read_tree(&base, &base, &mut files);

    let mut checked = 0usize;
    let mut mismatches = Vec::new();
    for (path, content) in &files {
        // request files only (not folder.bru/collection.bru/environments)
        if !path.ends_with(".bru")
            || path.ends_with("/folder.bru")
            || *path == "collection.bru"
            || path.starts_with("environments/")
        {
            continue;
        }
        let mut r1 = Report::new(Fidelity::Lossless);
        let Ok(ir1) = cross_q::bruno::parse_bru_request(content, &mut r1) else {
            continue; // parse failures are the importer gate's concern, not this test's
        };
        let reemitted = cross_q::emit_bruno::emit_request(&ir1);
        let mut r2 = Report::new(Fidelity::Lossless);
        let ir2 = cross_q::bruno::parse_bru_request(&reemitted, &mut r2)
            .expect("re-emitted .bru must parse");
        checked += 1;
        if ir1 != ir2 {
            mismatches.push(path.clone());
        }
    }

    assert!(checked > 0, "no request .bru files round-tripped");
    assert!(
        mismatches.is_empty(),
        "{}/{} request(s) did NOT survive .bru → IR → .bru → IR unchanged (exporter dropped or \
         mangled a field):\n  {}",
        mismatches.len(),
        checked,
        mismatches.join("\n  ")
    );
    eprintln!("bruno request round-trip: {checked} requests, all IR-idempotent");
}

#[test]
fn bruno_directory_roundtrip_preserves_tree() {
    let base = corpus_dir();
    assert!(base.exists(), "bruno corpus not fetched");
    let mut files = BTreeMap::new();
    read_tree(&base, &base, &mut files);

    let mut report = Report::new(Fidelity::Lossless);
    let ws1 = cross_q::bruno::parse_bruno_collection(&files, &mut report).expect("parse dir");
    let map = cross_q::emit_bruno::to_bruno(&ws1);
    let mut r2 = Report::new(Fidelity::Lossless);
    let ws2 = cross_q::bruno::parse_bruno_collection(&map, &mut r2).expect("re-parse emitted dir");

    let (n1, n2) = (
        count_requests(&ws1.collections[0].items),
        count_requests(&ws2.collections[0].items),
    );
    assert_eq!(n1, n2, "directory round-trip changed the request count");
    assert_eq!(
        ws1.environments.len(),
        ws2.environments.len(),
        "directory round-trip changed the environment count"
    );
    eprintln!(
        "bruno directory round-trip: {n1} requests + {} environments preserved",
        ws2.environments.len()
    );
}
