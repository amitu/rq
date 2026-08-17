//! Variable resolution, `{{templating}}`, prompting, and response capture.
//!
//! Precedence, highest first — the same ladder the Requestly client resolves on, so a
//! collection behaves the same in `rq` as it does in the app:
//!
//! ```text
//! --var (command line)  >  runtime (captured from a parent)  >  environment
//!                       >  global  >  collection (child before parent)  >  declared default
//! ```
//!
//! First write wins: a value found in a higher scope is never overwritten by a lower one.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{IsTerminal, Write};

use anyhow::{bail, Result};

use crate::doc::VarSpec;

/// A resolved variable set: values, where each came from, and which are secret.
#[derive(Clone, Debug, Default)]
pub struct Vars {
    values: BTreeMap<String, String>,
    origins: BTreeMap<String, String>,
    secrets: BTreeSet<String>,
}

impl Vars {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a layer *below* everything already present. Call from highest precedence to
    /// lowest; existing keys are kept.
    pub fn layer<I, K, V>(&mut self, origin: &str, entries: I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (k, v) in entries {
            let k = k.into();
            if k.is_empty() || self.values.contains_key(&k) {
                continue;
            }
            self.origins.insert(k.clone(), origin.to_string());
            self.values.insert(k, v.into());
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>, origin: &str) {
        let key = key.into();
        self.origins.insert(key.clone(), origin.to_string());
        self.values.insert(key, value.into());
    }

    pub fn mark_secret(&mut self, key: &str) {
        self.secrets.insert(key.to_string());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn is_secret(&self, key: &str) -> bool {
        self.secrets.contains(key)
    }

    pub fn origin(&self, key: &str) -> Option<&str> {
        self.origins.get(key).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.values.iter()
    }

    /// Every secret value currently known — what a redacting printer must hide.
    pub fn secret_values(&self) -> Vec<String> {
        self.secrets
            .iter()
            .filter_map(|k| self.values.get(k))
            .filter(|v| !v.is_empty())
            .cloned()
            .collect()
    }

    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.values
    }
}

/// The result of substituting into one string.
#[derive(Debug, Default)]
pub struct Substitution {
    pub text: String,
    /// Names referenced but not resolved. Left verbatim in `text`, never blanked.
    pub missing: Vec<String>,
}

/// Replace `{{name}}` with its value. Unresolved names stay exactly as written — a request
/// that goes out with a literal `{{token}}` is a visible bug; one that goes out with an
/// empty header is a mystery.
pub fn substitute(template: &str, vars: &Vars) -> Substitution {
    let mut out = String::with_capacity(template.len());
    let mut missing = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;

    while i < template.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = template[i + 2..].find("}}") {
                let raw = &template[i + 2..i + 2 + end];
                let name = raw.trim();
                if !name.is_empty() && !name.contains('{') {
                    match vars.get(name) {
                        Some(v) => out.push_str(v),
                        None => {
                            if !missing.iter().any(|m| m == name) {
                                missing.push(name.to_string());
                            }
                            out.push_str(&template[i..i + 4 + end]);
                        }
                    }
                    i += 4 + end;
                    continue;
                }
            }
        }
        let ch = template[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    Substitution { text: out, missing }
}

/// Fill in declared variables that no higher scope supplied: read the bound process
/// environment variable, ask, or fall back to the declared default.
///
/// `ask_all` is `--prompt`: ask for every declared variable even when a default exists.
pub fn resolve_declared(
    vars: &mut Vars,
    declared: &[(String, VarSpec)],
    ask_all: bool,
    interactive: bool,
) -> Result<()> {
    for (name, spec) in declared {
        if spec.secret {
            vars.mark_secret(name);
        }
        let already = vars.get(name).map(str::to_string);

        if !ask_all {
            if let Some(v) = already {
                if !v.is_empty() {
                    continue;
                }
            }
            if let Some(env_name) = &spec.env {
                if let Ok(v) = std::env::var(env_name) {
                    if !v.is_empty() {
                        vars.set(name, v, &format!("env:{env_name}"));
                        continue;
                    }
                }
            }
        }

        let should_ask = ask_all
            || spec.prompt.is_some() && spec.default.is_none()
            || spec.required && spec.default.is_none();

        if should_ask {
            if !interactive {
                if let Some(d) = &spec.default {
                    vars.set(name, d.clone(), "default");
                    continue;
                }
                bail!(
                    "`{name}` has no value and there is no terminal to ask on\n  \
                     pass `--var {name}=…`{}",
                    spec.env
                        .as_ref()
                        .map(|e| format!(" or set ${e}"))
                        .unwrap_or_default()
                );
            }
            let label = spec.prompt.clone().unwrap_or_else(|| name.clone());
            let value = ask(&label, spec.default.as_deref(), spec.secret)?;
            let value = if value.is_empty() {
                spec.default
                    .as_deref()
                    .map(|d| substitute(d, vars).text)
                    .unwrap_or_default()
            } else {
                value
            };
            if value.is_empty() && spec.required {
                bail!("`{name}` is required");
            }
            vars.set(name, value, "prompt");
            continue;
        }

        if let Some(default) = &spec.default {
            // A default may itself be a template — `login: { default: '{{owner}}' }`, or a
            // form field that offers you your own handle. Resolve it against what is known
            // so far, or the request goes out carrying `{{owner}}` as a literal.
            let resolved = substitute(default, vars).text;
            vars.layer("default", [(name.clone(), resolved)]);
        }

        if spec.required && vars.get(name).map(str::is_empty).unwrap_or(true) {
            bail!(
                "`{name}` is required but unset\n  pass `--var {name}=…`{}",
                spec.env
                    .as_ref()
                    .map(|e| format!(", set ${e}",))
                    .unwrap_or_default()
            );
        }
    }
    Ok(())
}

/// Ask on the terminal. Secrets are read without echo — the reason `secret: true` exists is
/// so a token never lands in a screen recording or a scrollback buffer.
fn ask(label: &str, default: Option<&str>, secret: bool) -> Result<String> {
    let suffix = match default {
        Some(d) if !d.is_empty() && !secret => format!(" [{d}]"),
        _ => String::new(),
    };
    if secret {
        let value = rpassword::prompt_password(format!("{label}{suffix}: "))?;
        return Ok(value.trim().to_string());
    }
    print!("{label}{suffix}: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

pub fn stdin_is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Parse a `--var key=value` pair.
pub fn parse_assignment(raw: &str) -> Result<(String, String)> {
    match raw.split_once('=') {
        Some((k, v)) if !k.trim().is_empty() => Ok((k.trim().to_string(), v.to_string())),
        _ => bail!("expected key=value, got `{raw}`"),
    }
}

/// Read a value out of a run's context for `capture:` — `response.body.access_token`,
/// `response.headers.etag`, `response.status`, `response.body.items[0].id`.
///
/// This is the declarative half of chaining: the common case (pull a token out of a JSON
/// response and hand it to the next request) needs no JavaScript at all.
pub fn extract(root: &serde_json::Value, path: &str) -> Option<String> {
    let mut cur = root;
    for raw in path.split('.') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let (name, indexes) = split_indexes(raw);
        if !name.is_empty() {
            cur = match cur {
                serde_json::Value::Object(map) => map.get(name)?,
                _ => return None,
            };
        }
        for i in indexes {
            cur = cur.as_array()?.get(i)?;
        }
    }
    Some(match cur {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
}

fn split_indexes(seg: &str) -> (&str, Vec<usize>) {
    let Some(open) = seg.find('[') else {
        return (seg, Vec::new());
    };
    let mut indexes = Vec::new();
    for part in seg[open..].split('[').skip(1) {
        if let Some(num) = part.strip_suffix(']') {
            if let Ok(i) = num.trim().parse::<usize>() {
                indexes.push(i);
            }
        }
    }
    (&seg[..open], indexes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars_of(pairs: &[(&str, &str)]) -> Vars {
        let mut v = Vars::new();
        v.layer("test", pairs.iter().map(|(k, val)| (*k, *val)));
        v
    }

    #[test]
    fn substitutes_and_reports_missing() {
        let v = vars_of(&[("owner", "anthropics")]);
        let s = substitute("https://x/{{owner}}/{{ repo }}", &v);
        assert_eq!(s.text, "https://x/anthropics/{{ repo }}");
        assert_eq!(s.missing, vec!["repo"]);
    }

    #[test]
    fn leaves_unrelated_braces_alone() {
        let v = vars_of(&[]);
        let s = substitute(r#"{"id": 1, "n": {{}} }"#, &v);
        assert_eq!(s.text, r#"{"id": 1, "n": {{}} }"#);
        assert!(s.missing.is_empty());
    }

    #[test]
    fn first_layer_wins() {
        let mut v = Vars::new();
        v.layer("cli", [("host", "cli.test")]);
        v.layer("environment", [("host", "env.test"), ("port", "8080")]);
        assert_eq!(v.get("host"), Some("cli.test"));
        assert_eq!(v.get("port"), Some("8080"));
        assert_eq!(v.origin("host"), Some("cli"));
    }

    #[test]
    fn declared_defaults_are_the_bottom_layer() {
        let mut v = Vars::new();
        v.layer("environment", [("owner", "from-env")]);
        let declared = vec![
            (
                "owner".to_string(),
                VarSpec {
                    default: Some("anthropics".into()),
                    ..VarSpec::default()
                },
            ),
            (
                "repo".to_string(),
                VarSpec {
                    default: Some("claude-code".into()),
                    ..VarSpec::default()
                },
            ),
        ];
        resolve_declared(&mut v, &declared, false, false).unwrap();
        assert_eq!(v.get("owner"), Some("from-env"));
        assert_eq!(v.get("repo"), Some("claude-code"));
    }

    #[test]
    fn env_binding_fills_a_declared_secret() {
        std::env::set_var("RQ_TEST_TOKEN", "s3cret");
        let mut v = Vars::new();
        let declared = vec![(
            "TOKEN".to_string(),
            VarSpec {
                env: Some("RQ_TEST_TOKEN".into()),
                secret: true,
                required: true,
                ..VarSpec::default()
            },
        )];
        resolve_declared(&mut v, &declared, false, false).unwrap();
        assert_eq!(v.get("TOKEN"), Some("s3cret"));
        assert!(v.is_secret("TOKEN"));
        assert_eq!(v.secret_values(), vec!["s3cret".to_string()]);
        std::env::remove_var("RQ_TEST_TOKEN");
    }

    #[test]
    fn required_without_a_value_fails_loudly() {
        let mut v = Vars::new();
        let declared = vec![(
            "TOKEN".to_string(),
            VarSpec {
                env: Some("RQ_TEST_ABSENT".into()),
                required: true,
                ..VarSpec::default()
            },
        )];
        let err = resolve_declared(&mut v, &declared, false, false).unwrap_err();
        assert!(err.to_string().contains("RQ_TEST_ABSENT"), "{err}");
    }

    #[test]
    fn a_default_can_refer_to_another_variable() {
        let mut v = Vars::new();
        v.layer("environment", [("owner", "anthropics")]);
        let declared = vec![(
            "login".to_string(),
            VarSpec {
                default: Some("{{owner}}".into()),
                ..VarSpec::default()
            },
        )];
        resolve_declared(&mut v, &declared, false, false).unwrap();
        assert_eq!(v.get("login"), Some("anthropics"));
    }

    #[test]
    fn extracts_capture_paths() {
        let json = serde_json::json!({
            "response": {
                "status": 200,
                "body": { "access_token": "abc", "items": [{ "id": 7 }] },
                "headers": { "etag": "W/\"x\"" }
            }
        });
        assert_eq!(
            extract(&json, "response.body.access_token").as_deref(),
            Some("abc")
        );
        assert_eq!(
            extract(&json, "response.body.items[0].id").as_deref(),
            Some("7")
        );
        assert_eq!(extract(&json, "response.status").as_deref(), Some("200"));
        assert_eq!(extract(&json, "response.body.nope"), None);
    }
}
