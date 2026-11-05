# r51: saved vegetation, vehicle illumination and bloom

26 October 2026. Implemented, release-built and checked in a real night
bloom on/off/on replay. The first r51 playtest ran and was closed by the user
before the corrected lamps and bloom were validated. Do not describe this
change as an 8 ms result or a new grass representation.

## Saved Sideways baseline

`games/sideways/data/render-baseline.toml` contains the user's applied settings:

| Setting | Value |
| --- | --- |
| Tree size / population | 150% / 45% |
| Crown RGB multipliers | 100% / 115% / 90% |
| Grass size / patch population | 200% / 55% |
| Grass RGB multipliers | 130% / 90% / 130% |
| Grass LOD error / draw range | 4 input pixels / 100% |

Evidence: live run `20261026T161351534Z-c9f58110`, F12 captures
`sideways-1793032144787-15052-0` (15% grass),
`sideways-1793032321796-15052-1` (55% grass), and
`sideways-1793032504917-15052-2` (applied tree settings).
The final tree transaction selected 143,394 of 319,179 source trees. The
earlier 15% grass choice is retained as a named reference, not the active preset.

The preset is game-owned. Warp retains neutral defaults. Startup and new
stages apply trees and matching colliders before constructing renderer
resources. Retaining authored tree transforms prevents compound scaling on
Apply/Reset. Subsequent stages retain the last applied tree edit; pending
edits are not silently applied. Reset returns to the saved preset.
`SIDEWAYS_RENDER_BASELINE=authored` selects the explicit neutral diagnostic
baseline; metadata records the origin and resolved settings.

## Local light implementation

Warp has a separate budget of 16 short-range local lights, alongside the
existing eight shadowed headlamps. The shared uniform is 1,968 bytes. The
extra lights use the material's direct response, finite source radius, smooth
distance falloff and a cone. A conservative union of their finite influence
bounds rejects the entire extra loop on distant surfaces. No new shadow
maps or geometry shadow passes are allocated.

Sideways uses two red rear emitters per human car (tail/brake), one merged
white reverse-pair emitter, and at most one event-driven exhaust flash.
Rear lenses use measured bundled-model surface positions; unknown rear
mounts are omitted. Intensities are artistic tuning, not measured photometry.
After user feedback the red spill is reduced to 2.5 cd / 2.2 m for tail and
32 cd / 3.2 m for full braking, with smooth interpolation and a broad
30/78-degree cone pointing 10 degrees down. Visible lens radiance is tuned
independently. Reverse remains 5 m and exhaust 2 m. Opaque geometry,
grass source/aggregate and tree volume material paths share incident lighting.

Exhaust emission has a bounded 3 mm body-local motion and a small plume-axis
wobble driven by a fixed-tick RPM phase. This animates emitted gas/flame/heat,
not the metal exhaust mesh, car body, camera or physics. The existing two-tick
backfire event controls both visible flame and its local light, including its
decay. Shot and GPU-run paths carry the same emitter state; synthetic lineups
do not receive exhaust effects.

These are unshadowed direct lights, not GI or volumetric scattering. A cone
limits forward spill through the car, but nearby walls or terrain can still
leak light. No cost saving or negligible-cost claim has been measured. Check
DLSS/TAA expiry: the world light is pre-temporal and the flame sprite is
composited later, so their apparent decay needs an actual moving GPU check.

## Validation and remaining GPU acceptance

The corrected release includes brighter lens cores, weaker red spill, and
generic bloom. SHA256:
`a7d6e697d0ea749d5f6f119fc2c7552f5ede0dcbdd30755e11b7a530c7f8317d`.
Build manifest:
`games/sideways/build/dev-runs/20261026T173428410Z-fe6ffcbd/iteration.json`.
Build time was 80.14 seconds; source was dirty at build time. The build-only
manifest's inherited graphics environment is NOT the validated runtime
environment; use the replay manifest below and the final play manifest.

Corrected CPU checks: 182 CLI tests passed, one ignored; two native probe
contract tests passed; three bloom, three renderer settings, eight composed
shader and four headlight tests passed. Actual lens geometry plus original
texture texels are checked: Asti/MV6 cores had landed on dark trim and now
land on bright red lens texels. Emitted RGB stays red; existing AgX makes the
hottest core pale. This is emissive radiance and tone mapping, not lightmapping.

### Bloom replay

Evidence:
`C:/dev/perf/warp-sideways-20261026/r51-local-lights/night-bloom-aba-20261026T173932Z/`.
192 rendered frames, one view, 3840x2160 output / DLSS Quality, seed1/km3,
clear night, tick1 held, wind0.6 at fixed time1.9. JPEG previews are1600x900.
The state at tick1 applies full starting brake: these show brake lights, not
idle tail lights. Capture metadata verifies bloom true/false/true, strength4%,
and the saved vegetation preset. No debug overlay is included in captures.

| Section | Steady samples | Covered GPU mean | Bloom filters mean |
| --- | ---: | ---: | ---: |
| On A | 32 | 16.711 ms | 0.0960 ms |
| Off | 48 | 16.638 ms | omitted |
| On B | 48 | 16.812 ms | 0.0973 ms |

Timing excludes section warmup and startup. GPU timestamps join their actual
frame IDs, not delayed receipt time. Bloom's scope covers its three filters;
the extra combine sample remains charged to tonemap. Full-frame drift is
larger than the small tone-map difference, so do not claim an exact total
cost delta from this short replay. This is a held one-player scene, not four
players, motion validation, or an8.33ms target result. Captures, per-frame
JSONL, execution/hash/environment manifest and summary.json are retained.

All three previews were inspected. Lamp cores are now visibly bright, with
subtle bloom, and the surrounding terrain does not get a general haze. The
1600px JPEG lamp-region on/off mean absolute difference is0.553/255 versus
0.030/255 for on/on; these are lossy preview diagnostics, not HDR energy or
pixel-exact regression assertions.

### Generic bloom implementation

Linear HDR lighting, exposure and AgX already existed; output remains SDR/sRGB,
not HDR-monitor signaling. Bloom is new: a per-view threshold/downsample plus
horizontal and vertical filters, all at quarter width/height. The downsample
thresholds each source pixel before averaging so tiny lamps survive. Filters
are normalized. HDR values are bounded before half-float storage. Separate
view resources and clamped samples prevent split-screen bleed. Threshold2,
soft knee1 in exposed linear light; exposure is applied exactly once.

Bloom runs after reconstruction and effects but before AgX. Off/zero strength
skips filters without resetting temporal history. Default strength4%, adjustable
0-20% in the Lighting submenu. It has its own `bloom` profiler scope (currently
grouped as Other in the coarse category summary), and capture metadata records
the resolved state. Two quarter-resolution RGBA16F images use about7.9MiB at
4K summed over views. Native four-view visual isolation still needs a fixture.

### Earlier build, retained evidence

The first release was built in 81.11 seconds with DLSS support, SHA256
`8a6a30591c3db1e5d8167e6dcf6620fcb51c68048228119893e8b3ee49f11980`.
The real night scenario completed 64 frames at 4K output / DLSS Quality,
one player. Its capture confirms the saved vegetation baseline and red light
on the ground. It holds simulation tick 1: the starting brake is fully held,
so this is a brake-light image, not an idle tail-light image. It predates the
user-requested dimmer spill and brighter lens correction above.

Evidence: `C:/dev/perf/warp-sideways-20261026/r51-local-lights/night-integration/`.
The 31 available post-warmup GPU samples gave covered-scope p95 16.96 ms;
this is one held integration scene, not a local-light cost delta or target
performance claim. The subsequent interactive launch is
`games/sideways/build/dev-runs/20261026T171102917Z-86961546/iteration.json`.

`engine/crates/render/examples/local_lights_probe.rs` adds a reusable
eight-case GPU regression, including 16 lights and last-slot-only output.
The final native fixture passes all eight cases in
`C:/dev/perf/warp-sideways-20261026/r51-local-lights/probe-native-b/report.json`.
Off-before/off-after match exactly, all outside-cone/range test regions stay
unchanged, and slot15-only and all16 lights produce the expected output.
An active front headlight keeps the same raw shadow-depth hash, one shadow
pass, eight selected triangles and96 submitted vertices in every case.
Receiver mean RGB-u8 deltas: tail[6.96,0,0], brake[89.52,1.16,0.84],
reverse[135.08,132.16,125.88], flash[163.44,90.64,33.64]. All eight previews
were inspected. This small native fixture checks opaque receiving geometry;
it does not validate motion, DLSS decay, grass/glass, local occlusion or cost.

The first run in `probe-native` is retained: weak-tail assertion sampled a
receiver too far from the dimmer source, giving only0.64 red code levels.
Only the measurement location moved from z1.1 to z0.7; thresholds remained
unchanged and all eight rendered RGBA hashes match the first run exactly.
The renderer was not changed to pass the test. `validation.json` records
both runs and limits. Probe executable SHA256:
`e3248427afb86f4f30ed9af1a7c5ddee4cadc4d494e63647d476aff34026c580`.

Run serially after the interactive game releases the GPU:

1. Same nighttime camera: tail, brake, reverse, downshift flash, release/off.
   Check terrain, grass and car response; no stale light after the event.
2. Native and DLSS motion; four-player maximum light count; emitter mounts.
3. Fixed night local0 versus local16, full GPU interval and material-shade
   scopes. Headlight shadow counts/depth should remain unchanged.

The full moving native/DLSS and four-player GPU checklist above remains
pending; do not infer it from compilation or a held night shot.

The developer launcher contract tests also pass (process identity, artifact
validation, presentation proof, quoting, debug forwarding and syntax). Latest
interactive launch manifest:
`games/sideways/build/dev-runs/20261026T174416078Z-ced5e454/iteration.json`.
It uses the corrected executable above, saved vegetation, one4K output,
DLSS Quality, wind0.6 and the measured work-reduction switches. This differs
from the build-only inherited environment noted above.

## Grass feedback still open

The grass LOD slider is wired correctly, but has only three source meshes.
In the older four-view r49 capture, 99.88% of roots already used the coarsest
source at 1 px. The new 55%/4 px F12 has all 13,030 roots at LOD2. Turning
the error up cannot choose a nonexistent cheaper fourth level or change the
far atlas. Those are different scenes, not a timing comparison.

The 120-blade recipe widens retained blades 3x relative to the 360-blade
source. Growth200% enlarges their width/height again while basal anchors stay
fixed. A better close-up appearance needs better curved/tapered shared source
shapes, not merely forcing the existing finest cut inside 10 m. All LODs and
far assets must derive from that same source and retain wind correspondence.

Grass currently receives scene shadows but does not cast its own shadows.
Candidates: source-derived local contact/sky occlusion, and a bounded near
grass sun map with independent offscreen-caster selection. Measure these as
separate options. Do not multiply albedo by a dark blob and call it directional
shadowing, or charge the complete grass world to every car shadow map.

The reported terrain-coloured silhouette border remains unproven. Source
Hi-Z bounds include live growth, wind and terrain shear and test full covered
minimum-depth footprints; no definite source-bound defect was found. The far
atlas blends multiple coverage samples but uses one representative depth.
That approximation can lose visible tips behind a ridge. Road clearance also
expands with growth and can expose a legitimate roadside strip.

Reproduce the exact captured settings and camera; independently disable
`KERN_MEADOW_OCCLUSION` and `KERN_MEADOW_SKIP_HIDDEN_SHADE`, then compare raw
source/aggregate coverage across a ridge at growth1/2. Existing
`capture_replay.rs` does not restore live debug grass settings automatically;
do not treat an unconfigured replay as a valid reproduction.
