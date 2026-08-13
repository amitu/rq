# rq

> A better curl, powered by collections — and the tooling around it.

This is the monorepo for the **`rq`** family: a set of small, honest, single-binary tools
for people who live in API collections but don't want to live in a GUI or a cloud.

Everything here is Rust, MIT-licensed, plain files, no telemetry, no account.

## What's in here

| Crate | Binary / package | What it is | Status |
|---|---|---|---|
| [`crates/cross-q`](crates) | `cq` | Convert API-client collections between formats (Postman ↔ Requestly ↔ Insomnia ↔ Bruno ↔ HAR ↔ OpenAPI ↔ cURL) through one idealised model, and report everything it couldn't carry cleanly. | 🏗️ building |
| `crates/cq-model` | — | The **Idealised Model** — the canonical intermediate representation every importer and exporter maps through. | ✅ v0.1 |
| `crates/cq-transform` + [`packages/cross-q-context`](packages/cross-q-context) | `@requestly/cross-q-context` (npm) | The scripting core: rewrite every dialect (`pm.*`, `postman.*`, `bru.*`) to the `rq.*` API (Rust/OXC → WASM), and execute it on a QuickJS runtime. | 🏗️ building |
| [`crates/rq`](crates/rq) | `rq` | The CLI: named requests, declared chaining, responses rendered as legible markdown. | 🏗️ building |

## Supported formats

The API-client formats `cq` converts between, by app and version. **Import** = read that
format into the Idealised Model; **Export** = write it out of the model. Support is
holistic (all of a version's features in one go), not feature-by-feature.

**Legend:** ✅ supported · 🏗️ in progress · 🔜 planned · — n/a

| App | Version | Import | Export | Notes |
|---|---|:--:|:--:|---|
| **Postman** | Collection v2.1.0 | ✅ | 🏗️ | 47/47 of Postman's own corpus parse; export is the round-trip emitter, not yet a CLI target |
| **Postman** | Collection v2.0.0 | ✅ | 🔜 | object-shaped auth |
| **Postman** | Collection v1.0.0 | ✅ | 🔜 | legacy flat `requests[]`/`folders[]` |
| **cURL** | command line | ✅ | 🔜 | single command ↔ request |
| **Requestly** | `LOCAL_FS` 1.12.0 | 🔜 | ✅ | the git-native on-disk tree |
| **Requestly** | `MappedItems` (bulk-create) | — | ✅ | the app's in-memory import contract |
| **Requestly** | export envelope 1.1.0 | 🔜 | 🔜 | single-file export |
| **Bruno** | `.bru` v2 | ✅ | ✅ | text DSL — requests, folder tree, environments, inheritance; round-trip proven by IR-idempotence |
| **Insomnia** | v4 / v5 | 🔜 | 🔜 | |
| **OpenAPI** | 3.0 / 3.1, Swagger 2.0 | 🔜 | 🔜 | |
| **HAR** | 1.2 | 🔜 | — | capture → requests |
| **Hoppscotch** | collection JSON | 🔜 | 🔜 | |
| **SOAP** | WSDL 1.1/1.2, SoapUI | 🔜 | — | |
| **dotenv** | `.env` | 🔜 | — | → environment variables |
| **`.http`** | VS Code REST Client / JetBrains | 🔜 | 🔜 | |
| **Hurl** | `.hurl` | 🔜 | 🔜 | |

Every format maps through the one [Idealised Model](docs/IDEALISED.md), so a new format is a
single importer/exporter, not N×N glue. New Postman schema versions (v2.2+) are watched for
automatically — see the CI tracking issues.

## Scripts & JS dialects

Most of these apps let you attach pre-request / post-response **JavaScript**. It's all the
same language, but each app binds a **different SDK**, so a script is only portable if you
know which dialect it's written in:

| App | Script SDK |
|---|---|
| Postman | `pm.*` (+ legacy `postman.*`) |
| Bruno | `bru.*` + `req`/`res`, with a `pm.*` compatibility shim |
| Requestly | `rq.*` |
| Insomnia | `insomnia.*` (+ `pm.*` compat) |
| Hoppscotch | `pw.*` / `hopp.*` |

cross-q's rule: **preserve the script verbatim and record its dialect — never blind-rewrite
`pm.`→`rq.`**. Silently string-replacing someone's code is how converters corrupt it. So a
Postman → Bruno conversion carries the `pm.*` script through tagged as `pm`; on export, if the
script's dialect isn't the target's native one, that's a **reported diagnostic**, not a silent
break (and Bruno, for one, runs `pm.*` via its own compat layer).

Two things keep this from becoming an N×M translation matrix:

- **Lift what isn't really code.** Assertions and variable get/set are often declarative, not
  imperative — `pm.expect(...).to.eql(200)` ≈ Bruno's `assert { res.status: eq 200 }` ≈ Hurl's
  `[Asserts]`. cross-q lifts those into **first-class IR** (`asserts`, `vars`), so they convert
  with **zero** JS translation.
- **Translate through one runtime, not per-pair.** Actually *executing* or transpiling across
  dialects is the job of [`cross-q-context`](docs/CONTEXT.md) — a QuickJS `rq.*` runtime with
  `pm.*`-compatible shims — not the converter. The converter carries source + dialect; the
  runtime reconciles them.

## The `rq` CLI

Curl in, named verb out, editor for everything else:

```bash
rq curl --save-as issues 'curl -H "Accept: application/vnd.github+json" \
    "https://api.github.com/repos/anthropics/claude-code/issues?state=open"'

rq r issues        # run it — anytime, from anywhere in the project
rq e issues        # open it in $EDITOR
rq l               # the tree: every request, its method, what it depends on
```

Each request is **one Markdown file** — frontmatter plus `-- description --`,
`-- view --`, `-- body --`, `-- pre --`, `-- post --` sections. The `-- view --` template
renders the response as markdown in your terminal, which is the thing no other client in
this category does. Dependencies are declared per request (`parents: [login]`) and the
values that flow between them are declared too (`capture: { token: response.access_token }`),
so the common chain needs no JavaScript at all.

Full spec: [`docs/RQ-FORMAT.md`](docs/RQ-FORMAT.md). Scripts (`-- pre --` / `-- post --`)
are parsed and round-tripped but **not yet executed** — every run that has one says so, and
`--strict` fails on it. That runtime is `cross-q-context`, landing next.

## Docs

- [`docs/RQ-FORMAT.md`](docs/RQ-FORMAT.md) — the `rq` file format: the request document, the project, variables, chaining.
- [`docs/cross-q.md`](docs/cross-q.md) — the cross-q converter: product + architecture.
- [`docs/IDEALISED.md`](docs/IDEALISED.md) — the Idealised Model (the IR) in full.
- [`docs/FORMAT.md`](docs/FORMAT.md) — the Requestly `LOCAL_FS` on-disk format `cross-q` writes.
- [`docs/CONTEXT.md`](docs/CONTEXT.md) — the `rq.*` scripting runtime spec.

## Build

```bash
cargo test        # run everything
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Rust 1.85+ (2021 edition). One workspace, one version train.

## Testing

Reliability is the product, so the tests are the spec. Three layers:

1. **Unit tests** next to each parser — the format's shapes and edge cases, in-file.
2. **Real third-party corpora**, not just our own fixtures — because the only way to know we
   parse real Postman/Bruno is to run against collections real tools produced. We don't
   vendor them (provider collections carry secret-shaped values that trip secret scanners);
   instead each corpus is a **pinned commit + a fetch script**, downloaded into a gitignored
   dir. Reproducible, no secrets in the repo. See
   [`crates/cross-q/tests/corpus/`](crates/cross-q/tests/corpus/):
   - **Postman** — [Adyen](https://github.com/Adyen/adyen-postman) (canonical v2.1) +
     [newman](https://github.com/postmanlabs/newman) (v2.0/v1), plus Postman's
     transformer examples for crash-safety.
   - **Bruno** — [usebruno's `bruno-tests`](https://github.com/usebruno/bruno) collection
     (a real `.bru` directory tree: folders, environments, every auth/body type).
3. **Fidelity gates, not "it parsed"** — each corpus test asserts **no hollow parse** (every
   request in the source survives into the model — equal counts) and, for the same-format
   round-trip, that field loss is bounded to a documented allowlist. A corpus test **fails
   loud if its data isn't fetched** — a silent skip would be a false green. CI runs the fetch
   scripts before the tests.

The engine also ships as WASM (`@requestly/cross-q`); `packages/cross-q` has a Node smoke
test that runs every importer through the compiled boundary.

## Why

The fastest-growing API tools are the ones that keep your work in plain files you own. `rq`
leans all the way into that: git-native by default, convertible from whatever you have
today, and honest about what it can and can't do with your data. Reliability is the
feature — see [`docs/cross-q.md`](docs/cross-q.md).
