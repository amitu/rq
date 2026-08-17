//! # cross-q — convert API-client collections through one idealised model.
//!
//! The library side of the `cq` binary. Today it wires the first end-to-end path:
//! cURL → [`cq_model`] → Requestly `LOCAL_FS`, producing a [`cq_report::Report`] of
//! everything that wasn't a clean 1:1. More importers and exporters slot into the same
//! parse → map → emit shape.

pub mod bruno;
pub mod curl;
pub mod emit_bruno;
pub mod emit_postman;
pub mod emit_rq;
pub mod emit_rq_md;
pub mod mappeditems;
pub mod postman;
pub mod rq_md;
pub mod rq_shape;

pub use mappeditems::to_mapped_items;

use std::path::Path;

use cq_model::{
    Collection, Item, ModelHeader, Protocol, RecordMeta, Request, SourceFormat, Workspace,
};
use cq_report::{Fidelity, Report};

/// A lowercase, hyphenated slug for deterministic ids (no randomness → byte-stable output).
fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "request".to_string()
    } else {
        trimmed
    }
}

/// Build a single-request [`Workspace`] from a curl command, recording parse diagnostics.
pub fn curl_to_workspace(input: &str, report: &mut Report) -> Result<Workspace, String> {
    let parsed = curl::parse_curl(input, report)?;
    let name = parsed.name.clone();
    let request = Request {
        meta: RecordMeta::new(format!("cq-{}", slug(&name)), name, SourceFormat::Curl),
        protocol: Protocol::Http(parsed.request),
        auth: parsed.auth,
        scripts: Default::default(),
        examples: Vec::new(),
        depends_on: Vec::new(),
        behavior: Default::default(),
    };
    // An unnamed root collection => the request lands at the top of the project.
    let root = Collection {
        meta: RecordMeta::new("cq-root", "", SourceFormat::Curl),
        items: vec![Item::Request(Box::new(request))],
        ..Collection::default()
    };
    Ok(Workspace {
        meta: RecordMeta::new("cq-workspace", "", SourceFormat::Curl),
        cross_q: ModelHeader::for_source(SourceFormat::Curl),
        collections: vec![root],
        environments: Vec::new(),
        packages: Vec::new(),
    })
}

/// Convert a curl command into a Requestly `LOCAL_FS` project at `out_dir`.
pub fn convert_curl_to_requestly(input: &str, out_dir: &Path) -> anyhow::Result<Report> {
    let mut report = Report::new(Fidelity::Lossless);
    let ws = curl_to_workspace(input, &mut report).map_err(|e| anyhow::anyhow!("{e}"))?;
    emit_rq::emit_rq(&ws, out_dir, &mut report)?;
    Ok(report)
}

/// Convert a Postman collection (v2.0/v2.1) into a Requestly `LOCAL_FS` project.
pub fn convert_postman_to_requestly(input: &str, out_dir: &Path) -> anyhow::Result<Report> {
    let mut report = Report::new(Fidelity::Lossless);
    let ws = postman::parse_postman(input, &mut report).map_err(|e| anyhow::anyhow!("{e}"))?;
    emit_rq::emit_rq(&ws, out_dir, &mut report)?;
    Ok(report)
}

/// Parse an input of a given source format into the Idealised Model. Shared by the `rq`
/// and `mapped` conversion targets.
pub fn build_workspace(
    source: &str,
    input: &str,
    report: &mut Report,
) -> anyhow::Result<Workspace> {
    match source {
        "curl" => curl_to_workspace(input, report).map_err(|e| anyhow::anyhow!("{e}")),
        "postman" => postman::parse_postman(input, report).map_err(|e| anyhow::anyhow!("{e}")),
        "bruno" => bruno::parse_bruno(input, report).map_err(|e| anyhow::anyhow!("{e}")),
        "rq" => rq_md::parse_rq_md(input, report).map_err(|e| anyhow::anyhow!("{e}")),
        other => {
            anyhow::bail!(
                "not_implemented: source format {other:?} (supported: curl, postman, bruno, rq)"
            )
        }
    }
}

/// Parse `content` of the given `format` and produce the Requestly `MappedItems` bundle
/// plus the conversion [`Report`], as a single serializable value. This is the function
/// the WASM boundary (and any host consuming the import engine) calls: input strings in,
/// one JSON value out.
///
/// The returned shape is `{ "ok": true, "mapped": <MappedItems>, "report": <Report> }`
/// on success, or `{ "ok": false, "error": <message> }` when the format is unknown or the
/// input can't be parsed at all (a hard failure — distinct from per-item diagnostics,
/// which ride inside `report`).
pub fn parse_to_mapped_items(format: &str, content: &str, _file_name: &str) -> serde_json::Value {
    let mut report = Report::new(Fidelity::Lossless);
    match build_workspace(format, content, &mut report) {
        Ok(ws) => {
            let mapped = mappeditems::to_mapped_items(&ws, &mut report);
            serde_json::json!({
                "ok": true,
                "mapped": mapped,
                "report": report,
            })
        }
        Err(e) => serde_json::json!({
            "ok": false,
            "error": e.to_string(),
        }),
    }
}

/// Parse Postman `content` and re-emit it as a Postman v2.1 collection — the reverse
/// round-trip used to detect any field we silently drop (`Postman → IR → Postman`). The
/// conversion report is discarded here; callers diff the returned JSON against the original.
pub fn postman_roundtrip(content: &str) -> Result<serde_json::Value, String> {
    let mut report = Report::new(Fidelity::Lossless);
    let ws = postman::parse_postman(content, &mut report)?;
    Ok(emit_postman::to_postman(&ws))
}

/// Parse a Bruno single `.bru` request and re-emit it as `.bru` text — the request-level
/// round-trip used to prove the exporter is lossless (parse → IR → `.bru` → IR recovers the
/// same IR). Returns the re-emitted `.bru`.
pub fn bruno_request_roundtrip(content: &str) -> Result<String, String> {
    let mut report = Report::new(Fidelity::Lossless);
    let req = bruno::parse_bru_request(content, &mut report)?;
    let ws = single_request_workspace(req);
    Ok(emit_bruno::emit_request(
        match &ws.collections[0].items[0] {
            Item::Request(r) => r,
            _ => unreachable!(),
        },
    ))
}

fn single_request_workspace(request: Request) -> Workspace {
    let root = Collection {
        meta: RecordMeta::new("bru-root", "", SourceFormat::Bruno),
        items: vec![Item::Request(Box::new(request))],
        ..Collection::default()
    };
    Workspace {
        meta: RecordMeta::new("bru-workspace", "", SourceFormat::Bruno),
        cross_q: ModelHeader::for_source(SourceFormat::Bruno),
        collections: vec![root],
        environments: Vec::new(),
        packages: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_clean() {
        assert_eq!(slug("List Issues!"), "list-issues");
        assert_eq!(slug("///"), "request");
        assert_eq!(slug("users"), "users");
    }

    #[test]
    fn parse_to_mapped_items_ok_shape() {
        let out = parse_to_mapped_items(
            "curl",
            "curl -H 'Accept: application/json' https://api.example.com/v1/users",
            "cmd.txt",
        );
        assert_eq!(out["ok"], serde_json::json!(true));
        assert_eq!(
            out["mapped"]["requests"][0]["data"]["type"],
            serde_json::json!("http")
        );
        // report is embedded and serializable
        assert!(out["report"]["fidelity"].is_string());
    }

    #[test]
    fn parse_to_mapped_items_unknown_format_is_soft_error() {
        let out = parse_to_mapped_items("insomnia", "{}", "x.json");
        assert_eq!(out["ok"], serde_json::json!(false));
        assert!(out["error"].as_str().unwrap().contains("not_implemented"));
    }

    #[test]
    fn end_to_end_curl_to_rq() {
        let dir = tempfile::tempdir().unwrap();
        let report = convert_curl_to_requestly(
            "curl -H 'Accept: application/json' https://api.example.com/v1/users",
            dir.path(),
        )
        .unwrap();

        assert!(dir.path().join("__requestly.json").exists());
        assert!(dir.path().join("apis/users/__metadata.json").exists());
        assert!(!report.has_errors());
        // curl → rq is declared Lossless (everything maps); no coercions keep it there.
        assert_eq!(report.effective_fidelity(), Fidelity::Lossless);
    }

    #[test]
    fn missing_url_surfaces_as_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = convert_curl_to_requestly("curl -X GET", dir.path());
        assert!(err.is_err());
    }
}
