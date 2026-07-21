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
| `crates/cross-q-context` | `@cross-q/context` (npm), `cross-q-context` (PyPI + crate) | A QuickJS-based runtime that executes pre-request / post-response scripts against the `rq.*` API, backward-compatible with Postman's `pm.*`. | 🔜 planned |
| `crates/rq` | `rq` | The CLI: named requests, declared chaining, responses rendered as legible markdown. | 🔜 planned |

## Docs

- [`docs/cross-q.md`](docs/cross-q.md) — the cross-q converter: product + architecture.
- [`docs/IDEALISED.md`](docs/IDEALISED.md) — the Idealised Model (the IR) in full.
- [`docs/FORMAT.md`](docs/FORMAT.md) — the Requestly on-disk format `rq` reads and writes.
- [`docs/CONTEXT.md`](docs/CONTEXT.md) — the `rq.*` scripting runtime spec.

## Build

```bash
cargo test        # run everything
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Rust 1.85+ (2021 edition). One workspace, one version train.

## Why

The fastest-growing API tools are the ones that keep your work in plain files you own. `rq`
leans all the way into that: git-native by default, convertible from whatever you have
today, and honest about what it can and can't do with your data. Reliability is the
feature — see [`docs/cross-q.md`](docs/cross-q.md).
