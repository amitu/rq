//! The network log: every request rq sent, in a file you can read.
//!
//! The console is a network panel over **one run**, because that is all a process knows. With
//! `--log` it is a panel over everything you have sent — the last week of requests, arrowable,
//! each with its request, response, headers and timings — which is what a browser's network
//! tab actually gives you.
//!
//! **JSONL, not JSON.** One object per line, appended:
//!
//! * appending is a write, not a read-modify-write. A JSON array would have to be re-serialised
//!   on every run, so two `rq` processes finishing at once would lose one of them.
//! * a torn write costs one line. A truncated JSON array is an unparseable file — the whole
//!   history gone because a laptop slept mid-write.
//! * it is already a unix file: `tail -f .rq/log.jsonl`, `jq -c 'select(.status >= 500)'`,
//!   `wc -l`, and `tail -n 500` is how you trim it. That is the same argument `--cookies` made:
//!   the path is the interface, so the format should be one the tools you have already speak.
//!
//! **Secrets are redacted on the way in**, with the same list the terminal redacts with. A log
//! is a file that outlives the run; writing an `Authorization:` header into it verbatim would
//! be storing a credential the run only borrowed.

use std::io::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::http::Response;
use crate::run::Step;
use crate::ui;

/// Bodies are logged, because a network panel that cannot show you the body is a list of URLs.
/// They are also capped, because one 40 MB download should not cost you the rest of the log.
const MAX_BODY: usize = 64 * 1024;

/// One request, as it goes into the file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    /// Seconds since the epoch — enough to order and group, no date library.
    pub at: u64,
    /// Which invocation this belonged to, so a run's steps stay together.
    pub run: String,
    pub name: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub request_headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default)]
    pub status_text: String,
    #[serde(default)]
    pub response_headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub bytes: usize,
    #[serde(default)]
    pub ms: u64,
}

fn clip(text: &str) -> String {
    if text.len() <= MAX_BODY {
        return text.to_string();
    }
    let mut end = MAX_BODY;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n… {} more bytes, not logged",
        &text[..end],
        text.len() - end
    )
}

impl Entry {
    /// Build a line from a step. `secrets` is the run's own list, so what the terminal hides
    /// the file hides too.
    pub fn of(step: &Step, run: &str, at: u64, secrets: &[String]) -> Entry {
        let hide = |s: &str| ui::redact(s, secrets);
        let resp: Option<&Response> = step.response.as_ref();
        Entry {
            at,
            run: run.to_string(),
            name: step.rel.clone(),
            method: step.method.clone(),
            url: hide(&step.url),
            request_headers: step
                .request_headers
                .iter()
                .map(|(k, v)| (k.clone(), hide(v)))
                .collect(),
            request_body: step.request_body.as_deref().map(|b| clip(&hide(b))),
            status: resp.map(|r| r.status),
            status_text: resp.map(|r| r.status_text.clone()).unwrap_or_default(),
            response_headers: resp
                .map(|r| {
                    r.headers
                        .iter()
                        .map(|(k, v)| (k.clone(), hide(v)))
                        .collect()
                })
                .unwrap_or_default(),
            body: resp.map(|r| clip(&hide(&r.body))).unwrap_or_default(),
            bytes: resp.map(|r| r.bytes).unwrap_or(0),
            ms: resp.map(|r| r.elapsed.as_millis() as u64).unwrap_or(0),
        }
    }
}

/// Append these entries. Opened once, in append mode, and each line flushed as written.
pub fn append(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for entry in entries {
        let line = serde_json::to_string(entry).unwrap_or_default();
        writeln!(file, "{line}")?;
    }
    Ok(())
}

/// Read a log back, oldest first.
///
/// A line that does not parse is **skipped, and counted** — a half-written last line from a
/// killed process should cost you that request, not the file. The count is reported so a log
/// that is quietly rotting says so.
pub fn read(path: &Path) -> (Vec<Entry>, usize) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (Vec::new(), 0);
    };
    let mut entries = Vec::new();
    let mut skipped = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<Entry>(line) {
            Ok(e) => entries.push(e),
            Err(_) => skipped += 1,
        }
    }
    (entries, skipped)
}

/// The most recent `n`, oldest first — what the console shows behind the current run.
pub fn tail(path: &Path, n: usize) -> (Vec<Entry>, usize) {
    let (mut entries, skipped) = read(path);
    if entries.len() > n {
        entries.drain(..entries.len() - n);
    }
    (entries, skipped)
}

/// Turn a logged entry back into the `Step` the console draws.
///
/// It is the same struct the live run produces, so every pane — request, response, headers,
/// timing — works on a request from last Tuesday without knowing it is one. What cannot come
/// back is what was never written: a step's captures, tests and script logs belong to the run
/// that produced them, and inventing them here would be putting words in its mouth.
impl Entry {
    pub fn into_step(self) -> crate::run::Step {
        let response = self.status.map(|status| crate::http::Response {
            status,
            status_text: self.status_text,
            headers: self.response_headers,
            body: self.body,
            bytes: self.bytes,
            elapsed: std::time::Duration::from_millis(self.ms),
            timings: crate::http::Timings::default(),
            final_url: self.url.clone(),
        });
        crate::run::Step {
            rel: self.name.clone(),
            name: self.name,
            method: self.method,
            url: self.url,
            request_headers: self.request_headers,
            body: None,
            request_body: self.request_body,
            response,
            captured: Vec::new(),
            tests: Vec::new(),
            logs: Vec::new(),
            notes: Vec::new(),
        }
    }
}
