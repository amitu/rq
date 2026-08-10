//! IR → Requestly on-the-wire JSON shapes, shared by both Requestly serializations:
//! the `LOCAL_FS` file tree ([`crate::emit_rq`]) and the in-memory `MappedItems` bundle
//! ([`crate::mappeditems`]). One IR→Requestly mapper, two serializations — so a fix lands
//! once. Matches `docs/FORMAT.md` (schema `1.12.0`).

use serde_json::{json, Value};

use cq_model::{Auth, Body, HttpRequest, KeyValue, PathVar, Scripts, Variable};

/// The Requestly `LOCAL_FS` schema version these shapes target.
pub const RQ_SCHEMA_VERSION: &str = "1.12.0";

pub fn schema_url(base: &str) -> String {
    format!("https://assets.requestly.com/local/v{RQ_SCHEMA_VERSION}/{base}.json")
}

/// The top-level `contentType` selector Requestly stores on a request.
pub fn content_type_selector(body: &Option<Body>) -> &'static str {
    match body {
        None | Some(Body::None) => "none",
        Some(Body::Raw { media_type, .. }) => {
            if media_type.contains("json") {
                "json"
            } else {
                "raw"
            }
        }
        Some(Body::UrlEncoded { .. }) => "form",
        Some(Body::FormData { .. }) => "multipart/form-data",
        Some(Body::Binary { .. }) => "binary",
        Some(Body::Graphql { .. }) => "raw",
    }
}

pub fn body_to_json(body: &Body) -> Value {
    match body {
        Body::None => json!({ "contentType": "none" }),
        Body::Raw { text, media_type } => json!({
            "contentType": if media_type.contains("json") { "json" } else { "raw" },
            "raw": text,
            "rawContentType": media_type,
        }),
        Body::UrlEncoded { fields } => json!({
            "contentType": "form",
            "formUrlEncoded": kvs_to_json(fields),
        }),
        Body::FormData { .. } => json!({
            "contentType": "multipart/form-data",
            "formData": [],
        }),
        Body::Binary { .. } => json!({ "contentType": "binary" }),
        Body::Graphql {
            query, variables, ..
        } => json!({
            "contentType": "raw",
            "raw": query,
            "graphqlVariables": variables,
        }),
    }
}

/// Requestly `keyValuePairSchema[]` — `{ id (number), key, value, isEnabled }`.
/// Ids are 0-based to match the app's importer.
pub fn kvs_to_json(kvs: &[KeyValue]) -> Value {
    Value::Array(
        kvs.iter()
            .enumerate()
            .map(|(i, kv)| {
                let mut m = serde_json::Map::new();
                m.insert("id".into(), json!(i as u64));
                m.insert("key".into(), json!(kv.key));
                m.insert("value".into(), json!(kv.value));
                m.insert("isEnabled".into(), json!(kv.enabled));
                // The app carries a header/param `description` when the source had one; emit
                // it only when present (matches `...(description ? {description} : {})`).
                if let Some(desc) = &kv.description {
                    m.insert("description".into(), json!(desc));
                }
                Value::Object(m)
            })
            .collect(),
    )
}

/// Requestly `pathVariableSchema[]` — `{ key, value, dataType }`.
pub fn path_vars_to_json(pvs: &[PathVar]) -> Value {
    Value::Array(
        pvs.iter()
            .map(|pv| {
                let mut m = serde_json::Map::new();
                m.insert("key".into(), json!(pv.key));
                m.insert("value".into(), json!(pv.value));
                m.insert(
                    "dataType".into(),
                    json!(format!("{:?}", pv.data_type).to_lowercase()),
                );
                if let Some(desc) = &pv.description {
                    m.insert("description".into(), json!(desc));
                }
                Value::Object(m)
            })
            .collect(),
    )
}

/// Requestly variables record — `Record<string, variableBase>`.
pub fn variables_record(vars: &[Variable]) -> Value {
    let mut map = serde_json::Map::new();
    for v in vars {
        map.insert(
            v.key.clone(),
            json!({
                "syncValue": v.value,
                "type": format!("{:?}", v.data_type).to_lowercase(),
                "isEnabled": v.enabled,
            }),
        );
    }
    Value::Object(map)
}

/// The `scripts` object (`{ preRequest?, postResponse? }`), or `None` when empty.
pub fn scripts_object(scripts: &Scripts) -> Option<Value> {
    if scripts.is_empty() {
        return None;
    }
    let mut m = serde_json::Map::new();
    if let Some(s) = &scripts.pre_request {
        m.insert("preRequest".into(), Value::String(s.source.clone()));
    }
    if let Some(s) = &scripts.post_response {
        m.insert("postResponse".into(), Value::String(s.source.clone()));
    }
    Some(Value::Object(m))
}

/// The result of mapping an IR [`Auth`] to a Requestly `authConfig`.
pub enum AuthMap {
    /// A mapped Requestly auth config.
    Mapped(Value),
    /// `Auth::None` — emit no auth.
    NoAuth,
    /// A kind cross-q can't emit to Requestly yet; the string describes it for a diagnostic.
    Unsupported(String),
}

/// Map an IR [`Auth`] to Requestly's `authConfigSchema` shape (discriminated on `type`).
/// Requestly's **auth convention**, applied at the reverse-conversion boundary
/// (IR → Requestly) to both collections and requests: an *unspecified* auth (`None`) means
/// "inherit from parent". This mirrors the app's `mapAuth(undefined) → { type: inherit }`
/// — a Requestly-*emitter* default, NOT an IR default. The IR stays neutral (`None` =
/// "source said nothing"); only this reverse converter injects Requestly's default. A
/// different target's emitter would choose its own default. `Some(Auth::None)` (explicit
/// no-auth) maps to `no_auth`; an unmappable kind yields `None` (the caller reports it).
pub fn requestly_auth_value(auth: &Option<Auth>) -> Option<Value> {
    match auth {
        None => Some(json!({ "type": "inherit" })),
        Some(a) => match auth_to_rq(a) {
            AuthMap::Mapped(v) => Some(v),
            AuthMap::NoAuth => Some(json!({ "type": "no_auth" })),
            AuthMap::Unsupported(_) => None,
        },
    }
}

pub fn auth_to_rq(auth: &Auth) -> AuthMap {
    match auth {
        Auth::None => AuthMap::NoAuth,
        Auth::Inherit => AuthMap::Mapped(json!({ "type": "inherit" })),
        Auth::Basic { username, password } => AuthMap::Mapped(json!({
            "type": "basic_auth",
            "username": username,
            "password": password,
        })),
        Auth::Bearer {
            token,
            header_prefix,
        } => {
            // Tri-state header_prefix: None → omit the field entirely (Requestly's default,
            // matches the app which never emits it); Some(x) → emit the explicit prefix.
            let mut m = serde_json::Map::new();
            m.insert("type".into(), json!("bearer_token"));
            m.insert("token".into(), json!(token));
            if let Some(p) = header_prefix {
                m.insert("headerPrefix".into(), json!(p));
            }
            AuthMap::Mapped(Value::Object(m))
        }
        Auth::ApiKey {
            key,
            value,
            placement,
        } => AuthMap::Mapped(json!({
            "type": "api_key",
            "key": key,
            "value": value,
            "placement": format!("{placement:?}").to_lowercase(),
        })),
        other => AuthMap::Unsupported(format!("{other:?}")),
    }
}

/// Assemble the Requestly `httpRequestSchema` object (the `request` field of an HTTP
/// `apiEntry`): `{ url, method, headers, queryParams, pathVariables, body, contentType }`.
pub fn http_request_object(http: &HttpRequest) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("url".into(), Value::String(http.url.raw.clone()));
    obj.insert(
        "method".into(),
        Value::String(String::from(http.method.clone())),
    );
    obj.insert("headers".into(), kvs_to_json(&http.headers));
    obj.insert("queryParams".into(), kvs_to_json(&http.query));
    obj.insert(
        "pathVariables".into(),
        path_vars_to_json(&http.path_variables),
    );
    obj.insert(
        "contentType".into(),
        Value::String(content_type_selector(&http.body).to_string()),
    );
    if let Some(body) = &http.body {
        obj.insert("body".into(), body_to_json(body));
    } else {
        obj.insert("body".into(), json!({ "contentType": "none" }));
    }
    Value::Object(obj)
}
