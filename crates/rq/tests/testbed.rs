//! The example project, run against the example backend, by the real binary.
//!
//! Every other suite here uses a stub that answers with canned strings. This one boots
//! `rq-testbed` and drives `examples/testbed/` — the project a person would run by hand —
//! so the docs, the fixture and the client are checked against each other rather than
//! against my memory of what they said.

use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_rq");

fn project() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/testbed")
        .canonicalize()
        .expect("examples/testbed is missing")
}

struct Fixture {
    server: rq_testbed::Server,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture {
            server: rq_testbed::Server::start(0).expect("start the testbed"),
        }
    }

    fn rq(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .arg("--project")
            .arg(project())
            .args(["--color=never", "--var"])
            .arg(format!("host={}", self.server.base_url))
            .env_remove("RQ_PROJECT")
            .env("NO_COLOR", "1")
            // The upload request sends a file by relative path.
            .current_dir(project())
            .output()
            .expect("running rq")
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn run(f: &Fixture, name: &str) -> String {
    let out = f.rq(&["r", name, "-e", "local"]);
    assert!(
        out.status.success(),
        "rq r {name} failed:\n{}\n{}",
        stdout(&out),
        stderr(&out)
    );
    stdout(&out)
}

#[test]
fn the_whole_example_project_runs_against_the_example_backend() {
    let f = Fixture::new();

    // Chaining by capture: login's token reaches me, and the view renders it.
    let me = run(&f, "me");
    assert!(me.contains("Amit Upadhyay"), "{me}");
    assert!(me.contains("via bearer"), "{me}");
    assert!(
        me.contains("joined 2024-08-12"),
        "the date filter ran: {me}"
    );

    // Chaining by cookie: same endpoint, no Authorization header at all.
    let cookie = run(&f, "me-by-cookie");
    assert!(
        cookie.contains("Authenticated via cookie"),
        "the jar carried the session: {cookie}"
    );

    // The rendered table.
    let issues = run(&f, "issues");
    assert!(issues.contains("5 open issues"), "{issues}");
    assert!(issues.contains("@kevinhq"), "{issues}");
    assert!(
        !issues.contains('|'),
        "the table was not rendered:\n{issues}"
    );

    // Auth built from `auth:`, both placements.
    assert!(run(&f, "basic-auth").contains("\"authenticated\": true"));
    assert!(run(&f, "api-key").contains("\"via\": \"query\""));

    // Bodies.
    let echo = run(&f, "echo");
    assert!(echo.contains("POST /echo"), "{echo}");
    assert!(echo.contains("X-Trace as received: abc123"), "{echo}");
    let upload = run(&f, "upload");
    assert!(upload.contains("2 part(s)"), "{upload}");
    assert!(upload.contains("sample.txt"), "{upload}");

    // Redirects are followed by default.
    assert!(run(&f, "redirected").contains("\"redirected\": true"));
}

#[test]
fn the_timing_breakdown_is_real_against_a_server_that_actually_waits() {
    let f = Fixture::new();
    let out = f.rq(&["r", "slow", "-e", "local", "--show", "timing"]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}{}", stderr(&out));
    assert!(text.contains("waiting"), "{text}");

    // The server slept 300ms, so the wait has to dominate — a timing pane that reported
    // otherwise would be measuring itself.
    let waiting: u64 = text
        .split("waiting ")
        .nth(1)
        .and_then(|s| s.split("ms").next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    assert!(waiting >= 250, "waiting was {waiting}ms in:\n{text}");
}

#[test]
fn a_timeout_is_reported_as_a_timeout() {
    let f = Fixture::new();
    // 6s of sleep against the request's own 5s timeout.
    let out = f.rq(&["r", "slow", "-e", "local", "--var", "ms=6000"]);
    assert!(!out.status.success());
    let text = stderr(&out);
    assert!(
        text.contains("timeout") || text.contains("timed out"),
        "{text}"
    );
}

#[test]
fn secrets_from_the_environment_never_reach_the_screen() {
    let f = Fixture::new();
    let out = f.rq(&["r", "basic-auth", "-e", "local", "--show", "request"]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}{}", stderr(&out));
    // `password` is declared secret in environments/local.md, and Basic auth is built from
    // it — the header goes out, the value does not come back to the terminal.
    assert!(!text.contains("hunter2"), "{text}");
    assert!(text.contains("Authorization"), "{text}");
}

// --- links: a view is a page, not a report ------------------------------------------------

#[test]
fn a_view_offers_numbered_links_and_follow_walks_them() {
    let f = Fixture::new();

    // `issues` links each row to `issue?number=N`; following one lands on that issue.
    let listed = run(&f, "issues");
    assert!(listed.contains("[1]"), "no links were numbered:\n{listed}");

    let out = f.rq(&["r", "issues", "-e", "local", "--follow", "1"]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}{}", stderr(&out));
    assert!(text.contains("follow →"), "{text}");
    // The second page is a different request, run with the link's own variable.
    assert!(text.contains("Issue 1287"), "{text}");
}

#[test]
fn following_a_link_that_isnt_there_says_what_is() {
    let f = Fixture::new();
    let out = f.rq(&["r", "issues", "-e", "local", "--follow", "99"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("no link [99]"), "{}", stderr(&out));
}

// --- the project-wide collection ----------------------------------------------------------

#[test]
fn the_root_collection_reaches_every_request() {
    let f = Fixture::new();
    // examples/testbed/apis/__collection.md sets these, and `echo` mirrors what arrived.
    let echoed = run(&f, "echo");
    assert!(
        echoed.contains("rq-testbed-example"),
        "the root collection's User-Agent never went out:\n{echoed}"
    );
}

// --- the console navigates -----------------------------------------------------------------

/// The demo path: open a run, press a digit, land on the next page, backspace to return.
/// Driven through the library rather than a terminal, so the navigation is tested even
/// though the drawing needs a tty.
#[test]
fn the_console_follows_links_and_goes_back() {
    let f = Fixture::new();
    let project = rq::project::Project::open(project()).unwrap();
    let opts = rq::run::RunOptions {
        cli_vars: vec![("host".into(), f.server.base_url.clone())],
        environment: Some("local".into()),
        ..rq::run::RunOptions::default()
    };
    let engine = rq::script::NoEngine;
    let target = project.resolve("issues").unwrap();
    let first = rq::run::run(&project, target, &opts, &engine).unwrap();

    let mut console = rq::console::Console::with_nav(
        first,
        rq::console::Nav {
            project: &project,
            opts: &opts,
            engine: &engine,
        },
    );

    let links = console.links();
    assert!(links.len() >= 5, "the issues table should link every row");
    assert!(
        links[0].target.starts_with("issue?number="),
        "{:?}",
        links[0]
    );

    // Open the first row — through the key, not the method, because the key is what a
    // person presses.
    console.on_key(crossterm::event::KeyEvent::from(
        crossterm::event::KeyCode::Char('1'),
    ));
    assert_eq!(console.run().target().rel, "issue");
    let page = console.body().join("\n");
    assert!(page.contains("Issue 1287"), "{page}");

    // …and back to where we were.
    console.on_key(crossterm::event::KeyEvent::from(
        crossterm::event::KeyCode::Backspace,
    ));
    assert_eq!(console.run().target().rel, "issues");
    assert!(console.body().join("\n").contains("open issues"));

    // Back from the first page says so rather than doing something surprising.
    console.back();
    assert_eq!(console.run().target().rel, "issues");
}

#[test]
fn a_link_that_does_not_exist_leaves_the_page_you_are_on() {
    let f = Fixture::new();
    let project = rq::project::Project::open(project()).unwrap();
    let opts = rq::run::RunOptions {
        cli_vars: vec![("host".into(), f.server.base_url.clone())],
        environment: Some("local".into()),
        ..rq::run::RunOptions::default()
    };
    let engine = rq::script::NoEngine;
    let target = project.resolve("issues").unwrap();
    let run = rq::run::run(&project, target, &opts, &engine).unwrap();

    let mut console = rq::console::Console::with_nav(
        run,
        rq::console::Nav {
            project: &project,
            opts: &opts,
            engine: &engine,
        },
    );
    console.follow(99);
    assert_eq!(
        console.run().target().rel,
        "issues",
        "a bad link must not lose the page you were reading"
    );
}
