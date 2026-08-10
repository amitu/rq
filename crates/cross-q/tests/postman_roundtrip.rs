//! Round-trip **completeness** analysis: Postman v2.1 → IR → Postman v2.1, then report
//! every object key present in the original but NOT in the re-emitted collection — i.e.
//! fields we currently drop. This is the triage list that drives "don't silently ignore":
//! each dropped field is either promoted to a first-class IR field (if meaningful across
//! the category) or preserved verbatim in `ext[format]` (if genuinely idiosyncratic).
//!
//! We emit v2.1, so the fair round-trip is the v2.1 corpus. Fails loud if the corpus isn't
//! fetched. For now this REPORTS the loss set (the todo list); it tightens to an
//! assert-empty gate as we drive the losses to zero.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::Value;

/// Collect every object key appearing anywhere in a JSON tree.
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

#[test]
fn postman_v21_roundtrip_field_coverage() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/postman-transformer/v2.1.0");
    assert!(
        dir.exists(),
        "corpus not fetched — run crates/cross-q/tests/corpus/fetch-postman-corpus.sh"
    );

    let mut lost: BTreeMap<String, usize> = BTreeMap::new();
    let mut files = 0usize;

    for entry in fs::read_dir(&dir).unwrap().flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = fs::read_to_string(&p).unwrap();
        let Ok(original) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        let Ok(emitted) = cross_q::postman_roundtrip(&content) else {
            continue;
        };
        files += 1;

        let mut ok = BTreeSet::new();
        let mut ek = BTreeSet::new();
        keys(&original, &mut ok);
        keys(&emitted, &mut ek);
        for k in ok.difference(&ek) {
            *lost.entry(k.clone()).or_default() += 1;
        }
    }

    eprintln!("\n=== Postman v2.1 round-trip — field names in original but not re-emitted ({files} files) ===");
    for (k, c) in &lost {
        eprintln!("  {k}  (×{c})");
    }
    eprintln!(
        "=== {} distinct dropped field names (triage: promote-to-IR vs ext) ===\n",
        lost.len()
    );

    assert!(files > 0, "no v2.1 corpus files round-tripped");
}
