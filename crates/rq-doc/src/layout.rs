//! The shape of an `rq` project on disk.
//!
//! ```text
//! my-apis/
//! ├── __requestly.json          project marker (discovery walks up to it, git-style)
//! ├── apis/
//! │   ├── issues/__metadata.md  a request
//! │   └── github/               a collection — just a directory
//! │       ├── __collection.md   its shared headers / auth / vars (optional)
//! │       └── login/__metadata.md
//! ├── environments/
//! │   ├── __global.md
//! │   └── staging.md
//! └── .requestly/state.json     the active environment (machine-local, gitignored)
//! ```
//!
//! These names live here, not in the CLI, because the converter writes this layout too:
//! one definition of where a request goes, read by everything that reads or writes it.

/// The project marker. Discovery walks up from the cwd looking for this file.
pub const MARKER: &str = "__requestly.json";
/// The file that makes a directory a request.
pub const REQUEST_FILE: &str = "__metadata.md";
/// A collection's own settings — optional; a directory is a collection without it.
pub const COLLECTION_FILE: &str = "__collection.md";
pub const APIS_DIR: &str = "apis";
pub const ENVS_DIR: &str = "environments";
/// Machine-local state (the active environment). Not part of the collection.
pub const STATE_DIR: &str = ".requestly";
/// The environment that applies under whichever other environment is active.
pub const GLOBAL_ENV: &str = "__global";

/// The contents of a fresh `__requestly.json`.
pub fn marker() -> String {
    "{\n  \"version\": \"1\",\n  \"include\": [],\n  \"exclude\": []\n}\n".to_string()
}

/// A filesystem-safe name. Slashes survive, because a name with one in it nests into
/// collections — `github/issues` is a feature, not an error.
pub fn slug_path(name: &str) -> Result<String, String> {
    let mut parts = Vec::new();
    for raw in name.split('/') {
        let seg: String = raw
            .trim()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let seg = seg.trim_matches('-').to_string();
        if seg.is_empty() || seg.starts_with("__") || seg == "." || seg == ".." {
            return Err(format!("`{name}` is not a usable request name"));
        }
        parts.push(seg);
    }
    if parts.is_empty() {
        return Err("a request name is required".to_string());
    }
    Ok(parts.join("/"))
}

/// A single path segment — the same rules, with any `/` folded away.
pub fn slug_segment(name: &str, fallback: &str) -> String {
    slug_path(name)
        .map(|s| s.replace('/', "-"))
        .unwrap_or_else(|_| fallback.to_string())
}

/// `apis/github/issues/__metadata.md` for `github/issues`.
pub fn request_path(rel: &str) -> String {
    format!("{APIS_DIR}/{rel}/{REQUEST_FILE}")
}

/// `apis/github/__collection.md` for `github`; `apis/__collection.md` for the root.
pub fn collection_path(rel: &str) -> String {
    if rel.is_empty() {
        format!("{APIS_DIR}/{COLLECTION_FILE}")
    } else {
        format!("{APIS_DIR}/{rel}/{COLLECTION_FILE}")
    }
}

pub fn environment_path(name: &str) -> String {
    format!("{ENVS_DIR}/{name}.md")
}

/// Is this directory name one of `rq`'s own (`__examples/`, `__scripts/`) rather than an
/// entity? Also true for hidden directories.
pub fn is_reserved_dir(name: &str) -> bool {
    name.starts_with('.') || name.starts_with("__")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_keep_nesting_and_reject_traversal() {
        assert_eq!(slug_path("github/issues").unwrap(), "github/issues");
        assert_eq!(slug_path("My Request!").unwrap(), "My-Request");
        assert!(slug_path("../etc").is_err());
        assert!(slug_path("").is_err());
        assert_eq!(slug_segment("a/b", "x"), "a-b");
        assert_eq!(slug_segment("", "fallback"), "fallback");
    }

    #[test]
    fn paths_match_the_documented_layout() {
        assert_eq!(
            request_path("github/issues"),
            "apis/github/issues/__metadata.md"
        );
        assert_eq!(collection_path("github"), "apis/github/__collection.md");
        assert_eq!(collection_path(""), "apis/__collection.md");
        assert_eq!(environment_path("staging"), "environments/staging.md");
    }
}
