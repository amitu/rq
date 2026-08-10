//! Postman exporter fidelity by **semantic idempotence** — the same gate the Bruno exporter
//! uses. For the canonical v2.1 corpus (Adyen), `Postman → IR → Postman → IR` must recover
//! the same IR. This is stronger and less brittle than the text key-diff in
//! `postman_roundtrip.rs`: it proves the exporter is a fixed point of the model, so promoting
//! `to_postman` to a real `cq` export target is safe.
//!
//! **Fails loud if the corpus isn't fetched** (run `tests/corpus/fetch-realworld-corpus.sh`).

use std::fs;
use std::path::{Path, PathBuf};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/realworld")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("json") {
                out.push(p);
            }
        }
    }
}

#[test]
fn postman_v21_export_is_ir_idempotent() {
    let base = corpus_dir();
    assert!(
        base.exists(),
        "real-world corpus not fetched — run `crates/cross-q/tests/corpus/fetch-realworld-corpus.sh`"
    );

    let mut files = Vec::new();
    walk(&base, &mut files);

    let mut checked = 0usize;
    let mut mismatches = Vec::new();
    for path in &files {
        let content = fs::read_to_string(path).unwrap();
        // Only same-dialect v2.1 sources: we emit v2.1, so a v1/v2.0 source's IR would
        // re-emit as v2.1 and isn't a fair fixed-point comparison (covered elsewhere).
        let is_v21 = serde_json::from_str::<serde_json::Value>(&content)
            .ok()
            .and_then(|v| {
                v.get("info")
                    .and_then(|i| i.get("schema"))
                    .and_then(|s| s.as_str())
                    .map(|s| s.contains("v2.1.0"))
            })
            .unwrap_or(false);
        if !is_v21 {
            continue;
        }

        let mut r1 = cq_report::Report::new(cq_report::Fidelity::Lossless);
        let Ok(ws1) = cross_q::postman::parse_postman(&content, &mut r1) else {
            continue;
        };
        let reemitted = cross_q::emit_postman::to_postman(&ws1);
        let text = serde_json::to_string(&reemitted).unwrap();
        let mut r2 = cq_report::Report::new(cq_report::Fidelity::Lossless);
        let ws2 =
            cross_q::postman::parse_postman(&text, &mut r2).expect("re-emitted Postman must parse");
        checked += 1;
        if ws1 != ws2 {
            mismatches.push(path.file_name().unwrap().to_string_lossy().to_string());
        }
    }

    assert!(checked > 0, "no v2.1 corpus files exercised");
    assert!(
        mismatches.is_empty(),
        "{}/{} collection(s) did NOT survive Postman → IR → Postman → IR unchanged:\n  {}",
        mismatches.len(),
        checked,
        mismatches.join("\n  ")
    );
    eprintln!("postman v2.1 export: {checked} collections, all IR-idempotent");
}
