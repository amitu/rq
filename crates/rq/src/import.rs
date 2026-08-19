//! Bringing collections in: `rq curl --save-as` and `rq import`.
//!
//! The CLI owns no conversion of its own. cross-q parses whatever you have into the
//! Idealised Model and emits an `rq` project as a virtual filesystem; this writes that
//! map to disk. One converter, one format definition, no second implementation to drift.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};

use rq_doc::layout;

/// Write an emitted project map under `root`, returning the requests it created (paths
/// below `apis/`, in tree order).
pub fn write_project(map: &BTreeMap<String, String>, root: &Path) -> Result<Vec<String>> {
    let mut written = Vec::new();
    for (rel, content) in map {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        // The marker is a project's identity, not a document: never overwrite one that a
        // person may have edited (include/exclude globs live there).
        if rel == layout::MARKER && path.exists() {
            continue;
        }
        std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
        if let Some(request) = request_rel(rel) {
            written.push(request);
        }
    }
    Ok(written)
}

/// `github/issues.md` → `github/issues`. `index.md` belongs to its collection, not to a
/// request of its own.
fn request_rel(path: &str) -> Option<String> {
    let file = path.rsplit('/').next()?;
    if !layout::is_request_file(file) {
        return None;
    }
    layout::request_name(path).map(str::to_string)
}

/// Guess the source format for `rq import`, the way a person would: by looking.
pub fn detect_format(path: &Path, content: &str) -> Option<&'static str> {
    if path.is_dir() {
        return None; // directories are resolved by the caller, which knows the layout
    }
    if path.extension().is_some_and(|e| e == "bru") {
        return Some("bruno");
    }
    let head = content.trim_start();
    if head.starts_with("curl ") {
        return Some("curl");
    }
    if path.extension().is_some_and(|e| e == "md") && head.starts_with("---") {
        return Some("rq");
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(head) {
        if json.get("info").is_some() || json.get("_postman_id").is_some() {
            return Some("postman");
        }
    }
    if head.contains("\nget {") || head.starts_with("meta {") || head.contains("\npost {") {
        return Some("bruno");
    }
    None
}

/// Read a directory into the virtual-FS map the importers take. Mirrors `cq`'s own reader:
/// hidden entries are never part of a collection.
pub fn read_dir_map(dir: &Path) -> Result<BTreeMap<String, String>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, String>) -> Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
            {
                continue;
            }
            if path.is_dir() {
                walk(&path, base, out)?;
            } else if let Ok(content) = std::fs::read_to_string(&path) {
                out.insert(
                    path.strip_prefix(base)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    content,
                );
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// What a directory is, judged by what is in it. Directories are trees, so they are read
/// into the virtual-FS map first and identified from the names inside.
pub fn detect_dir(map: &BTreeMap<String, String>) -> Option<&'static str> {
    // Order matters, and "any .md file" is the weakest signal of the three: a Bruno
    // collection with a `readme.md` in it — usebruno's own test collection has one — was
    // being read as an rq project, which finds no requests at all and says so in a way that
    // sounds like the collection is empty. A marker or a `.bru` is a statement about what
    // the directory IS; loose markdown is just a file that happens to be there.
    if map.contains_key(layout::MARKER) {
        Some("rq")
    } else if map.keys().any(|k| k.ends_with(".bru") || k == "bruno.json") {
        Some("bruno")
    } else if map.keys().any(|k| k.ends_with(".md")) {
        Some("rq")
    } else {
        None
    }
}

/// Everything cross-q needs to read `path`: the content in the shape the importer takes, and
/// the format it appears to be.
pub fn source_at(path: &Path) -> Result<(String, Option<&'static str>)> {
    if path.is_dir() {
        let map = read_dir_map(path)?;
        let detected = detect_dir(&map);
        Ok((serde_json::to_string(&map)?, detected))
    } else {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let detected = detect_format(path, &text);
        Ok((text, detected))
    }
}

/// Read anything cross-q understands and produce **the rq project it describes** — the same
/// map `rq import` writes to disk, without writing it.
///
/// This is what lets `rq` open a Postman file directly. The conversion is not a lesser
/// version of importing: it is the identical call, so a collection read in place behaves
/// exactly like the same collection imported, and there is still one converter.
pub fn to_project_map(
    path: &Path,
    forced: Option<&str>,
    report: &mut cq_report::Report,
) -> Result<(BTreeMap<String, String>, String)> {
    let (content, detected) = source_at(path)?;
    let format = match forced {
        Some(f) => f.to_string(),
        None => detected
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "could not tell what {} is — pass --from postman|bruno|curl|rq",
                    path.display()
                )
            })?
            .to_string(),
    };
    let ws = cross_q::build_workspace(&format, &content, report)?;
    Ok((cross_q::emit_rq_md::to_rq_md(&ws, report), format))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{self, Project};

    #[test]
    fn writes_an_emitted_project_and_reports_its_requests() {
        let dir = tempfile::tempdir().unwrap();
        project::init(dir.path()).unwrap();

        let mut report = cq_report::Report::new(cq_report::Fidelity::Lossless);
        let ws = cross_q::curl_to_workspace(
            "curl -X POST https://api.test/login -d '{\"user\":\"amitu\"}'",
            &mut report,
        )
        .unwrap();
        let map = cross_q::emit_rq_md::to_rq_md(&ws, &mut report);

        let written = write_project(&map, dir.path()).unwrap();
        assert_eq!(written.len(), 1);
        let project = Project::open(dir.path().to_path_buf()).unwrap();
        let idx = project.resolve(&written[0]).unwrap();
        let (doc, _) = project.load(idx).unwrap();
        assert_eq!(doc.front.method.as_deref(), Some("POST"));
        assert_eq!(doc.section("body"), Some("{\"user\":\"amitu\"}"));
    }

    #[test]
    fn a_bruno_tree_with_a_readme_is_still_bruno() {
        let bruno = BTreeMap::from([
            ("bruno.json".to_string(), "{}".to_string()),
            ("readme.md".to_string(), "# notes".to_string()),
            ("get.bru".to_string(), "meta { name: get }".to_string()),
        ]);
        assert_eq!(detect_dir(&bruno), Some("bruno"));

        // A real rq project still wins on its marker, README or not.
        let project = BTreeMap::from([
            (layout::MARKER.to_string(), String::new()),
            ("readme.md".to_string(), "# notes".to_string()),
        ]);
        assert_eq!(detect_dir(&project), Some("rq"));

        // Markdown alone is still an rq tree — that is how a project without its marker
        // (a subdirectory, say) is read.
        let loose = BTreeMap::from([("issues.md".to_string(), "---\nurl: x\n---\n".to_string())]);
        assert_eq!(detect_dir(&loose), Some("rq"));
    }

    #[test]
    fn an_existing_marker_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        project::init(dir.path()).unwrap();
        let marker = dir.path().join(project::MARKER);
        std::fs::write(&marker, "{\"version\":\"1\",\"include\":[\"apis/**\"]}\n").unwrap();

        let map = BTreeMap::from([(project::MARKER.to_string(), "{}\n".to_string())]);
        write_project(&map, dir.path()).unwrap();
        assert!(std::fs::read_to_string(&marker)
            .unwrap()
            .contains("apis/**"));
    }

    #[test]
    fn detects_the_obvious_formats() {
        assert_eq!(
            detect_format(Path::new("x.json"), "{\"info\": {}, \"item\": []}"),
            Some("postman")
        );
        assert_eq!(
            detect_format(Path::new("x.txt"), "curl https://x.test"),
            Some("curl")
        );
        assert_eq!(detect_format(Path::new("x.bru"), "get {\n}"), Some("bruno"));
        assert_eq!(
            detect_format(Path::new("__metadata.md"), "---\nurl: https://x\n---\n"),
            Some("rq")
        );
        assert_eq!(detect_format(Path::new("x.txt"), "nonsense"), None);
    }
}
