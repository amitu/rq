//! Finding the project, reading its tree, and resolving a name to a request.
//!
//! A project is a directory of plain files, discovered the way `git` discovers a repo:
//! walk up from the cwd until the marker appears.
//!
//! ```text
//! my-apis/
//! ├── rq.toml            project marker
//! ├── issues.md          a request — one file, no folder
//! ├── github/            a collection — just a directory
//! │   ├── index.md       its shared headers / auth / vars (optional)
//! │   └── login.md
//! ├── env/
//! │   └── staging.md
//! ├── .env               secrets, gitignored
//! └── .rq/state.json     which environment is active (machine-local)
//! ```
//!
//! The tree *is* the hierarchy: a request's parent collection is the directory above it.
//! Nothing stores a parent id, so `git mv` is a legal way to reorganize a collection.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

use crate::doc::{Document, Note};

// The layout itself is defined in `rq-doc`, so the converter writes the same tree the CLI
// reads. Re-exported here because this is where the rest of the CLI looks for it.
pub use rq_doc::layout::{COLLECTION_FILE, DOTENV, ENVS_DIR, MARKER, STATE_DIR};

/// A tree child: the name as written, and where its document lives.
type Named = (String, PathBuf);

/// One thing the converter said about the source, kept with its locator.
#[derive(Clone, Debug)]
pub struct ConversionNote {
    /// True when the source item could not be read at all — a request that is missing from
    /// what rq loaded, as opposed to one that came through with less than it had.
    pub fatal: bool,
    /// Which file (or which part of it) the converter was looking at.
    pub at: String,
    pub message: String,
}

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

/// Where a project's documents come from.
///
/// `Converted` is what makes `rq ./collection.postman_collection.json` work. cross-q turns
/// the foreign file into the very markdown an rq project on disk would hold, and the tree is
/// built from that map instead of from `read_dir`. Nothing downstream — running, inheritance,
/// the console, `--json` — can tell the difference, because there is no difference: it is the
/// same converter `rq import` runs, minus the writing.
#[derive(Debug)]
pub enum Source {
    /// Files on disk, below `root`.
    Disk,
    /// A converted foreign collection, held in memory. Reads never touch the disk, so the
    /// original file is never at risk and nothing is left behind.
    Converted {
        /// The file or directory it was read from — for messages, and for `rq e`.
        from: PathBuf,
        /// What cross-q decided it was: `postman`, `bruno`, `curl`, `rq`.
        format: String,
        /// Absolute virtual path → contents.
        files: BTreeMap<PathBuf, String>,
        /// What the converter had to say while reading it. Carried rather than summarised
        /// away, because "1 dropped" and "`broken.bru`: no HTTP method block" are not the
        /// same sentence — and a dropped request is a request you have that rq does not.
        diagnostics: Vec<ConversionNote>,
    },
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
    /// Disk, or a foreign collection converted in memory.
    pub source: Source,
}

/// What a `.md` file in a project turned out to be.
enum Classification {
    Request,
    /// No frontmatter at all: someone's notes, and none of our business.
    Documentation,
    Unusable(String),
}

fn classify(text: Option<String>) -> Classification {
    let Some(text) = text else {
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

/// Collections lying about that rq could read directly, sorted by name.
///
/// Only files cross-q positively identifies count — a Postman export says so in its own
/// `info`/`_postman_id`, a `.bru` says `meta {`. A folder of unrelated JSON is not a project
/// and does not become one by being looked at.
pub fn readable_collections(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
        })
        .filter(|p| {
            // Read only what could plausibly be one; `detect_format` needs the text, and a
            // 200 MB video is not a collection.
            std::fs::metadata(p)
                .map(|m| m.len() < 32 * 1024 * 1024)
                .unwrap_or(false)
                && std::fs::read_to_string(p)
                    .ok()
                    .and_then(|text| crate::import::detect_format(p, &text))
                    .is_some()
        })
        .collect();
    found.sort();
    found
}

/// A file whose NAME says what it is, which rq could not read.
///
/// "no rq project found" is a poor answer when `acme.postman_collection.json` is sitting
/// right there. The useful answer names the file and says what the reader made of it —
/// which is also the answer to "what would anyone do if the JSON is corrupted": be told
/// which file, and why, instead of being told there is nothing here.
fn announce_unreadable(dir: &Path) -> Option<anyhow::Error> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut named: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.ends_with(".postman_collection.json")
                    || n == "bruno.json"
                    || n == "collection.json"
            })
        })
        .collect();
    named.sort();
    let candidate = named.first()?;
    let name = candidate
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let format = if name == "bruno.json" {
        "bruno"
    } else {
        "postman"
    };
    let mut report = cq_report::Report::new(cq_report::Fidelity::Lossless);
    let why = match crate::import::to_project_map(candidate, Some(format), &mut report) {
        Ok(_) => "it did not look like a collection when rq read it".to_string(),
        Err(e) => e.to_string(),
    };
    Some(anyhow!(
        "{name} is here but rq could not read it as {format}:\n  {why}"
    ))
}

impl Project {
    /// Locate the project: an explicit `--project`, then `RQ_PROJECT`, then the marker
    /// walking up from `start` — and failing all of that, a collection sitting right here
    /// that rq can read without being asked twice.
    pub fn find(explicit: Option<&Path>, start: &Path) -> Result<Project> {
        Project::locate(explicit, start, None).map(|(p, _)| p)
    }

    /// `find`, plus the conversion report when the project turned out to be a foreign
    /// collection — what the converter had to say about reading it.
    pub fn locate(
        explicit: Option<&Path>,
        start: &Path,
        format: Option<&str>,
    ) -> Result<(Project, Option<cq_report::Report>)> {
        // An explicit target that is a file is never an rq project directory — it is the
        // collection itself, which is exactly what `--project foo.postman_collection.json`
        // is asking for.
        if let Some(p) = explicit {
            if p.is_file() || (p.is_dir() && !p.join(MARKER).is_file()) {
                let (project, report) = Project::open_foreign(p, format)?;
                return Ok((project, Some(report)));
            }
        }
        Project::find_on_disk(explicit, start)
            .map(|p| (p, None))
            .or_else(|e| {
                // Nothing of ours here. Before failing, look for something we can read anyway.
                match readable_collections(start).as_slice() {
                    [] => Err(announce_unreadable(start).unwrap_or(e)),
                    [one] => {
                        let (project, report) = Project::open_foreign(one, format)?;
                        Ok((project, Some(report)))
                    }
                    many => bail!(
                        "no rq project here, but {} collections rq can read:\n{}\n  \
                     name one — `rq l <file>` — or `rq import <file>` to make it a project",
                        many.len(),
                        many.iter()
                            .map(|p| format!(
                                "    {}",
                                p.file_name().unwrap_or_default().to_string_lossy()
                            ))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                }
            })
    }

    /// Strictly a project on disk — the marker, or nothing. Writers use this: `rq curl
    /// --save-as` and `rq import` need somewhere to put a file, and a collection rq is
    /// merely reading is not that.
    pub fn find_on_disk(explicit: Option<&Path>, start: &Path) -> Result<Project> {
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
        Project::build(root, Source::Disk)
    }

    /// Open a foreign collection — a Postman export, a Bruno tree, a curl file — **in place**.
    ///
    /// No temporary directory, no files written next to someone's download: cross-q converts
    /// it to the rq project it describes and that map *is* the project. `format` forces the
    /// reader when the file is ambiguous; `None` lets cross-q decide by looking.
    pub fn open_foreign(from: &Path, format: Option<&str>) -> Result<(Project, cq_report::Report)> {
        let mut report = cq_report::Report::new(cq_report::Fidelity::Lossless);
        let (map, format) = crate::import::to_project_map(from, format, &mut report)?;
        // The source path doubles as the project root. For a file that is a path with no
        // directory at it — deliberately, so a scan can never wander into whatever else
        // happens to sit in the same folder.
        let root = from.to_path_buf();
        let files = map
            .into_iter()
            .map(|(rel, content)| (root.join(rel), content))
            .collect();
        let diagnostics = report
            .diagnostics
            .iter()
            .filter(|d| {
                matches!(
                    d.severity,
                    cq_report::Severity::Coerced
                        | cq_report::Severity::Dropped
                        | cq_report::Severity::Error
                )
            })
            .map(|d| ConversionNote {
                // `Error` is the converter saying it could not complete an item — the
                // request is not there. `Dropped`/`Coerced` came through with less than it
                // had, which is worth knowing and is not the same thing.
                fatal: d.severity == cq_report::Severity::Error,
                at: if d.provenance.locator.is_empty() {
                    from.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                } else {
                    d.provenance.locator.clone()
                },
                message: d.message.clone(),
            })
            .collect();
        let project = Project::build(
            root,
            Source::Converted {
                from: from.to_path_buf(),
                format,
                files,
                diagnostics,
            },
        )?;
        Ok((project, report))
    }

    fn build(root: PathBuf, source: Source) -> Result<Project> {
        let mut project = Project {
            root,
            entries: Vec::new(),
            roots: Vec::new(),
            notes: Vec::new(),
            source,
        };
        let root = project.root.clone();
        project.roots = project.scan(&root, None, "")?;
        Ok(project)
    }

    /// True when this project was converted rather than read from disk.
    pub fn is_converted(&self) -> bool {
        matches!(self.source, Source::Converted { .. })
    }

    /// What a converted project came from, and what cross-q read it as.
    pub fn converted_from(&self) -> Option<(&Path, &str)> {
        match &self.source {
            Source::Converted { from, format, .. } => Some((from.as_path(), format.as_str())),
            Source::Disk => None,
        }
    }

    /// What the converter said while reading the source. Empty for a project on disk.
    pub fn conversion_notes(&self) -> &[ConversionNote] {
        match &self.source {
            Source::Converted { diagnostics, .. } => diagnostics,
            Source::Disk => &[],
        }
    }

    /// Read one of the project's documents, wherever they live.
    fn read(&self, path: &Path) -> Option<String> {
        match &self.source {
            Source::Disk => std::fs::read_to_string(path).ok(),
            Source::Converted { files, .. } => files.get(path).cloned(),
        }
    }

    /// Does this document exist? `is_file()` on disk; a key in the map when converted.
    fn has(&self, path: &Path) -> bool {
        match &self.source {
            Source::Disk => path.is_file(),
            Source::Converted { files, .. } => files.contains_key(path),
        }
    }

    /// The immediate children of a directory: `(subdirectories, request files)`. Converted
    /// projects have no directories to read, so the tree is recovered from the map's keys.
    fn list(&self, dir: &Path) -> Result<(Vec<Named>, Vec<Named>)> {
        let mut dirs: Vec<Named> = Vec::new();
        let mut files: Vec<Named> = Vec::new();

        match &self.source {
            Source::Disk => {
                for entry in
                    std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?
                {
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
            }
            Source::Converted { files: map, .. } => {
                let mut seen_dirs = std::collections::BTreeSet::new();
                for path in map.keys() {
                    let Ok(rest) = path.strip_prefix(dir) else {
                        continue;
                    };
                    let mut parts = rest.components();
                    let Some(first) = parts.next() else { continue };
                    let name = first.as_os_str().to_string_lossy().to_string();
                    if parts.next().is_some() {
                        // Something below a subdirectory: the subdirectory is the child.
                        if !rq_doc::layout::is_reserved_dir(&name) && seen_dirs.insert(name.clone())
                        {
                            dirs.push((name, dir.join(first)));
                        }
                    } else if rq_doc::layout::is_request_file(&name) {
                        files.push((name, path.clone()));
                    }
                }
            }
        }
        dirs.sort_by(|a, b| a.0.cmp(&b.0));
        files.sort_by(|a, b| a.0.cmp(&b.0));
        Ok((dirs, files))
    }

    fn scan(&mut self, dir: &Path, parent: Option<usize>, prefix: &str) -> Result<Vec<usize>> {
        let (dirs, files) = self.list(dir)?;
        let mut out = Vec::new();

        // Requests first: a directory listing reads better when the things you can run come
        // before the things you have to open.
        for (file, path) in files {
            let Some(name) = rq_doc::layout::request_name(&file).map(str::to_string) else {
                continue;
            };
            // A markdown file with no frontmatter is documentation — a README next to the
            // requests it describes, which is the point of keeping them in one directory.
            match classify(self.read(&path)) {
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
            let runnable =
                self.has(&index) && matches!(classify(self.read(&index)), Classification::Request);
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
        self.parse(&path)
    }

    /// Parse one document from wherever this project keeps it.
    fn parse(&self, path: &Path) -> Result<(Document, Vec<Note>)> {
        match &self.source {
            Source::Disk => load_document(path),
            Source::Converted { .. } => {
                let text = self
                    .read(path)
                    .ok_or_else(|| anyhow!("{} is not in this collection", path.display()))?;
                Document::parse(&text).map_err(|e| anyhow!("{}: {e}", path.display()))
            }
        }
    }

    /// The project's own `index.md`: what every request in it shares.
    ///
    /// The project root is a collection like any other directory, but it has no entry in
    /// the tree to hang from — so it is read from here.
    pub fn root_collection(&self) -> Result<Option<(Document, Vec<Note>)>> {
        let path = self.root.join(COLLECTION_FILE);
        if !self.has(&path) {
            return Ok(None);
        }
        self.parse(&path).map(Some)
    }

    /// A collection's own `__collection.md`, if it wrote one.
    pub fn load_collection(&self, idx: usize) -> Result<Option<(Document, Vec<Note>)>> {
        let path = self.entries[idx].file();
        if !self.has(&path) {
            return Ok(None);
        }
        self.parse(&path).map(Some)
    }

    /// Everything that can be run, in tree order.
    pub fn requests(&self) -> impl Iterator<Item = (usize, &Entry)> {
        self.entries.iter().enumerate().filter(|(_, e)| e.runnable)
    }

    // --- environments --------------------------------------------------------------------

    pub fn env_dir(&self) -> PathBuf {
        self.root.join(ENVS_DIR)
    }

    /// Environment names, global first, then alphabetical. A converted collection brings its
    /// own environments along — a Bruno tree has them in the same import, so they are here
    /// for the same reason they would be after `rq import`.
    pub fn environments(&self) -> Vec<String> {
        let dir = self.env_dir();
        let mut names: Vec<String> = match &self.source {
            Source::Disk => match std::fs::read_dir(&dir) {
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
            },
            Source::Converted { files, .. } => files
                .keys()
                .filter(|p| p.parent() == Some(dir.as_path()))
                .filter(|p| p.extension().is_some_and(|x| x == "md"))
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
                .collect(),
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
        if !self.has(&path) {
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
        self.parse(&path)
    }

    /// The always-on variable layer: `KEY=value` lines from `.env`, `#` comments and an
    /// optional `export ` allowed. Absent is simply empty — a project with one set of
    /// values needs nothing else.
    pub fn dotenv(&self) -> Vec<(String, String)> {
        let Some(text) = self.read(&self.root.join(DOTENV)) else {
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
        if self.is_converted() {
            return None;
        }
        let text = std::fs::read_to_string(self.state_path()).ok()?;
        let json: serde_json::Value = serde_json::from_str(&text).ok()?;
        json.get("environment")?.as_str().map(str::to_string)
    }

    pub fn set_active_env(&self, name: Option<&str>) -> Result<()> {
        // A converted collection has no directory of ours to remember anything in, and
        // scattering an `.rq/` beside somebody's download to record a preference would be
        // taking a liberty. Per-run `-e` still selects one.
        if let Some((from, _)) = self.converted_from() {
            bail!(
                "{} is read directly, so there is nowhere to save an active environment\n  \
                 use `-e <name>` per run, or `rq import {}` to make it a project",
                from.display(),
                from.display()
            );
        }
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
        // Written as one concatenated literal, NOT a `\`-continued one: a continuation
        // carries the source indentation into the string, which put 13 spaces in front of
        // every pattern. git does not strip leading whitespace, so the file ignored nothing
        // and `.env` — the file this project tells people to put secrets in — was staged by
        // the next `git add .`.
        std::fs::write(
            &gitignore,
            concat!(
                "# rq keeps the active environment and other machine-local state here.\n",
                ".rq/\n",
                "\n",
                "# Secrets belong to your machine, not to the collection.\n",
                ".env\n",
            ),
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

    /// `rq init` writes a `.gitignore` whose patterns actually match.
    ///
    /// They did not: the literal was `\`-continued, so every pattern carried the source's
    /// indentation — `             .env` — and git does not strip leading whitespace from a
    /// pattern. The file this project tells people to keep secrets in was left tracked, and
    /// the .gitignore sitting next to it said otherwise. A test on the exact bytes, because
    /// "it looks right" is what shipped it.
    #[test]
    fn a_new_project_ignores_its_secrets_and_its_state() {
        let dir = tempfile::tempdir().unwrap();
        init(dir.path()).unwrap();
        let ignore = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();

        let patterns: Vec<&str> = ignore
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .collect();
        assert_eq!(patterns, vec![".rq/", ".env"], "{ignore:?}");
        for p in &patterns {
            assert_eq!(
                *p,
                p.trim_start(),
                "a leading space makes the pattern match nothing: {p:?}"
            );
        }
    }

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
