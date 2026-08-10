//! IR → Requestly on-the-wire JSON shapes, shared by both Requestly serializations:
//! the `LOCAL_FS` file tree ([`crate::emit_rq`]) and the in-memory `MappedItems` bundle
//! ([`crate::mappeditems`]). One IR→Requestly mapper, two serializations — so a fix lands
//! once. Matches `docs/FORMAT.md` (schema `1.12.0`).

use serde_json::{json, Value};

use cq_model::{Auth, Body, FileRef, FormField, HttpRequest, KeyValue, PathVar, Scripts, Variable};

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
        Body::FormData { fields } => json!({
            "contentType": "multipart/form-data",
            "formData": form_data_to_json(fields),
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

/// Requestly multipart `formData[]` — `{ id, key, value, isEnabled, type }` (`type` is
/// `"text"` or `"file"`), matching the app's form-data mapping.
fn form_data_to_json(fields: &[FormField]) -> Value {
    Value::Array(
        fields
            .iter()
            .enumerate()
            .map(|(i, f)| match f {
                FormField::Text(kv) => json!({
                    "id": i as u64,
                    "key": kv.key,
                    "value": kv.value,
                    "isEnabled": kv.enabled,
                    "type": "text",
                }),
                FormField::File(file) => {
                    let (name, path) = match file {
                        FileRef::Reference { name, path, .. } => (name.as_str(), path.as_str()),
                        FileRef::Content { name, .. } => (name.as_str(), ""),
                    };
                    json!({
                        "id": i as u64,
                        "key": name,
                        "value": path,
                        "isEnabled": true,
                        "type": "file",
                    })
                }
            })
            .collect(),
    )
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
        Auth::Digest { params } => {
            let g = |k: &str| params.get(k).cloned().unwrap_or_default();
            AuthMap::Mapped(json!({
                "type": "digest_auth",
                "username": g("username"),
                "password": g("password"),
                "algorithm": canon_enum(
                    &g("algorithm"),
                    &["MD5", "MD5-sess", "SHA-256", "SHA-256-sess", "SHA-512-256", "SHA-512-256-sess"],
                    "MD5",
                ),
                "qop": canon_enum(&g("qop"), &["auth", "auth-int"], "auth"),
                "realm": "", "nonce": "", "nonceCount": "", "clientNonce": "",
                "opaque": "", "disableRetry": false,
            }))
        }
        Auth::OAuth1 { params } => auth_oauth1(params),
        Auth::OAuth2 { params, .. } => auth_oauth2(params),
        other => AuthMap::Unsupported(format!("{other:?}")),
    }
}

/// Requestly's default OAuth2 callback URL (matches the app's `DEFAULT_CALLBACK_URL`).
const DEFAULT_CALLBACK_URL: &str = "https://oauth.rqstag.com/callback";

/// Map Postman OAuth 2.0 params → Requestly `oauth_2` (FR-25), a nested grant-specific
/// `config`. Fields pass through verbatim (empty strings are legal). An absent/unknown grant
/// salvages: hosted-flow fields → `authorization_code`; a pasted token → `manual`; otherwise
/// `inherit`. Mirrors the app's `mapPostmanOAuth2`.
fn auth_oauth2(params: &std::collections::BTreeMap<String, String>) -> AuthMap {
    let g = |k: &str| params.get(k).cloned().unwrap_or_default();
    let auth_url = g("authUrl");
    // v2.1 uses `accessTokenUrl`; older exports/Newman use `tokenUrl`.
    let token_url = {
        let t = g("accessTokenUrl");
        if t.is_empty() {
            g("tokenUrl")
        } else {
            t
        }
    };
    let callback_url = {
        let c = g("callBackUrl");
        if c.is_empty() {
            DEFAULT_CALLBACK_URL.to_string()
        } else {
            c
        }
    };
    let client_id = g("clientId");
    let client_secret = g("clientSecret");
    let scope = g("scope");
    let state = g("state");

    // The hosted authorization-code config (also the salvage target for an omitted grant).
    let hosted_auth_code = || {
        let mut c = serde_json::Map::new();
        c.insert("grantType".into(), json!("authorization_code"));
        c.insert("authUrl".into(), json!(auth_url));
        c.insert("tokenUrl".into(), json!(token_url));
        c.insert("callbackUrl".into(), json!(callback_url));
        c.insert("clientId".into(), json!(client_id));
        c.insert("clientSecret".into(), json!(client_secret));
        c.insert("scope".into(), json!(scope));
        if !state.is_empty() {
            c.insert("state".into(), json!(state));
        }
        c.insert("mode".into(), json!("hosted"));
        Value::Object(c)
    };

    let config = match g("grant_type").as_str() {
        "authorization_code" => hosted_auth_code(),
        "authorization_code_with_pkce" => {
            let challenge = if g("challengeAlgorithm") == "plain" {
                "plain"
            } else {
                "S256"
            };
            let mut c = serde_json::Map::new();
            c.insert("grantType".into(), json!("authorization_code_pkce"));
            c.insert("authUrl".into(), json!(auth_url));
            c.insert("tokenUrl".into(), json!(token_url));
            c.insert("callbackUrl".into(), json!(callback_url));
            c.insert("clientId".into(), json!(client_id));
            if !client_secret.is_empty() {
                c.insert("clientSecret".into(), json!(client_secret));
            }
            c.insert("challengeMethod".into(), json!(challenge));
            c.insert("scope".into(), json!(scope));
            if !state.is_empty() {
                c.insert("state".into(), json!(state));
            }
            c.insert("mode".into(), json!("hosted"));
            Value::Object(c)
        }
        "client_credentials" => json!({
            "grantType": "client_credentials", "tokenUrl": token_url,
            "clientId": client_id, "clientSecret": client_secret, "scope": scope,
        }),
        "password_credentials" => {
            let mut c = serde_json::Map::new();
            c.insert("grantType".into(), json!("password"));
            c.insert("tokenUrl".into(), json!(token_url));
            c.insert("clientId".into(), json!(client_id));
            if !client_secret.is_empty() {
                c.insert("clientSecret".into(), json!(client_secret));
            }
            c.insert("username".into(), json!(g("username")));
            c.insert("password".into(), json!(g("password")));
            c.insert("scope".into(), json!(scope));
            Value::Object(c)
        }
        "implicit" => {
            let mut c = serde_json::Map::new();
            c.insert("grantType".into(), json!("implicit"));
            c.insert("authUrl".into(), json!(auth_url));
            c.insert("callbackUrl".into(), json!(callback_url));
            c.insert("clientId".into(), json!(client_id));
            c.insert("scope".into(), json!(scope));
            if !state.is_empty() {
                c.insert("state".into(), json!(state));
            }
            c.insert("mode".into(), json!("hosted"));
            Value::Object(c)
        }
        // No/unknown grant — salvage in priority order (matches the app's `default`).
        _ => {
            if !auth_url.is_empty() || !token_url.is_empty() {
                hosted_auth_code()
            } else {
                let token = g("accessToken");
                if token.is_empty() {
                    return AuthMap::Mapped(json!({ "type": "inherit" }));
                }
                json!({ "grantType": "manual", "token": token })
            }
        }
    };
    AuthMap::Mapped(json!({ "type": "oauth_2", "config": config }))
}

/// Case-insensitively match `value` against `allowed`, returning the canonical entry, or
/// `default` — mirrors the app's `canonicalEnum` (used for digest algorithm/qop).
fn canon_enum(value: &str, allowed: &[&str], default: &str) -> String {
    allowed
        .iter()
        .find(|a| a.eq_ignore_ascii_case(value))
        .copied()
        .unwrap_or(default)
        .to_string()
}

/// Map Postman OAuth 1.0 params → Requestly `oauth1` (FR-24). An unsupported signature
/// method falls back to `inherit`, matching the app.
fn auth_oauth1(params: &std::collections::BTreeMap<String, String>) -> AuthMap {
    let g = |k: &str| params.get(k).cloned().unwrap_or_default();
    let b = |k: &str, d: bool| params.get(k).map(|v| v == "true").unwrap_or(d);

    let raw_method = {
        let m = g("signatureMethod");
        if m.is_empty() {
            "HMAC-SHA1".to_string()
        } else {
            m
        }
    };
    const SUPPORTED: &[&str] = &[
        "HMAC-SHA1",
        "HMAC-SHA256",
        "HMAC-SHA512",
        "PLAINTEXT",
        "RSA-SHA1",
        "RSA-SHA256",
        "RSA-SHA512",
    ];
    if !SUPPORTED.contains(&raw_method.as_str()) {
        // Unsupported signature method → inherit (the app does the same, with a warning).
        return AuthMap::Mapped(json!({ "type": "inherit" }));
    }
    let signing = if raw_method.starts_with("RSA") {
        json!({ "signatureMethod": raw_method, "privateKey": g("privateKey") })
    } else {
        json!({ "signatureMethod": raw_method })
    };
    let mut cfg = serde_json::Map::new();
    cfg.insert("type".into(), json!("oauth_1"));
    cfg.insert("consumerKey".into(), json!(g("consumerKey")));
    cfg.insert("consumerSecret".into(), json!(g("consumerSecret")));
    cfg.insert("accessToken".into(), json!(g("token"))); // Postman calls it `token`
    cfg.insert("tokenSecret".into(), json!(g("tokenSecret")));
    cfg.insert("signing".into(), signing);
    cfg.insert(
        "parameterTransmission".into(),
        json!(if b("addParamsToHeader", true) {
            "header"
        } else {
            "body"
        }),
    );
    cfg.insert("includeBodyHash".into(), json!(b("bodyHash", false)));
    cfg.insert(
        "addEmptyParametersToSignature".into(),
        json!(b("addEmptyParametersToSignature", false)),
    );
    cfg.insert(
        "encodeOAuthParametersInHeader".into(),
        json!(b("encodeOAuthParametersInHeader", true)),
    );
    let realm = g("realm");
    if !realm.is_empty() {
        cfg.insert("realm".into(), json!(realm));
    }
    AuthMap::Mapped(Value::Object(cfg))
}

/// Assemble the Requestly `httpRequestSchema` object (the `request` field of an HTTP
/// `apiEntry`): `{ url, method, headers, queryParams, pathVariables, body, contentType }`.
/// Requestly's `RequestMethod` is a **closed enum** (GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS)
/// validated by the persistence schema. Any other method (TRACE, WebDAV verbs like COPY, a
/// custom method) would be rejected on import — so, at the Requestly boundary only, coerce
/// the unknowns to GET, matching the app's `mapHttpMethodResult`. (Postman export keeps the
/// verbatim method — this coercion is Requestly-specific.)
fn requestly_method(method: &cq_model::Method) -> String {
    let m = String::from(method.clone());
    match m.as_str() {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" => m,
        _ => "GET".to_string(),
    }
}

pub fn http_request_object(http: &HttpRequest) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("url".into(), Value::String(http.url.raw.clone()));
    obj.insert(
        "method".into(),
        Value::String(requestly_method(&http.method)),
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
