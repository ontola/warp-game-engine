# Growing Guns on Warp

A port of **[Growing Guns (Godot)](https://github.com/ontola/growing-guns)**
to [Warp](../../engine/README.md). The Godot game is the reference: a fast
last-man-standing arena shooter where the loser of a round picks a card that
grows their gun. Gameplay follows Godot commit `3daf27e` (`player.gd`,
`weapon.gd`, `bullet.gd`, `blast.gd`, `grenade.gd`, `game.gd`,
`round_modifiers.gd`), checked tick for tick in `sim/src/tests.rs`.

This is the version GameNight plays on the ManCave. The status below is
kept current: see [Keeping this README current](#keeping-this-readme-current).

## Status against Godot

Done: as in Godot. Close: present, with the stated difference. Not yet:
missing.

| Area | Status | Notes |
|---|---|---|
| Movement | Done | Jump, double jump, wall jump, step-ups (0.35 m), lava burn and kill, falling off the map |
| The gun | Done | Projectile rounds with drop, spread, bloom, recoil, ricochet, drill, homing, hitscan, poison, slow, lifesteal, explosive splash, impact mines, three hit ellipsoids |
| Gun model | Close | Procedural stat-driven parts; `gun-review` captures upgrade combinations from three angles using gameplay geometry and materials. Full-size chamfered blocks close assembly gaps. Large drums have a smooth 48-sided silhouette and rounded rims. Fixed grip/trigger/magazine clearances survive upgrades; a shaped pistol grip with rubber inserts, a steel trigger and an angular guard replace the intersecting cylinder/ring layout. Slim base receiver/magazine, damage-gated rail and separate green poison/blue chilling reservoirs angled rearward along the receiver; low/minimum profiles use simpler opaque gun assemblies with fewer parts/triangles and half the reserved instances; broader finish/detail refinements remain open |
| Cards | Done | All 49 cards in `mods/base/cards.luau`; Ninja, Dodge and Backblast removed along with dash. Damage, Big Mag and Rapid Fire have 3x common draw weight; rarity, firepower pity slot and trailing-player factor remain |
| Specials (RMB) | Done | Grenade, cluster, teleport, zoom, sword with backstabs, air strike, ion cannon; echo, tactical reload, refresh |
| Rounds and match | Done | Spawn cage, kill order, card picks (the dead pick mid-round with 3+ players), first to ten wins, rematch, extend |
| Round modifiers | Close | All twelve play; see the table below |
| Arenas | Done | Walled, hole, open and lava island (18 %), mirrored layouts; destructible stone chunks with wear, bullet dents and debris; stadium footings extend into lava, with cached orange lava illumination |
| Stadium | Done | Colosseum masonry, 8,363 animated spectators, crowd sound, four live TV screens |
| Bots | Done | Aim, reaction, strafing, jumps, pickups and specials; they keep to lava islands. Versus reserves eight visual seats so adding/removing bots keeps the renderer |
| Characters | Close | Knight, paladin and zombie with textures and clips, GPU skinning, 11-body ragdolls with approximate angular limits, progressive end-range resistance and weak damped neutral-pose springs; bullet hits wake and push the struck limb at the contact point, with blood feedback; model-derived head/body gibs. Cuts use cached model geometry; animated skin-aware dismemberment, aim and strafe layers remain incomplete |
| Effects | Done | Muzzle flash and shot light, bullet streaks, blasts with a short, smoothly fading fireball light (no expanding light ring), experimental baked volumetric smoke, adaptive per-view quality and a gradual tail up to 4.6 seconds on full desktop quality (64 active plumes plus up to 32 fading out; low/minimum retain billows), sparks, chips, scorch and blood, air strike rocket and white-out; ion cannon charges as one depth-clipped luminous volume (including from inside and when looking up/down), with wide bloom and bright ground illumination, with a dense field of bright soft blue dots across the impact disc that appear on live solid ground, lift like glowing sand with independent delays, varied speeds and acceleration, then suddenly become upward streaks from their exact positions at impact and nearly vertical rubble lift/fall |
| Sky and light | Done | Godot's sky by default; procedural time of day and weather, per round or as a cycle; visible sun and GPU lens flare with per-view occlusion; floodlights with shadows at night |
| Gameplay replay | Added | F12 saves a screenshot plus a bounded state replay; F9 review, pause/step/seek, saved frames and headless re-render after code edits. Local/host recording; see below for visual-history limits |
| HUD and menus | Done | Godot's layout, fonts (Anton, Barlow) and timing; mouse, keyboard and pads; see [docs/ui-parity.md](docs/ui-parity.md) |
| Standalone Quest VR | Experimental | Native Android/OpenXR APK uses this same app and Warp renderer, all 49 cards and tracked growing gun. Shared minimum graphics profile removes stadium/crowd/TVs and GPU rubble; simple blocks, primitive characters and vertex lighting keep the full eye resolution. Draws directly into runtime eye attachments; measured near 120 fps at 1440×1584 per eye with short dips before restoring gameplay FX. Bullet tracers use a round, eye-facing glow and a short soft wake without bloom, including minimum graphics. Muzzle flashes use the current tracked barrel position. Minimum lava has animated glowing cracks and dark crust from a small baked texture field. Explosions and ion warning columns now have an in-place minimum path; full-field VR flashes use a projection layer. Wider cube joints improve separation without adding triangles; distant unfiltered edges can still alias. Cage bars retain their small authored mesh on PC and VR; a long chain no longer lets coarse LODs erase them. Audio starts after headset focus so a sleeping headset cannot abort launch. The previous lava/effects APK was installed; its active 64-sample Quest run averaged 107 fps (median 118, lowest one-second sample 46) at full eye resolution and 120 Hz, so stable 120 fps is not yet achieved. Detailed profiling identifies repeated HUD glyph warmup/atlas work and large FX as hitch sources; the earlier lava added about 2.67 ms GPU time in a controlled paired Quest probe; the new single-read colour bake reduced that isolated cost to about 0.23 ms before the latest main integration. [Cost investigation and limits](../../engine/docs/quest-profile-2026-10-31.md). The new label cache, bounded explosion coverage and round tracers build into a signed APK; the integrated version is installed and launched, with an active-headset playtest still required. [Optimization measurements](../../engine/docs/quest-optimization-2026-11-01.md). Low and full profiles remain available on PC. Local bots, stereo, snap turn and original menus; desktop replay recording excludes VR poses. [Build, controls and validation limits](../../engine/docs/quest-native.md). The earlier simplified WebXR demo remains separate. |
| Split screen | Done | 1 to 4 local players, hot-join with X, own audio flat on your side of the screen. Join/leave reuses visual slots and clears unused screen regions |
| Online | Close | Host-authoritative over iroh; local movement prediction and buffered remote snapshots. Tested over loopback (`nettest`), not yet between two machines or under realistic network jitter; transport currently binds IPv4 only |
| Wave survival (coop) | Done | `--coop`: grunts, snipers, grenadiers, fraggers, bombers, revives |
| Combat audio | Added | Explosions louder than weapon reports, with bass reserved for explosive impacts; headroom for stacked weapons, one synth per report with a room send, bounded overlapping reports/bass and size-dependent brown-noise rumbles up to 3.2 seconds, preserving the deep Godot-style aftershock. Own shots bypass loudness culling. Offline spam regression covers uzi ×2 + bazooka ×2; receiver/headset listening remains a playtest check |
| Music | Done | Godot's stems, following the match |
| Engine controls | Done | Always-reserved F10 render/debug panel, F11 fullscreen and F12 feedback capture, ahead of game menus, typing and replay. F3 remains game debug; F9 replay |
| GameNight | Done | Party settings, the next round's modifier, live knobs, each player's own stick sensitivity, the session assistant (MCP), loading and pause |
| Debug card editing | Done | F3: LMB adds; RMB removes one owned copy and updates stats. Start/pause menus include RESTART MATCH. |
| Menu backdrop | Not yet | Godot's cinematic lava fort behind the start menu |
| Ghost mines | Not yet | Dead players planting mines |

### Round modifiers

| Modifier | Status | Notes |
|---|---|---|
| Low gravity | Done | |
| Fog | Close | Plays, but the fog itself is not drawn yet |
| Supply storm | Done | |
| Sudden death | Done | The lava leak from the start |
| Headshots only | Done | |
| Boom boom | Done | |
| Brrrrt | Done | |
| Gun swap | Done | |
| Sniper fight | Done | |
| Air strikes | Done | |
| Ion cannon | Done | |
| Blackout | Done | Sun, sky, floods and lamps out; every player carries a shadowed flashlight |

## Play

```sh
cargo run --release -p growing-guns-app -- play                   # 1 player + 3 bots
cargo run --release -p growing-guns-app -- play --players 4       # split screen, pads
cargo run --release -p growing-guns-app -- play --give explosive  # start with a card (repeatable)
cargo run --release -p growing-guns-app -- play --modifier blackout --rounds 3 --island
cargo run --release -p growing-guns-app -- play --coop --players 2
cargo run --release -p growing-guns-app -- play --host            # prints a ticket
cargo run --release -p growing-guns-app -- play --join TICKET
```

Keyboard and mouse: WASD, mouse, Space jump, left mouse fire,
right mouse special, R reload. Pad: sticks, A/LB jump, RT fire,
LT/B special, X reload. F10 always opens the engine render/debug panel, F11 toggles fullscreen, and F12 saves a screenshot plus recent gameplay; F9 opens its replay. F3 (Back + Start on a pad)
opens the debug overlay: give cards, set specials, drop pickups, pick the
sky, force a modifier, start the lava leak, next round, pause the bots.

For slower PCs, pass `--graphics low` or `--graphics minimum` (also `GG_GRAPHICS`); desktop defaults to `full`.
Both profiles automatically select low-poly guns: plain full-size boxes, a 16-sided drum,
8-sided reservoirs without glass or cage rods, a solid rail and four shroud plates.
The shaped grip, trigger/guard, scope, upgrade colours, firing/reload and owned-loadout
growth remain. Gun instance reservations drop from 320 to 160 per view representation;
materials are shared across shapes. These are geometry/budget reductions, not a measured
FPS guarantee on low-end hardware. The same minimum profile is used by the Quest demo;
gameplay and collisions are unchanged. Minimum lava keeps dark crust and animated
glowing cracks using one filtered read from a baked colour field, without an extra
render pass. Muzzle flashes remain enabled; VR shots use the current tracked
barrel position. Low/minimum cache animated HUD labels as scalable bitmaps.
Explosion visuals merge neighbouring events and cap burst/puff counts under load;
minimum also limits projected smoke/fire coverage, while ion warning beams and
simulation damage remain independent of the decorative budget.

For the native standalone Quest APK, run `python3 engine/tools/build-quest-native.py`.
Add `--install` with an authorized USB-connected headset. See
[Native Quest / OpenXR](../../engine/docs/quest-native.md) for controls and test limits.
The earlier [simplified browser demo](../../engine/docs/quest-vr.md) remains available.

## Gun modelling review

`cargo run --release -p growing-guns-app -- gun-review --out build/gun-review`
renders eleven real card combinations (including short Shotgun/Uzi builds and combined poison/chilling) with fixed neutral lighting, in three-quarter,
side and receiver/trigger-detail views. It writes PNGs and `manifest.json` with
cards, visual stats, dimensions, part counts and authored triangle counts. Use `--only base` for a quick
iteration or `--size 2000x1250` for larger images. Each overview fits the model
individually; detail views share a fixed camera. Meshes and materials are exactly
those used during gameplay; the capture tool applies no geometry or material overrides.
Pass `--graphics low` or `--graphics minimum` to review the lighter models and their
actual lighting path; this also works with `--growth`. Compare detail views for identical
framing across profiles. Triangle counts describe the authored model before renderer LOD.

`gun-review --growth --out build/gun-growth-review` captures five 90-frame clips at
30 fps: damage/rail growth, sniper/scope growth, box magazine to drum/minigun, elemental reservoirs,
and a character-only card with no morph. Use `--only drum-minigun` to isolate one.
Parts match by stable component IDs; changed attachments grow from their mounts,
and a retarget continues from the visible pose. Rendering interpolates growth and
inspection between simulation ticks. Round-start growth uses the previous owned
card loadout as its source, never the borrowed gun from a Gun Swap round.
The target is the new round's equipped weapon. Inspection starts only when the
resulting geometry or material actually differs; early card picks while dead
wait until the next round to animate.

## Replay a moment while editing graphics

During local play, press **F12** to save the screenshot and the preceding
35–40 seconds of gameplay (less near startup or after frequent arena resets).
**F9** opens that saved clip, paused at the screenshot's exact sub-tick moment.
The live local match waits while reviewing; **F9** or **Esc** returns to it.
Online hosts can save, but must open the replay separately; clients currently
save screenshots only. Recording stores game events, not an audio track, menus or private names. Playback rebuilds effects, sampled sounds, crowd reactions and music through the normal audio mixer; their exact waveforms/music position are not recorded. Pausing and seeking silence playback.

- **Space / Enter** or the Play button: play/pause. Playing from the end restarts.
- **Left / Right**: one simulation frame (1/60 s). Click or drag the timeline to seek, including dragging outside its bounds. Rapid drag updates are coalesced until the pointer briefly settles or is released.
- **R / Save frame**: bookmark this frame. **F12** in review also takes a screenshot
  and bookmarks the current moment. Gold ticks mark saved frames; click one to restore its exact sub-tick phase,
  or use its frame number when rendering from the command line.
- **F12 moment** returns to the original screenshot's moment.

After editing rendering code, rebuild and press **F9** again: it also opens the
latest saved replay after a restart. You can open a specific file below. The most recently
saved bookmark opens automatically; the recording's gameplay stays fixed while
the new code draws its particles, lighting, meshes and effects:

```sh
cargo run --release -p growing-guns-app -- replay latest
cargo run --release -p growing-guns-app -- replay build/screenshots/NAME.ggreplay
# Repeat a particular frame without a window (add --ui for the review controls):
cargo run --release -p growing-guns-app -- replay build/screenshots/NAME.ggreplay --frame 210 --alpha 0.5 --out build/frame-210.png
```

Files stay local in `build/screenshots` (`WARP_SCREENSHOTS` overrides this): PNG,
screenshot JSON, compressed `.ggreplay`, and `.marks.json` bookmarks. F12 keeps
its existing clipboard behavior. Recording stores binary state/events and arena
destruction, without rendering/encoding video. Its rolling buffer is capped at
64 MiB; compression and atomic file publication happen on a worker. The toast
confirms completion or reports failure. A save in progress retains shared chunks
until it finishes; only one save may run at a time.

Replay uses recorded authoritative states, not today's simulation or mods. It
rebuilds CPU effects and steps GPU debris in order while seeking. Rendering edits
are supported; incompatible state-schema edits produce a load error. The beginning
of a rolling clip has no effects from before that clip, so use its later moments
for comparisons. Replay has a canonical 60 Hz presentation history and disables
TAA. Playback uses wall-clock time at 1x speed with sub-tick interpolation, independent of display rate; adaptive quality stays enabled while playing/dragging and is fixed for paused frame review. Stadium TV feeds use their static initial texture,
and cycling skies bake synchronously in review. It reproduces gameplay and the saved interpolation phase, not a pixel-identical
original window frame with its variable-rate camera punch, adaptive quality,
menus, live lighting overrides or asynchronous sky transitions. New replays of
the same saved frame should be stable. Seeking back reconstructs effects and can
take a moment; reconstruction has a per-paint time budget and the UI shows progress. Rewinds reuse the existing GPU scene when its reserved layout matches, and bounded in-memory lighting caches avoid rebaking unchanged stadiums. No video/audio file is encoded.

## Change it while it runs

- **GameNight app**: the party settings (rounds to win, modifiers (how
  often), card time, gravity, bullet drop, body shot damage, random pickup
  rate), a "Next
  round" modifier pick, the live knobs, and each player's stick sensitivity
  under My settings. Declared in `app/src/party.rs`.
- **Live knobs** (`mods/base/knobs.luau`): damage, fire rate, move speed,
  bullet speed, explosions, health, as multipliers on what the cards give.
  Console `knobs` / `knob NAME VALUE`, MCP `knobs` / `set_knob`, or the app.
- **Mods**: cards and rules are Luau in [`mods/`](mods/API.md), hot-reloaded
  within a quarter second, run inside the host's sim tick so matches still
  replay. `growing-guns mods-api` regenerates `mods/API.md`.
- **Live control (MCP)**: `play --live [PORT]` or, under GameNight, always on
  a free port for the session assistant: match state, cards, weapons, bots,
  rounds, mods, knobs, screenshots. See `app/src/live.rs`.
- **Console** (`help`): `give`, `special`, `modifier`, `round`, `arena SEED`,
  `arena island`, `lava`, `sky`, `light`, `lua`, `mods`, `knob` and more.
- **Environment**: `GG_HITCH=MS` logs slow steps (12 ms by default under
  GameNight), `GG_FLASHLIGHT`, `GG_BLACKOUT_EV`, `GG_FLOOD_SHADOWS=0`,
  `GG_BOUNCE`, `GG_CHARACTERS=off`, `GG_MODS=DIR`, `GG_SKY`.

## Test and measure

```sh
cargo test -p growing-guns -p growing-guns-app
cargo run -p growing-guns-app -- script games/growing-guns/scripts/smoke.json --out /tmp/gg
cargo run -p growing-guns-app -- shot --out s.png --views 4 --seconds 4   # no window
cargo run -p growing-guns-app -- sim --seconds 120 --trace players.0.health,round
cargo run -p growing-guns-app -- nettest                                  # host + client over loopback
cargo run --release -p growing-guns-app -- bench --views 4 --size 3840x2160
```

`shot`, `script` and `bench` open no window; `KERN_ADAPTER=software` runs them
on a CPU rasteriser. Results and their scope: [docs/measurements.md](docs/measurements.md).

## Layout

- `sim/`: the deterministic 60 Hz game (arena generator, collision, weapons,
  rounds, bots, coop). `World` implements `kern::Sim`; no rendering.
- `app/`: the game on Warp (`app::Game`): view, HUD, sound, music, sessions,
  GameNight party, live control, `shot`, `script`, `bench`.
- `mods/`: the Luau cards, rules and knobs.
- `docs/`: [UI parity](docs/ui-parity.md), [lava parity](docs/lava-parity.md),
  [measurements](docs/measurements.md), playtest notes.

Online peers must run the same protocol version (`warp/growing-guns/2`);
snapshots are positional postcard, so older builds cannot join.

## Keeping this README current

Every commit that adds, finishes or removes something a player would notice
updates the status tables above in the same commit (AGENTS.md says so too).
`cargo test -p growing-guns-app readme` fails when the README misses a round
modifier, a GameNight setting, a live knob or the card count, or stops
linking the Godot repository.

### Baked explosion smoke trial

Full desktop graphics uses a shared 32-frame density-volume animation for
explosion smoke. The initial burst retains its speed; later frames ease into a
4.6-second tail with a thinning-density cutoff that preserves sparse wisps.
Fire, sparks, heat distortion and lights retain their recipe.
Low/minimum profiles (including native Quest) keep the earlier billows.
`GG_BAKED_SMOKE=0` selects the earlier effect for comparison. The animation
is baked offline and does not collide with walls; walls occlude its rendering.

Each desktop view adapts the smoke/fire layer to full, half, quarter or eighth
width and height based on projected smoke coverage, including overlapping
plumes. Nearby explosions and crowded views use fewer pixels and ray samples;
lower tiers trade fine detail for less rendering work. The layer preserves
fire/smoke ordering and uses scene depth when reconstructing opaque edges.
A recovery delay prevents rapid quality switching. Heat distortion and sparks
remain at the view's full resolution. `WARP_SMOKE_QUALITY=full` forces the
full-resolution path for comparison; adaptive quality is the default.

Smoke has a budget of 64 active volumes plus up to 32 retiring volumes. When
the active budget fills, the oldest plume fades over 0.30 seconds while its
original motion continues. A hard limit of 96 volumes bounds extreme bursts;
the normal 4.6-second lifetime remains available below the active budget.

See [the engine note](../../engine/docs/baked-explosion-smoke.md) for limits
and validation, and [the original asset recipe](../../engine/assets/fx/README.md).
The [earlier scalability measurements](../../engine/docs/baked-smoke-scaling.md)
predate adaptive resolution and this larger smoke budget; they describe the
previous fixed-resolution implementation.

A headless review of a real queued gameplay explosion (no window):

```powershell
cargo build --locked -p growing-guns-app
games/growing-guns/tools/capture-explosion-smoke.ps1 -Out build/smoke-baked
games/growing-guns/tools/capture-explosion-smoke.ps1 -Variant legacy -Out build/smoke-legacy
games/growing-guns/tools/capture-explosion-smoke.ps1 -Views 4 -Orbit -Out build/smoke-four
```

Use `-Executable PATH` for a separate Cargo target directory. The helper
writes 156 frames at 30 fps, the exact scenario JSON and (if FFmpeg is installed)
`explosion.mp4`. The shorter `scripts/explosion-smoke.json` captures key ages.
These are development review captures, not store gameplay previews.

### Shooting ragdolls

Corpses react to collision-checked bullet paths, including delayed projectile
flight and instant sniper rays. Hits apply a bounded contact impulse to the
nearest corpse limb, waking resting bodies and generating a small blood burst.
The primitive corpse fallback also supports hits. Corpse hits are cosmetic:
bullets pass through, corpses do not become cover or award damage/kills, and
severed gibs are not additional shot targets. World and live-player hits clip
the path before the corpse query. Explosions retain their existing physics push.

Headless fixture: `growing-guns script games/growing-guns/scripts/shoot-ragdoll.json
--out build/shoot-ragdoll --players 1 --seed 52 --sky noon --no-music`.
The debug state exposes `corpse_hits` (reset with the arena).

A new append-only `BulletPath` presentation event carries the checked segment
and shot damage in snapshots/replays. Existing recordings load but cannot gain
corpse bullet reactions they never recorded. Older binaries cannot decode the
new event; online participants should use this same build. Corpse physics stays
local presentation, not network-authoritative collision. No mixed-version or
real-network/hardware validation is claimed.


Validation: 14 physics, 54 simulation and 101 app tests passed (2 app tests
ignored). The final 324-tick shooting fixture records two corpse hits and a
changed resting position, with blood feedback captured at 960x540. No real
network, hardware-controller, Quest or performance benchmark was run for this
change. Tuning uses 0.8 N s per damage point, capped at 40 N s per contact.

Start and pause menus offer RESTART MATCH (host/offline only), clearing scores
and cards. In the F3 CARDS tab, left-click adds a card and right-click removes
one copy from the selected player, rebuilding stats immediately.

Lava death tint clears after the card pick when playing as a ghost. Large
explosions retain a sustained 38-90 Hz bass body under the bounded mixer.

Rock debris keeps the 4096-piece full-quality cap (128 on low). Fresh
shards are protected for 0.8 seconds. New bursts thin out with occupancy; a
full pool replaces at most 1/64 of its older shards per tick instead of
erasing earlier blasts wholesale.

Under debris pressure, the largest new fragments take priority. Recycling
selects small, old pieces first and never replaces a larger piece with a
smaller incoming splinter.

Smoke takes colored skylight, sun visibility and haze into account. The
colosseum tiers cast sunlight shadows; local atmospheric solar scattering
samples those shadows along the view ray. The visible solar disc shares the
lighting direction and is enabled for the default sky as well as presets.
