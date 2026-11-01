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
