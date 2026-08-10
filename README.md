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
| **Bruno** | `.bru` v2 | 🏗️ | 🔜 | text DSL — request-level import done; folder-tree ingestion next |
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
