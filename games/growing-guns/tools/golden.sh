#!/usr/bin/env bash
# Golden-image check for Growing Guns: renders scripts/golden.json on the
# software adapter (deterministic on one machine) and compares every capture
# with scripts/golden/*.png. Exits non-zero on any difference.
#   games/growing-guns/tools/golden.sh            # check (debug build)
#   games/growing-guns/tools/golden.sh --bless    # replace the references
# References are made with the software adapter; a different GPU or driver
# renders slightly differently, so compare like with like.
set -euo pipefail
root="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$root"
profile="${PROFILE:-debug}"
flag=(); [ "$profile" = release ] && flag=(--release)
cargo build -q "${flag[@]}" -p growing-guns-app
if [ "${1:-}" = "--bless" ]; then rm -f games/growing-guns/scripts/golden/*.png; fi
out="$(mktemp -d)"
KERN_ADAPTER=software "target/$profile/growing-guns" script games/growing-guns/scripts/golden.json --out "$out" \
  | tee "$out/log.jsonl" | grep -E '"compare"|"expect"|"done"'
grep -q '"failed":0' "$out/log.jsonl"
