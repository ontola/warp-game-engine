## Local race setup - 1 November 2026

Start new race uses the displayed selections. World, Difficulty and Track seed
start on Random in normal local launches; only those fields roll on each new
race. Player count stays fixed. Explicit CLI/host values and deterministic
headless defaults remain supported. Luau owns random-value selection and menu
navigation; Rust supplies startup entropy and constructs the chosen seeded race.
Resume does not reroll, even with Random selections. Initial menu hides Resume.
The seed's recipe name updates before rebuilding; all six retained recipe types
remain present and source recipe names match.

Validated with focused Luau/native tests (fixed settings, combined direction and
Start, same-seed restart, Random and resume), plus the native 1080p
`scenarios/menu-settings.json` flow. Captures in `build/spaceracer-menu-settings`
show random defaults, explicit choices, new race and pause. No physical pad or
Android validation; this does not change the host settings protocol.

## Per-level shadows and sharp boost arcs - 1 November 2026

SpaceRacer opts into a shared 4096-square static sun map. It owns static source
geometry/alpha, including solid roads/buildings, while live slot pools, cars,
skinning and animated rigid parts retain the existing dynamic maps. Dynamic
slots are registered even when empty at construction. Camera/wind/adaptive LOD
changes do not rebuild the cache. A seed/biome replacement or a changed static
source/sun/alpha state does. One 64 MiB depth map is shared across views, plus
caster buffers and initial bake cost. This is direct sun visibility, not GI.

Receiver-plane filtering accounts for projection roundoff at grazing sun and
the measured error of different visible/cached mesh LODs. Detailed near shadows
still transition to coarser distant shadows; far foliage uses its rest pose.
This reduces the disappearance of whole distant shadows, not every LOD change.

Motion blur previously sampled electrical sprites using the road's velocity.
SpaceRacer now opts into `Graphics.sharp_sprites`: world blur precedes sprite,
exhaust and particle composition; bloom follows them. Other games keep their
existing order. The blur target permits COPY_SRC for late sprite composition.

Validation: native 1080p thirteen-frame boost sequence with blur enabled,
`scenarios/boost-lightning-sharp.json`; reproduced faded parallel arc copies in
`build/spaceracer-lightning-before` and sharp arcs in `build/spaceracer-lightning-final`.
A second thirteen-frame sequence in `build/spaceracer-lightning-split` passes
with two independent viewports. The shadow scenario stages seed 76973 / hard / forest at the reported 9097 m and
100/220 m farther on (`build/spaceracer-stage-shadow-reviewed`); it is not a
replay of the user's input trajectory. Coarse hill geometry remains visible in
its self-shadow silhouettes; this change does not retessellate terrain.
`far_sun_visibility --stage` checks source alpha, remote casters, live-owner
exclusion, invalidation and identical four-view results on an exclusive GPU.
Six far-sun, eight static-sun and 24 SpaceRacer app unit tests pass. No physical
controller/display refresh-rate or steady gameplay performance certification.

# F12 split-road fix - 1 November 2026

Reported seed 89502 / hard / forest at 4854 m. Original F12 showed a stair-step
central cutout with missing inner guards. `build/spaceracer-split-road-fixed`
contains native 1080p entry/wide/exit views after exact continuous ribbon and
inner wall/neon construction. Verified against the F12 location. Teleport-only
fixture has a missed-checkpoint banner, not a newly introduced gameplay failure.

# Respawn and paint wear - 1 November 2026

Native 1920x1080 captures: `build/spaceracer-respawn-wear-final` and
`build/spaceracer-respawn-split`. Same portrait pose at energy 100/45/12 shows
progressive chips and soot while keeping the original body colour. Reconstruction
shows the real hull appearing under a bright scan, then returning to paint.
Iteration removed the floating mounted weapon during reconstruction. Dedicated
crash camera samples show departure, travel and arrival; mathematical tests cover
endpoint velocity/position. Smoke is soft, non-additive and bounded to 12 per car.
These are staged native review shots, not controller footage or equal-content FPS.

# Warp, minigun and glide bomb follow-up - 31 October 2026

Source geometry and behaviour restored; see parity-review.md for exact scope.
Single-view diagnostic captures show idle/active minigun, next-frame removal of
tracers, powered glide and always-on intercept guide. Split review isolates the
warp owner's screen distortion from the other viewport; other views see only
the world-space lens. No new physical-controller or performance certification.

# Source restoration review - 31 October 2026

See [parity-review.md](parity-review.md). New captures exercise the native visor,
Open Sans SemiBold/logo/menu, faceted missile, launch and four independent finish angles.
Reviewed solo at 2560x1440 and two-player at 3840x2160. Iteration caught and fixed
an overly wide replay lens in split view, duplicate white-ball missile emission,
menu gradient seams, and energy status overlapping the arc. Tests are scripted;
not a new controller-playtest or an equal-content performance comparison.

# City playtest fixes - 31 October 2026

Actual 4K playtest exposed an existing city crash that the forest host fixture
missed. Luau-authored city lamps used an 8 m source radius, exceeding the renderer
contract of 1 m. Their radius is now 1 m. A regression drives seed 35 for 2,400
ticks and validates every city/effect/jet light at interpolated presentation time.

Menu/HUD geometry, type and clips now scale from 900 vertical logical pixels,
including split-screen and target brackets. Menu rows are larger, muted text has
higher contrast, and text gets a dark outline. Native output resolution is used
independently of adaptive 3D rendering resolution.

Billboard/window reflections now use a balanced near-first BVH of the real
screen planes and facade boxes. This replaces the top-six screen list per 128 m
receiver tile that could cut an image in half. Limits: 1,600 m ray range, 256
node visits, 32 stack entries and analytic facade proxies; not full scene RT.
One texture lookup after the closest hit. Hierarchy coverage/depth test retains
all 1,500 synthetic proxies. Motion blur starts at 90 m/s, reaches full weight
at 320 m/s, uses 1/75 s shutter and a stronger boost contribution. Pause/menus
remain sharp; this is true scene-motion blur, before the outlined HUD.

`scenarios/city-playtest.json`: 676 ticks, menu/pause at 3840x2160 and twelve
consecutive moving boost frames, no errors. Inspected the readable menu, complete
billboard reflections and visible background/traffic blur. Captures are in
`build/spaceracer-city-playtest`. A 30-frame 1920x1080 headless sample (seed 35,
tick 750, one view, RTX 5070 Ti/Vulkan) measured shade 2.43 ms and blur 0.14 ms
(smoothed GPU passes). Readback throughput is not interactive FPS; this is not a
4K/4-player performance certification. Original 4K startup built the renderer in
about 11 seconds; it is not a steady-state hitch measurement.

# Native GameNight integration - 31 October 2026

The installed Windows Preview daemon launched the portable native game through
its real authenticated launch path on a private loopback port. The test owns its
host store and uses synthetic profiles; it does not touch the running party.

- Twelve app and sixteen shared-client tests pass. Avatar composition covers
  skin, hat/head coordinates, invalid payload fallback and live revisions.
- All nine installed-daemon checks pass: launch/Ready after GPU upload, two
  named views, ten settings, invalid-value rejection, atomic settings/Undo,
  live avatar/paint/name updates while paused, overlay and explicit pause
  semantics, swapped-seat Prepare with saved seed/settings, SDK6 new-race
  action and joining up to four named views.
- Captures at 1600x900: `build/spaceracer-gamenight-final/profiles-before.png`,
  `profiles-after.png`, `two-player.png`, `four-player.png`. Inspected both face
  projection and split-view layout. Hats are visible above the face circle;
  profiles are in the hull shading, not separate floating meshes.
- One atlas layer, uploaded only on profile changes. Paint changes update
  relevant hull/debris materials; mip sampling is bounded to prevent tile bleed.
- GameNight keeps a fixed race roster: changing seats in the overlay prepares a
  fresh race on close. The first test incorrectly expected an immediate active
  seat swap; the corrected test verifies the actual host Prepare boundary.
- Registration updates only the local SpaceRacer launch, backs up both JSON
  files, and needs a GameNight restart to load. Public catalog artifacts remain
  the previously published build. No user-owned process was stopped.

Physical pads, rumble, foreground/window transitions and sustained gameplay
performance were not measured. These are headless integration/visual checks,
not full content/visual parity or a certification of the downloadable release.
Run `tools/test-gamenight.py` with an SDK6 daemon and a new output directory to
repeat. See `README.md` for build/registration commands and settings timing.

# Crash effects and motion blur - 31 October 2026

Baseline: 2200c12. Added the remaining effects discussed with the user:
actual ship-fragment destruction and true motion-vector blur. Existing boost
arcs, exhaust heat/flaring and phosphor trails remain in use.

- `rules/effects.luau` owns shutter/speed/boost tuning, explosion light scale,
  fragment count/lifetime, launch impulses and spin. Read at race generation.
- Up to 28 actual hull components per racer retain the owner's paint under
  procedural soot/scratches. Interpolated 120 Hz cosmetic gravity, drag and
  bounce; nearest 64 scenery bounds, terrain and a local ribbon plane are
  approximate contacts. Small fire/smoke plumes; independent expiry after
  respawn. This is not connected-body fracture or detailed hull collision.
- Shared Growing Guns billows now use 32x18 geometry (embers remain 16x9).
  SpaceRacer opts into continuous soot shading and a single visible soft shell,
  removing unsorted back/front triangle seams. Other games retain the banded
  palette. Night explosion light scale is 500 instead of the arena default
  37,000; roads still receive a warm bright specular flash.
- Per-view 12-tap motion-vector blur follows HDR effects and precedes bloom/HUD.
  Fixed shutter with speed/boost strength, depth/velocity rejection, separate
  split-view targets. Pause/crash/recovery and invalid/long history gaps disable
  blur. Custom octopus skin motion is not independently reconstructed; particle
  blur uses underlying scene motion. See `engine/docs/motion-blur.md`.

## Verification

Ten app tests pass, including fragment pause, respawn independence, repeated
crash replacement, expiry across presentation gaps and dissipative bounce.
Seven shared FX tests, two motion-blur tests and renderer layout/timestamp
checks pass. `scenarios/crash-effects.json`: 1,724 ticks, 18 advancing boost
captures, crash/fire/debris, pause, respawn and zero debris after expiry; all
expectations pass, including a repeat on the rebuilt portable executable.
The initial expiry expectation ran before a draw (cosmetic state only advances
at presentation); adding the final capture verifies actual cleanup, alongside
the isolated unit test. Four-view boost/crash fixture: 803 ticks, all checks pass.

Reviewed identical seeded motion frames with blur on/off: background and passing
craft blur while the followed craft and HUD remain readable. Reviewed smoke
and debris after reducing overexposure and removing polygon seams. These are
headless views, not a physical controller playtest or full visual parity claim.

Artifacts: `build/spaceracer-effects/portable/` (final packaged build),
`smooth/`, `no-blur/`, `four/` and JSON timing reports. Windows portable rebuilt;
no Android build or interactive window launched.

## Measurements

RTX 5070 Ti, Vulkan, NVIDIA 617.14; city seed 31, 750 warmup simulation ticks,
1920x1080 output, native TAA; four views are 960x540 each. No simultaneous GPU
fixture or compiler. Background services were not isolated.

| Run | Moving frames | Headless readback frames/s | Motion vectors ms | Blur ms |
| --- | ---: | ---: | ---: | ---: |
| Four players, on | 120 | 115.42 | 0.0569 | 0.1062 |
| Four players, off | 120 | 85.75 | absent | absent |
| One player, on | 120 | 118.58 | 0.0494 | 0.1106 |
| Four players, off (longer repeat) | 600 | 117.17 | absent | absent |
| Four players, on (longer repeat) | 600 | 107.32 | 0.0491 | 0.1298 |

GPU columns are final exponentially smoothed named-pass samples, summed across
views by the shared profiler, not latency percentiles or whole-frame cost.
Throughput excludes the first two render frames but includes readback; it is
not interactive FPS. The contradictory short-run throughput shows noise;
longer runs still do not establish isolated overhead. No 4K/120 or mobile
performance claim. Initial scene upload and PNG encoding are excluded here.

---

# Reef planting clearance - 31 October 2026

Fix for stalks clipping into canyon walls: plant clearance now derives its
bounds and terrain probes from the rendered kelp, lantern, fan and sponge
geometry, including bent stalks and bulbs. Terrain probes cover triangle
interiors as well as edges. Reject steep roots and buried upper geometry;
allow a small anchoring root band. Tests cover flat ground, an overhanging
lantern facing into/away from a cliff, and steep terrain, plus the existing
five-biome stage/road checks. These two focused tests pass.

Placement and tolerance policy remain Luau (`reefPlantClearance`); Rust owns
the batched geometry query. Probes are cached per generated stage, with no
additional per-frame work. Changes apply when generating the next race.
The user photo's seed/camera is unknown; visual review uses ocean seed 35.
Artifacts: `build/spaceracer-reef-clearance/`. Windows build regenerated;
no Android build or interactive/hardware test.

---

# Biome character and material polish - 31 October 2026

Baseline: 4f4964d. Keep distinctive biome life while improving the native
presentation; literal source-image parity is not the artistic goal.

- Restored four giant octopus landmarks with a breathing mantle, eight
  continuous GPU-skinned arms, animated chromatophore shading, golden eyes and
  procedural sucker rows. Luau selects size, orientation and clear sites.
  Raised swimming placements, lower reef shelves and lighter underwater haze
  expose the animals from the racing camera instead of hiding behind cliffs.
- Preserved fish schools, 112 jellyfish and six mantas; added 320 small rising
  bubble instances. Arms are visual scenery, mantle collision is a coarse box.
  Bubbles use opaque rim shading, not transparent volumetric refraction.
- Clearcoat hulls, separate reflective canopy glass, restrained road roughness,
  brighter city accents/posters, and stronger sun/shade separation in forest.
  Added route-side mossy rocks and fern clusters; smooth terrain vertex normals
  without changing the triangle height field used by collision.
- Animal palettes use interpolated race time and freeze when paused. Existing
  shared GPU skinning handles all four views; no per-frame mesh rebuild.

## Verification and limitations

20 simulation tests and seven application tests pass, including animated bounds,
skin topology/palette validation, pause-time stability and five-biome road
clearance. The stage test was rerun after the final placement change. Native
city, forest and ocean screenshots were inspected. The initial preview sweep
advanced 3,000 ticks without script errors; the reproducible final ocean fixture
is `scenarios/ocean-life.json` (run with `--seed 35 --biome ocean`).

Artifacts in `build/spaceracer-polish/`: `city-final.png`, `forest-final.png`,
`ocean-final.png`, `ocean-four.png` and `ocean-four-bench.json`. The portable
Windows build was regenerated and used for the final four-view measurements.
No Android build, physical controller test or interactive launch was performed.

Ocean seed 35, four views, 750 warmup ticks, 1920x1080 output, native-resolution
TAA, 120 moving frames, RTX 5070 Ti / Vulkan / NVIDIA 617.14:
**77.91 headless readback frames/s**. Reported smoothed weighted-skinning pass
**0.0041 ms**, shade **0.596 ms**. These are pass samples, not summed full-frame
timings or interactive FPS; background load was not isolated. 4K/120 is not
claimed. The animal rig is stylized; full foliage motion, shafts, marine snow,
sandworm, visor and remaining effect fidelity still need work.

---

# Exhaust and aircraft corrections - 31 October 2026

Baseline: 7488c0b. Source `ship.gd`, `racer_design.gd`, plasma/glow/flare/
boost-trail shaders inform the native implementation; retained assets unchanged.
No retired runtime or export tools were run.

- Replaced intersecting static white plume sheets with per-view procedural
  plasma: narrow hot core, coloured transparent envelope and moving shock cells.
- Restored nozzle glows, source-occluded horizontal flares, actual background
  heat refraction, orange reverse jets and short soft exhaust wisps. Wisps move
  backwards relative to the craft faster than its forward speed.
- Throttle drives the nozzle even during countdown; acceleration/boost extend
  the plume. Coloured local lights share that same state. EMP, crash and
  recovery suppress them immediately; releasing throttle fades smoothly.
- Boost adds irregular discharges that change on each advancing presentation
  frame, and 0.38-second world-space phosphor trails. Pause freezes effects;
  crash/recovery/teleport clears trail history. No per-frame mesh generation.
- Restored source YXZ component replacement for wings, rudders and brakes;
  preserves authored sweep/cant and clamps wing travel. Weapon mounts use the
  source universal socket (0, 1.15, -1.8), instead of floating higher/forward.
- `rules/jets.luau` holds effect tuning and anchors, read when generating a race.
  GPU shading, bounded geometry/history and per-frame interpolation stay native.

## Evidence

26 focused sim/app checks pass. `scenarios/jet-effects.json` captures 24 advancing
boost frames, countdown, throttle, brake and pause (601 ticks). It uses a virtual
controller, not physical hardware. The existing 1,207-tick controller/pause/EMP
scenario also passes. The initial test script incorrectly sent analog triggers
to a keyboard seat; it was corrected to a virtual pad before the final run.

Artifacts in `build/spaceracer-jets/`: `city.png`, `forest.png`, `four.png`,
`effects/boost-18.png`, `effects/boost-motion.webp`, `effects/countdown-throttle.png`,
`effects/brake.png` and `controller-flow/`. City/day/four-view images were inspected.
The moving sequence advances two 120 Hz ticks per frame. Pause effect buffers
are identical in the unit check; complete pause screenshots differ by about
0.47/255 mean RGB as temporal reconstruction settles, so pixel identity is not
claimed. Hardware controllers and production GameNight were not tested.

Four-view city, seed 31, 750 warmup ticks, 1920x1080, 120 moving frames,
RTX 5070 Ti / Vulkan / NVIDIA 617.14: **81.11 headless readback frames/s**.
Smoothed reported sprite and exhaust passes were 0.050 and 0.047 ms respectively.
These counters are not full-frame timings and must not be summed across views.
Background load was not isolated; no steady 120 fps or improvement over the
prior run is claimed. Report: `four-bench.json`.

This improves the exhaust and aircraft details; full weapon meshes, visor,
wear/debris/respawn presentation and the listed missing stage features remain.
The source formulas are adapted to Warp, not a claim of identical rendering.

---

# Stage restoration review - 31 October 2026

Baseline: native commit 08a9888, which still replaced original stages with
sparse generic objects. This pass reads the retained MIT source recipes and
assets; no retired runtime or export pipeline is used. Retained asset bytes
are unchanged. New scenery collision/terrain is intentional; committed
handling, flight and track fixture outputs remain unchanged.

## Restored content

- City: original eight building families, six roof families, 120 m block grid,
  mixed lots, tall opening landmarks, aerials, mounted source artwork, office
  texture/shader and separate ground/air traffic. Seed 31 has 699 moving cars.
- Road: bounded analytic reflections of actual nearby screens and facade
  boxes. A 128 m spatial grid selects six screens/twelve boxes, sampled using
  the same artwork/material mapping. This is not complete scene ray tracing.
- Forest: original pine/birch assets at source age ranges (up to 650 m), ferns,
  noise landscape, meadow/stone/shore materials and shoreline-aware water.
- Cell: continuous double helices, knotted proteins, mitochondrial folds,
  layered reticulum, nucleus/pores, 31 animated walkers in seed 31 and the
  source Voronoi enclosing membrane. Fixed overlapping backfaces.
- Desert/ocean: real shared heightfields, source mesa/reef and canyon equations,
  sandstone strata, hoodoos, cacti/arches or kelp/lanterns/fans/sponges.
  Ocean is submerged, with fish schools, jellyfish and manta rays.
- City and cell use an authored directional ambient probe volume independent
  of visible sky brightness. Vehicles and asphalt stay legible at night;
  this is artistic ambient light, not a baked GI claim. Ocean ambient is raised.
- All five retained ship designs display; stage animation uses interpolated
  race time and freezes with pause.

Composition is in `rules/stages.luau`; geometry, terrain sampling, mesh sharing,
clearance and per-frame transforms remain native. The stage file is re-read
when generating a race. Source recipes are adapted to the native track,
spatial queries and rendering, not bit-identical scene reproduction.

## Reproduction and evidence

`tools/build-native.ps1` builds the portable executable. Captures and reports
are in `build/spaceracer-faithful/` (ignored artifacts):

- City: seed 31, 750 ticks, 1600x900, `city-final.png`.
- Forest: seed 35, 700 ticks, 1600x900, `forest-final.png`.
- Cell/desert: seed 31, 750 ticks, 1600x900, `*-final.png`.
- Ocean: seed 35, 1600 ticks, 1600x900, `ocean-final.png`.
- Controller/pause/EMP flow: `scenarios/native-race.json`, `flow/`.
- Four-view city: 1920x1080, 120 moving headless frames, seed 31/tick 750.

23 focused simulation/app tests pass, including stage composition and road
clearance in five worlds, terrain triangle interpolation and ridge sweeps.
Shots warm eight stationary frames; screenshots alone do not validate moving
image quality. Real controllers and the production host are not re-tested.

## Performance and remaining differences

RTX 5070 Ti / Vulkan / NVIDIA 617.14, optimized dev build. Four-view city
measured **158.74 readback frames/s** in the final run over 120 frames at
1920x1080; this is not
interactive FPS or a 4K/120 claim. See `city-four-final-bench.json`. Smoothed GPU
samples include shade 0.689 ms, raster 0.226 ms, cull 0.260 ms; they are not
full-frame percentiles and must not be summed across the split-screen views.
Background CPU load was not isolated; an earlier run measured 78.25 readback
frames/s, so throughput is sensitive to the run conditions. The restored city has 50,152 instances;
it is materially heavier than the sparse prior port.

Full parity remains incomplete: source vista protections, desert phase/slot/
basin specialization and sandworm, giant octopuses, bubbles/snow/light shafts,
full water/foliage/creature animation, visor and weapon/damage presentation.
Tree/organic collisions are bounds proxies. These omissions are documented,
not replaced by a claim that the screenshots now match the original exactly.

---

# Native visual review — 31 October 2026

Baseline: native commit b0abd49. No retired runtime or export workflow used.
All retained source assets are unchanged. Simulation geometry, collision bounds
and random-number sequence are preserved; Luau city/cell lighting values changed.

## Reproduction

Use the packaged executable from `tools/build-native.ps1` with:

- City: `--demo --seed 31 --ticks 750 --players 2 --shot build/city.png`.
- Four views: `--demo --seed 31 --ticks 960 --players 4 --width 1920 --height 1080 --shot build/four.png`.
- Other worlds: `--demo --seed 35 --ticks 700 --biome forest --shot build/forest.png`
  (also cell, desert and ocean).

Local before/after captures are in `build/spaceracer-visual-fix/`.
The before-city capture used the previous executable immediately before rebuilding.
The before-forest/four captures are retained outputs of b0abd49.
Screenshots warm eight stationary frames; they do not establish moving-image quality.

## Corrections

- Six corporate posters were incorrectly treated as an eight-tile atlas.
  Correct tile coordinates, outward orientation and poster proportions.
- Windows no longer stretch to fill the height of each tower. Warm/cool-white
  office groups have metre-sized floors, mullions and occupancy variation.
- Hull surfaces restore authored marking styles without floating decal meshes;
  retained emissive marker strips are rendered again.
- Road material has filtered grain, panel seams, lane/edge markings and varied
  roughness. Fascias make elevated ribbons solid at the edge.
- Local track fixtures and recalibrated jet lighting produce bounded highlights.
  Light selection is capped to four track fixtures per player and 32 total sources.
- Exhaust replaces overlapping glowing motes with a soft tapered mesh. Texture
  padding and tapered geometry prevent repeat-sampler stars at the far end.
- Daytime atmosphere, less haze, thin-foliage material flags, stone silhouettes
  and quieter water normals replace several flat placeholder materials.
- Pickups have open luminous frames instead of solid glowing cubes.

## Measurements and limits

RTX 5070 Ti, Vulkan, NVIDIA 617.14, optimized development build. Seed 31,
four views at 1920x1080, 120 headless frames after 750 simulation ticks.
The final revision measured 195.4 readback frames/s, recorded in `build/spaceracer-visual-fix/bench-final.json`. This includes CPU
submission/readback and is not an interactive FPS claim. GPU pass samples are
smoothed engine counters, not full-frame latency percentiles. Background CPU
load was not isolated. No original-engine performance comparison is claimed.

20 focused simulation/native-app tests and the 1,207-tick controller/pause/EMP
scenario pass. Five worlds and one/two/four-view captures were rendered.
Real controllers and production GameNight were not re-tested in this visual pass.

Full scene reflections (including billboard/window images), city traffic,
complete biome dressing, visor effects and damage/respawn presentation remain
incomplete. The build is still a development port, not full visual parity.

## Flight collisions and combat feedback - 1 November 2026

F12 `spaceracer---warp-1793543480547` identifies seed 46990/hard/cell.
Flight previously compared heights against two independently selected nearest
road sections; this missed finite walls and could invent a crossing at an
intersection or below a jump landing. Swept ribbon triangles now gate landing
and underside contacts, with matching split edges, guardrails and tunnel roofs.
The rendering/collision ribbon subdivision helper is shared. Oriented scenery
boxes remove the empty corners of rotated world AABBs; weapon rays no longer
inherit the aircraft's old 2.5 m inflation. Aircraft/scenery proxies remain
approximate. This is a targeted fix, not a full exact-mesh collision conversion.
The F12 state now includes flight transforms; the old still cannot reconstruct
the exact preceding controller trajectory.

EMP range is 100 m Euclidean distance (including altitude), owner immune, with
local missile cancellation and small fading surface arcs. No fireball for EMP.
Pickup spacing doubles to 1300 m. Sentry cadence/DPS stay 20 Hz/10 DPS, but each
presentation consumes only its newest shot; interpolated endpoints, 2-pixel
minimum tracer width and 0.11 s hit sparks remove subpixel breakup and invisible
centre-of-hull impacts. Boost onset arcs reach up to 23 m, settling to 5 m, with
bounded radial road-contact glows and one extra local light per boosting ship.
Warp world/view refraction rises to 48/64 pixels with radial swirl; the HUD is
composited later and each view lens remains restricted to its owner.

Validation: 37 simulation and 24 app tests pass, including the reported seed's
walls, finite gaps, roof/underside hits, rotated-box empty space, EMP 99/101 m
horizontal/vertical boundaries, shot consumption and spark/arc decay. Retained
flight/landing traces pass; underside crash positions now stop at the contact
rather than up to one tick beyond it. Native 1920x1080 screenshots are in
`build/spaceracer-flight-combat`. These staged captures do not certify controller
feel or frame-time performance.

## Finish racing - 1 November 2026

`scenarios/finish-racing.json`, seed 35 / hard / city, native 1920x1080:
`build/spaceracer-finish-racing/finish-boost.png` shows the rear-quarter boost
shot with mounted sentry, bright exhaust and sharp electrical arcs. Front/side
shots and the all-finished results view retain moving vehicles. The first
boost capture records 434 m/s; simulation tests exercise repeated boosts and
hairpin braking. World blur is enabled in replay; effects remain after blur.
AI driving styles are validated in simulation rather than inferred from stills.

## Grounded boost lightning and distant engine bloom - 1 November 2026

Boost discharges now vary widely in reach and branch count each presentation.
Eight-segment jagged arches rise outward from the nozzles and descend onto actual
supported road points; gaps do not receive fake ground contacts. Each nozzle has
at most two deck strikes and one short free corona branch (48 segments per ship
maximum). The initial large burst decays into smaller arcs. Contact glows are
softened, with the existing bounded local contact light. No bolt history or
extra temporal accumulation: sprites remain after world blur and before bloom.

Boost plumes are roughly twice as wide/bright, with longer irregular exhaust,
larger nozzle emission and stronger local light. A separate screen-facing engine
halo has a 24-pixel minimum (about 35 on boost), including on distant opponents.
Its source depth gates visibility behind scenery; it fades between 650 and
1200 m. This is a cheap optical glow, not a large physical plume at distance.
The settings live in `rules/jets.luau`; geometry/shading remain native presentation.

Validation: eight exhaust/effects tests pass, including frame-to-frame contact
variation, endpoints, raised arches, size/radiance, bounded geometry, pause and
EMP shutoff. Native 1920x1080 `scenarios/boost-arcs-ground.json` captures thirteen
successive 120 Hz ticks plus sustained boost, with four boosting ships staged
at different distances, in city and forest. Reviewed captures are in
`build/spaceracer-arcs-city-final` and `build/spaceracer-arcs-forest-final`; prior
city captures are in `build/spaceracer-arcs-before`. Scenario shader/error checks
pass. This verifies rendered sequences, not physical-display temporal perception
or a new frame-time performance target.

## Compact bloom and road clipping correction - 1 November 2026

Reduced boosted nozzle glow diameter from 3.9 to 2 m and optical halo from
5.5 to 2.7 m, with a narrower/fainter outer falloff. The distant minimum is now
14 pixels (about 20 on boost), retaining a compact distant engine highlight.
Lightning contact diameter drops from 2-5 m to 0.65-1.65 m; contact illumination
is smaller/weaker too. The bright plasma plume and moving arcs remain.

The road cutoff came from fragment depth-testing enlarged optical quads. Engine
nozzle/halo/flare now opt into shared `sprites::DepthTest::Source`: hide when the
light source is hidden, but do not slice the halo where it spreads over the road.
Road-contact sprites and other effects keep normal per-fragment depth.

Validation: eight effects tests; GPU sprite regression checks source/fragment
occlusion on a crossing surface and fully hidden source, in normal and minimal
renderers. Native 1080p city/forest thirteen-frame boost and sustain scenarios
pass; reviewed screenshots in `build/spaceracer-bloom-tight-city` and
`build/spaceracer-bloom-tight-forest`. No physical display or performance claim.

## White-blue nozzle, dim exhaust tail - 1 November 2026

Forward exhaust now uses blue HDR emission (.12/.4/1) instead of mixing ship
paint into its colour. The plume concentrates radiance at its root and decays
exponentially along its length; maximum authored length before surge is 7.2 m
(previously 14.55 m). AgX supplies the bright blue-to-white highlight rolloff;
no global tone-map change. The engine light is blue too. Wisps use .12 rather
than .5 radiance multiplier; the blue history wake uses .065 rather than .38
and lasts .24 rather than .38 seconds. Compact optical halos/road occlusion and
the moving arc shapes remain.

Eight effect tests pass. Native 1080p city/forest thirteen-frame onset plus
sustained-boost scenarios pass; reviewed captures in `build/spaceracer-blue-core-city`
and `build/spaceracer-blue-core-forest`. This is a presentation adjustment, with
no handling changes or new performance claim.

## Shared Growing Guns explosion smoke - 1 November 2026

Checked fetched origin/main (902994a) and the local smoke development checkout.
The latest committed Growing Guns baked density animation/adaptive renderer was
already present in Warp; SpaceRacer had not enabled it. The common Effects pool
now calls set_baked_smoke using `baked_explosion_smoke=true` in effects.luau, so
vehicle crashes and non-EMP weapon blasts use exactly the shared smoke path.
No duplicate asset/shader, no unrelated Growing Guns checkout changes. Fire,
debris, heat, explosion damage and EMP presentation retain their current paths.

This reuses the 6.75 MiB shared density animation, seeded volume orientation,
4.6-second gradual tail, scene-depth clipping and adaptive per-view smoke/fire
resolution. Baked smoke does not flow around collisions; shared renderer limits
and quality tradeoffs remain (see engine/docs/baked-explosion-smoke.md).

Validation: five baked-smoke renderer tests pass (asset/bounds, RNG preservation,
retirement and hard bounds). New scenarios/explosion-smoke.json stages an actual
vehicle crash, captures fire/rise/body/fade/expiry, and passes shader/script checks
at native 1920x1080 in city, forest and two-player split-screen. Captures are in
build/spaceracer-smoke-city, -forest and -split; earlier billows in -before.
Reviewed early/body/tail images. No new frame-time target or physical-pad claim.
