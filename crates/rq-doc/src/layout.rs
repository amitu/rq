//! The shape of an `rq` project on disk.
//!
//! ```text
//! my-apis/
//! ├── rq.toml          the project marker — discovery walks up to it, git-style
//! ├── issues.md        a request
//! ├── github/          a collection — just a directory
//! │   ├── index.md     its shared headers / auth / vars, and its landing page
//! │   ├── login.md
//! │   └── me.md
//! ├── env/
//! │   └── staging.md   a named environment, for `-e staging`
//! ├── .env             the always-on variable layer
//! └── .rq/             machine-local state (the active environment); gitignored
//! ```
//!
//! Three rules and there is nothing else to learn:
//!
//! - **A request is a file.** One markdown file, named for the request.
//! - **A collection is a directory.** Its `index.md` holds what it shares with everything
//!   beneath it — and if that file has a `url:`, the collection has a landing page.
//! - **Anything else is yours.** A `.md` with no frontmatter is documentation: keep your
//!   README and your notes right next to the requests they describe.
//!
//! These names live here rather than in the CLI because the converter writes this layout
//! too: one definition of where a request goes, read by everything that reads or writes it.

/// The project marker. Discovery walks up from the cwd looking for this file.
pub const MARKER: &str = "rq.toml";
/// What a fresh marker contains — enough to be valid, not so much as to be noise.
pub const MARKER_TEMPLATE: &str = "# An rq project. https://github.com/browserstack/rq\n\
                                   [project]\nversion = 1\n";
/// A collection's own file: its shared settings, and optionally its landing request.
pub const COLLECTION_FILE: &str = "index.md";
/// Named environments live here.
pub const ENVS_DIR: &str = "env";
/// The always-on variable layer, in the format every project already has one of.
pub const DOTENV: &str = ".env";
/// Machine-local state (which environment is active). Not part of the collection.
pub const STATE_DIR: &str = ".rq";
pub const EXTENSION: &str = "md";

/// The contents of a fresh `rq.toml`.
pub fn marker() -> String {
    MARKER_TEMPLATE.to_string()
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
        if seg.is_empty() || seg == "." || seg == ".." || seg.starts_with('.') {
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

/// `github/issues` → `github/issues.md`.
pub fn request_path(rel: &str) -> String {
    format!("{rel}.{EXTENSION}")
}

/// `github` → `github/index.md`; the project root's own is just `index.md`.
pub fn collection_path(rel: &str) -> String {
    if rel.is_empty() {
        COLLECTION_FILE.to_string()
    } else {
        format!("{rel}/{COLLECTION_FILE}")
    }
}

pub fn environment_path(name: &str) -> String {
    format!("{ENVS_DIR}/{name}.{EXTENSION}")
}

/// Is this directory the tool's own rather than a collection? Hidden directories and the
/// environment directory are not collections.
pub fn is_reserved_dir(name: &str) -> bool {
    name.starts_with('.') || name == ENVS_DIR
}

/// Is this file a candidate request? `index.md` is the collection's, not a request of its
/// own, and anything that isn't markdown is somebody else's business.
pub fn is_request_file(name: &str) -> bool {
    name != COLLECTION_FILE
        && !name.starts_with('.')
        && std::path::Path::new(name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// The request name a file has: `issues.md` → `issues`.
pub fn request_name(file: &str) -> Option<&str> {
    file.strip_suffix(&format!(".{EXTENSION}"))
        .filter(|stem| !stem.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_keep_nesting_and_reject_traversal() {
        assert_eq!(slug_path("github/issues").unwrap(), "github/issues");
        assert_eq!(slug_path("My Request!").unwrap(), "My-Request");
        assert!(slug_path("../etc").is_err());
        assert!(slug_path(".hidden").is_err());
        assert!(slug_path("").is_err());
        assert_eq!(slug_segment("a/b", "x"), "a-b");
    }

    #[test]
    fn paths_match_the_documented_layout() {
        assert_eq!(request_path("github/issues"), "github/issues.md");
        assert_eq!(collection_path("github"), "github/index.md");
        assert_eq!(collection_path(""), "index.md");
        assert_eq!(environment_path("staging"), "env/staging.md");
    }

    #[test]
    fn a_request_file_is_markdown_that_is_not_the_index() {
        assert!(is_request_file("issues.md"));
        assert!(!is_request_file("index.md"));
        assert!(!is_request_file("README.txt"));
        assert!(!is_request_file(".hidden.md"));
        assert_eq!(request_name("issues.md"), Some("issues"));
        assert_eq!(request_name("issues"), None);
    }

    #[test]
    fn the_env_directory_is_not_a_collection() {
        assert!(is_reserved_dir("env"));
        assert!(is_reserved_dir(".rq"));
        assert!(!is_reserved_dir("github"));
    }
}
