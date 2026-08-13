//! End-to-end tests: the real binary, against a real socket.
//!
//! Everything below runs the shipped `rq` against a stub HTTP server, because the failures
//! that matter (a header that never went out, a captured token that never arrived) only
//! show up on the wire.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc;

const BIN: &str = env!("CARGO_BIN_EXE_rq");

/// What the stub saw, so a test can assert on what actually left the process.
#[derive(Debug, Clone)]
struct Received {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A one-thread HTTP server that answers `count` requests from a routing closure.
struct Stub {
    base: String,
    seen: mpsc::Receiver<Received>,
}

impl Stub {
    fn start<F>(count: usize, route: F) -> Stub
    where
        F: Fn(&Received) -> (u16, &'static str, String) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let (tx, seen) = mpsc::channel();

        std::thread::spawn(move || {
            for _ in 0..count {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                if let Some(req) = serve(stream, &route) {
                    let _ = tx.send(req);
                }
            }
        });

        Stub {
            base: format!("http://127.0.0.1:{port}"),
            seen,
        }
    }

    fn next(&self) -> Received {
        self.seen
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the stub server never saw a request")
    }
}

fn serve<F>(mut stream: TcpStream, route: &F) -> Option<Received>
where
    F: Fn(&Received) -> (u16, &'static str, String),
{
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
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
        reader.read_exact(&mut body).ok()?;
    }

    let received = Received {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    };
    let (status, reason, payload) = route(&received);
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(response.as_bytes()).ok()?;
    stream.flush().ok()?;
    Some(received)
}

// --- helpers ------------------------------------------------------------------------------

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let f = Fixture { dir };
        f.rq(&["init"]);
        f
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.root().join("apis").join(rel).join("__metadata.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn write_env(&self, name: &str, contents: &str) {
        let path = self.root().join("environments").join(format!("{name}.md"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
    }

    fn rq(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .arg("--color=never")
            .current_dir(self.root())
            // A predictable, non-interactive environment: no inherited RQ_PROJECT, no
            // colour, no editor surprises.
            .env_remove("RQ_PROJECT")
            .env("NO_COLOR", "1")
            .output()
            .expect("running rq")
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

// --- tests --------------------------------------------------------------------------------

#[test]
fn runs_a_request_and_renders_its_view_as_a_table() {
    let stub = Stub::start(1, |_| {
        (
            200,
            "OK",
            serde_json::json!([
                { "number": 1287, "title": "feat: shell-mode", "user": { "login": "kevinhq" } },
                { "number": 1265, "title": "bug: palette flickers", "user": { "login": "lainamai" } }
            ])
            .to_string(),
        )
    });

    let f = Fixture::new();
    f.write(
        "issues",
        &format!(
            "---\nurl: {}/repos/{{{{owner}}}}/issues\nquery:\n  state: open\n\
             vars:\n  owner: anthropics\n---\n\n\
             -- view --\n\n\
             # {{{{ response | length }}}} open issues in **{{{{ vars.owner }}}}**\n\n\
             | # | Title | Author |\n|---|---|---|\n\
             {{% for i in response %}}| #{{{{ i.number }}}} | {{{{ i.title }}}} | @{{{{ i.user.login }}}} |\n{{% endfor %}}\n",
            stub.base
        ),
    );

    let out = f.rq(&["r", "issues"]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}{}", stderr(&out));

    let seen = stub.next();
    assert_eq!(seen.method, "GET");
    assert_eq!(seen.path, "/repos/anthropics/issues?state=open");

    assert!(text.contains("200 OK"), "{text}");
    assert!(text.contains("2 open issues in anthropics"), "{text}");
    // The table is column-aligned, not raw markdown pipes.
    assert!(
        !text.contains('|'),
        "the rendered table still has pipes:\n{text}"
    );
    let rows: Vec<&str> = text.lines().filter(|l| l.starts_with("#1")).collect();
    assert_eq!(rows.len(), 2, "{text}");
    assert!(
        rows[0].contains("feat: shell-mode") && rows[0].ends_with("@kevinhq"),
        "{text}"
    );
    assert_eq!(
        rows[0].find('@'),
        rows[1].find('@'),
        "the author column is not aligned:\n{text}"
    );
}

#[test]
fn a_parent_runs_first_and_its_capture_feeds_the_child() {
    let stub = Stub::start(2, |req| match req.path.as_str() {
        "/auth/login" => (
            200,
            "OK",
            serde_json::json!({ "access_token": "tok-123" }).to_string(),
        ),
        _ => (
            200,
            "OK",
            serde_json::json!({ "name": "Amit", "plan": "pro" }).to_string(),
        ),
    });

    let f = Fixture::new();
    f.write(
        "login",
        &format!(
            "---\nmethod: POST\nurl: {}/auth/login\n\
             headers:\n  Content-Type: application/json\n\
             capture:\n  token: response.access_token\n---\n\n\
             -- body --\n\n{{\"user\": \"amitu\"}}\n",
            stub.base
        ),
    );
    f.write(
        "me",
        &format!(
            "---\nurl: {}/me\nheaders:\n  Authorization: Bearer {{{{token}}}}\n\
             parents: [login]\n---\n\n-- view --\n\n**{{{{ response.name }}}}** on {{{{ response.plan }}}}\n",
            stub.base
        ),
    );

    let out = f.rq(&["r", "me"]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}{}", stderr(&out));

    let login = stub.next();
    assert_eq!(login.method, "POST");
    assert_eq!(login.body, "{\"user\": \"amitu\"}");
    assert_eq!(login.header("content-type"), Some("application/json"));

    let me = stub.next();
    assert_eq!(me.path, "/me");
    // The whole point: the token captured from login reached the second request.
    assert_eq!(me.header("authorization"), Some("Bearer tok-123"));

    // Both steps are reported, parent first, with the capture shown.
    let login_at = text.find("login").expect("login step missing");
    let me_at = text.find("me ").expect("me step missing");
    assert!(login_at < me_at, "{text}");
    assert!(text.contains("captured token = tok-123"), "{text}");
    assert!(text.contains("Amit on pro"), "{text}");
}

#[test]
fn command_line_variables_beat_the_environment() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write_env(
        "staging",
        "---\nvars:\n  host: https://staging.invalid\n  who: staging-user\n---\n",
    );
    f.rq(&["env", "switch", "staging"]);
    f.write(
        "hello",
        "---\nurl: '{{host}}/hello'\nquery:\n  who: '{{who}}'\n---\n",
    );

    let out = f.rq(&["r", "hello", "--var", &format!("host={}", stub.base)]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    let seen = stub.next();
    // host came from --var, who from the active environment.
    assert_eq!(seen.path, "/hello?who=staging-user");
}

#[test]
fn secrets_are_masked_in_shown_output() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write(
        "secure",
        &format!(
            "---\nurl: {}/x\nheaders:\n  Authorization: Bearer {{{{TOKEN}}}}\n\
             vars:\n  TOKEN: {{ env: RQ_IT_TOKEN, secret: true }}\n---\n",
            stub.base
        ),
    );

    let out = Command::new(BIN)
        .args(["r", "secure", "--show", "request", "--color=never"])
        .current_dir(f.root())
        .env("RQ_IT_TOKEN", "super-secret-value")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let text = stdout(&out);
    assert!(out.status.success(), "{text}{}", stderr(&out));

    // It went out for real…
    let seen = stub.next();
    assert_eq!(
        seen.header("authorization"),
        Some("Bearer super-secret-value")
    );
    // …but never appeared on screen.
    assert!(!text.contains("super-secret-value"), "{text}");
    assert!(text.contains("Bearer ***"), "{text}");
}

#[test]
fn a_failing_status_is_visible_and_opt_in_for_the_exit_code() {
    let stub = Stub::start(2, |_| (404, "Not Found", "{\"error\":\"nope\"}".into()));
    let f = Fixture::new();
    f.write("missing", &format!("---\nurl: {}/nope\n---\n", stub.base));

    let plain = f.rq(&["r", "missing"]);
    assert!(
        stdout(&plain).contains("404 Not Found"),
        "{}",
        stdout(&plain)
    );
    assert_eq!(
        plain.status.code(),
        Some(0),
        "a 404 is a result, not a crash"
    );

    let failing = f.rq(&["r", "missing", "--fail"]);
    assert_eq!(failing.status.code(), Some(1));
}

#[test]
fn an_unrun_script_is_reported_on_every_run() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    f.write(
        "scripted",
        &format!(
            "---\nurl: {}/x\n---\n\n-- post --\n\nrq.test('ok', () => true);\n",
            stub.base
        ),
    );

    let out = f.rq(&["r", "scripted"]);
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("NOT executed"),
        "an unexecuted script must be reported: {}",
        stderr(&out)
    );

    // …and --strict turns that note into a failure.
    let strict = f.rq(&["r", "scripted", "--strict"]);
    assert_eq!(strict.status.code(), Some(2), "{}", stderr(&strict));
}

#[test]
fn an_unresolved_variable_is_reported_not_blanked() {
    let f = Fixture::new();
    f.write("broken", "---\nurl: 'https://x.invalid/{{nope}}'\n---\n");
    let out = f.rq(&["r", "broken"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("unresolved"), "{}", stderr(&out));
}

#[test]
fn list_shows_the_tree_with_methods_and_dependencies() {
    let f = Fixture::new();
    f.write(
        "github/login",
        "---\nmethod: POST\nurl: https://api.test/auth/login\n---\n",
    );
    f.write(
        "github/me",
        "---\nurl: https://api.test/me\nparents: [login]\n---\n",
    );

    let out = f.rq(&["l"]);
    let text = stdout(&out);
    assert!(text.contains("github/"), "{text}");
    assert!(text.contains("login"), "{text}");
    assert!(text.contains("POST"), "{text}");
    assert!(text.contains("← login"), "{text}");
    assert!(text.contains("2 requests across 1 collection"), "{text}");
}

#[test]
fn curl_round_trips_into_a_runnable_request() {
    let stub = Stub::start(1, |_| (200, "OK", "{\"ok\":true}".into()));
    let f = Fixture::new();

    let saved = f.rq(&[
        "curl",
        "--save-as",
        "github/create",
        "--",
        "-X",
        "POST",
        "-H",
        "X-Trace: abc",
        "-d",
        "{\"a\": 1}",
        &format!("{}/things", stub.base),
    ]);
    assert!(
        saved.status.success(),
        "{}{}",
        stdout(&saved),
        stderr(&saved)
    );

    let file = std::fs::read_to_string(f.root().join("apis/github/create/__metadata.md")).unwrap();
    assert!(file.contains("method: POST"), "{file}");
    assert!(file.contains("X-Trace: abc"), "{file}");
    assert!(file.contains("{\"a\": 1}"), "{file}");

    let run = f.rq(&["r", "github/create"]);
    assert!(run.status.success(), "{}{}", stdout(&run), stderr(&run));
    let seen = stub.next();
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.header("x-trace"), Some("abc"));
    assert_eq!(seen.body, "{\"a\": 1}");
}

#[test]
fn a_collection_shares_headers_and_auth_with_its_requests() {
    let stub = Stub::start(1, |_| (200, "OK", "{}".into()));
    let f = Fixture::new();
    std::fs::write(
        {
            let p = f.root().join("apis/acme");
            std::fs::create_dir_all(&p).unwrap();
            p.join("__collection.md")
        },
        "---\nheaders:\n  X-Team: platform\nauth: { type: bearer, token: shared }\n---\n",
    )
    .unwrap();
    f.write("acme/ping", &format!("---\nurl: {}/ping\n---\n", stub.base));

    let out = f.rq(&["r", "acme/ping"]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    let seen = stub.next();
    assert_eq!(seen.header("x-team"), Some("platform"));
    assert_eq!(seen.header("authorization"), Some("Bearer shared"));
}

#[test]
fn without_a_project_the_error_says_what_to_do() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(BIN)
        .args(["l", "--color=never"])
        .current_dir(dir.path())
        .env_remove("RQ_PROJECT")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let text = stderr(&out);
    assert!(text.contains("no rq project found"), "{text}");
    assert!(text.contains("rq init"), "{text}");
}

#[test]
fn import_reads_a_postman_collection_into_runnable_requests() {
    let stub = Stub::start(1, |_| (200, "OK", "{\"ok\":true}".into()));
    let f = Fixture::new();

    let collection = serde_json::json!({
        "info": { "name": "Acme", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
        "item": [{
            "name": "ping",
            "request": {
                "method": "GET",
                "header": [{ "key": "X-Trace", "value": "abc" }],
                "url": { "raw": format!("{}/ping", stub.base) }
            }
        }]
    });
    let path = f.root().join("acme.postman_collection.json");
    std::fs::write(&path, collection.to_string()).unwrap();

    let out = f.rq(&["import", path.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}{}", stderr(&out));
    assert!(text.contains("imported 1 request"), "{text}");

    // It is a real request, not just files on disk.
    let run = f.rq(&["r", "Acme/ping"]);
    assert!(run.status.success(), "{}{}", stdout(&run), stderr(&run));
    assert_eq!(stub.next().header("x-trace"), Some("abc"));
}

#[test]
fn import_reads_another_rq_project_directory() {
    let source = Fixture::new();
    source.write(
        "github/issues",
        "---\nurl: https://api.github.com/issues\nheaders:\n  Accept: application/json\n---\n",
    );

    let target = Fixture::new();
    let out = target.rq(&["import", source.root().to_str().unwrap()]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}{}", stderr(&out));
    assert!(
        text.contains("(rq)"),
        "the source format must be detected: {text}"
    );

    let listed = stdout(&target.rq(&["l"]));
    assert!(listed.contains("issues"), "{listed}");
}
