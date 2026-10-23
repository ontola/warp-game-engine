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
