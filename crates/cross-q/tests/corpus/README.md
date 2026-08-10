# Postman corpus (pinned fetch, not vendored)

cross-q's Postman parsers are validated against Postman's **own** cross-version example
collections from
[`postman-collection-transformer`](https://github.com/postmanlabs/postman-collection-transformer)
(`examples/` — the same collection expressed in v1.0.0 / v2.0.0 / v2.1.0).

## Why fetched, not vendored

Those demo collections contain **secret-shaped dummy values** (e.g. OAuth consumer
secrets in `twitter`/`box`) that correctly trip secret scanners. Rather than commit
third-party secret-shaped data into this repo, we **fetch a pinned snapshot**:

- `postman-transformer.pin` — the exact upstream commit SHA (deterministic, reproducible).
- `fetch-postman-corpus.sh` — downloads that pinned tarball into `./postman-transformer/`
  (which is **gitignored** — never committed).

This keeps the corpus reproducible (pinned SHA) *and* keeps secret-shaped data out of the
repo. The daily staleness watcher bumps the pin via PR when upstream changes; the weekly
watcher opens an issue on a new schema version.

## Running the corpus test

```bash
crates/cross-q/tests/corpus/fetch-postman-corpus.sh   # one-time, into the gitignored dir
cargo test -p cross-q --test postman_corpus
```

`tests/postman_corpus.rs` **fails loud** if the corpus hasn't been fetched (a corpus test
that silently skipped would be a false green). Run the fetch script once; CI runs it before
tests.

## Attribution & license

Fetched files are © Postman, Inc. and contributors, under the **Apache License 2.0**
(https://github.com/postmanlabs/postman-collection-transformer/blob/main/LICENSE.md). They
are third-party test data, fetched at test time — not vendored, not redistributed, not
relicensed. This repository is MIT.
