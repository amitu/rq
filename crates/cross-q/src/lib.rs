//! # cross-q — convert API-client collections through one idealised model.
//!
//! The library side of the `cq` binary. Today it wires the first end-to-end path:
//! cURL → [`cq_model`] → Requestly `LOCAL_FS`, producing a [`cq_report::Report`] of
//! everything that wasn't a clean 1:1. More importers and exporters slot into the same
//! parse → map → emit shape.

pub mod curl;
pub mod emit_rq;
pub mod postman;

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
    };
    // An unnamed root collection => the request lands directly under apis/.
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
pub fn convert_curl_to_rq(input: &str, out_dir: &Path) -> anyhow::Result<Report> {
    let mut report = Report::new(Fidelity::Lossless);
    let ws = curl_to_workspace(input, &mut report).map_err(|e| anyhow::anyhow!("{e}"))?;
    emit_rq::emit_rq(&ws, out_dir, &mut report)?;
    Ok(report)
}

/// Convert a Postman collection (v2.0/v2.1) into a Requestly `LOCAL_FS` project.
pub fn convert_postman_to_rq(input: &str, out_dir: &Path) -> anyhow::Result<Report> {
    let mut report = Report::new(Fidelity::Lossless);
    let ws = postman::parse_postman(input, &mut report).map_err(|e| anyhow::anyhow!("{e}"))?;
    emit_rq::emit_rq(&ws, out_dir, &mut report)?;
    Ok(report)
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
    fn end_to_end_curl_to_rq() {
        let dir = tempfile::tempdir().unwrap();
        let report = convert_curl_to_rq(
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
        let err = convert_curl_to_rq("curl -X GET", dir.path());
        assert!(err.is_err());
    }
}
