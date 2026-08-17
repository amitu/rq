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
pub use rq_doc::layout::{COLLECTION_FILE, DOTENV, ENVS_DIR, MARKER, STATE_DIR};

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
    /// A collection whose `index.md` has a `url:` — its landing page, runnable by the
    /// collection's own name. Always true for a request.
    pub runnable: bool,
}

impl Entry {
    /// The file that defines this entity. A collection without an `index.md` is a perfectly
    /// good collection — it just has nothing to say, and that path will not exist.
    pub fn file(&self) -> PathBuf {
        match self.kind {
            Kind::Request => self.dir.clone(),
            Kind::Collection => self.dir.join(COLLECTION_FILE),
        }
    }

    /// The directory this entity lives in — itself for a collection, its parent for a
    /// request.
    pub fn dir(&self) -> PathBuf {
        match self.kind {
            Kind::Request => self
                .dir
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.dir.clone()),
            Kind::Collection => self.dir.clone(),
        }
    }
}

#[derive(Debug)]
pub struct Project {
    pub root: PathBuf,
    pub entries: Vec<Entry>,
    /// Indices of the top-level entries, in display order.
    pub roots: Vec<usize>,
    /// Files that looked like requests but weren't usable — reported, never silently
    /// skipped.
    pub notes: Vec<String>,
}

/// What a `.md` file in a project turned out to be.
enum Classification {
    Request,
    /// No frontmatter at all: someone's notes, and none of our business.
    Documentation,
    Unusable(String),
}

fn classify(path: &Path) -> Classification {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Classification::Unusable("could not be read".into());
    };
    if !text.trim_start().starts_with("---") {
        return Classification::Documentation;
    }
    match Document::parse(&text) {
        Ok((doc, _)) if doc.front.url.is_some() => Classification::Request,
        Ok(_) => Classification::Unusable(
            "has frontmatter but no `url:`, so it is not a request — add one, or remove the \
             frontmatter to keep it as notes"
                .into(),
        ),
        Err(e) => Classification::Unusable(e),
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

fn display_rel(prefix: &str, name: &str) -> String {
    join(prefix, name)
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
            notes: Vec::new(),
        };
        let root = project.root.clone();
        project.roots = project.scan(&root, None, "")?;
        Ok(project)
    }

    fn scan(&mut self, dir: &Path, parent: Option<usize>, prefix: &str) -> Result<Vec<usize>> {
        let mut dirs: Vec<(String, PathBuf)> = Vec::new();
        let mut files: Vec<(String, PathBuf)> = Vec::new();

        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.file_type()?.is_dir() {
                if !rq_doc::layout::is_reserved_dir(&name) {
                    dirs.push((name, entry.path()));
                }
            } else if rq_doc::layout::is_request_file(&name) {
                files.push((name, entry.path()));
            }
        }
        dirs.sort_by(|a, b| a.0.cmp(&b.0));
        files.sort_by(|a, b| a.0.cmp(&b.0));

        let mut out = Vec::new();

        // Requests first: a directory listing reads better when the things you can run come
        // before the things you have to open.
        for (file, path) in files {
            let Some(name) = rq_doc::layout::request_name(&file).map(str::to_string) else {
                continue;
            };
            // A markdown file with no frontmatter is documentation — a README next to the
            // requests it describes, which is the point of keeping them in one directory.
            match classify(&path) {
                Classification::Request => {}
                Classification::Documentation => continue,
                Classification::Unusable(why) => {
                    self.notes
                        .push(format!("{}: {why}", display_rel(prefix, &name)));
                    continue;
                }
            }
            let rel = join(prefix, &name);
            let idx = self.entries.len();
            self.entries.push(Entry {
                kind: Kind::Request,
                dir: path,
                rel,
                name,
                parent,
                children: Vec::new(),
                runnable: true,
            });
            out.push(idx);
        }

        for (name, path) in dirs {
            let rel = join(prefix, &name);
            let index = path.join(COLLECTION_FILE);
            // A collection's index may be a request in its own right: the landing page you
            // get by naming the collection.
            let runnable = index.is_file() && matches!(classify(&index), Classification::Request);
            let idx = self.entries.len();
            self.entries.push(Entry {
                kind: Kind::Collection,
                dir: path.clone(),
                rel: rel.clone(),
                name,
                parent,
                children: Vec::new(),
                runnable,
            });
            let children = self.scan(&path, Some(idx), &rel)?;
            self.entries[idx].children = children;
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
            .position(|e| e.rel == needle && e.runnable)
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
                .position(|e| e.runnable && e.dir.canonicalize().ok() == canon)
            {
                return Ok(i);
            }
        }

        let matches: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.runnable && e.name == needle)
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
            .filter(|e| e.runnable)
            .map(|e| (common_prefix(&e.name, needle), e))
            .filter(|(score, _)| *score >= 2)
            // Best match first; on a tie the least-nested one, which is the likelier
            // intent and — unlike "whichever we saw last" — the same answer every time.
            .min_by_key(|(score, e)| (std::cmp::Reverse(*score), e.rel.len()))
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

    /// The project's own `index.md`: what every request in it shares.
    ///
    /// The project root is a collection like any other directory, but it has no entry in
    /// the tree to hang from — so it is read from here.
    pub fn root_collection(&self) -> Result<Option<(Document, Vec<Note>)>> {
        let path = self.root.join(COLLECTION_FILE);
        if !path.is_file() {
            return Ok(None);
        }
        load_document(&path).map(Some)
    }

    /// A collection's own `__collection.md`, if it wrote one.
    pub fn load_collection(&self, idx: usize) -> Result<Option<(Document, Vec<Note>)>> {
        let path = self.entries[idx].file();
        if !path.is_file() {
            return Ok(None);
        }
        load_document(&path).map(Some)
    }

    /// Everything that can be run, in tree order.
    pub fn requests(&self) -> impl Iterator<Item = (usize, &Entry)> {
        self.entries.iter().enumerate().filter(|(_, e)| e.runnable)
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

    /// The always-on variable layer: `KEY=value` lines from `.env`, `#` comments and an
    /// optional `export ` allowed. Absent is simply empty — a project with one set of
    /// values needs nothing else.
    pub fn dotenv(&self) -> Vec<(String, String)> {
        let Ok(text) = std::fs::read_to_string(self.root.join(DOTENV)) else {
            return Vec::new();
        };
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .filter_map(|line| line.strip_prefix("export ").unwrap_or(line).split_once('='))
            .map(|(key, value)| {
                (
                    key.trim().to_string(),
                    value
                        .trim()
                        .trim_matches(|c| c == '"' || c == '\'')
                        .to_string(),
                )
            })
            .collect()
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

/// Create a project. One file: a project is a directory of markdown, and an `rq init` that
/// scattered empty directories would be making that harder to see, not easier.
pub fn init(root: &Path) -> Result<bool> {
    let marker = root.join(MARKER);
    if marker.is_file() {
        return Ok(false);
    }
    std::fs::create_dir_all(root)?;
    std::fs::write(&marker, rq_doc::layout::marker())?;

    let gitignore = root.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(
            &gitignore,
            "# rq keeps the active environment and other machine-local state here.\n             .rq/\n\n             # Secrets belong to your machine, not to the collection.\n             .env\n",
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
            let path = root.join(format!("{rel}.md"));
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
        let deep = p.root.join("github");
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
