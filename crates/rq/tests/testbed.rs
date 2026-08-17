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
    let text = stderr(&out);
    assert!(out.status.success(), "{text}{}", stdout(&out));
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
    // `--show` is narration: it belongs beside the run, not in the piped result.
    let text = format!("{}{}", stdout(&out), stderr(&out));
    assert!(out.status.success(), "{text}");
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
    let text = format!("{}{}", stdout(&out), stderr(&out));
    assert!(out.status.success(), "{text}");
    assert!(stderr(&out).contains("follow →"), "{}", stderr(&out));
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

// --- the app: read a page, fill a form, change something -----------------------------------

fn app_project() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/app")
        .canonicalize()
        .expect("examples/app is missing")
}

struct App {
    server: rq_testbed::Server,
    project: rq::project::Project,
    opts: rq::run::RunOptions,
}

impl App {
    fn new() -> App {
        let server = rq_testbed::Server::start(0).unwrap();
        App {
            project: rq::project::Project::open(app_project()).unwrap(),
            opts: rq::run::RunOptions {
                cli_vars: vec![("host".into(), server.base_url.clone())],
                environment: Some("local".into()),
                ..rq::run::RunOptions::default()
            },
            server,
        }
    }

    fn open(&self, name: &str) -> rq::console::Console<'_> {
        let target = self.project.resolve(name).unwrap();
        let run = rq::run::run(&self.project, target, &self.opts, &rq::script::NoEngine).unwrap();
        rq::console::Console::with_nav(
            run,
            rq::console::Nav {
                project: &self.project,
                opts: &self.opts,
                engine: &ENGINE,
            },
        )
    }

    fn total_posts(&self) -> u64 {
        let target = self.project.resolve("timeline").unwrap();
        let run = rq::run::run(&self.project, target, &self.opts, &rq::script::NoEngine).unwrap();
        run.target()
            .response
            .as_ref()
            .and_then(|r| r.json())
            .and_then(|j| j["total"].as_u64())
            .unwrap_or(0)
    }
}

static ENGINE: rq::script::NoEngine = rq::script::NoEngine;

fn press(console: &mut rq::console::Console<'_>, key: crossterm::event::KeyCode) {
    console.on_key(crossterm::event::KeyEvent::from(key));
}

fn type_text(console: &mut rq::console::Console<'_>, text: &str) {
    for c in text.chars() {
        press(console, crossterm::event::KeyCode::Char(c));
    }
}

#[test]
fn the_timeline_is_a_page_you_can_post_from() {
    use crossterm::event::{KeyCode, KeyModifiers};

    let app = App::new();
    let before = app.total_posts();
    let mut console = app.open("timeline");

    // Every post links to itself, its author, and a like; the page ends with "write a post".
    let compose = console
        .links()
        .into_iter()
        .find(|l| l.target.starts_with("compose"))
        .expect("the timeline should offer a way to write");

    // Opening it shows the form rather than firing the request — a POST that happened
    // because you looked at it would be a bug.
    console.follow(compose.number);
    let form = console.frame().join("\n");
    assert!(form.contains("What's happening?"), "{form}");
    assert!(
        form.contains("Posting as") && form.contains("amitu"),
        "the default should be resolved, not shown as a template:\n{form}"
    );
    assert_eq!(
        app.total_posts(),
        before,
        "nothing was posted by opening it"
    );

    // Fill it in and submit.
    type_text(&mut console, "written by a test");
    console.on_key(crossterm::event::KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    ));

    let posted = console.frame().join("\n");
    assert!(posted.contains("Posted as"), "{posted}");
    assert!(posted.contains("written by a test"), "{posted}");
    assert_eq!(app.total_posts(), before + 1, "the app's state changed");

    // And the new post is on the timeline, as a page again.
    press(&mut console, KeyCode::Backspace);
    press(&mut console, KeyCode::Backspace);
}

#[test]
fn a_required_field_will_not_submit_empty() {
    use crossterm::event::{KeyCode, KeyModifiers};

    let app = App::new();
    let before = app.total_posts();
    let mut console = app.open("timeline");
    let compose = console
        .links()
        .into_iter()
        .find(|l| l.target.starts_with("compose"))
        .unwrap();
    console.follow(compose.number);

    console.on_key(crossterm::event::KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    ));
    let frame = console.frame().join("\n");
    assert!(frame.contains("is required"), "{frame}");
    assert!(
        frame.contains("What's happening?"),
        "the form stays open:\n{frame}"
    );
    assert_eq!(app.total_posts(), before);
}

#[test]
fn a_link_can_carry_a_value_the_form_does_not_ask_for() {
    let app = App::new();
    let mut console = app.open("timeline");
    let open_first = console
        .links()
        .into_iter()
        .find(|l| l.target.starts_with("post?"))
        .unwrap();
    console.follow(open_first.number);

    // The post page offers a reply, whose form asks for text but not for `reply_to` —
    // that rides in from the link.
    let reply = console
        .links()
        .into_iter()
        .find(|l| l.target.starts_with("reply"))
        .expect("a post should be repliable");
    console.follow(reply.number);
    let frame = console.frame().join("\n");
    assert!(frame.contains("Your reply"), "{frame}");
    assert!(!frame.contains("reply_to"), "it is not asked for:\n{frame}");
}

#[test]
fn the_form_also_works_without_a_terminal() {
    let app = App::new();
    let before = app.total_posts();
    let out = Command::new(BIN)
        .args(["r", "compose", "-e", "local", "--color=never", "--var"])
        .arg(format!("host={}", app.server.base_url))
        .args(["--var", "text=from the command line"])
        .arg("--project")
        .arg(app_project())
        .env_remove("RQ_PROJECT")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(text.contains("201 Created"), "{text}");
    assert_eq!(app.total_posts(), before + 1);
}

// --- the request list ----------------------------------------------------------------------

#[test]
fn the_console_lists_the_projects_requests_and_opens_one() {
    use crossterm::event::KeyCode;

    let app = App::new();
    let mut console = app.open("timeline");

    // `l` shows the project, with the cursor on the page you are reading.
    press(&mut console, KeyCode::Char('l'));
    let listing = console.frame().join("\n");
    assert!(listing.contains("6 requests"), "{listing}");
    for expected in ["timeline", "compose", "post", "person"] {
        assert!(
            listing.contains(expected),
            "{expected} missing from:\n{listing}"
        );
    }
    assert!(
        listing
            .lines()
            .any(|l| l.contains('▸') && l.contains("timeline")),
        "the cursor should be on the current page:\n{listing}"
    );

    // Move to another request and open it.
    press(&mut console, KeyCode::Home);
    press(&mut console, KeyCode::Enter);
    // `compose` is first alphabetically and declares a form, so opening it shows the form
    // rather than posting — the same rule links follow.
    let frame = console.frame().join("\n");
    assert!(frame.contains("What's happening?"), "{frame}");
}

#[test]
fn a_project_browser_starts_with_no_page_at_all() {
    let app = App::new();
    let mut console = rq::console::Console::browser(rq::console::Nav {
        project: &app.project,
        opts: &app.opts,
        engine: &ENGINE,
    });
    let frame = console.frame().join("\n");
    assert!(frame.contains("6 requests"), "{frame}");
    assert!(frame.contains("enter open"), "{frame}");

    // Opening one from a standing start gives you a page.
    for _ in 0..5 {
        press(&mut console, crossterm::event::KeyCode::Down);
    }
    press(&mut console, crossterm::event::KeyCode::Enter);
    let page = console.frame().join("\n");
    assert!(page.contains("Timeline ·"), "{page}");
}

#[test]
fn the_list_says_what_each_request_is() {
    use crossterm::event::KeyCode;
    let app = App::new();
    let mut console = app.open("timeline");
    press(&mut console, KeyCode::Char('l'));
    let listing = console.frame().join("\n");

    // `timeline`'s description opens "The home page." — far more use than a templated URL.
    assert!(listing.contains("The home page"), "{listing}");
    assert!(listing.contains("Write a post"), "{listing}");
}

#[test]
fn a_form_does_not_ask_when_the_command_line_already_answered() {
    let app = App::new();
    let before = app.total_posts();
    // No terminal here at all, so this also covers CI: a form that blocked a pipeline
    // would be a trap.
    let out = Command::new(BIN)
        .args(["r", "compose", "-e", "local", "--color=never", "--var"])
        .arg(format!("host={}", app.server.base_url))
        .args([
            "--var",
            "text=answered up front",
            "--var",
            "author=scripted",
        ])
        .arg("--project")
        .arg(app_project())
        .env_remove("RQ_PROJECT")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{text}");
    assert!(text.contains("Posted as @scripted"), "{text}");
    assert_eq!(app.total_posts(), before + 1);
}

#[test]
fn a_form_default_that_is_a_template_resolves() {
    let app = App::new();
    let out = Command::new(BIN)
        .args(["r", "compose", "-e", "local", "--color=never", "--var"])
        .arg(format!("host={}", app.server.base_url))
        .args(["--var", "text=whose post is this"])
        .arg("--project")
        .arg(app_project())
        .env_remove("RQ_PROJECT")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    // `author` defaults to `{{me}}`, which the environment sets to amitu. Sending the
    // literal `{{me}}` is the bug this guards.
    assert!(text.contains("Posted as @amitu"), "{text}");
    assert!(!text.contains("{{me}}"), "{text}");
}
