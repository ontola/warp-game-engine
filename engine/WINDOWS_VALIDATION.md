# Windows validation, 2026-10-23

This is a development port, not visual or gameplay parity with Godot. The
reference Godot files beside `engine/` were not edited for this iteration.

## Reproducible graphics baseline

Source baseline: `ad1cfdb`. Windows Rust 1.96.1, NVIDIA GeForce RTX 5070 Ti,
Vulkan, NVIDIA 617.14. Evidence lives in
`C:\dev\perf\warp-sideways-20261023` on the test machine.

The stage/oracle check against a fresh Godot 4.5.2 dump passed, including
layout, road surfaces, river points/levels, sampled terrain and tree placement.
The baseline handling bench passed: 0–100 km/h 3.67 s versus the stored Godot
3.68 s; 100–0 km/h 42.4 m versus 42.7 m; top speed 213.6 km/h in both.
The baseline 45,000-tick simulation passed with hash `49837f7ed8b68d92`.

`baseline-clear1.png`, `grass-clear1.png`, and
`godot-matched-clear1.png` use seed 1, 3 km, start at 300 m, clear weather,
1920×1080. The Godot comparison fixture copies the recorded Kern camera,
displayed body/wheel transforms, paint and sun direction. It retains each
engine's own lighting/material/postprocessing implementation. Its sidecar
records those differences. Godot renders through Xvfb/lavapipe, so its timing
is not a hardware performance comparison. Godot's script reports a resource
still in use on shutdown after saving the image; the image and metadata exist.

Visual review: photographed grass fills the previously bare road verge.
Godot still has denser ground cover and additional shrubs. Kern's forest is
denser/darker, sky filtering differs, and sun/ambient balance and vehicle
materials are not visually equivalent. Neither a frozen frame nor stable
world-cell positions prove acceptable foliage flicker while driving.

## Grass cost and measurement limits

The photographed grass uses stable world-cell placement, actual terrain
triangle heights, conservative road clearance, one shared six-triangle tuft,
a separate clamped anisotropic sampler and a 60,000-instance global limit.
It receives shadows without casting thousands of tiny shadow silhouettes.
This first scatter is for static shots; the interactive path does not yet
stream it as the camera moves.

The grass runner records raw samples, warmup, true per-frame instrumented GPU
totals and failure status. `gpu_ms_total_median` is the median of per-frame
totals. The older `gpu_ms_total` is explicitly retained only as the sum of
rounded pass medians; it is not a median frame time.

Matched native 3840×2160 four-view runs, four warmup plus 31 measured frames:

| Order | Grass | GPU median | GPU p95 |
|---|---|---:|---:|
| A | off | 13.440512 ms | 14.035424 ms |
| B | on | 13.488224 ms | 14.025632 ms |
| B | on | 13.512352 ms | 14.211232 ms |
| A | off | 13.454528 ms | 13.771968 ms |

The pair differences are approximately +0.048 and +0.058 ms in this held
scene. These are instrumented render-pass measurements, not gameplay FPS:
the camera/cars are stationary, shadow rendering is forced every frame, and
wall time includes synchronous image readback. One-view runs drifted from
about 9.2 to 12.5 ms during the batch, so they do not establish a stable
one-view cost. The renderer is not yet within the 8.33 ms / 120 FPS target.

The frozen grass snapshot predates the split-layout correction. Its four
views used a four-physical-pixel gap; the current interactive renderer uses
Godot's eight-physical-pixel gap at 4K, with four 1916×1076 views in one
3840×2160 output, or two stacked 3840×1076 views. Do not silently mix these
benchmarks with later builds.

## Interactive verification

The native input probe `physical-controller-probe2.json` recorded 257 active
samples from the user's 8BitDo Ultimate, exposed by XInput as "Xbox Controller".
Both sticks were observed, RT/LT reached 1.0, steering reached approximately
-1.0 and +1.0, and A/X/Y/Start produced handbrake, recovery, camera and pause
events. This is actual physical input through the same mapping as `play`,
not synthesized input or just device enumeration. The first probe recorded
zero activity because its measurement window had already ended.

The final SDK 6 mock-host test of release r4 passed 32 lifecycle/input checks
and exited naturally with code 0, with no forced cleanup
(`mock-host-r4-extended/acceptance.json`). It covers
canonical controller tokens, actual car movement, stale input, button edges,
pause/resume including stopped GPU submissions, consecutive session prepares,
roster/profile updates, settings, camera directions and disposal.
The installed GameNight Preview daemon separately advertises protocol 1 / SDK 5;
SDK 6-only actions and room logs are explicitly disabled for that host.

Release r4 (`sideways-play-r4.exe`) SHA256:
`a226b5f2f393baa92508638193e8697347201f006ee782c9df7d6a9602612a96`.
All 22 workspace tests passed, including WGSL validation and queued
Prepare/Prepare/Dispose transaction ordering. Managed pause now releases
fullscreen, hides the window and stops GPU submissions while keeping host
polling alive. Focus loss also clears pending recovery/shift edges.

The actual installed daemon test passed all eight checks
(`real-daemon-r5/report.json`). It launched the candidate with its generated
authentication token, delivered a runtime-sampled physical-controller frame
for `ordinal:0` to canonical seat 0 (17 ms old at observation), applied
settings/profile changes, paused/resumed and disposed the game. The candidate
exited naturally with code 0; forced cleanup was not needed. The fixture used
an authenticated headless replacement lobby and isolated data/library/port.
No client injected controller frames. This run observed neutral frames, so
the physical input probe and host-delivery test remain distinct evidence.
The installed daemon SHA256 was
`88823d43cd77e45390409c1295fbfb7be3a924f2aacd3a1a827de3acfc511d59`.

The r4 four-view smoke completed 24 actual GPU frames at native 3840x2160,
advancing 192 simulation ticks at 120 Hz with four substeps. The four view
rectangles were exactly 1916x1076. Capture and state:
`play-r4-native4k-four.png` / `play-r4-native4k-four.json`. This was scripted
driving, not four physical controllers or a gameplay-performance benchmark.
The one-view capture `play-r4-native4k-one.png` completed 48 actual GPU frames
at the same native output size (384 simulation ticks).

The exact r4 executable is staged at `engine/build/sideways.exe`, with its hash
verified after copying. The local Windows GameNight library now has the
additional entry `sideways-kern`, titled "Sideways (Kern preview)". All nine
previous entries were preserved, and the previous `local-games.json` was
backed up before registration (`local-registration-r4.json`). Normal play
starts fullscreen at 4K output when explicitly selected by the user.

Hidden GPU checks exercise the presentation shader against the real surface
format but report zero presented frames. They do not prove visible swapchain
presentation or a complete physical-controller driving session through the
GameNight UI. Release r4 had no streamed grass, TAA, audio, full HUD,
particles or in-game settings menu. The HUD update below adds the instruments.

## Native-resolution HUD update

The next candidate, `sideways-hud-r1.exe`, SHA256
`12e27b9cffdd5404f483c4568dd7f3c54933633355c56953d0aebe3b09b98b4f`,
adds shared Glyphon 0.12 / Cosmic Text 0.19 font rendering, bundled Barlow,
native-output UI geometry and the Godot instrument layout. The one shared
atlas serves every view. Scene presentation now prefers an sRGB surface for
linear alpha blending; screenshots include the composited HUD with correct
BGRA channel order.

All 32 workspace tests passed (`hud-workspace-tests-r2.log`), including WGSL,
font shaping for Latin accents, stable cache identities, view clipping,
time rollover, countdown/recovery/finish/results layouts and GPU query
alignment/validation. No simulation forces or physics rate changed.

Real RTX 5070 Ti GPU runs used native 3840x2160, 48 rendered frames and eight
warmup frames, leaving 40 measured samples. Each run advanced 384 simulation
ticks with scripted driving. Captures `hud-r1-one.png` and `hud-r1-four.png`
were visually checked for readable text and clipping; corresponding
`*-profile.json` files retain every per-frame duration and CPU sample.

| Views | HUD GPU median | HUD GPU p95 | CPU prepare median | CPU layout median |
|---|---:|---:|---:|---:|
| 1 | 0.006000 ms | 0.006400 ms | 0.186350 ms | 0.016350 ms |
| 4 | 0.006256 ms | 0.006752 ms | 0.225600 ms | 0.026950 ms |

GPU timestamps bracket only the HUD geometry/text pass, excluding scene
rendering, presentation blit, CPU shaping/uploads and query resolution.
This satisfies the proposed 0.3 ms HUD draw budget in this short racing sample;
it does not establish a total-frame improvement, 120 FPS gameplay, or costs for
results screens, resize or first-time Unicode fallback. No total on/off ABBA
claim is made. Hidden graphics wall time includes GPU synchronization.

After the checks, the exact candidate was copied to `engine/build/sideways.exe`
and its hash verified. The existing GameNight preview registration points to
this path. At the user's request, a visible one-player 4K fullscreen playtest
was started; `hud-r1-live.log` confirms the first actual presented frame and
the running window title showed live race speed/time. This visible session was
standalone. Earlier real GameNight protocol checks remain release-r4 evidence;
the full daemon suite was not rerun during the user's live playtest.

## Indirect visibility and shadow stability update

Candidate `sideways-lighting-r3.exe`, SHA256
`efedcf3634af2e97019b313eefc8e0bf6a5d3f0b80fbd01039c4a4d533f6f8a0`,
was built from the lighting update and copied unchanged to `engine/build/sideways.exe`.
All 42 workspace tests passed (`lighting-workspace-tests-r4.log`): shader
validation, opaque proxy construction/pose invariance, absolute shadow-grid
stability, split extent hysteresis, stone wall winding and terrain-edge normals
are included. No driving forces, physics rate or GameNight protocol changed.

The production shader was exercised in a deterministic real-car fixture with
identical car/camera poses, open ground versus an actual 18 m terrain ridge,
car/no-car and independent contact/reflection toggles. All 12 image checks
passed in `lighting-r2-fixture/analysis.json`. Debug images separate albedo,
direct, diffuse indirect, environment specular and visibility. Contact shading
reduced indirect light below the car without altering direct sun or albedo;
blocked reflected directions darkened while visible sky reflections remained.
The beauty images were also inspected. This isolates the mechanisms; it is
not a reconstruction of the user's exact route position or photometric truth.

Native 4K fixture on/off/off/on measurements used 8 warmup + 40 measured frames
per run. GPU pass sums, including canopy filtering, increased by 0.307816 ms
for one view (2.973488 -> 3.281304 ms), and 0.321080 ms for four repeated views
(3.063816 -> 3.384896 ms). This measures the two visibility toggles as a package,
not the cost of every change versus the prior build. The synthetic four-view
fixture repeats one car/camera and does not exercise four separated cars.

Separate actual one/four-car route smoke runs completed 48 GPU frames and
384 simulation ticks at native 3840x2160, with HUD enabled and TAA disabled. Four rectangles
were 1916x1076. Captures `lighting-r3-one.png` and `lighting-r3-four.png` were
visually inspected. Hidden graphics-wall medians were 15.89495 / 24.7234 ms
(`lighting-r3-*-profile.json`), including GPU synchronization. They are not
visible gameplay FPS or an ABBA comparison against the previous release.
120 FPS is not established. No full GameNight lifecycle rerun was needed for
these renderer-only changes; prior protocol evidence remains version-specific.

The prior playtest exited through its normal window-close path. The new
candidate was started explicitly, fullscreen at 4K with one player.
`lighting-r3-live.log` confirms the first visible presented frame and a
connected XInput controller; the running title subsequently showed live speed.
GPU tests stopped before the user resumed play.

Limits: car proxies approximate opaque geometry and may miss fine cavities;
reflections use bounded heightfield/canopy visibility and average blocked
radiance, not exact local scene reflection. Distant thin occluders can be
missed. The shared 192 m canopy grid and single sun map still trade detail or
coverage when players separate. Snapshot and CPU-grid tests do not prove that
all moving foliage/shadow flicker is gone. Live vegetation still uses cards;
no experimental Nanite-inspired voxel geometry is enabled in this candidate.

## Rapid visual iteration: car glass and distant forest (r5)

The user explicitly requested quick build/restart rounds rather than another
long GPU acceptance suite. `sideways-forest-r5.exe` has SHA256
`87ce473aa0d6a41ed18b7c09ef02e879314ef6c0872ef5c041d91f16beab8635`.
It includes the preceding r4 change that removes self-occlusion from coarse
car interior proxies and uses source car-body geometry in both visible and
sun-shadow passes. It adds continuous forward alpha for glTF BLEND glass and
decals, excluding those triangles from opaque depth, shadows and proxies.
Transparent triangles sort per camera and blend into linear HDR before tone.

Terrain lighting now uses the smooth authored normal for visibility and
receiver bias, keeping the triangle normal for intersection derivatives.
Terrain occlusion samples use the native mesh diagonal instead of bilinear
patches. Normal transforms handle orthogonal nonuniform instance scales.
These address specific causes of triangle-shaped lighting; the user's exact
screenshot has not been reproduced or declared fixed by an image comparison.

The default distant vista adds bounded rolling terrain and up to 10,000
instances of three original exposed-face voxel pine meshes, using existing
cluster LOD. `SIDEWAYS_DISTANT_FOREST=0` disables it for comparison. Native
terrain, road and driving collision are unchanged; new distant terrain has
no driving collision. The light-tracing heightfield remains at 2 m spacing
and copies valid native samples exactly. Between distant samples it can
approximate the coarser background triangles. This is a simple geometry
prototype, not Unreal Nanite or its streaming/assembly/voxel pipeline.

All 28 render library tests passed, including composed transparency WGSL,
blend metadata/order and the eight sparse voxel-baker tests
(`forest-r5-render-tests.log`). Release compilation passed
(`forest-r5-build.log`). The exact executable was copied to the registered
GameNight preview path and launched in a visible one-player native 4K session.
`forest-r5-live.log` records the connected XInput controller and first
presented frame. No competing GPU benchmark ran during the user's playtest;
this iteration makes no FPS, four-view cost or complete artifact-removal claim.

The separate CPU density-voxel bake remains experimental and inactive in
play. `voxel-crown-preview-r1` contains real fir-crown comparisons: baking
took 149.3 ms; 0.25/0.5/1 m cells produced 4,955/1,258/347 sparse cells with
conserved area. Images show softness and excess apparent coverage, especially
at coarse levels. These volumes are not accepted as a near-tree replacement.

## Near shadows and procedural cliff detail (r6)

`sideways-rocks-r6.exe`, SHA256
`af57c418ba16e889854edf5896a1a0e6b9e4578cd2814916a650c3f1007c50ee`,
adds one retained 2048-square near sun map per active view. Its light-space
half-width is 16 m (1.5625 cm per texel), blending across the outer 20% into
the shared distant map. This improves sampling resolution; it does not prove
all close shadow artifacts are resolved. Both opaque and transparent shading
use the same combination, with separately named near cull/raster timestamps.
`near_shadows=0` disables it for a future isolated cost comparison. Four-view
cost has not been measured for this candidate.

New original fractured cliff meshes follow existing native uphill cuts.
Bounded surface relief is visual, at most 0.46 m; native collision is retained.
The running seed-1 3 km scene reports 12 patches, 147,456 source triangles,
1,125 coarsest-LOD triangles and up to eight levels. The first is at 294 m.
They use the existing CC0 rock texture and cluster renderer. This is detailed
cliff dressing, not a standalone procedural boulder/collider system.

The same runtime log confirms 45,248 existing trees plus 10,000 new background
voxel pines. The three shared pine meshes contain 2,990 source / 211 root
triangles total. The background contains 1,153 tiles, extends to a 768 m
radius and uses 1,855,425 height samples. Scene-build logs are emitted for
initial creation and stage changes, including any disabled or failed feature.

All 57 selected CPU tests passed (26 CLI, 31 render), including native height
preservation, both cliff windings, actual tree LOD reduction, composed WGSL,
near shadow coverage and query capacity (`rocks-r6-cpu-tests.log`). The
release build passed (`rocks-r6-build.log`). The exact binary was launched
visibly at native 4K with a connected controller; `rocks-r6-live.log` records
the first presented frame and `rocks-r6-live-errors.log` contains scene stats.
A read-only capture of the user's live window was viewed. No benchmark ran
alongside play; no 120 FPS or isolated speed improvement is claimed.

User feedback on this version: rock forms/materials look visibly repetitive.
Further shape variation is pending; the near foliage remains the existing
card geometry. F12 developer screenshots were requested after this build.

## Developer capture, source-matched distant trees and varied cliffs (r7)

`sideways-dev-r7.exe`, SHA256
`76b15ade48e82f117ed7067ee0aafe1fb9034a0ccd78859d181336d5afcebcc2`,
was built with 66 passing selected CPU tests (30 CLI, 2 preview-example,
34 render). Evidence is in `dev-r7-cpu-tests.log` and `dev-r7-build.log`.
The exact binary was copied to `engine/build/sideways.exe` and launched
visibly at native 4K with the connected controller. The first actual
presented frame is recorded in `dev-r7-live.log`.

F12 captures the already-rendered scene with that frame's HUD into a
same-format presentation target. It adds no simulation tick or scene render.
Readback/PNG encoding happen only on demand. Windows CF_DIB clipboard data
uses explicit bottom-up BGRA conversion, covered by CPU tests. The actual
F12 key was exercised in the running game: `engine/build/screenshots/`
`sideways-1792784655171-227228-1.png` and its JSON sidecar were created.
The sidecar confirms 3840x2160, HUD enabled, zero extra scene renders/ticks,
and successful image clipboard transfer. The in-game toast also showed
`Copied: Ctrl+V`. Captures do not automatically transmit to chat.

The background default now reuses the same three source tree meshes as the
near forest instead of the generic voxel pines. Those remain available only
with `SIDEWAYS_GENERIC_VOXEL_TREES=1`. Higher warped ridges are visible above
the canopy, while native terrain samples and driving geometry stay unchanged.
The 10,000-background-tree cap still makes the hills too empty, per the
user's current feedback; matching native stem density is the next iteration.

Cliffs use irregular 13.7-18.0 m footprints in this seed, continuous warped
geology, variable bed spacing, broken joints and 3.2-5.8 m texture repeats.
All 12 patches remain present, one using the clearance fallback footprint;
147,456 source triangles reduce to 1,120 root triangles. Close foliage
lighting was retained. The user's F12 capture was inspected and shows the
new skyline/cliff treatment, but no matched quality or FPS comparison is
claimed. No competing GPU benchmark was run during play.

The separate CPU voxel preview r2 fixes a source of oblique over-coverage
with 13 directional projected-area samples and continuous interpolation.
At 60 degrees the fine source-coverage ratio changed from 1.2545 to 0.9699.
Far-footprint ratios across three angles are 0.9476-0.9659 (fine),
0.9763-1.0177 (middle), and 1.0455-1.0901 (coarse). These are CPU preview
measurements in `voxel-crown-preview-r2`, not GPU performance or acceptance
of a runtime Nanite-like voxel renderer. Coarse softness remains visible.

## Live grass and native-density background forest (r8)

`sideways-forest-grass-r8.exe`, SHA256
`c03b9b971d2b9f47827d12ab4f3ce254306c045d3ba55de7bc1f246848a0c9da`,
passed 74 selected CPU tests (37 CLI, 35 render, 2 preview-example), release
compilation and an actual visible native-4K startup with the controller.
Evidence: `forest-grass-r8-cpu-tests.log`, `forest-grass-r8-build.log`, and
`forest-grass-r8-live*.log/jsonl`. The registered GameNight executable path
was updated to this exact binary. No competing GPU benchmark ran.

Seed 1 now has 126,720 background trees instead of 10,000, using the same
source models as the 45,248 native trees. Measured native and background
habitat density both equal 298.7 trees/hectare. Background habitat covers
4,242,064 horizontal square metres, 89.8% of the generated hills; steep
terrain excludes 480,624 square metres. The count matches the calibrated
target without hitting the 200,000-instance cap. It is density per eligible
horizontal area, not a claim that every cliff face has trees.

The first live grass implementation reserves 16,000 slots for one view or
24,000 shared slots for multiple views before the cars. World-cell candidates
retain poses/sizes and native triangle roots; only changed instance ranges
upload. Initial seed 1 populated 3,424 slots. Three focused CPU tests cover
root/road exclusion, retained poses/indices and four-view sharing/caps.
Empty pool slots now return before the cull barrier. The user's playtest
found this version too sparse, with conspicuous pop-in: the 32 m single-view
radius and distance-based whole-plant selection are explicitly not accepted
as the final grass treatment. Denser coverage and true opacity LOD transitions
are the next iteration. No four-view GPU cost or 120 FPS result is claimed.

Crown-only simplification now receives UV/normal attributes and a conservative
error contribution from lost projected geometry area, preserving the source
vertices and the opaque wood policy. Exact DAG-cut CPU checks pass in both
orthographic and perspective cases. However, the matched CPU comparator in
`forest-r8-crown-comparison-tests.log` shows little general change at the
sampled screen sizes: most selected counts are unchanged. Only one sampled
far case (fir_b, 800 m, 2160 output height) selects 543 versus 271 crown
triangles, retaining 0.9824 versus 0.7839 of summed projected geometric area.
This metric is not alpha-union coverage, and these results do not establish
a general fix for sparse distant crowns or a speed improvement.

The user's F12 capture `sideways-1792785290021-227228-2` exposed a separate
terrain foundation bug in round 2: river sections marked dry still carve
deep channels beneath a road. The CPU reproduction records up to 17.8 m of
unwanted carve and a separate 3.68 m discrepancy from local nearest-road
searches. Those fixes are subsequent work and are not present in r8.

## Dense grass opacity and road foundation (r9)

The r9 source removes camera-distance habitat thinning. Fixed world-cell
plants retain their full three-card geometry and size, with a 70,000-slot
single-view pool or 115,000 shared slots for split screen. Static woodland
density increases. A nearest-layer HDR opacity pass fades covered pixels
over 40-60 m (single) or 20-38 m (split); CPU residency extends to 64/42 m.
Pool clipping, including visible clipping, is recorded in live state/F12 data.
This does not yet provide a cheaper distant grass geometry representation.

The transition preserves the frontmost colour at the opaque limit and does
not use screen dithering or shrinking plants. It intentionally omits deeper
partially transparent layers. Glass still tests against the original opaque
depth; optional TAA would likewise reproject that depth. Live TAA remains off.
Persistent RGBA16Float plus Depth32Float targets cost 99,532,800 bytes for
one combined native 4K image, with 33,177,600 bytes of depth copy per frame.
The copy/raster interval and composite interval are separately timestamped.
All pool slots submit transition vertices (18 per slot per view); vertex
rejection does not eliminate that cost. No 120 FPS or performance gain claim.

CPU validation: 80 selected tests passed (39 CLI, 39 renderer, 2 voxel preview)
plus five targeted foundation tests. These include shader validation,
opaque-to-opacity colour continuity, nearest-depth semantics, stable grass
residency and pool budgets. Logs: `forest-grass-r9-cpu-tests.log` and
`foundation-r9-cpu-tests.log` in the dated Windows evidence directory.

The exact round-2 F12 road repro now ignores river segments with wet-mask 0,
matching water placement. Terrain chunk samples use the authoritative road
nearest-index query. Real road/terrain Rapier rays pass the foundation test.
`terrain-foundation-round2-fixed.json` reports zero samples differing by
more than 5 cm (previously 53), maximum numerical height difference under
one micrometre (previously 3.68 m), and maximum river carve 0.946 m in the
sample area (previously 17.799 m). Remaining carving is the valid wet river.
This intentionally fixes terrain geometry at the affected locations without
changing the route RNG, drivetrain or tyre force model. The Godot reference
source is unchanged. The CPU probe is now renderer-independent:

    cargo run -p sideways --example terrain_foundation_probe -- <report.json>

Release artifact `sideways-forest-grass-r9.exe` has SHA256
`ddc2599e0a584a2a734ba0170cd7cf68c80a627f257cd33420bf9e82332f17cb`.
The actual NVIDIA RTX 5070 Ti renderer completed three invisible native-4K
frames, including the new depth-copy and opacity pipelines, and saved
`forest-grass-r9-smoke.png/json`. The screenshot was inspected: grass is
substantially denser; distant hill coverage still visibly thins and is not
accepted as a complete LOD solution. Initial grass has 48,796 resident tufts
with zero pool clipping. This short startup is not a frame-rate benchmark
or proof of motion stability. The prior playtest was stopped before GPU smoke;
the registered executable was then replaced and a visible r9 playtest started.
No controller was connected at this startup; previous physical-controller
validation is not repeated by this run.

The r9 executable subsequently completed three invisible four-view native-4K
frames as well (`forest-grass-r9-four-smoke.*`): 23,081 shared resident grass
tufts, zero pool clipping, no renderer error. This is a pipeline smoke, not a
four-player 120 FPS benchmark.

## First actual GPU foliage volumes (r10 experiment)

`foliage_volume_data.rs` packs the three existing source crowns once, with
13 directional extinction samples and six shading-normal lobes. Per-tree
inverse transforms preserve nonuniform scaling. A compute pass bins tree
bounds per 16x16 tile and integrates overlapping densities front to back,
clipped by existing opaque depth. Only each bound owner's crown material is
removed from the colour raster; real trunks and source shadow casters remain.
The fractional layer never writes false opaque first-hit depth. Tree owners
are static; moving them requires rebuilding the packed data.

This is deliberately limited to 256 trees and an explicit packed detail
level. There is no near/far transition, hierarchical instancing or world
streaming. A one-metre canopy-light cache and six-lobe normal model approximate
source lighting. Trilinear reconstruction is softer than the source cutouts.
The optical model itself is statistical, not an exact reconstruction of
individual leaf visibility. Default play leaves this experiment disabled.

90 CPU/shader tests passed after fixing a reserved WGSL identifier. The
independent review also fixed missing half-cell filter support in ray bounds
and a potential non-progressing small-step loop. Every loop path is bounded;
incomplete rays are magenta and counted as failures. Readback is explicit and
off by default in normal rendering.

Actual RTX 5070 Ti fixtures, no simultaneous game GPU workload:

| Fixture | Voxel integrate GPU ms | Alpha/source area | Soft IoU | Incomplete rays |
| --- | ---: | ---: | ---: | ---: |
| One tree, 512x288, 60 degrees, 0.25 m cells | 0.681 | 0.831 | 0.376 | 0 |
| 64 trees, 960x540, 0 degrees, 0.25 m | 29.055 | 0.931 | 0.650 | 0 |
| 256 trees, 960x540, 250 m / 60 degrees, 0.25 m | 107.818 | invalid | invalid | 1,778 |
| Same 256-tree view, 0.5 m cells | 69.597 | 0.913 | 0.767 | 0 |

These are single first-frame GPU intervals, not averaged gameplay performance.
The reference coverage is the actual binary crown visibility buffer with no
TAA/supersampling; volume alpha comes from raw RGBA16F readback. The invalid
fine 256-tree image includes magenta diagnostics and is not quality evidence.
Coarse timing is a changed representation, not an isolated speed improvement.
Files are under `voxel-gpu-r1-one`, `voxel-gpu-r1-64`, `voxel-gpu-r1-256-far`
and `voxel-gpu-r1-256-far-level1`; each contains images and report.json.

Conclusion: the GPU path works, but it is too soft nearby and too expensive
for the intended forest. Do not enable it by default or claim Nanite foliage
is complete. Next work targets ray-candidate compaction and provably empty
regions before expanding the scene. The optional live patch is fixed around
the initial camera and is explicitly experimental.

The visible remote-viewing demo uses `play --autoplay` with regular source
trees. `sideways-voxel-r10-autoplay.exe` SHA256:
`c836c45b1bf600dee6b5d0e3fd9ecb0075d85ad8a292b9896c7c7dc6edfd26a6`.
This adds the existing race AI to standalone visible play; GameNight controls
remain host-owned. It is not a new physics or driving-force implementation.

## Voxel traversal r2 and incremental grass residency (r11)

The experimental ray integrator compacts actual ray intersections once,
caches up to 16 active volumes and skips only regions proven empty by the
fine-to-coarse occupancy hierarchy. Both candidate/cache overflows retain a
complete scan fallback. Coarse levels accelerate traversal without changing
the explicitly shaded level. `first_level_bounds` preserves the fine grid's
half-cell filter support when extra occupancy levels are packed. Every path
remains bounded and incomplete rays remain explicit failures.

94 CPU/shader checks passed; the separate optical example adds three CPU
checks and eleven actual 128x128 GPU cases. These compare raw RGBA16F against
analytic tent-function/Beer-Lambert integrals, including empty/sparse media,
both ray directions, overlapping colour/order, nonuniform scale, opaque
clipping and disconnected intervals. All passed. Evidence is
`voxel-optics-r2-gpu-1/report.json`; these validate optical arithmetic, not
the artistic match of real tree crowns.

With the visible demo stopped, matched first-frame RTX 5070 Ti integrate
intervals were: one tree 0.681 -> 2.859 ms, 64 trees 29.055 -> 32.242 ms,
256 fine trees 107.818 -> 41.702 ms, and 256 coarse trees 69.597 -> 34.108 ms.
The fine 256-tree result still has 836 incomplete rays and is invalid quality
evidence. Thus this is not a universal performance improvement. Small groups
regress; high private cache cost is a hypothesis awaiting a separate test.
No gameplay FPS or 120 FPS claim follows from these intervals.

`voxel-r1-r2-comparison.json` confirms identical source pixels and cameras in
all four fixtures. Valid raw alpha-area changes are -0.0208%, +0.0009% and
-0.0025%; 8-bit alpha-mask mean absolute differences over nonzero support are
0.00107, 0.00019 and 0.00010. Sampling/cache phase changed, so output is not
bit-identical. Inspected voxel images remain too cloudy, especially nearby.
The source normal map is not yet baked; volume sky visibility still uses +Y.
No global darkening/density multiplier is used to conceal these limitations.

Live grass now keeps per-tile residency bitsets and mutates only border
changes. World membership, three-card poses/sizes, surviving slots and caps
are unchanged; cap recovery retains the previous full reconciliation. An
independent review and full-scan regression passed. The CPU-only 64-move
fixture measured selection totals 1.955 ms (masks) versus 8.201 ms (scan),
with complete update p50 2.278 ms / p95 3.257 ms. This is a synthetic fixture,
not gameplay timing or reduced GPU-upload proof; the contiguous upload span
is unchanged. The demo was restarting during the CPU experiment.

`sideways-voxel-r11-autoplay.exe` SHA256
`50f3d592b04a993ab3660d120ac2ea553731e09248a7c5724f1eae4ee1fc7965`
completed three invisible native-4K frames (`r11-smoke.*`) and then resumed
the visible 4K AI demo. Voxel patches remain off by default. The actual F12
image `engine/build/screenshots/sideways-1792788753189-145256-0.png` records
the preceding r10 automatic drive with source trees, not the voxel fixture.

## Developer workflow and terrain-following far grass (r12b)

The thread now has two explicit goals: improve the engine and apply it in
Sideways. `VolumeViewInputs` centralizes the typed depth/light/canopy resources
and shares one bind group per camera between the two volume renderers.
`render_texture_profiled` measures the exact captured scene frame without a
redundant image readback; normal `render_texture` still performs no CPU readback.
F12 metadata and hidden-capture `.png.metrics.json` contain these measurements,
explicitly excluding HUD, presentation and CPU. This is not continuous FPS
telemetry. Existing readback error handling still needs hardening.

`tools/play-dev-windows.ps1` builds while the previous tracked game keeps
running, stages an immutable executable filename, preserves logs/SHA and only
restarts its own PID after verifying path and creation time. Helper contract
tests passed. A real BuildOnly took 83.30 seconds and left the previous demo
alive (`build/dev-runs/20261023T213248089Z-f8c84fd6`). Explicit visible launch
and restart also passed, using runs `20261023T214434940Z-d90fdae3` and
`20261023T214614975Z-67c58bce`. The latter cached build took 1.99 seconds,
then 10.76 seconds to first visible frame; this is not a changed-source build
speed claim. The old process exited with `window_closed`, no error. A new
first-visible-frame marker handles recovery when the first acquire fails.

`SIDEWAYS_FAR_GRASS=1` enables four-bin statistical grass density following
the native 2 m terrain triangles, road distance, habitat and source photograph.
The 40-60 m single-view / 20-38 m split transition blends representations
before one composite. No new near tufts are added. Half-resolution RGBA16F
and root-distance targets use about 24.9 MB at combined 4K, plus static field
textures. The seed-1 3 km field had 353x1633 points, 388,898 occupied cells and
a 543 ms one-time CPU bake. Density is a four-sample habitat expectation,
not individual blades. Half-resolution nearest reconstruction can show 2 px
edges. Mean appearance is too smooth on some open slopes; retain opt-in status.

101 distinct CPU/shader/oracle checks passed (one unrelated CPU benchmark is
ignored by default). Twelve actual GPU optical cases initially found an AABB
entry/exit rounding bug, preserved in `far-grass-r12-optics-1`. Integer-cell
ownership now clamps only the known clipped entry and recognizes grid exit;
no epsilon interval is skipped. All twelve cases passed in
`far-grass-r12-optics-2/report.json`, including negative/grid-boundary rays,
odd viewports, native terrain diagonals, partial-road clipping and conservative
2x2 opaque clipping. Eleven existing tree optical GPU cases also passed with
the shared bindings (`voxel-optics-r12-shared-view`).

Exact r12b executable SHA256:
`05ec994693442cc72ec7d2cb10641666a616fa0d13db25138a860e75c479097e`.
Four separate native-4K hidden runs rendered 100 frames each, to physics tick
800, with far grass off/on and one/four views. Paired player/control states
matched exactly; see `r12b-grass-comparison.json`. Final-frame overflow was zero
in every view (single max 96 iterations; four max 119). Actual images were
inspected and show distant grassy coverage without filling the road or car.

| Final captured frame | Scene GPU pass sum, ms | Far-grass pass, ms |
| --- | ---: | ---: |
| One view, off | 17.873 | absent |
| One view, on | 17.977 | 0.190 |
| Four views, off | 35.084 | absent |
| Four views, on | 36.954 | 0.768 |

These are single-frame pass intervals, not averaged gameplay FPS or isolated
package speedups. A later actual F12 open-slope frame cost 1.190 ms for far
grass, showing view dependence. Its real image/metadata are
`engine/build/screenshots/sideways-1792791953659-192224-0.*`. Sparse captures
do not certify temporal shimmer, all stage seeds/weather or the whole route.
120 FPS remains unmet. The visible one-player 4K autoplay uses the opt-in
grass volume and regular source trees.

The intermediate r3 tree-cache specialization was rejected: 64-tree integrate
regressed from r2 32.242 to 44.546 ms. Only the 4096-iteration correctness cap
was retained with the original r2 caches. With the shared view bindings this
completed the formerly invalid fine 256-tree fixture in 49.705 ms, max 1330
steps, zero incomplete rays (`voxel-r12-256-fine`). Coverage ratio 0.890806 and
soft IoU 0.773714 remain approximate/cloudy. This is still far too expensive
for the intended full forest and stays disabled by default.
