#!/usr/bin/env bash
# The models (monsters, the wizard, every biome's props, wall rocks, torches)
# exported from the Godot game (joepio/spellstorm, tools/warp_kits.gd) into
# app/assets/kits, so both engines draw the same shapes and materials.
#   games/spellstorm/tools/export-kits.sh [path/to/spellstorm]
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
godot_game="${1:-${SPELLSTORM:-$here/../../../spellstorm}}"
out="$here/app/assets/kits"
rm -rf "$out"
mkdir -p "$out"
godot --headless --path "$godot_game" res://tools/warp_kits.tscn -- "$out"
gzip -9 -n "$out/meshes.bin"
ls "$out" | wc -l
