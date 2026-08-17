//! IR → Requestly on-the-wire JSON shapes, shared by both Requestly serializations:
//! the `LOCAL_FS` file tree ([`crate::emit_rq`]) and the in-memory `MappedItems` bundle
//! ([`crate::mappeditems`]). One IR→Requestly mapper, two serializations — so a fix lands
//! once. Matches `docs/FORMAT.md` (schema `1.12.0`).

use serde_json::{json, Value};

use cq_model::{Auth, Body, FileRef, FormField, HttpRequest, KeyValue, PathVar, Scripts, Variable};

/// The Requestly `LOCAL_FS` schema version these shapes target.
pub const RQ_SCHEMA_VERSION: &str = "1.12.0";

/// The server's example/name length cap (`MAX_EXAMPLE_NAME_LENGTH`) — a transactional
/// bulk-create rejects one over-long name for the whole import (RQ-5357).
pub const MAX_NAME_LENGTH: usize = 255;

/// Truncate a name to `max` **UTF-16 code units** with a trailing `…`, never slicing a
/// surrogate pair — a byte-exact port of the app's `truncateName` (JS `.length` is UTF-16).
pub fn truncate_name(name: &str, max: usize) -> String {
    let units: Vec<u16> = name.encode_utf16().collect();
    if units.len() <= max {
        return name.to_string();
    }
    let mut cut = max - 1;
    // If the last kept unit is a lone high surrogate, drop it (no `�`).
    if cut >= 1 && (0xd800..=0xdbff).contains(&units[cut - 1]) {
        cut -= 1;
    }
    format!("{}…", String::from_utf16_lossy(&units[..cut]))
}

// --- saved-response cookies (ADR-107) --------------------------------------
//
// Port of the app's `collectCookies` / `postmanCookieToCookie` / `structuredCookieToCookie`.
// Postman saved responses carry a `cookie[]`; each becomes a Requestly `Cookie` resolved
// against the response's request URL (its own `originalRequest`, else the parent request).
// These ride `mapped.cookies` (the SDK routes them to the device-local cookie jar, never
// bulk.create). CHIPS partition state lives in three flat `extraAttributes` keys.

/// Extract Requestly cookies from a Postman saved-response object's `cookie[]`, resolved
/// against `request_url`. Nameless placeholder cookies (a Postman saved-response artifact) are
/// dropped, matching the app.
pub fn cookies_from_response(response: &Value, request_url: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(Value::Array(arr)) = response.get("cookie") {
        for pm in arr {
            if let Some(c) = postman_cookie_to_cookie(pm, request_url) {
                out.push(c);
            }
        }
    }
    out
}

fn postman_cookie_to_cookie(pm: &Value, request_url: &str) -> Option<Value> {
    // A nameless cookie is meaningless to us — drop it (mirrors the app). An empty *value* is
    // legal; only the name gates the drop. `??`-style nullish defaults: a present-but-empty
    // domain/path is kept; an absent one falls back (RFC 6265 §5.2).
    let name = pm
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let value = pm.get("value").and_then(Value::as_str).unwrap_or("");
    let domain = pm
        .get("domain")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| url_host(request_url));
    let path = pm.get("path").and_then(Value::as_str).unwrap_or("/");
    let secure = pm.get("secure").and_then(Value::as_bool).unwrap_or(false);
    let http_only = pm.get("httpOnly").and_then(Value::as_bool).unwrap_or(false);

    let mut cookie = serde_json::Map::new();
    cookie.insert("kind".into(), json!("core-v1"));
    cookie.insert("name".into(), json!(name));
    cookie.insert("value".into(), json!(value));
    cookie.insert("domain".into(), json!(domain));
    cookie.insert("path".into(), json!(path));
    cookie.insert("secure".into(), json!(secure));
    cookie.insert("httpOnly".into(), json!(http_only));
    cookie.insert("expiry".into(), cookie_expiry(pm.get("expires")));

    // CHIPS partition key (ADR-107): only when `partitioned === true` and the request URL is
    // http(s) — the three-key `extraAttributes` invariant, matching `setPartitionKey`.
    if pm.get("partitioned").and_then(Value::as_bool) == Some(true) {
        if let Some((scheme, top_level_site)) = url_top_level_site(request_url) {
            cookie.insert(
                "extraAttributes".into(),
                json!({
                    "Partitioned": true,
                    "PartitionScheme": scheme,
                    "PartitionTopLevelSite": top_level_site,
                }),
            );
        }
    }
    Some(Value::Object(cookie))
}

/// Postman `expires` (string | number | absent) → Requestly `expiry`. A finite number is epoch
/// SECONDS (Postman's serialization) → absolute ISO; a parseable ISO string → absolute; empty /
/// absent / unparseable → session. Mirrors `normalisePostmanExpiry` + `parseExpires`.
fn cookie_expiry(expires: Option<&Value>) -> Value {
    match expires {
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.is_finite() => {
                json!({ "type": "absolute", "date": epoch_ms_to_iso((f * 1000.0) as i64) })
            }
            _ => json!({ "type": "session" }),
        },
        Some(Value::String(s)) => {
            let t = s.trim();
            match if t.is_empty() { None } else { canonical_iso(t) } {
                Some(iso) => json!({ "type": "absolute", "date": iso }),
                None => json!({ "type": "session" }),
            }
        }
        _ => json!({ "type": "session" }),
    }
}

/// Dedupe cookies last-write-wins keyed by name+domain+path (mirrors
/// `dedupeCookiesLastWriteWins`), preserving first-seen order.
pub fn dedupe_cookies(cookies: Vec<Value>) -> Vec<Value> {
    use std::collections::HashMap;
    let mut order: Vec<String> = Vec::new();
    let mut by_key: HashMap<String, Value> = HashMap::new();
    for c in cookies {
        let key = format!(
            "{}\u{0}{}\u{0}{}",
            c.get("name").and_then(Value::as_str).unwrap_or(""),
            c.get("domain").and_then(Value::as_str).unwrap_or(""),
            c.get("path").and_then(Value::as_str).unwrap_or(""),
        );
        if !by_key.contains_key(&key) {
            order.push(key.clone());
        }
        by_key.insert(key, c);
    }
    order
        .into_iter()
        .filter_map(|k| by_key.remove(&k))
        .collect()
}

/// The request URL's host — `new URL(url).hostname`, with the app's fallback: an unparseable
/// URL (e.g. a bare `{{var}}`) yields the whole string.
fn url_host(raw: &str) -> String {
    match split_scheme_host(raw) {
        Some((_, host)) => host,
        None => raw.to_string(),
    }
}

/// The CHIPS top-level site — `(scheme, "scheme://host")` — only for an http(s) URL that parses.
fn url_top_level_site(raw: &str) -> Option<(String, String)> {
    let (scheme, host) = split_scheme_host(raw)?;
    if scheme == "https" || scheme == "http" {
        Some((scheme.clone(), format!("{scheme}://{host}")))
    } else {
        None
    }
}

/// Split `scheme://[userinfo@]host[:port][/...]` into `(scheme, host)`. `None` when there is no
/// `://` or the host is empty (mirrors `new URL()` throwing).
fn split_scheme_host(raw: &str) -> Option<(String, String)> {
    let idx = raw.find("://")?;
    let scheme = raw[..idx].to_ascii_lowercase();
    let rest = &raw[idx + 3..];
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = host_port.split(':').next().unwrap_or(host_port);
    if host.is_empty() {
        None
    } else {
        Some((scheme, host.to_string()))
    }
}

/// Validate + canonicalize an ISO-8601 UTC timestamp to `YYYY-MM-DDTHH:MM:SS.sssZ` (what JS
/// `new Date(s).toISOString()` yields). Returns `None` for anything not in that basic form —
/// which falls back to a session cookie, never a wrong date.
fn canonical_iso(s: &str) -> Option<String> {
    // Expect `YYYY-MM-DDTHH:MM:SS` then optional `.fff` then optional `Z`.
    let bytes = s.as_bytes();
    if s.len() < 19 {
        return None;
    }
    let digits = |a: usize, b: usize| -> Option<u32> {
        let sub = s.get(a..b)?;
        if sub.bytes().all(|c| c.is_ascii_digit()) {
            sub.parse().ok()
        } else {
            None
        }
    };
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let (y, mo, d) = (digits(0, 4)?, digits(5, 7)?, digits(8, 10)?);
    let (h, mi, se) = (digits(11, 13)?, digits(14, 16)?, digits(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 59 {
        return None;
    }
    // Milliseconds: first up to 3 fractional digits after '.', if present.
    let mut ms = 0u32;
    if bytes.get(19) == Some(&b'.') {
        let frac: String = s[20..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .take(3)
            .collect();
        if !frac.is_empty() {
            ms = format!("{frac:0<3}").parse().unwrap_or(0);
        }
    }
    Some(format!(
        "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{se:02}.{ms:03}Z"
    ))
}

/// Epoch milliseconds → `YYYY-MM-DDTHH:MM:SS.sssZ` (UTC), byte-identical to JS
/// `new Date(ms).toISOString()`. Civil-date-from-days per Howard Hinnant's algorithm; no crate.
fn epoch_ms_to_iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (h, mi, se, msec) = (
        rem / 3_600_000,
        (rem % 3_600_000) / 60_000,
        (rem % 60_000) / 1000,
        rem % 1000,
    );
    // civil_from_days
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{se:02}.{msec:03}Z")
}

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
            // An auth kind with no Requestly equivalent (edgegrid, an unknown type) falls
            // back to `inherit`, matching the app's `mapAuth` default — never dropped.
            AuthMap::Unsupported(_) => Some(json!({ "type": "inherit" })),
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
            "placement": match placement {
                cq_model::ApiKeyPlacement::Query => "query_param",
                cq_model::ApiKeyPlacement::Header => "header",
            },
        })),
        Auth::JwtBearer { algorithm, params } => {
            let g = |k: &str| params.get(k).cloned().unwrap_or_default();
            let non_empty = |s: String, dflt: &str| if s.is_empty() { dflt.to_string() } else { s };
            let add_to = g("addTokenTo");
            let attachment = if add_to == "queryParam" || add_to == "query" {
                json!({ "placement": "queryParam", "paramName": non_empty(g("queryParamKey"), "access_token") })
            } else {
                json!({ "placement": "header", "headerName": "Authorization", "prefix": non_empty(g("headerPrefix"), "Bearer") })
            };
            AuthMap::Mapped(json!({
                "type": "jwt_bearer",
                "algorithm": algorithm,
                "signingKey": g("secret"),
                "secretBase64Encoded": params.get("isSecretBase64Encoded").map(|v| v == "true").unwrap_or(false),
                "payload": non_empty(g("payload"), "{}"),
                "jwtHeader": non_empty(g("header"), "{}"),
                "attachment": attachment,
            }))
        }
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
        Auth::AwsSigV4 { params } => {
            let g = |k: &str| params.get(k).cloned().unwrap_or_default();
            let mut m = serde_json::Map::new();
            m.insert("type".into(), json!("aws_sigv4"));
            m.insert("signatureVersion".into(), json!("v4"));
            m.insert("accessKeyId".into(), json!(g("accessKey")));
            m.insert("secretAccessKey".into(), json!(g("secretKey")));
            m.insert("sessionToken".into(), json!(g("sessionToken")));
            m.insert("region".into(), json!(g("region")));
            m.insert("service".into(), json!(g("service")));
            m.insert("profileName".into(), json!(""));
            // `addAuthDataToQuery` → presigned-URL mode (expiry backfilled to the schema default).
            if params
                .get("addAuthDataToQuery")
                .map(|v| v == "true")
                .unwrap_or(false)
            {
                m.insert("attachment".into(), json!("presigned_url"));
                m.insert("expirySeconds".into(), json!(3600));
            } else {
                m.insert("attachment".into(), json!("live_request"));
            }
            AuthMap::Mapped(Value::Object(m))
        }
        Auth::Ntlm { params } => {
            let g = |k: &str| params.get(k).cloned().unwrap_or_default();
            AuthMap::Mapped(json!({
                "type": "ntlm",
                "username": g("username"), "password": g("password"),
                "domain": g("domain"), "workstation": g("workstation"),
            }))
        }
        Auth::Hawk { params } => {
            let g = |k: &str| params.get(k).cloned().unwrap_or_default();
            AuthMap::Mapped(json!({
                "type": "hawk",
                "authId": g("authId"), "authKey": g("authKey"),
                "algorithm": canon_enum(&g("algorithm"), &["sha1", "sha256"], "sha256"),
                "user": g("user"), "nonce": g("nonce"), "timestamp": g("timestamp"),
                "ext": g("extraData"), "app": g("app"), "dlg": g("dlg"),
                "includePayloadHash": params.get("includePayloadHash").map(|v| v == "true").unwrap_or(false),
            }))
        }
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
                    // Nothing to salvage — fall back to inherit AND flag it (the app warns
                    // `advanced_auth`/oauth2_unmappable_grant); Unsupported drives both.
                    return AuthMap::Unsupported("oauth2 (unmappable grant)".to_string());
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
        // Unsupported signature method → inherit AND flag it (the app warns `advanced_auth`/
        // oauth1_signature_method); Unsupported drives both the inherit fallback and the warning.
        return AuthMap::Unsupported("oauth1 (unsupported signature method)".to_string());
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

/// The Requestly **GraphQL** request object (for an `apiEntry` of `type: "graphql"`):
/// `{ url, method, headers, queryParams, query, variables?, operationName? }` — no `body`
/// or `contentType` (those are HTTP-only). Matches the app's graphql `buildRequestEntry` arm.
pub fn graphql_request_object(
    http: &HttpRequest,
    query: &str,
    variables: &str,
    operation_name: Option<&str>,
) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("url".into(), json!(http.url.raw));
    obj.insert("method".into(), json!(requestly_method(&http.method)));
    obj.insert("headers".into(), kvs_to_json(&http.headers));
    obj.insert("queryParams".into(), kvs_to_json(&http.query));
    obj.insert("query".into(), json!(query));
    if !variables.is_empty() {
        obj.insert("variables".into(), json!(variables));
    }
    if let Some(op) = operation_name {
        obj.insert("operationName".into(), json!(op));
    }
    Value::Object(obj)
}
