//! Building and sending one request. Blocking, one at a time, no runtime — `rq r` is a
//! shell command, not a server.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

/// A request with every `{{template}}` already substituted — what actually goes on the wire.
#[derive(Clone, Debug, Default)]
pub struct Prepared {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Payload>,
    pub timeout_ms: Option<u64>,
    pub follow_redirects: bool,
    pub verify_tls: bool,
}

#[derive(Clone, Debug)]
pub enum Payload {
    /// A single blob — raw text, JSON, XML — with the media type it is sent as.
    Text { text: String, media_type: String },
    /// `application/x-www-form-urlencoded`.
    Form(Vec<(String, String)>),
    /// `multipart/form-data`. A value of `@path` is read from disk as a file part.
    Multipart(Vec<(String, String)>),
    /// A file sent as the whole body.
    File { path: String, media_type: String },
}

impl Payload {
    pub fn describe(&self) -> String {
        match self {
            Payload::Text { media_type, text } => format!("{media_type} ({} bytes)", text.len()),
            Payload::Form(f) => format!("form-urlencoded ({} fields)", f.len()),
            Payload::Multipart(f) => format!("multipart/form-data ({} parts)", f.len()),
            Payload::File { path, .. } => format!("file {path}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub bytes: usize,
    pub elapsed: Duration,
    /// The URL the response actually came from, after any redirects.
    pub final_url: String,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The body parsed as JSON, when it is JSON. Templates and `capture:` read this.
    pub fn json(&self) -> Option<serde_json::Value> {
        let looks_json = self
            .header("content-type")
            .map(|ct| ct.contains("json"))
            .unwrap_or(false);
        let trimmed = self.body.trim_start();
        if !looks_json && !trimmed.starts_with(['{', '[']) {
            return None;
        }
        serde_json::from_str(&self.body).ok()
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Send it. Any HTTP status is a response — only transport failures are errors, because
/// `rq r` showing you a 404 is the whole point.
pub fn send(req: &Prepared) -> Result<Response> {
    let (body, extra_headers) = encode_body(req.body.as_ref())?;

    let mut builder = ureq::config::Config::builder()
        .http_status_as_error(false)
        .max_redirects(if req.follow_redirects { 10 } else { 0 })
        .save_redirect_history(true);
    if let Some(ms) = req.timeout_ms.filter(|ms| *ms > 0) {
        builder = builder.timeout_global(Some(Duration::from_millis(ms)));
    }
    if !req.verify_tls {
        builder = builder.tls_config(
            ureq::tls::TlsConfig::builder()
                .disable_verification(true)
                .build(),
        );
    }
    let agent = ureq::Agent::new_with_config(builder.build());

    let method = req.method.to_ascii_uppercase();
    let mut http_req = ureq::http::Request::builder()
        .method(method.as_str())
        .uri(&req.url);
    for (k, v) in extra_headers
        .iter()
        .filter(|(k, _)| !req.headers.iter().any(|(hk, _)| hk.eq_ignore_ascii_case(k)))
        .chain(req.headers.iter())
    {
        http_req = http_req.header(k.as_str(), v.as_str());
    }
    let http_req = http_req
        .body(body)
        .with_context(|| format!("building the request for {}", req.url))?;

    let started = Instant::now();
    let mut resp = agent.run(http_req).map_err(|e| explain(e, &req.url))?;
    let status = resp.status();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                v.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let final_url = {
        use ureq::ResponseExt;
        resp.get_uri().to_string()
    };
    // 50 MiB: large enough for any sane API response, small enough that a runaway stream
    // fails instead of eating the machine.
    let text = resp
        .body_mut()
        .with_config()
        .limit(50 * 1024 * 1024)
        .read_to_string()
        .unwrap_or_else(|e| format!("<body could not be read as text: {e}>"));
    let elapsed = started.elapsed();

    Ok(Response {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        headers,
        bytes: text.len(),
        body: text,
        elapsed,
        final_url,
    })
}

/// Turn a transport failure into something a human can act on.
fn explain(e: ureq::Error, url: &str) -> anyhow::Error {
    let hint = match &e {
        ureq::Error::ConnectionFailed => "\n  the host refused the connection or DNS failed",
        ureq::Error::Timeout(_) => "\n  raise it with `timeout: <ms>` in the request",
        ureq::Error::Tls(_) => {
            "\n  set `verify_tls: false` in the request if this is a known self-signed host"
        }
        _ => "",
    };
    anyhow::anyhow!("{url}: {e}{hint}")
}

/// A serialized body and the headers it implies.
type Encoded = (Vec<u8>, Vec<(String, String)>);

/// Serialize the payload and produce the headers it implies. Explicit headers on the
/// request always win — this only fills in what wasn't stated.
fn encode_body(payload: Option<&Payload>) -> Result<Encoded> {
    let Some(payload) = payload else {
        return Ok((Vec::new(), Vec::new()));
    };
    match payload {
        Payload::Text { text, media_type } => Ok((
            text.as_bytes().to_vec(),
            vec![("content-type".into(), media_type.clone())],
        )),
        Payload::Form(fields) => {
            let encoded = fields
                .iter()
                .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
                .collect::<Vec<_>>()
                .join("&");
            Ok((
                encoded.into_bytes(),
                vec![(
                    "content-type".into(),
                    "application/x-www-form-urlencoded".into(),
                )],
            ))
        }
        Payload::Multipart(fields) => {
            let boundary = format!("----rq{:x}", nonce());
            let mut out: Vec<u8> = Vec::new();
            for (name, value) in fields {
                out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
                match value.strip_prefix('@') {
                    Some(path) => {
                        let bytes = std::fs::read(path)
                            .with_context(|| format!("reading form file {path}"))?;
                        let filename = std::path::Path::new(path)
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "file".into());
                        out.extend_from_slice(
                            format!(
                                "Content-Disposition: form-data; name=\"{name}\"; \
                                 filename=\"{filename}\"\r\n\
                                 Content-Type: application/octet-stream\r\n\r\n"
                            )
                            .as_bytes(),
                        );
                        out.extend_from_slice(&bytes);
                        out.extend_from_slice(b"\r\n");
                    }
                    None => {
                        out.extend_from_slice(
                            format!(
                                "Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                            )
                            .as_bytes(),
                        );
                    }
                }
            }
            out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
            Ok((
                out,
                vec![(
                    "content-type".into(),
                    format!("multipart/form-data; boundary={boundary}"),
                )],
            ))
        }
        Payload::File { path, media_type } => {
            let bytes = std::fs::read(path).with_context(|| format!("reading body file {path}"))?;
            Ok((bytes, vec![("content-type".into(), media_type.clone())]))
        }
    }
}

/// Percent-encode everything outside the unreserved set. Small enough to own; a dependency
/// for 15 lines of table lookup is not a trade worth making.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Append query parameters to a URL that may already carry some.
pub fn with_query(url: &str, query: &[(String, String)]) -> String {
    if query.is_empty() {
        return url.to_string();
    }
    let encoded = query
        .iter()
        .map(|(k, v)| {
            if v.is_empty() {
                percent_encode(k)
            } else {
                format!("{}={}", percent_encode(k), percent_encode(v))
            }
        })
        .collect::<Vec<_>>()
        .join("&");

    let (base, fragment) = match url.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (url, None),
    };
    let sep = if base.contains('?') {
        if base.ends_with('?') || base.ends_with('&') {
            ""
        } else {
            "&"
        }
    } else {
        "?"
    };
    let mut out = format!("{base}{sep}{encoded}");
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Fill `{key}` and `:key` path placeholders, the two spellings the category uses.
pub fn apply_path_vars(url: &str, path_vars: &[(String, String)]) -> String {
    let mut out = url.to_string();
    for (k, v) in path_vars {
        out = out.replace(&format!("{{{k}}}",), &percent_encode(v));
        out = out.replace(&format!(":{k}"), &percent_encode(v));
    }
    out
}

/// Validate the URL early, with a message that says what to fix.
pub fn check_url(url: &str) -> Result<()> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        bail!("the request has no `url:`");
    }
    if trimmed.contains("{{") {
        bail!("unresolved variable in the url: {trimmed}");
    }
    if !trimmed.contains("://") {
        bail!("`{trimmed}` has no scheme — write https://{trimmed}");
    }
    Ok(())
}

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_query_to_bare_and_existing() {
        let q = vec![("state".to_string(), "open".to_string())];
        assert_eq!(with_query("https://x/y", &q), "https://x/y?state=open");
        assert_eq!(
            with_query("https://x/y?a=1", &q),
            "https://x/y?a=1&state=open"
        );
        assert_eq!(
            with_query("https://x/y#frag", &q),
            "https://x/y?state=open#frag"
        );
        assert_eq!(with_query("https://x/y", &[]), "https://x/y");
    }

    #[test]
    fn encodes_reserved_characters() {
        assert_eq!(percent_encode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(percent_encode("keep-._~"), "keep-._~");
    }

    #[test]
    fn substitutes_both_path_var_spellings() {
        let vars = vec![("id".to_string(), "7 8".to_string())];
        assert_eq!(
            apply_path_vars("https://x/{id}/a", &vars),
            "https://x/7%208/a"
        );
        assert_eq!(
            apply_path_vars("https://x/:id/a", &vars),
            "https://x/7%208/a"
        );
    }

    #[test]
    fn rejects_a_url_with_an_unresolved_variable() {
        let err = check_url("https://x/{{owner}}").unwrap_err().to_string();
        assert!(err.contains("unresolved"), "{err}");
        assert!(check_url("nohost.example")
            .unwrap_err()
            .to_string()
            .contains("scheme"));
    }

    #[test]
    fn encodes_a_form_body_with_its_content_type() {
        let (body, headers) = encode_body(Some(&Payload::Form(vec![
            ("a".into(), "1 2".into()),
            ("b".into(), "&".into()),
        ])))
        .unwrap();
        assert_eq!(String::from_utf8(body).unwrap(), "a=1%202&b=%26");
        assert_eq!(headers[0].1, "application/x-www-form-urlencoded");
    }

    #[test]
    fn multipart_carries_a_boundary_that_matches_the_header() {
        let (body, headers) =
            encode_body(Some(&Payload::Multipart(vec![("a".into(), "1".into())]))).unwrap();
        let boundary = headers[0].1.split("boundary=").nth(1).unwrap();
        let text = String::from_utf8(body).unwrap();
        assert!(text.starts_with(&format!("--{boundary}\r\n")), "{text}");
        assert!(text.ends_with(&format!("--{boundary}--\r\n")), "{text}");
        assert!(text.contains("name=\"a\""));
    }
}
