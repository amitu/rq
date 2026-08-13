//! Terminal styling. One switch, honoured everywhere: `--color`, `NO_COLOR`, and "is this
//! actually a terminal" all resolve to the same flag before anything prints.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

static COLOR: AtomicBool = AtomicBool::new(false);
static UNICODE: AtomicBool = AtomicBool::new(true);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

pub fn init(choice: ColorChoice) {
    let on = match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        // NO_COLOR is honoured whatever its value — that is the point of the convention.
        ColorChoice::Auto => {
            std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
        }
    };
    COLOR.store(on, Ordering::Relaxed);
}

pub fn set_unicode(on: bool) {
    UNICODE.store(on, Ordering::Relaxed);
}

pub fn color_enabled() -> bool {
    COLOR.load(Ordering::Relaxed)
}

pub fn unicode() -> bool {
    UNICODE.load(Ordering::Relaxed)
}

fn wrap(code: &str, s: &str) -> String {
    if color_enabled() {
        format!("\u{1b}[{code}m{s}\u{1b}[0m")
    } else {
        s.to_string()
    }
}

pub fn bold(s: &str) -> String {
    wrap("1", s)
}
pub fn dim(s: &str) -> String {
    wrap("2", s)
}
pub fn italic(s: &str) -> String {
    wrap("3", s)
}
pub fn underline(s: &str) -> String {
    wrap("4", s)
}
pub fn red(s: &str) -> String {
    wrap("31", s)
}
pub fn green(s: &str) -> String {
    wrap("32", s)
}
pub fn yellow(s: &str) -> String {
    wrap("33", s)
}
pub fn blue(s: &str) -> String {
    wrap("34", s)
}
pub fn magenta(s: &str) -> String {
    wrap("35", s)
}
pub fn cyan(s: &str) -> String {
    wrap("36", s)
}

/// Colour a status code the way a network panel does.
pub fn status(code: u16, text: &str) -> String {
    match code {
        200..=299 => green(text),
        300..=399 => cyan(text),
        400..=499 => yellow(text),
        _ => red(text),
    }
}

pub fn arrow() -> &'static str {
    if unicode() {
        "▸"
    } else {
        ">"
    }
}

pub fn branch() -> &'static str {
    if unicode() {
        "└─"
    } else {
        "\\_"
    }
}

pub fn tee() -> &'static str {
    if unicode() {
        "├──"
    } else {
        "|--"
    }
}

pub fn elbow() -> &'static str {
    if unicode() {
        "└──"
    } else {
        "\\--"
    }
}

pub fn pipe() -> &'static str {
    if unicode() {
        "│  "
    } else {
        "|  "
    }
}

pub fn tick() -> &'static str {
    if unicode() {
        "✓"
    } else {
        "+"
    }
}

pub fn warn_sign() -> &'static str {
    "!"
}

/// Print a non-fatal note to stderr, so piping stdout stays clean.
pub fn note(msg: &str) {
    eprintln!("{} {}", yellow(warn_sign()), dim(msg));
}

/// Visible width, ignoring ANSI escapes. Good enough for aligning table cells: it counts
/// chars, not grapheme clusters, which is honest for the ASCII-plus-accents case and
/// degrades gracefully everywhere else.
pub fn width(s: &str) -> usize {
    let mut n = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c == 'm' {
                in_escape = false;
            }
            continue;
        }
        if c == '\u{1b}' {
            in_escape = true;
            continue;
        }
        n += 1;
    }
    n
}

/// Hide known secret values wherever they appear in printed output.
pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for s in secrets {
        if s.len() >= 4 {
            out = out.replace(s.as_str(), "***");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_ignores_escapes() {
        COLOR.store(true, Ordering::Relaxed);
        assert_eq!(width(&bold("abc")), 3);
        COLOR.store(false, Ordering::Relaxed);
        assert_eq!(bold("abc"), "abc");
    }

    #[test]
    fn redaction_hides_secret_values() {
        let out = redact(
            "Authorization: Bearer s3cret-token",
            &["s3cret-token".into()],
        );
        assert_eq!(out, "Authorization: Bearer ***");
    }

    #[test]
    fn short_secrets_are_not_redacted_into_noise() {
        // A two-character "secret" would blank out half the output; leave it be.
        assert_eq!(redact("a b c", &["b".into()]), "a b c");
    }
}
