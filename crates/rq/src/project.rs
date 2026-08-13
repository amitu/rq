//! Finding the project, reading its tree, and resolving a name to a request.
//!
//! A project is a directory of plain files, discovered the way `git` discovers a repo:
//! walk up from the cwd until the marker appears.
//!
//! ```text
//! my-apis/
//! ├── __requestly.json          project marker
//! ├── apis/
//! │   ├── issues/__metadata.md  a request
//! │   └── github/               a collection
//! │       ├── __collection.md   its shared headers / auth / vars (optional)
//! │       └── login/__metadata.md
//! ├── environments/
//! │   ├── __global.md
//! │   └── staging.md
//! └── .requestly/state.json     which environment is active (machine-local)
//! ```
//!
//! The tree *is* the hierarchy: a request's parent collection is the directory above it.
//! Nothing stores a parent id, so `git mv` is a legal way to reorganize a collection.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::doc::{Document, Note};

// The layout itself is defined in `rq-doc`, so the converter writes the same tree the CLI
// reads. Re-exported here because this is where the rest of the CLI looks for it.
pub use rq_doc::layout::{
    APIS_DIR, COLLECTION_FILE, ENVS_DIR, GLOBAL_ENV, MARKER, REQUEST_FILE, STATE_DIR,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Request,
    Collection,
}

/// One node of the tree. Entries live in an arena so a request can walk to its ancestors
/// (for inherited headers and auth) without borrowing gymnastics.
#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: Kind,
    /// Directory holding the entity.
    pub dir: PathBuf,
    /// Slash-separated path below `apis/` — the unambiguous name (`github/issues`).
    pub rel: String,
    /// Last segment — the short name people type (`issues`).
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

impl Entry {
    /// The file that defines this entity, if it has one. A collection folder without a
    /// `__collection.md` is a perfectly good collection — it just has nothing to say.
    pub fn file(&self) -> PathBuf {
        match self.kind {
            Kind::Request => self.dir.join(REQUEST_FILE),
            Kind::Collection => self.dir.join(COLLECTION_FILE),
        }
    }
}

#[derive(Debug)]
pub struct Project {
    pub root: PathBuf,
    pub entries: Vec<Entry>,
    /// Indices of the top-level entries, in display order.
    pub roots: Vec<usize>,
}

impl Project {
    /// Locate the project: an explicit `--project`, then `RQ_PROJECT`, then the marker
    /// walking up from `start`.
    pub fn find(explicit: Option<&Path>, start: &Path) -> Result<Project> {
        let root = if let Some(p) = explicit {
            let p = p.to_path_buf();
            if !p.join(MARKER).is_file() {
                bail!("{} is not an rq project (no {MARKER})", p.display());
            }
            p
        } else if let Some(p) = std::env::var_os("RQ_PROJECT") {
            let p = PathBuf::from(p);
            if !p.join(MARKER).is_file() {
                bail!("RQ_PROJECT={} has no {MARKER}", p.display());
            }
            p
        } else {
            let mut cur = Some(start);
            loop {
                match cur {
                    Some(dir) if dir.join(MARKER).is_file() => break dir.to_path_buf(),
                    Some(dir) => cur = dir.parent(),
                    None => bail!(
                        "no rq project found in {} or any parent directory\n  \
                         run `rq init` to start one, or `rq curl --save-as <name> …` to \
                         create one from a curl",
                        start.display()
                    ),
                }
            }
        };
        Project::open(root)
    }

    pub fn open(root: PathBuf) -> Result<Project> {
        let mut project = Project {
            root,
            entries: Vec::new(),
            roots: Vec::new(),
        };
        let apis = project.root.join(APIS_DIR);
        if apis.is_dir() {
            project.roots = project.scan(&apis, None, "")?;
        }
        Ok(project)
    }

    fn scan(&mut self, dir: &Path, parent: Option<usize>, prefix: &str) -> Result<Vec<usize>> {
        let mut kids: Vec<(String, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // `__`-prefixed folders are rq's own (`__examples/`, `__scripts/`), never entities.
            if rq_doc::layout::is_reserved_dir(&name) {
                continue;
            }
            kids.push((name, entry.path()));
        }
        kids.sort_by(|a, b| a.0.cmp(&b.0));

        let mut out = Vec::new();
        for (name, path) in kids {
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let kind = if path.join(REQUEST_FILE).is_file() {
                Kind::Request
            } else {
                Kind::Collection
            };
            let idx = self.entries.len();
            self.entries.push(Entry {
                kind,
                dir: path.clone(),
                rel: rel.clone(),
                name,
                parent,
                children: Vec::new(),
            });
            if kind == Kind::Collection {
                let children = self.scan(&path, Some(idx), &rel)?;
                self.entries[idx].children = children;
            }
            out.push(idx);
        }
        Ok(out)
    }

    /// Resolve what the user typed to exactly one request. Accepts the short name
    /// (`issues`), the qualified path (`github/issues`), or a filesystem path.
    pub fn resolve(&self, query: &str) -> Result<usize> {
        let needle = query.trim_matches('/').replace('\\', "/");

        if let Some(i) = self
            .entries
            .iter()
            .position(|e| e.rel == needle && e.kind == Kind::Request)
        {
            return Ok(i);
        }

        // A path on disk — `rq r ./apis/github/issues` after a tab-complete.
        let as_path = Path::new(query);
        if as_path.exists() {
            let canon = as_path.canonicalize().ok();
            if let Some(i) = self
                .entries
                .iter()
                .position(|e| e.kind == Kind::Request && e.dir.canonicalize().ok() == canon)
            {
                return Ok(i);
            }
        }

        let matches: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == Kind::Request && e.name == needle)
            .map(|(i, _)| i)
            .collect();

        match matches.len() {
            1 => Ok(matches[0]),
            0 => {
                let near = self.nearest(&needle);
                bail!(
                    "no request named `{query}`{}\n  `rq l` lists everything in this project",
                    near.map(|n| format!(" (did you mean `{n}`?)"))
                        .unwrap_or_default()
                )
            }
            _ => {
                let list: Vec<&str> = matches
                    .iter()
                    .map(|i| self.entries[*i].rel.as_str())
                    .collect();
                bail!(
                    "`{query}` is ambiguous — {} requests share that name:\n  {}\n  \
                     use the full path, e.g. `rq r {}`",
                    matches.len(),
                    list.join("\n  "),
                    list[0]
                )
            }
        }
    }

    fn nearest(&self, needle: &str) -> Option<&str> {
        self.entries
            .iter()
            .filter(|e| e.kind == Kind::Request)
            .map(|e| (common_prefix(&e.name, needle), e))
            .filter(|(score, _)| *score >= 2)
            .max_by_key(|(score, _)| *score)
            .map(|(_, e)| e.rel.as_str())
    }

    /// Every collection above `idx`, outermost first — the order inherited settings apply in.
    pub fn ancestors(&self, idx: usize) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut cur = self.entries[idx].parent;
        while let Some(i) = cur {
            chain.push(i);
            cur = self.entries[i].parent;
        }
        chain.reverse();
        chain
    }

    pub fn load(&self, idx: usize) -> Result<(Document, Vec<Note>)> {
        let path = self.entries[idx].file();
        load_document(&path)
    }

    /// A collection's own `__collection.md`, if it wrote one.
    pub fn load_collection(&self, idx: usize) -> Result<Option<(Document, Vec<Note>)>> {
        let path = self.entries[idx].file();
        if !path.is_file() {
            return Ok(None);
        }
        load_document(&path).map(Some)
    }

    pub fn requests(&self) -> impl Iterator<Item = (usize, &Entry)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == Kind::Request)
    }

    // --- environments --------------------------------------------------------------------

    pub fn env_dir(&self) -> PathBuf {
        self.root.join(ENVS_DIR)
    }

    /// Environment names on disk, global first, then alphabetical.
    pub fn environments(&self) -> Vec<String> {
        let mut names: Vec<String> = match std::fs::read_dir(self.env_dir()) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                .filter_map(|e| {
                    e.path()
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names.sort_by_key(|n| n != GLOBAL_ENV);
        names
    }

    pub fn env_path(&self, name: &str) -> PathBuf {
        self.env_dir().join(format!("{name}.md"))
    }

    /// Read an environment file. Environments use the same document format as requests —
    /// one `vars:` block — so there is one parser and one set of rules to learn.
    pub fn load_env(&self, name: &str) -> Result<(Document, Vec<Note>)> {
        let path = self.env_path(name);
        if !path.is_file() {
            let known = self.environments();
            bail!(
                "no environment `{name}` in {}{}",
                self.env_dir().display(),
                if known.is_empty() {
                    String::new()
                } else {
                    format!(" (have: {})", known.join(", "))
                }
            );
        }
        load_document(&path)
    }

    fn state_path(&self) -> PathBuf {
        self.root.join(STATE_DIR).join("state.json")
    }

    /// The active environment. Machine-local (under `.requestly/`, gitignored) because
    /// "which environment am I pointed at" is a property of your shell, not of the repo.
    pub fn active_env(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.state_path()).ok()?;
        let json: serde_json::Value = serde_json::from_str(&text).ok()?;
        json.get("environment")?.as_str().map(str::to_string)
    }

    pub fn set_active_env(&self, name: Option<&str>) -> Result<()> {
        let dir = self.root.join(STATE_DIR);
        std::fs::create_dir_all(&dir)?;
        let body = match name {
            Some(n) => serde_json::json!({ "environment": n }),
            None => serde_json::json!({}),
        };
        std::fs::write(self.state_path(), format!("{body:#}\n"))?;
        Ok(())
    }
}

pub fn load_document(path: &Path) -> Result<(Document, Vec<Note>)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Document::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

/// Write a document, creating parents. Every write goes through here so files land with a
/// trailing newline and no partial state.
pub fn save_document(path: &Path, doc: &Document) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, doc.write()).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Create a project skeleton. Idempotent: an existing marker is left alone.
pub fn init(root: &Path) -> Result<bool> {
    let marker = root.join(MARKER);
    if marker.is_file() {
        return Ok(false);
    }
    std::fs::create_dir_all(root.join(APIS_DIR))?;
    std::fs::create_dir_all(root.join(ENVS_DIR))?;
    std::fs::write(&marker, rq_doc::layout::marker())?;

    let gitignore = root.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(
            &gitignore,
            "# rq keeps the active environment and other machine-local state here.\n.requestly/\n",
        )?;
    }
    Ok(true)
}

/// A filesystem-safe folder name for `--save-as`, from the format crate.
pub fn slug_path(name: &str) -> Result<String> {
    rq_doc::layout::slug_path(name).map_err(|e| anyhow::anyhow!(e))
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(x, y)| x.eq_ignore_ascii_case(y))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        init(&root).unwrap();
        for rel in ["issues", "github/login", "github/issues", "acme/login"] {
            let path = root.join(APIS_DIR).join(rel).join(REQUEST_FILE);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "---\nurl: https://x.test\n---\n").unwrap();
        }
        let project = Project::open(root).unwrap();
        (dir, project)
    }

    #[test]
    fn scans_the_tree() {
        let (_d, p) = fixture();
        assert_eq!(p.requests().count(), 4);
        let names: Vec<&str> = p.entries.iter().map(|e| e.rel.as_str()).collect();
        assert!(names.contains(&"github"));
        assert!(names.contains(&"github/login"));
    }

    #[test]
    fn resolves_unique_names_and_reports_ambiguity() {
        let (_d, p) = fixture();
        assert_eq!(p.entries[p.resolve("issues").unwrap()].rel, "issues");
        assert_eq!(
            p.entries[p.resolve("github/login").unwrap()].rel,
            "github/login"
        );
        let err = p.resolve("login").unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "{err}");
        assert!(err.contains("github/login"), "{err}");
    }

    #[test]
    fn unknown_name_suggests_a_neighbour() {
        let (_d, p) = fixture();
        let err = p.resolve("issue").unwrap_err().to_string();
        assert!(err.contains("did you mean `issues`"), "{err}");
    }

    #[test]
    fn finds_the_root_by_walking_up() {
        let (_d, p) = fixture();
        let deep = p.root.join(APIS_DIR).join("github").join("login");
        let found = Project::find(None, &deep).unwrap();
        assert_eq!(
            found.root.canonicalize().unwrap(),
            p.root.canonicalize().unwrap()
        );
    }

    #[test]
    fn ancestors_are_outermost_first() {
        let (_d, p) = fixture();
        let idx = p.resolve("github/login").unwrap();
        let chain: Vec<&str> = p
            .ancestors(idx)
            .iter()
            .map(|i| p.entries[*i].rel.as_str())
            .collect();
        assert_eq!(chain, vec!["github"]);
    }

    #[test]
    fn active_environment_round_trips() {
        let (_d, p) = fixture();
        assert_eq!(p.active_env(), None);
        p.set_active_env(Some("staging")).unwrap();
        assert_eq!(p.active_env().as_deref(), Some("staging"));
    }
}
