#!/usr/bin/env bash
# Fetch a real-world Bruno collection at a PINNED commit into a local, gitignored dir.
# We use usebruno's own `bruno-tests` collection (MIT) — a large, canonical `.bru` v2
# directory tree (folders, environments, scripts, every auth/body type). It's the fidelity
# oracle for the Bruno directory importer, the same role Adyen plays for Postman.
#
# Not vendored: the collection carries secret-shaped placeholder values. Pinned + fetched +
# gitignored = reproducible, no secrets in the repo, not redistributed. Pin: ./bruno.pin.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
DEST="$DIR/bruno-testbench"
SUBDIR="packages/bruno-tests/collection"
SHA="$(awk '/^usebruno\/bruno/ {print $3}' "$DIR/bruno.pin")"
[ -n "$SHA" ] || { echo "error: no usebruno/bruno pin in bruno.pin" >&2; exit 1; }

if [ -d "$DEST" ] && [ -n "$(ls -A "$DEST" 2>/dev/null || true)" ]; then
  echo "bruno corpus already present at $DEST (pin $SHA)"
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
URL="https://github.com/usebruno/bruno/archive/${SHA}.tar.gz"
echo "fetching pinned bruno corpus: $URL"
curl -fsSL "$URL" | tar xz -C "$TMP"

SRC="$TMP/bruno-${SHA}/$SUBDIR"
[ -d "$SRC" ] || { echo "error: $SUBDIR not found in tarball" >&2; exit 1; }
mkdir -p "$DEST"
cp -R "$SRC/." "$DEST/"
echo "bruno corpus ready at $DEST ($(find "$DEST" -name '*.bru' | wc -l | tr -d ' ') .bru files, pin $SHA)"
