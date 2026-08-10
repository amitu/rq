//! End-to-end tests for the `cq` binary's export targets — proving the exporters are wired
//! as real CLI targets, not just library functions.

use std::process::Command;

fn cq() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cq"))
}

#[test]
fn convert_curl_to_postman_writes_a_valid_collection() {
    let dir = tempfile::tempdir().unwrap();
    let status = cq()
        .args([
            "convert",
            "curl -X POST -H 'Accept: application/json' https://api.example.com/v1/users",
            "--to",
            "postman",
            "--output",
        ])
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "cq convert --to postman failed");

    // A single .postman_collection.json is written; it parses and carries the request.
    let file = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(".postman_collection.json"))
        .expect("no .postman_collection.json written");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert!(v["info"]["schema"].as_str().unwrap().contains("v2.1.0"));
    assert_eq!(v["item"][0]["request"]["method"], serde_json::json!("POST"));
}

#[test]
fn convert_curl_to_bruno_writes_a_collection_dir() {
    let dir = tempfile::tempdir().unwrap();
    let status = cq()
        .args([
            "convert",
            "curl https://api.example.com/ping",
            "--to",
            "bruno",
            "--output",
        ])
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "cq convert --to bruno failed");
    assert!(
        dir.path().join("bruno.json").exists(),
        "no bruno.json written"
    );
    // exactly one request .bru at the root (curl → single request)
    let bru = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|e| e.to_str()) == Some("bru"))
        .expect("no .bru written");
    let text = std::fs::read_to_string(&bru).unwrap();
    assert!(
        text.contains("get {"),
        "emitted .bru missing method block:\n{text}"
    );
    assert!(text.contains("url: https://api.example.com/ping"));
}

#[test]
fn formats_lists_the_new_export_targets() {
    let out = cq().arg("formats").output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("EXPORT   postman"),
        "formats missing postman export"
    );
    assert!(
        text.contains("EXPORT   bruno"),
        "formats missing bruno export"
    );
}
