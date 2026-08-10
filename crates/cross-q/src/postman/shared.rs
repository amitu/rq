//! Version-agnostic Postman primitives, shared by the v1.0.0 / v2.0.0 / v2.1.0 parsers.
//!
//! The reuse core: small leaf helpers (coercion, key/value arrays, bodies, scripts,
//! variables) plus the v2 tree walk — which v2.0 and v2.1 share entirely, differing only
//! in how `auth` params are shaped (array vs object), so the auth parser is passed in.
//! Each version module wires *its own* raw shape into the IR by composing these; nobody
//! copy-pastes a whole parser.

use std::collections::BTreeMap;

use serde_json::Value;

use cq_model::{
    Auth, Body, Collection, Example, FormField, HttpRequest, Item, KeyValue, Method, ModelHeader,
    PathVar, Protocol, Provenance, RecordMeta, Request, ScalarType, Script, ScriptDialect,
    ScriptLang, Scripts, SourceFormat, Url, Variable, Workspace,
};
use cq_report::{Phase, Report};

pub(super) fn prov(locator: impl Into<String>) -> Provenance {
    Provenance {
        format: SourceFormat::Postman,
        locator: locator.into(),
    }
}

pub(super) fn obj_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

/// Read a Postman `description`, which may be a plain string OR the object form
/// `{ content, type }` (the app flattens the latter to its `content`). Empty → `None`.
pub(super) fn description(v: &Value) -> Option<String> {
    match v.get("description") {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Object(o)) => o
            .get("content")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

/// Read a v2 field by its canonical singular name, falling back to the plural spelling that
/// some Postman-published collections use (`headers`/`responses`/`events`). The value shape
/// is identical — only the key name differs — so tolerating it recovers data that would
/// otherwise be silently dropped (see `tests/corpus/README.md`). Detection stays strict;
/// structure is read liberally.
pub(super) fn field<'a>(v: &'a Value, singular: &str, plural: &str) -> Option<&'a Value> {
    v.get(singular).or_else(|| v.get(plural))
}

/// Coerce a key-value *key* into a string (RQ-3458). `None` (with a `Dropped` diagnostic)
/// only for genuinely ambiguous keys (object/array).
pub(super) fn coerce_key(v: Option<&Value>, report: &mut Report, locator: &str) -> Option<String> {
    match v {
        None | Some(Value::Null) => {
            report.coerced(
                Phase::Parse,
                prov(locator),
                "null/absent key coerced to \"\"",
            );
            Some(String::new())
        }
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => {
            report.coerced(
                Phase::Parse,
                prov(locator),
                format!("numeric key {n} coerced to string"),
            );
            Some(n.to_string())
        }
        Some(Value::Bool(b)) => {
            report.coerced(
                Phase::Parse,
                prov(locator),
                format!("boolean key {b} coerced to string"),
            );
            Some(b.to_string())
        }
        Some(other) => {
            report.dropped(
                Phase::Parse,
                prov(locator),
                format!("key-value with a non-scalar key dropped: {other}"),
            );
            None
        }
    }
}

/// Coerce a value into a string: strings verbatim, numbers/bools stringified, arrays
/// space-joined, null → "".
pub(super) fn coerce_value(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| x.to_string())
            })
            .collect::<Vec<_>>()
            .join(" "),
        Some(other) => other.to_string(),
    }
}

/// Parse an array of Postman key/value pairs (`header`, `query`, urlencoded, formdata).
pub(super) fn parse_kv_array(
    v: Option<&Value>,
    report: &mut Report,
    locator: &str,
) -> Vec<KeyValue> {
    let mut out = Vec::new();
    let Some(Value::Array(items)) = v else {
        return out;
    };
    for (i, item) in items.iter().enumerate() {
        let loc = format!("{locator}[{i}]");
        let Some(key) = coerce_key(item.get("key"), report, &loc) else {
            continue;
        };
        let mut kv = KeyValue::new(key, coerce_value(item.get("value")));
        if item
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            kv.enabled = false;
        }
        if let Some(desc) = item.get("description").and_then(Value::as_str) {
            kv.description = Some(desc.to_string());
        }
        out.push(kv);
    }
    out
}

/// Parse an HTTP header block given as a newline-delimited string ("K: V\nK2: V2") —
/// the v1.0.0 shape. (v2 headers are a kv array; use [`parse_kv_array`] there.)
pub(super) fn parse_header_string(s: &str) -> Vec<KeyValue> {
    s.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (k, v) = line.split_once(':')?;
            Some(KeyValue::new(k.trim(), v.trim()))
        })
        .collect()
}

/// Postman `url` may be a string or an object with `raw` + structured `query`.
pub(super) fn parse_url(
    v: Option<&Value>,
    report: &mut Report,
    locator: &str,
) -> (Url, Vec<KeyValue>) {
    match v {
        Some(Value::String(s)) => (Url::raw(s), Vec::new()),
        Some(Value::Object(_)) => {
            let raw = obj_str(v.unwrap(), "raw").unwrap_or_default();
            let query =
                parse_kv_array(v.unwrap().get("query"), report, &format!("{locator}.query"));
            (Url::raw(raw), query)
        }
        _ => (Url::default(), Vec::new()),
    }
}

/// Parse a v2 `body` object (mode-tagged).
pub(super) fn parse_body(v: Option<&Value>, report: &mut Report, locator: &str) -> Option<Body> {
    let body = v?;
    let mode = body.get("mode").and_then(Value::as_str)?;
    match mode {
        "raw" => {
            let text = obj_str(body, "raw").unwrap_or_default();
            let media_type = body
                .get("options")
                .and_then(|o| o.get("raw"))
                .and_then(|r| r.get("language"))
                .and_then(Value::as_str)
                .map(raw_language_to_media_type)
                .unwrap_or("text/plain")
                .to_string();
            Some(Body::Raw { text, media_type })
        }
        "urlencoded" => Some(Body::UrlEncoded {
            fields: parse_kv_array(
                body.get("urlencoded"),
                report,
                &format!("{locator}.urlencoded"),
            ),
        }),
        "formdata" => {
            let fields =
                parse_kv_array(body.get("formdata"), report, &format!("{locator}.formdata"));
            Some(Body::FormData {
                fields: fields.into_iter().map(FormField::Text).collect(),
            })
        }
        "graphql" => {
            let query = body
                .get("graphql")
                .and_then(|g| g.get("query"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let variables = body
                .get("graphql")
                .and_then(|g| g.get("variables"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(Body::Graphql {
                query,
                variables,
                operation_name: None,
            })
        }
        "file" => {
            report.dropped(
                Phase::Parse,
                prov(format!("{locator}.file")),
                "file body cannot be carried without the file; emitted as no body",
            );
            None
        }
        other => {
            report.dropped(
                Phase::Parse,
                prov(locator),
                format!("unknown body mode {other:?} dropped"),
            );
            None
        }
    }
}

pub(super) fn raw_language_to_media_type(lang: &str) -> &'static str {
    match lang {
        "json" => "application/json",
        "xml" => "application/xml",
        "html" => "text/html",
        "javascript" => "application/javascript",
        _ => "text/plain",
    }
}

/// The shared `type`-dispatched auth builder. Both v2.0 and v2.1 (and v1) extract a
/// `{param → value}` map in their own way, then hand it here — so the mapping from
/// Postman auth kinds to IR [`Auth`] lives in exactly one place. Unknown types are
/// preserved as [`Auth::Unknown`] so a credential is never lost.
pub(super) fn build_auth(
    ty: &str,
    params: BTreeMap<String, String>,
    raw: &Value,
    report: &mut Report,
    locator: &str,
) -> Option<Auth> {
    let get = |k: &str| params.get(k).cloned().unwrap_or_default();
    match ty {
        "noauth" => Some(Auth::None),
        "inherit" => Some(Auth::Inherit),
        "basic" => Some(Auth::Basic {
            username: get("username"),
            password: get("password"),
        }),
        "bearer" => Some(Auth::Bearer {
            token: get("token"),
            header_prefix: None,
        }),
        "apikey" => {
            let placement = match params.get("in").map(String::as_str) {
                Some("query") => cq_model::ApiKeyPlacement::Query,
                _ => cq_model::ApiKeyPlacement::Header,
            };
            Some(Auth::ApiKey {
                key: get("key"),
                value: get("value"),
                placement,
            })
        }
        "oauth2" => Some(Auth::OAuth2 {
            grant: "manual".to_string(),
            params,
        }),
        "oauth1" => Some(Auth::OAuth1 { params }),
        "digest" => Some(Auth::Digest { params }),
        "hawk" => Some(Auth::Hawk { params }),
        "awsv4" => Some(Auth::AwsSigV4 { params }),
        "ntlm" => Some(Auth::Ntlm { params }),
        other => {
            report.coerced(
                Phase::Map,
                prov(locator),
                format!("auth type {other:?} preserved as 'unknown'"),
            );
            Some(Auth::Unknown {
                raw_type: other.to_string(),
                raw: raw.clone(),
            })
        }
    }
}

/// Postman v2 `event[]` → pre-request / post-response scripts (dialect `pm`).
pub(super) fn parse_scripts(v: Option<&Value>) -> Scripts {
    let mut scripts = Scripts::default();
    let Some(Value::Array(events)) = v else {
        return scripts;
    };
    for ev in events {
        let listen = ev.get("listen").and_then(Value::as_str).unwrap_or("");
        let src = ev
            .get("script")
            .and_then(|s| s.get("exec"))
            .map(join_exec)
            .unwrap_or_default();
        if src.trim().is_empty() {
            continue;
        }
        let script = pm_script(src);
        match listen {
            "prerequest" => scripts.pre_request = Some(script),
            "test" => scripts.post_response = Some(script),
            _ => {}
        }
    }
    scripts
}

/// A `pm.*`-dialect JS script (preserved verbatim; translation is cross-q-context's job).
pub(super) fn pm_script(source: String) -> Script {
    Script {
        source,
        language: ScriptLang::JavaScript,
        dialect: ScriptDialect::Pm,
    }
}

fn join_exec(exec: &Value) -> String {
    match exec {
        Value::String(s) => s.clone(),
        Value::Array(lines) => lines
            .iter()
            .map(|l| l.as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub(super) fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let t = out.trim_matches('-').to_string();
    if t.is_empty() {
        "item".to_string()
    } else {
        t
    }
}

pub(super) fn parse_variables(v: Option<&Value>) -> Vec<Variable> {
    let mut out = Vec::new();
    if let Some(Value::Array(items)) = v {
        for it in items {
            if let Some(key) = it.get("key").and_then(Value::as_str) {
                out.push(Variable {
                    key: key.to_string(),
                    value: coerce_value(it.get("value")),
                    initial: None,
                    scope: cq_model::Scope::Collection,
                    data_type: cq_model::VarType::String,
                    category: cq_model::VarCategory::Scoped,
                    enabled: !it.get("disabled").and_then(Value::as_bool).unwrap_or(false),
                    rank: None,
                });
            }
        }
    }
    out
}

/// Assemble a `RecordMeta` from an id/name with a provenance locator + optional description.
pub(super) fn record_meta(
    id: String,
    name: String,
    locator: &str,
    description: Option<String>,
) -> RecordMeta {
    let mut meta = RecordMeta::new(id, name, SourceFormat::Postman);
    meta.source.locator = locator.to_string();
    meta.description = description;
    meta
}

/// Identity + metadata for an IR request/collection node.
pub(super) struct NodeMeta {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// Wrap a pre-built [`HttpRequest`] into an IR [`Request`] with its metadata, auth, and
/// scripts (the shared final assembly step). Callers build the protocol payload in their
/// own version-specific way, then hand it here.
pub(super) fn http_request(
    meta: NodeMeta,
    locator: &str,
    http: HttpRequest,
    auth: Option<Auth>,
    scripts: Scripts,
) -> Request {
    Request {
        meta: record_meta(meta.id, meta.name, locator, meta.description),
        protocol: Protocol::Http(http),
        auth,
        scripts,
        examples: Vec::new(),
        depends_on: Vec::new(),
        behavior: Default::default(),
    }
}

// -------------------------------------------------------------------------------------
// The v2 tree walk — shared by v2.0 and v2.1. Only the auth shape differs, so the
// version's auth parser is passed in as `auth_fn`.
// -------------------------------------------------------------------------------------

/// A version's auth parser: reads a raw Postman `auth` value → IR [`Auth`].
pub(super) type AuthFn = fn(Option<&Value>, &mut Report, &str) -> Option<Auth>;

/// Parse a v2.x collection (`info` + `item[]` tree) into a [`Workspace`].
pub(super) fn parse_v2_tree(root: &Value, report: &mut Report, auth_fn: AuthFn) -> Workspace {
    let info_name = root
        .get("info")
        .and_then(|i| obj_str(i, "name"))
        .unwrap_or_else(|| "Imported Collection".to_string());

    let items = parse_v2_items(root.get("item"), report, "item", auth_fn);
    let collection = Collection {
        meta: record_meta(
            root.get("info")
                .and_then(|i| obj_str(i, "_postman_id"))
                .unwrap_or_else(|| format!("pm-{}", slugify(&info_name))),
            info_name.clone(),
            "info",
            root.get("info").map(description).unwrap_or(None),
        ),
        auth: auth_fn(root.get("auth"), report, "auth"),
        headers: Vec::new(),
        scripts: parse_scripts(field(root, "event", "events")),
        variables: parse_variables(root.get("variable")),
        items,
    };

    Workspace {
        meta: record_meta("pm-workspace".into(), info_name, "", None),
        cross_q: ModelHeader::for_source(SourceFormat::Postman),
        collections: vec![collection],
        environments: Vec::new(),
        packages: Vec::new(),
    }
}

fn parse_v2_items(
    v: Option<&Value>,
    report: &mut Report,
    locator: &str,
    auth_fn: AuthFn,
) -> Vec<Item> {
    let mut out = Vec::new();
    if let Some(Value::Array(items)) = v {
        for (i, item) in items.iter().enumerate() {
            out.push(parse_v2_item(
                item,
                report,
                &format!("{locator}.item[{i}]"),
                auth_fn,
            ));
        }
    }
    out
}

fn parse_v2_item(item: &Value, report: &mut Report, locator: &str, auth_fn: AuthFn) -> Item {
    if item.get("item").is_some() {
        let name = obj_str(item, "name").unwrap_or_else(|| "folder".to_string());
        let id = obj_str(item, "id").unwrap_or_else(|| format!("pm-{}", slugify(locator)));
        Item::Collection(Box::new(Collection {
            meta: record_meta(id, name, locator, description(item)),
            auth: auth_fn(item.get("auth"), report, &format!("{locator}.auth")),
            headers: Vec::new(),
            scripts: parse_scripts(field(item, "event", "events")),
            variables: Vec::new(),
            items: parse_v2_items(item.get("item"), report, locator, auth_fn),
        }))
    } else {
        Item::Request(Box::new(parse_v2_request(item, report, locator, auth_fn)))
    }
}

/// Parse a Postman **request object** (the value of `item.request`, or a saved response's
/// `originalRequest`) into an [`HttpRequest`] + its [`Auth`]. Shared by the request mapper
/// and the example mapper.
pub(super) fn parse_request_obj(
    req: Option<&Value>,
    report: &mut Report,
    locator: &str,
    auth_fn: AuthFn,
) -> (HttpRequest, Option<Auth>) {
    match req {
        Some(Value::String(s)) => (
            HttpRequest {
                url: Url::raw(s),
                ..HttpRequest::default()
            },
            None,
        ),
        Some(r @ Value::Object(_)) => {
            let method = r
                .get("method")
                .and_then(Value::as_str)
                .map(|m| Method::from(m.to_string()))
                .unwrap_or(Method::Get);
            let (url, query) = parse_url(r.get("url"), report, &format!("{locator}.url"));
            let path_variables = parse_path_vars(r.get("url"));
            let headers = parse_kv_array(
                field(r, "header", "headers"),
                report,
                &format!("{locator}.header"),
            );
            let body = parse_body(r.get("body"), report, &format!("{locator}.body"));
            let auth = auth_fn(r.get("auth"), report, &format!("{locator}.auth"));
            (
                HttpRequest {
                    method,
                    url,
                    headers,
                    query,
                    path_variables,
                    body,
                    settings: cq_model::RequestSettings::default(),
                },
                auth,
            )
        }
        _ => (HttpRequest::default(), None),
    }
}

fn parse_v2_request(item: &Value, report: &mut Report, locator: &str, auth_fn: AuthFn) -> Request {
    let name = obj_str(item, "name").unwrap_or_else(|| "request".to_string());
    let (http, auth) = parse_request_obj(item.get("request"), report, locator, auth_fn);

    let id = obj_str(item, "id")
        .or_else(|| obj_str(item, "_postman_id"))
        .unwrap_or_else(|| format!("pm-{}", slugify(locator)));
    let desc = item.get("request").and_then(description);

    let mut request = http_request(
        NodeMeta {
            id,
            name,
            description: desc,
        },
        locator,
        http,
        auth,
        parse_scripts(field(item, "event", "events")),
    );
    // Saved responses (`response[]`) → examples, stored verbatim so the round-trip is
    // lossless (incl. Postman-internal fields like `_postman_previewlanguage`).
    request.examples = parse_examples(
        field(item, "response", "responses"),
        report,
        locator,
        auth_fn,
    );
    request
}

/// Postman saved responses (`response[]`) → examples. The full response object is kept
/// verbatim in [`Example::response`] (lossless round-trip); the response's `originalRequest`
/// is *also* parsed into [`Example::request`] so exporters that model an example as a
/// (request, response) pair (e.g. Requestly) can emit it without re-parsing.
pub(super) fn parse_examples(
    v: Option<&Value>,
    report: &mut Report,
    locator: &str,
    auth_fn: AuthFn,
) -> Vec<Example> {
    let mut out = Vec::new();
    if let Some(Value::Array(items)) = v {
        for (i, resp) in items.iter().enumerate() {
            let name = obj_str(resp, "name").unwrap_or_else(|| format!("example {i}"));
            let id = obj_str(resp, "id").unwrap_or_else(|| format!("pm-ex-{}", slugify(&name)));
            let loc = format!("{locator}.response[{i}]");
            // The saved response's `originalRequest` (when present) is the request that
            // produced it — parse it so exporters can emit the (request, response) pair.
            let (request, auth) = match resp.get("originalRequest") {
                Some(orig) => {
                    let (http, auth) = parse_request_obj(Some(orig), report, &loc, auth_fn);
                    (Some(http), auth)
                }
                None => (None, None),
            };
            out.push(Example {
                meta: record_meta(id, name, &loc, None),
                request,
                auth,
                response: Some(resp.clone()),
            });
        }
    }
    out
}

/// Postman url `variable[]` (path variables, e.g. `:id`) → IR path variables.
pub(super) fn parse_path_vars(url: Option<&Value>) -> Vec<PathVar> {
    let mut out = Vec::new();
    if let Some(Value::Object(o)) = url {
        if let Some(Value::Array(vars)) = o.get("variable") {
            for v in vars {
                if let Some(key) = v.get("key").and_then(Value::as_str) {
                    out.push(PathVar {
                        key: key.to_string(),
                        value: coerce_value(v.get("value")),
                        data_type: ScalarType::default(),
                        description: v
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    });
                }
            }
        }
    }
    out
}
