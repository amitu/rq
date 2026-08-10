//! `cq` — the cross-q command line.

use std::collections::BTreeMap;
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
        /// A curl command, or a path to an input file or (Bruno) collection directory.
        input: String,
        /// Target format: rq | mapped | postman | bruno.
        #[arg(long)]
        to: String,
        /// Source format override (autodetected if omitted).
        #[arg(long)]
        from: Option<String>,
        /// Output directory (a file is written inside it for single-file formats).
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Detect and summarize an input without writing anything.
    Inspect {
        /// A curl command string, or a path to an input file / directory.
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
            let resolved = resolve_input(&input, None)?;
            println!(
                "format: {}",
                resolved.source.as_deref().unwrap_or("unknown")
            );
            println!("source: {}", resolved.origin);
            println!("bytes:  {}", resolved.payload.len());
            Ok(ExitCode::SUCCESS)
        }
        Command::Formats => {
            println!("cross-q supported conversions:\n");
            println!("  IMPORT   curl        ✅");
            println!("  IMPORT   postman     ✅   (Collection v1.0 / v2.0 / v2.1)");
            println!("  IMPORT   bruno       ✅   (.bru v2 — file or collection directory)");
            println!("  EXPORT   rq          ✅   (Requestly LOCAL_FS 1.12.0)");
            println!(
                "  EXPORT   mapped      ✅   (Requestly MappedItems — the app's import contract)"
            );
            println!("  EXPORT   postman     ✅   (Collection v2.1)");
            println!("  EXPORT   bruno       ✅   (.bru v2 collection directory)");
            println!("\n  any importer composes with any exporter through the idealised model;");
            println!("  unsupported targets fail with a clear not_implemented error, never a");
            println!("  partial write.");
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// A resolved input: where it came from, the payload to hand a parser, and the detected
/// source format (if any). For a Bruno collection directory the payload is the virtual-FS
/// map (JSON) the importer expects.
struct Resolved {
    origin: String,
    payload: String,
    source: Option<String>,
}

/// Resolve `input` (a directory, a file path, or an inline string) into a payload + detected
/// source. `from` overrides detection.
fn resolve_input(input: &str, from: Option<&str>) -> anyhow::Result<Resolved> {
    let p = Path::new(input);
    if p.is_dir() {
        // A directory is a Bruno collection: walk it into the virtual-FS map the importer
        // consumes (the native side of the no-filesystem-in-WASM boundary).
        let map = read_dir_map(p)?;
        anyhow::ensure!(
            map.keys().any(|k| k.ends_with(".bru") || k == "bruno.json"),
            "directory {} has no .bru / bruno.json — not a Bruno collection",
            p.display()
        );
        return Ok(Resolved {
            origin: format!("directory {}", p.display()),
            payload: serde_json::to_string(&map)?,
            source: Some(from.unwrap_or("bruno").to_string()),
        });
    }
    let (origin, payload) = if p.is_file() {
        (format!("file {}", p.display()), fs::read_to_string(p)?)
    } else {
        ("inline argument".to_string(), input.to_string())
    };
    let source = from
        .map(str::to_string)
        .or_else(|| detect_source(&payload, p).map(str::to_string));
    Ok(Resolved {
        origin,
        payload,
        source,
    })
}

/// Read every file under `dir` into a map keyed by path relative to `dir`.
fn read_dir_map(dir: &Path) -> anyhow::Result<BTreeMap<String, String>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, String>) -> anyhow::Result<()> {
        for entry in fs::read_dir(dir)? {
            let p = entry?.path();
            // Skip hidden entries (.git, .cross-q report dir, …) — never part of a collection.
            if p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
            {
                continue;
            }
            if p.is_dir() {
                walk(&p, base, out)?;
            } else if let Ok(content) = fs::read_to_string(&p) {
                let rel = p.strip_prefix(base)?.to_string_lossy().to_string();
                out.insert(rel, content);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

fn detect_source(text: &str, path: &Path) -> Option<&'static str> {
    let t = text.trim_start();
    if t == "curl" || t.starts_with("curl ") || t.starts_with("curl\t") || t.starts_with("curl\\") {
        Some("curl")
    } else if path.extension().and_then(|e| e.to_str()) == Some("bru")
        || (t.starts_with("meta {") && t.contains('}'))
    {
        Some("bruno")
    } else if t.starts_with('{')
        && (t.contains("schema.getpostman.com")
            || t.contains("_postman_id")
            || (t.contains("\"info\"") && t.contains("\"item\"")))
    {
        Some("postman")
    } else {
        None
    }
}

/// A lowercase, hyphenated slug for output filenames.
fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let t = out.trim_matches('-').to_string();
    if t.is_empty() {
        "collection".into()
    } else {
        t
    }
}

fn convert(input: &str, to: &str, from: Option<&str>, output: &Path) -> anyhow::Result<ExitCode> {
    let resolved = resolve_input(input, from)?;
    let source = resolved.source.ok_or_else(|| {
        anyhow::anyhow!(
            "could not detect source format; pass --from (supported: curl, postman, bruno)"
        )
    })?;

    let mut report = cq_report::Report::new(cq_report::Fidelity::Lossless);
    let ws = cross_q::build_workspace(&source, &resolved.payload, &mut report)?;

    let written: PathBuf = match to {
        "rq" => {
            cross_q::emit_rq::emit_rq(&ws, output, &mut report)?;
            output.to_path_buf()
        }
        // The in-memory Requestly MappedItems bundle — the shape the app's importer returns.
        "mapped" => {
            let mapped = cross_q::to_mapped_items(&ws, &mut report);
            fs::create_dir_all(output)?;
            let path = output.join("mapped-items.json");
            fs::write(&path, serde_json::to_string_pretty(&mapped)? + "\n")?;
            path
        }
        // Postman Collection v2.1 — a single JSON file.
        "postman" => {
            let value = cross_q::emit_postman::to_postman(&ws);
            fs::create_dir_all(output)?;
            let name = ws
                .collections
                .first()
                .map(|c| slug(&c.meta.name))
                .unwrap_or_else(|| "collection".into());
            let path = output.join(format!("{name}.postman_collection.json"));
            fs::write(&path, serde_json::to_string_pretty(&value)? + "\n")?;
            path
        }
        // Bruno .bru — a collection directory (virtual-FS map written out as files).
        "bruno" => {
            let map = cross_q::emit_bruno::to_bruno(&ws);
            for (rel, content) in &map {
                let path = output.join(rel);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, content)?;
            }
            output.to_path_buf()
        }
        other => anyhow::bail!(
            "not_implemented: target format {other:?} (supported: rq, mapped, postman, bruno)"
        ),
    };

    // Machine-readable report alongside the output.
    let report_dir = output.join(".cross-q");
    fs::create_dir_all(&report_dir)?;
    let report_path = report_dir.join("report.json");
    fs::write(&report_path, serde_json::to_string_pretty(&report)? + "\n")?;

    println!("✓ converted {source} → {to}  ({})", report.summary());
    println!("  output: {}", written.display());
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
