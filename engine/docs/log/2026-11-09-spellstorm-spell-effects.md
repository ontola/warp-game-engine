# Spellstorm spell effects, first design pass, 9 November 2026

Base commit 43eb867. Cloud container, lavapipe (software Vulkan), no real
GPU: images only, no timings. Guide: [Spellstorm spell
effects](../../../games/spellstorm/docs/effects.md).

## Captures

45 scenes in the chapel at 1920x1080, cropped to the middle 960x540:
staff bolts of every element (`--bolts`), sprays and beams (`--cast-at 560
--cast X,X`), a strike, three throws, and a nova of 13 elements 6, 20 and 60
ticks after it (`--lua-at 600`). `--bolts` and `--cast-at` are new in
`spellstorm shot` for this.

## What was found

- **Fire turned pink.** Flame particles were bright saturated red-orange,
  which AgX turns salmon. The `flame` look also wrote its colours as if they
  were sRGB into the linear image (its yellow showed as cream). Lowering the
  glow alone (0.55 down to 0.005) never brought orange back: the hue came
  from the shader's near-white heart and from heat being near 1 over most of
  each particle. Fixed by sRGB stops through `lin()`, heat that falls with
  age so most gas is orange, oranges that lean yellow, and glow 0.8.
- **The flamethrower's lights** (3.2 and 2.4 energy) lit the floor brighter
  than the flames; now 1.1 and 0.85. Its flames are drawn stretched along
  their flight (0.04 m per m/s), which turned a column of puffs into a jet.
- Every other bolt was a halo and a dot; frost breath and wind gusts were
  confetti. Rebuilt per element (see the guide): ice spikes, water and
  poison orbs, a black void disc, rocks, molten rock, a vortex ring.
- A slow `Ember` showed a square: its glow did not reach zero at the quad's
  edge.

## Open

- Timing and a look on Mancave with four players and many bolts at once
  (particle budget 3000, sprites 4096).
- GPU extras on crashes (`KERN_GPU_PARTICLES=1`) were not captured.
- Fusions' own touches, fields, traps and overloads with the new layers.
