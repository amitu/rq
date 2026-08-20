//! OpenAPI 3.x (and Swagger 2.0 basics) → IR.
//!
//! Mirrors the app's TypeScript importer (`packages/importers/src/openapi`) so the two can be
//! held to a differential parity gate. Input is JSON or YAML; `$ref`s are dereferenced locally
//! against the document (circular refs are left as `{$ref}`, matching swagger-parser's
//! `circular: 'ignore'`); external (`http`) refs are never fetched. The folder tree is built from
//! URL **path segments** (not tags — tags only supply folder descriptions), each server becomes an
//! environment with a `base_url` variable, and requests carry `{{base_url}}`-prefixed URLs.

use serde_json::{Map, Value};

use cq_model::{
    ApiKeyPlacement, Auth, Body, Collection, Environment, FormField, HttpRequest, Item, KeyValue,
    KvKind, Method, PathVar, Protocol, RecordMeta, Request, Scope, SourceFormat, Url, VarCategory,
    VarType, Variable, Workspace,
};
use cq_report::Report;

const BASE_URL_VAR: &str = "{{base_url}}";
const HTTP_METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "head", "options", "trace",
];

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn parse_openapi(content: &str, report: &mut Report) -> Result<Workspace, String> {
    if content.trim().is_empty() {
        return Err("openapi: empty input".to_string());
    }
    let raw = parse_input(content)?;
    let root_obj = raw
        .as_object()
        .ok_or_else(|| "openapi: document is not an object".to_string())?;
    if root_obj.contains_key("asyncapi") {
        return Err("openapi: AsyncAPI documents are not supported".to_string());
    }
    if !root_obj.contains_key("openapi") && !root_obj.contains_key("swagger") {
        return Err("openapi: missing `openapi`/`swagger` version field".to_string());
    }
    // Dereference the whole document once; everything below reads a $ref-free tree.
    let root = deref(&raw);

    let spec_title = root
        .pointer("/info/title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("OpenAPI Import")
        .to_string();

    let servers = servers_of(&root);
    let environments = build_environments(&servers, &spec_title);
    let root_vars = build_root_collection_variables(&servers);

    // Root auth = the first security scheme that maps.
    let schemes = security_schemes(&root);
    let root_auth = first_mapped_auth(&schemes);

    let description = root
        .pointer("/info/description")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // Build the path-segment folder tree under the root collection.
    let mut builder = RootBuilder::new(&root, report);
    let items = builder.build_tree();

    let root_collection = Collection {
        meta: {
            let mut m = RecordMeta::new("oa-root", spec_title, SourceFormat::OpenApi);
            m.description = description;
            m
        },
        auth: root_auth,
        variables: root_vars,
        items,
        ..Default::default()
    };

    Ok(Workspace {
        meta: RecordMeta::new("oa-workspace", "", SourceFormat::OpenApi),
        cross_q: cq_model::ModelHeader::for_source(SourceFormat::OpenApi),
        collections: vec![root_collection],
        environments,
        packages: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Input parsing + $ref dereference
// ---------------------------------------------------------------------------

/// Parse JSON first, fall back to YAML (a superset of JSON, so this order is safe and matches the
/// app's `JSON.parse ?? yaml.parse`).
fn parse_input(content: &str) -> Result<Value, String> {
    if let Ok(v) = serde_json::from_str::<Value>(content) {
        return Ok(v);
    }
    serde_yaml::from_str::<Value>(content)
        .map_err(|e| format!("openapi: not valid JSON or YAML: {e}"))
}

/// Return a fully dereferenced clone of `root`. Local `#/...` refs are resolved against the
/// document; a ref already on the resolution stack (circular) is left as `{$ref}`; external refs
/// are left untouched (never fetched).
fn deref(root: &Value) -> Value {
    resolve_refs(root, root, &mut Vec::new())
}

fn resolve_refs(node: &Value, root: &Value, stack: &mut Vec<String>) -> Value {
    match node {
        Value::Object(m) => {
            if let Some(Value::String(r)) = m.get("$ref") {
                if let Some(ptr) = r.strip_prefix('#') {
                    if stack.iter().any(|s| s == r) {
                        return node.clone(); // circular — leave the ref
                    }
                    if let Some(target) = root.pointer(ptr) {
                        stack.push(r.clone());
                        let resolved = resolve_refs(target, root, stack);
                        stack.pop();
                        return resolved;
                    }
                }
                return node.clone(); // unresolved / external
            }
            let mut out = Map::new();
            for (k, v) in m {
                out.insert(k.clone(), resolve_refs(v, root, stack));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| resolve_refs(x, root, stack)).collect()),
        _ => node.clone(),
    }
}

// ---------------------------------------------------------------------------
// Example-value synthesis (mirrors utils/src/openapi-example-value.ts)
// ---------------------------------------------------------------------------

/// Synthesize an example value from a JSON Schema, matching the app's precedence:
/// `example` → `default` → `enum[0]` → `allOf` (merged object) → `oneOf[0]` → `anyOf[0]` →
/// type-specific defaults.
fn generate_example_value(schema: &Value) -> Value {
    let s = match schema.as_object() {
        Some(s) => s,
        None => return Value::Null,
    };
    if let Some(e) = s.get("example") {
        return e.clone();
    }
    if let Some(d) = s.get("default") {
        return d.clone();
    }
    if let Some(Value::Array(en)) = s.get("enum") {
        if let Some(first) = en.first() {
            return first.clone();
        }
    }
    if let Some(Value::Array(all)) = s.get("allOf") {
        let mut merged = Map::new();
        for sub in all {
            if let Value::Object(o) = generate_example_value(sub) {
                for (k, v) in o {
                    merged.insert(k, v);
                }
            }
        }
        return Value::Object(merged);
    }
    if let Some(Value::Array(one)) = s.get("oneOf") {
        if let Some(first) = one.first() {
            return generate_example_value(first);
        }
    }
    if let Some(Value::Array(any)) = s.get("anyOf") {
        if let Some(first) = any.first() {
            return generate_example_value(first);
        }
    }
    match normalize_type(s.get("type")).as_deref() {
        Some("string") => {
            let fmt = s.get("format").and_then(Value::as_str).unwrap_or("");
            Value::String(
                match fmt {
                    "date" => "2024-01-01",
                    "date-time" => "2024-01-01T00:00:00Z",
                    "email" => "user@example.com",
                    "uri" | "url" => "https://example.com",
                    "uuid" => "3fa85f64-5717-4562-b3fc-2c963f66afa6",
                    _ => "string",
                }
                .to_string(),
            )
        }
        Some("number") | Some("integer") => Value::from(0),
        Some("boolean") => Value::Bool(false),
        Some("array") => match s.get("items") {
            Some(items) => Value::Array(vec![generate_example_value(items)]),
            None => Value::Array(vec![]),
        },
        Some("object") | None => {
            if let Some(Value::Object(props)) = s.get("properties") {
                let mut o = Map::new();
                for (k, v) in props {
                    o.insert(k.clone(), generate_example_value(v));
                }
                Value::Object(o)
            } else {
                Value::Object(Map::new())
            }
        }
        _ => Value::Null,
    }
}

/// OpenAPI 3.1 allows `type: ["string","null"]`. Pick the first non-`null` entry; a bare string
/// type is returned as-is.
fn normalize_type(t: Option<&Value>) -> Option<String> {
    match t {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .find(|s| *s != "null")
            .map(str::to_string),
        _ => None,
    }
}

/// Render a synthesized example for a raw request/response body: a JSON string is returned
/// verbatim (not re-quoted); everything else is pretty-printed with 2-space indent; null →
/// empty string.
fn stringify_example(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

/// A parameter/field value: `example` if present else a synthesized value; non-strings are
/// JSON-encoded; null/absent → empty string.
fn scalar_example(schema_holder: &Map<String, Value>) -> String {
    if let Some(ex) = schema_holder.get("example") {
        return value_to_field_string(ex);
    }
    if let Some(schema) = schema_holder.get("schema") {
        return value_to_field_string(&generate_example_value(schema));
    }
    String::new()
}

fn value_to_field_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// Servers → environments + base_url variables
// ---------------------------------------------------------------------------

fn servers_of(root: &Value) -> Vec<Value> {
    match root.get("servers") {
        Some(Value::Array(a)) if !a.is_empty() => a.clone(),
        _ => {
            // Swagger 2.0 fallback: scheme://host + basePath.
            if let Some(host) = root.get("host").and_then(Value::as_str) {
                let scheme = root
                    .get("schemes")
                    .and_then(Value::as_array)
                    .and_then(|s| s.first())
                    .and_then(Value::as_str)
                    .unwrap_or("https");
                let base = root.get("basePath").and_then(Value::as_str).unwrap_or("");
                let url = format!("{scheme}://{host}{base}");
                vec![serde_json::json!({ "url": url })]
            } else {
                Vec::new()
            }
        }
    }
}

/// Substitute `{var}` with its `default` from `server.variables`, then strip a trailing slash.
fn resolve_server_url(server: &Value) -> String {
    let mut url = server
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(Value::Object(vars)) = server.get("variables") {
        for (name, def) in vars {
            if let Some(d) = def.get("default").and_then(Value::as_str) {
                url = url.replace(&format!("{{{name}}}"), d);
            }
        }
    }
    url.strip_suffix('/').map(str::to_string).unwrap_or(url)
}

fn build_environments(servers: &[Value], spec_title: &str) -> Vec<Environment> {
    servers
        .iter()
        .enumerate()
        .map(|(i, server)| {
            let name = if i == 0 {
                spec_title.to_string()
            } else {
                format!("{spec_title} ({})", i + 1)
            };
            Environment {
                meta: RecordMeta::new(format!("oa-env-{i}"), name, SourceFormat::OpenApi),
                variables: server_variables(server),
                ..Default::default()
            }
        })
        .collect()
}

/// Root-collection variables come from the FIRST server only.
fn build_root_collection_variables(servers: &[Value]) -> Vec<Variable> {
    servers.first().map(server_variables).unwrap_or_default()
}

fn server_variables(server: &Value) -> Vec<Variable> {
    let mut out = vec![make_var("base_url", &resolve_server_url(server), 0)];
    if let Some(Value::Object(vars)) = server.get("variables") {
        let mut rank = 1;
        for (name, def) in vars {
            if name == "base_url" {
                continue;
            }
            let default = def.get("default").and_then(Value::as_str).unwrap_or("");
            out.push(make_var(name, default, rank));
            rank += 1;
        }
    }
    out
}

fn make_var(key: &str, value: &str, rank: usize) -> Variable {
    Variable {
        key: key.to_string(),
        value: value.to_string(),
        initial: None,
        scope: Scope::Environment,
        data_type: VarType::String,
        category: VarCategory::Scoped,
        enabled: true,
        rank: Some(rank.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Security schemes → auth
// ---------------------------------------------------------------------------

/// `components.securitySchemes` (3.x) or `securityDefinitions` (2.0), by scheme name.
fn security_schemes(root: &Value) -> Map<String, Value> {
    root.pointer("/components/securitySchemes")
        .or_else(|| root.get("securityDefinitions"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn map_security_scheme(scheme: &Value) -> Option<Auth> {
    let ty = scheme.get("type").and_then(Value::as_str)?;
    match ty {
        "http" => {
            let sub = scheme.get("scheme").and_then(Value::as_str).unwrap_or("");
            match sub.to_ascii_lowercase().as_str() {
                "basic" => Some(Auth::Basic {
                    username: String::new(),
                    password: String::new(),
                }),
                "bearer" => Some(Auth::Bearer {
                    token: String::new(),
                    header_prefix: None,
                }),
                "digest" => Some(Auth::Digest {
                    params: Default::default(),
                }),
                _ => Some(Auth::Bearer {
                    token: String::new(),
                    header_prefix: None,
                }),
            }
        }
        "apiKey" => {
            let name = scheme
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let placement = match scheme.get("in").and_then(Value::as_str) {
                Some("query") => ApiKeyPlacement::Query,
                _ => ApiKeyPlacement::Header,
            };
            Some(Auth::ApiKey {
                key: name,
                value: String::new(),
                placement,
            })
        }
        "oauth2" => Some(Auth::OAuth2 {
            grant: oauth2_grant(scheme),
            params: Default::default(),
        }),
        // openIdConnect and unknown schemes are not mapped here (the app warns and skips).
        _ => None,
    }
}

/// First present flow, in the app's precedence order.
fn oauth2_grant(scheme: &Value) -> String {
    let flows = scheme.get("flows").and_then(Value::as_object);
    for grant in [
        "authorizationCode",
        "clientCredentials",
        "password",
        "implicit",
    ] {
        if flows.map(|f| f.contains_key(grant)).unwrap_or(false) {
            return grant.to_string();
        }
    }
    "authorizationCode".to_string()
}

fn first_mapped_auth(schemes: &Map<String, Value>) -> Option<Auth> {
    schemes.values().find_map(map_security_scheme)
}

// ---------------------------------------------------------------------------
// Path-segment tree + operations
// ---------------------------------------------------------------------------

struct RootBuilder<'a> {
    doc: &'a Value,
    #[allow(dead_code)]
    report: &'a mut Report,
    schemes: Map<String, Value>,
}

impl<'a> RootBuilder<'a> {
    fn new(doc: &'a Value, report: &'a mut Report) -> Self {
        let schemes = security_schemes(doc);
        Self {
            doc,
            report,
            schemes,
        }
    }

    /// Build the whole tree of sub-collections + requests under the root, from URL path segments.
    fn build_tree(&mut self) -> Vec<Item> {
        let paths = match self.doc.get("paths").and_then(Value::as_object) {
            Some(p) => p.clone(),
            None => return Vec::new(),
        };
        // The mutable folder tree: a recursive node of children collections keyed by segment.
        let mut root = TreeNode::default();
        for (path, item) in &paths {
            let path_item = match item.as_object() {
                Some(o) => o,
                None => continue,
            };
            let node = root.get_or_create(path);
            for method in HTTP_METHODS {
                if let Some(op) = path_item.get(*method) {
                    let req = self.build_request(path, method, path_item, op);
                    node.requests.push(req);
                }
            }
        }
        root.into_items()
    }

    fn build_request(
        &mut self,
        path: &str,
        method: &str,
        path_item: &Map<String, Value>,
        op: &Value,
    ) -> Request {
        let op_obj = op.as_object().cloned().unwrap_or_default();

        // Name: summary → operationId → "METHOD path".
        let name = op_obj
            .get("summary")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                op_obj
                    .get("operationId")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("{} {path}", method.to_ascii_uppercase()));

        // URL: {{base_url}} + path, with {var} → :var.
        let resolved_path = templated_path(path);
        let url = format!("{BASE_URL_VAR}{resolved_path}");

        let mut http = HttpRequest {
            method: Method::from(method.to_ascii_uppercase()),
            url: Url::raw(url),
            ..HttpRequest::default()
        };

        // Parameters: path-level then operation-level, operation wins on (name, in).
        let params = merge_parameters(path_item.get("parameters"), op_obj.get("parameters"));
        for p in &params {
            let (pname, pin) = (
                p.get("name").and_then(Value::as_str).unwrap_or(""),
                p.get("in").and_then(Value::as_str).unwrap_or(""),
            );
            if pname.is_empty() {
                continue;
            }
            let pobj = p.as_object().cloned().unwrap_or_default();
            match pin {
                "query" => http.query.push(kv(pname, &scalar_example(&pobj), p)),
                "header" => http.headers.push(kv(pname, &scalar_example(&pobj), p)),
                "path" => http.path_variables.push(PathVar {
                    key: pname.to_string(),
                    value: scalar_example(&pobj),
                    data_type: Default::default(),
                    description: p
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                }),
                _ => {} // cookie / body(2.0) are dropped
            }
        }
        // Path vars from the URL template that no `in: path` param covered.
        for tmpl in template_vars(path) {
            if !http.path_variables.iter().any(|pv| pv.key == tmpl) {
                http.path_variables.push(PathVar {
                    key: tmpl,
                    value: String::new(),
                    data_type: Default::default(),
                    description: None,
                });
            }
        }

        http.body = self.extract_body(&op_obj);

        // Auth: operation.security ?? spec.security, first resolving scheme.
        let auth = self.operation_auth(&op_obj);

        let mut meta = RecordMeta::new(
            format!("oa-{}-{}", method, slug(path)),
            name,
            SourceFormat::OpenApi,
        );
        meta.description = op_obj
            .get("description")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        Request {
            meta,
            protocol: Protocol::Http(http),
            auth,
            scripts: Default::default(),
            examples: self.build_examples(&op_obj),
            depends_on: Vec::new(),
            behavior: Default::default(),
        }
    }

    fn extract_body(&self, op: &Map<String, Value>) -> Option<Body> {
        let content = op
            .get("requestBody")
            .and_then(|rb| rb.get("content"))
            .and_then(Value::as_object)?;
        // Fixed precedence, first match wins.
        if let Some(mt) = content.get("application/json") {
            return Some(Body::Raw {
                text: stringify_example(&media_type_example(mt)),
                media_type: "application/json".to_string(),
            });
        }
        if let Some(mt) = content.get("application/x-www-form-urlencoded") {
            return Some(Body::UrlEncoded {
                fields: form_fields(mt),
            });
        }
        if let Some(mt) = content.get("multipart/form-data") {
            return Some(Body::FormData {
                fields: form_fields(mt).into_iter().map(FormField::Text).collect(),
            });
        }
        for xml in ["application/xml", "text/xml"] {
            if content.contains_key(xml) {
                return Some(Body::Raw {
                    text: String::new(),
                    media_type: xml.to_string(),
                });
            }
        }
        // Fallback: first content-type, raw.
        content.keys().next().map(|first| Body::Raw {
            text: stringify_example(&media_type_example(&content[first])),
            media_type: first.clone(),
        })
    }

    fn operation_auth(&self, op: &Map<String, Value>) -> Option<Auth> {
        let security = op
            .get("security")
            .or_else(|| self.doc.get("security"))
            .and_then(Value::as_array)?;
        for req in security {
            if let Some(obj) = req.as_object() {
                for name in obj.keys() {
                    if let Some(scheme) = self.schemes.get(name) {
                        if let Some(auth) = map_security_scheme(scheme) {
                            return Some(auth);
                        }
                    }
                }
            }
        }
        None
    }

    fn build_examples(&self, op: &Map<String, Value>) -> Vec<cq_model::Example> {
        let responses = match op.get("responses").and_then(Value::as_object) {
            Some(r) => r,
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        for (code, resp) in responses {
            if code.parse::<u32>().is_err() {
                continue; // skip `default` and other non-numeric statuses
            }
            let name = resp
                .get("description")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("{code} {}", status_text(code)));
            let response = response_body(resp);
            out.push(cq_model::Example {
                meta: RecordMeta::new(format!("oa-ex-{code}"), name, SourceFormat::OpenApi),
                request: None,
                auth: None,
                response,
            });
        }
        out
    }
}

/// The response body value stored on an example (from the first content media type's example).
fn response_body(resp: &Value) -> Option<Value> {
    let content = resp.get("content").and_then(Value::as_object)?;
    let first = content.keys().next()?;
    Some(Value::String(stringify_example(&media_type_example(
        &content[first],
    ))))
}

/// media-type example precedence: `example` → first `examples` entry (unwrapping `{value}`) →
/// synthesized from `schema`.
fn media_type_example(mt: &Value) -> Value {
    if let Some(ex) = mt.get("example") {
        return ex.clone();
    }
    if let Some(Value::Object(examples)) = mt.get("examples") {
        if let Some(first) = examples.values().next() {
            return first.get("value").cloned().unwrap_or_else(|| first.clone());
        }
    }
    if let Some(schema) = mt.get("schema") {
        return generate_example_value(schema);
    }
    Value::Null
}

fn form_fields(mt: &Value) -> Vec<KeyValue> {
    mt.pointer("/schema/properties")
        .and_then(Value::as_object)
        .map(|props| {
            props
                .iter()
                .map(|(name, schema)| {
                    KeyValue::new(
                        name.clone(),
                        value_to_field_string(&generate_example_value(schema)),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Merge path-level and operation-level parameters; operation wins on matching `(name, in)`.
fn merge_parameters(path_level: Option<&Value>, op_level: Option<&Value>) -> Vec<Value> {
    let key = |p: &Value| {
        (
            p.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            p.get("in")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        )
    };
    let mut merged: Vec<Value> = path_level
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(Value::Array(ops)) = op_level {
        for p in ops {
            let k = key(p);
            if let Some(slot) = merged.iter_mut().find(|e| key(e) == k) {
                *slot = p.clone();
            } else {
                merged.push(p.clone());
            }
        }
    }
    merged
}

fn kv(name: &str, value: &str, param: &Value) -> KeyValue {
    let mut k = KeyValue::new(name, value);
    k.kind = KvKind::Text;
    k.description = param
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);
    k
}

/// OpenAPI `{var}` → Requestly `:var`.
fn templated_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            out.push(':');
            for c2 in chars.by_ref() {
                if c2 == '}' {
                    break;
                }
                out.push(c2);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Names inside `{...}` in a path template.
fn template_vars(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut name = String::new();
            for c2 in chars.by_ref() {
                if c2 == '}' {
                    break;
                }
                name.push(c2);
            }
            if !name.is_empty() {
                out.push(name);
            }
        }
    }
    out
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Minimal status-text table (matches the app's hardcoded map for the common codes).
fn status_text(code: &str) -> &'static str {
    match code {
        "200" => "OK",
        "201" => "Created",
        "202" => "Accepted",
        "204" => "No Content",
        "301" => "Moved Permanently",
        "302" => "Found",
        "304" => "Not Modified",
        "400" => "Bad Request",
        "401" => "Unauthorized",
        "403" => "Forbidden",
        "404" => "Not Found",
        "409" => "Conflict",
        "422" => "Unprocessable Entity",
        "429" => "Too Many Requests",
        "500" => "Internal Server Error",
        "502" => "Bad Gateway",
        "503" => "Service Unavailable",
        _ => "",
    }
}

// ---------------------------------------------------------------------------
// Path-segment tree
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TreeNode {
    /// Child folders, in insertion order, keyed by segment.
    children: Vec<(String, TreeNode)>,
    requests: Vec<Request>,
}

impl TreeNode {
    /// Descend/create the node for a URL path, returning the deepest node.
    fn get_or_create(&mut self, path: &str) -> &mut TreeNode {
        let mut node = self;
        for seg in path.split('/').filter(|s| !s.is_empty()) {
            let idx = match node.children.iter().position(|(k, _)| k == seg) {
                Some(i) => i,
                None => {
                    node.children.push((seg.to_string(), TreeNode::default()));
                    node.children.len() - 1
                }
            };
            node = &mut node.children[idx].1;
        }
        node
    }

    fn into_items(self) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();
        for (seg, child) in self.children {
            let collection = Collection {
                meta: RecordMeta::new(format!("oa-seg-{}", slug(&seg)), seg, SourceFormat::OpenApi),
                items: child.into_items(),
                ..Default::default()
            };
            items.push(Item::Collection(Box::new(collection)));
        }
        for req in self.requests {
            items.push(Item::Request(Box::new(req)));
        }
        items
    }
}
