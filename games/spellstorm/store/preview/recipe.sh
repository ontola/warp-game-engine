# Spellstorm's store preview, for engine/tools/store-preview.sh: calm level 1
# with two wizards, a burst of level-3 chaos, then the Gravekeeper (Joep, 23
# Nov 2026). Two bots play (seed 11); the HUD is off.
#
# Capture later levels early in their run (ticks 300 to 600): from about tick
# 1800 the party has wiped and the run is back in the chapel.

capture() {
  (cd "$ROOT" && cargo build --locked --release -p spellstorm-app)
  local exe=$ROOT/target/release/spellstorm
  shot() {
    local dir=$1
    shift
    mkdir -p "$dir"
    (cd "$ROOT/games/spellstorm" && SPELLSTORM_NO_HUD=1 "$exe" shot --out-dir "$dir" --every 2 \
      --size 960x540 --humans 0 --bots 2 --seed 11 "$@")
  }
  # 1. The Forsaken Chapel: two wizards at work.
  shot "$OUT/calm" --biome 0 --from 240 --count 90
  # 2. Level 3: a Soaking Thunderbolt Strike.
  shot "$OUT/chaos" --biome 2 --from 600 --count 90 --cast water,lightning --cast-at 630
  # 3. The Gravekeeper, a little closer.
  SPELLSTORM_ZOOM=0.6 shot "$OUT/boss" --biome 0 --from 400 --count 90 --spawn gravekeeper

  cut "$OUT/calm" 0 69
  cut "$OUT/chaos" 0 39
  cut "$OUT/boss" 50 89
  POSTER=00020.png
}
