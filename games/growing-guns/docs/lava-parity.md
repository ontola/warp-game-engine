# Lava: Godot original vs the Rust port

How the lava in the Rust port compares with the Godot original
(`/home/claude/growing-guns`, Godot 4.5.2), what was changed to close the
gap, and what is still different. The shots compared come from
`scripts/lava.json`. Run it with
`KERN_ADAPTER=software target/debug/growing-guns script games/growing-guns/scripts/lava.json --out DIR`.
The Godot references were captured with the same camera placement, through
a capture script run under xvfb and lavapipe (arena_procedural.tscn, glow,
fog and Filmic as the game has them).

## Where lava appears

| | Godot | Rust |
|---|---|---|
| Pool | `_build_lava_pool`: a 6 km plane at `LAVA_POOL_Y` = -3 under every arena, seen through holes and past an open arena's edge | 900 m slab with its top at `LAVA_KILL_Y` = -3 (`view/arena.rs`) |
| Arena variants | 18% all-floor-lava islands, 24% open, 16% hole, 42% walled | Same split, from the same single roll (see [The island arena](#the-island-arena)) |
| Island floor | `_build_lava_floor_surface`: a plane the size of the arena at `LAVA_FLOOR_SURFACE_Y` = 0.04, over the pool | 10 cm lava slab with its top at 0.04 (`view/things.rs`), rising with the leak |
| Leak (sudden death) | `arena_procedural.gd`: after 30 s (or at once with the Sudden Death modifier), 4 s down the inside of the walls, then floor fronts creep in from the edges (smoothstep over the rest of 20 s), then the surface rises 1.5 m/s to 18 m | Same timings in `sim/src/flow.rs`. Fronts and wall sheets in `view/things.rs` follow `_set_front_rect` and `LavaWallFlow`: 0.045 m thick fronts on the surface, at least 0.08 m wide once they show, and 0.45 m sheets at half - 0.62, growing down from the top |
| Particles, embers, heat haze | None for lava. The embers and heat distortion in Godot belong to explosions | None |

## Visuals

### Shader

Godot's `lava_flow.gdshader` is `unshaded`, so it shows ALBEDO only; its
EMISSION output is ignored in that mode. It reads a 256 px noise texture
(`get_lava_noise_texture`: FastNoiseLite OpenSimplex2S FBm, frequency 0.07,
five octaves, seed 90210) at `world.xz * 0.048 * 0.01` through two 4-octave
fbm stacks. The noise drifts with `TIME * 0.035 * 0.42` and mixes the
`source_color` colours cool, hot and bright in linear space.

The Rust port, in `engine/crates/render/src/shaders/procedural_surface.wgsl`
(`proc_lava`):

- **Noise.** It now samples the exact texture. `view/lava.rs::noise_texture`
  generates it with the fastnoise-lite crate, and it matches Godot's
  exported image texel for texel (a test pins sample texels). It is handed to
  the renderer as `Scene::noise`, a new general engine feature: a mipmapped
  R16Float tiling field at scene binding 28, sampled with
  `textureSampleLevel` at the mip level of the pixel footprint, like Godot's
  `filter_linear_mipmap`. Before this, the shader used a gradient noise with
  the same statistics but a different pattern; it remains as the fallback
  for scenes without a field.
- **Colours.** The constants are now decoded with the exact sRGB curve, as
  Godot does for `source_color`, instead of `pow 2.2` (cool was 25% too dark).
- **Radiance.** It is `tint * scale` (material fields), so a game can fit the
  emission per channel.
- **Device limit.** The scene layout now samples 17 textures per stage, so
  the device asks for `min(adapter, 32)` (`gpu.rs`); WebGPU's default is 16.

### Colour and brightness: the remaining gap

Godot shows the lava through Filmic (exposure 1.08) with glow (intensity
0.9, bloom 0.2), so molten lava reads as saturated yellow-orange. The engine
tonemaps with AgX, which desaturates bright colours towards white and
cannot reach that saturation at that brightness. The hot colour displays as
at best about (212, 92, 52) in sRGB, against Godot's (250, 99, 6). The
decision is to stay with AgX. The lava's radiance is the best fit to the
Godot captures under AgX: `LAVA_TINT` (1, 0.92, 0) times `LAVA_NITS` 45000
(`view/arena.rs`). That is a red-leaning white with no blue; AgX's channel
crosstalk supplies the blue.

Mean CIELAB difference (ΔE76) from the Godot captures, per lava region:

| Render | hole | open | leak_eye | leak_mid | mean |
|---|---|---|---|---|---|
| Before (approx. noise, 12000 nits, sRGB-space mix) | 19.6 | 58.2 | 29.1 | 51.5 | 39.6 |
| Earlier WIP (approx. noise, 30000 nits, linear mix) | 18.9 | 43.4 | 23.3 | 40.2 | 31.5 |
| Fit (Godot noise, 45000 nits, tint 1, 0.92, 0; no leak fronts yet) | 15.2 | 31.0 | 25.3 | 10.2 | 20.4 |
| Now (the same, with the leak fronts, pool level and lava light) | 15.1 | 33.9 | 27.4 | 9.2 | 21.4 |

Other radiances were rendered and measured the same way. A fit of the
tonemapper alone suggested tint (1, 0.808, 0) at 21450 nits, but rendered it
scores 26.6; (1, 0.85, 0) at 26000 to 55000 nits scores 21.0 to 23.7. The
regions, in the 960 x 540 captures: hole x 380..560, y 200..300; open x
0..500, y 300..539; leak_eye x 0..500, y 350..539; leak_mid x 0..100, y
280..400.

Mean colour over the same regions, sRGB (the fit):

| Shot | Godot | Now |
|---|---|---|
| hole | (248, 187, 81) | (241, 198, 114) |
| leak_mid | (242, 182, 92) | (237, 183, 106) |
| leak_eye | (240, 139, 39) | (202, 100, 49) |
| open | (247, 181, 76) | (211, 118, 60) |

What is left of the gap:

- **AgX saturation.** Highlights are paler and pinker, the darker crust is
  redder and browner.
- **Fog.** Godot's exponential fog (density 0.01, a light grey) lightens and
  flattens far lava. The engine's linear fog starts at 40 m, so far lava in
  the open and leak views stays darker and more contrasted. This is a global
  atmosphere difference, not a lava one.
- **Glow.** Godot's glow adds about +10 green on the lava and a soft halo onto
  the nearby stone. The engine's bloom has a threshold of 2 exposed and
  strength 0.04, so lava barely blooms.
- **Pattern position.** In the game, Godot's arena sits at `ARENA_OFFSET`
  (1.74, 4.54, 16.38), so its in-game pattern is shifted by that much. The
  references and the port use the arena-local origin.

### Light on the surroundings

The view now casts the light lava gives off, in `view/lava.rs::LavaLight`.
There was none before.

- **Pool light** (Godot `lava_floor_openness.gd`): an unshadowed orange omni
  at the pool + 0.6 m. Its energy is 5.5 times the floor openness to the
  power 0.72, its range 0.85 times the arena size, attenuation 0.65.
  Openness is measured per 2.8 m column with Godot's ray (from just above
  the lava up to 1.35 m) and re-measured when chunks break.
  - It copies a Godot quirk: the ray starts inside an intact floor chunk, so
    an intact floor reads nearly open, and the pool light is near full
    strength in every walled arena too.
- **Leak light** (Godot `LavaLeakGlow`): up to 7.5 energy, scaled by how far
  the leak has spread and by the larger of the floor openness and 0.92 times
  the leak's coverage. It sits at the leak surface + 0.45 m with range half
  the arena size times 1.35.
- **Calibration.** Godot energy converts to the engine at 37000 per unit, as
  for the blast lights.

### At night

Godot has no night; the port's procedural skies (`--sky night`,
`app/src/sky.rs`) expose a floodlit stadium at EV 11 instead of Godot's
13.6. Two things keep the lava reading as lava there.

- **Emission held at the day exposure.** `render::Lighting` has
  `emissive_compensation` (0 off, the default, to 1) and `emissive_ev100`.
  Emitted radiance (EMISSIVE and LAVA materials, car lamps; opaque and
  transparent) is multiplied by
  2^((ev100 - emissive_ev100) * emissive_compensation), so with 1 it is
  shown as at `emissive_ev100` under every sky. Growing Guns sets 1 and
  13.6 (`view::lighting`, `EMISSIVE_COMPENSATION`): at night the lava,
  tracers and beacons keep their day colour instead of clipping to white, and
  at noon (EV 14) they are 1.3 times brighter, as the exposure asks. The
  light that local lights cast is not scaled, so against the dark sky the
  lava's glow on its surroundings stands out more than by day. Sideways
  leaves it off and is unchanged. The explosion puffs (`render::fx`) are
  already scaled to the light of the scene and are untouched.
- **Night glows** (`view/lava.rs`, not in Godot). They fade in with the
  floodlights (`Visuals::set_floodlights`: 0 in daylight, 1 below -4 deg of
  sun), so the day look stays the Godot one, pixel for pixel.
  - Over exposed floor lava: up to 4 inverse-square omni lights, one per
    11.2 m tile with at least 4 m² of lava showing, at the centroid of the
    exposed cells, 0.3 + 0.3 r m above the floor (r: the patch's radius),
    12000 cd per m² showing. Exposure is measured per 2.8 m cell from 2 x 2
    columns with nothing over them at all (an intact floor reads covered,
    unlike Godot's ray), and re-measured as chunks break, so breaking the
    floor brightens and adds glows.
  - Past the arena's edge: 8 lights (falloff 1, 100000 lux at 1 m) around
    the ring halfway from the edge to the stands; they light the crowd, the
    stands and the outside of the arena. Above the floor in open arenas, so
    its rim catches them; below it in walled ones, so they do not shine
    through the walls onto the floor.
  - The colour is Godot's lava light colour, as the pool and leak glows.
- **Light budget.** Growing Guns now uses all 32 local lights the renderer
  allows (was 16): ion strikes and blasts up to 14, then the 4 floodlights,
  the lava (pool, leak, 4 + 8 night glows: at most 14), the roof beacons and
  muzzle flashes in what is left.

`scripts/lava-night.json` shows the night views: the arena, the stands
over the pool, a floor hole with a player at its rim, an open arena's edge
and the leak.

### Edges against stone

Both draw the lava as plain geometry meeting the stone, with no blending
and no glow fringe. Godot's slight halo comes from its global glow (above).
The leak fronts sit 4.5 cm proud of the floor in both.

## Gameplay

All in the sim; deterministic, fixed 120 Hz.

| | Godot | Rust |
|---|---|---|
| Pool kill | below -3 (capsule centre): `kill_environmental("lava_fall")`, lethal past the phoenix | same (`player.rs::lava_tick`, `kill_environmental`) |
| Pool burn | 15 damage per 0.5 s while within 1.2 m above the pool; the accumulator resets on leaving | same |
| Leak kill | below the leak surface (0.09 + rise), anywhere in the arena | same (`flow.rs::lava_leak_tick`) |
| Leak burn | 15 per 0.5 s within 2.4 m above the surface, inside the flooded edge band (once the floor phase is active) or within 1.25 m of a wall under its lava sheet (vertical band ±0.8 m) | same |
| Accumulators | separate dictionaries for pool and leak | one per player, shared. Same result, because a player in the pool band is below the leak surface and dies to the leak first |
| Destruction | breaking the floor drops players into the pool; the pool light brightens as columns open | same (the light via `ArenaView::break_chunks`) |
| Island kill | below the lava floor (0.04, capsule centre), lethal past the phoenix; the pool's kill and burn are off | same (`player.rs::lava_tick`) |
| Island burn | 15 per 0.5 s up to 1.15 m above the lava floor, outside the islands' safe disks (radius half side + 0.25, so a square column's corners burn) | same |
| Island leak | `_lava_floor_already_full`: no wall sheets, the floor phase is complete at once, so it rises 1.5 m/s from the start to 18 m; kills below 0.09 + rise, burns 1.15 m above that, anywhere | same (`LavaLeak::full`) |

### Death

Godot's `Violence.play_lava_death` (tuned in `lava_death_lab.tscn`) does not
ragdoll and spills no blood. The body takes the lava material and sinks with
a sine ease-in:

- **Fall** (killed by falling in): 0.75 m below the surface over 2.15 s.
  First comes a splash pillar, a cylinder 2.2 m tall, radius 0.55 at the
  bottom and 0.23 at the top, rising for 0.18 s and sinking for 0.55 s.
- **Burn** (killed by lava damage): 0.5 m below over 1.35 s.

Before this work, the Rust port ragdolled bodies that fell in, with blood.
Bodies burnt to death left no corpse, only a blood burst.

The port now matches Godot:

- **Sim.** `Gib::Lava { fall }` on the kill event, set through `Blow::lava`.
  It does not change the world state or the versus digest: bots-only
  versus runs of 180 s for seeds 1, 7, 42, 99 and 4242 (4 to 38 lava events
  each) give the same digests and scores before and after.
- **View.** `view/players.rs` (`lava_death`, `pose_lava`) draws the sinking
  body and the splash.
  - **Skinned characters.** Each gets a lava twin: the same skeleton and
    skinning drawn in the lava material (`assets::Character::add_actor_with_material`),
    frozen in the death pose. Twins use the renderer's spare skinning actors
    (8 in all, players first).
  - **Fallback.** Primitive bodies, or players beyond the spare actors, get
    lava-material blobs.
  - **Revive.** A revived player loses the lava body.
- **Quirk kept.** Godot takes the surface from the generator's
  `get_lava_surface_world_y`, which is always the pool at -3 on non-island
  arenas (0.04 on island arenas, even under a risen leak), even for leak
  deaths. A body burnt in the leak therefore sinks
  through the floor to -3.5, and a leak fall's splash rises from the hidden
  pool. The port does the same.
- **Small difference.** During a fall's first 0.18 s, Godot still shows the
  body in its own materials. The port shows the lava twin from the start.
  The body is already below the surface by then.

### Sound

The lava sizzle (`sounds/lava_sizzle.luau`, played on `Event::Lava`) was
already done and is unchanged.

## The island arena

Godot's `_build_lava_island_layout` (not the menu's cinematic fort), ported
in `sim/src/arena.rs`. Run `scripts/island.json` to see it.

### Choice and determinism

Godot draws the size, then one `variant_roll`: below 0.18 (or with
`force_all_floor_lava`) the island branch, which builds walls and corner
towers, the pool, the lava floor and the islands, then returns early with no
floor, buildings or cover. Nothing else is drawn at the decision point, so
the other variants draw the same stream whether islands exist or not. The
port's `Arena::generate_with` keeps that: walled, hole and open arenas are
unchanged, generator draw for draw. Before, island rolls fell through to the
walled layout.

The port's random stream (PCG32 per arena seed) is not Godot's, so a seed
gives a different layout than in Godot; the draws follow Godot's order.

**Versus digests** (`growing-guns sim --seed S --seconds 180 --arenas`,
bots only). A run stays identical tick for tick until its first island
round, then diverges: an island arena has different spawns, so the round's
spawn shuffle draws differently from the match stream.

| Seed | First island round | Before | After |
|---|---|---|---|
| 1 | tick 1759 | 267adf3df3c7468d | c0feffd45eb915c2 |
| 7 | tick 3124 | bdd8b64915fac337 | eca9be71db8f5dbf |
| 42 | the first round | 6c2d3b8de92cceb8 | bfbef284bd0166f8 |
| 99 | tick 6050 | a27eee66c0f1ad0c | 1cfa1f69eb77bf7e |
| 4242 | tick 8644 | d92d8ade5a4df2bb | 076d2a42585ba8a0 |

The digests at 29, 52, 100 and 144 s (just before those rounds) match the
old build for seeds 1, 7, 99 and 4242. Seeds 8, 10, 11, 20 and 25 roll no
island in 180 s and keep their digests. The golden images use the default
script seed, whose arenas are not islands, and are unchanged pixel for pixel.
Arena seeds 4, 5, 10 and 11 now roll islands, so the tests and scripts that
used them as walled arenas moved to 15 and 0 (`destruct.rs`), 52
(`lava.json`, `lava-night.json`; the Godot comparison above was measured on
11, also walled and 76 m) and 85 (`wall.json`, `blast.json`, `boom.json`).

### Layout

| | Godot and the port |
|---|---|
| Centre | an island of half side 4.2 m, top at 1.0 |
| Spine | 3 to 5 islands towards -X, half side 3 to 4.7, each 1.25 to 2.05 m above the last, gap 2.7 m to `max_gap` |
| Grown pairs | round(size / 6) + 1..4, clamped to 10..18, each grown off a random island of the -X half: half side 2.2 to 5.2, height step -1.55 to 2.05 (58%: 0.6 to 2.05), gap 2.8 m to `max_gap`, 80 tries |
| Stepping stones | half side 3, top 1.15, on the X axis at ±10.5 m, linked to the centre |
| Reach | `max_gap` goes from 5.6 m (level) to 3.1 m (a 2.05 m climb); tops between 0.9 and 9 m, inside half the arena minus 3 m |
| Islands | square destructible columns (2 x half side), from 0.45 to 1.5 m below the lava up to their top, `_mat_dark` (`Kind::Island`); everything but the centre mirrored at (-x, -z) |
| Planks | over 22% of the links at least 7 m long, length - 4.8 m by 0.22 by 1 m, 8 cm above the lower top. Godot turns them by atan2(dz, dx) as rotation.y, which mirrors them in Z unless the link runs along X; kept |
| Cover | on 38% of the islands with half side 3 or more: 1.8 to 2.8 by 1.1 by 0.8 to 1.2 m at a random angle. Planks and cover are not mirrored |
| Spawns | one 1 m over each island top, pruned to `is_lava_spawn_safe` (at least 1.69 m up, within the safe disk minus 0.65 m) |

`island_arenas_are_connected_and_spawn_safely` (arena.rs) ports Godot's
`_validate_lava_platform_graph`: every link within `max_gap` and the step,
every island reachable from the centre, for every island seed below 200 (41
of 200 arenas, Godot 18%; 21 to 43 islands each).

### The rest of the game on islands

- **Spawns.** The round's greedy spaced pick over the island spawns (Godot
  also searches a ring on a platform and caps the spacing on small ones;
  with 21 to 43 islands the pick already spaces players). Sky drops
  (phoenix, revive) reject ground off a safe disk.
- **Strikes.** The sky ray finds islands (there is no floor), as in Godot.
  The ion column reaches 20 m below the lava floor.
- **Pickups.** A drop that misses the islands falls on for the rest of the
  round and keeps its place under the cap of four, as Godot never frees it.
- **Coop.** Wave enemies emerge on islands: no height cap, on safe disks.
- **Debug.** `arena island [SEED]` (everyone moves onto an island), `play
  --island` or `script --island` for every arena, `sim --island --arenas`.

### Bots

Godot's `_bot_on_lava_map` handling is ported (`sim/src/bot.rs`): the move
direction is replaced by a safe one (`_bot_lava_safe_move_dir`: ahead,
towards the centre, across it, then back, forward, left, right) unless
0.9, 1.6 and 3 m ahead are on a safe disk with ground under them; edge probes
reach 3 m; gap landings, ledge tops and pickups must be safe; random hops are
rarer (0.0015 a tick) and only towards safe ground; dashes only when safe; no
teleport special. Once a second a bot rolls a 7% lapse of 0.6 s in which the
checks are skipped. The port rolls it on island arenas only (Godot rolls on
every map), so other arenas keep their bot streams.

As written, Godot's careful bots cannot cross: the safe direction is chosen
before the gap check, so a careful bot never gap-jumps, and the same check
runs in the air and turns a jumping bot back over the gap. The port adds two
things. A careful bot keeps heading for its target when a gap lies ahead
with a safe landing it can reach (a rise of at most 2 m; past 7 m only with
a dash ready, as Godot's gap jump dashes there). In the air it then steers
for that landing, brakes over it and double-jumps only if the fall would land
short. Godot's headless players could not be run here to compare (their FBX
characters are not imported), so this rests on reading `player.gd`.

`bots_keep_to_the_islands_and_jump_between_them` (tests.rs), four bots,
four seeds, 120 s each: 36 kills, 160 moves between islands, 39 falls, all
but 3 within 1.5 s of a lapse or a cage drop. With Godot's logic alone:
8 kills, 8 moves and 45 falls (most of them aborted jumps).

### Visuals

- **Lava floor.** The lava material (Godot noise, `LAVA_TINT` x
  `LAVA_NITS`) on a slab the size of the arena. Godot leaves its floor at
  0.04 while the leak rises invisibly and kills; the port draws the floor at
  the leak's surface once it is higher, so the rising lava shows. No fronts
  or wall sheets, as in Godot.
- **Light.** Godot's LavaGlow: energy 8, range 0.78 x the arena, 0.35 m over
  the lava, attenuation 0.65 (`view/lava.rs`, following the drawn surface).
  No pool or leak light, as Godot sets up no openness and hides the leak
  glow there.
- **At night.** The night glows work unchanged: up to 4 over the largest
  patches of open lava between the islands (now just over the lava floor)
  and the ring past the edge. The floor glow is not scaled by the exposure,
  so the islands catch more orange light than by day.
- **Inferno palette.** Godot's `_inferno_palette`: stone pulled towards
  basalt (floor 0.62, wall 0.68, building 0.64, dark 0.72 of the way), a sky
  from (0.06, 0.05, 0.09) to (0.58, 0.24, 0.10) and a sun of (1, 0.72, 0.38)
  at 2.55 / 2.0 of the usual energy. The sky and sun change only under
  Godot's sky (the default), which takes a renderer rebuild, done only when
  a round switches between an island arena and another; the procedural skies
  (`--sky night` and others) keep their own. Not ported: the fill light,
  ambient colour, exposure 1.14, sky energy 1.55 and glow. The engine's
  linear fog towards the sky gives a salmon haze where Godot's exponential
  orange fog is a saturated orange.

Captures from `scripts/island.json` (960 x 540, software adapter): the
arena from a corner tower, from the centre island and from above the north
wall; the bots after 5 and 10 s; a bot falling into the lava and sinking;
the leak risen 6 m; and the same at night. The Godot references, made with
a scratch capture script under xvfb (Godot seed 3, so a different layout),
look the same in structure; Godot's fog is a deeper orange.

## Still to do

- Island arenas: Godot's fill light, ambient, exposure and glow of the
  inferno palette, and its per-platform spawn ring; the menu's cinematic
  lava fort (`cinematic_present`).
- Colour beyond what AgX allows: only a different tonemapper would close it
  (decided against).
- Fog and glow parity, which are scene-wide rather than lava.
