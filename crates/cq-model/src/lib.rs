//! # cq-model — the Idealised Model
//!
//! `cross-q`'s canonical intermediate representation (IR). Every importer maps a source
//! format *into* these types; every exporter maps *out of* them. Nothing converts to
//! another format directly — everything goes through here.
//!
//! The model is the **superset** of the API-client category, not the intersection: if any
//! tool can express a thing, the model can hold it. What a target can't represent is
//! dropped *loudly* by the exporter (a diagnostic), never silently. See `docs/IDEALISED.md`
//! for the full design rationale.
//!
//! ## Scope of this crate (v0.1)
//! The common envelope ([`RecordMeta`], [`Provenance`], [`ExtBag`]), the tree
//! ([`Workspace`] → [`Collection`] → [`Request`]), variables/environments, auth, scripts,
//! and chaining are complete. [`Protocol`] currently models `http` and `graphql`; gRPC,
//! MQTT, WebSocket, Socket.IO and SOAP are follow-up work (the enum grows additively).
//!
//! Conventions: fields are `snake_case` on the wire; maps are [`BTreeMap`] for
//! deterministic, byte-stable output (a git-diff-friendly IR).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Arbitrary JSON, used for the extension bag and for preserving source data we don't yet
/// model as first-class fields.
pub type Json = serde_json::Value;

/// Format-specific fields with no first-class home, kept verbatim so a round-trip back to
/// the *same* format is byte-stable. Keyed by the source format that produced them.
pub type ExtBag = BTreeMap<SourceFormat, Json>;

/// The model's own schema version, independent of any tool-format version.
pub const MODEL_VERSION: &str = "0.1.0";

fn default_true() -> bool {
    true
}

fn is_false(b: &bool) -> bool {
    !*b
}

// ---------------------------------------------------------------------------------------
// Provenance & envelope
// ---------------------------------------------------------------------------------------

/// Every format `cross-q` can read or write. Used as a provenance tag and as an
/// [`ExtBag`] key (it serializes to a plain snake_case string).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Postman,
    Insomnia,
    Bruno,
    Har,
    OpenApi,
    Curl,
    Requestly,
    Hoppscotch,
    ThunderClient,
    RestClient,
    Hurl,
    Wsdl,
    SoapUi,
    Dotenv,
    /// The IR itself, or an origin we couldn't attribute.
    #[default]
    Unknown,
}

/// Where a node came from — the source format plus a human-readable locator (e.g. a
/// Postman `item[3].request.auth` path). A diagnostic points back here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub format: SourceFormat,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub locator: String,
}

/// Common metadata carried by every node in the tree. Custody fields
/// (`created_*`/`owner_id`) are deliberately absent — the IR models content, not custody.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordMeta {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Tree edge; `None` = root of its collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Fractional-index string for sibling ordering — opaque, never parsed as a number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Soft-disable without deleting.
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(default)]
    pub source: Provenance,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: ExtBag,
}

impl RecordMeta {
    /// Convenience constructor for an id + name at a given source.
    pub fn new(id: impl Into<String>, name: impl Into<String>, format: SourceFormat) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            source: Provenance {
                format,
                ..Provenance::default()
            },
            ..Self::default()
        }
    }
}

/// The top of every converted document, plus the header that records how it was produced.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelHeader {
    pub model_version: String,
    pub source_format: SourceFormat,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub generated_by: String,
}

impl ModelHeader {
    pub fn for_source(source_format: SourceFormat) -> Self {
        Self {
            model_version: MODEL_VERSION.to_string(),
            source_format,
            generated_by: concat!("cq-model ", env!("CARGO_PKG_VERSION")).to_string(),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Workspace / Collection / Item
// ---------------------------------------------------------------------------------------

/// A whole converted document: the collections, environments and reusable packages that
/// made up the source, plus a [`ModelHeader`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub meta: RecordMeta,
    pub cross_q: ModelHeader,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collections: Vec<Collection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environments: Vec<Environment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<CodePackage>,
}

/// A folder of requests and sub-collections. `auth`/`scripts` here are inherited by
/// descendants unless a descendant overrides them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    pub meta: RecordMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
    /// Headers declared on the folder/collection and inherited by descendant requests
    /// (Bruno `collection.bru`/`folder.bru` `headers`, Insomnia folder headers).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<KeyValue>,
    #[serde(default, skip_serializing_if = "Scripts::is_empty")]
    pub scripts: Scripts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
}

/// An ordered child of a collection: either a request or a nested collection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "node", rename_all = "snake_case")]
pub enum Item {
    // Boxed: `Request`/`Collection` are large, and a `Vec<Item>` of unboxed variants
    // wastes space on every element. Serde treats `Box<T>` transparently.
    Request(Box<Request>),
    Collection(Box<Collection>),
}

/// A reusable JS module (Requestly/Postman "package").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodePackage {
    pub meta: RecordMeta,
    pub source: String,
    #[serde(default)]
    pub language: ScriptLang,
}

// ---------------------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------------------

/// A single named request. `auth: None` means "inherit from the enclosing collection".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub meta: RecordMeta,
    pub protocol: Protocol,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
    #[serde(default, skip_serializing_if = "Scripts::is_empty")]
    pub scripts: Scripts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<Example>,
    /// Declared chaining (the superset of Requestly `run_order` and rq's `parents:`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<Dependency>,
    /// Request-scoped variable operations and response assertions that several tools express
    /// as first-class blocks (Bruno `vars:*`/`assert`, Hurl `[Captures]`/`[Asserts]`).
    #[serde(default, skip_serializing_if = "RequestBehavior::is_empty")]
    pub behavior: RequestBehavior,
}

/// The protocol-specific payload of a request. Internally tagged on `type`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Protocol {
    Http(HttpRequest),
    Graphql(GraphQlRequest),
}

/// HTTP method. Known verbs serialize as their uppercase name; anything else round-trips
/// verbatim through [`Method::Other`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
#[derive(Default)]
pub enum Method {
    #[default]
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
    Trace,
    Other(String),
}

impl From<String> for Method {
    fn from(s: String) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "PATCH" => Method::Patch,
            "DELETE" => Method::Delete,
            "HEAD" => Method::Head,
            "OPTIONS" => Method::Options,
            "TRACE" => Method::Trace,
            _ => Method::Other(s),
        }
    }
}

impl From<Method> for String {
    fn from(m: Method) -> Self {
        match m {
            Method::Get => "GET".into(),
            Method::Post => "POST".into(),
            Method::Put => "PUT".into(),
            Method::Patch => "PATCH".into(),
            Method::Delete => "DELETE".into(),
            Method::Head => "HEAD".into(),
            Method::Options => "OPTIONS".into(),
            Method::Trace => "TRACE".into(),
            Method::Other(s) => s,
        }
    }
}

/// A URL kept both verbatim (`raw`, templates intact) and, best-effort, parsed. Both are
/// carried so a structured-query format and a string-URL format each round-trip.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Url {
    pub raw: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl Url {
    pub fn raw(raw: impl Into<String>) -> Self {
        Self {
            raw: raw.into(),
            ..Self::default()
        }
    }
}

/// A header, query param, or form field. `.passthrough`-style extras that a source carries
/// live in `ext`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyValue {
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub kind: KvKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: ExtBag,
}

impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            enabled: true,
            kind: KvKind::Text,
            description: None,
            ext: ExtBag::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KvKind {
    #[default]
    Text,
    File,
    Secret,
}

/// A `:name` / `{name}` path variable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathVar {
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(default)]
    pub data_type: ScalarType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    #[default]
    String,
    Number,
    Integer,
    Boolean,
}

/// Per-request transport settings. Defaults are the sane HTTP defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestSettings {
    #[serde(default = "default_true")]
    pub follow_redirects: bool,
    #[serde(default = "default_true")]
    pub verify_tls: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default = "default_true")]
    pub encode_url: bool,
}

impl Default for RequestSettings {
    fn default() -> Self {
        Self {
            follow_redirects: true,
            verify_tls: true,
            timeout_ms: None,
            encode_url: true,
        }
    }
}

impl RequestSettings {
    fn is_default(&self) -> bool {
        *self == RequestSettings::default()
    }
}

/// An HTTP (or HTTP-shaped) request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: Method,
    pub url: Url,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<KeyValue>,
    /// Kept separate from `url.raw` so both a structured-query and a string-URL source
    /// round-trip without re-parsing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query: Vec<KeyValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_variables: Vec<PathVar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Body>,
    #[serde(default, skip_serializing_if = "RequestSettings::is_default")]
    pub settings: RequestSettings,
}

/// A GraphQL request. `variables` is kept as a *string* (templates are substituted before
/// it is parsed as JSON) — never as a parsed object.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphQlRequest {
    pub url: Url,
    #[serde(default)]
    pub method: Method,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<KeyValue>,
    pub query: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub variables: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
}

// ---------------------------------------------------------------------------------------
// Body
// ---------------------------------------------------------------------------------------

/// A request body. Internally tagged on `kind`. `raw` carries its media type so JSON, XML,
/// text etc. are one lossless variant rather than many.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    None,
    Raw {
        text: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        media_type: String,
    },
    FormData {
        fields: Vec<FormField>,
    },
    UrlEncoded {
        fields: Vec<KeyValue>,
    },
    Binary {
        file: FileRef,
    },
    Graphql {
        query: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        variables: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operation_name: Option<String>,
    },
}

/// A multipart field: either a text key/value or a file part.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FormField {
    Text(KeyValue),
    File(FileRef),
}

/// A file reference in a body. Only [`FileRef::Reference`] is persisted; `Content` is a
/// transient variant resolved at send time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ref", rename_all = "snake_case")]
pub enum FileRef {
    Reference {
        id: String,
        name: String,
        path: String,
        #[serde(default)]
        size: u64,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        source: String,
    },
    Content {
        id: String,
        name: String,
        bytes: Vec<u8>,
        #[serde(default)]
        size: u64,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        source: String,
    },
}

// ---------------------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------------------

/// Where an API key is placed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyPlacement {
    #[default]
    Header,
    Query,
}

/// The auth attached to a request or collection. Internally tagged on `kind`. Types the IR
/// doesn't model are preserved as [`Auth::Unknown`] so a credential is never stripped just
/// because it wasn't understood.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Auth {
    None,
    Inherit,
    Basic {
        username: String,
        password: String,
    },
    Bearer {
        token: String,
        /// Tri-state: `None` → emit no prefix, `Some("Bearer")` → default, `Some(x)` →
        /// custom. Collapsing it silently changes requests.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        header_prefix: Option<String>,
    },
    ApiKey {
        key: String,
        value: String,
        #[serde(default)]
        placement: ApiKeyPlacement,
    },
    OAuth2 {
        grant: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, String>,
    },
    OAuth1 {
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, String>,
    },
    JwtBearer {
        /// `String`, not an enum, so a `{{var}}` is allowed.
        algorithm: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, String>,
    },
    Digest {
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, String>,
    },
    Hawk {
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, String>,
    },
    AwsSigV4 {
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, String>,
    },
    Ntlm {
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, String>,
    },
    EdgeGrid {
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, String>,
    },
    Unknown {
        raw_type: String,
        raw: Json,
    },
}

// ---------------------------------------------------------------------------------------
// Variables & environments
// ---------------------------------------------------------------------------------------

/// Resolution scope, highest-precedence last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Global,
    Collection,
    Environment,
    Runtime,
    Top,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VarType {
    #[default]
    String,
    Number,
    Boolean,
    Secret,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VarCategory {
    #[default]
    Scoped,
    Dynamic,
    Vault,
}

/// A single variable. For `category = Vault` the `value` is a key reference, never the
/// secret itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variable {
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    /// Postman's "initial" vs "current" split, when a source distinguishes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial: Option<String>,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub data_type: VarType,
    #[serde(default)]
    pub category: VarCategory,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<String>,
}

/// A named set of variables (an environment/globals file).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    pub meta: RecordMeta,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_global: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,
}

// ---------------------------------------------------------------------------------------
// Scripts, examples, chaining
// ---------------------------------------------------------------------------------------

/// Pre-request and post-response scripts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scripts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_request: Option<Script>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_response: Option<Script>,
}

impl Scripts {
    pub fn is_empty(&self) -> bool {
        self.pre_request.is_none() && self.post_response.is_none()
    }
}

/// A script plus the namespace dialect it is written against. `cross-q` records the
/// dialect rather than blindly rewriting `pm.` → `rq.`; real translation is `cross-q-context`'s
/// job (see `docs/CONTEXT.md`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Script {
    pub source: String,
    #[serde(default)]
    pub language: ScriptLang,
    #[serde(default)]
    pub dialect: ScriptDialect,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptLang {
    #[default]
    JavaScript,
    Other,
}

/// Which SDK namespace a script targets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptDialect {
    Rq,
    Pm,
    Bru,
    Hurl,
    #[default]
    Raw,
}

/// A saved response / example attached to a request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Example {
    pub meta: RecordMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<Json>,
}

/// A declared dependency on another request, and how that request's outputs bind into
/// this one's variables.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// The `id` of the request this one depends on.
    pub target: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binds: Vec<VarBinding>,
}

/// Maps an output of a dependency (`from`, e.g. a JSON path) to a variable (`to`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VarBinding {
    pub from: String,
    pub to: String,
}

/// Request-scoped behaviors that several API clients express as first-class blocks, distinct
/// from free-form scripts: variables set before a request or captured from its response, and
/// assertions checked against the response. Sources that fold these into scripts (Postman)
/// leave this empty; sources that make them first-class (Bruno `vars:*`/`assert`, Hurl
/// `[Captures]`/`[Asserts]`) populate it, so the distinction round-trips.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestBehavior {
    /// Variables set before the request runs (Bruno `vars:pre-request`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_request_vars: Vec<Variable>,
    /// Variables captured from the response (Bruno `vars:post-response`, Hurl `[Captures]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_response_vars: Vec<Variable>,
    /// Response assertions (Bruno `assert`, Hurl `[Asserts]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asserts: Vec<Assertion>,
}

impl RequestBehavior {
    pub fn is_empty(&self) -> bool {
        self.pre_request_vars.is_empty()
            && self.post_response_vars.is_empty()
            && self.asserts.is_empty()
    }
}

/// A response assertion: an expression and the predicate it must satisfy, both kept as
/// strings so any assertion DSL round-trips verbatim (Bruno writes `res.status: eq 200`,
/// i.e. `expr = "res.status"`, `predicate = "eq 200"`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assertion {
    pub expr: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub predicate: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_http_request() -> Request {
        Request {
            meta: RecordMeta::new("req-1", "list issues", SourceFormat::Postman),
            protocol: Protocol::Http(HttpRequest {
                method: Method::Get,
                url: Url::raw("https://api.github.com/repos/{{owner}}/{{repo}}/issues"),
                headers: vec![KeyValue::new("Accept", "application/vnd.github+json")],
                query: vec![KeyValue::new("state", "open")],
                body: Some(Body::Raw {
                    text: "{}".into(),
                    media_type: "application/json".into(),
                }),
                ..HttpRequest::default()
            }),
            auth: Some(Auth::Bearer {
                token: "{{GH_TOKEN}}".into(),
                header_prefix: Some("Bearer".into()),
            }),
            scripts: Scripts {
                post_response: Some(Script {
                    source: "rq.test('ok', () => rq.response.to.have.status(200));".into(),
                    language: ScriptLang::JavaScript,
                    dialect: ScriptDialect::Rq,
                }),
                ..Scripts::default()
            },
            examples: Vec::new(),
            depends_on: vec![Dependency {
                target: "login".into(),
                binds: vec![VarBinding {
                    from: "body.access_token".into(),
                    to: "token".into(),
                }],
            }],
            behavior: RequestBehavior::default(),
        }
    }

    fn sample_workspace() -> Workspace {
        Workspace {
            meta: RecordMeta::new("ws", "GitHub", SourceFormat::Postman),
            cross_q: ModelHeader::for_source(SourceFormat::Postman),
            collections: vec![Collection {
                meta: RecordMeta::new("c1", "GitHub API", SourceFormat::Postman),
                items: vec![Item::Request(Box::new(sample_http_request()))],
                variables: vec![Variable {
                    key: "owner".into(),
                    value: "anthropics".into(),
                    initial: None,
                    scope: Scope::Collection,
                    data_type: VarType::String,
                    category: VarCategory::Scoped,
                    enabled: true,
                    rank: None,
                }],
                ..Collection::default()
            }],
            environments: vec![Environment {
                meta: RecordMeta::new("env-prod", "prod", SourceFormat::Postman),
                is_global: false,
                variables: vec![Variable {
                    key: "GH_TOKEN".into(),
                    value: "vault:gh_token".into(),
                    initial: None,
                    scope: Scope::Environment,
                    data_type: VarType::Secret,
                    category: VarCategory::Vault,
                    enabled: true,
                    rank: None,
                }],
            }],
            packages: Vec::new(),
        }
    }

    #[test]
    fn workspace_round_trips_through_json() {
        let ws = sample_workspace();
        let json = serde_json::to_string_pretty(&ws).expect("serialize");
        let back: Workspace = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            ws, back,
            "workspace must survive a JSON round-trip unchanged"
        );
    }

    #[test]
    fn serialization_is_deterministic() {
        // BTreeMap ordering + stable field order => byte-stable output (git-diff friendly).
        let ws = sample_workspace();
        let a = serde_json::to_string(&ws).unwrap();
        let b = serde_json::to_string(&ws).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn protocol_is_tagged_by_type() {
        let req = sample_http_request();
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["protocol"]["type"], serde_json::json!("http"));
        assert_eq!(v["protocol"]["method"], serde_json::json!("GET"));
    }

    #[test]
    fn unknown_method_round_trips_verbatim() {
        let m: Method = "PROPFIND".to_string().into();
        assert_eq!(m, Method::Other("PROPFIND".into()));
        let s: String = m.into();
        assert_eq!(s, "PROPFIND");
    }

    #[test]
    fn bearer_prefix_is_tristate() {
        // Absent prefix and explicit prefix must serialize differently.
        let no_prefix = serde_json::to_value(Auth::Bearer {
            token: "t".into(),
            header_prefix: None,
        })
        .unwrap();
        let with_prefix = serde_json::to_value(Auth::Bearer {
            token: "t".into(),
            header_prefix: Some("Token".into()),
        })
        .unwrap();
        assert!(no_prefix.get("header_prefix").is_none());
        assert_eq!(with_prefix["header_prefix"], serde_json::json!("Token"));
    }

    #[test]
    fn unknown_auth_preserves_raw() {
        let raw = serde_json::json!({"foo": "bar"});
        let auth = Auth::Unknown {
            raw_type: "edgegrid_v2".into(),
            raw: raw.clone(),
        };
        let back: Auth = serde_json::from_value(serde_json::to_value(&auth).unwrap()).unwrap();
        assert_eq!(auth, back);
    }

    #[test]
    fn empty_optionals_are_omitted_from_output() {
        // A minimal request should not emit empty vecs/maps/defaults.
        let req = Request {
            meta: RecordMeta::new("r", "r", SourceFormat::Curl),
            protocol: Protocol::Http(HttpRequest {
                method: Method::Get,
                url: Url::raw("https://example.com"),
                ..HttpRequest::default()
            }),
            auth: None,
            scripts: Scripts::default(),
            examples: Vec::new(),
            depends_on: Vec::new(),
            behavior: RequestBehavior::default(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("\"headers\""), "empty headers omitted");
        assert!(!json.contains("\"auth\""), "None auth omitted");
        assert!(!json.contains("\"settings\""), "default settings omitted");
        assert!(!json.contains("\"ext\""), "empty ext bag omitted");
    }
}
