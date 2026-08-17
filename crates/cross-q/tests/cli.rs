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

#[test]
fn convert_curl_to_rq_writes_a_project_you_can_read() {
    let dir = tempfile::tempdir().unwrap();
    let status = cq()
        .args([
            "convert",
            "curl -X POST -H 'Accept: application/json' https://api.example.com/v1/users",
            "--to",
            "rq",
            "--output",
        ])
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "cq convert --to rq failed");

    assert!(dir.path().join("rq.toml").is_file());
    let doc = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "md"))
        .expect("no request document written");
    let text = std::fs::read_to_string(&doc).unwrap();
    assert!(text.contains("method: POST"), "{text}");
    assert!(
        text.contains("url: https://api.example.com/v1/users"),
        "{text}"
    );
    assert!(text.contains("Accept: application/json"), "{text}");
}

#[test]
fn an_rq_project_directory_is_detected_and_converts_back_out() {
    // Write a project with `cq`…
    let project = tempfile::tempdir().unwrap();
    assert!(cq()
        .args([
            "convert",
            "curl https://api.example.com/v1/users",
            "--to",
            "rq",
            "--output",
        ])
        .arg(project.path())
        .status()
        .unwrap()
        .success());

    // …then read that directory back with no --from, and emit Postman from it.
    let out = tempfile::tempdir().unwrap();
    let status = cq()
        .arg("convert")
        .arg(project.path())
        .args(["--to", "postman", "--output"])
        .arg(out.path())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "cq could not read back its own rq project"
    );

    let file = std::fs::read_dir(out.path())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(".postman_collection.json"))
        .expect("no collection written");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(
        v["item"][0]["request"]["url"]["raw"],
        serde_json::json!("https://api.example.com/v1/users")
    );
}

#[test]
fn the_requestly_local_fs_target_is_still_reachable_under_its_own_name() {
    let dir = tempfile::tempdir().unwrap();
    let status = cq()
        .args([
            "convert",
            "curl https://api.example.com/v1/users",
            "--to",
            "requestly",
            "--output",
        ])
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "cq convert --to requestly failed");
    // The LOCAL_FS tree splits a request across JSON files — not the Markdown form.
    assert!(dir.path().join("apis").is_dir());
    let has_metadata_json = walkdir(&dir.path().join("apis"))
        .iter()
        .any(|p| p.ends_with("__metadata.json"));
    assert!(has_metadata_json, "expected the LOCAL_FS split-JSON tree");
}

fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            out.extend(walkdir(&p));
        } else {
            out.push(p);
        }
    }
    out
}
