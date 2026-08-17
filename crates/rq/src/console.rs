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

use crate::render;
use crate::run::{Run, Step};
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

/// The console's state: which step, which pane, how far scrolled.
pub struct Console<'a> {
    run: &'a Run,
    step: usize,
    pane: Pane,
    scroll: usize,
    height: usize,
    width: usize,
}

impl<'a> Console<'a> {
    pub fn new(run: &'a Run) -> Self {
        Self {
            run,
            step: run.steps.len().saturating_sub(1),
            pane: Pane::View,
            scroll: 0,
            height: 24,
            width: 100,
        }
    }

    fn selected(&self) -> &Step {
        &self.run.steps[self.step]
    }

    /// The lines the current pane shows, already styled.
    pub fn body(&self) -> Vec<String> {
        let step = self.selected();
        let secrets = &self.run.secrets;
        match self.pane {
            Pane::View => {
                let text = match (&self.run.view, self.step + 1 == self.run.steps.len()) {
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
        for (i, step) in self.run.steps.iter().enumerate() {
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
        ui::dim("↑/↓ step · ←/→ pane · j/k scroll · q quit")
    }

    /// Apply one keypress. Returns `false` when the console should close.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        let page = self.height.saturating_sub(8).max(1);
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return false,
            KeyCode::Char('q') | KeyCode::Esc => return false,
            KeyCode::Up => {
                self.step = self.step.saturating_sub(1);
                self.scroll = 0;
            }
            KeyCode::Down => {
                self.step = (self.step + 1).min(self.run.steps.len() - 1);
                self.scroll = 0;
            }
            KeyCode::Left | KeyCode::BackTab => {
                self.pane = step_pane(self.pane, -1);
                self.scroll = 0;
            }
            KeyCode::Right | KeyCode::Tab | KeyCode::Enter => {
                self.pane = step_pane(self.pane, 1);
                self.scroll = 0;
            }
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
            _ => {}
        }
        true
    }

    /// One frame, as lines — separated from drawing so it can be tested without a terminal.
    pub fn frame(&self) -> Vec<String> {
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

/// Open the console over a finished run and block until the user quits.
pub fn open(run: &Run) -> Result<()> {
    if run.steps.is_empty() {
        return Ok(());
    }
    let mut console = Console::new(run);
    let mut out = io::stdout();

    enable_raw_mode()?;
    execute!(out, crossterm::terminal::EnterAlternateScreen, cursor::Hide)?;

    let result = event_loop(&mut console, &mut out);

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
        let console = Console::new(&run);
        assert_eq!(console.step, 1, "the step you asked for, not its parent");
        assert_eq!(console.pane, Pane::View);
    }

    #[test]
    fn arrows_move_between_steps_and_panes() {
        let run = run_of(vec![step("login", true), step("me", true)]);
        let mut console = Console::new(&run);

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
        let mut console = Console::new(&run);
        assert!(!console.on_key(KeyEvent::from(KeyCode::Char('q'))));
        assert!(!console.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(console.on_key(KeyEvent::from(KeyCode::Char('j'))));
    }

    #[test]
    fn secrets_are_masked_in_every_pane() {
        let run = run_of(vec![step("me", true)]);
        let mut console = Console::new(&run);
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
        let mut console = Console::new(&run);
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
        let mut console = Console::new(&run);
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
        let mut console = Console::new(&run);
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
        let mut console = Console::new(&run);
        console.pane = Pane::Response;
        console.height = 20;
        let first = console.frame();
        console.on_key(KeyEvent::from(KeyCode::PageDown));
        assert_ne!(first, console.frame(), "page down changed nothing");
    }
}
