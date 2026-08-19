//! `rq` — a better curl, powered by collections.
//!
//! Curl in, named verb out, editor for everything else.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

use rq::console;
use rq::doc::Document;
use rq::embedded;
use rq::import;
use rq::project::{self, Kind, Project};
use rq::render;
use rq::run::{self, RunOptions};
use rq::script::{self, TestStatus};
use rq::ui::{self, ColorChoice};
use rq::vars;

#[derive(Parser, Debug)]
#[command(
    name = "rq",
    version,
    about = "A better curl, powered by collections",
    long_about = "rq runs named requests kept as plain files in your project.\n\n\
                  Save a curl, give it a name, and it becomes a verb in your shell:\n  \
                  rq curl --save-as issues 'curl https://api.github.com/…'\n  \
                  rq r issues",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Use this project instead of searching upward for rq.toml. A Postman export, a Bruno
    /// collection or a curl file works here too — rq reads it in place, writing nothing.
    #[arg(long, global = true, value_name = "DIR|FILE")]
    project: Option<PathBuf>,

    /// Read the project as this format instead of detecting it: postman, bruno, curl, rq.
    #[arg(long, global = true, value_name = "FORMAT")]
    from: Option<String>,

    /// When to colour output.
    #[arg(long, global = true, value_enum, default_value = "auto")]
    color: Color,

    /// Draw with ASCII instead of box-drawing characters.
    #[arg(long, global = true)]
    ascii: bool,

    /// Environment for bare `rq` (the project browser); defaults to the active one.
    #[arg(short = 'e', long, value_name = "NAME")]
    environment: Option<String>,

    /// Set a variable for bare `rq`. Repeatable.
    #[arg(long = "var", value_name = "KEY=VALUE")]
    vars: Vec<String>,

    /// Browse the list. On by default when there is a terminal to draw on.
    #[arg(long, short = 'c')]
    console: bool,

    /// Print the list instead of browsing it.
    #[arg(long, conflicts_with = "console")]
    no_console: bool,

    /// The project as JSON on stdout.
    #[arg(long)]
    json: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Color {
    Auto,
    Always,
    Never,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run a request, and everything it declares as a parent.
    #[command(alias = "run")]
    R(RunArgs),

    /// Open a request in $EDITOR.
    #[command(alias = "edit")]
    E {
        /// Request name, or path within the project.
        name: String,
    },

    /// List the requests in this project. This is what bare `rq` does.
    #[command(alias = "list", alias = "ls")]
    L {
        /// A collection to read instead of this project: a Postman export, a Bruno tree, a
        /// curl file. Read in place — nothing is written and nothing is converted on disk.
        #[arg(value_name = "FILE")]
        source: Option<PathBuf>,

        /// Browse them: arrow to one, enter to run it. On by default on a terminal.
        #[arg(long, short = 'c')]
        console: bool,

        /// Print them instead.
        #[arg(long, conflicts_with = "console")]
        no_console: bool,

        /// The project as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Create an empty project in this directory.
    Init {
        /// Where to create it (default: here).
        dir: Option<PathBuf>,
    },

    /// Save a curl command as a named request.
    Curl(CurlArgs),

    /// Import a Postman collection, Bruno tree, or curl file into this project.
    Import(ImportArgs),

    /// Show, list, or switch the active environment.
    Env {
        #[command(subcommand)]
        command: Option<EnvCommand>,
    },
}

#[derive(Args, Debug)]
struct RunArgs {
    /// Request name (`issues`) or path (`github/issues`).
    name: String,

    /// Environment to run against; defaults to the active one.
    #[arg(short = 'e', long, value_name = "NAME")]
    environment: Option<String>,

    /// Set a variable for this run. Repeatable.
    #[arg(long = "var", value_name = "KEY=VALUE")]
    vars: Vec<String>,

    /// Ask for every declared variable, even the ones with defaults.
    #[arg(long)]
    prompt: bool,

    /// Print the response body instead of the rendered view.
    #[arg(long)]
    raw: bool,

    /// Show more of what happened. Repeatable.
    #[arg(long, value_enum, value_name = "WHAT")]
    show: Vec<Show>,

    /// Exit non-zero when the response is not 2xx.
    #[arg(long)]
    fail: bool,

    /// Treat any note (unresolved variable, unrun script) as an error.
    #[arg(long)]
    strict: bool,

    /// Wall clock for each script, in milliseconds.
    #[arg(long, value_name = "MS")]
    script_timeout: Option<u64>,

    /// Open the run in the console. On by default when there is a terminal to draw on.
    #[arg(long, short = 'c')]
    console: bool,

    /// Don't open the console, even on a terminal. Printing only.
    #[arg(long, conflicts_with = "console")]
    no_console: bool,

    /// The whole run as one JSON object on stdout, and nothing else.
    #[arg(long)]
    json: bool,

    /// Follow a numbered link from the rendered view, then show that. Repeatable, so
    /// `--follow 2 --follow 1` walks two pages in.
    #[arg(long, value_name = "N")]
    follow: Vec<usize>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Show {
    Request,
    Headers,
    Timing,
    Vars,
    All,
}

#[derive(Args, Debug)]
struct CurlArgs {
    /// Name for the saved request. Slashes nest it in a collection.
    #[arg(long = "save-as", value_name = "NAME")]
    save_as: Option<String>,

    /// The curl command — with or without the leading `curl`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
    command: Vec<String>,
}

#[derive(Args, Debug)]
struct ImportArgs {
    /// File to import.
    file: PathBuf,

    /// Source format. Detected from the file when omitted.
    #[arg(long, value_name = "FORMAT")]
    from: Option<String>,
}

#[derive(Subcommand, Debug)]
enum EnvCommand {
    /// List environments; the active one is marked.
    List,
    /// Make an environment active for this machine.
    Switch {
        /// Environment name, or `none` to clear it.
        name: String,
    },
    /// Print an environment's variables.
    Show { name: Option<String> },
}

fn main() {
    restore_sigpipe();
    let cli = Cli::parse();
    ui::init(match cli.color {
        Color::Auto => ColorChoice::Auto,
        Color::Always => ColorChoice::Always,
        Color::Never => ColorChoice::Never,
    });
    ui::set_unicode(!cli.ascii);

    match dispatch(&cli) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("{} {e:#}", ui::red("error:"));
            std::process::exit(2);
        }
    }
}

/// Die quietly when the reader goes away, the way `cat` and `ls` do.
///
/// Rust ignores `SIGPIPE`, so a write to a closed pipe returns `EPIPE` and `println!`
/// *panics* — which means `rq l | head` printed a backtrace at you. Restoring the default
/// handler makes the process end where the pipe ended.
fn restore_sigpipe() {
    #[cfg(unix)]
    // SAFETY: setting a signal disposition to the OS default before any threads exist.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

fn dispatch(cli: &Cli) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    match &cli.command {
        // Bare `rq` *is* `rq l` — same output, same flags, no surprise depending on
        // whether something is watching. `-c` browses either way.
        None => list_or_browse(cli, None, cli.console, cli.no_console, cli.json),
        Some(Command::L {
            source,
            console,
            no_console,
            json,
        }) => list_or_browse(cli, source.as_deref(), *console, *no_console, *json),
        Some(Command::R(args)) => {
            let project = open(cli, &cwd)?;
            run_request(&project, args)
        }
        Some(Command::E { name }) => {
            let project = open(cli, &cwd)?;
            edit(&project, name)?;
            Ok(0)
        }
        Some(Command::Init { dir }) => {
            let dir = dir.clone().unwrap_or(cwd);
            if project::init(&dir)? {
                println!(
                    "{} project created in {}",
                    ui::green(ui::tick()),
                    dir.display()
                );
                println!(
                    "  next: {}",
                    ui::bold("rq curl --save-as ip 'curl https://api.ipify.org?format=json'")
                );
            } else {
                println!("already an rq project: {}", dir.display());
            }
            Ok(0)
        }
        Some(Command::Curl(args)) => save_curl(cli, &cwd, args),
        Some(Command::Import(args)) => import(cli, &cwd, args),
        Some(Command::Env { command }) => {
            let project = open(cli, &cwd)?;
            environment(&project, command.as_ref())
        }
    }
}

fn open(cli: &Cli, cwd: &Path) -> Result<Project> {
    open_at(cli, cli.project.as_deref(), cwd)
}

/// `open`, with the project location supplied rather than taken from `--project`.
fn open_at(cli: &Cli, explicit: Option<&Path>, cwd: &Path) -> Result<Project> {
    let (project, report) = Project::locate(explicit, cwd, cli.from.as_deref())?;
    // Reading a foreign collection is not a silent act: say what was read and as what, so a
    // surprising result is traceable to the guess that produced it. Narration, so it lands on
    // stderr and never in piped data.
    if let Some((from, format)) = project.converted_from() {
        let requests = project.requests().count();
        eprintln!(
            "{} {} {} as {} — {} request{}, nothing written",
            ui::dim("reading"),
            ui::bold(&from.file_name().unwrap_or_default().to_string_lossy()),
            ui::dim("in place"),
            ui::bold(format),
            requests,
            if requests == 1 { "" } else { "s" }
        );
        // Not every note, every time. A conversion of a large collection can have dozens,
        // and a wall of them before each run buries whatever you were doing — the count is
        // what tells you whether to go looking.
        if let Some(report) = &report {
            if let Some(summary) = conversion_summary(report) {
                ui::note(&format!(
                    "{summary} — `rq import {}` writes it out and lists them",
                    from.display()
                ));
            }
        }
    }
    Ok(project)
}

/// The project's requests: printed, or browsed when asked for and possible.
fn list_or_browse(
    cli: &Cli,
    source: Option<&Path>,
    console: bool,
    no_console: bool,
    json: bool,
) -> Result<i32> {
    // `rq l ./api.postman_collection.json` is the same thing as `--project` pointing at it;
    // the positional exists because pointing rq at a file is the obvious way to ask.
    let cwd = std::env::current_dir()?;
    let project = match source {
        Some(path) => open_at(cli, Some(path), &cwd)?,
        None => open(cli, &cwd)?,
    };
    if json {
        println!("{:#}", project_json(&project));
        return Ok(0);
    }
    // Browsing is what you want when you are looking; printing is what you want when
    // something else is reading. `--console` insists, `--no-console` refuses.
    if wants_console(console, no_console) {
        return browse(cli, &project);
    }
    if console {
        ui::note("--console needs a terminal; printed the list instead");
    }
    list(&project);
    Ok(0)
}

/// Interactive unless told otherwise, and never without a terminal to be interactive in.
///
/// `--console` does not force anything a pipe cannot do; it only makes the intent explicit,
/// so that when there is no terminal we can say so instead of silently printing.
fn wants_console(_console: bool, no_console: bool) -> bool {
    !no_console && console::available()
}

/// The project, for tooling: one object per runnable request.
fn project_json(project: &Project) -> serde_json::Value {
    let requests: Vec<serde_json::Value> = project
        .requests()
        .map(|(idx, entry)| {
            let doc = project.load(idx).ok();
            let front = doc.as_ref().map(|(d, _)| &d.front);
            serde_json::json!({
                "name": entry.rel,
                "method": front.and_then(|f| f.method.clone()).unwrap_or_else(|| "GET".into()),
                "url": front.and_then(|f| f.url.clone()),
                "description": doc.as_ref().and_then(|(d, _)| d.summary()),
                "parents": front.map(|f| f.parents.clone()).unwrap_or_default(),
                "collection": entry.kind == Kind::Collection,
                "form": doc
                    .as_ref()
                    .and_then(|(d, _)| d.form().ok())
                    .map(|fields| fields.iter().map(|f| f.name.clone()).collect::<Vec<_>>())
                    .unwrap_or_default(),
            })
        })
        .collect();
    serde_json::json!({
        "root": project.root,
        "requests": requests,
        "environments": project.environments(),
        "active_environment": project.active_env(),
        "notes": project.notes,
    })
}

/// Open the project browser: the request list, with the same environment and variables a
/// run would use.
fn browse(cli: &Cli, project: &Project) -> Result<i32> {
    let mut cli_vars = Vec::new();
    for raw in &cli.vars {
        cli_vars.push(vars::parse_assignment(raw)?);
    }
    let opts = RunOptions {
        cli_vars,
        environment: cli.environment.clone(),
        interactive: false,
        ..RunOptions::default()
    };
    let engine = pick_engine();
    console::browse(console::Nav {
        project,
        opts: &opts,
        engine: engine.as_ref(),
    })?;
    Ok(0)
}

// ---------------------------------------------------------------------------------------
// rq r
// ---------------------------------------------------------------------------------------

/// The engine to run scripts with.
///
/// By default it is the one compiled into this binary: the cross-q-context guest realm on an
/// in-process QuickJS. Nothing to install, so `-- pre --` and `-- post --` work the same on a
/// laptop, in CI, and in a downloaded release.
///
/// Setting `RQ_SCRIPT_ENGINE` opts back into the Node sidecar against a cross-q-context
/// checkout. That is for developing the engine — running a script through both and comparing
/// — and for the suites that pin it at a nonexistent path to keep themselves hermetic.
fn pick_engine() -> Box<dyn script::ScriptEngine> {
    if std::env::var_os("RQ_SCRIPT_ENGINE").is_some() {
        return match script::NodeEngine::discover() {
            Ok(engine) => Box::new(engine),
            Err(why) => Box::new(script::NoEngine::because(why)),
        };
    }
    Box::new(embedded::EmbeddedEngine)
}

/// One run, as data. Everything the terminal shows and the things it can't: per-phase
/// timings, every header, the parsed body when there is one.
///
/// This is the shape CI reads — `rq r checks --json | jq '.tests.failed'` — so it carries
/// the run rather than a rendering of it.
fn run_json(outcome: &run::Run) -> serde_json::Value {
    let steps: Vec<serde_json::Value> = outcome
        .steps
        .iter()
        .map(|step| {
            let response = step.response.as_ref();
            serde_json::json!({
                "name": step.rel,
                "method": step.method,
                "url": step.url,
                "skipped": step.skipped(),
                "status": response.map(|r| r.status),
                "statusText": response.map(|r| r.status_text.clone()),
                "headers": response.map(|r| r
                    .headers
                    .iter()
                    .map(|(k, v)| (k.to_lowercase(), serde_json::Value::String(v.clone())))
                    .collect::<serde_json::Map<String, serde_json::Value>>()),
                "bytes": response.map(|r| r.bytes),
                "timeMs": response.map(|r| r.elapsed.as_millis() as u64),
                "timings": response.map(|r| serde_json::json!({
                    "dnsMs": r.timings.dns.as_millis() as u64,
                    "tcpMs": r.timings.tcp.as_millis() as u64,
                    "waitingMs": r.timings.waiting.as_millis() as u64,
                    "downloadMs": r.timings.download.as_millis() as u64,
                })),
                "captured": step
                    .captured
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect::<serde_json::Map<String, serde_json::Value>>(),
                "tests": step.tests.iter().map(|t| serde_json::json!({
                    "name": t.name,
                    "status": format!("{:?}", t.status).to_lowercase(),
                    "error": t.error,
                })).collect::<Vec<_>>(),
                "notes": step.notes,
            })
        })
        .collect();

    let target = outcome.target();
    let response = target.response.as_ref();
    serde_json::json!({
        "request": target.rel,
        "ok": response.map(|r| r.ok()).unwrap_or(false),
        "steps": steps,
        "body": response.map(|r| r.body.clone()),
        // The parsed body when it is JSON, so nobody has to pipe through `fromjson`.
        "json": response.and_then(|r| r.json()),
        "view": outcome.view,
        "tests": {
            "total": outcome.total_tests(),
            "failed": outcome.failed_tests(),
        },
        "notes": outcome.notes,
    })
}

/// The form a request declares, filled in — or `None` when there is nothing to ask.
///
/// Nothing is asked when: the request has no form, there is no terminal to ask on (CI
/// passes `--var`, and a form that blocked a pipeline would be a trap), or every field
/// already has a value.
fn maybe_fill_form(
    project: &Project,
    target: usize,
    opts: &RunOptions,
) -> Result<Option<Vec<(String, String)>>> {
    if !console::available() {
        return Ok(None);
    }
    let (doc, _) = project.load(target)?;
    let mut fields = doc.form().map_err(|e| anyhow::anyhow!("{e}"))?;
    if fields.is_empty() {
        return Ok(None);
    }

    // `default: '{{me}}'` should show you, not its own source code.
    let ambient = run::ambient_vars(project, opts);
    for field in &mut fields {
        if let Some(default) = &field.default {
            field.default = Some(vars::substitute(default, &ambient).text);
        }
    }

    let supplied: Vec<(String, String)> = opts.cli_vars.clone();
    if fields
        .iter()
        .all(|f| supplied.iter().any(|(k, v)| *k == f.name && !v.is_empty()))
    {
        return Ok(None);
    }

    let title = doc
        .summary()
        .unwrap_or_else(|| project.entries[target].rel.clone());
    match console::fill_form(&title, fields, &supplied)? {
        Some(values) => Ok(Some(values)),
        None => bail!("cancelled"),
    }
}

/// Everything one run puts on screen: the step tree, whatever `--show` asked for, and
/// the rendered view. Following a link prints the next page with the same function, so
/// page two looks exactly like page one.
fn print_run(outcome: &run::Run, args: &RunArgs) {
    let show = |what: Show| args.show.contains(&what) || args.show.contains(&Show::All);
    let secrets = &outcome.secrets;

    // The run tree: one line per step, the requested one last.
    for (i, step) in outcome.steps.iter().enumerate() {
        let lead = if i == 0 {
            ui::arrow().to_string()
        } else {
            format!("{}{}", "  ".repeat(i - 1), ui::branch())
        };
        let outcome_cell = match &step.response {
            Some(r) => format!(
                "{}  {}",
                ui::status(
                    r.status,
                    format!("{} {}", r.status, r.status_text).trim_end()
                ),
                ui::dim(&format!("{}ms", r.elapsed.as_millis()))
            ),
            None => ui::dim("skipped"),
        };
        eprintln!(
            "{lead} {}  {} {}  {outcome_cell}",
            ui::bold(&step.name),
            step.method,
            ui::dim(&ui::short_url(&step.url)),
        );
        for log in &step.logs {
            eprintln!(
                "     {} {}",
                ui::dim(&format!("{}:", log.level)),
                ui::redact(&log.message(), secrets)
            );
        }
        for test in &step.tests {
            let (mark, name) = match test.status {
                TestStatus::Passed => (ui::green(ui::tick()), test.name.clone()),
                TestStatus::Failed => (ui::red("✗"), ui::bold(&test.name)),
                TestStatus::Skipped => (ui::dim("-"), ui::dim(&test.name)),
            };
            let detail = test
                .error
                .as_deref()
                .map(|e| ui::dim(&format!(" — {e}")))
                .unwrap_or_default();
            eprintln!("     {mark} {name}{detail}");
        }
        for (key, value) in &step.captured {
            let shown = if secrets.iter().any(|s| s == value) {
                "***".to_string()
            } else {
                truncate(value, 40)
            };
            eprintln!("     {} {} = {}", ui::dim("captured"), ui::cyan(key), shown);
        }
    }

    let target_step = outcome.target();

    if show(Show::Request) {
        eprintln!("\n{}", ui::dim("── request ─────────────────────────────"));
        eprintln!("{} {}", target_step.method, target_step.url);
        for (k, v) in &target_step.request_headers {
            eprintln!("{k}: {}", ui::redact(v, secrets));
        }
        if let Some(body) = &target_step.body {
            eprintln!("{}", ui::dim(&format!("[{body}]")));
        }
    }
    if show(Show::Headers) {
        eprintln!("\n{}", ui::dim("── response headers ────────────────────"));
        for (k, v) in target_step.response.iter().flat_map(|r| r.headers.iter()) {
            eprintln!("{k}: {v}");
        }
    }
    if show(Show::Vars) {
        eprintln!("\n{}", ui::dim("── variables ───────────────────────────"));
        for (key, value, origin) in &outcome.vars {
            eprintln!(
                "{:<24} {:<32} {}",
                ui::cyan(key),
                truncate(value, 32),
                ui::dim(origin)
            );
        }
    }
    if show(Show::Timing) {
        eprintln!("\n{}", ui::dim("── timing ──────────────────────────────"));
        for step in &outcome.steps {
            match &step.response {
                Some(r) => {
                    let phases = r
                        .timings
                        .phases()
                        .into_iter()
                        .map(|(name, d)| format!("{name} {}ms", d.as_millis()))
                        .collect::<Vec<_>>()
                        .join(&ui::dim(" · "));
                    eprintln!("{:<16} {:>6}ms  {phases}", step.name, r.elapsed.as_millis());
                }
                None => eprintln!("{:<16} {:>8}", step.name, "skipped"),
            }
        }
    }

    // The result goes to stdout and nowhere else, so `rq r x > file` and `rq r x | jq`
    // get exactly what a person sees on screen — the same bytes either way.
    eprintln!();
    let body = if args.raw || outcome.view.is_none() {
        outcome.raw.clone()
    } else {
        render::markdown_to_terminal(outcome.view.as_deref().unwrap_or_default())
    };
    let body = ui::redact(&body, secrets);
    println!("{}", body.trim_end());

    for note in &outcome.notes {
        ui::note(note);
    }
    for step in &outcome.steps {
        for note in &step.notes {
            ui::note(&format!("{}: {note}", step.rel));
        }
    }
}

fn run_request(project: &Project, args: &RunArgs) -> Result<i32> {
    let target = project.resolve(&args.name)?;
    let mut cli_vars = Vec::new();
    for raw in &args.vars {
        cli_vars.push(vars::parse_assignment(raw)?);
    }

    let mut opts = RunOptions {
        cli_vars,
        environment: args.environment.clone(),
        prompt: args.prompt,
        interactive: vars::stdin_is_interactive(),
        strict: args.strict,
        script_timeout_ms: args.script_timeout,
    };

    // A request that declares a `-- form --` is asking to be filled in. Show the form —
    // no flag, because the request already said so. Everything already supplied on the
    // command line is prefilled, and if that covers every field there is nothing to ask.
    if let Some(values) = maybe_fill_form(project, target, &opts)? {
        for (key, value) in values {
            opts.cli_vars.retain(|(k, _)| *k != key);
            opts.cli_vars.push((key, value));
        }
    }

    let engine = pick_engine();
    let outcome = run::run(project, target, &opts, engine.as_ref())?;
    if args.json {
        println!("{:#}", run_json(&outcome));
    } else {
        print_run(&outcome, args);
    }

    // Link following: each hop replaces what is on screen, the way clicking does.
    let mut outcome = outcome;
    for number in &args.follow {
        let links = outcome.links();
        let Some(link) = links.iter().find(|l| l.number == *number) else {
            bail!(
                "no link [{number}] in that view{}",
                if links.is_empty() {
                    " (it offers none)".to_string()
                } else {
                    format!(" — it offers 1..{}", links.len())
                }
            );
        };
        eprintln!(
            "\n{} {}",
            ui::dim("follow →"),
            ui::bold(&format!("{} ({})", link.label.trim(), link.target))
        );
        outcome = run::follow(project, link, &opts, engine.as_ref())?;
        print_run(&outcome, args);
    }

    if !args.json && wants_console(args.console, args.no_console) {
        {
            // The console navigates: its links run through the same project, options and
            // engine this run used, so following one stays in the same session.
            // Inside the console nothing may prompt on the terminal — the alt screen is
            // already drawing there. A form is how the console asks.
            let console_opts = RunOptions {
                interactive: false,
                ..opts.clone()
            };
            let nav = console::Nav {
                project,
                opts: &console_opts,
                engine: engine.as_ref(),
            };
            console::open(outcome.clone(), Some(nav))?;
        }
    } else if args.console {
        ui::note("--console needs a terminal; printed the run instead");
    }

    let failed_tests = outcome.failed_tests();
    if outcome.total_tests() > 0 {
        let summary = format!(
            "{}/{} test(s) passed",
            outcome.total_tests() - failed_tests,
            outcome.total_tests()
        );
        eprintln!(
            "\n{}",
            if failed_tests == 0 {
                ui::green(&summary)
            } else {
                ui::red(&summary)
            }
        );
    }

    // A failed assertion is a non-zero exit with no flag needed — that is what makes `rq r`
    // usable as a CI step. A non-2xx response is only an error if you say so with `--fail`,
    // because a 404 may be exactly what the request was checking for.
    Ok(if failed_tests > 0 || (args.fail && outcome.failed()) {
        1
    } else {
        0
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

// ---------------------------------------------------------------------------------------
// rq l
// ---------------------------------------------------------------------------------------

fn list(project: &Project) {
    let mut requests = 0usize;
    let mut collections = 0usize;
    print_children(project, &project.roots, "", &mut requests, &mut collections);

    let envs = project.environments();
    let active = project.active_env();
    println!();
    println!(
        "{}",
        ui::dim(&format!(
            "{requests} request{} across {collections} collection{} · {} environment{}{}",
            plural(requests),
            plural(collections),
            envs.len(),
            plural(envs.len()),
            active
                .map(|a| format!(" · active: {a}"))
                .unwrap_or_default()
        ))
    );
    if requests == 0 {
        println!(
            "  {}",
            ui::dim("nothing here yet — `rq curl --save-as <name> '<curl …>'` saves your first")
        );
    }
}

fn print_children(
    project: &Project,
    children: &[usize],
    indent: &str,
    requests: &mut usize,
    collections: &mut usize,
) {
    let width = children
        .iter()
        .filter(|i| project.entries[**i].kind == Kind::Request)
        .map(|i| project.entries[*i].name.len())
        .max()
        .unwrap_or(0)
        .max(8);

    for (n, idx) in children.iter().enumerate() {
        let entry = &project.entries[*idx];
        let last = n + 1 == children.len();
        let stem = if last { ui::elbow() } else { ui::tee() };
        match entry.kind {
            Kind::Collection => {
                *collections += 1;
                println!("{indent}{stem} {}", ui::bold(&format!("{}/", entry.name)));
                let next = format!("{indent}{}", if last { "   " } else { ui::pipe() });
                print_children(project, &entry.children, &next, requests, collections);
            }
            Kind::Request => {
                *requests += 1;
                let (method, path, parents, summary) = match project.load(*idx) {
                    Ok((doc, _)) => (
                        doc.front.method.clone().unwrap_or_else(|| "GET".into()),
                        doc.front
                            .url
                            .clone()
                            .map(|u| ui::short_url(&u))
                            .unwrap_or_default(),
                        doc.front.parents.clone(),
                        doc.summary(),
                    ),
                    Err(_) => (
                        "?".into(),
                        ui::red("unreadable").to_string(),
                        Vec::new(),
                        None,
                    ),
                };
                let arrows = if parents.is_empty() {
                    String::new()
                } else {
                    ui::dim(&format!("  ← {}", parents.join(", ")))
                };
                println!(
                    "{indent}{stem} {:<width$}  {:<6} {}{}",
                    entry.name,
                    method,
                    ui::dim(&path),
                    arrows,
                );
                // What the request says it is, under it — the URL says where it goes, which
                // is not the same question.
                if let Some(summary) = summary {
                    let gutter = if last { "   " } else { ui::pipe() };
                    println!("{indent}{gutter} {:<width$}  {}", "", ui::dim(&summary));
                }
            }
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

// ---------------------------------------------------------------------------------------
// rq e
// ---------------------------------------------------------------------------------------

fn edit(project: &Project, name: &str) -> Result<()> {
    let idx = project.resolve(name)?;
    // A converted request has no file — it was made out of the source collection a moment
    // ago. Opening the source is what someone asking to edit it actually wants; opening a
    // path that does not exist is not.
    let path = match project.converted_from() {
        Some((from, format)) => {
            ui::note(&format!(
                "{} is a {format} collection — opening it, not the request",
                from.display()
            ));
            from.to_path_buf()
        }
        None => project.entries[idx].file(),
    };
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".into()
            } else {
                "vi".into()
            }
        });

    let mut parts = editor.split_whitespace();
    let program = parts.next().unwrap_or("vi");
    let status = std::process::Command::new(program)
        .args(parts)
        .arg(&path)
        .status()
        .with_context(|| format!("running {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}");
    }

    // Tell them now, not on the next run.
    let text = std::fs::read_to_string(&path)?;
    match Document::parse(&text) {
        Ok((_, notes)) => {
            for note in notes {
                ui::note(&note.to_string());
            }
        }
        Err(e) => bail!("{}: {e}", path.display()),
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// rq curl / rq import
// ---------------------------------------------------------------------------------------

fn save_curl(cli: &Cli, cwd: &Path, args: &CurlArgs) -> Result<i32> {
    let command = join_command(&args.command);
    let (project, created) = open_or_init(cli, cwd)?;

    let mut report = cq_report::Report::new(cq_report::Fidelity::Lossless);
    let mut ws = cross_q::curl_to_workspace(&command, &mut report)
        .map_err(|e| anyhow::anyhow!("could not read that curl command: {e}"))?;

    if let Some(name) = &args.save_as {
        let rel = project::slug_path(name)?;
        rename_single_request(&mut ws, &rel)?;
    }

    let map = cross_q::emit_rq_md::to_rq_md(&ws, &mut report);
    let written = import::write_project(&map, &project.root)?;
    if created {
        println!(
            "{} created collection {} (no project found — initialized one)",
            ui::green(ui::tick()),
            ui::bold(&project.root.display().to_string())
        );
    }
    let reopened = Project::open(project.root.clone())?;
    for rel in &written {
        let idx = reopened.resolve(rel)?;
        let (doc, _) = reopened.load(idx)?;
        println!(
            "{} saved request: {}  ({} {})",
            ui::green(ui::tick()),
            ui::bold(rel),
            doc.front.method.clone().unwrap_or_else(|| "GET".into()),
            ui::short_url(doc.front.url.as_deref().unwrap_or(""))
        );
    }
    report_notes(&report);
    if let Some(rel) = written.first() {
        println!("  run it: {}", ui::bold(&format!("rq r {rel}")));
    }
    Ok(0)
}

fn import(cli: &Cli, cwd: &Path, args: &ImportArgs) -> Result<i32> {
    // A directory is a collection tree (an rq project, a Bruno collection): it reaches the
    // importer as a virtual-FS map, exactly as it does through `cq`.
    let (content, detected) = if args.file.is_dir() {
        let map = import::read_dir_map(&args.file)?;
        let detected =
            if map.contains_key(project::MARKER) || map.keys().any(|k| k.ends_with(".md")) {
                Some("rq")
            } else if map.keys().any(|k| k.ends_with(".bru") || k == "bruno.json") {
                Some("bruno")
            } else {
                None
            };
        (serde_json::to_string(&map)?, detected)
    } else {
        let text = std::fs::read_to_string(&args.file)
            .with_context(|| format!("reading {}", args.file.display()))?;
        let detected = import::detect_format(&args.file, &text);
        (text, detected)
    };
    let format = match &args.from {
        Some(f) => f.clone(),
        None => detected
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "could not tell what {} is — pass --from postman|bruno|curl|rq",
                    args.file.display()
                )
            })?
            .to_string(),
    };

    let (project, created) = open_or_init(cli, cwd)?;
    let mut report = cq_report::Report::new(cq_report::Fidelity::Lossless);
    let ws = cross_q::build_workspace(&format, &content, &mut report)?;
    let map = cross_q::emit_rq_md::to_rq_md(&ws, &mut report);
    let written = import::write_project(&map, &project.root)?;
    let environments = map
        .keys()
        .filter(|k| k.starts_with(&format!("{}/", project::ENVS_DIR)) || *k == project::DOTENV)
        .count();

    if created {
        println!(
            "{} initialized {}",
            ui::green(ui::tick()),
            project.root.display()
        );
    }
    println!(
        "{} imported {} request{} and {} environment{} from {} ({format})",
        ui::green(ui::tick()),
        written.len(),
        plural(written.len()),
        environments,
        plural(environments),
        args.file.display(),
    );
    report_notes(&report);
    println!("  see them: {}", ui::bold("rq l"));
    Ok(0)
}

/// Print what the conversion couldn't carry cleanly. A conversion that claims success
/// while dropping data is the one unforgivable bug.
/// What the conversion had to change, in one line — or nothing at all when it was clean.
fn conversion_summary(report: &cq_report::Report) -> Option<String> {
    let mut dropped = 0usize;
    let mut coerced = 0usize;
    let mut errors = 0usize;
    for d in &report.diagnostics {
        match d.severity {
            cq_report::Severity::Dropped => dropped += 1,
            cq_report::Severity::Coerced => coerced += 1,
            cq_report::Severity::Error => errors += 1,
            _ => {}
        }
    }
    let mut parts = Vec::new();
    if errors > 0 {
        parts.push(format!(
            "{errors} error{}",
            if errors == 1 { "" } else { "s" }
        ));
    }
    if dropped > 0 {
        parts.push(format!("{dropped} dropped"));
    }
    if coerced > 0 {
        parts.push(format!("{coerced} coerced"));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn report_notes(report: &cq_report::Report) {
    let mut shown = 0usize;
    for d in &report.diagnostics {
        if matches!(
            d.severity,
            cq_report::Severity::Coerced
                | cq_report::Severity::Dropped
                | cq_report::Severity::Error
        ) {
            ui::note(&format!("{:?}: {}", d.severity, d.message));
            shown += 1;
            if shown == 10 {
                ui::note(&format!(
                    "…and {} more",
                    report.diagnostics.len().saturating_sub(shown)
                ));
                break;
            }
        }
    }
}

fn open_or_init(cli: &Cli, cwd: &Path) -> Result<(Project, bool)> {
    // Disk only: this is the path that is about to write a file.
    match Project::find_on_disk(cli.project.as_deref(), cwd) {
        Ok(p) => Ok((p, false)),
        Err(_) => {
            let root = cli.project.clone().unwrap_or_else(|| cwd.to_path_buf());
            project::init(&root)?;
            Ok((Project::open(root)?, true))
        }
    }
}

/// `--save-as github/issues`: rename the single request a curl produced, and nest it.
fn rename_single_request(ws: &mut cq_model::Workspace, rel: &str) -> Result<()> {
    let (folders, name) = match rel.rsplit_once('/') {
        Some((f, n)) => (f, n),
        None => ("", rel),
    };
    let root = ws
        .collections
        .first_mut()
        .ok_or_else(|| anyhow::anyhow!("the curl produced nothing to save"))?;
    let Some(cq_model::Item::Request(request)) = root.items.first_mut() else {
        bail!("the curl produced nothing to save");
    };
    request.meta.name = name.to_string();

    // Wrap it in one collection per path segment, outermost first.
    for folder in folders.split('/').rev().filter(|s| !s.is_empty()) {
        let inner = std::mem::take(&mut root.items);
        let wrapper = cq_model::Collection {
            meta: cq_model::RecordMeta::new(
                format!("rq-{folder}"),
                folder,
                cq_model::SourceFormat::Curl,
            ),
            items: inner,
            ..cq_model::Collection::default()
        };
        root.items = vec![cq_model::Item::Collection(Box::new(wrapper))];
    }
    Ok(())
}

/// Rebuild a shell-safe command line from the arguments clap collected, so
/// `rq curl --save-as x -d '{"a": 1}' https://…` reaches the parser as one command.
fn join_command(parts: &[String]) -> String {
    // One argument is already a command line — `rq curl 'curl -H "…" https://…'`, the
    // shape you get from pasting a curl out of Slack with quotes around it.
    if parts.len() == 1 {
        let single = parts[0].trim();
        let body = single.strip_prefix("curl ").unwrap_or(single).trim_start();
        return format!("curl {body}");
    }

    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 && part == "curl" {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        let needs_quotes = part.is_empty()
            || part.chars().any(|c| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '{' | '}' | '$' | '&' | '|')
            });
        if needs_quotes && !part.contains('\'') {
            out.push('\'');
            out.push_str(part);
            out.push('\'');
        } else if needs_quotes {
            out.push('"');
            out.push_str(&part.replace('\\', "\\\\").replace('"', "\\\""));
            out.push('"');
        } else {
            out.push_str(part);
        }
    }
    format!("curl {out}")
}

// ---------------------------------------------------------------------------------------
// rq env
// ---------------------------------------------------------------------------------------

fn environment(project: &Project, command: Option<&EnvCommand>) -> Result<i32> {
    let active = project.active_env();
    match command {
        None | Some(EnvCommand::List) => {
            let envs = project.environments();
            if envs.is_empty() {
                println!(
                    "no environments yet — add {}",
                    ui::bold(&format!("{}/staging.md", project.env_dir().display()))
                );
                return Ok(0);
            }
            for name in envs {
                let mark = if Some(&name) == active.as_ref() {
                    ui::green("*")
                } else {
                    " ".to_string()
                };
                let count = project
                    .load_env(&name)
                    .map(|(d, _)| d.front.vars.len())
                    .unwrap_or(0);
                println!("{mark} {:<20} {}", name, ui::dim(&format!("{count} vars")));
            }
            Ok(0)
        }
        Some(EnvCommand::Switch { name }) => {
            if name == "none" {
                project.set_active_env(None)?;
                println!("{} no active environment", ui::green(ui::tick()));
                return Ok(0);
            }
            project.load_env(name)?;
            project.set_active_env(Some(name))?;
            println!(
                "{} active environment: {}",
                ui::green(ui::tick()),
                ui::bold(name)
            );
            Ok(0)
        }
        Some(EnvCommand::Show { name }) => {
            let name = name
                .clone()
                .or(active)
                .ok_or_else(|| anyhow::anyhow!("no active environment — `rq env switch <name>`"))?;
            let (doc, notes) = project.load_env(&name)?;
            println!("{}", ui::bold(&name));
            for (key, spec) in &doc.front.vars {
                let value = if spec.secret {
                    "***".to_string()
                } else {
                    spec.default.clone().unwrap_or_default()
                };
                let from = spec
                    .env
                    .as_ref()
                    .map(|e| ui::dim(&format!("  (from ${e})")))
                    .unwrap_or_default();
                println!("  {:<24} {value}{from}", ui::cyan(key));
            }
            for note in notes {
                ui::note(&note.to_string());
            }
            Ok(0)
        }
    }
}
