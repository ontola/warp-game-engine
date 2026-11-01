# SpaceRacer port review - 31 October 2026

The port is playable, but **not a complete replacement**. This review reads the
retained implementation and exercises the native game; it does not launch the
retired Godot runtime. `port-status.json` retains the pinned source provenance.

## Restored in this change

- **HUD and menu:** Open Sans SemiBold from the original engine's packaged fallback,
  retained logo, compact left menu, explicit aligned values, native-resolution
  text, per-view 800x450 proportions, portrait profiles, minimap, segmented
  energy arc, circular X/Y buttons, active/stored item indication, quiet victory
  HUD and timed standings. The actual offscreen visor projection restores
  curvature, instrument glow, faint ghosting and undercoat. Splitscreen uses
  0.010 curvature versus 0.035 solo; reticles compensate for the projection.
- **Cruise missile:** the old velocity-lerp approximation could cut through
  banked turns and orbit targets. It now samples the track ribbon at cruise,
  ejects from the mount, lofts and accelerates to 500 m/s (1800 km/h), retargets
  the eligible leader until final lock, then attacks directly. It can fire after
  the former leader finishes or without a target; an unguided shot has a fuse.
  A fresh high-G terminal jink can evade it; EMP removes missiles. The original
  faceted body, dark radome, swept wings and canted tail replace the cone/cylinder.
  Launch/flight share that mesh. Bright nozzle, lens streak, local illumination,
  continuous camera-faded smoke, blinking brackets and late white-core laser
  replace the placeholders. Smoke is a native ribbon adaptation, not identical
  to the old shader; smoke does not persist after projectile destruction yet.
- **Finish sequence:** a separate noncompetitive pilot retains mounted hardware,
  animates controls, follows splits and arcs across gaps. It continues after
  results. Four 4.8-second camera shots use a 1.3-second vehicle-relative entry,
  obstacle trace and the original horizontal-lens fit on wide split views.
  Each local player's view changes independently. No speed blur during replay.

Timing, cruise policy and victory choreography live in `rules/game.luau`;
geometry integration, collision queries and rendering remain native.

## Remaining differences (do not call these done)

| Area | Current gap |
|---|---|
| Other weapons | Warp, minigun and guided glide behaviour/presentation restored in the follow-up below. Original railgun guide remains simplified. Unguided bomb preview shows a ballistic path, not an exact scenery-impact prediction. |
| EMP | Engine/HUD disable and missile cancellation work; initial glitch and electrical arcs on each affected hull are missing. |
| Driving information | Original severity-coded corner/no-rail signs, some jump/checkpoint notices, gate art and the flight roll reference are missing. |
| Vehicle damage | Energy-linked retained paint-wear mask, critical-energy smoke and respawn scan/camera restored below. Native optics differ; no continuous mesh deformation. |
| Standalone menu | Visual structure restored; original bot/graphics/lighting pages, credits panel, seed text editing and held D-pad shortcuts remain incomplete. |
| AI | Seeded driving/flight/boost decisions work; original personalities, tactical mistakes and item tactics are not all preserved. |
| Environment | Five distinct native biomes exist. Forest motion/water, underwater shafts/terrain detail, city searchlights/vista protection and sandworm encounters still differ. |
| Lighting/performance | Analytic road reflections and local jets work. This is not full scene ray tracing, original lighting parity or a certified 4K/four-player/120 FPS build. |
| Sound/platforms | Sound remains disabled as requested. Native source audio, touch controls and Android/Deck packaging are not completed by this change. |

## Verification

- 25 simulation tests, including a moving leader after a previous winner finishes,
  banked-route following, changing leader before/after lock, untargeted fuse,
  EMP cancellation and post-results movement with retained hardware.
- 16 application tests, including missile outward normals and camera entry/split
  lens continuity, plus existing jet/debris/stage/profile/host regressions.
- 13 renderer HUD tests, serial GPU fixtures, including bounded viewport optics
  and return to a flat menu. Optics is opt-in; other games keep their existing HUD.
- `source-restoration.json`: 2560x1440 solo HUD, mount/launch, four victory angles
  and pause. `source-restoration-split.json`: 3840x2160 two views, one finisher
  while the other races, and pause. `missile-restoration.json`: diagnostic
  close-camera launch/cruise review. These deliberately stage the finish/weapon
  state, rather than pretending the screenshots are an uninterrupted human race.
- Screenshots under `build/spaceracer-restoration-final`,
  `build/spaceracer-restoration-split`, `build/spaceracer-missile-review`.
  No new physical-controller or real GameNight-host playtest in this pass.
  Readback/capture time is not interactive FPS.

## Weapon restoration follow-up

Read the retained `weapons.gd`, `weapon_vfx.gd`, `dive_bomb.gd` and warp shaders.
The earlier prose weapon guide contains superseded balance values; current
source code is the reference. No retired runtime was launched.

- Warp now uses a dedicated ribbon autopilot: 2.8 seconds, 630 m/s peak,
  smooth entry/exit, bend-aware handback, split lane selection, extension across
  missing deck, traffic/attack phasing, and EMP interruption into real flight.
  Ground-only activation preserves the item if attempted in the air. The source
  coil, filaments and untinted depth-aware refractive lens replace the placeholder.
  The owner also gets radial/chromatic scene distortion; the HUD stays separate.
  Native HDR refraction is an adaptation, not a pixel-identical old postprocess.
- Minigun: attached low-segment housing, motor, rotor, black ammo cassette/feed,
  six spinning barrels, scanning/tracking head, idle lowered barrels, recoil and
  blinking LED. 20 shots/s after 250 ms spinup, 10 damage/s, forward progress/range
  gates and scenery occlusion. Shot serials survive low render rates and are
  consumed once per presented frame, so every split view sees the same flash and
  no tracer persists into the next frame. Damage is independent of render rate.
- Glide bomb: the same automatic lock is used by HUD and launch, 1100 m/55-degree
  acquisition, predictive intercept along the road, 360 m/s launch motor for
  1.25 seconds, gravity/drag, finite 1.15 rad/s steering and forward seeker loss.
  Swept deck/scenery contact and 18 m target proximity fuse prevent tunnelling.
  Source 120 m blast and 145-point centre damage use smooth power falloff;
  launcher immunity lasts until physical separation, then return self-blast works.
  Source payload body, folded/deployed wings and amber launch jet are restored.
  Guided preview is cheap and current-frame; unguided guide deliberately avoids
  full-track collision searches. Sound remains disabled.

Validation: 31 simulation, 17 app and 3 exhaust shader/data tests. Weapon cases
cover safe warp expiry and EMP gaps, minigun DPS/occlusion and one-frame visuals,
steering limits/gravity, moving-target intercept and owner clearance. Scripted
1920x1080 weapon review and 2560x1440 two-view warp captures are under
`build/spaceracer-weapons-review` and `build/spaceracer-weapons-split`. These are
staged diagnostic captures, not a physical-controller playtest or FPS benchmark.

## Respawn and vehicle wear restored - 1 November 2026

The previous port only had fixed scuffs. Hull paint now uses the retained
`paint-wear.png` triplanar mask and source chip/rim/scratch/dirt/scorch formulas,
with damage = 1 - energy/100. Recharging restores the finish with energy.
Below 22 energy, twelve bounded, soft non-additive smoke billboards emerge from
the rear panels; they stop on crash/recovery and freeze with simulation time.
The smoke threshold and reconstruction light intensity are in `rules/jets.luau`.

Respawn now uses the source quintic camera flight, bounded inherited momentum,
raised arc and an exact normal-chase endpoint. Camera and reset share the safe
checkpoint/split-lane destination. The actual hull is revealed from below by a
cyan/white construction scan and lattice with local illumination. A game-owned
visibility shader clips unreconstructed geometry in colour/depth raster passes;
there are no duplicate floating hull meshes. Mounted weapons return after the
existing 0.65-second recovery. Source used 0.7 seconds; the port keeps its existing
recovery duration. Detached wreck pieces remain independent. This is a native
adaptation, not identical optical output; there is no mesh dent/deformation pass.

Validation: 32 simulation + 20 app + 1 renderer shader test passed. Native build
succeeded. `respawn-wear.json` and `respawn-wear-split.json` capture actual native
1920x1080 solo/two-player views, energy 100/45/12, scan stages and crash travel.
Pause state is checked separately. These are staged diagnostic scenes, not a
new controller playtest or a frame-time performance certification.

## F12 split-road correction - 1 November 2026

Feedback `spaceracer---warp-1793530264653` (seed 89502, hard, forest,
distance 4854 m) showed jagged missing central road cells and no visible inner
barrier. The mesh had discarded whole lateral cells by midpoint; collision used
continuous `split_gap`. Decks now interpolate exact branch boundaries at both
ends of each segment, including taper-in/out. Inner walls, guards and one neon
edge strip follow the same boundary as `Pilot::constrain`. Their height tapers
at the split nose. Dead-end branches omit their walls as well as their deck.
Boost chevrons stay on each surviving branch. Launch lips now extend to the
actual first air-gap node, matching `Track::sample` and the retained source.
Track recipes and driving/flight physics are unchanged.

Two targeted geometry regressions passed (supported coverage across seeds
89502/35/80385, exact inner barrier alignment). Native 1920x1080 captures of the
reported seed at 4854/4950/5300 m are in `build/spaceracer-split-road-fixed` using
`scenarios/split-road-89502.json`. Diagnostic teleport deliberately triggers the
missed-checkpoint notice; this is not a controller replay or FPS measurement.

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

## Racing personalities and finish laps - 1 November 2026

The six seeded AI styles now differ in racing line, pace, braking margin,
strafe use, energy reserve and boost cadence. Independent decision clocks run
before boost becomes available, so reaching lap two or a straight does not
trigger the whole field together. Drivers periodically choose whether to chase
an available pickup; line changes blend smoothly. These decisions live in Luau;
Rust retains deterministic RNG, timers and the unchanged movement integrators.

Finish laps now maintain race speed and use staggered boosts, with corner
anticipation, braking, bank and control-surface input. Ending a boost coasts down
instead of braking hard. Existing boosts survive the finish line. Cinematic
camera angles and mounted hardware remain, world motion blur stays enabled,
and results/finish times remain fixed. This remains a noncompetitive ribbon
pilot after finishing, not the normal collision/weapon AI.

Validation: all 41 simulation tests pass, including deterministic but distinct
six-driver control/boost traces, pickup choice variation, three completed races
(seeds 31/33/35, all six finish), finish boost speed above 440 m/s, staggered
activation, corner braking and immutable finish results. The native 1080p
`scenarios/finish-racing.json` capture shows 434 m/s (1562 km/h) one second after
finishing, with boost emission, animated controls and cinematic angles.
Captures: `build/spaceracer-finish-racing`. This is not a hardware-controller
playtest or a claim of tactical/human-like AI parity on every generated seed.

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
