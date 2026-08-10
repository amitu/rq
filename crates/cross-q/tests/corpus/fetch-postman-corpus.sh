#!/usr/bin/env bash
# Fetch Postman's collection-transformer example corpus at a PINNED commit into a local,
# gitignored dir. We do NOT vendor these files: Postman's demo collections contain
# secret-shaped dummy OAuth values that (correctly) trip secret scanners, so we keep them
# out of the repo and fetch a pinned, reproducible snapshot instead.
#
# Deterministic: the SHA is pinned in ./postman-transformer.pin. The daily staleness
# watcher bumps that pin via PR. Source: postmanlabs/postman-collection-transformer
# (Apache-2.0, © Postman, Inc.) — used here as third-party test data, not redistributed.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
PIN="$(tr -d '[:space:]' < "$DIR/postman-transformer.pin")"
DEST="$DIR/postman-transformer"

if [ -d "$DEST" ] && [ -n "$(ls -A "$DEST" 2>/dev/null || true)" ]; then
  echo "postman corpus already present at $DEST (pin $PIN)"
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
URL="https://github.com/postmanlabs/postman-collection-transformer/archive/${PIN}.tar.gz"
echo "fetching pinned corpus: $URL"
curl -fsSL "$URL" | tar xz -C "$TMP"

SRC="$TMP/postman-collection-transformer-${PIN}/examples"
mkdir -p "$DEST"
for v in v1.0.0 v2.0.0 v2.1.0; do
  cp -R "$SRC/$v" "$DEST/$v"
done
echo "postman corpus ready at $DEST (pin $PIN)"
