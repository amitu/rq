# The Idealised Model — `cross-q`'s canonical IR

> One model to which every API-client format maps, and from which every format is
> emitted. It is the **superset** of the whole Postman/Requestly/Insomnia/Bruno/HAR/
> OpenAPI/cURL category — not the intersection. If any tool in the category can express
> a thing, the Idealised Model can hold it. If a target can't, the emitter drops it
> **loudly** (a diagnostic), never silently.
>
> This is the contract. It lives in `crates/cq-model` as Rust types + a versioned JSON
> Schema. Importers are written *into* it; exporters are written *out of* it; neither
> talks to another format directly. Read this before writing either.

---

## How to read this document

- §1 — the one idea (hub, superset, lossy-but-honest).
- §2 — the noun model (Workspace → Collection → Request → …). Start here.
- §3–§9 — each noun in field-level detail. These are the schema.
- §10 — the **extension bag**, provenance, and how nothing is ever silently lost.
- §11 — fidelity levels and the round-trip contract.
- §12 — versioning, §13 — what this is NOT.

**Conventions.** Field names are `snake_case`. Types borrow Rust spelling (`Option<T>`,
`Vec<T>`, `Map<K,V>`). `?` after a field = optional. Enums list their variants inline.
The rule the whole model obeys: **one concept, one field — if a field means two things,
it is too broad; split it.**

---

## 1. The one idea

```
   Postman ─┐                                       ┌─ Requestly / rq
   Insomnia ─┤                                       ├─ Postman
   Bruno    ─┤        ┌───────────────────────┐      ├─ Insomnia
   HAR      ─┼──►parse│  THE IDEALISED MODEL   │emit──┼─ Bruno
   OpenAPI  ─┤        │   (superset, versioned)│      ├─ OpenAPI
   cURL     ─┘        └───────────────────────┘      └─ cURL
```

- **Superset, not intersection.** The intersection of all these tools is roughly
  "an HTTP request with a URL." That is useless — it would erase auth, scripts,
  environments, GraphQL, gRPC, render templates. The Idealised Model is the *union*:
  it carries every feature any member of the category has, and marks which are native
  to which.
- **Lossy is fine; silent is not.** Converting `rq` → cURL genuinely cannot carry a
  post-response test. That loss is *reported* (§10), never hidden. A conversion that
  claims success while dropping data is the one unforgivable bug.
- **The schema is software.** The model is versioned and validated. A mapper that
  produces an IR failing schema validation is a bug in the mapper, caught in CI — the
  same discipline the sibling projects apply to `rq`'s format and cyberium's `policy.md`.

---

## 2. The noun model

```
Workspace
└── Environment[]            (global · collection · environment · runtime scopes)
└── Collection               (recursive: folders within folders)
    ├── auth?                (inherited by children unless overridden)
    ├── scripts?             (pre_request / post_response, inherited)
    ├── variables[]          (collection-scoped)
    └── item[]  ─── Request  │ Collection (nesting)
        └── Request
            ├── protocol     (http · graphql · grpc · mqtt · websocket · socketio · soap)
            ├── auth?
            ├── body?
            ├── scripts?
            ├── examples[]   (saved responses)
            ├── depends_on[] (declared chaining — rq's `parents:`)
            └── presentation? (render template + input form — rq's `-- view/form --`)
```

Every node carries a common **`RecordMeta`** (§3). Ordering among siblings is a
fractional-index `rank` string (opaque; never an integer). The tree is explicit
(`item[]` nesting), and *also* recoverable from `parent_id` — both are kept so a flat
format (Postman's flat `records[]` + `parentId`) and a nested format (a directory tree)
round-trip without guessing.

---

## 3. `RecordMeta` — on every node

```
id            Uuid
name          String
description?   String                 // markdown
parent_id     Option<Uuid>            // tree edge; None = root of its collection
rank          Option<String>          // fractional index — sibling ordering
tags          Vec<String>
disabled      bool                    // soft-disable without deleting
source        Provenance              // where this came from (§10)
ext           ExtBag                  // format-specific survivors (§10)
```

`created_*` / `updated_*` / `owner_id` are **not** in the model — they are
server/account artifacts, stripped on import and synthesized on export if a target needs
them. The IR is about *content*, not custody.

---

## 4. Workspace & Collection

```
Workspace {
  meta:          RecordMeta
  collections:   Vec<Collection>
  environments:  Vec<Environment>
  packages:      Vec<CodePackage>      // reusable JS modules (rq/Postman "packages")
  cross_q:       ModelHeader           // version, source_format, generated_by
}

Collection {
  meta:      RecordMeta
  auth:      Option<Auth>              // default for descendants
  scripts:   Option<Scripts>          // pre/post inherited by descendants
  variables: Vec<Variable>            // collection scope
  items:     Vec<Item>                // ordered: Request | Collection
}

enum Item { Request(Request), Collection(Collection) }
```

Auth and scripts on a collection are **inherited** by descendant requests unless the
request overrides them — this models Postman/Requestly `inherit` semantics as a first-
class edge, not a magic string.

---

## 5. Request

```
Request {
  meta:          RecordMeta
  protocol:      Protocol              // discriminant
  auth:          Option<Auth>          // None = inherit from collection
  scripts:       Option<Scripts>
  examples:      Vec<Example>          // saved responses
  depends_on:    Vec<Dependency>       // declared chaining (§8)
  presentation:  Option<Presentation>  // render template + form (§9)
}

enum Protocol {
  Http(HttpRequest),
  GraphQl(GraphQlRequest),
  Grpc(GrpcRequest),
  Mqtt(MqttRequest),
  WebSocket(WebSocketRequest),
  SocketIo(SocketIoRequest),
  Soap(SoapRequest),
}
```

### 5.1 HttpRequest

```
HttpRequest {
  method:          Method              // GET POST PUT PATCH DELETE HEAD OPTIONS TRACE + Other(String)
  url:             Url                 // structured, see below
  headers:         Vec<KeyValue>
  query:           Vec<KeyValue>       // kept separate from url.raw so both round-trip
  path_variables:  Vec<PathVar>
  body:            Option<Body>
  settings:        RequestSettings     // redirects, TLS verify, timeouts, encoding, proxy
}

Url {
  raw:      String                     // the verbatim string, templates intact
  scheme?   host?  port?  path?        // parsed components, best-effort
}

KeyValue {
  key:       String
  value:     String
  enabled:   bool
  kind:      KvKind                    // Text | File | Secret
  content_type?: String
  description?:  String
}

PathVar { key: String, value: String, data_type: ScalarType, description?: String }
```

`query` is stored **separately** from `url.raw` even though it is redundant, because
Postman keeps a structured `query[]` and HAR keeps it in the URL string — carrying both
lets either round-trip without re-parsing (and re-parsing URL query strings is a classic
data-loss bug).

### 5.2 Body

```
Body {
  kind: BodyKind
  ...one payload per kind
}

enum BodyKind {
  None,
  Raw { text: String, language: RawLanguage },   // text/json/html/xml/js/graphql-vars/...
  Json { text: String },                          // kept as text (templates precede parsing)
  FormData { fields: Vec<FormField> },            // multipart
  UrlEncoded { fields: Vec<KeyValue> },
  Binary { file: FileRef },
  GraphQl { query: String, variables: String, operation_name: Option<String> },
}

FormField = KeyValue | FileRef        // multipart mixes text + file parts

enum FileRef {                         // union — only Reference persists
  Reference { id, name, path, size, source },
  Content   { id, name, bytes: Vec<u8>, size, source },   // transient
}
```

`Json`/`GraphQl` variables are stored as **text**, not parsed structures, because
`{{templates}}` are substituted before the text is parsed — parsing first would corrupt
`{ "id": {{userId}} }`. (This is the RQ-3800 substitute-then-parse rule, generalized.)

### 5.3 Other protocols (superset territory)

`GrpcRequest` (method_path, method_type, schema_source: reflection|proto_file|
proto_content, proto_bundle, metadata[], message, timeouts), `MqttRequest` (version,
client_id, clean_session, keepalive, subscriptions[], publish, last_will?, properties?),
`WebSocketRequest`, `SocketIoRequest` (events[], listeners[], auth_payload?),
`SoapRequest` (wsdl_ref, operation, envelope). Most source formats can't produce these;
they exist so Requestly and future tools round-trip, and so a `postman → rq` conversion
that *does* carry them isn't blocked by the lowest common denominator.

---

## 6. Auth

```
Auth {
  kind: AuthKind
  ...one config per kind
}

enum AuthKind {
  None,
  Inherit,                              // explicit "use my parent's auth"
  Basic      { username, password },
  Bearer     { token, header_prefix: Option<String> },   // None=no prefix, Some("Bearer")=default
  ApiKey     { key, value, placement: In },               // In = Header | Query
  OAuth2     { grant: OAuth2Grant, params: OAuth2Params },
  OAuth1     { ... },
  JwtBearer  { algorithm: String, ... },  // String, not enum — allows {{var}}
  Digest     { ... },                     // RFC 7616
  Hawk       { ... },
  AwsSigV4   { mode: Live | Presigned, ... },
  Ntlm       { ... },
  EdgeGrid   { ... },                     // Akamai — Requestly lacks it; carried, warned on rq-emit
  Unknown    { raw_type: String, raw: Json },   // preserve auth we don't model
}
```

Two deliberate choices, both about **not losing credentials**:
- `Bearer.header_prefix` is tri-state (`None` = emit no prefix, `Some("Bearer")` =
  default, `Some(x)` = custom). Collapsing it silently changes requests.
- `Unknown { raw_type, raw }` preserves any auth type the model doesn't recognize so a
  round-trip through `cross-q` never strips a credential it merely didn't understand.

When a target can't represent an auth kind, the emitter downgrades to `Inherit` (or
`None`) **and emits a diagnostic** — never a silent drop. (Mirrors Requestly's
`edgegrid → inherit + warn`.)

---

## 7. Variables & environments

```
Environment {
  meta:       RecordMeta
  is_global:  bool
  variables:  Vec<Variable>
}

Variable {
  key:        String
  value:      String                   // the resolved/persisted value
  initial?:   String                   // Postman's "initial" vs "current" split
  scope:      Scope                     // Global | Collection | Environment | Runtime | Top
  data_type:  VarType                  // String | Number | Boolean | Secret
  category:   VarCategory              // Scoped | Dynamic | Vault
  enabled:    bool
  rank:       Option<String>
}
```

**Resolution precedence** (highest wins), carried so runners in any target agree:

```
top  >  runtime  >  environment  >  collection (child > parent)  >  global
```

- `category = Dynamic` → generated values like `{{$randomEmail}}`, `{{$randomInt 1 100}}`.
- `category = Vault` → `{{vault:key}}`, a reference resolved out-of-band; the **value is
  never stored** in the IR (a vault ref carries the key, not the secret).
- `data_type = Secret` → value is present but flagged; exporters that can't mark secrets
  emit a diagnostic (the value would otherwise land in plaintext in a shared file).

**Templating** is normalized to `{{var}}` (Handlebars/Mustache family) in the IR.
Formats using other delimiters (`:var`, `${var}`, `<var>`) are translated on import and
back on export; unresolved `{{unknown}}` is preserved verbatim, never blanked.

---

## 8. Scripts & chaining

```
Scripts {
  pre_request:   Option<Script>
  post_response: Option<Script>
}

Script {
  source:   String                     // the code, verbatim
  language: ScriptLang                 // JavaScript (default) | Other(String)
  dialect:  ScriptDialect              // Rq | Pm | Bru | Hurl | Raw
}
```

`dialect` records **which namespace the source is written against** — `pm.*` (Postman),
`rq.*` (Requestly), `bru.*` (Bruno), etc. `cross-q` does **not** blindly rewrite
`pm.` → `rq.`; that textual rename is exactly the reliability trap called out in
`FORMAT.md` §11 (it produces code that imports clean and throws at runtime). Instead
the dialect is preserved, and the *actual* translation is delegated to the
`cross-q-context` runtime and its documented `pm.*` compatibility layer (see
[`CONTEXT.md`](./CONTEXT.md)). A conversion may carry a script as-is with its dialect
tagged, and flag it for context-level translation — an honest "not automatically ported"
beats a silent mistranslation.

### Chaining — `depends_on`

```
Dependency {
  target:   RequestRef                 // another request in the workspace
  binds:    Vec<VarBinding>            // outputs of `target` → variables this request reads
}
```

This is the superset of two different models: Requestly's runner `run_order` (linear
ordering) and `rq`'s declared `parents: [...]` (a dependency DAG). The IR stores the
**DAG** (`depends_on`), because a DAG can always be linearized into a run order, but a
run order cannot recover a DAG. Emitting to a linear-only target flattens with a
diagnostic if the graph isn't already a chain.

---

## 9. Presentation (the `rq` north-star surface)

```
Presentation {
  view?:  RenderTemplate               // response → rendered markdown (rq's `-- view --`)
  form?:  InputForm                    // JSON-Forms schema + uischema (rq's `-- form --`)
}
```

No shipping format except `rq`'s north-star pitch has these; they live in the IR so that
(a) a future `rq` gains them losslessly and (b) importing a rendered/interactive `rq`
request into Postman degrades *visibly* (the template becomes a doc comment + a
diagnostic) rather than vanishing. Everything else in the model is real today;
`presentation` is the model admitting where the category is heading.

---

## 10. Nothing is lost silently — provenance, ext bag, diagnostics

Three mechanisms, together the model's core promise:

**1. `Provenance` — where every node came from.**
```
Provenance { format: SourceFormat, locator: String }   // e.g. postman "item[3].request.auth"
```
Attached to every `RecordMeta`. When a diagnostic fires, it points back to the exact
source location — the parse-phase byte span, the map-phase IR path, the emit-phase record
id (RQ-4302, "error context across all three import phases").

**2. `ExtBag` — format-specific survivors.**
```
ExtBag = Map<SourceFormat, Json>       // verbatim fields the model doesn't have a home for
```
A field that has no first-class IR home (a Postman `_postman_id`, an Insomnia
`metaSortKey`, a Bruno `seq`) is stashed here keyed by its source format. On export back
to the *same* format, the ext bag is re-merged → **byte-level round-trip**. On export to
a *different* format, it's ignored (but preserved for any later round-trip). This is how
`postman → rq → postman` returns what you put in.

**3. `Diagnostic` — the report.**
```
Diagnostic {
  severity:  Ok | Coerced | Dropped | Error
  phase:     Parse | Map | Emit
  provenance: Provenance
  message:   String
  detail?:   Json
}
```
Every conversion emits a `Report { diagnostics: Vec<Diagnostic>, fidelity: Fidelity }`.
`Coerced` = we changed a value to make it fit (`key: null → ""`). `Dropped` = the target
couldn't hold it. This is `cross-q`'s equivalent of cyberium's append-only audit log:
**if the tool made a decision, the decision is on the record.**

---

## 11. Fidelity & the round-trip contract

Each `source → target` pair carries a declared fidelity, computed from which model
features survive:

| Level | Guarantee |
|---|---|
| **round-trip** | `A → IR → A` is byte-equivalent (ext bag re-merged). |
| **lossless** | Every A feature is representable in the target; target may add defaults. |
| **lossy (reported)** | Some features have no target home; each is a `Dropped` diagnostic. |
| **degraded** | Structural downshift (gRPC → cURL); many `Dropped` diagnostics. |

The contract: **`cross-q` never overstates fidelity.** A run's summary line names its
fidelity up front, and `--strict` turns any `Coerced`/`Dropped` into a non-zero exit for
CI. Round-trip fidelity for same-format `A → IR → A` is a tested invariant, not an
aspiration — every importer/exporter pair has a corpus of real files it must round-trip.

---

## 12. Versioning

`ModelHeader { model_version: SemVer, source_format, generated_by }`. The IR is
versioned independently of any tool's format. Additive fields bump minor; a field that
changes meaning bumps major and ships a migration in `cq-model`. Emitters target a
tool-format version explicitly (`rq` `1.12.0` `LOCAL_FS`, Postman `2.1.0`) — the IR
version and the tool-format version are orthogonal, the same separation `FORMAT.md`
draws between `SCHEMA_VERSION` and `REQUESTLY_EXPORT_VERSION`.

---

## 13. What the Idealised Model is NOT

- **Not any one tool's format.** It is not Postman-with-extras or Requestly-in-Rust. It
  is a neutral union. For Requestly's actual bytes, read [`FORMAT.md`](./FORMAT.md).
- **Not a runtime.** The IR describes requests; it does not run them or execute scripts.
  Script execution is `cross-q-context` (see [`CONTEXT.md`](./CONTEXT.md)).
- **Not lossless by fiat.** It is lossless *where the target allows* and honest
  everywhere else. The promise is the report, not magic.
- **Not frozen.** It grows to admit new category features (that's what `presentation` is).
  Growth is additive and versioned.

---

## The one-sentence version

The Idealised Model is `cross-q`'s versioned, superset intermediate representation of the
entire API-client category — every collection, request, protocol, auth type, variable
scope, script, chain, and render surface any tool can express — carried with full
provenance and an extension bag so that conversions are lossless where possible, honestly
reported where not, and byte-stable on round-trip.
