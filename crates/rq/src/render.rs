//! The `-- view --` section: response in, legible markdown out, rendered for a terminal.
//!
//! This is the feature the rest of the category doesn't have. Postman's visualizers live
//! inside Postman; Newman and Bruno don't template at all. Rendering here means the output
//! of an API call is something you can hand to a person who doesn't have your tooling.

use anyhow::{Context, Result};
use minijinja::{Environment, UndefinedBehavior};

use crate::ui;

/// A followable link found in a rendered view.
///
/// `[label](rq:name?a=b)` points at another request in the same project — that is what
/// makes a view a *page* rather than a report. An ordinary `http(s)` link is rendered but
/// not numbered: following one would mean issuing a request the project never described.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// 1-based, in reading order — what `--follow N` and the console's digit keys take.
    pub number: usize,
    pub label: String,
    /// The part after `rq:` — `name?a=b`.
    pub target: String,
}

/// Rendered markdown, plus the links it offers.
#[derive(Clone, Debug, Default)]
pub struct Rendered {
    pub text: String,
    pub links: Vec<Link>,
}

/// Split `name?a=b&c=d` into the request name and the variables to run it with.
pub fn parse_target(target: &str) -> (String, Vec<(String, String)>) {
    let target = target.trim();
    let (name, query) = match target.split_once('?') {
        Some((n, q)) => (n, q),
        None => (target, ""),
    };
    let vars = query
        .split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), percent_decode(v.trim())))
        .collect();
    (name.trim().to_string(), vars)
}

/// Undo the encoding a template may have produced when it built a link.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(byte) => {
                    out.push(byte);
                    i += 3;
                }
                Err(_) => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Render a `-- view --` template against the run context.
///
/// Undefined names are an **error**, not an empty string: a template that silently renders
/// "# open issues" because the field was renamed is worse than one that says so.
pub fn render_view(template: &str, ctx: &serde_json::Value) -> Result<String> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.add_filter("date", date_filter);
    env.add_template("view", template)
        .context("the -- view -- template could not be parsed")?;
    let tmpl = env.get_template("view")?;
    let value = minijinja::Value::from_serialize(ctx);
    tmpl.render(value)
        .context("the -- view -- template could not be rendered")
}

/// `{{ created_at | date('YYYY-MM-DD') }}` for ISO-8601 input — the shape APIs actually
/// return. Anything else passes through untouched rather than guessing.
fn date_filter(value: String, format: Option<String>) -> String {
    let fmt = format.unwrap_or_else(|| "YYYY-MM-DD".to_string());
    let bytes = value.as_bytes();
    let at = |range: std::ops::Range<usize>| -> Option<&str> {
        if bytes.len() >= range.end && bytes[range.clone()].iter().all(|b| b.is_ascii_digit()) {
            Some(&value[range])
        } else {
            None
        }
    };
    let (Some(y), Some(mo), Some(d)) = (at(0..4), at(5..7), at(8..10)) else {
        return value;
    };
    let (h, mi, s) = (
        at(11..13).unwrap_or("00"),
        at(14..16).unwrap_or("00"),
        at(17..19).unwrap_or("00"),
    );
    fmt.replace("YYYY", y)
        .replace("MM", mo)
        .replace("DD", d)
        .replace("HH", h)
        .replace("mm", mi)
        .replace("ss", s)
}

/// Render markdown for a terminal: headings, emphasis, code, lists, and — the one that
/// matters — aligned tables.
pub fn markdown_to_terminal(md: &str) -> String {
    markdown(md).text
}

/// Render markdown for a terminal and collect the links it offers.
pub fn markdown(md: &str) -> Rendered {
    let mut links: Vec<Link> = Vec::new();
    let text = render(md, &mut links);
    Rendered { text, links }
}

fn render(md: &str, links: &mut Vec<Link>) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        if let Some(rest) = trimmed.strip_prefix("```") {
            let mut block = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                block.push(lines[i]);
                i += 1;
            }
            i += 1;
            if !rest.trim().is_empty() {
                out.push_str(&ui::dim(&format!("  {}\n", rest.trim())));
            }
            for b in block {
                out.push_str(&ui::dim(&format!("  {b}")));
                out.push('\n');
            }
            continue;
        }

        if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 1 {
            let mut rows = Vec::new();
            while i < lines.len() {
                let t = lines[i].trim();
                if !(t.starts_with('|') && t.len() > 1) {
                    break;
                }
                rows.push(t);
                i += 1;
            }
            out.push_str(&render_table(&rows, links));
            continue;
        }

        if let Some(rest) = heading(trimmed) {
            out.push_str(&ui::bold(&inline(rest, links)));
            out.push('\n');
            i += 1;
            continue;
        }

        if is_rule(trimmed) {
            out.push_str(&ui::dim(
                &"─".repeat(if ui::unicode() { 40 } else { 0 }).to_string(),
            ));
            if !ui::unicode() {
                out.push_str(&ui::dim(&"-".repeat(40)));
            }
            out.push('\n');
            i += 1;
            continue;
        }

        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let indent = line.len() - trimmed.len();
            let bullet = if ui::unicode() { "•" } else { "-" };
            out.push_str(&format!(
                "{}{bullet} {}\n",
                " ".repeat(indent),
                inline(rest, links)
            ));
            i += 1;
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("> ") {
            out.push_str(&format!(
                "{} {}\n",
                ui::dim("│"),
                ui::italic(&inline(rest, links))
            ));
            i += 1;
            continue;
        }

        out.push_str(&inline(line, links));
        out.push('\n');
        i += 1;
    }
    out
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ') {
        Some(line[hashes + 1..].trim())
    } else {
        None
    }
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*'))
}

/// A markdown table, column-aligned. The header separator row (`|---|---|`) is consumed,
/// and its colons choose the alignment.
fn render_table(rows: &[&str], links: &mut Vec<Link>) -> String {
    let parse = |row: &str| -> Vec<String> {
        row.trim()
            .trim_start_matches('|')
            .trim_end_matches('|')
            .split('|')
            .map(|c| c.trim().to_string())
            .collect()
    };
    let is_separator = |cells: &[String]| {
        !cells.is_empty()
            && cells
                .iter()
                .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
    };

    let parsed: Vec<Vec<String>> = rows.iter().map(|r| parse(r)).collect();
    let sep_at = parsed.iter().position(|c| is_separator(c));
    let aligns: Vec<Align> = sep_at
        .map(|i| parsed[i].iter().map(|c| Align::parse(c)).collect())
        .unwrap_or_default();

    let body: Vec<(bool, Vec<String>)> = parsed
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != sep_at)
        .map(|(i, cells)| {
            let is_header = sep_at.is_some_and(|s| i < s);
            let rendered = cells
                .iter()
                .map(|c| {
                    let text = inline(c, links);
                    if is_header {
                        ui::bold(&text)
                    } else {
                        text
                    }
                })
                .collect();
            (is_header, rendered)
        })
        .collect();

    let columns = body.iter().map(|(_, c)| c.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; columns];
    for (_, cells) in &body {
        for (i, c) in cells.iter().enumerate() {
            widths[i] = widths[i].max(ui::width(c));
        }
    }

    let mut out = String::new();
    for (is_header, cells) in &body {
        let mut line = String::new();
        for (i, cell) in cells.iter().enumerate() {
            let pad = widths[i].saturating_sub(ui::width(cell));
            let align = aligns.get(i).copied().unwrap_or(Align::Left);
            if i > 0 {
                line.push_str("  ");
            }
            match align {
                Align::Right => {
                    line.push_str(&" ".repeat(pad));
                    line.push_str(cell);
                }
                Align::Center => {
                    let left = pad / 2;
                    line.push_str(&" ".repeat(left));
                    line.push_str(cell);
                    line.push_str(&" ".repeat(pad - left));
                }
                Align::Left => {
                    line.push_str(cell);
                    if i + 1 < cells.len() {
                        line.push_str(&" ".repeat(pad));
                    }
                }
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
        if *is_header {
            let rule: String = widths
                .iter()
                .map(|w| "─".repeat(*w))
                .collect::<Vec<_>>()
                .join("  ");
            let rule = if ui::unicode() {
                rule
            } else {
                widths
                    .iter()
                    .map(|w| "-".repeat(*w))
                    .collect::<Vec<_>>()
                    .join("  ")
            };
            out.push_str(&ui::dim(&rule));
            out.push('\n');
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Right,
    Center,
}

impl Align {
    fn parse(cell: &str) -> Align {
        match (cell.starts_with(':'), cell.ends_with(':')) {
            (true, true) => Align::Center,
            (false, true) => Align::Right,
            _ => Align::Left,
        }
    }
}

/// Inline markdown: `**bold**`, `*italic*`, `` `code` ``, `[text](url)`.
fn inline(text: &str, links: &mut Vec<Link>) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '`' {
            if let Some(end) = find(&chars, i + 1, '`') {
                out.push_str(&ui::cyan(&chars[i + 1..end].iter().collect::<String>()));
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            if let Some(end) = find_pair(&chars, i + 2) {
                out.push_str(&ui::bold(&chars[i + 2..end].iter().collect::<String>()));
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '*' {
            if let Some(end) = find(&chars, i + 1, '*') {
                out.push_str(&ui::italic(&chars[i + 1..end].iter().collect::<String>()));
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some(close) = find(&chars, i + 1, ']') {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(paren) = find(&chars, close + 2, ')') {
                        let label: String = chars[i + 1..close].iter().collect();
                        let url: String = chars[close + 2..paren].iter().collect();
                        match url.trim().strip_prefix("rq:") {
                            // A link into the project: number it, so it can be followed.
                            Some(target) => {
                                links.push(Link {
                                    number: links.len() + 1,
                                    label: label.clone(),
                                    target: target.to_string(),
                                });
                                out.push_str(&ui::underline(&label));
                                out.push_str(&ui::cyan(&format!(" [{}]", links.len())));
                            }
                            None => {
                                out.push_str(&ui::underline(&label));
                                out.push_str(&ui::dim(&format!(" ({url})")));
                            }
                        }
                        i = paren + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn find(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|i| chars[*i] == target)
}

fn find_pair(chars: &[char], from: usize) -> Option<usize> {
    (from..chars.len().saturating_sub(1)).find(|i| chars[*i] == '*' && chars[*i + 1] == '*')
}

/// What to print when a request has no `-- view --`: pretty (and, with colour on, highlighted)
/// JSON, or the body as it came — syntax-highlighted from its `Content-Type` when that maps to a
/// known grammar (HTML/XML/JS/CSS/YAML). With colour off (piped, `--color never`, `NO_COLOR`)
/// JSON is byte-for-byte the plain `to_string_pretty` and everything else is the raw body, so
/// nothing downstream sees escape codes.
pub fn default_body(
    body: &str,
    json: Option<&serde_json::Value>,
    content_type: Option<&str>,
) -> String {
    match json {
        Some(v) if ui::color_enabled() => highlight_json(v),
        Some(v) => serde_json::to_string_pretty(v).unwrap_or_else(|_| body.to_string()),
        None => crate::highlight::highlight(body, content_type).unwrap_or_else(|| body.to_string()),
    }
}

/// Pretty-print a JSON value with 2-space indent (matching `serde_json::to_string_pretty`),
/// colouring by token kind: keys blue, strings green, numbers/bools yellow, null dim; structural
/// punctuation stays plain. Only reached when colour is enabled.
fn highlight_json(v: &serde_json::Value) -> String {
    let mut out = String::new();
    write_json(v, 0, &mut out);
    out
}

fn write_json(v: &serde_json::Value, depth: usize, out: &mut String) {
    use serde_json::Value;
    // A JSON scalar as its exact serialized text (handles string escaping, number formatting).
    let scalar = |x: &Value| serde_json::to_string(x).unwrap_or_default();
    match v {
        Value::Null => out.push_str(&ui::dim("null")),
        Value::Bool(_) | Value::Number(_) => out.push_str(&ui::yellow(&scalar(v))),
        Value::String(_) => out.push_str(&ui::green(&scalar(v))),
        Value::Array(a) if a.is_empty() => out.push_str("[]"),
        Value::Array(a) => {
            out.push_str("[\n");
            for (i, item) in a.iter().enumerate() {
                indent(out, depth + 1);
                write_json(item, depth + 1, out);
                out.push_str(if i + 1 < a.len() { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push(']');
        }
        Value::Object(m) if m.is_empty() => out.push_str("{}"),
        Value::Object(m) => {
            out.push_str("{\n");
            let last = m.len() - 1;
            for (i, (k, val)) in m.iter().enumerate() {
                indent(out, depth + 1);
                out.push_str(&ui::blue(&scalar(&Value::String(k.clone()))));
                out.push_str(": ");
                write_json(val, depth + 1, out);
                out.push_str(if i < last { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push('}');
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_view_template() {
        let ctx = serde_json::json!({ "response": [{"n": 1}, {"n": 2}], "status": 200 });
        let out = render_view("{{ response | length }} rows, {{ status }}", &ctx).unwrap();
        assert_eq!(out, "2 rows, 200");
    }

    #[test]
    fn an_undefined_field_is_an_error_not_a_blank() {
        let ctx = serde_json::json!({ "response": {} });
        let err = render_view("{{ response.nope.deeper }}", &ctx).unwrap_err();
        assert!(err.to_string().contains("could not be rendered"), "{err}");
    }

    #[test]
    fn date_filter_handles_iso_and_passes_through_the_rest() {
        assert_eq!(
            date_filter("2024-08-12T09:30:00Z".into(), None),
            "2024-08-12"
        );
        assert_eq!(
            date_filter(
                "2024-08-12T09:30:00Z".into(),
                Some("DD/MM/YYYY HH:mm".into())
            ),
            "12/08/2024 09:30"
        );
        assert_eq!(date_filter("last tuesday".into(), None), "last tuesday");
    }

    #[test]
    fn aligns_table_columns() {
        let md = "| # | Title |\n|---|---|\n| 1287 | feat: shell mode |\n| 5 | bug |\n";
        let out = markdown_to_terminal(md);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "#     Title");
        assert_eq!(lines[2], "1287  feat: shell mode");
        assert_eq!(lines[3], "5     bug");
    }

    #[test]
    fn renders_headings_lists_and_inline_marks() {
        let out = markdown_to_terminal("# Title\n\n- one `code`\n- **two**\n");
        assert!(out.starts_with("Title\n"), "{out}");
        assert!(out.contains("one code"), "{out}");
        assert!(out.contains("two"), "{out}");
    }

    #[test]
    fn json_highlight_never_alters_the_text() {
        let v = serde_json::json!({
            "name": "amitu", "age": 30, "ok": true, "tags": ["a", "b"], "meta": null,
            "nested": { "x": 1 }, "empty": {}, "none": []
        });
        let plain = serde_json::to_string_pretty(&v).unwrap();
        // Colour is a process-global other tests share, so this asserts the property that holds in
        // EITHER state: stripping any escape codes yields serde's exact pretty print. Highlighting
        // only ever adds colour — it never changes the JSON text, indentation, or key order.
        assert_eq!(strip_ansi(&highlight_json(&v)), plain);
        assert_eq!(
            strip_ansi(&default_body("", Some(&v), Some("application/json"))),
            plain
        );
    }

    /// Drop CSI escape sequences (`ESC [ … m`) so the underlying text can be compared.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}
