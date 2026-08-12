# `cross-q-context` — the open-source `rq.*` scripting runtime

> Every tool in the Postman category runs user scripts: a snippet before the request, a
> snippet after the response, with an SDK for reading the response, setting variables,
> and asserting. Everyone builds their own sandbox for it. Nobody wants to.
>
> **`cross-q-context` is that sandbox, extracted, hardened, and given away.** One
> Rust crate — a QuickJS-based JavaScript executor that runs the **`rq.*`** scripting API
> (backward-compatible with Postman's **`pm.*`**), published as a native crate, an `npm`
> package (WASM), and a `PyPI` wheel. It lives in the `cross-q` repo because it is
> cross-tool by nature; it is named `rq.*` on purpose (see §11).
>
> This document is the **specification** the open package must honor. It is grounded in
> the runtime Requestly ships internally today (`@requestly/sandbox-definitions`,
> `@requestly/sandbox-node`, `@requestly/script-analysis` — all currently private);
> `cross-q-context` is the clean-room, single-language, publishable unification of those.

> ⚠️ **2026-08-12 architecture correction (supersedes the "one Rust crate" framing above).**
> The "one Rust crate executor, published as crate + WASM + PyPI" plan was set *before* we
> discovered that Requestly's shipped safe engine — `QuickJsSandbox` (`@requestly/sandbox-node`,
> `quickjs-emscripten`) — is already a mature QuickJS-**WASM** runtime that runs in **both** Node
> **and** the browser. So the pillars split by language:
> - **Transform pillar** (`pm.*`/`bru.* → rq.*`, §3) — **Rust** (`cq-transform`) → WASM. Real, shipping (`@requestly/cross-q-context`), consumed by the app.
> - **Execute pillar** (the QuickJS sandbox, §4) — **the app's `QuickJsSandbox` (JS), absorbed** into cross-q-context and made browser-portable — **not** a separate Rust `rquickjs` runtime.
>
> The scaffolded Rust `rquickjs` runtime was **retired** on this date: a second QuickJS engine
> would reintroduce the very drift "one core" existed to prevent, and no target host needs it
> (quickjs-emscripten already covers Node + browser). Consequently the **PyPI runtime wheel** and
> the **cross-engine conformance oracle (§9)** are dropped. §1/§4/§7/§9 below still describe the
> retired Rust-executor plan and will be rewritten during the runtime migration; read them through
> this correction until then.

---

## How to read this document

- §1 — what it is and why one Rust crate.
- §2 — the **`rq.*` API surface** (the contract users write against). The core.
- §3 — the **`pm.*` compatibility** layer (how Postman scripts run unchanged).
- §4 — the **executor/sandbox** (QuickJS, isolation, marshalling).
- §5 — the **execution model** (phases, async, timeouts, `sendRequest`, chaining).
- §6 — the **host wire contract** (`ScriptExecutionInput` / `…Result`).
- §7 — **packaging** (crate / npm / PyPI), §8 — **security**, §9 — **conformance**.
- §10 — the current-vs-target delta, §11 — **why `rq.*`**, §12 — what it is NOT.

**Conventions.** Names are verbatim from the reference implementation; where this doc
prescribes something the reference doesn't do yet, it says **TARGET**. The rule: the
`rq.*` surface is the contract — a host may swap engines beneath it, but scripts must not
be able to tell.

---

## 1. What it is, and why one Rust crate

Requestly's runtime is three private packages: an API-definition package
(`sandbox-definitions`, the `rq.*` factory), an executor package (`sandbox-node`, which
runs QuickJS-WASM in safe mode and `node:vm` in developer mode), and a `pm.*→rq.*`
transform (`script-analysis`, **already a Rust crate** — `script-transform`, built on the
OXC parser, compiled to WASM). `cross-q-context` unifies these into **one Rust crate**
with three deliverables:

```
                        crates.io: cross-q-context            (native, embed in rq CLI / any Rust host)
   ┌──────────────────┐   npm:    @cross-q/context  (WASM)    (embed in Bruno, Yaak, web clients)
   │  ONE RUST CRATE  │→  PyPI:    cross-q-context   (wheel)   (pytest / CI / data pipelines)
   └──────────────────┘
        │
        ├─ engine     QuickJS via `rquickjs` (native) / quickjs-wasm (browser)
        ├─ namespace  the rq.* SDK (§2), built once, engine-agnostic
        ├─ compat     pm.* / postman.* → rq.* AST rewrite via OXC (§3)
        └─ wire       ScriptExecutionInput → ScriptExecutionResult (§6)
```

Why Rust, why one crate: the transform is already Rust; QuickJS has first-class Rust
bindings (`rquickjs`); one core compiles to native, WASM, and a Python extension, so the
`rq.*` semantics are defined **once** and cannot drift between a browser client, a CLI,
and a CI runner. Same reason the sibling projects ship single binaries — shared core,
shared schemas, one distribution, nothing to keep in lockstep.

---

## 2. The `rq.*` API surface

The single source of truth is a namespace factory (`createRqNamespace(...)` in the
reference). It takes all VM dependencies as parameters — no globals, no `declare const` —
so the same logical object exists in every engine. Top-level members:

```
rq.test · rq.expect · rq.info · rq.environment · rq.globals
rq.collectionVariables · rq.variables · rq.iterationData · rq.request
rq.response · rq.vault · rq.cookies · rq.sendRequest · rq.execution · rq.isSafeMode
```

### 2.1 Tests & assertions

```js
rq.test(name: string, fn: () => void): void        // fn throws → { status:'failed', error }
rq.test.skip(name: string, fn?): void              // → { status:'skipped' }
rq.expect                                           // Chai's ExpectStatic (injected lib)
```
Each `rq.test` pushes a `TestResult` (`{ name, status: 'passed'|'failed'|'skipped', error? }`).

### 2.2 `rq.info` (frozen)

```
{ requestId, requestName, iteration, iterationCount, entryIndex, totalEntries, eventName }
```
`eventName` is the phase (`"pre-request"` | `"post-response"`). `Object.freeze`d;
`collectionId` is deliberately excluded (host-internal).

### 2.3 Variable scopes — `environment`, `globals`, `collectionVariables`, `variables`

All four are the same shape (one factory, different backing scope):

```js
get(key): any            // coerces back to the recorded type (number/boolean/string)
set(key, value: string|number|boolean): void
unset(key): void
clear(): void
has(key): boolean
toObject(): Record<string,string>    // always-string view (asymmetric with get, by design)
```
Rules that must be preserved: `set('')` (empty key) throws
`"<scope> variable key must be a non-empty string"`; `collectionVariables` mutators are
**silent no-ops** when there is no collection context (standalone request). `iterationData`
and `vault` are the **read-only** variants — `get` / `has` / `toObject` only, no `set`.
`vault` is backed by out-of-band secrets and never serialized into the script.

### 2.4 `rq.request` (protocol-dispatched)

HTTP/GraphQL request facade:
```
url: string · method: string · headers: RequestHeaders · queryParams · body?
addHeader({key,value}) · removeHeader(name) · upsertHeader({key,value}) · toJSON()
```
`headers` is a **mutable facade** whose mutations are recorded and applied *before* the
request fires: `add · upsert · remove(name) · clear()` (clears ALL — Postman
`HeaderList.clear()` parity) · `has(name)` · `get(name)` · `all()`, header names matched
case-insensitively. gRPC gets `url · methodPath · metadata · message · auth · toJSON()`.

### 2.5 `rq.response` (`null` in pre-request)

HTTP/GraphQL response:
```
status · code (=status) · statusText · headers · body · bodyEncoding?('utf8'|'base64')
time · responseTime (=time) · size · to · json() · text() · toJSON()
```
`headers` is hybrid: index access **and** `get(name)/has(name)/all()`, case-insensitive.
The assertion chain `rq.response.to`:
- `.to.be.{ ok, success, accepted, info, redirection, clientError, badRequest,
  unauthorized, forbidden, notFound, rateLimited, serverError, error }` — status-class
  getters that throw on mismatch.
- `.to.have.status(n|s)` · `.have.header(name)` · `.have.body(expected)` (**full-string
  equality**, not substring — Postman parity) · `.have.jsonBody()` /
  `jsonBody(path)` / `jsonBody(path, value)` · `.have.jsonSchema(schema, opts?)` (Ajv).
- `.to.not.<be|have>…` — negated forms.

gRPC response: `statusCode · statusMessage · metadata · trailers · messages ·
responseTime · json() · text() · toJSON()` with `.to.be.{ok,success,error}` and
`.to.have.{status,metadata,trailer,message,jsonMessage,jsonSchema}`.

### 2.6 `rq.cookies` (host-allowlisted)

```js
rq.cookies.jar().set(url, name, value | cookieInput, cb?): Promise<Cookie>
                .get(url, name, cb?): Promise<string|undefined>
                .getAll(url, cb?): Promise<Cookie[]>
                .unset(url, name, cb?): Promise<void>
                .clear(url, cb?): Promise<void>
```
Every method is dual promise + Node-callback. Host derived from `new URL(url).hostname`;
a denied host rejects with `CookieJarHostDenied`, a bad URL with `CookieJarInvalidUrl`.

### 2.7 `rq.sendRequest(input, cb?)`

`input = string | { url, method?, header?, body? }` (body `raw` | `urlencoded`). Resolves
`{ code, status, headers (get()+index), responseTime, json(), text() }`. Transport
failure → `SendRequestError`; an HTTP 4xx/5xx is **not** an error. Wraps the host-injected
`fetch`; the guest never sees a live `Response`.

### 2.8 `rq.execution` (Postman `pm.execution` parity)

```js
rq.execution.setNextRequest(nameOrNull: string|null): void   // null = stop iteration
rq.execution.location                                        // readonly string[] & { current }
rq.execution.skipRequest(): never                            // PRE-REQUEST ONLY (throws otherwise)
rq.execution.runRequest?(requestId, opts?): Promise<...>     // present only if host wires it; ≤10 calls
```

### 2.9 Injected globals (non-`rq`)

The engine exposes a curated global set — `console`, timers, `fetch`, `URL`,
`URLSearchParams`, `Headers`, `Request`, `Response`, `AbortController`, `TextEncoder`/
`Decoder`, `crypto`, `Blob`, `FormData`, `structuredClone`, `atob`/`btoa`, `performance`,
`EventTarget`/`Event` — plus `require`, `_` (lodash), `xml2Json`, dynamic-variable helpers
(`$guid`, `$randomInt`, …), and **warn-once deprecation shims** for legacy Postman globals
(`responseBody`, `responseCode`, `tv4`, `CryptoJS`, `Backbone`, `globals`, `environment`).

### 2.10 `rq.visualizer` — unsupported stub (TARGET: implement)

A chainable no-op Proxy that warns once. It maps to `rq`'s north-star `-- view --` render
surface (see `FORMAT.md` §10 and `IDEALISED.md` §9); wiring it to a real renderer is
where the CLI's rendered output comes from.

---

## 3. The `pm.*` compatibility layer

**How Postman scripts run unchanged: not at execution time, but as an AST rewrite at
import time.** A script written against `pm.*` is transformed once to `rq.*` and stored
rewritten. This is why runtime `pm` support is a near-non-issue — and why the transform
must be exact.

**Mechanism (verbatim from the reference crate):** parse with **OXC**
(`oxc_parser` / `oxc_ast` / `oxc_span` — a Rust JS/TS parser), walk the AST with a
**scope stack** that tracks user-declared bindings, emit byte-offset
`Replacement { start, end, new_text, message }` records, and splice them in **reverse
start order** so formatting, comments, and whitespace are untouched. It is **not** a
textual `pm.`→`rq.` find-replace and **not** a `globalThis.pm = rq` alias — either of
those corrupts user code that happens to contain the substring or shadows the name.

Public API the crate must expose (verbatim):
```ts
transformScript({ source, platform }): TransformResult
batchTransformScripts({ scripts: Record<id,{preRequest?,postResponse?}>, platform }): BatchTransformResult
extractRequires(source): ExtractedRequire[]
// TransformResult = { success, code, diagnostics:{kind:'Replacement'|'Warning'|'Error',message,span?}[], summary }
```

### The mapping

**Phase 1 — root swap (covers the whole surface).** Because `rq.*` mirrors `pm.*`
member-for-member, rewriting only the **root identifier** `pm` → `rq` (and `postman` →
`rq`) maps the entire SDK 1:1: `pm.environment.*`, `pm.globals.*`,
`pm.collectionVariables.*`, `pm.variables.*`, `pm.iterationData.*`, `pm.vault.*`,
`pm.request.*`, `pm.response.*`, `pm.test`, `pm.expect`, `pm.info.*`, `pm.sendRequest`,
`pm.cookies.jar()`, `pm.execution.*` → `rq.*`.

**Phase 2 — legacy `postman.*` calls** (explicit table):

| legacy | → |
|---|---|
| `postman.setEnvironmentVariable(k,v)` | `rq.environment.set` |
| `postman.getEnvironmentVariable(k)` | `rq.environment.get` |
| `postman.clearEnvironmentVariable(k)` | `rq.environment.unset` |
| `postman.setGlobalVariable(k,v)` | `rq.globals.set` |
| `postman.getGlobalVariable(k)` | `rq.globals.get` |
| `postman.clearGlobalVariable(k)` | `rq.globals.unset` |
| `postman.setNextRequest(n)` | `rq.execution.setNextRequest` |
| `postman.getResponseHeader(h)` | `rq.response.headers.get` |

**Phase 2b — bare legacy globals** (scope-gated: a user binding of the same name is left
alone):

| bare pattern | → |
|---|---|
| `responseBody` | `rq.response.text()` |
| `responseHeaders` | `rq.response.headers` |
| `responseTime` | `rq.response.responseTime` |
| `iteration` | `rq.info.iteration` |
| `request` | `rq.request` |
| `responseCode.{code,name,detail}` | `rq.response.{code,name,detail}` |
| `data.X` | `rq.iterationData.get('X')` |
| `globals.NAME` / `environment.NAME` read | `rq.globals.get('NAME')` / `rq.environment.get('NAME')` |
| `globals.NAME = v` | `rq.globals.set('NAME', v)` |
| `tests["label"] = expr` | `rq.test("label", () => rq.expect(expr).to.be.ok)` |

### Gaps — carried honestly (this is the reliability rule)

- `pm.visualizer.*` and `pm.variables.replaceIn()` → **warning, no rewrite** (unsupported
  / partial).
- Deep chains (`globals.a.b`) and dynamic keys (`globals[k]`) are **deliberately left
  un-rewritten** — a missed rewrite is recoverable (the deprecation shim catches it at
  runtime); corrupt JS is not. **Asymmetric safety: never emit code you can't prove
  parses.**
- **The known trap (must be fixed here, TARGET):** the rename does **not verify the
  resulting `rq.*` call exists**. `pm.request.headers.add()` → `rq.request.headers.add()`
  imports clean but throws at runtime on a read-only request shape. `cross-q-context`
  must run a post-rewrite **validation pass** against the known `rq.*` surface and emit a
  diagnostic for any call that renamed but won't resolve — turning a runtime `TypeError`
  into an import-time warning. This is the single most valuable thing the open package
  fixes over the status quo (see `FORMAT.md` §11).

---

## 4. The executor / sandbox

One `Sandbox` interface, two engines, selected per execution by `mode`
(`safe` | `developer`); **default and fail-closed to `safe`** (any unrecognized mode →
safe).

### Safe mode — QuickJS

Reference runs QuickJS-WASM (`quickjs-emscripten`); **TARGET: native QuickJS via
`rquickjs`** in the crate, WASM for the npm build. Isolation invariants the crate must
keep:

- **Fresh runtime + context per execution**; the compiled module is memoized per process.
- **Memory cap** 128 MB (`setMemoryLimit`).
- **Interrupt** on an op-count ceiling (`1_000_000` ops) **and** a wall-clock deadline
  (`timeoutMs`, default 5 min).
- **Bare ES realm** — *no* ambient Web/Node globals. Everything in §2.9 is installed
  explicitly by evaluating shim strings in a fixed order (core → console → process(inert)
  → Buffer → crypto → util/stream/zlib → fetch → require → chai → the `rq` shim →
  deprecation shim → optional runRequest).
- **Copy-only boundary** — no live host reference ever enters the guest. Sync bridges are
  guest functions; async bridges (`fetch`, `runRequest`) use a guest-promise + a manual
  `executePendingJobs()` pump loop (not asyncified host functions).
- On a timeout kill, **leak the killed runtime** and drop the memoized module rather than
  risk an asyncify teardown race.
- In-guest `rq.isSafeMode === true`; timer stubs **ignore the delay** (microtask-only).

### Developer mode — `node:vm` (host-embedding only)

A `node:vm` context with the host's globals copied in and `require` wired to the host
resolver. Weaker containment (a script can keep running after the timeout — enforced via
`Promise.race`, not interruption). This mode exists for trusted, local, developer-machine
use; **the WASM/PyPI builds ship safe mode only.**

### Marshalling

- **In:** the serializable `ScriptExecutionContext` is passed as a single JSON string and
  `JSON.parse`d inside the guest.
- **Out:** the guest accumulates results on reserved globals (`__rq_testResults`,
  `__rq_mutations`, `__rq_requestMutations`, `__rq_executionDirective`) and the host
  drains them with one `JSON.stringify`. Raw `{value,type}` mutation entries are inflated
  host-side into a full `MutationDiff` that preserves `syncValue` and **never downgrades a
  `secret`** to a plain value.

---

## 5. Execution model

- **Phases:** `pre-request` (`"pre-request"`) and `post-response` (`"post-response"`).
  `rq.response` is absent in pre-request; `rq.execution.skipRequest()` is pre-request only.
- **Chains:** pre-request scripts run forward, post-response scripts run **reversed**
  (collection → folder → request unwinds correctly). Each script is its own
  `sandbox.execute` call; the entry is re-prepared after each pre-request script so later
  scripts see earlier mutations.
- **`async`/`await`:** supported (the script is wrapped in an async IIFE). Safe mode pumps
  pending jobs; developer mode races against a timeout.
- **`sendRequest`:** routed through the host `fetch` bridge; body drained to a string.
- **Chaining out:** `setNextRequest` / `skipRequest` set an `ExecutionDirective` drained
  on the result; a runaway is capped (`MAX_ENTRY_EXECUTIONS_PER_RUN = 1000`,
  `runRequest` ≤ 10 calls).

---

## 6. Host wire contract

The crate's job is a pure function of a serializable input to a serializable output — this
is what every host (Rust, JS, Python) calls:

```ts
ScriptExecutionInput = {
  script, phase, context, entryId, entryType, mode, timeoutMs?,
  userPackages?, blacklistedPackages
}

ScriptExecutionResult = {
  mutationDiff: MutationDiff,                 // { global?, environment?, collection?, runtime?, vault? }
  logs: LogEntry[],                           // { level, args, timestamp }
  testResults: TestResult[],                  // { name, status, error? }
  cookieMutations?: CookieJarMutation[],
  requestMutationDiff?: RequestMutationDiff,  // header add|upsert|remove|clear
  executionDirective?: ExecutionDirective,    // {kind:'set-next-request',target} | {kind:'skip-request'}
  error?: string,
  errorDetails?: unknown
}
```

Logs stream live as events during execution; the streamed union is
`{type:'log'} | {type:'deprecation'} | {type:'result'}`. Because the whole contract is
JSON-serializable, the WASM and PyPI builds expose the *same* call — `execute(input) →
result` — with no live objects crossing the language boundary.

---

## 7. Packaging

| Target | Name | Build | Consumers |
|---|---|---|---|
| Rust crate | `cross-q-context` (crates.io) | native, `rquickjs` | the `rq` CLI, any Rust host |
| npm | `@cross-q/context` | `wasm-pack --target bundler` / `nodejs` | Bruno, Yaak, web clients, Node CI |
| PyPI | `cross-q-context` | `maturin` / `pyo3` wheel | pytest, data pipelines, CI |

One crate, three artifacts, one set of semantics. The transform already builds to two WASM
targets (`bundler` + `nodejs`) in the reference; `cross-q-context` extends that discipline
to the whole runtime. Everything ships **MIT**, no telemetry, no network calls except the
`fetch`/`sendRequest` the host explicitly wires.

---

## 8. Security posture

- **Safe by construction, not configuration.** The published (WASM/PyPI) builds are safe
  mode only; there is no flag that grants filesystem or arbitrary-network access. Developer
  mode (`node:vm`) is available solely when a Rust/Node host embeds it deliberately.
- **No ambient authority.** The guest realm starts empty; every capability
  (`fetch`, cookies, `runRequest`) is a host-injected bridge that can be withheld. Cookies
  are host-allowlisted per-hostname.
- **Bounded.** Memory cap, op-count interrupt, wall-clock deadline, call caps on
  `runRequest` and request re-execution — a hostile or runaway script cannot wedge the host.
- **Secrets never leak.** `vault` values are out-of-band and never serialized into the
  script context; `MutationDiff` never downgrades a `secret` to plaintext.

---

## 9. Conformance

The value of `cross-q-context` is that `rq.*` means the *same thing* everywhere it runs.
That is enforced by a **shared conformance corpus** — a set of `(script, context) →
expected result` fixtures the native, WASM, and PyPI builds must all pass identically, plus
a **Postman-compat corpus** of real `pm.*` scripts that must transform-then-run to the
documented outcome. A partner tool (Bruno/Yaak) adopting the package runs the same corpus;
"passes the corpus" is what lets them advertise Postman-script compatibility honestly.

---

## 10. Reference → target delta (what we change by open-sourcing)

| Today (Requestly internal) | `cross-q-context` (target) |
|---|---|
| 3 private packages, unpublished | 1 public crate → crate / npm / PyPI |
| Executor in TS over quickjs-emscripten | Rust over `rquickjs` (native) + WASM |
| Transform is Rust/OXC (already) | kept, plus a post-rewrite **validation pass** (§3 gap) |
| `rq.*` defined in TS factory | `rq.*` defined once in Rust, engine-agnostic |
| Semantics can drift per client | one conformance corpus, all builds |

The `rq.*` surface, the `pm.*` mapping, the wire contract, and the isolation model are
**preserved exactly** — this is an extraction and a hardening, not a redesign.

---

## 11. Why the namespace stays `rq.*`

It would be tidier to call it `cq.*` or `ctx.*`. We do not, for three reasons:
1. **Migration.** A Postman user's muscle memory is `pm.*`; the smallest possible leap is
   a namespace that maps 1:1. `rq.*` is that.
2. **Compat is the feature.** The whole pitch to a partner tool is "your users' Postman
   scripts just run." That promise is the `pm.*→rq.*` mapping; renaming the target throws
   the mapping's clarity away.
3. **Mindshare is the moat.** Every script written against `rq.*` in a partner tool is a
   seed for the `rq` CLI. The namespace *is* the standard we are trying to own. `cross-q`
   is the repo; `rq.*` is the brand — on purpose.

---

## 12. What `cross-q-context` is NOT

- **Not an HTTP client.** It executes scripts; it does not send the primary request. It
  *calls back* to a host-provided `fetch` for `sendRequest`/`runRequest`. Sending is the
  host's job (`rq`, Bruno, a CI runner).
- **Not a converter.** Converting collections is `cross-q` (see `cross-q.md`); this runs
  the scripts inside them.
- **Not a Postman reimplementation.** It is `pm.*`-*compatible* via a documented mapping
  with named gaps — not a claim of 100% parity. The gaps are in the report, not hidden.
- **Not `node:vm` in production.** The published builds are QuickJS-safe-mode only;
  developer mode is an embedding-only escape hatch for trusted local use.

---

## The one-sentence version

`cross-q-context` is the open-source, Rust-built, QuickJS-based runtime that executes
pre-request and post-response scripts against the `rq.*` API — backward-compatible with
Postman's `pm.*` through an OXC-based AST rewrite — shipped identically as a crate, an npm
WASM package, and a PyPI wheel, so that one definition of `rq.*` runs the same in the `rq`
CLI, in a partner tool, and in CI.
