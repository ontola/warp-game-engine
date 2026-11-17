# Warp Humans animation rig

The skeleton Warp Humans are fitted to: `player-rig.json` (joint names,
inverse binds, clip bounds) and `player.anim.bin` (its baked pistol idle,
run and jump palettes). A copy of `games/growing-guns/assets/characters`,
built into the `warp` runtime for Luau games' `humans` (see
`engine/crates/warp/src/humans.rs`). They are posed with the `body` crate's
captures, not these clips.

The rig and clips come from the original Growing Guns project's
Mixamo-rigged knight and pistol animations; `provenance.json` records the
source hashes. The upstream software notice is in
`SOURCE-SOFTWARE-LICENSE.txt`. Third-party asset rights remain with their
original owners.
