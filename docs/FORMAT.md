# `rq` / Requestly API Client — On-Disk Format

> The file format `rq` reads and writes. This is a **reference specification**, not a
> pitch. It is dense on purpose: enough detail that a converter (`cross-q`) or a fresh
> `rq` implementation can round-trip Requestly data from these pages alone.
>
> Everything here is grounded in the current implementation
> (`~/bs/requestly-api-client`, the Zod-schema-as-source-of-truth model in
> `packages/schemas`). Where the developer pitch (`rq — a better curl, powered by
> collections`) describes something the shipped code does *not* do yet, it is called out
> explicitly as **NORTH STAR**, not as fact.

---

## How to read this document

- If you are writing an **exporter** into Requestly (this is `cross-q`'s job), read
  §2 (the two representations), then §4 (the record model), then §9 (the on-disk
  `LOCAL_FS` tree — that is the byte layout you emit).
- If you are writing an **importer** *from* Requestly, read §3 (envelope) and §9, then
  §10 (the north-star single-file form the CLI is moving toward).
- If you are debugging **import reliability** (RQ-4300 / RQ-3458), skip to §11.

There are two audiences and one rule: **field names are verbatim from the schemas.**
Where this doc and the code disagree, the code wins — file paths are cited so you can
check.

---

## 1. What `rq` is, in one paragraph

`rq` is the Requestly API Client's git-native command surface — "a better curl, powered
by collections." It runs named requests, walks declared dependency graphs, and renders
responses. It reads two things: **Requestly projects** (the native format specified
here) and **Postman v2.1 collections** (via import). This document specifies the
Requestly native format only; Postman is an *external* format that `cross-q` converts
*into* this one.

---

## 2. Two representations, one model

Requestly serializes the **same logical model** two ways. Do not confuse them — they
have independent version numbers.

| | Single-file **export envelope** | Directory tree (**`LOCAL_FS`**) |
|---|---|---|
| Shape | one `.json` file | a folder of many small files |
| Purpose | share, back up, hand off | git-native project, hand-editable |
| Version field | `meta.version` = **`1.1.0`** | `$schema` URL embeds **`1.12.0`** |
| Const in code | `REQUESTLY_EXPORT_VERSION` | `SCHEMA_VERSION` (`local/version.ts`) |
| Extension(s) | `.json` (also `.zip`) | `.json` + `.ts` + `.md` |
| `cross-q` target | supported | **primary target** |

The two version numbers are **independent**. `meta.version: 1` in an old export maps to
`1.1.0` on import. The `LOCAL_FS` changelog runs `1.0.1 → 1.12.0`
(`local/version.ts`). Never assume one implies the other.

> **NORTH STAR.** The pitch describes a *third* form — one Markdown file per request
> (`__metadata.md`) with YAML frontmatter plus `-- description / view / pre / post /
> form --` sections. The shipped client does **not** write that; it writes the split
> JSON tree in §9. Treat the single-`.md` form as a direction, documented in §10, not a
> format to emit today.

---

## 3. The single-file export envelope (`1.1.0`)

`packages/exporters/src/requestly/`. One JSON object:

```json
{
  "meta": {
    "version": "1.1.0",
    "exportedAt": "<ISO-8601>",
    "exportedUsing": "Requestly API Client"
  },
  "records": [ /* SanitizedRecord — collections + apis + examples, flat, linked by parentId */ ],
  "environments": [ /* SanitizedEnvironment */ ],
  "packages": [ /* SanitizedCustomPackage */ ]
}
```

**Sanitization on export (do the same when you synthesize an envelope):**
- Strip server-owned fields: `createdBy`, `updatedBy`, `ownerId`, `createdTs`,
  `updatedTs`.
- Strip variable `localValue` (client-only working copy).
- OAuth2 `manual` `token` → `""` (never export a live token).
- Literal secrets are counted and surfaced as a warning, not silently shipped.

Records are stored **flat** with parent links (`parentId`), not nested — reconstruct the
tree from `parentId`.

---

## 4. The record model (the heart of it)

`packages/schemas/src/api/common.schema.ts`, `constants/index.ts`.

### 4.1 Record envelope — every entity has this

```
id           uuid
name         string
description?  string
parentId     uuid | null        // tree edge; null = root
ownerId      string
deleted      bool
createdBy    string
updatedBy    string
createdTs    number (ms)
updatedTs    number (ms)
rank         string | null      // fractional index — sibling ordering, NOT an integer
```

`rank` is a **fractional-index string** (e.g. `"a0"`, `"a0V"`), not an ordinal. It lets a
record be reordered between two siblings without renumbering the rest. Preserve it as an
opaque string; do not parse it as a number.

`RecordType` (`type` discriminant): `api` · `collection` · `example`.

### 4.2 Collection

`collectionResponseSchema` = record envelope + `type: 'collection'` + `projectId` +
`version` (int, optimistic concurrency) + `data`:

```
collectionDataSchema {
  variables?  Record<string, variableData>   // collection-scoped vars
  auth?       authConfig                       // discriminated union on `type`
  scripts?    { preRequest?: string, postResponse?: string }
}
```

Collections nest through `parentId`. A recursive `collectionTreeNodeSchema`
(`requests[]` + `children[]`) is the in-memory tree view.

### 4.3 Request ("API entry")

`requestResponseSchema` = record envelope + `type: 'api'` + `projectId` + `version` +
optional `protoContent` (bundled `.proto` for gRPC) + `data: apiEntrySchema`.

`apiEntrySchema` is a **discriminated union on `type`** over six protocols (`EntryType`):

```
http · graphql · grpc · mqtt · websocket · socketio
```

Each arm: `{ type, request, response?, auth?, scripts? }`.

### 4.4 Example (saved response)

`type: 'example'`. Postman "saved responses" import to these. Attached under a request
via `parentId`.

---

## 5. Request payloads, per protocol

### 5.1 HTTP — `httpRequestSchema`

```
url            string
method         GET | POST | PUT | PATCH | DELETE | HEAD | OPTIONS
headers        keyValuePair[]
queryParams    keyValuePair[]
pathVariables  pathVariable[]
body           httpBody
contentType    raw | json | form | formData | binary | none
includeCredentials?  bool
```

`keyValuePair` = `{ id: number, key: string, value: string, isEnabled: bool,
description?, type? }`.
`pathVariable` = `{ key, value, description?, dataType?: string|number|integer|boolean }`.

`httpBody`:

```
contentType     raw | json | form | formData | binary | none
raw?            string
rawContentType? text/plain | application/json | text/html | application/xml | application/javascript
formData?       formDataKeyValuePair[]   // multipart
formUrlEncoded? keyValuePair[]           // application/x-www-form-urlencoded
binary?         string
```

`RequestContentType` enum values are **not** their labels — verbatim:
`raw='raw'`, `json='json'`, `form='form'`, `formData='multipart/form-data'`,
`binary='binary'`, `none='none'`.

**Multipart file values** (`multipartFileValueSchema`) are a union:
- `{ type: 'reference', id, name, path, size, source }` — **persisted**.
- `{ type: 'content', id, name, contents: Uint8Array, size, source }` — transient,
  resolved at send time, **never persisted**.
When emitting, only the `reference` variant is durable.

### 5.2 GraphQL — `graphqlRequestSchema`

```
url, method, headers[], queryParams[],
query         string
variables?    string    // a raw JSON *string*, not an object (substitute-then-parse, RQ-3800)
operationName?
connectionInitPayload?
```

`variables` being a **string** (not a parsed object) is deliberate: templates like
`{{id}}` are substituted before the string is parsed as JSON. Preserve it verbatim.

### 5.3 gRPC — `grpcRequestSchema`

`url` (`grpc://`|`grpcs://`), `methodPath`, `methodType`
(`unary`|`server_streaming`|`client_streaming`|`bidi_streaming`), `schemaSource`
(`reflection`|`proto_file`|`proto_content`), `metadata[]`, `message` (JSON string),
timeouts. `.proto` bundle: `protoContent { entryPoint, files: [{ relativePath, content }] }`.

### 5.4 MQTT / WebSocket / Socket.IO

Full realtime configs (`packages/schemas/src/api/realtime/`). MQTT carries
`version`, `clientId`, `cleanSession`, `keepalive`, `subscriptions[]`, `publish{}`,
`lastWill?`, `properties?`. These have **no Postman vocabulary** — a Postman exporter
drops them (with a warning). `cross-q` must treat them as first-class in the IR even
though most source formats can't produce them.

---

## 6. Auth — `authConfigSchema`

Discriminated union on `type` (`AuthType`):

```
inherit · basic_auth · bearer_token · api_key · oauth_2 · oauth_1
       · jwt_bearer · digest_auth · hawk · aws_sigv4 · ntlm
```

Notable shapes:
- `bearer_token`: `{ token, headerPrefix?: string|null }` — `undefined` → `"Bearer"`,
  explicit `null` → **no prefix**. The tri-state matters; don't collapse it.
- `api_key`: `{ key, value, placement: 'header' | 'query_param' }`.
- `oauth_2`: nested union on `grantType`
  (`authorization_code` · `authorization_code_pkce` · `client_credentials` · `implicit`
  · `password` · `manual`). **No token fields** — tokens are system-managed. Plus
  `customAuthParams` / `customTokenParams` / `customRefreshParams`.
- `jwt_bearer`: `algorithm` is `z.string()` (not an enum) so `{{var}}` is allowed.
- `aws_sigv4`: `live_request` vs `presigned_url`.

**Read-side tolerance.** Persisted auth of a type no longer in the union is preserved as
`{ type: 'unknown', rawType }` rather than dropped. Absent auth on an entry = no auth.
Default on create = `inherit`.

> **`cross-q` rule.** When a source auth type has no Requestly equivalent, fall back to
> `inherit` **and emit a warning** — never silently drop credentials. This mirrors the
> Postman importer's `edgegrid → inherit + warn` behavior.

---

## 7. Variables, scopes, templating

### 7.1 Variable shape

`variableBaseSchema` (server-stored, and what lands on disk):

```
id         uuid
syncValue  <the persisted value>
type       string | number | boolean | secret     // VariableDataType
isEnabled? bool
rank       string | null
createdAt? updatedAt? createdBy? updatedBy?
```

`variableDataSchema` adds client-only `localValue` and `isPersisted` — **stripped before
persistence**. On disk you see `syncValue`, never `localValue`.

### 7.2 Scopes and precedence

`VariableScope`: `global · collection · environment · runtime · top`.
Resolution is **first-write-wins** in this order (highest wins):

```
top  >  runtime  >  environment  >  collection (child > parent)  >  global
```

Collection scope walks child-first, so a sub-collection variable overrides its parent's.
Disabled variables (`isEnabled === false`) are skipped entirely.

### 7.3 Templating

Handlebars `{{var}}`. Three categories (`VariableCategory`):
- `scoped` — ordinary user variables.
- `dynamic` — faker-backed, e.g. `{{$randomEmail}}`, `{{$randomInt 1 100}}`.
- vault secrets — `{{vault:key}}`, resolved inline via a non-serializable `SecretLookup`
  *before* Handlebars runs; vault values never cross a serialization boundary.

Unresolved `{{unknown}}` is left in place (escaped), not blanked.

---

## 8. Scripts, runners, scheduled runs

### 8.1 Scripts

`{ preRequest?: string, postResponse?: string }` at **both** entry and collection level.
`ScriptPhase`: `preRequest='pre-request'`, `postResponse='post-response'`. The script API
is `rq.*` (a rename of Postman's `pm.*` — see §11 for the reliability caveat). On disk,
scripts are raw `.ts` files (§9).

### 8.2 On-demand collection runner — `runner.schema.ts`

```
upsertRunConfig {
  runOrder    [{ id: uuid, isSelected: bool }]   // array order = execution order
  delay       int 0..300000    // ms between iterations
  iterations  int 1..1000
  skipJarRead?         bool
  saveCookiesAfterRun? bool
}
```

Persisted config adds `id, createdTs, updatedTs`. Per-request results
(`requestExecutionResultSchema`) carry `iteration`, `executionOrder`, `protocol`,
`status: success|error|skipped`, `statusCode`, and `testResults[]`
(`{ name, status: passed|failed|skipped, error? }`).

### 8.3 Cloud scheduled runs — `scheduled-run.schema.ts`

`{ collectionId, environmentId?, timerType: hour|day|week|month, interval, retryCount
0..10, retryBackoffStrategy: none|fixed|linear|exponential, state: active|paused,
runOrder[] }`. Portable form: `kind: 'requestly/scheduled-run-config'`, `schemaVersion: 1`.

---

## 9. The `LOCAL_FS` tree (`1.12.0`) — the byte layout to emit

`packages/schemas/src/local/`; engine in `modules/repository/src/local/`. **This is the
format `cross-q` writes.**

### 9.1 Project root

```
<project>/
├── __requestly.json        # project marker: { version, include[], exclude[] }
├── apis/                    # collections + requests (entity folders)
├── environments/           # one file per environment
├── packages/               # custom JS packages
├── specs/        (optional)
├── components/   (optional)
└── .requestly/   (optional)
```

`__requestly.json` is the discovery marker (walk up from cwd to find it, git-style).

### 9.2 Entity folders

A collection or request is a **folder under `apis/`**. Folder name = entity name (no type
suffix). Type is decided by the folder's `__metadata.json`. Nesting mirrors the
collection hierarchy — `parentId` is *derived from directory structure*, not stored.

`__metadata.json` (`local/metadata.schema.ts`) is the only local-only schema:
discriminated on `type` (`collection` | `api`) and, for APIs, on `entryType`. It carries
identity plus the sidebar-visible fields (`id`, `rank`, and for HTTP: `url`, `method`,
`contentType`).

### 9.3 A request is split across resource files

`ENTITY_FILES` in `local/constants.ts`, written by `writeHttpResourceFiles`:

| Canonical field | File | Schema |
|---|---|---|
| method / url / contentType | `__metadata.json` | metadata.schema.ts |
| headers | `__headers.json` | `keyValuePair[]` |
| queryParams | `__query-params.json` | `keyValuePair[]` |
| pathVariables | `__path-variables.json` | `pathVariable[]` |
| body | `__body.json` | `httpBody` |
| auth | `__auth.json` | `authConfig` (file **deleted** when auth absent) |
| scripts.preRequest | `__scripts/__pre-request.ts` | raw TS |
| scripts.postResponse | `__scripts/__post-response.ts` | raw TS |
| description | `__README.md` | raw markdown |
| collection variables | `__variables.json` | `Record<string, variableBase\|null>` (null = delete) |

Protocol-specific files: GraphQL `__query.json`; gRPC `__grpc-metadata.json` +
`__message.json`; WebSocket `__messages.json` + `__settings.json`; Socket.IO
`__events.json` / `__listeners.json` / `__auth-payload.json`; MQTT `__subscriptions.json`
/ `__publish.json` / `__mqtt-properties.json` / `__last-will.json`. Examples live in
`__examples/`; runner config in `__runner/__config.json`.

**One concept per file** — this is the same discipline `rq`'s pitch calls "one concept,
one file," realized as a directory rather than a single Markdown doc.

### 9.4 Environments

Flat files in `environments/`, each `<name>.json` = `{ id, variables }`
(`local/environment.schema.ts`). The global environment is the file `__global.json`
(`isGlobal` is derived from that filename). Extension is plain `.json` — a doc-comment in
the schema says `*.env.json`, but **the code writes `.json`; trust the code.**

### 9.5 Extensions & versioning

`.json` for data · `.ts` for scripts · `.md` for descriptions · `.js` for custom
packages. Version is **not** an inline scalar — every written JSON file gets a `$schema`
URL embedding the version:

```
"$schema": "https://assets.requestly.com/local/v1.12.0/metadata.json"
```

(`local/schema-urls.ts`). A migration subsystem lives in
`modules/repository/src/local/migration/`. Emit the `$schema` URL at the current
`SCHEMA_VERSION` so migrations and validation can key off it.

---

## 10. NORTH STAR — the single-`.md` request (pitch, not shipped)

The developer pitch proposes collapsing each request's split files back into one
hand-readable Markdown document, `__metadata.md`:

```markdown
---
method: GET
url: https://api.github.com/repos/{{owner}}/{{repo}}/issues
headers:
  Accept: application/vnd.github+json
query:
  state: open
vars:
  owner: { default: anthropics, prompt: "Repository owner" }
  GH_TOKEN: { env: GH_TOKEN, secret: true, required: true }
parents: []
---

-- description --   free markdown
-- view --          Jinja-like response→markdown template  (the pitch's killer feature)
-- pre --           JS run before the request (rq.*)
-- post --          JS run after: rq.test(...), rq.vars.set(...)
-- form --          JSON-Forms schema for terminal input
```

Two concepts here exist **only** in the pitch and have **no** field in the shipped schema
today — flag them if you invent IR fields for them:
- **`parents: [...]`** — declared per-request dependency graph. The shipped model has
  runner `runOrder` (§8.2), which is ordering, not a dependency DAG.
- **`-- view --` / `-- form --`** — response render template and JSON-Forms input. No
  equivalent persisted field exists.

Do not emit these into a `1.12.0` project. Carry them in the IR (see
`IDEALISED.md`) as extension fields so they survive a round-trip once `rq` grows
real support.

---

## 11. Import reliability (RQ-4300 / RQ-3458) — what breaks and why

The importers are Zod-only (no ajv/JSON-Schema). Known failure modes, all worth a test
fixture in `cross-q`:

- **Null / numeric key-value keys (RQ-3458).** A 28MB Postman import aborted because one
  `key: null` failed a strict `z.string()`. Fix in `postman/schema.ts`:
  `key: z.union([z.string(), z.number().transform(String), z.null().transform(() => '')]).optional().default('')`.
  Deliberately **not** `.catch('')` — object/array/boolean keys still hard-fail (they
  indicate genuinely malformed input). Values coerce number/boolean, and arrays are
  space-joined.
- **Placeholder cookies.** `name`/`value` made optional; nameless cookies are dropped,
  not fatal.
- **Malformed percent-encoding.** `safeDecodeURIComponent` returns the raw string on
  throw instead of aborting.
- **OpenAPI circular refs / stack overflow (RQ-1978).** `circular: 'ignore'` +
  `RangeError` caught → `invalid_format`.
- **HAR null bytes.** `U+0000` stripped (Postgres `jsonb` rejects it).
- **Script rename is textual, not validated.** `pm.request.headers.add()` →
  `rq.request.headers.add()` imports clean but throws at runtime if `rq.*` has no such
  method. A rename ≠ a port.

The throughline: **parse tolerantly, coerce visibly, abort only on genuinely ambiguous
input** — and every coercion should surface as a warning, never a silent edit. That
principle is `cross-q`'s whole reliability thesis (see `cross-q.md` §Reliability).

---

## 12. What this document is NOT

- **Not the Postman format.** Postman v2.1 is an external source `cross-q` converts *in*;
  its schema lives in `packages/importers/src/postman/schema.ts`, not here.
- **Not the wire/API format.** The server request/response bodies
  (`upsert*BodySchema`) are adjacent to but distinct from the on-disk shapes.
- **Not a promise about `1.13.0`+.** Fields move. When in doubt, read the Zod schema at
  the version embedded in the file's `$schema` URL.

---

## The one-sentence version

`rq`'s native format is the Requestly record model — collections, api-entries,
examples, environments, and typed variables — serialized either as a single sanitized
`1.1.0` export envelope or, preferably, as a `1.12.0` `LOCAL_FS` directory tree that
splits every request into one-concept-per-file JSON, with a north-star single-Markdown
form still on the horizon.
