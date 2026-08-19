//! The cookie jar.
//!
//! A chain like `login → me` usually carries its session in a `Set-Cookie`, not a token in
//! a JSON body, so without a jar half of the real chains in the world don't work.
//!
//! By default the jar lives for one `rq r` invocation and never touches the disk: a terminal
//! client that quietly persisted your session cookies would be storing credentials you did
//! not ask it to store. `--cookies` asks it to — and then the file is the whole interface.
//! There is no `rq cookies list` or `rq cookies clear` because there is nothing to wrap: the
//! path is yours, `cat` reads it and `rm` clears it. Session cookies ARE credentials, so a
//! jar you keep is a secret you keep; `--cookies` with no path puts it under `.rq/`, which is
//! gitignored.
//!
//! Scope, stated plainly: host and path matching, `Secure`, and `Max-Age=0` deletion.
//! `Expires` is not evaluated — a run is seconds long, and a wrong date parser that silently
//! drops a live cookie would be worse than keeping one a moment past its welcome.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// The host it belongs to, lowercased and without a leading dot.
    pub domain: String,
    pub path: String,
    /// `Domain=` was absent: send only to this exact host.
    pub host_only: bool,
    pub secure: bool,
    pub http_only: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Jar {
    cookies: Vec<Cookie>,
}

impl Jar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a jar from `path`. A file that is not there is simply an empty jar — the first
    /// run with `--cookies` has nothing to load, and that is not a problem to report.
    /// A file that is there and unreadable IS reported: silently starting empty would log you
    /// out and look like the server's fault.
    pub fn load(path: &std::path::Path) -> (Jar, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Jar::new(), None),
            Err(e) => {
                return (
                    Jar::new(),
                    Some(format!(
                        "{}: {e}; starting with an empty jar",
                        path.display()
                    )),
                )
            }
        };
        match serde_json::from_str::<Jar>(&text) {
            Ok(jar) => (jar, None),
            Err(e) => (
                Jar::new(),
                Some(format!(
                    "{}: not a cookie jar rq can read ({e}); starting with an empty one",
                    path.display()
                )),
            ),
        }
    }

    /// Write the jar to `path`, creating its directory. Pretty-printed on purpose: the file
    /// is the interface, so it has to be readable by the person who owns it.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into());
        std::fs::write(path, format!("{json}\n"))
    }

    /// How many cookies are held, for the line a run prints when it keeps them.
    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Cookie> {
        self.cookies.iter()
    }

    /// Take every `Set-Cookie` from a response received from `url`.
    pub fn ingest(&mut self, url: &str, headers: &[(String, String)]) {
        let Some(origin) = Origin::parse(url) else {
            return;
        };
        for (name, value) in headers {
            if !name.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            if let Some((cookie, delete)) = parse_set_cookie(value, &origin) {
                self.cookies.retain(|c| {
                    !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
                });
                if !delete {
                    self.cookies.push(cookie);
                }
            }
        }
    }

    /// The `Cookie:` header value for a request to `url`, if any cookie matches.
    ///
    /// More specific paths first, which is the ordering RFC 6265 asks senders for.
    pub fn header_for(&self, url: &str) -> Option<String> {
        let origin = Origin::parse(url)?;
        let mut matching: Vec<&Cookie> =
            self.cookies.iter().filter(|c| c.matches(&origin)).collect();
        if matching.is_empty() {
            return None;
        }
        matching.sort_by_key(|c| std::cmp::Reverse(c.path.len()));

        let mut out = String::new();
        for c in matching {
            if !out.is_empty() {
                out.push_str("; ");
            }
            let _ = write!(out, "{}={}", c.name, c.value);
        }
        Some(out)
    }

    /// The cookies a script may read through `rq.cookies.jar(host)`, in the runtime's shape.
    pub fn seed_for(&self, host: &str) -> Vec<serde_json::Value> {
        let host = host.trim().to_ascii_lowercase();
        self.cookies
            .iter()
            .filter(|c| domain_matches(&host, &c.domain, c.host_only))
            .map(|c| {
                serde_json::json!({
                    "key": c.name,
                    "value": c.value,
                    "domain": c.domain,
                    "path": c.path,
                    "secure": c.secure,
                    "httpOnly": c.http_only,
                })
            })
            .collect()
    }

    /// Every host this jar holds a cookie for — what `hostAllowlist` is seeded from.
    pub fn hosts(&self) -> Vec<String> {
        let mut hosts: Vec<String> = self.cookies.iter().map(|c| c.domain.clone()).collect();
        hosts.sort();
        hosts.dedup();
        hosts
    }
}

impl Cookie {
    fn matches(&self, origin: &Origin) -> bool {
        if self.secure && !origin.secure {
            return false;
        }
        domain_matches(&origin.host, &self.domain, self.host_only)
            && path_matches(&origin.path, &self.path)
    }
}

/// RFC 6265 §5.1.3, minus the public-suffix check: a host-only cookie needs an exact match,
/// otherwise a domain cookie also covers subdomains.
fn domain_matches(host: &str, domain: &str, host_only: bool) -> bool {
    if host == domain {
        return true;
    }
    !host_only && host.ends_with(domain) && host[..host.len() - domain.len()].ends_with('.')
}

/// RFC 6265 §5.1.4.
fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    if request_path == cookie_path {
        return true;
    }
    if !request_path.starts_with(cookie_path) {
        return false;
    }
    cookie_path.ends_with('/') || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')
}

/// Parse one `Set-Cookie` value. Returns the cookie and whether it is a deletion
/// (`Max-Age=0`, which is how a server logs you out).
fn parse_set_cookie(header: &str, origin: &Origin) -> Option<(Cookie, bool)> {
    let mut parts = header.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    let mut cookie = Cookie {
        name: name.to_string(),
        value: value.trim().to_string(),
        domain: origin.host.clone(),
        path: default_path(&origin.path),
        host_only: true,
        secure: false,
        http_only: false,
    };
    let mut delete = false;

    for attr in parts {
        let (key, val) = match attr.split_once('=') {
            Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim()),
            None => (attr.trim().to_ascii_lowercase(), ""),
        };
        match key.as_str() {
            "domain" if !val.is_empty() => {
                let domain = val.trim_start_matches('.').to_ascii_lowercase();
                // A server may only widen to its own registrable parent, never to someone
                // else's host.
                if domain_matches(&origin.host, &domain, false) {
                    cookie.domain = domain;
                    cookie.host_only = false;
                }
            }
            "path" if val.starts_with('/') => cookie.path = val.to_string(),
            "secure" => cookie.secure = true,
            "httponly" => cookie.http_only = true,
            "max-age" if val.trim().parse::<i64>().map(|n| n <= 0).unwrap_or(false) => {
                delete = true;
            }
            _ => {}
        }
    }
    Some((cookie, delete))
}

/// RFC 6265 §5.1.4 default-path: the directory of the request path.
fn default_path(request_path: &str) -> String {
    match request_path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => request_path[..i].to_string(),
    }
}

/// The parts of a URL cookie matching needs. Kept here rather than pulling a URL crate for
/// three fields.
struct Origin {
    host: String,
    path: String,
    secure: bool,
}

impl Origin {
    fn parse(url: &str) -> Option<Origin> {
        let (scheme, rest) = url.split_once("://")?;
        let secure = scheme.eq_ignore_ascii_case("https");
        let rest = rest.split(['#']).next().unwrap_or(rest);
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let authority = authority.rsplit('@').next().unwrap_or(authority);
        let host = authority
            .split(':')
            .next()
            .unwrap_or(authority)
            .to_ascii_lowercase();
        if host.is_empty() {
            return None;
        }
        let path = path.split('?').next().unwrap_or(path);
        Some(Origin {
            host,
            path: if path.is_empty() {
                "/".into()
            } else {
                path.to_string()
            },
            secure,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(jar: &mut Jar, url: &str, header: &str) {
        jar.ingest(url, &[("Set-Cookie".into(), header.into())]);
    }

    #[test]
    fn a_cookie_comes_back_on_the_next_request_to_the_same_host() {
        let mut jar = Jar::new();
        set(
            &mut jar,
            "https://api.test/auth/login",
            "session=abc; Path=/",
        );
        assert_eq!(
            jar.header_for("https://api.test/me").as_deref(),
            Some("session=abc")
        );
        assert_eq!(jar.header_for("https://other.test/me"), None);
    }

    #[test]
    fn host_only_unless_the_server_widens_it() {
        let mut jar = Jar::new();
        set(&mut jar, "https://api.test/", "a=1");
        assert_eq!(jar.header_for("https://sub.api.test/"), None);

        set(&mut jar, "https://api.test/", "b=2; Domain=api.test");
        assert_eq!(
            jar.header_for("https://sub.api.test/").as_deref(),
            Some("b=2")
        );
        // …and never to a host it doesn't own.
        set(&mut jar, "https://api.test/", "c=3; Domain=evil.test");
        assert_eq!(jar.header_for("https://evil.test/"), None);
    }

    #[test]
    fn paths_are_matched_and_the_longest_is_sent_first() {
        let mut jar = Jar::new();
        set(&mut jar, "https://api.test/", "root=1; Path=/");
        set(&mut jar, "https://api.test/", "deep=2; Path=/admin");
        assert_eq!(
            jar.header_for("https://api.test/other").as_deref(),
            Some("root=1")
        );
        assert_eq!(
            jar.header_for("https://api.test/admin/x").as_deref(),
            Some("deep=2; root=1")
        );
    }

    #[test]
    fn a_secure_cookie_never_goes_out_in_the_clear() {
        let mut jar = Jar::new();
        set(&mut jar, "https://api.test/", "s=1; Secure");
        assert_eq!(jar.header_for("https://api.test/").as_deref(), Some("s=1"));
        assert_eq!(jar.header_for("http://api.test/"), None);
    }

    #[test]
    fn a_later_set_cookie_replaces_the_earlier_one_and_max_age_zero_deletes() {
        let mut jar = Jar::new();
        set(&mut jar, "https://api.test/", "session=one");
        set(&mut jar, "https://api.test/", "session=two");
        assert_eq!(
            jar.header_for("https://api.test/").as_deref(),
            Some("session=two")
        );
        set(&mut jar, "https://api.test/", "session=; Max-Age=0");
        assert_eq!(jar.header_for("https://api.test/"), None);
        assert!(jar.is_empty());
    }

    #[test]
    fn the_default_path_is_the_directory_of_the_request() {
        assert_eq!(default_path("/auth/login"), "/auth");
        assert_eq!(default_path("/login"), "/");
        assert_eq!(default_path("/"), "/");
    }

    #[test]
    fn seeds_carry_the_runtimes_field_names() {
        let mut jar = Jar::new();
        set(
            &mut jar,
            "https://api.test/x",
            "session=abc; Path=/; Secure; HttpOnly",
        );
        let seed = jar.seed_for("api.test");
        assert_eq!(seed[0]["key"], "session");
        assert_eq!(seed[0]["value"], "abc");
        assert_eq!(seed[0]["secure"], true);
        assert_eq!(seed[0]["httpOnly"], true);
        assert_eq!(jar.hosts(), vec!["api.test".to_string()]);
    }

    #[test]
    fn urls_with_ports_credentials_and_queries_still_match() {
        let mut jar = Jar::new();
        set(&mut jar, "http://user:pw@api.test:8080/a/b?x=1", "k=v");
        assert_eq!(
            jar.header_for("http://api.test:8080/a/c").as_deref(),
            Some("k=v")
        );
    }
}
