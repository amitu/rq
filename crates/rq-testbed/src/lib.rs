//! A small HTTP API to point `rq` at.
//!
//! Every example in the docs so far talked about `/auth/login` and `/me` as though they
//! existed. They didn't — they were canned strings in a test. This is the real thing: a
//! server you can run (`cargo run -p rq-testbed`) and a project you can run against it
//! (`examples/testbed/`), so the whole surface — auth, cookies, chaining, form and
//! multipart bodies, redirects, timeouts, content types — is exercised against a socket
//! rather than described.
//!
//! **Deliberately dependency-free and deliberately dumb.** `std::net` and `serde_json`,
//! nothing else: a test backend that dragged in an async runtime would cost more to build
//! in CI than the thing it tests. It is single-purpose, answers are deterministic so tests
//! can assert exact values, and it binds to loopback only.
//!
//! Routing is a pure function ([`route`]) over a parsed [`Request`], so most of it is
//! testable without opening a socket at all.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

/// The token `POST /auth/login` hands out. Fixed, so a test can assert the exact string
/// that reached the next request instead of matching a pattern.
pub const TOKEN: &str = "tok-abc123";
/// The session cookie it sets alongside the token.
pub const SESSION: &str = "sess-xyz789";
pub const USER: &str = "amitu";
pub const PASSWORD: &str = "hunter2";
pub const API_KEY: &str = "key-9f8e7d";

// ---------------------------------------------------------------------------------------
// The parsed request / reply
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Request {
    pub method: String,
    /// Path with the query string removed, percent-decoded.
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    /// The cookies the client sent, in order.
    pub fn cookies(&self) -> Vec<(String, String)> {
        self.header("cookie")
            .map(|raw| {
                raw.split(';')
                    .filter_map(|pair| pair.split_once('='))
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(status: u16, value: Value) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: serde_json::to_vec_pretty(&value).unwrap_or_default(),
        }
    }

    pub fn text(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type".into(), content_type.into())],
            body: body.into(),
        }
    }

    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Reply {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    fn reason(&self) -> &'static str {
        match self.status {
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            301 => "Moved Permanently",
            302 => "Found",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            418 => "I'm a teapot",
            422 => "Unprocessable Entity",
            429 => "Too Many Requests",
            500 => "Internal Server Error",
            503 => "Service Unavailable",
            _ => "Unknown",
        }
    }
}

// ---------------------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------------------

/// Answer one request. A pure function of the request — no state, no clock, no randomness,
/// so a test asserting on the body gets the same bytes every time.
pub fn route(req: &Request) -> Reply {
    let segments: Vec<&str> = req.path.trim_matches('/').split('/').collect();

    match (req.method.as_str(), segments.as_slice()) {
        (_, ["health"]) => Reply::json(200, json!({ "ok": true, "service": "rq-testbed" })),

        // --- the chaining story ---------------------------------------------------------
        ("POST", ["auth", "login"]) => login(req),
        ("GET", ["me"]) => me(req),
        ("POST", ["auth", "logout"]) => Reply::json(200, json!({ "ok": true }))
            .with_header("Set-Cookie", "session=; Path=/; Max-Age=0"),

        // --- something worth rendering --------------------------------------------------
        ("GET", ["issues"]) => issues(req),
        ("GET", ["issues", number]) => match number.parse::<u32>() {
            Ok(n) if n < 100 => Reply::json(
                200,
                json!({ "number": n, "title": format!("Issue {n}"), "state": "open" }),
            ),
            _ => Reply::json(404, json!({ "error": "no such issue" })),
        },

        // --- the assertion workhorse ----------------------------------------------------
        (_, ["echo"]) => echo(req),

        // --- auth schemes ---------------------------------------------------------------
        ("GET", ["basic-auth"]) => basic_auth(req),
        ("GET", ["api-key"]) => api_key(req),

        // --- bodies ---------------------------------------------------------------------
        ("POST", ["upload"]) => upload(req),
        ("POST", ["form"]) => form(req),

        // --- shapes the client has to handle --------------------------------------------
        ("GET", ["status", code]) => match code.parse::<u16>() {
            Ok(status) if (100..600).contains(&status) => {
                Reply::json(status, json!({ "status": status }))
            }
            _ => Reply::json(400, json!({ "error": "status must be 100-599" })),
        },
        ("GET", ["delay", ms]) => {
            // Capped: a testbed that could be told to sleep for an hour is a way to wedge
            // your own CI.
            let ms = ms.parse::<u64>().unwrap_or(0).min(10_000);
            thread::sleep(Duration::from_millis(ms));
            Reply::json(200, json!({ "delayed_ms": ms }))
        }
        ("GET", ["redirect", n]) => {
            let n = n.parse::<u32>().unwrap_or(0);
            if n == 0 {
                Reply::json(200, json!({ "redirected": true }))
            } else {
                Reply::json(302, json!({ "next": n - 1 }))
                    .with_header("Location", format!("/redirect/{}", n - 1))
            }
        }
        ("GET", ["cookies"]) => {
            let cookies: serde_json::Map<String, Value> = req
                .cookies()
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect();
            Reply::json(200, json!({ "cookies": cookies }))
        }
        ("GET", ["cookies", "set"]) => {
            let mut reply = Reply::json(200, json!({ "set": req.query.len() }));
            for (k, v) in &req.query {
                reply = reply.with_header("Set-Cookie", format!("{k}={v}; Path=/"));
            }
            reply
        }
        ("GET", ["xml"]) => Reply::text(
            200,
            "application/xml",
            "<?xml version=\"1.0\"?>\n<user><name>Amit</name></user>",
        ),
        ("GET", ["text"]) => Reply::text(200, "text/plain", "just text, no json in sight"),
        ("GET", ["html"]) => Reply::text(200, "text/html", "<h1>hello</h1>"),
        ("GET", ["bytes"]) => {
            let n = req
                .param("n")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(16)
                .min(1_000_000);
            Reply {
                status: 200,
                headers: vec![("Content-Type".into(), "application/octet-stream".into())],
                body: (0..n).map(|i| (i % 256) as u8).collect(),
            }
        }

        _ => Reply::json(
            404,
            json!({ "error": "no such route", "path": req.path, "method": req.method }),
        ),
    }
}

fn login(req: &Request) -> Reply {
    let parsed: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
    let user = parsed.get("user").and_then(Value::as_str).unwrap_or("");
    let password = parsed.get("pass").and_then(Value::as_str).unwrap_or("");

    if user != USER || password != PASSWORD {
        return Reply::json(
            401,
            json!({ "error": "bad credentials", "hint": "user=amitu pass=hunter2" }),
        );
    }
    Reply::json(
        200,
        json!({ "access_token": TOKEN, "token_type": "Bearer", "expires_in": 3600 }),
    )
    .with_header("Set-Cookie", format!("session={SESSION}; Path=/; HttpOnly"))
}

/// Accepts either credential — the bearer token a script or `capture:` carried forward, or
/// the session cookie the jar picked up on its own. Both paths matter to `rq`.
fn me(req: &Request) -> Reply {
    let bearer = req
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let session = req
        .cookies()
        .into_iter()
        .find(|(k, _)| k == "session")
        .map(|(_, v)| v);

    let via = match (bearer, session.as_deref()) {
        (Some(TOKEN), _) => "bearer",
        (_, Some(SESSION)) => "cookie",
        _ => {
            return Reply::json(401, json!({ "error": "not authenticated" }))
                .with_header("WWW-Authenticate", "Bearer")
        }
    };

    Reply::json(
        200,
        json!({
            "name": "Amit Upadhyay",
            "email": "amitu@example.com",
            "plan": "pro",
            "joined_at": "2024-08-12T09:30:00Z",
            "authenticated_via": via,
        }),
    )
}

fn issues(req: &Request) -> Reply {
    let per_page = req
        .param("per_page")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(5)
        .clamp(1, 50);
    let state = req.param("state").unwrap_or("open");

    let titles = [
        ("feat: shell-mode improvements", "kevinhq", 3),
        ("bug: command palette flickers", "lainamai", 8),
        ("docs: add quickstart for windows", "asknitin", 1),
        ("feat: support custom keybindings", "grellie", 4),
        ("bug: regex search escape handling", "lainamai", 0),
        ("perf: lazy-load the sidebar", "kevinhq", 2),
    ];
    let items: Vec<Value> = titles
        .iter()
        .cycle()
        .take(per_page)
        .enumerate()
        .map(|(i, (title, login, comments))| {
            json!({
                "number": 1287 - i as u32,
                "title": title,
                "state": state,
                "comments": comments,
                "user": { "login": login },
                "created_at": "2026-08-11T10:15:00Z",
            })
        })
        .collect();
    Reply::json(200, Value::Array(items))
}

/// Mirrors what arrived. This is what an end-to-end test asserts against when the question
/// is "did the client actually send what it said it sent?".
fn echo(req: &Request) -> Reply {
    Reply::json(
        200,
        json!({
            "method": req.method,
            "path": req.path,
            "query": req.query.iter().map(|(k, v)| json!({ "key": k, "value": v })).collect::<Vec<_>>(),
            "headers": req.headers.iter().map(|(k, v)| (k.to_lowercase(), Value::String(v.clone())))
                .collect::<serde_json::Map<String, Value>>(),
            "body": req.body_text(),
            "body_bytes": req.body.len(),
        }),
    )
}

fn basic_auth(req: &Request) -> Reply {
    // The expected header for USER:PASSWORD, precomputed so the testbed needs no base64.
    const EXPECTED: &str = "Basic YW1pdHU6aHVudGVyMg==";
    match req.header("authorization") {
        Some(EXPECTED) => Reply::json(200, json!({ "authenticated": true, "user": USER })),
        _ => Reply::json(401, json!({ "error": "bad or missing basic auth" }))
            .with_header("WWW-Authenticate", "Basic realm=\"rq-testbed\""),
    }
}

fn api_key(req: &Request) -> Reply {
    let from_header = req.header("x-api-key");
    let from_query = req.param("api_key");
    if from_header == Some(API_KEY) || from_query == Some(API_KEY) {
        Reply::json(
            200,
            json!({ "authenticated": true, "via": if from_header.is_some() { "header" } else { "query" } }),
        )
    } else {
        Reply::json(401, json!({ "error": "bad or missing api key" }))
    }
}

/// Reports the parts of a multipart body without pulling a parser in: names, whether each
/// part was a file, and how big it was. Enough to prove the client built it correctly.
fn upload(req: &Request) -> Reply {
    let boundary = req
        .header("content-type")
        .and_then(|ct| ct.split("boundary=").nth(1))
        .map(|b| b.trim().to_string());
    let Some(boundary) = boundary else {
        return Reply::json(400, json!({ "error": "not a multipart body" }));
    };

    let text = req.body_text();
    let parts: Vec<Value> = text
        .split(&format!("--{boundary}"))
        .filter(|chunk| chunk.contains("Content-Disposition"))
        .map(|chunk| {
            let name = between(chunk, "name=\"", "\"").unwrap_or_default();
            let filename = between(chunk, "filename=\"", "\"");
            let body = chunk
                .split_once("\r\n\r\n")
                .map(|(_, b)| b.trim_end_matches("\r\n").to_string())
                .unwrap_or_default();
            json!({
                "name": name,
                "filename": filename,
                "size": body.len(),
                "value": if filename.is_some() { Value::Null } else { Value::String(body) },
            })
        })
        .collect();

    Reply::json(200, json!({ "parts": parts, "count": parts.len() }))
}

fn form(req: &Request) -> Reply {
    let text = req.body_text();
    let fields: serde_json::Map<String, Value> = text
        .split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (percent_decode(k), Value::String(percent_decode(v))))
        .collect();
    Reply::json(200, json!({ "form": fields }))
}

fn between(haystack: &str, start: &str, end: &str) -> Option<String> {
    let from = haystack.find(start)? + start.len();
    let rest = &haystack[from..];
    let to = rest.find(end)?;
    Some(rest[..to].to_string())
}

// ---------------------------------------------------------------------------------------
// The server
// ---------------------------------------------------------------------------------------

/// A running testbed. Dropping it stops the server.
pub struct Server {
    /// e.g. `http://127.0.0.1:53412` — hand this to `rq --var host=…`.
    pub base_url: String,
    running: Arc<AtomicBool>,
    port: u16,
}

impl Server {
    /// Start on `port` (0 = any free port), on loopback only.
    pub fn start(port: u16) -> std::io::Result<Server> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let port = listener.local_addr()?.port();
        let running = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&running);

        thread::spawn(move || {
            for stream in listener.incoming() {
                if !flag.load(Ordering::Relaxed) {
                    break;
                }
                match stream {
                    Ok(stream) => {
                        // A thread per connection: at this scale it is the simplest thing
                        // that cannot deadlock a test.
                        thread::spawn(move || {
                            let _ = serve(stream);
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Server {
            base_url: format!("http://127.0.0.1:{port}"),
            running,
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        // Unblock the accept loop so the thread notices.
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn serve(mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let target = parts.next().unwrap_or("/").to_string();

    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((k, v)) = header.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    let length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    if length > 0 {
        reader.read_exact(&mut body)?;
    }

    let (path, query) = split_target(&target);
    let request = Request {
        method,
        path,
        query,
        headers,
        body,
    };

    let reply = route(&request);
    let mut head = format!("HTTP/1.1 {} {}\r\n", reply.status, reply.reason());
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
    head.push_str("Connection: close\r\n\r\n");

    stream.write_all(head.as_bytes())?;
    stream.write_all(&reply.body)?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Write);
    Ok(())
}

fn split_target(target: &str) -> (String, Vec<(String, String)>) {
    let (path, raw_query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };
    let query = raw_query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect();
    (percent_decode(path), query)
}

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

/// Every route, for `--help` and the README.
pub const ROUTES: &[(&str, &str)] = &[
    ("GET  /health", "liveness"),
    (
        "POST /auth/login",
        "user+pass → token, and a session cookie",
    ),
    ("GET  /me", "needs the bearer token or the session cookie"),
    ("POST /auth/logout", "clears the session cookie"),
    ("GET  /issues?state=&per_page=", "a list worth rendering"),
    ("GET  /issues/:number", "one of them, or a 404"),
    ("ANY  /echo", "mirrors method, query, headers and body"),
    ("GET  /basic-auth", "401 unless Basic amitu:hunter2"),
    ("GET  /api-key", "X-Api-Key header or ?api_key="),
    ("POST /upload", "multipart: reports each part"),
    ("POST /form", "urlencoded: echoes the fields"),
    ("GET  /status/:code", "answers with that status"),
    ("GET  /delay/:ms", "sleeps first (capped at 10s)"),
    ("GET  /redirect/:n", "n chained 302s"),
    ("GET  /cookies", "the cookies you sent"),
    ("GET  /cookies/set?a=b", "sets them"),
    ("GET  /xml · /text · /html", "other content types"),
    ("GET  /bytes?n=", "n bytes of binary"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn get(path: &str) -> Reply {
        let (path, query) = split_target(path);
        route(&Request {
            method: "GET".into(),
            path,
            query,
            ..Request::default()
        })
    }

    fn body(reply: &Reply) -> Value {
        serde_json::from_slice(&reply.body).unwrap()
    }

    #[test]
    fn login_needs_the_right_credentials_and_hands_out_both_carriers() {
        let bad = route(&Request {
            method: "POST".into(),
            path: "/auth/login".into(),
            body: br#"{"user":"amitu","pass":"wrong"}"#.to_vec(),
            ..Request::default()
        });
        assert_eq!(bad.status, 401);

        let ok = route(&Request {
            method: "POST".into(),
            path: "/auth/login".into(),
            body: br#"{"user":"amitu","pass":"hunter2"}"#.to_vec(),
            ..Request::default()
        });
        assert_eq!(ok.status, 200);
        assert_eq!(body(&ok)["access_token"], TOKEN);
        assert!(ok
            .headers
            .iter()
            .any(|(k, v)| k == "Set-Cookie" && v.contains(SESSION)));
    }

    #[test]
    fn me_accepts_either_the_token_or_the_cookie_and_says_which() {
        let with_token = route(&Request {
            method: "GET".into(),
            path: "/me".into(),
            headers: vec![("Authorization".into(), format!("Bearer {TOKEN}"))],
            ..Request::default()
        });
        assert_eq!(body(&with_token)["authenticated_via"], "bearer");

        let with_cookie = route(&Request {
            method: "GET".into(),
            path: "/me".into(),
            headers: vec![("Cookie".into(), format!("session={SESSION}"))],
            ..Request::default()
        });
        assert_eq!(body(&with_cookie)["authenticated_via"], "cookie");

        assert_eq!(get("/me").status, 401);
    }

    #[test]
    fn echo_mirrors_what_arrived() {
        let reply = route(&Request {
            method: "POST".into(),
            path: "/echo".into(),
            query: vec![("a".into(), "1".into())],
            headers: vec![("X-Trace".into(), "abc".into())],
            body: b"hello".to_vec(),
        });
        let v = body(&reply);
        assert_eq!(v["method"], "POST");
        assert_eq!(v["query"][0]["key"], "a");
        assert_eq!(v["headers"]["x-trace"], "abc");
        assert_eq!(v["body"], "hello");
    }

    #[test]
    fn the_shapes_a_client_has_to_handle() {
        assert_eq!(get("/status/418").status, 418);
        assert_eq!(get("/status/999").status, 400);
        assert_eq!(get("/redirect/2").status, 302);
        assert_eq!(get("/redirect/0").status, 200);
        assert_eq!(get("/issues/1").status, 200);
        assert_eq!(get("/issues/500").status, 404);
        assert_eq!(get("/nope").status, 404);
        assert_eq!(get("/bytes?n=32").body.len(), 32);
    }

    #[test]
    fn issues_honours_per_page_and_is_clamped() {
        assert_eq!(
            body(&get("/issues?per_page=3")).as_array().unwrap().len(),
            3
        );
        assert_eq!(
            body(&get("/issues?per_page=999")).as_array().unwrap().len(),
            50
        );
    }

    #[test]
    fn form_and_multipart_are_reported_field_by_field() {
        let form = route(&Request {
            method: "POST".into(),
            path: "/form".into(),
            body: b"q=rust+lang&page=2".to_vec(),
            ..Request::default()
        });
        assert_eq!(body(&form)["form"]["q"], "rust lang");
        assert_eq!(body(&form)["form"]["page"], "2");

        let multipart = route(&Request {
            method: "POST".into(),
            path: "/upload".into(),
            headers: vec![(
                "Content-Type".into(),
                "multipart/form-data; boundary=XYZ".into(),
            )],
            body: concat!(
                "--XYZ\r\nContent-Disposition: form-data; name=\"caption\"\r\n\r\nhello\r\n",
                "--XYZ\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"cat.png\"\r\n",
                "Content-Type: application/octet-stream\r\n\r\nBINARY\r\n--XYZ--\r\n"
            )
            .as_bytes()
            .to_vec(),
            query: Vec::new(),
        });
        let v = body(&multipart);
        assert_eq!(v["count"], 2);
        assert_eq!(v["parts"][0]["name"], "caption");
        assert_eq!(v["parts"][0]["value"], "hello");
        assert_eq!(v["parts"][1]["filename"], "cat.png");
        assert_eq!(v["parts"][1]["size"], 6);
    }

    #[test]
    fn auth_schemes_reject_before_they_accept() {
        assert_eq!(get("/basic-auth").status, 401);
        assert_eq!(get("/api-key").status, 401);
        assert_eq!(get(&format!("/api-key?api_key={API_KEY}")).status, 200);

        let header = route(&Request {
            method: "GET".into(),
            path: "/basic-auth".into(),
            headers: vec![("Authorization".into(), "Basic YW1pdHU6aHVudGVyMg==".into())],
            ..Request::default()
        });
        assert_eq!(header.status, 200);
    }

    #[test]
    fn percent_decoding_survives_the_usual_suspects() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("%26%3D"), "&=");
        assert_eq!(percent_decode("plain"), "plain");
        // A stray `%` is data, not a parse error.
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn it_answers_on_a_real_socket() {
        let server = Server::start(0).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains("rq-testbed"), "{response}");
    }
}
