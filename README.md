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
| [`crates/rq-doc`](crates/rq-doc) | — | The `rq` request document — one Markdown file per request — and the project layout around it. Read by the CLI, written by the converter. | ✅ v0.1 |

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
| **rq** | project (`__metadata.md`) | ✅ | ✅ | the format the `rq` CLI reads — one Markdown file per request; round-trip proven by IR-idempotence |
| **Requestly** | `LOCAL_FS` 1.12.0 | 🔜 | ✅ | the app's split-JSON tree (`--to requestly`) |
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
cargo install --path crates/rq    # once — see Install below

rq curl --save-as issues 'curl -H "Accept: application/vnd.github+json" \
    "https://api.github.com/repos/anthropics/claude-code/issues?state=open"'

rq r issues        # run it — anytime, from anywhere in the project
rq e issues        # open it in $EDITOR
rq l               # the tree: every request, its method, what it depends on
rq r me -c         # the console: arrow between the steps of a run, drill into each
```

In the console, a digit opens that link and `backspace` goes back — the run you are
reading becomes a place you can move around in. It is also the browser network panel for
your terminal — every step of the run, its
request, its response, its headers, and where the milliseconds actually went — over the run
you already did. Nothing is re-sent, and there is no second `--verbose` pass.

Each request is **one Markdown file** — frontmatter plus `-- description --`,
`-- view --`, `-- body --`, `-- pre --`, `-- post --` sections. The `-- view --` template
renders the response as markdown in your terminal, which is the thing no other client in
this category does. Dependencies are declared per request (`parents: [login]`) and the
values that flow between them are declared too (`capture: { token: response.access_token }`),
so the common chain needs no JavaScript at all.

The format is a first-class cross-q citizen in both directions — `cq convert x.postman_collection.json --to rq`
brings a collection in, and `cq convert ./my-apis --to bruno` takes it anywhere else — so
`rq import` is that converter, not a second implementation of it.

Full spec: [`docs/RQ-FORMAT.md`](docs/RQ-FORMAT.md). Scripts (`-- pre --` / `-- post --`)
are parsed and round-tripped but **not yet executed** — every run that has one says so, and
`--strict` fails on it. `rq` *hosts* the engine rather than implementing it: the header
mutations, variable writes, test results, execution directives and cookie-jar seeding a
script produces are already wired through the run and covered by tests against a stub
engine, so `cross-q-context` drops into one trait when it ships.

## Try it against something real

`examples/testbed/` is a project you can actually run, and `rq-testbed` is the API it talks
to — a small dependency-free server (`std::net` and JSON, nothing else) with the endpoints
the examples use: a login that hands out both a token and a session cookie, a `/me` that
accepts either, a list worth rendering, and an `/echo` that mirrors whatever you sent.

```bash
cargo run -p rq-testbed          # http://127.0.0.1:8087 — `--routes` lists them
cd examples/testbed
rq r me -e local                 # login runs first, its token lands on this request
rq r me-by-cookie -e local       # same endpoint, no header — the cookie jar carried it
rq r issues -e local             # the rendered table
rq r slow -e local --show timing # a server that really waits, so the phases are real
```

`crates/rq/tests/testbed.rs` runs that project against that server with the shipped binary,
so the example, the docs and the client are checked against each other rather than against
someone's memory of what they said.

## The demo: an app whose pages are markdown files

`rq-testbed` also serves a small social app — a timeline, posts, replies, likes, people —
and `examples/app/` is a **frontend for it**, written entirely as request documents.

```bash
cargo run -p rq-testbed            # the app's backend, in another terminal
cd examples/app
rq r timeline -e local --console   # the app
```

Inside: `tab` moves between the links on the page, `enter` opens one, `backspace` goes
back. Opening a link to a request that declares a `-- form --` — "write a post" — shows the
form rather than firing the request; fill it, `ctrl-s`, and the timeline you return to has
your post on it.

A page is a `-- view --`. A link is `[label](rq:name?var=value)`. A form is `-- form --`.
That is the whole vocabulary, and it is the same markdown you would have written to
document the API.

## Docs

- [`docs/RQ-FORMAT.md`](docs/RQ-FORMAT.md) — the `rq` file format: the request document, the project, variables, chaining.
- [`docs/cross-q.md`](docs/cross-q.md) — the cross-q converter: product + architecture.
- [`docs/IDEALISED.md`](docs/IDEALISED.md) — the Idealised Model (the IR) in full.
- [`docs/FORMAT.md`](docs/FORMAT.md) — the Requestly `LOCAL_FS` on-disk format `cross-q` writes.
- [`docs/CONTEXT.md`](docs/CONTEXT.md) — the `rq.*` scripting runtime spec.

## Install

There are no packages yet — no Homebrew tap, no apt repo, no release binaries. Until there
are, it is one command, and you need a Rust toolchain ([rustup](https://rustup.rs)):

```bash
git clone https://github.com/browserstack/rq && cd rq
cargo install --path crates/rq            # → ~/.cargo/bin/rq
cargo install --path crates/rq-testbed    # optional: the demo backend
```

Check it landed:

```bash
rq --version
rq --help
```

If `rq` isn't found afterwards, `~/.cargo/bin` isn't on your `PATH` — rustup normally adds
it to your shell profile, and `export PATH="$HOME/.cargo/bin:$PATH"` fixes it for the
session.

> **A name to know about.** Requestly's own npm CLI (`@requestly/cli`) also installs a
> binary called `rq`. If you have both, whichever comes first on your `PATH` wins.

To pick up changes later, `git pull` and run the same `cargo install` again; it replaces the
installed binary in place. `cargo install --path crates/rq --locked` builds against the
committed `Cargo.lock` if you would rather not resolve fresh dependency versions.

## Build

```bash
cargo test        # run everything
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Rust 1.85+ (2021 edition). One workspace, one version train. `cargo run -p rq -- <args>`
runs the CLI out of the working tree without installing it.

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
