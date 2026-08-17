//! `rq` — a better curl, powered by collections.
//!
//! Curl in, named verb out, editor for everything else.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

use rq::console;
use rq::doc::Document;
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

    /// Use this project instead of searching upward for __requestly.json.
    #[arg(long, global = true, value_name = "DIR")]
    project: Option<PathBuf>,

    /// When to colour output.
    #[arg(long, global = true, value_enum, default_value = "auto")]
    color: Color,

    /// Draw with ASCII instead of box-drawing characters.
    #[arg(long, global = true)]
    ascii: bool,
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

    /// List the requests in this project.
    #[command(alias = "list", alias = "ls")]
    L,

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

    /// Open the run in the console: arrow between steps, drill into each one.
    #[arg(long, short = 'c')]
    console: bool,

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

fn dispatch(cli: &Cli) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    match &cli.command {
        None | Some(Command::L) => {
            let project = open(cli, &cwd)?;
            list(&project);
            Ok(0)
        }
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
    Project::find(cli.project.as_deref(), cwd)
}

// ---------------------------------------------------------------------------------------
// rq r
// ---------------------------------------------------------------------------------------

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
        println!(
            "{lead} {}  {} {}  {outcome_cell}",
            ui::bold(&step.name),
            step.method,
            ui::dim(&ui::short_url(&step.url)),
        );
        for log in &step.logs {
            println!(
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
            println!("     {mark} {name}{detail}");
        }
        for (key, value) in &step.captured {
            let shown = if secrets.iter().any(|s| s == value) {
                "***".to_string()
            } else {
                truncate(value, 40)
            };
            println!("     {} {} = {}", ui::dim("captured"), ui::cyan(key), shown);
        }
    }

    let target_step = outcome.target();

    if show(Show::Request) {
        println!("\n{}", ui::dim("── request ─────────────────────────────"));
        println!("{} {}", target_step.method, target_step.url);
        for (k, v) in &target_step.request_headers {
            println!("{k}: {}", ui::redact(v, secrets));
        }
        if let Some(body) = &target_step.body {
            println!("{}", ui::dim(&format!("[{body}]")));
        }
    }
    if show(Show::Headers) {
        println!("\n{}", ui::dim("── response headers ────────────────────"));
        for (k, v) in target_step.response.iter().flat_map(|r| r.headers.iter()) {
            println!("{k}: {v}");
        }
    }
    if show(Show::Vars) {
        println!("\n{}", ui::dim("── variables ───────────────────────────"));
        for (key, value, origin) in &outcome.vars {
            println!(
                "{:<24} {:<32} {}",
                ui::cyan(key),
                truncate(value, 32),
                ui::dim(origin)
            );
        }
    }
    if show(Show::Timing) {
        println!("\n{}", ui::dim("── timing ──────────────────────────────"));
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
                    println!("{:<16} {:>6}ms  {phases}", step.name, r.elapsed.as_millis());
                }
                None => println!("{:<16} {:>8}", step.name, "skipped"),
            }
        }
    }

    println!();
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

    let opts = RunOptions {
        cli_vars,
        environment: args.environment.clone(),
        prompt: args.prompt,
        interactive: vars::stdin_is_interactive(),
        strict: args.strict,
        script_timeout_ms: args.script_timeout,
    };

    // The engine this build hosts. Swapping in a real one is this line.
    let engine = script::NoEngine;
    let outcome = run::run(project, target, &opts, &engine)?;
    print_run(&outcome, args);

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
        println!(
            "\n{} {}",
            ui::dim("follow →"),
            ui::bold(&format!("{} ({})", link.label.trim(), link.target))
        );
        outcome = run::follow(project, link, &opts, &engine)?;
        print_run(&outcome, args);
    }

    if args.console {
        if console::available() {
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
                engine: &engine,
            };
            console::open(outcome.clone(), Some(nav))?;
        } else {
            ui::note("--console needs a terminal; printed the run instead");
        }
    }

    let failed_tests = outcome.failed_tests();
    if outcome.total_tests() > 0 {
        let summary = format!(
            "{}/{} test(s) passed",
            outcome.total_tests() - failed_tests,
            outcome.total_tests()
        );
        println!(
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
                let (method, path, parents) = match project.load(*idx) {
                    Ok((doc, _)) => (
                        doc.front.method.clone().unwrap_or_else(|| "GET".into()),
                        doc.front
                            .url
                            .clone()
                            .map(|u| ui::short_url(&u))
                            .unwrap_or_default(),
                        doc.front.parents.clone(),
                    ),
                    Err(_) => ("?".into(), ui::red("unreadable").to_string(), Vec::new()),
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
    let path = project.entries[idx].file();
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
        let detected = if map.contains_key(project::MARKER)
            || map.keys().any(|k| k.ends_with(project::REQUEST_FILE))
        {
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
        .filter(|k| k.starts_with(&format!("{}/", project::ENVS_DIR)))
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
    match Project::find(cli.project.as_deref(), cwd) {
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
