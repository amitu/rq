//! Shared test scaffolding: a stub HTTP server that records what actually reached the wire.
//!
//! Both suites use it — the CLI tests drive the binary, the seam tests drive the library —
//! because the failures worth catching (a header that never went out, a cookie that never
//! came back) are only visible on the socket.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;

/// What the stub saw, so a test can assert on what actually left the process.
#[derive(Debug, Clone)]
pub struct Received {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Received {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A one-thread HTTP server that answers `count` requests from a routing closure.
pub struct Stub {
    pub base: String,
    seen: mpsc::Receiver<Received>,
}

impl Stub {
    /// Answer `count` requests with `(status, reason, body)`.
    pub fn start<F>(count: usize, route: F) -> Stub
    where
        F: Fn(&Received) -> (u16, &'static str, String) + Send + 'static,
    {
        Stub::start_with_headers(count, move |req| {
            let (status, reason, body) = route(req);
            (status, reason, body, Vec::new())
        })
    }

    /// The same, plus response headers — for the ones the client has to act on
    /// (`Set-Cookie`, `Location`, an unusual `Content-Type`).
    pub fn start_with_headers<F>(count: usize, route: F) -> Stub
    where
        F: Fn(&Received) -> (u16, &'static str, String, Vec<(String, String)>) + Send + 'static,
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

    pub fn next(&self) -> Received {
        self.seen
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the stub server never saw a request")
    }
}

fn serve<F>(mut stream: TcpStream, route: &F) -> Option<Received>
where
    F: Fn(&Received) -> (u16, &'static str, String, Vec<(String, String)>),
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
    let (status, reason, payload, extra) = route(&received);
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n",
        payload.len()
    );
    for (name, value) in &extra {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    let response = format!("{head}\r\n{payload}");
    stream.write_all(response.as_bytes()).ok()?;
    stream.flush().ok()?;
    Some(received)
}
