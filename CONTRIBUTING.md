# Contributing to `rq`

You **use** `rq` tools the way you install anything — `npm`, `pip`, a CLI binary. You only
meet Rust if you want to change the **core**, because the shared engine is written in Rust
and compiled to each ecosystem (a native crate, a WebAssembly build for npm, a wheel for
PyPI). If you're a JavaScript or Python dev, this page is your on-ramp: a weekend of Rust is
plenty to be useful.

## You can contribute with zero Rust

- File issues, improve docs, add conversion **fixtures** (input → expected output JSON).
- Work on the **JS/Python binding glue** and their tests (`@cross-q/context`, the PyPI wheel).
- Report real-world collections that convert badly — these are gold.

## Learn just enough Rust (≈ a weekend)

Do these in order — you can stop after step 3 for most contributions:

1. **Install the toolchain** — [rustup.rs](https://rustup.rs). Gives you `cargo` (think
   `npm` + `node` + `jest` + `eslint`/`prettier` in one tool).
2. **The Rust Book, ch. 1–6 + 9–10** — [doc.rust-lang.org/book](https://doc.rust-lang.org/book/):
   ownership & borrowing, structs/enums, `Option`/`Result` error handling, generics/traits.
   That's 90% of what you'll read here.
3. **Rustlings** — [github.com/rust-lang/rustlings](https://github.com/rust-lang/rustlings):
   small hands-on exercises. The fastest way to make it stick.
4. *(binding work only)* **wasm-pack** — [rustwasm.github.io/wasm-pack](https://rustwasm.github.io/wasm-pack/)
   for the npm/WASM build; [PyO3](https://pyo3.rs) for the Python wheel.

Keep [Rust by Example](https://doc.rust-lang.org/rust-by-example/) open as a phrasebook.

## The essentials (everything repo-specific, in < 500 words)

**Mental model.** One Rust core, three shipping shapes. Logic lives **once** in the Rust
crates under `crates/`; the npm and PyPI packages are thin bindings that call the same code.
So a bug fix in a crate fixes it everywhere. You'll spend most time in `crates/`, not in
binding code.

**Layout.** It's a single Cargo *workspace* (one version train, `members = ["crates/*"]`):

| Path | What it is |
|---|---|
| `crates/cq-model` | The **Idealised Model** — the canonical intermediate representation every importer/exporter maps through. The contract. |
| `crates/cross-q` | The `cq` converter binary (Postman ↔ Requestly ↔ Insomnia ↔ Bruno ↔ HAR ↔ OpenAPI ↔ cURL). |
| `crates/cq-report` | The "what couldn't be carried cleanly" report. |
| *(planned)* `crates/cross-q-context` | The QuickJS runtime for `rq.*` scripts — ships as crate + npm (WASM) + PyPI wheel. |
| *(planned)* `crates/rq` | The `rq` CLI. |

Deep specs live in `docs/` (`IDEALISED.md`, `CONTEXT.md`, `FORMAT.md`, `cross-q.md`) — read
the one nearest your change.

**Build & test loop** (this is the whole thing):

```bash
cargo test                                 # run everything (like `npm test`)
cargo clippy --all-targets -- -D warnings  # the linter — must be clean
cargo fmt                                  # the formatter — run before committing
```

Rust 1.87+, 2021 edition. `cargo` downloads dependencies on first `test` — no separate
install step.

**Contribution loop.**

1. Branch off `main`.
2. Make the change. Prefer adding a **fixture** that fails, then making it pass — most of
   this codebase is data-in → data-out, so a failing example is the best bug report and the
   best test.
3. `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt` — all green.
4. Open a PR. Describe the behaviour change and link the fixture.

**Conventions.** `snake_case` fields; return `Result<_, _>` and use `?` rather than panicking;
keep the Idealised Model (`cq-model`) as the source of truth — importers/exporters map *to
and from* it, never to each other directly.

**Where to start.** Look for `good-first-issue`, or add a conversion fixture for a tool you
use. If you're stuck on Rust itself, open a draft PR and ask — a rough PR from a JS/Python dev
is exactly what we want to help across the line.
