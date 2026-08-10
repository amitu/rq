#!/usr/bin/env bash
# Fetch a REAL-WORLD, CANONICAL Postman corpus at PINNED commits into a local, gitignored
# dir. Unlike the postman-collection-transformer `examples/` (a non-canonical PLURAL-key
# dialect — good for crash-safety, wrong for fidelity), these are collections exported by
# real API providers in the wild: canonical singular-key v2.1 (Adyen), plus v2.0 and v1
# (Postman's own newman examples). They are the fidelity oracle for the round-trip and the
# app differential-equivalence gate.
#
# We do NOT vendor them: provider collections carry secret-shaped placeholder values that
# (correctly) trip secret scanners. Pinned + fetched + gitignored = reproducible, no secrets
# in the repo, not redistributed. Pins live in ./realworld.pin; the version watcher bumps
# them via PR.
#
#   Adyen/adyen-postman   — MIT        — 18 canonical v2.1 services (latest per service)
#   postmanlabs/newman    — Apache-2.0 — v2.0 sample + v1 legacy collection
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
DEST="$DIR/realworld"

adyen_sha() { awk '/^Adyen\/adyen-postman/ {print $3}' "$DIR/realworld.pin"; }
newman_sha() { awk '/^postmanlabs\/newman/ {print $3}' "$DIR/realworld.pin"; }

if [ -d "$DEST" ] && [ -n "$(ls -A "$DEST" 2>/dev/null || true)" ]; then
  echo "real-world corpus already present at $DEST"
  exit 0
fi

ADYEN="$(adyen_sha)"
NEWMAN="$(newman_sha)"
[ -n "$ADYEN" ] && [ -n "$NEWMAN" ] || { echo "error: missing pin(s) in realworld.pin" >&2; exit 1; }

# Fetch <repo> <sha> <path-in-repo> <out>. Prefer the raw CDN (works in CI, no auth);
# fall back to the GitHub API via `gh` (works in sandboxes where the raw CDN is blocked).
fetch() {
  local repo="$1" sha="$2" path="$3" out="$4"
  echo "  fetch $out"
  if curl -fsSL "https://raw.githubusercontent.com/$repo/$sha/$path" -o "$out" 2>/dev/null; then
    return 0
  fi
  if command -v gh >/dev/null 2>&1 &&
     gh api "repos/$repo/contents/$path?ref=$sha" --jq '.content' 2>/dev/null | base64 -d > "$out" 2>/dev/null &&
     [ -s "$out" ]; then
    return 0
  fi
  echo "error: could not fetch $repo/$path @ $sha (raw CDN and gh both failed)" >&2
  return 1
}

# --- Adyen: latest collection per service (canonical v2.1) -----------------------------
mkdir -p "$DEST/adyen"
ADYEN_FILES=(
  BalanceControlService-v2 BalancePlatformService-v2 BinLookupService-v54
  CapitalService-v1 CheckoutService-v72 DataProtectionService-v1
  DisputeService-v30 ForeignExchangeService-v1 LegalEntityService-v4
  ManagementNotificationService-v3 ManagementService-v3 PayoutService-v68
  RecurringService-v68 SessionAuthenticationService-v1 StoredValueService-v46
  TestCardService-v1 TfmAPIService-v1 TransferService-v4
)
echo "fetching Adyen (MIT) @ $ADYEN"
for f in "${ADYEN_FILES[@]}"; do
  fetch "Adyen/adyen-postman" "$ADYEN" "postman/$f.json" "$DEST/adyen/$f.json"
done

# --- newman: v2.0 sample + v1 legacy (Apache-2.0) --------------------------------------
mkdir -p "$DEST/newman"
echo "fetching newman (Apache-2.0) @ $NEWMAN"
fetch "postmanlabs/newman" "$NEWMAN" "examples/sample-collection.json" "$DEST/newman/sample-collection-v2.0.json"
fetch "postmanlabs/newman" "$NEWMAN" "examples/v1.postman_collection.json" "$DEST/newman/legacy-v1.json"

count=$(find "$DEST" -name '*.json' | wc -l | tr -d ' ')
echo "real-world corpus ready at $DEST ($count collections)"
