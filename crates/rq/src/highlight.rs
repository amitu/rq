//! Terminal syntax highlighting for response bodies, via syntect's bundled grammars + theme.
//!
//! JSON is highlighted separately (a hand-rolled colouriser over the already-parsed value); this
//! covers the other text formats a response might carry — HTML, XML, JavaScript, CSS, YAML — keyed
//! off the `Content-Type`. Output is 24-bit ANSI. Like every other colour path it is gated on
//! [`ui::color_enabled`], so piped / `--color never` / `NO_COLOR` output stays plain text.

use std::sync::LazyLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::{as_24_bit_terminal_escaped, LinesWithEndings};

use crate::ui;

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME: LazyLock<Theme> = LazyLock::new(|| {
    let mut themes = ThemeSet::load_defaults().themes;
    themes
        .remove("base16-ocean.dark")
        .expect("syntect ships base16-ocean.dark")
});

/// Highlight `content` for the terminal when its `Content-Type` maps to a known grammar and colour
/// is enabled. Returns `None` when colour is off or the type isn't one we highlight — the caller
/// then prints the body as-is.
pub fn highlight(content: &str, content_type: Option<&str>) -> Option<String> {
    if !ui::color_enabled() {
        return None;
    }
    let syntax = pick_syntax(content_type)?;
    Some(highlight_with(content, syntax))
}

/// The grammar for a `Content-Type`, or `None` for types we leave plain (JSON is handled upstream;
/// `text/plain` and unknowns fall through). Matched on substrings so parameters like
/// `; charset=utf-8` and `+xml`/`+json`-style suffixes are tolerated.
fn pick_syntax(content_type: Option<&str>) -> Option<&'static SyntaxReference> {
    let ct = content_type?.to_ascii_lowercase();
    let ext = if ct.contains("html") {
        "html"
    } else if ct.contains("xml") {
        "xml"
    } else if ct.contains("javascript") || ct.contains("ecmascript") {
        "js"
    } else if ct.contains("css") {
        "css"
    } else if ct.contains("yaml") {
        "yaml"
    } else {
        return None;
    };
    SYNTAXES.find_syntax_by_extension(ext)
}

/// Colourise every line to 24-bit ANSI. Always highlights (no colour gate) so it stays unit-
/// testable; the public entry point does the gating. Foreground only (`false`) — no background
/// fill that would fight the terminal's own theme — with a trailing reset.
fn highlight_with(content: &str, syntax: &SyntaxReference) -> String {
    let mut h = HighlightLines::new(syntax, &THEME);
    let mut out = String::new();
    for line in LinesWithEndings::from(content) {
        match h.highlight_line(line, &SYNTAXES) {
            Ok(ranges) => out.push_str(&as_24_bit_terminal_escaped(&ranges[..], false)),
            Err(_) => out.push_str(line), // never fail the render over a highlight hiccup
        }
    }
    out.push_str("\u{1b}[0m");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn content_type_picks_the_right_grammar() {
        assert_eq!(
            pick_syntax(Some("text/html; charset=utf-8")).map(|s| s.name.as_str()),
            Some("HTML")
        );
        assert_eq!(
            pick_syntax(Some("application/xml")).map(|s| s.name.as_str()),
            Some("XML")
        );
        assert_eq!(
            pick_syntax(Some("text/css")).map(|s| s.name.as_str()),
            Some("CSS")
        );
        assert_eq!(
            pick_syntax(Some("application/javascript")).map(|s| s.name.as_str()),
            Some("JavaScript")
        );
        // JSON is handled upstream (hand-rolled), and unknown types stay plain.
        assert!(pick_syntax(Some("application/json")).is_none());
        assert!(pick_syntax(Some("text/plain")).is_none());
        assert!(pick_syntax(None).is_none());
    }

    #[test]
    fn highlighting_only_adds_colour() {
        let html = "<html>\n  <body>Hello</body>\n</html>\n";
        let syntax = SYNTAXES.find_syntax_by_extension("html").unwrap();
        let out = highlight_with(html, syntax);
        assert!(out.contains('\u{1b}'), "expected ANSI colour codes");
        // The text itself is untouched — highlighting only wraps it in escapes.
        assert_eq!(strip_ansi(&out), html);
    }
}
