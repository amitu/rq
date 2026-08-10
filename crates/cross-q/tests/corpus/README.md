# Test corpora (pinned fetch, not vendored)

cross-q's parsers are validated against **real, third-party collections** — not just our own
fixtures. Nothing here is vendored: provider/demo collections carry secret-shaped placeholder
values that correctly trip secret scanners, so for each corpus we commit a pinned SHA + a
fetch script and keep the downloaded data **gitignored**. Deterministic (pinned SHA),
reproducible, no secrets in the repo, not redistributed. Every corpus test **fails loud** if
its data isn't fetched (a corpus test that silently skipped would be a false green); CI runs
the fetch scripts before tests.

| Corpus | Format | Job | Canonical? | Test |
|---|---|---|---|---|
| **real-world** (Adyen, newman) | Postman | **fidelity** — no hollow parse, bounded round-trip loss | ✅ yes | `postman_realworld.rs` |
| **transformer** (postman-collection-transformer) | Postman | **crash-safety + tolerance** — parses odd/plural shapes without loss | mixed (incl. plural-shape fixtures — now tolerated) | `postman_corpus.rs` |
| **bruno-testbench** (usebruno/bruno) | Bruno | **fidelity** — no hollow parse over a real directory tree | ✅ yes | `bruno_corpus.rs` |

## 1. Real-world corpus — the fidelity oracle

Collections exported by real API providers in the wild:

- **[`Adyen/adyen-postman`](https://github.com/Adyen/adyen-postman)** (MIT) — 18 canonical
  singular-key **v2.1** collections (latest per service: Checkout, BalancePlatform,
  Management, LegalEntity, …). Rich auth, bodies, variables, and saved responses.
- **[`postmanlabs/newman`](https://github.com/postmanlabs/newman)** (Apache-2.0) — the
  **v2.0** sample collection + a **v1.0.0** legacy collection, for cross-version coverage.

Pins: `realworld.pin`. Fetch: `fetch-realworld-corpus.sh` → gitignored `./realworld/`.
`postman_realworld.rs` asserts two things stronger than "it parsed":

1. **No hollow parse** — every request in the source survives into `MappedItems` (equal
   request counts, source vs re-emitted). This is the guard the transformer corpus could
   never give: it read *near-empty* and still "passed".
2. **Bounded round-trip loss** — Postman → IR → Postman drops only keys on a documented
   allowlist (each with a rationale in the test). Any *new* dropped key fails the test —
   the "don't silently ignore a field" gate. The key-diff runs only on the v2.1 subset
   (same dialect in and out); v1/v2.0 fidelity is covered by request-count parity.

## 2. Transformer corpus — crash-safety only

[`postman-collection-transformer`](https://github.com/postmanlabs/postman-collection-transformer)
`examples/` (Apache-2.0): the same collection expressed in v1.0.0 / v2.0.0 / v2.1.0.

**Note:** these are the transformer *library's* test fixtures, not Postman **app** exports,
and they're a **mix**: most are canonical singular `header`/`response`/`event`, but several
(e.g. `box`, `github`, `twitter`, `proper-url-parsing`, `rawjsonbody` — ~5 of the 13 v2.1
files) use a **plural** shape (`headers`/`responses`/`events`). Since Postman published these,
the parser now **tolerates plural** as an alias for the singular keys (identical value shape),
so nothing is silently dropped — e.g. `box.json` recovers 92 requests / 91 headers that were
previously read empty. This corpus is still not a *round-trip fidelity* oracle for the plural
files (we emit canonical singular, so a byte key-diff shows `headers`→`header` as a
difference — correct normalization, not loss); corpus-wide fidelity lives in the real-world
corpus above.

Pin: `postman-transformer.pin`. Fetch: `fetch-postman-corpus.sh` → gitignored
`./postman-transformer/`. Test: `postman_corpus.rs`.

## 3. Bruno corpus — the directory-import fidelity oracle

[`usebruno/bruno`](https://github.com/usebruno/bruno)'s own `bruno-tests` collection (MIT) —
a large, canonical `.bru` v2 **directory tree**: nested folders, `environments/*.bru`,
`collection.bru`/`folder.bru` inheritance, and every auth/body type. Same role for the Bruno
directory importer that Adyen plays for Postman.

Pin: `bruno.pin`. Fetch: `fetch-bruno-corpus.sh` → gitignored `./bruno-testbench/`. Test:
`bruno_corpus.rs` reads the tree into the virtual-FS map the host would pass, then asserts
**no hollow parse** — every request `.bru` file becomes a request in the workspace (equal
counts) — plus that folders come through as nested collections and environments carry their
variables.

## Running

```bash
crates/cross-q/tests/corpus/fetch-realworld-corpus.sh   # Postman fidelity (Adyen + newman)
crates/cross-q/tests/corpus/fetch-postman-corpus.sh     # Postman crash-safety (transformer)
crates/cross-q/tests/corpus/fetch-bruno-corpus.sh       # Bruno fidelity (bruno-testbench)
cargo test -p cross-q --test postman_realworld --test postman_corpus --test bruno_corpus
```

Both tests **fail loud** if their corpus hasn't been fetched — a corpus test that silently
skipped would be a false green. CI runs the fetch scripts before tests. The daily staleness
watcher bumps the pins via PR; the weekly watcher opens an issue on a new schema version.

## Attribution & license

Fetched files are third-party test data — fetched at test time, not vendored, not
redistributed, not relicensed. This repository is MIT.

- Adyen collections © Adyen, **MIT** (https://github.com/Adyen/adyen-postman/blob/main/LICENSE).
- newman examples © Postman, Inc., **Apache-2.0** (https://github.com/postmanlabs/newman/blob/develop/LICENSE.md).
- transformer examples © Postman, Inc. and contributors, **Apache-2.0**
  (https://github.com/postmanlabs/postman-collection-transformer/blob/main/LICENSE.md).
- bruno-tests collection © usebruno, **MIT** (https://github.com/usebruno/bruno/blob/main/license.md).
