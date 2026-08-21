# rq

> A better curl, powered by collections — and the tooling around it.

This is the monorepo for the **`rq`** family: a set of small, honest, single-binary tools
for people who live in API collections but don't want to live in a GUI or a cloud.

Everything here is Rust, MIT-licensed, plain files, no telemetry, no account.

## What's in here

| Crate | Binary / package | What it is | Status |
|---|---|---|---|
| [`crates/cross-q`](crates) | `cq` | Convert API-client collections between formats (today: Postman ↔ Bruno ↔ Requestly ↔ rq ↔ cURL; more below) through one idealised model, and report everything it couldn't carry cleanly. | 🏗️ building |
| `crates/cq-model` | — | The **Idealised Model** — the canonical intermediate representation every importer and exporter maps through. | ✅ v0.1 |
| `crates/cq-transform` + [`packages/cross-q-context`](packages/cross-q-context) | `@requestly/cross-q-context` (npm) | The scripting core: rewrite every dialect (`pm.*`, `postman.*`, `bru.*`) to the `rq.*` API (Rust/OXC → WASM), and execute it on a QuickJS runtime. | 🏗️ building |
| [`crates/rq`](crates/rq) | `rq` | The CLI: named requests, declared chaining, responses rendered as legible markdown. | 🏗️ building |
| [`crates/rq-testbed`](crates/rq-testbed) | `rq-testbed` | The demo API the examples talk to — a dependency-free server (`std::net` and JSON) with a small stateful app behind it. | ✅ v0.1 |
| [`crates/rq-doc`](crates/rq-doc) | — | The `rq` request document — one Markdown file per request — and the project layout around it. Read by the CLI, written by the converter. | ✅ v0.1 |

## Supported formats

The API-client formats `cq` converts between, by app and version. **Import** = read that
format into the Idealised Model; **Export** = write it out of the model. Support is
holistic (all of a version's features in one go), not feature-by-feature.

**Legend:** ✅ supported · 🏗️ in progress · 🔜 planned · — n/a

| App | Version | Import | Export | Notes |
|---|---|:--:|:--:|---|
| **Postman** | Collection v2.1.0 | ✅ | ✅ | 47/47 of Postman's own corpus parse; `--to postman` |
| **Postman** | Collection v2.0.0 | ✅ | 🔜 | object-shaped auth |
| **Postman** | Collection v1.0.0 | ✅ | 🔜 | legacy flat `requests[]`/`folders[]` |
| **cURL** | command line | ✅ | 🔜 | single command ↔ request |
| **rq** | project (`*.md` + `rq.toml`) | ✅ | ✅ | the format the `rq` CLI reads — one Markdown file per request; round-trip proven by IR-idempotence |
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
  dialects is the job of the runtime — a QuickJS `rq.*` engine plus the `pm.*` → `rq.*`
  transform — not the converter. The converter carries source + dialect; `rq` reconciles them
  at execution.

  **A Postman script runs unmodified**, which is the point of carrying it verbatim. Export
  from Postman, point `rq` at the file, run it: the document records `script_dialect: pm` and
  every run reconciles the source through [`crates/cq-transform`](crates/cq-transform), an
  OXC-based transform that parses the script and rewrites identifiers *in scope* rather than
  string-replacing `pm.` and hoping. `pm.*`, the legacy `postman.setEnvironmentVariable(…)`,
  and v1's `tests['x'] = …` / `responseCode` / `responseBody` all work.

  **A Bruno script runs unmodified too**, by the other route: `bru`, `req` and `res` are
  *objects*, so the runtime simply provides them. Postman needs a rewrite because its v1 forms
  are syntax — `tests['ok'] = expr` is an assignment no object can intercept — while Bruno has
  nothing to rewrite. The mapped surface is the one usebruno's own 223-request collection
  actually uses; anything outside it throws **by name** rather than returning `undefined` and
  making the next line wrong.

## The `rq` CLI

Curl in, named verb out, editor for everything else:

```bash
cargo install --path crates/rq    # once — see Install below

rq curl --save-as issues 'curl -H "Accept: application/vnd.github+json" \
    "https://api.github.com/repos/anthropics/claude-code/issues?state=open"'

rq r issues        # run it — anytime, from anywhere in the project
rq e issues        # open it in $EDITOR
rq l               # the tree: every request, its method, what it depends on
rq check           # what a run would trip over: broken parents, unset vars, bad captures
rq fmt             # one shape for files people also edit by hand
rq r me -c         # the console: arrow between the steps of a run, drill into each
rq                 # the project's requests — bare `rq` is `rq l`
rq l -c            # browse them instead: arrow to one, enter to run it
```

`rq` on its own lists the project — it is exactly `rq l`, printing the same thing whether
or not anything is watching. `-c` browses instead: arrow to a request, `enter` runs it.
From any page, `l` brings that list back with the cursor on where you are. In the console,
a digit opens a link and `backspace` goes back — the run you are
reading becomes a place you can move around in. It is also the browser network panel for
your terminal — every step of the run, its
request, its response, its headers, and where the milliseconds actually went — over the run
you already did. Nothing is re-sent, and there is no second `--verbose` pass.

Each request is **one Markdown file** — `github/login.md`, no wrapper directory — frontmatter plus `-- description --`,
`-- view --`, `-- body --`, `-- pre --`, `-- post --` sections. The `-- view --` template
renders the response as markdown in your terminal, which is the thing no other client in
this category does. Dependencies are declared per request (`parents: [login]`) and the
values that flow between them are declared too (`capture: { token: response.access_token }`),
so the common chain needs no JavaScript at all.

The format is a first-class cross-q citizen in both directions — `cq convert x.postman_collection.json --to rq`
brings a collection in, and `cq convert ./my-apis --to bruno` takes it anywhere else — so
`rq import` is that converter, not a second implementation of it.

Full spec: [`docs/RQ-FORMAT.md`](docs/RQ-FORMAT.md).

**The files are edited by hand and by scripts, so there is a checker.** `rq check` reads
every file the way a run would and reports what a run would trip over — a `parents:` naming a
request that was renamed, a `capture:` path that can never match, a `-- view --` template that
stopped parsing, a `{{TOKEN}}` nothing provides. Errors exit 1; warnings do not unless you
pass `--strict`, because a run does not fail on them either. `--json` for CI.

A placeholder never reaches the wire: a request still carrying `{{TOKEN}}` is **not sent**, and
the run says which variable and how to supply it. `Bearer {{TOKEN}}` would come back 401 and
read like a credentials problem, sending you to the API instead of to your file. Declaring a
variable is how you say it may legitimately be empty — an empty credential is dropped, not
sent — and `required: true` is how you say it may not.

`rq fmt` rewrites requests in their canonical form, and `rq fmt --check` fails without
writing. Frontmatter keys and sections this build does not know are preserved verbatim —
formatting a file must never be how you find out something was dropped.

**You do not have to convert anything to start.** `rq` reads a Postman export, a Bruno
collection or a file of curl commands *in place* — the same converter `rq import` runs, in
memory:

```bash
rq l acme.postman_collection.json      # its requests, as a tree
rq r health                            # run one
rq --project ./bruno-collection l      # a directory works the same way
```

Drop into a folder that has one and bare `rq` finds it. The collection stays exactly as it
is, so something a colleague sent you is runnable before you have decided whether to keep it.
When you decide to keep it, `rq import <file>` makes it an rq project — the same conversion
you were already running, saved this time.

**Scripts run.** `-- pre --` and `-- post --` execute on
[cross-q-context](packages/cross-q-context) — the same QuickJS engine and the same `rq.*`
API the Requestly app uses, so a collection behaves the same in both. `rq.test(…)` results
print and set the exit code, `console.log` appears under its step, `rq.variables.set(…)`
reaches the next request in the graph, and `rq.request.headers.*` changes what goes on the
wire.

**Nothing to install.** The engine is compiled into the binary: QuickJS in-process, running
cross-q-context's own guest realm — the `rq.*` namespace, the `pm.*` shims, chai and the
`require`-able packages are source the package generates and `rq` evaluates. So the semantics
have one owner and no second implementation, and a downloaded release runs scripts on a
machine with no Node on it at all.

`require('crypto')`, `require('lodash')`, `Buffer`, `zlib`, `fetch` and `rq.sendRequest` all
work; a package a sandbox genuinely cannot serve (`fs`, a native addon) says so and why.

Setting `RQ_SCRIPT_ENGINE=/path/to/cross-q-context` runs scripts through Node against a
checkout instead — for working on the engine and comparing the two.

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

## Cookies

A chain like `login → me` usually carries its session in a `Set-Cookie`, so `rq` keeps a jar
for the length of a run. It is **not** written to disk unless you ask, because session cookies
are credentials and a client that quietly stored yours would be storing them without consent.

```bash
rq r login --cookies                    # keep them in .rq/cookies.json (gitignored)
rq r me --cookies                       # a later run, still logged in
rq r me --cookies ~/.rq/work.json       # or wherever you want them
cat .rq/cookies.json                    # it is just a file
rm .rq/cookies.json                     # and that is how you log out
```

There is no `rq cookies list` or `rq cookies clear`, because there is nothing worth wrapping:
the path is explicit, so `cat` and `rm` already do it.

## The network log

The console is a panel over the run you just did, because that is all one process knows.
`--log` makes it a panel over your work:

```bash
rq r issues --log                  # append to .rq/log.jsonl (gitignored)
rq r issues --log ~/net.jsonl      # or wherever
rq r issues --log -c               # the console; backspace opens everything sent before
tail -f .rq/log.jsonl              # it is a file
jq -c 'select(.status >= 500)' .rq/log.jsonl
rm .rq/log.jsonl                   # and that is how you clear it
```

**JSONL, one object per request.** Appending is a write rather than a read-modify-write, so
two `rq` processes finishing at once cannot lose each other's requests; a torn write costs one
line instead of the whole history; and `tail`, `jq` and `wc -l` already work on it. Secrets are
redacted on the way in, with the same list the terminal redacts with — a log outlives the run,
and it should not keep the credential the run only borrowed.

## Piping, scripts and CI

One rule: **stdout is the result, stderr is the narration, and the result is the same
either way.** Only two things change when nothing is watching — where the narration lands
visually, and whether the console opens.

```bash
rq r issues                 # a terminal: the rendered view, plus the console
rq r issues > report.md     # the same rendered view, alone, in the file
rq r issues --raw | jq .    # the response body
rq r issues --json | jq .   # the whole run: status, headers, timings, tests, captures
rq --json | jq '.requests'  # the project itself, for tooling and completions
```

The step tree, captured values, test lines and notes go to **stderr**, so they stay visible
while you work and stay out of your data when you pipe. Nothing is ever asked on a
terminal that isn't there: a `-- form --` is skipped, prompts are skipped, and a missing
required value is an error rather than a hang.

`--json` on a run carries what the terminal can't show anyway — every header, the per-phase
timings, each test's status — which is what makes it the shape for CI:

```bash
rq r checks --json | jq -e '.tests.failed == 0'
```

`rq check` is the other half of that, and it sends no requests at all — a gate that runs on a
pull request touching the collection, before anything is spent:

```bash
rq check --strict            # exit 1 on anything, including warnings
rq check --json | jq '.findings[] | select(.level == "error")'
rq fmt --check               # exit 1 if any file is not in canonical form
```

Exit codes: **0** normally, **1** when a `rq.test(...)` failed (no flag needed — an
assertion that fails quietly is how people stop trusting a runner) or when `--fail` is set
and the response wasn't 2xx, **2** for anything rq itself couldn't do.

`--no-console` turns off the interactive layer anywhere, for recordings and for scripts
that do run in a terminal.

## Not built yet

Named here so nobody has to find out by trying. Roughly in the order they matter:

**Running.**
- data-driven iteration (`-d data.csv`, `-n 5`) and a JUnit reporter — the runtime already
  carries `iteration`/`iteration_count`/`iteration_data` into every script, so this is CLI
  plumbing over an engine that is ready for it
- saved response examples — a fidelity gap in both directions today (the model carries
  `examples`, `rq` drops them), and the thing that would make offline runs and `rq diff`
  possible
- retries and backoff; proxies and client certificates

**Rendering.** Terminal-width-aware tables: columns are sized to their content, so a table
with long cells is wider than an 80-column window and wraps. Nothing is truncated.

**Formats.** OpenAPI/Swagger, Insomnia, HAR, Hoppscotch, `.http`, Hurl — see the table above.
Requestly is export-only; `curl` has no exporter.

**Packaging.** No release binaries, no Homebrew tap, no apt repo. `rq` is also taken on
crates.io, so publishing means a different crate name with `[[bin]] name = "rq"`.

**Not planned:** an editor. `rq e` hands the file to `$EDITOR`, and that is the whole feature —
see [`docs/RQ-FORMAT.md`](docs/RQ-FORMAT.md#not-planned-editing).

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

Rust 1.88+ (2021 edition). One workspace, one version train. `cargo run -p rq -- <args>`
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

Every pull request runs [`.github/workflows/ci.yml`](.github/workflows/ci.yml): rustfmt and
`clippy -D warnings`; the workspace suite on **Linux and macOS** after fetching all three
corpora; a `cargo check` on the MSRV we advertise; and the TypeScript runtime — its own
suites, a check that the committed `dist/` and the guest bundle rq compiles in are both in
sync with `src/`, and rq's engine integration tests with `RQ_REQUIRE_ENGINE=1`, which is the
one place the "no engine installed" skip can never fire.

## Why

The fastest-growing API tools are the ones that keep your work in plain files you own. `rq`
leans all the way into that: git-native by default, convertible from whatever you have
today, and honest about what it can and can't do with your data. Reliability is the
feature — see [`docs/cross-q.md`](docs/cross-q.md).
