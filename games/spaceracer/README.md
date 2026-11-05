# SpaceRacer on Warp

Playable **native development port**, using Rust/Warp and hot-reloaded Luau.
This is not yet a content-equivalent replacement for the original game or a
valid equal-content performance comparison. Source provenance remains pinned
in `port-status.json`; original assets and their attribution are retained.
There is no Godot runtime or Godot build/export step.

## VR on Quest (5 November 2026, not yet tried on a headset)

`src/xr.rs` runs the same race in VR from the cockpit: the play space rides a
seat on the player's ship (`SEAT`, or `SPACERACER_SEAT=x,y,z` to try another),
the triggers are throttle and brake, the menus show on the head-locked HUD quad
and use the stick and A. Pressing the right thumbstick switches to a level chase view behind the ship (`SPACERACER_VIEW=chase` starts there; `xr-shot --chase`). Motion blur and screen curvature are off. Build with
`python3 engine/tools/build-quest-native.py --game spaceracer --install`.
`cargo run --release -p spaceracer-app -- xr-shot --out stereo.png` renders a
stereo frame from the cockpit on the desktop renderer (not a headset test).
Open: frame rate on Quest (the full renderer, not the minimum one), a level
horizon setting in the menu, a comfort vignette, race HUD on the dashboard.

## Exhaust volume and speed presentation (2 November 2026)

Jet volumes now bend, ripple and vary in length rapidly, independently for each
engine, with their roots fixed to the nozzles. Railgun carries its original
twin-rail model, blue-white beam, expanding rings and endpoint flashes. Its
always-visible aim circle turns green for a clear target; tapping X fires one
50-damage shot. Selection follows the closest target to the circle centre and
the straight shot stops at vehicle hulls, road geometry and scenery. Guide size,
range and the AI aim cone are Luau combat parameters.

Boost light, plume and newly emitted trail strength ease out over 0.55 seconds
(`boost_release` in `rules/jets.luau`). Each trail point retains its emission
strength, so the line tapers continuously after boost. EMP arcs choose fresh
continuous positions across nose, cockpit, wings, nacelles and fins per flash,
instead of using fixed anchors; their existing end-of-effect fade is retained.

Forward exhaust is a bounded 3D gas cone, with a compact white-blue motor core
and a dimmer tapered tail. Countdown shows only the engine lamps; thrust starts
the plume. Boost increases it without covering the vehicle. Sonic vapor uses
soft overlapping billows instead of the angular lattice-noise pattern.
The speed display is calibrated 25% lower; condensation now occurs around
1235 displayed km/h, later into boost, and lasts 0.72 seconds. Actual driving
physics are unchanged. A lower, closer chase camera and stronger acceleration
FOV response are tunable in `rules/effects.luau`.

`scenarios/boost-onset.json` captures countdown, thrust, ignition, crossing and
dissipation. `--profile-no-plumes` omits just the volume plumes for matched GPU
measurements, retaining lamps, lights and other effects.

## Startup, collisions and parity audit (2 November 2026)

EMP vehicles remain solid. Vehicle collisions now use oriented hulls and resolve
packs/rear impacts; boost lightning is a short, aggressive ignition burst.
Crossing the sound barrier briefly produces a hollow, fluctuating vapor cone without
changing boost or covering the centre of the view.
Windows opens without a terminal, shows an icon in the taskbar while loading,
and defaults to fullscreen (`--windowed` opts out). See the dated
[parity audit](parity-review.md#source-audit-and-follow-up---2-november-2026)
for remaining startup-animation, signage, menu and environment gaps and tests.

## Growing Guns smoke (1 November 2026)

Crashes and weapon explosions now use Growing Guns' shared animated volume
smoke, with gradual dissipation and adaptive rendering for overlapping plumes.
`baked_explosion_smoke` in `rules/effects.luau` selects it for a new race.
See [the shared smoke implementation](../../engine/docs/baked-explosion-smoke.md)
and the parity review for checks and limits.

## Native 4K performance (1 November 2026)

City reflections now use a surface-area hierarchy with stackless traversal,
retaining every facade and billboard proxy. Two repeated matched comparisons
reduced the measured GPU interval from about 8.6 to 6.2 ms; the reference 4K
image is pixel-identical. Packaging defaults to release. Native measurements
cover all five biomes with one/two views and two city seeds with four views.
This is not a claim of locked 4K/120 FPS: see
[measurements and remaining costs](../../engine/docs/spaceracer-native4k-2026-11-01.md).

## Boost presentation (1 November 2026)

Uneven, changing lightning arches now strike the road with soft contact light.
Boost exhaust has a short white-blue core and a dimmer blue tail; distant
engines retain a small luminous halo. Tunable in `rules/jets.luau`; consecutive city/daylight captures and effect
checks are described in the parity review.

## Racing personalities and finish laps (1 November 2026)

AI drivers use six seeded styles with different lines, braking margins, pickup
choices and independent boost timing. Finished cars keep racing and boosting,
with corner braking, animated fins and the existing cinematic cameras. Settings
live in `rules/game.luau`; results remain locked. See the parity review for checks.

## Source restoration follow-up (31 October 2026)

See [the parity review](parity-review.md) for restored HUD/visor, cruise guidance
and model, four-angle victory replay, checks and the explicit remaining gaps.
The port remains incomplete. `missile` and `victory` in `rules/game.luau` now
control cruise timing and finish choreography without a native rebuild.

Warp, rotary sentry and powered glide now use the source behaviour and mounted
hardware. `combat` and `warp` in `rules/game.luau` control acquisition/motor values
and the warp speed envelope. See the follow-up in the parity review for limits,
including the approximate unguided bomb preview and native HDR refraction.

Energy-linked paint chips, scratches and scorch now use the retained wear mask.
Critical energy emits soft smoke. Respawn has a smooth camera flight and a bright
bottom-up construction scan (1 November 2026); see the parity review for checks.

Split decks now follow exact smooth gap boundaries with visible inner guards,
matching collision. F12 seed 89502 is covered by a focused capture scenario.

## Flight collision and combat effects (1 November 2026)

Flight now sweeps the finite road ribbons, split guards and tunnel shell instead
of treating nearest-track projections as impacts. Rotated scenery uses oriented
bounds. Scenery and the aircraft hull remain collision proxies, not per-triangle
vehicle contacts. F12 now records flight position, velocity and orientation.

EMP reaches 100 metres in 3D, leaves its owner immune, and disables only nearby
cruise missiles. Hit ships short-circuit with fading blue-white arcs. Pickups are
spaced 1300 metres apart (previously 650); catch-up weighting is unchanged.
Sentry tracers consume at most one new shot per owner/presentation and leave
brief surface sparks. Boost lightning decays from a long onset to smaller arcs
with road-contact glows. Warp refraction is stronger, with per-view isolation.
The EMP radius/pickup spacing are Luau gameplay values; arc/warp tuning lives in
`rules/jets.luau`. `scenarios/flight-combat-effects.json` captures these effects.

## Sharp electrical effects (1 November 2026)

World motion blur now runs before SpaceRacer's sprites. Lightning, tracers and
engine emission retain bloom without inheriting the road's motion vector (which
previously made many faded copies of each bolt). Temporal AA was already before
these effects. `scenarios/boost-lightning-sharp.json` captures thirteen successive
120 Hz boost ticks at native 1080p with blur enabled.

## Level shadow cache (1 November 2026)

Every generated level now opts into Warp's shared 4096-square static sun map.
Roads, buildings and trees retain distant shadows as the camera moves. Dynamic
slot pools (ships, weapons, traffic, creatures and debris) stay outside the
bake and keep near shadows. Camera quality changes do not invalidate the map.
The depth texture costs 64 MiB once per level, shared by split-screen views;
source buffers and initial baking have additional cost. A new seed/biome
rebuilds it. This is direct directional light, not baked GI or local-jet light.
Nearby detail still uses live maps and distant foliage is in its rest pose.
`KERN_FAR_SUN_SHADOW=0` provides the previous path for comparison.

## Play and capture

From the workspace root:

```sh
cargo run --locked -p spaceracer-app --bin spaceracer -- --players 2 --fullscreen
cargo run --locked -p spaceracer-app --bin spaceracer -- --demo --seed 31 --players 2 --ticks 750 --shot build/spaceracer.png
cargo run --locked -p spaceracer-app --bin spaceracer -- --script games/spaceracer/scenarios/native-race.json --out build/spaceracer-flow
```

Windows packaging (builds only, does not launch or stop another game):

```powershell
./games/spaceracer/tools/build-native.ps1
./build/spaceracer-native/spaceracer.exe --players 2 --fullscreen
```

Packaging defaults to the optimized `release` profile. Use `-Profile dev` for
quick compilation, but use release for performance checks. The headless native
4K profiling matrix and per-frame cost viewer are documented in
[native profiling](../../engine/docs/native-profiling.md).

The packaged executable resolves its adjacent `spaceracer/rules` and
`spaceracer/assets`. Development runs fall back to the source directory;
`--game-dir PATH` overrides both. Seeds are 1–99999. Worlds: `city`, `forest`,
`cell`, `desert`, `ocean`; difficulty: `easy`, `normal`, `hard`.
The existing GameNight catalog/release is deliberately not switched to this
incomplete port.

Controller: RT throttle, LT brake/slide, left X steer/yaw, right X strafe/roll,
either stick Y grip/nose pitch, B boost, X fire, Y reset, LB/RB side bump.
Start pauses. Menu: up/down select, left/right adjust, Start plays/resumes.
A selects an action. Local launches default World, Difficulty and Track seed to
Random. Left/right selects Random or a fixed value. Start new race resolves only
Random fields and uses every fixed selection; Resume keeps the current race.
Player count is never randomized. Explicit CLI parameters remain fixed, and
headless fixtures retain deterministic defaults (`--random-settings` opts in).
The six seed-selected types remain Grand Circuit (mixed), Underpass (tunnels),
Switchback (tight turns/chicanes), Sky Circus (loops/jumps), Velocity (boost
lanes), and Pipeline (half-pipes/tubes). The menu previews the selected seed's
type immediately. Settings regenerate the world only when starting a race.
Controllers take the local seats; keyboard takes seat one when no pad is present.
Keyboard: W/S throttle/brake, A/D steer, arrows up/down trim, Space boost,
R fire, F reset, Q/E bump, Enter start/pause. F10/F11/F12 are engine debug,
fullscreen and feedback capture. Touch input is not ported.

## Luau / native boundary

Edit [`rules/game.luau`](rules/game.luau). No rebuild for:

- `track`: six recipe families, road width, base radius/stretch, elevation and
  corner ranges, loop radii/positions, tunnels, jumps, sections, boost strips,
  exposed edges. Spans are normalized positions around the lap (0–1); physical
  dimensions are metres. `road_width` is base **half-width**, wave amplitude,
  and additional easy-mode half-width.
- `ai` and `plan`: steering/braking/boost decisions, racing line and pace changes;
  deterministic random samples and geometric observations come from Rust.
- `pickup` and `activate`: rank-weighted items and weapon activation policies,
  including airborne-only glide bombs and no warp pickup for the leader.
- `balance`: cooldowns, regeneration, weapon damage/duration, pickup density.
- `menu`: navigation and settings changes.
- `environment`: fallback collision ground and lighting/exposure.
- [`rules/jets.luau`](rules/jets.luau): engine anchors, weapon socket, plume length,
  throttle response, light intensity, heat and trail lifetime. Read on the next race.
- [`rules/stages.luau`](rules/stages.luau): source-based stage composition, building
  families/roofs/posters, traffic lanes, tree age/scale, cell structures, reef
  plants and creature placement. Read when generating the next race.

Rust keeps 120 Hz handling/flight, track tessellation and projection, swept
collision queries, targeting geometry, projectile integration, rendering and
controller/host transport. Race bookkeeping still lives in `sim/src/race.rs`;
it is not claimed to be an all-Luau game. Geometry-specific component formulas
remain native. New gameplay and composition rules default to Luau. Native code
is for renderer/physics integration and bulk numerical kernels, not merely
because a feature is easier to write in Rust. Keep tuning/policy in Luau and
measure before moving scripted logic for performance. For reef planting,
`reefPlantClearance` in `rules/stages.luau` sets slope, root burial and spacing;
Rust tests cached samples of the actual mesh against terrain and road bounds. Luau has a 16 MiB memory limit and execution budget; design
parameters are validated before allocation/generation.

The stage builder uses a separate 96 MiB / 8 million-instruction host; geometry
queries and shared mesh construction remain native. Its source is read on each
new race; it does not regenerate a running track on edit.

Rules poll twice a second. AI, item and menu edits apply live; validated balance
values refresh on reload. Existing timers retain their elapsed state. Track,
scenery and pickup placement edits apply to the **next race**. A syntax error
keeps the previous valid module and logs the failure. A runtime/schema error
pauses simulation with a diagnostic until a valid edit is loaded. A script
edit intentionally changes replay behavior: save the exact rules alongside
any replay/seed used for comparisons.

## Implemented and remaining

The native build connects seeded roads, grounded handling, smooth flight and
landing, physical checkpoints, boost/recharge, side bumps, six racers, automated
recovery, ranks/laps, AI finishing, basic weapon gameplay, mounted items, target
indicators, menus, 1–4 viewports and controller input. Ship control surfaces
animate for humans and AI. City facades/posters and original forest meshes are
retained; exhaust lights illuminate road and ships, with shared Warp explosion
and additive effects. Scenery collision uses swept bounds and ground heights.

This is a **development visual integration**, not full visual parity. Source-based
city blocks, mounted screens, traffic, tall forest trees, shorelines, cell
structures/walkers/membrane, sandstone canyons and underwater reef plants are
now present, including giant animated octopuses, fish schools, jellyfish, mantas
and rising bubble vents. Five retained ship variants are displayed. Sandworm encounters,
full foliage/water animation, remaining visor/weapon optical differences and
original audio still need work. Forest collision uses trunk proxies;
cell/rock proxies are approximate. Vehicle contact is simplified rather than
the original complete rotated hull solver. Weapon tuning and AI behavior need
controller playtesting. No Android or Steam Deck build/hardware claim is made.

GameNight uses the shared native client: prepare/ready/start/pause/resume/dispose,
exact seat tokens, names, host overlay, finished notification and the optional
SDK6 new-race action. Ready follows scene upload/draw. Faces (including hats),
skin and paint colours update live, including while paused. One cached atlas is
projected onto the nacelle paint; there are no floating face meshes. Names and
profiles are matched by player ID. Fixed race rosters decline instant joining;
changing seats/players in the overlay takes effect through the host's next Prepare.

All eleven host settings are declared and validated by `rules/party.luau`. Laps,
difficulty, world, seed and pickup placement wait for the next race. Boost cost,
energy regeneration and crash recovery apply live; duration affects the next
boost. Graphics presets change LOD error, motion blur and nearby shadows, not
resolution. The app's New race action races the settings again: seed 0 and the
Random world pick anew, a fixed seed or world stays. Start opens the race menu
in the game; only Back goes to the lobby.

Register the packaged native build locally on Windows:

```powershell
./games/spaceracer/tools/build-native.ps1
./games/spaceracer/tools/register-local.ps1
```

Registration backs up `local-games.json` and `shelf.json`, updates only SpaceRacer,
and keeps a stable path for subsequent builds. **Restart GameNight once** to reload
its library; the script does not interrupt existing sessions. Published catalog
artifacts are unchanged.

The installed-daemon test uses real authenticated process launch, a headless GPU
and synthetic profiles. It checks two/four viewports, live profile changes,
settings and Undo, overlay pause/resume, seat re-preparation and the SDK6 action:

```powershell
python games/spaceracer/tools/test-gamenight.py --daemon PATH/TO/gamenight-daemon.exe --game build/spaceracer-native/spaceracer.exe --out build/NEW-test-directory
```

The daemon must advertise SDK6 for the action check. The output directory must
not exist; the script creates an isolated host store and only stops its own
processes. `--integration-probe DIR` is an explicit headless test mode, never part
of the normal local launch. Tests passed with the installed Windows Preview host;
physical controllers, vibration and desktop foreground transitions remain manual
checks. This does not certify the older downloadable catalog package.

## Native visual revision

The renderer now uses game-owned WGSL surfaces for opaque hull livery, road
panels/markings and roughness, metre-scaled office windows, water normals,
stone and organic materials. Posters use the correct 3-by-2 atlas and face
outwards without mirrored text. Facades have solid ribs/parapets; elevated
roads have edge thickness and visible local-light fixtures. Original leaf
materials use Warp's thin-foliage lighting. Day worlds use the physical sky;
city lighting remains an artistic night setup configured from Luau. Separate
directional ambient probes keep ships/roads legible without brightening the
night sky; they are authored ambient light, not baked scene GI.

The city billboard/window reflection query uses a balanced hierarchy of actual
screen planes and facade boxes, avoiding receiver-grid cuts. It remains a bounded
analytic approximation, not full scene ray tracing. HUD/menu sizes follow output
resolution and use outlined text; 4K presentation and consecutive boost frames
are captured by `scenarios/city-playtest.json`.

Hull paint and canopy now have separate clearcoat/glass finishes with bounded
city reflections. Road roughness is less patchy, and forest lighting uses a
directional afternoon sun with mossy rocks and fern clusters along the route.
Ocean shelves and lighter haze keep its animals visible from the racing line.
Four giant octopus landmarks use bounded GPU skinning (eight continuous arms,
suckers and breathing mantle); placement stays in Luau. Mantles have coarse
collision proxies; animated arms are scenery. Bubbles are small opaque rim-lit
instances, not physically refractive volumes.

Exhaust uses per-view axis-facing plasma, hot nozzle cores, depth-occluded
anamorphic flares, soft fast wisps and short electrical boost discharges.
Throttle lights the nozzle before the countdown finishes; acceleration and
boost lengthen the plume. Braking opens the source airbrake pivots and fires
orange reverse jets. Heat refracts the actual background; bounded 0.38-second
phosphor trails follow the driven path and clear on crash/recovery/teleport.
Effects freeze with race time and shut down immediately during EMP.

Crashes disintegrate the actual hull into bounded spinning fragments, retaining
paint under procedural soot and scorches. Parts use gravity, damped bounces,
small flame/smoke plumes and independent expiry after respawn. Collision is
cosmetic (nearby scenery bounds, terrain and local road plane), not a full
connected rigid-body destruction solver. Explosion light energy is calibrated
for the night scene, with continuous dark smoke and smoother billow geometry.

Optional per-view motion-vector blur uses a fixed shutter with speed/boost
strength, before bloom and HUD. It excludes stationary pixels and rejects
foreground bleed, invalid history, pause and recovery. Split views own separate
targets. Artistic tuning/fragment launches live in `rules/effects.luau`; native
code handles GPU work and bounded collision. Reload on the next race.
`--no-motion-blur` or live `blur off`/`blur on` permits A/B review;
`crash INDEX` triggers a deterministic visual test.
Their bounded local lights follow the same interpolated engine state and create
real specular highlights on roads and nearby ships. Roads now also trace bounded analytic rays against nearby facade boxes and
actual billboard planes, sampling the retained office/poster artwork. This is
not hardware ray tracing or full-scene SSR: thin ribs, traffic and arbitrary
geometry are not included. Full foliage, HUD and effect fidelity remain outstanding.

Reproducible headless measurement (no window):

```sh
cargo run --release --locked -p spaceracer-app --bin spaceracer -- --demo --seed 31 --players 4 --ticks 750 --warmup 120 --bench 360 --width 3840 --height 2160 --out build/spaceracer-bench.json
```

The report includes adapter, geometry counts, scenario/build settings, raw CPU
and GPU frame samples and p50/p95/p99 costs. Warmup and pixel readback are excluded.
These are separate CPU/GPU timings, not interactive FPS or an equal-content
comparison with another engine. See [native profiling](../../engine/docs/native-profiling.md).

## Verification

```sh
cargo test --locked -p spaceracer --lib
cargo test --locked -p spaceracer-app --bin spaceracer
cargo test --locked -p app --lib
cargo check --locked --workspace --all-targets
```

Retained fixtures check 4,800 ground ticks, 1,800 flight ticks, 700 drafting
and 1,680 bump ticks, 18 full ribbons, 90 centreline cases, 1,800 camera ticks
and five landing cases. These run directly against committed data; the retired
Godot extraction tools are not part of the workflow. All original asset bytes
remain unchanged (`assets/manifest.json`).

New tests also check all five stage component sets, scenery/terrain clearance,
triangle-consistent ground heights and swept flight across a terrain ridge.
New tests cover deterministic full-race replay, three AI race completions,
EMP/recovery, swept obstacle contact, script reload/validation, changing road
width without rebuilding, analog bindings and a real loopback GameNight
WebSocket with sparse seat IDs and pause/resume/dispose. The committed headless
scenario checks two independent controllers, partial throttle, right stick,
paused time, EMP owner exemption and screenshots. This does not prove real
controller feel or production-host compatibility. Captures were reviewed at
1280×720 (one/two views) and 1920×1080 (four views), native rendering. All five
biomes render from the portable package. No steady-frame FPS/4K performance
result is claimed.
