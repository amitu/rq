//! Parse a `curl` command line into the Idealised Model.
//!
//! A focused, dependency-free tokenizer + flag parser. It handles the flags people
//! actually paste (`-X`, `-H`, `-d`/`--data*`, `-u`, `-G`, `-A`, `-b`, `-e`, `--url`),
//! ignores cosmetic ones (`-s`, `-L`, `-k`, `--compressed`, …), and records a
//! `Dropped` diagnostic for any flag it doesn't understand — so nothing is silently lost.

use cq_model::{Auth, Body, HttpRequest, KeyValue, Method, Provenance, SourceFormat, Url};
use cq_report::{Phase, Report};

/// The result of parsing one curl command.
pub struct CurlParse {
    pub request: HttpRequest,
    pub auth: Option<Auth>,
    /// A suggested request name derived from the URL.
    pub name: String,
}

fn prov(locator: &str) -> Provenance {
    Provenance {
        format: SourceFormat::Curl,
        locator: locator.to_string(),
    }
}

/// Split a command line into tokens, honoring single quotes, double quotes (with
/// backslash escapes), backslash line-continuations, and backslash escaping.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                match chars.peek() {
                    Some('\n') => {
                        chars.next(); // line continuation: drop the backslash+newline
                    }
                    Some(&next) => {
                        cur.push(next);
                        chars.next();
                        in_token = true;
                    }
                    None => {}
                }
            }
            '\'' => {
                in_token = true;
                for c2 in chars.by_ref() {
                    if c2 == '\'' {
                        break;
                    }
                    cur.push(c2);
                }
            }
            '"' => {
                in_token = true;
                while let Some(c2) = chars.next() {
                    if c2 == '"' {
                        break;
                    }
                    if c2 == '\\' {
                        match chars.peek() {
                            Some(&n) if matches!(n, '"' | '\\' | '$' | '`') => {
                                cur.push(n);
                                chars.next();
                            }
                            Some('\n') => {
                                chars.next();
                            }
                            _ => cur.push('\\'),
                        }
                    } else {
                        cur.push(c2);
                    }
                }
            }
            c if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            c => {
                cur.push(c);
                in_token = true;
            }
        }
    }
    if in_token {
        tokens.push(cur);
    }
    tokens
}

/// Boolean (no-argument) flags we accept and ignore.
fn is_bool_flag(tok: &str) -> bool {
    matches!(
        tok,
        "-s" | "--silent"
            | "-S"
            | "--show-error"
            | "-L"
            | "--location"
            | "-k"
            | "--insecure"
            | "-i"
            | "--include"
            | "-v"
            | "--verbose"
            | "-f"
            | "--fail"
            | "-O"
            | "--remote-name"
            | "-#"
            | "--progress-bar"
            | "--compressed"
    )
}

fn suggest_name(url: &str) -> String {
    // Last non-empty path segment, else host, else "request".
    let no_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let after_host = no_scheme.split_once('/').map(|x| x.1).unwrap_or("");
    let path = after_host.split(['?', '#']).next().unwrap_or("");
    let seg = path
        .rsplit('/')
        .find(|s| !s.is_empty() && !s.contains("{{"));
    if let Some(s) = seg {
        return s.to_string();
    }
    let host = no_scheme.split('/').next().unwrap_or("");
    let host = host.split(['?', '#', ':']).next().unwrap_or(host);
    if host.is_empty() {
        "request".to_string()
    } else {
        host.to_string()
    }
}

/// Parse a curl command string into an [`HttpRequest`] (+ optional auth), recording
/// diagnostics for anything unmapped.
pub fn parse_curl(input: &str, report: &mut Report) -> Result<CurlParse, String> {
    // Normalize `--flag=value` into two tokens.
    let mut norm: Vec<String> = Vec::new();
    for t in tokenize(input) {
        if t.starts_with("--") {
            if let Some(eq) = t.find('=') {
                norm.push(t[..eq].to_string());
                norm.push(t[eq + 1..].to_string());
                continue;
            }
        }
        norm.push(t);
    }
    // Drop a leading `curl` (or a path ending in /curl).
    if norm
        .first()
        .is_some_and(|t| t == "curl" || t.ends_with("/curl"))
    {
        norm.remove(0);
    }

    let mut method: Option<Method> = None;
    let mut url: Option<String> = None;
    let mut headers: Vec<KeyValue> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let mut get_with_data = false;
    let mut auth: Option<Auth> = None;

    let mut i = 0;
    while i < norm.len() {
        let tok = norm[i].clone();

        // Resolve a short flag's value: attached (`-Xvalue`) or the next token.
        let take_short = |i: &mut usize, prefix_len: usize| -> Option<String> {
            if tok.len() > prefix_len {
                Some(tok[prefix_len..].to_string())
            } else {
                *i += 1;
                norm.get(*i).cloned()
            }
        };

        if tok == "-G" || tok == "--get" {
            get_with_data = true;
        } else if tok == "-X" || tok == "--request" || tok.starts_with("-X") {
            let v = if tok == "--request" {
                i += 1;
                norm.get(i).cloned()
            } else {
                take_short(&mut i, 2)
            };
            if let Some(m) = v {
                method = Some(Method::from(m));
            }
        } else if tok == "-H" || tok == "--header" || tok.starts_with("-H") {
            let v = if tok == "--header" {
                i += 1;
                norm.get(i).cloned()
            } else {
                take_short(&mut i, 2)
            };
            if let Some(h) = v {
                if let Some((k, val)) = h.split_once(':') {
                    headers.push(KeyValue::new(k.trim(), val.trim()));
                } else {
                    report.dropped(
                        Phase::Parse,
                        prov("-H"),
                        format!("header without a colon ignored: {h:?}"),
                    );
                }
            }
        } else if tok == "-d"
            || tok == "--data"
            || tok == "--data-raw"
            || tok == "--data-binary"
            || tok == "--data-ascii"
            || tok == "--data-urlencode"
            || tok.starts_with("-d")
        {
            let v = if tok.starts_with("--") {
                i += 1;
                norm.get(i).cloned()
            } else {
                take_short(&mut i, 2)
            };
            if let Some(d) = v {
                data.push(d);
            }
        } else if tok == "-u" || tok == "--user" || tok.starts_with("-u") {
            let v = if tok == "--user" {
                i += 1;
                norm.get(i).cloned()
            } else {
                take_short(&mut i, 2)
            };
            if let Some(cred) = v {
                let (user, pass) = cred.split_once(':').unwrap_or((cred.as_str(), ""));
                auth = Some(Auth::Basic {
                    username: user.to_string(),
                    password: pass.to_string(),
                });
            }
        } else if tok == "-A" || tok == "--user-agent" {
            i += 1;
            if let Some(ua) = norm.get(i).cloned() {
                headers.push(KeyValue::new("User-Agent", ua));
            }
        } else if tok == "-b" || tok == "--cookie" {
            i += 1;
            if let Some(c) = norm.get(i).cloned() {
                headers.push(KeyValue::new("Cookie", c));
            }
        } else if tok == "-e" || tok == "--referer" {
            i += 1;
            if let Some(r) = norm.get(i).cloned() {
                headers.push(KeyValue::new("Referer", r));
            }
        } else if tok == "--url" {
            i += 1;
            url = norm.get(i).cloned();
        } else if tok == "-o" || tok == "--output" {
            i += 1; // consume + ignore the output filename
        } else if is_bool_flag(&tok) {
            // accepted, no effect on the request
        } else if tok.starts_with('-') && tok != "-" {
            report.dropped(
                Phase::Parse,
                prov(&tok),
                format!("unsupported curl flag ignored: {tok}"),
            );
        } else {
            // A positional argument: the URL.
            if url.is_none() {
                url = Some(tok);
            } else {
                report.dropped(
                    Phase::Parse,
                    prov("url"),
                    format!("extra positional argument ignored: {tok:?}"),
                );
            }
        }
        i += 1;
    }

    let url = url.ok_or_else(|| "no URL found in curl command".to_string())?;

    // Content-Type from an explicit header, if any (case-insensitive).
    let content_type = headers
        .iter()
        .find(|h| h.key.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.clone());

    // Resolve method: explicit wins; else POST when there's a body; else GET.
    let method = method.unwrap_or(if !data.is_empty() && !get_with_data {
        Method::Post
    } else {
        Method::Get
    });

    let mut query: Vec<KeyValue> = Vec::new();
    let mut body: Option<Body> = None;
    if !data.is_empty() {
        let joined = data.join("&");
        if get_with_data {
            // -G: data becomes query params.
            for pair in joined.split('&') {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                query.push(KeyValue::new(k, v));
            }
        } else {
            body = Some(Body::Raw {
                text: joined,
                media_type: content_type
                    .unwrap_or_else(|| "application/x-www-form-urlencoded".to_string()),
            });
        }
    }

    let name = suggest_name(&url);
    let request = HttpRequest {
        method,
        url: Url::raw(url),
        headers,
        query,
        path_variables: Vec::new(),
        body,
        settings: cq_model::RequestSettings::default(),
    };

    Ok(CurlParse {
        request,
        auth,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_report::Fidelity;

    fn parse(s: &str) -> (CurlParse, Report) {
        let mut r = Report::new(Fidelity::Lossless);
        let p = parse_curl(s, &mut r).expect("parse");
        (p, r)
    }

    #[test]
    fn simple_get() {
        let (p, _) = parse("curl https://api.example.com/users");
        assert_eq!(p.request.method, Method::Get);
        assert_eq!(p.request.url.raw, "https://api.example.com/users");
        assert_eq!(p.name, "users");
    }

    #[test]
    fn headers_and_bearer_style() {
        let (p, _) = parse(
            "curl -H 'Accept: application/vnd.github+json' -H \"Authorization: Bearer {{TOKEN}}\" https://api.github.com/repos/a/b/issues",
        );
        assert_eq!(p.request.headers.len(), 2);
        assert_eq!(p.request.headers[0].key, "Accept");
        assert_eq!(p.request.headers[1].value, "Bearer {{TOKEN}}");
        assert_eq!(p.name, "issues");
    }

    #[test]
    fn post_with_data_defaults_to_post() {
        let (p, _) = parse(
            "curl -X POST https://api.example.com/login -H 'Content-Type: application/json' -d '{\"u\":\"a\"}'",
        );
        assert_eq!(p.request.method, Method::Post);
        match p.request.body.as_ref().unwrap() {
            Body::Raw { text, media_type } => {
                assert_eq!(text, "{\"u\":\"a\"}");
                assert_eq!(media_type, "application/json");
            }
            other => panic!("expected raw body, got {other:?}"),
        }
    }

    #[test]
    fn data_without_method_is_post() {
        let (p, _) = parse("curl https://x.test/submit -d name=amit -d project=rq");
        assert_eq!(p.request.method, Method::Post);
        match p.request.body.unwrap() {
            Body::Raw { text, media_type } => {
                assert_eq!(text, "name=amit&project=rq");
                assert_eq!(media_type, "application/x-www-form-urlencoded");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn get_flag_turns_data_into_query() {
        let (p, _) = parse("curl -G https://x.test/search -d q=rust -d n=5");
        assert_eq!(p.request.method, Method::Get);
        assert!(p.request.body.is_none());
        assert_eq!(p.request.query.len(), 2);
        assert_eq!(p.request.query[0].key, "q");
        assert_eq!(p.request.query[0].value, "rust");
    }

    #[test]
    fn basic_auth_from_user_flag() {
        let (p, _) = parse("curl -u alice:s3cret https://x.test/");
        match p.auth.unwrap() {
            Auth::Basic { username, password } => {
                assert_eq!(username, "alice");
                assert_eq!(password, "s3cret");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn unknown_flag_is_dropped_not_fatal() {
        let (p, r) = parse("curl --fancy-nonexistent https://x.test/ok --compressed -s");
        assert_eq!(p.request.url.raw, "https://x.test/ok");
        // --compressed and -s are accepted silently; only the unknown flag is dropped.
        assert_eq!(r.count(cq_report::Severity::Dropped), 1);
    }

    #[test]
    fn line_continuations_and_attached_flags() {
        let (p, _) = parse("curl -XPUT \\\n  -H'X-A: 1' \\\n  https://x.test/thing");
        assert_eq!(p.request.method, Method::Put);
        assert_eq!(p.request.headers[0].key, "X-A");
        assert_eq!(p.name, "thing");
    }

    #[test]
    fn missing_url_is_an_error() {
        let mut r = Report::new(Fidelity::Lossless);
        assert!(parse_curl("curl -X GET", &mut r).is_err());
    }
}
