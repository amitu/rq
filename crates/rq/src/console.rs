//! The post-run console — a network panel for your terminal.
//!
//! Browsers give you a network tab: every request the page made, click one, see what went
//! out and what came back. A terminal client gives you `--verbose` and asks you to run the
//! request again. This is the first: `rq r me --console` opens the run you just did, arrow
//! between its steps, and drill into the request, the response, the headers, the timing.
//!
//! It reads the [`Run`](crate::run::Run) that already happened. **Nothing is re-sent** — a
//! panel that re-issued your POST to show you what it did would be a bug, not a feature.
//!
//! The drawing is deliberately plain: raw mode and key events from `crossterm`, every frame
//! composed with the same [`ui`](crate::ui) helpers the non-interactive output uses, so the
//! two never drift into different vocabularies.

use std::io::{self, IsTerminal, Write};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use crossterm::{cursor, execute, queue};

use crate::doc::FormField;
use crate::project::Project;
use crate::render;
use crate::run::{Run, RunOptions, Step};
use crate::script::ScriptEngine;
use crate::ui;

/// Which pane is showing for the selected step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    /// The rendered `-- view --`, or the body when there is none.
    View,
    Request,
    Response,
    Headers,
    Timing,
}

impl Pane {
    fn title(&self) -> &'static str {
        match self {
            Pane::View => "view",
            Pane::Request => "request",
            Pane::Response => "response",
            Pane::Headers => "headers",
            Pane::Timing => "timing",
        }
    }

    const ALL: [Pane; 5] = [
        Pane::View,
        Pane::Request,
        Pane::Response,
        Pane::Headers,
        Pane::Timing,
    ];
}

/// What the console needs in order to *go somewhere*: the project the links point into,
/// and the same options and engine the first run used, so page two is the same session as
/// page one.
pub struct Nav<'a> {
    pub project: &'a Project,
    pub opts: &'a RunOptions,
    pub engine: &'a dyn ScriptEngine,
}

/// The console's state: where you've been, which step, which pane, how far scrolled.
pub struct Console<'a> {
    nav: Option<Nav<'a>>,
    /// Every page visited, oldest first — following pushes, backspace steps back.
    history: Vec<Run>,
    cursor: usize,
    step: usize,
    pane: Pane,
    scroll: usize,
    height: usize,
    width: usize,
    /// A one-line report of the last thing that happened (or failed to).
    message: Option<String>,
    /// The form being filled in, when there is one.
    form: Option<FormState>,
    /// The project's requests, when that is what is showing.
    list: Option<ListState>,
    /// Which link `enter` would open. Digits reach the first nine; this reaches all of
    /// them, which matters the moment a page lists more than nine things.
    link_cursor: usize,
}

impl<'a> Console<'a> {
    /// A console with no way to navigate — everything renders, links do nothing.
    pub fn new(run: Run) -> Self {
        let step = run.steps.len().saturating_sub(1);
        Self {
            nav: None,
            history: vec![run],
            cursor: 0,
            step,
            pane: Pane::View,
            scroll: 0,
            height: 24,
            width: 100,
            message: None,
            form: None,
            list: None,
            link_cursor: 0,
        }
    }

    pub fn with_nav(run: Run, nav: Nav<'a>) -> Self {
        Self {
            nav: Some(nav),
            ..Console::new(run)
        }
    }

    /// A console with nothing run yet: the project's request list *is* the home page.
    pub fn browser(nav: Nav<'a>) -> Self {
        let mut console = Self {
            nav: Some(nav),
            history: Vec::new(),
            step: 0,
            ..Console::new(Run {
                steps: Vec::new(),
                view: None,
                raw: String::new(),
                vars: Vec::new(),
                notes: Vec::new(),
                secrets: Vec::new(),
            })
        };
        console.history.clear();
        console.open_list();
        console
    }

    pub fn run(&self) -> &Run {
        &self.history[self.cursor]
    }

    /// The links the current page offers, in reading order.
    pub fn links(&self) -> Vec<crate::render::Link> {
        self.run().links()
    }

    /// Open link `number`, pushing it onto the history. Anything ahead of the cursor is
    /// dropped — you followed a different way, so the old forward path is gone, exactly as
    /// a browser does it.
    pub fn follow(&mut self, number: usize) {
        let Some(nav) = &self.nav else {
            self.message = Some("this console has nothing to navigate with".into());
            return;
        };
        let links = self.run().links();
        let Some(link) = links.iter().find(|l| l.number == number) else {
            self.message = Some(match links.len() {
                0 => "this page has no links".to_string(),
                n => format!("no link [{number}] — this page offers 1..{n}"),
            });
            return;
        };

        // A request that declares a form is asking to be filled in, not fired. Running it
        // here would try to prompt on a terminal this console is already drawing on.
        let (name, vars) = crate::render::parse_target(&link.target);
        let page_vars = self.run().vars.clone();
        match form_of(nav.project, &name, &page_vars) {
            Ok(Some(state)) => {
                self.form = Some(state.seeded(vars));
                self.message = Some(format!("→ {}", link.label.trim()));
                return;
            }
            Ok(None) => {}
            Err(e) => {
                self.message = Some(format!("{name}: {e:#}"));
                return;
            }
        }

        match crate::run::follow(nav.project, link, nav.opts, nav.engine) {
            Ok(next) => {
                self.history.truncate(self.cursor + 1);
                self.history.push(next);
                self.cursor = self.history.len() - 1;
                self.step = self.run().steps.len().saturating_sub(1);
                self.pane = Pane::View;
                self.scroll = 0;
                self.message = Some(format!("→ {}", link.label.trim()));
                self.link_cursor = 0;
            }
            // A link that fails is a page that didn't load, not a reason to lose the one
            // you were reading.
            Err(e) => self.message = Some(format!("{}: {e:#}", link.label.trim())),
        }
    }

    /// Move the link cursor. 0 means "none selected", which is where a fresh page starts.
    pub fn select_link(&mut self, delta: isize) {
        let count = self.run().links().len() as isize;
        if count == 0 {
            self.message = Some("this page has no links".into());
            return;
        }
        let next = self.link_cursor as isize + delta;
        self.link_cursor = if next < 1 {
            count as usize
        } else if next > count {
            1
        } else {
            next as usize
        };
        self.message = self
            .run()
            .links()
            .iter()
            .find(|l| l.number == self.link_cursor)
            .map(|l| format!("[{}] {}", l.number, l.label.trim()));
    }

    pub fn back(&mut self) {
        if self.cursor == 0 {
            self.message = Some("this is the first page".into());
            return;
        }
        self.cursor -= 1;
        self.link_cursor = 0;
        self.step = self.run().steps.len().saturating_sub(1);
        self.pane = Pane::View;
        self.scroll = 0;
        self.message = None;
    }

    fn selected(&self) -> &Step {
        &self.run().steps[self.step]
    }

    /// The lines the current pane shows, already styled.
    pub fn body(&self) -> Vec<String> {
        let step = self.selected();
        let secrets = &self.run().secrets;
        match self.pane {
            Pane::View => {
                let text = match (&self.run().view, self.step + 1 == self.run().steps.len()) {
                    // Only the requested step has a rendered view; a parent shows its body.
                    (Some(view), true) => render::markdown_to_terminal(view),
                    _ => match &step.response {
                        Some(r) => render::default_body(&r.body, r.json().as_ref()),
                        None => "(not sent)".to_string(),
                    },
                };
                lines(&ui::redact(&text, secrets))
            }
            Pane::Request => {
                let mut out = vec![format!("{} {}", ui::bold(&step.method), step.url)];
                for (k, v) in &step.request_headers {
                    out.push(format!("{}: {}", ui::cyan(k), ui::redact(v, secrets)));
                }
                if let Some(body) = &step.request_body {
                    out.push(String::new());
                    out.extend(lines(&ui::redact(body, secrets)));
                }
                out
            }
            Pane::Response => match &step.response {
                Some(r) => lines(&ui::redact(
                    &render::default_body(&r.body, r.json().as_ref()),
                    secrets,
                )),
                None => vec![ui::dim("the request was not sent")],
            },
            Pane::Headers => match &step.response {
                Some(r) => r
                    .headers
                    .iter()
                    .map(|(k, v)| format!("{}: {v}", ui::cyan(k)))
                    .collect(),
                None => vec![ui::dim("the request was not sent")],
            },
            Pane::Timing => match &step.response {
                Some(r) => timing_bars(r, self.width.saturating_sub(28)),
                None => vec![ui::dim("the request was not sent")],
            },
        }
    }

    /// Everything above the body: the step list and the pane tabs.
    pub fn header(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, step) in self.run().steps.iter().enumerate() {
            let marker = if i == self.step {
                ui::cyan(ui::arrow())
            } else {
                " ".to_string()
            };
            let outcome = match &step.response {
                Some(r) => format!(
                    "{}  {}",
                    ui::status(r.status, &format!("{} {}", r.status, r.status_text)),
                    ui::dim(&format!("{}ms", r.elapsed.as_millis()))
                ),
                None => ui::dim("skipped"),
            };
            let name = if i == self.step {
                ui::bold(&step.name)
            } else {
                step.name.clone()
            };
            out.push(format!(
                "{marker} {name}  {} {}  {outcome}",
                ui::dim(&step.method),
                ui::dim(&ui::short_url(&step.url))
            ));
        }
        out.push(String::new());

        let tabs = Pane::ALL
            .iter()
            .map(|p| {
                if *p == self.pane {
                    ui::bold(&ui::underline(p.title()))
                } else {
                    ui::dim(p.title())
                }
            })
            .collect::<Vec<_>>()
            .join("  ");
        out.push(tabs);
        out
    }

    fn footer(&self) -> String {
        let keys = if self.nav.is_some() && !self.run().links().is_empty() {
            "tab/1-9 pick · enter open · backspace back · l list · f form · q quit"
        } else if self.nav.is_some() {
            "l list · f form · ↑/↓ step · ←/→ pane · j/k scroll · q quit"
        } else {
            "↑/↓ step · ←/→ pane · j/k scroll · q quit"
        };
        match &self.message {
            Some(message) => format!("{}  {}", ui::cyan(message), ui::dim(keys)),
            None => ui::dim(keys),
        }
    }

    /// Apply one keypress. Returns `false` when the console should close.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        // A form owns the keyboard while it is open — otherwise typing "q" into a field
        // would quit, which is the sort of thing you only forgive once.
        if self.form.is_some() {
            return self.on_form_key(key);
        }
        if self.list.is_some() {
            return self.on_list_key(key);
        }
        let page = self.height.saturating_sub(8).max(1);
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return false,
            KeyCode::Char('q') | KeyCode::Esc => return false,
            KeyCode::Up => {
                self.step = self.step.saturating_sub(1);
                self.scroll = 0;
            }
            KeyCode::Down => {
                self.step = (self.step + 1).min(self.run().steps.len() - 1);
                self.scroll = 0;
            }
            KeyCode::Left => {
                self.pane = step_pane(self.pane, -1);
                self.scroll = 0;
            }
            KeyCode::Right => {
                self.pane = step_pane(self.pane, 1);
                self.scroll = 0;
            }
            // Tab through the links and open one with enter — the way you move through a
            // page you can't reach with a single digit.
            KeyCode::Tab => self.select_link(1),
            KeyCode::BackTab => self.select_link(-1),
            KeyCode::Enter => {
                let selected = self.link_cursor;
                if selected > 0 {
                    self.follow(selected);
                } else {
                    self.pane = step_pane(self.pane, 1);
                    self.scroll = 0;
                }
            }
            // A digit opens that link — the console's whole reason to know about links.
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                self.follow(c.to_digit(10).unwrap_or(0) as usize)
            }
            KeyCode::Backspace => self.back(),
            KeyCode::Char('j') => self.scroll += 1,
            KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll += page,
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(page),
            KeyCode::Home => self.scroll = 0,
            // The single-letter jumps a network panel trains into your fingers.
            KeyCode::Char('v') => self.pane = Pane::View,
            KeyCode::Char('r') => self.pane = Pane::Request,
            KeyCode::Char('b') => self.pane = Pane::Response,
            KeyCode::Char('h') => self.pane = Pane::Headers,
            KeyCode::Char('t') => self.pane = Pane::Timing,
            KeyCode::Char('f') => self.open_form(),
            KeyCode::Char('l') => self.open_list(),
            _ => {}
        }
        true
    }

    /// One frame, as lines — separated from drawing so it can be tested without a terminal.
    pub fn frame(&self) -> Vec<String> {
        if let Some(list) = &self.list {
            let mut out = list.lines(self.width);
            while out.len() + 2 < self.height {
                out.push(String::new());
            }
            let keys = if self.history.is_empty() {
                "↑/↓ pick · enter open · q quit"
            } else {
                "↑/↓ pick · enter open · esc back to the page · q quit"
            };
            out.push(match &self.message {
                Some(message) => format!("{}  {}", ui::cyan(message), ui::dim(keys)),
                None => ui::dim(keys),
            });
            return out;
        }
        if let Some(form) = &self.form {
            let mut out = form.lines(self.width);
            while out.len() + 2 < self.height {
                out.push(String::new());
            }
            out.push(match &self.message {
                Some(message) => format!(
                    "{}  {}",
                    ui::cyan(message),
                    ui::dim("enter next · ctrl-s submit · esc cancel")
                ),
                None => ui::dim("enter next · ctrl-s submit · esc cancel"),
            });
            return out;
        }
        let mut out = self.header();
        let body = self.body();
        let room = self.height.saturating_sub(out.len() + 2).max(1);
        let scroll = self.scroll.min(body.len().saturating_sub(1));
        out.push(String::new());
        out.extend(body.into_iter().skip(scroll).take(room));
        out.push(self.footer());
        out
    }
}

// ---------------------------------------------------------------------------------------
// Forms
// ---------------------------------------------------------------------------------------

/// A form being filled in. The values start from whatever the field declared, so a form is
/// something you correct rather than something you type from nothing.
pub struct FormState {
    pub rel: String,
    pub title: String,
    pub fields: Vec<FormField>,
    pub values: Vec<String>,
    pub cursor: usize,
    /// Variables the link carried that the form doesn't ask for — `reply_to`, say. They
    /// are part of the submission even though nobody types them.
    pub extra: Vec<(String, String)>,
}

impl FormState {
    pub fn new(rel: String, title: String, fields: Vec<FormField>) -> FormState {
        let values = fields
            .iter()
            .map(|f| f.default.clone().unwrap_or_default())
            .collect();
        FormState {
            rel,
            title,
            fields,
            values,
            cursor: 0,
            extra: Vec::new(),
        }
    }

    /// Seed from what a link supplied: a matching field is prefilled, the rest ride along.
    fn seeded(mut self, vars: Vec<(String, String)>) -> FormState {
        for (key, value) in vars {
            match self.fields.iter().position(|f| f.name == key) {
                Some(i) => self.values[i] = value,
                None => self.extra.push((key, value)),
            }
        }
        self
    }

    /// The field values as variables for the request.
    pub fn as_vars(&self) -> Vec<(String, String)> {
        self.fields
            .iter()
            .zip(&self.values)
            .map(|(f, v)| (f.name.clone(), v.clone()))
            .collect()
    }

    pub fn missing(&self) -> Option<&FormField> {
        self.fields
            .iter()
            .zip(&self.values)
            .find(|(f, v)| f.required && v.trim().is_empty())
            .map(|(f, _)| f)
    }

    pub fn lines(&self, width: usize) -> Vec<String> {
        let mut out = vec![ui::bold(&self.title), String::new()];
        let label_width = self
            .fields
            .iter()
            .map(|f| f.title().chars().count())
            .max()
            .unwrap_or(0)
            .min(24);

        for (i, (field, value)) in self.fields.iter().zip(&self.values).enumerate() {
            let selected = i == self.cursor;
            let marker = if selected {
                ui::cyan(ui::arrow())
            } else {
                " ".to_string()
            };
            let shown = if field.secret {
                "•".repeat(value.chars().count())
            } else {
                value.clone()
            };
            // The caret sits where typing would land.
            let box_width = width.saturating_sub(label_width + 8).clamp(10, 60);
            let visible: String = shown
                .chars()
                .rev()
                .take(box_width.saturating_sub(1))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let caret = if selected { "_" } else { "" };
            let required = if field.required && value.trim().is_empty() {
                ui::red(" *")
            } else {
                String::new()
            };
            out.push(format!(
                "{marker} {:<label_width$}  {}{required}",
                field.title(),
                ui::underline(&format!("{visible}{caret}")),
            ));
            if let Some(help) = &field.help {
                out.push(format!("  {:<label_width$}  {}", "", ui::dim(help)));
            }
        }
        out
    }
}

impl Console<'_> {
    /// Open the form of the request on the current page, prefilled from its declarations.
    pub fn open_form(&mut self) {
        let Some(nav) = &self.nav else {
            self.message = Some("this console has nothing to submit with".into());
            return;
        };
        let rel = self.run().target().rel.clone();
        let page_vars = self.run().vars.clone();
        match form_of(nav.project, &rel, &page_vars) {
            Ok(Some(state)) => {
                self.form = Some(state);
                self.message = None;
            }
            Ok(None) => self.message = Some(format!("`{rel}` has no -- form --")),
            Err(e) => self.message = Some(format!("{rel}: {e:#}")),
        }
    }

    pub fn close_form(&mut self) {
        self.form = None;
        self.message = None;
    }

    /// Run the request with what was typed, and make the result the next page.
    pub fn submit_form(&mut self) {
        let (Some(nav), Some(form)) = (&self.nav, &self.form) else {
            return;
        };
        if let Some(field) = form.missing() {
            self.message = Some(format!("{} is required", field.title()));
            return;
        }

        let mut opts = nav.opts.clone();
        for (key, value) in form.extra.iter().cloned().chain(form.as_vars()) {
            opts.cli_vars.retain(|(k, _)| *k != key);
            opts.cli_vars.push((key, value));
        }
        let rel = form.rel.clone();

        let outcome = nav
            .project
            .resolve(&rel)
            .and_then(|idx| crate::run::run(nav.project, idx, &opts, nav.engine));

        match outcome {
            Ok(next) => {
                self.history.truncate(self.cursor + 1);
                self.history.push(next);
                self.cursor = self.history.len() - 1;
                self.step = self.run().steps.len().saturating_sub(1);
                self.pane = Pane::View;
                self.scroll = 0;
                self.form = None;
                self.link_cursor = 0;
                self.message = Some(format!("submitted {rel}"));
            }
            // The form stays open with what you typed still in it: losing a filled-in form
            // to a failed request would be its own small tragedy.
            Err(e) => self.message = Some(format!("{rel}: {e:#}")),
        }
    }

    /// Keys while a form is open. Returns false when the console should close.
    fn on_form_key(&mut self, key: KeyEvent) -> bool {
        let Some(form) = &mut self.form else {
            return true;
        };
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return false,
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.submit_form()
            }
            KeyCode::Esc => self.close_form(),
            KeyCode::Up | KeyCode::BackTab => form.cursor = form.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => {
                form.cursor = (form.cursor + 1).min(form.fields.len().saturating_sub(1))
            }
            KeyCode::Enter => {
                // Enter moves on, and submits from the last field — the shape a form has
                // in every other program.
                if form.cursor + 1 < form.fields.len() {
                    form.cursor += 1;
                } else {
                    self.submit_form();
                }
            }
            KeyCode::Backspace => {
                if let Some(value) = form.values.get_mut(form.cursor) {
                    value.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(value) = form.values.get_mut(form.cursor) {
                    value.push(c);
                }
            }
            _ => {}
        }
        true
    }
}

/// The form a request declares, if it declares one.
/// Resolve `{{templates}}` in a form's defaults against the page you opened it from, so
/// `default: '{{me}}'` shows *you* rather than showing you its own source code.
///
/// Masked secrets are left as written: substituting `***` into a field would be worse than
/// leaving the template there, because you would submit the mask.
fn resolve_defaults(fields: &mut [FormField], page_vars: &[(String, String, String)]) {
    let mut vars = crate::vars::Vars::new();
    vars.layer(
        "page",
        page_vars
            .iter()
            .filter(|(_, value, _)| value != "***")
            .map(|(key, value, _)| (key.clone(), value.clone())),
    );
    for field in fields {
        if let Some(default) = &field.default {
            field.default = Some(crate::vars::substitute(default, &vars).text);
        }
    }
}

fn form_of(
    project: &Project,
    rel: &str,
    page_vars: &[(String, String, String)],
) -> Result<Option<FormState>> {
    let Ok(idx) = project.resolve(rel) else {
        return Ok(None);
    };
    let (doc, _) = project.load(idx)?;
    let mut fields = doc.form().map_err(|e| anyhow::anyhow!("{e}"))?;
    if fields.is_empty() {
        return Ok(None);
    }
    resolve_defaults(&mut fields, page_vars);
    let title = doc.summary().unwrap_or_else(|| rel.to_string());
    Ok(Some(FormState::new(rel.to_string(), title, fields)))
}

// ---------------------------------------------------------------------------------------
// The request list
// ---------------------------------------------------------------------------------------

/// Every request in the project, as a page you can open one from. This is what bare `rq`
/// shows: the project is the home page, and running something is picking it off a list
/// rather than remembering its name.
pub struct ListState {
    pub rows: Vec<ListRow>,
    pub cursor: usize,
}

pub struct ListRow {
    pub rel: String,
    pub name: String,
    pub method: String,
    pub url: String,
    /// What the request says it is. A templated URL tells you far less.
    pub summary: Option<String>,
    pub depth: usize,
    /// A collection's landing page reads differently from a request under it.
    pub is_index: bool,
}

impl ListState {
    fn of(project: &Project) -> ListState {
        let rows = project
            .requests()
            .map(|(idx, entry)| {
                let (method, url, summary) = project
                    .load(idx)
                    .map(|(doc, _)| {
                        (
                            doc.front.method.clone().unwrap_or_else(|| "GET".into()),
                            doc.front.url.clone().unwrap_or_default(),
                            doc.summary(),
                        )
                    })
                    .unwrap_or_else(|_| ("?".into(), String::new(), None));
                ListRow {
                    depth: entry.rel.matches('/').count(),
                    is_index: entry.kind == crate::project::Kind::Collection,
                    rel: entry.rel.clone(),
                    name: entry.name.clone(),
                    method,
                    url,
                    summary,
                }
            })
            .collect();
        ListState { rows, cursor: 0 }
    }

    fn lines(&self, width: usize) -> Vec<String> {
        if self.rows.is_empty() {
            return vec![
                ui::bold("No requests yet"),
                String::new(),
                ui::dim("  rq curl --save-as <name> '<curl …>'  saves your first"),
            ];
        }
        // The path, not the leaf: `mine/me` is both unambiguous and exactly what you would
        // type. Indenting leaves under a collection header that isn't in the list only looks
        // like a tree.
        let name_width = self
            .rows
            .iter()
            .map(|r| r.rel.chars().count())
            .max()
            .unwrap_or(0)
            .min(36);

        let mut out = vec![
            ui::bold(&format!("{} requests", self.rows.len())),
            String::new(),
        ];
        for (i, row) in self.rows.iter().enumerate() {
            let marker = if i == self.cursor {
                ui::cyan(ui::arrow())
            } else {
                " ".to_string()
            };
            let shown = format!("{}{}", row.rel, if row.is_index { "/" } else { "" });
            let label = if i == self.cursor {
                ui::bold(&shown)
            } else {
                shown.clone()
            };
            let pad = name_width.saturating_sub(shown.chars().count());
            let room = width.saturating_sub(name_width + 14);
            // What it is, if it says; otherwise where it goes.
            let about = row.summary.clone().unwrap_or_else(|| row.url.clone());
            out.push(format!(
                "{marker} {label}{} {:<6} {}",
                " ".repeat(pad),
                ui::dim(&row.method),
                ui::dim(&truncate(&about, room.max(20)))
            ));
        }
        out
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!(
        "{}…",
        s.chars().take(max.saturating_sub(1)).collect::<String>()
    )
}

impl Console<'_> {
    /// Show the project's requests.
    pub fn open_list(&mut self) {
        let Some(nav) = &self.nav else {
            self.message = Some("this console has no project to list".into());
            return;
        };
        let mut list = ListState::of(nav.project);
        // Land on the request you are looking at, so `l` answers "where am I" as well as
        // "what else is there".
        if let Some(current) = self
            .history
            .get(self.cursor)
            .map(|r| r.target().rel.clone())
        {
            if let Some(i) = list.rows.iter().position(|r| r.rel == current) {
                list.cursor = i;
            }
        }
        self.list = Some(list);
        self.message = None;
    }

    pub fn close_list(&mut self) {
        // With nothing run yet the list *is* the console — there is nowhere to close to.
        if !self.history.is_empty() {
            self.list = None;
            self.message = None;
        }
    }

    /// Run whatever the cursor is on, and make it the current page.
    pub fn open_selected(&mut self) {
        let (Some(nav), Some(list)) = (&self.nav, &self.list) else {
            return;
        };
        let Some(row) = list.rows.get(list.cursor) else {
            return;
        };
        let rel = row.rel.clone();

        // A request that declares a form asks to be filled in, not fired — the same rule
        // links follow.
        let page_vars = self
            .history
            .last()
            .map(|r| r.vars.clone())
            .unwrap_or_default();
        match form_of(nav.project, &rel, &page_vars) {
            Ok(Some(state)) => {
                self.list = None;
                self.form = Some(state);
                self.message = Some(format!("→ {rel}"));
                return;
            }
            Ok(None) => {}
            Err(e) => {
                self.message = Some(format!("{rel}: {e:#}"));
                return;
            }
        }

        let outcome = nav
            .project
            .resolve(&rel)
            .and_then(|idx| crate::run::run(nav.project, idx, nav.opts, nav.engine));
        match outcome {
            Ok(next) => {
                self.history.truncate(self.cursor_end());
                self.history.push(next);
                self.cursor = self.history.len() - 1;
                self.step = self.run().steps.len().saturating_sub(1);
                self.pane = Pane::View;
                self.scroll = 0;
                self.link_cursor = 0;
                self.list = None;
                self.message = Some(format!("→ {rel}"));
            }
            Err(e) => self.message = Some(format!("{e:#}")),
        }
    }

    fn cursor_end(&self) -> usize {
        if self.history.is_empty() {
            0
        } else {
            self.cursor + 1
        }
    }

    /// Keys while the list is open.
    fn on_list_key(&mut self, key: KeyEvent) -> bool {
        let Some(list) = &mut self.list else {
            return true;
        };
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return false,
            KeyCode::Char('q') => return false,
            KeyCode::Esc | KeyCode::Char('l') => self.close_list(),
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                list.cursor = list.cursor.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                list.cursor = (list.cursor + 1).min(list.rows.len().saturating_sub(1))
            }
            KeyCode::Home => list.cursor = 0,
            KeyCode::End => list.cursor = list.rows.len().saturating_sub(1),
            KeyCode::Enter => self.open_selected(),
            _ => {}
        }
        true
    }
}

fn step_pane(pane: Pane, delta: isize) -> Pane {
    let i = Pane::ALL.iter().position(|p| *p == pane).unwrap_or(0) as isize;
    let n = Pane::ALL.len() as isize;
    Pane::ALL[((i + delta).rem_euclid(n)) as usize]
}

/// A proportional bar per phase — the shape of the wait, not just its size.
fn timing_bars(response: &crate::http::Response, width: usize) -> Vec<String> {
    let phases = response.timings.phases();
    let total = response.timings.total.as_secs_f64().max(f64::MIN_POSITIVE);
    let width = width.clamp(10, 60);

    let mut out: Vec<String> = phases
        .iter()
        .map(|(name, d)| {
            let share = d.as_secs_f64() / total;
            let filled = ((share * width as f64).round() as usize)
                .clamp(if d.is_zero() { 0 } else { 1 }, width);
            format!(
                "{:<10} {:>6}ms  {}",
                name,
                d.as_millis(),
                ui::cyan(&"█".repeat(filled))
            )
        })
        .collect();
    out.push(String::new());
    out.push(format!(
        "{:<10} {:>6}ms",
        ui::bold("total"),
        response.timings.total.as_millis()
    ));
    out.push(String::new());
    out.push(ui::dim(
        "the TLS handshake is inside `waiting`: ureq completes it on first use, not on connect",
    ));
    out
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

/// Is there a terminal to draw on? A console that opened while piping would hang a script.
pub fn available() -> bool {
    io::stdout().is_terminal() && io::stdin().is_terminal()
}

/// Fill in a form on its own, outside the console: what `rq r <request>` does when the
/// request declares one.
///
/// Returns the values, or `None` if the person cancelled. The alt screen is entered and
/// left around the form, so what you see afterwards is the ordinary run output — filling a
/// form should not change where your results appear.
pub fn fill_form(
    title: &str,
    fields: Vec<FormField>,
    prefill: &[(String, String)],
) -> Result<Option<Vec<(String, String)>>> {
    if fields.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut state = FormState::new(String::new(), title.to_string(), fields);
    for (key, value) in prefill {
        if let Some(i) = state.fields.iter().position(|f| f.name == *key) {
            state.values[i] = value.clone();
        }
    }

    let mut out = io::stdout();
    enable_raw_mode()?;
    execute!(out, crossterm::terminal::EnterAlternateScreen, cursor::Hide)?;
    let outcome = fill_loop(&mut state, &mut out);
    execute!(out, crossterm::terminal::LeaveAlternateScreen, cursor::Show)?;
    disable_raw_mode()?;
    outcome
}

fn fill_loop(state: &mut FormState, out: &mut io::Stdout) -> Result<Option<Vec<(String, String)>>> {
    let mut message: Option<String> = None;
    loop {
        let (width, height) = crossterm::terminal::size().unwrap_or((100, 24));
        let (width, height) = ((width as usize).max(40), (height as usize).max(10));

        queue!(out, cursor::MoveTo(0, 0), Clear(ClearType::All))?;
        let mut lines = state.lines(width);
        while lines.len() + 2 < height {
            lines.push(String::new());
        }
        lines.push(match &message {
            Some(m) => format!(
                "{}  {}",
                ui::cyan(m),
                ui::dim("enter next · ctrl-s send · esc cancel")
            ),
            None => ui::dim("enter next · ctrl-s send · esc cancel"),
        });
        for line in lines {
            queue!(out, cursor::MoveToColumn(0))?;
            write!(out, "{line}\r\n")?;
        }
        out.flush()?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != event::KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(None),
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                match state.missing() {
                    Some(field) => message = Some(format!("{} is required", field.title())),
                    None => return Ok(Some(state.as_vars())),
                }
            }
            KeyCode::Enter => {
                if state.cursor + 1 < state.fields.len() {
                    state.cursor += 1;
                } else {
                    match state.missing() {
                        Some(field) => message = Some(format!("{} is required", field.title())),
                        None => return Ok(Some(state.as_vars())),
                    }
                }
            }
            KeyCode::Up | KeyCode::BackTab => state.cursor = state.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => {
                state.cursor = (state.cursor + 1).min(state.fields.len() - 1)
            }
            KeyCode::Backspace => {
                if let Some(value) = state.values.get_mut(state.cursor) {
                    value.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(value) = state.values.get_mut(state.cursor) {
                    value.push(c);
                }
            }
            _ => {}
        }
    }
}

/// Open the console over a finished run and block until the user quits. With a [`Nav`],
/// its links are live.
pub fn open(run: Run, nav: Option<Nav<'_>>) -> Result<()> {
    if run.steps.is_empty() {
        return Ok(());
    }
    let mut console = match nav {
        Some(nav) => Console::with_nav(run, nav),
        None => Console::new(run),
    };
    draw(&mut console)
}

/// Open the console on the project itself, with nothing run yet.
pub fn browse(nav: Nav<'_>) -> Result<()> {
    let mut console = Console::browser(nav);
    draw(&mut console)
}

fn draw(console: &mut Console<'_>) -> Result<()> {
    let mut out = io::stdout();

    enable_raw_mode()?;
    execute!(out, crossterm::terminal::EnterAlternateScreen, cursor::Hide)?;

    let result = event_loop(console, &mut out);

    execute!(out, crossterm::terminal::LeaveAlternateScreen, cursor::Show)?;
    disable_raw_mode()?;
    result
}

fn event_loop(console: &mut Console<'_>, out: &mut io::Stdout) -> Result<()> {
    loop {
        // Some terminals (and some pty wrappers) report a zero size. Taking that at face
        // value collapses the body to a single line, so floor it at something usable.
        let (width, height) = crossterm::terminal::size().unwrap_or((100, 24));
        console.width = (width as usize).max(40);
        console.height = (height as usize).max(10);

        queue!(out, cursor::MoveTo(0, 0), Clear(ClearType::All))?;
        for line in console.frame() {
            queue!(out, cursor::MoveToColumn(0))?;
            write!(out, "{line}\r\n")?;
        }
        out.flush()?;

        // Key *release* events arrive too on some terminals; acting on both would move two
        // steps per press.
        if let Event::Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Press && !console.on_key(key) {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Response, Timings};
    use crate::run::Step;
    use std::time::Duration;

    fn response(status: u16, body: &str) -> Response {
        Response {
            status,
            status_text: "OK".into(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.to_string(),
            bytes: body.len(),
            elapsed: Duration::from_millis(120),
            timings: Timings {
                dns: Duration::from_millis(20),
                tcp: Duration::from_millis(10),
                waiting: Duration::from_millis(80),
                download: Duration::from_millis(10),
                total: Duration::from_millis(120),
            },
            final_url: "https://api.test/x".into(),
        }
    }

    fn step(name: &str, sent: bool) -> Step {
        Step {
            rel: name.into(),
            name: name.into(),
            method: "GET".into(),
            url: format!("https://api.test/{name}"),
            request_headers: vec![("Authorization".into(), "Bearer s3cret-token".into())],
            body: None,
            request_body: Some("{\"a\": 1}".into()),
            response: sent.then(|| response(200, "{\"id\":7}")),
            captured: Vec::new(),
            tests: Vec::new(),
            logs: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn run_of(steps: Vec<Step>) -> Run {
        Run {
            steps,
            view: Some("# hello".into()),
            raw: "{}".into(),
            vars: Vec::new(),
            notes: Vec::new(),
            secrets: vec!["s3cret-token".into()],
        }
    }

    #[test]
    fn it_opens_on_the_requested_step() {
        let run = run_of(vec![step("login", true), step("me", true)]);
        let console = Console::new(run);
        assert_eq!(console.step, 1, "the step you asked for, not its parent");
        assert_eq!(console.pane, Pane::View);
    }

    #[test]
    fn arrows_move_between_steps_and_panes() {
        let run = run_of(vec![step("login", true), step("me", true)]);
        let mut console = Console::new(run);

        console.on_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(console.step, 0);
        console.on_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(console.step, 0, "stops at the top rather than wrapping");

        console.on_key(KeyEvent::from(KeyCode::Right));
        assert_eq!(console.pane, Pane::Request);
        console.on_key(KeyEvent::from(KeyCode::Left));
        assert_eq!(console.pane, Pane::View);
        console.on_key(KeyEvent::from(KeyCode::Left));
        assert_eq!(console.pane, Pane::Timing, "panes wrap");
    }

    #[test]
    fn q_and_ctrl_c_close_it() {
        let run = run_of(vec![step("me", true)]);
        let mut console = Console::new(run);
        assert!(!console.on_key(KeyEvent::from(KeyCode::Char('q'))));
        assert!(!console.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(console.on_key(KeyEvent::from(KeyCode::Char('j'))));
    }

    #[test]
    fn secrets_are_masked_in_every_pane() {
        let run = run_of(vec![step("me", true)]);
        let mut console = Console::new(run);
        for pane in Pane::ALL {
            console.pane = pane;
            let text = console.body().join("\n");
            assert!(
                !text.contains("s3cret-token"),
                "{} pane leaked a secret:\n{text}",
                pane.title()
            );
        }
    }

    #[test]
    fn a_skipped_step_says_so_instead_of_showing_a_stale_body() {
        let run = run_of(vec![step("me", false)]);
        let mut console = Console::new(run);
        for pane in [Pane::Response, Pane::Headers, Pane::Timing] {
            console.pane = pane;
            assert!(
                console.body().join("\n").contains("not sent"),
                "{} pane",
                pane.title()
            );
        }
    }

    #[test]
    fn the_timing_pane_shows_every_measured_phase() {
        let run = run_of(vec![step("me", true)]);
        let mut console = Console::new(run);
        console.pane = Pane::Timing;
        let text = console.body().join("\n");
        for phase in ["DNS", "TCP", "waiting", "download", "total"] {
            assert!(text.contains(phase), "{phase} missing from:\n{text}");
        }
        // …and is honest about where the handshake went.
        assert!(text.contains("TLS handshake is inside `waiting`"), "{text}");
    }

    #[test]
    fn a_frame_fits_the_terminal_it_was_given() {
        let mut run = run_of(vec![step("me", true)]);
        run.view = Some("# title\n\nline\n".repeat(50));
        let mut console = Console::new(run);
        console.height = 20;
        assert!(
            console.frame().len() <= 20,
            "frame was {} lines",
            console.frame().len()
        );
    }

    #[test]
    fn scrolling_moves_through_a_long_body() {
        let mut run = run_of(vec![step("me", true)]);
        run.view = None;
        run.steps[0].response = Some(response(
            200,
            &(1..=100).map(|i| format!("line {i}\n")).collect::<String>(),
        ));
        let mut console = Console::new(run);
        console.pane = Pane::Response;
        console.height = 20;
        let first = console.frame();
        console.on_key(KeyEvent::from(KeyCode::PageDown));
        assert_ne!(first, console.frame(), "page down changed nothing");
    }
}
