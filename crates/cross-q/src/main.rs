//! `cq` — the cross-q command line.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cq",
    version,
    about = "Convert API-client collections between formats, through one idealised model."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert a collection to another format.
    Convert {
        /// A curl command string, or a path to an input file.
        input: String,
        /// Target format (currently: rq).
        #[arg(long)]
        to: String,
        /// Source format override (autodetected if omitted).
        #[arg(long)]
        from: Option<String>,
        /// Output directory (for tree formats like rq).
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Detect and summarize an input without writing anything.
    Inspect {
        /// A curl command string, or a path to an input file.
        input: String,
    },
    /// List supported formats.
    Formats,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(4)
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    match Cli::parse().command {
        Command::Convert {
            input,
            to,
            from,
            output,
        } => convert(&input, &to, from.as_deref(), &output),
        Command::Inspect { input } => {
            let (src, text) = resolve_input(&input)?;
            let detected = detect_source(&text);
            println!("format: {}", detected.unwrap_or("unknown"));
            println!("source: {src}");
            println!("bytes:  {}", text.len());
            Ok(ExitCode::SUCCESS)
        }
        Command::Formats => {
            println!("cross-q supported conversions:\n");
            println!("  IMPORT   curl        ✅");
            println!("  EXPORT   rq          ✅   (Requestly LOCAL_FS 1.12.0)");
            println!("\n  more importers/exporters land per the roadmap; unsupported");
            println!("  targets fail with a clear not_implemented error, never a partial write.");
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Where the input came from, for the human summary.
fn resolve_input(input: &str) -> anyhow::Result<(String, String)> {
    let p = Path::new(input);
    if p.is_file() {
        let text = fs::read_to_string(p)?;
        Ok((format!("file {}", p.display()), text))
    } else {
        Ok(("inline argument".to_string(), input.to_string()))
    }
}

fn detect_source(text: &str) -> Option<&'static str> {
    let t = text.trim_start();
    if t == "curl" || t.starts_with("curl ") || t.starts_with("curl\t") || t.starts_with("curl\\") {
        Some("curl")
    } else {
        None
    }
}

fn convert(input: &str, to: &str, from: Option<&str>, output: &Path) -> anyhow::Result<ExitCode> {
    let (_src, text) = resolve_input(input)?;

    let source = from
        .map(str::to_string)
        .or_else(|| detect_source(&text).map(str::to_string));
    let source = source.ok_or_else(|| {
        anyhow::anyhow!("could not detect source format; pass --from (supported: curl)")
    })?;

    if to != "rq" {
        anyhow::bail!("not_implemented: target format {to:?} (supported: rq)");
    }
    if source != "curl" {
        anyhow::bail!("not_implemented: source format {source:?} (supported: curl)");
    }

    let report = cross_q::convert_curl_to_rq(&text, output)?;

    // Machine-readable report alongside the output.
    let report_dir = output.join(".cross-q");
    fs::create_dir_all(&report_dir)?;
    let report_path = report_dir.join("report.json");
    fs::write(&report_path, serde_json::to_string_pretty(&report)? + "\n")?;

    println!("✓ converted curl → rq  ({})", report.summary());
    println!("  output: {}", output.display());
    println!("  report: {}", report_path.display());

    let diagnostics = report.count(cq_report::Severity::Coerced)
        + report.count(cq_report::Severity::Dropped)
        + report.count(cq_report::Severity::Error);
    if report.has_errors() {
        Ok(ExitCode::from(4))
    } else if diagnostics > 0 {
        Ok(ExitCode::from(2))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}
