# Kern: an engine built for fast iteration

Rust workspace for the Sideways port. Designed around the loops that cost an
AI (or a person) time: compiling, tuning numbers, seeing results, finding
why two runs differ, and comparing with the Godot original.

| Crate | What |
| --- | --- |
| `kern` | engine core: deterministic `Sim` trait, state digests, first-divergence finder, traces as compact tables, timings per system |
| `gdcompat` | bit-exact Godot 4 pieces: PCG32 (`seed`, `randi`, `randf`, `randf_range`), `hash()`, FastNoiseLite |
| `sideways` | the game simulation: stage, terrain, road, barriers, car physics (Rapier), AI, race loop |
| `render` | the renderer: cluster LOD DAG (Nanite-style), GPU culling, visibility buffer, HDR shading in physical units, canopy-volume GI, AgX; headless, renders into textures |
| `cli` | the `sideways` tool |

## Commands

    cargo run --release -p cli -- play --views 1 --size 1920x1080               # explicitly open a playable window
    cargo run --release -p cli -- play --views 4 --size 3840x2160               # one 4K output, four split-screen views
    cargo run --release -p cli -- play --list-controllers                       # enumerate native pads; no window
    cargo run --release -p cli -- play --probe-controllers 15 --state-out pads.json # observe actual native inputs; no window
    cargo run --release -p cli -- play --smoke-ticks 600 --state-out input.json # bounded manual-input simulation; no GPU/window
    cargo run --release -p cli -- play --smoke-render 16 --capture play.png --state-out play.json # invisible bounded GPU smoke
    cargo run --release -p cli -- play --views 4 --size 3840x2160 --smoke-render 48 --hud-profile hud.json --capture hud.png # HUD-only GPU / CPU profile
    cargo run --release -p cli -- play --hud off                                # matched scene without the HUD

    cargo run -p cli -- shot out.png --seed 1 --km 3 --at 300 --weather clear   # a race moment, offscreen (16 TAA frames)
    cargo run -p cli -- shot out.png --taa 1                                     # one frame, no TAA: the quick look
    cargo run -p cli -- shot out.png --views 4 --size 1920x1080                  # split screen
    cargo run -p cli -- shot bare.png --ground-cover off                         # matched frame without grass
    cargo run -p cli -- shot out.png --pick 420,230                              # what covers that pixel
    cargo run -p cli -- shot out.png --set ev100=13.5 --set gi=0 --set debug=1   # retune light, A/B GI, sky visibility view
    cargo run -p cli -- gpu-run --out ../gpu-out                                 # gpu-jobs.toml (see GPU_RUNNER.md)
    cargo test -p render                                                         # every WGSL shader validated, no GPU
    cargo test -p cli ground_cover                                               # scatter/root/road/budget checks, no GPU

    cargo run -p cli -- bench                      # handling bench, next to Godot's numbers
    cargo run -p cli -- sim --ticks 45000 --seed 1 --km 2 --events
    cargo run -p cli -- sim --ticks 14000 --trace players.0.kmh,players.0.car.gear --from 12000 --every 30
    cargo run -p cli -- sim --ticks 13660 --probe 14.2,-166.0,1.5   # what colliders are there
    cargo run -p cli -- check /tmp/o.json --seed 1 --km 2           # compare with Godot

Car numbers live in `data/cars.toml` and are read at every run: retune and
rerun without rebuilding. `--cars other.toml` runs a variant.

Every `shot` also writes `<output.png>.json` with the actual camera positions,
targets, vertical FOV, split rectangles, car/wheel poses, weather, resolution
and AA/ground-cover settings. Use these inputs when comparing renderers;
matching the seed and starting distance alone does not match the final camera
after different physics simulations. GPU jobs accept `ground_cover = false`
for a matched no-grass baseline.

## Explicit Windows playtest

Build and stage the Windows preview with `tools\build-windows.ps1`. Launch it
explicitly with `tools\play-windows.ps1` (one view, 4K fullscreen), or add
`-Views 4 -Windowed -Size 1920x1080`. Keep this checkout: the current development
executable loads assets from its build-time repository path and is not a
standalone redistributable package. Measured Windows evidence and remaining
limitations are in [WINDOWS_VALIDATION.md](WINDOWS_VALIDATION.md).

Only `play` opens a window. Headless commands never create one, and
`play --smoke-render N` creates an invisible window for validation. In a
managed GameNight session, even normal `play` stays hidden during preparation
and becomes visible only after a valid host start/resume. Host pause hides the
window, releases fullscreen and stops GPU rendering while input/lifecycle
polling continues. The initial play
path runs the same car simulation at 120 Hz with four drivetrain substeps,
updates existing GPU body/wheel instances, invalidates moving shadows, and
presents the GPU image without a full-frame CPU readback.

Invisible windows are pumped explicitly because Win32 can suppress their
redraw events. Hidden validation creates/configures the real Windows surface,
then renders the presentation pipeline into an offscreen target of that surface
format. It reports zero presented frames; it does not prove visible swapchain
presentation. The smoke loop has a 90-second wall deadline in the event pump,
independent of redraw callbacks (an external timeout is still needed for a
driver call that never returns).

Windows native controllers use gilrs's XInput backend (the WGI backend did not
enumerate the attached 8BitDo in the initial test). `--probe-controllers 1..60`
records real mapped controls and device names through the same input path as
play; press the physical controls during its bounded run. Its optional
`--state-out` JSON distinguishes native activity from controller enumeration.

Native controllers use RT throttle, LT brake/reverse, A handbrake, X recovery,
Y chase-distance toggle, RB/LB manual shifts and Start pause. The right stick
selects a viewing direction around the car: right shows its left side, down
shows its front and release returns behind it. Keyboard fallback is WASD or
arrows, Space handbrake, R recovery, C camera, Q/E gears, Escape pause, N next
stage, M automatic/manual gearbox and F11 fullscreen. Back is left to the
host. Loss of focus neutralizes standalone driving and pauses simulation.

GameNight activation uses the existing SDK 6 environment variables
(`GAMENIGHT`, `GAMENIGHT_ADDR`, `GAMENIGHT_GAME_ID`, `GAMENIGHT_TOKEN`). Managed
input is routed by canonical seat/controller tokens, not native device order.
Start/pause/resume/dispose follow host sessions; missing/stale input becomes
neutral. Name and couch-order updates preserve car identities and poses.
The installed Windows Preview daemon advertises SDK 5. Optional SDK 6 actions
and room logs are sent only when supported; SDK 5 therefore has no declared
"New stage" action from this game. Stage rollover still works.
Roster additions/removals apply at the next stage, as advertised by
`instant_join=false`. Gearbox is live; stage length, rivals, contact and weather
are pending until the next stage. Optional `--state-trace trace.jsonl` appends
safe lifecycle/input/pose snapshots without launch tokens.

This first playable milestone is not feature parity or a 120 FPS claim.
Live driving disables the existing camera-only TAA until object motion vectors
are implemented. It also omits the frozen-shot grass field rather than leaving
grass behind at the start; live grass streaming is still pending. The basic
instruments are now rendered by the shared native-resolution HUD described below.
Audio, particles and an in-game settings menu are still missing. Native device enumeration alone does not
prove physical button response; the smoke trace reports observed native active
samples separately. GPU timings from frozen shots are not live-game frame rates.

## HUD and fonts

The HUD follows the Godot layout: stage time and driver name, gear/speed/revs,
two pace notes, route progress, handbrake/recovery, countdown and stage results.
Each player's output rectangle clips its HUD; the text-cache ID follows the
car when GameNight couch order changes. One font system, glyph atlas and text
renderer serve all four views. Bundled Barlow Condensed SemiBold and ExtraBold
Italic fonts retain their existing SIL OFL license. Latin accents need no system
fonts; missing glyphs trigger a one-time system-font fallback lookup.

Geometry and glyphs render after the tonemapped scene at physical output
resolution. The surface prefers sRGB so alpha coverage blends in linear light.
HUD text is independent of scene TAA. Resizing changes physical font sizes;
position/color changes preserve shaped buffers. Glyphs are prewarmed for the
instrument alphabet and buffers are retained only for active text IDs.

`--capture` now captures the final composite, including HUD and correct BGRA
channel conversion. `--hud-profile FILE` is restricted to hidden smoke runs of
at most 4096 frames. It records raw HUD-only GPU durations, CPU layout/prepare
and graphics wall time, dropping eight warmup frames by default
(`--hud-warmup N`). GPU query readback happens once at completion. Normal play
does not allocate profiling queries or perform a HUD readback. Profile wall
times include hidden GPU synchronization and are not gameplay FPS.

The oracle for `check`:

    godot --headless --path .. --script res://tools/oracle_dump.gd -- --seed 1 --km 2 --out /tmp/o.json

## Indirect visibility and stable shadows

Car bodies now provide up to six local opaque geometry proxies, excluding
glass, for smooth contact occlusion and reflected-ray blocking. Proxies follow
the live instance transform. Glossy materials also query terrain/canopy
visibility along their reflected direction; clearcoat reuses that query. The
HDRI sun is capped consistently with diffuse SH because the shadowed GGX sun
already supplies direct specular lighting. Approximate ground bounce uses
local sun/sky visibility instead of assuming sunlit ground everywhere.

Sun projections snap absolute light-space coordinates to texels. Split-screen
extent grows in fixed buckets retained for a stage. The canopy density grid
is world-aligned and hardware-trilinear-filtered; cars no longer splat into
this coarse translucent grid. Stone-wall winding/normals and terrain boundary
gradients were corrected. Thin foliage can still alias with live TAA disabled;
widely separated players still exceed one shared canopy volume's useful area.

`--set contact_occlusion=0` and `--set reflection_occlusion=0` isolate the new
effects in `shot`. Debug views are 1 sky visibility, 2 albedo, 3 direct light,
4 diffuse indirect, 5 environment specular, 6 contact visibility and 7 reflected
visibility. `cargo build --release -p cli --example lighting_fixture` builds a
deterministic offscreen real-car/ground/mountain fixture. The commands in
`tools/lighting_regression.py` separately plan, run and analyze it. Running is
an explicit GPU operation; do not compete with a user's live playtest.

These are bounded approximations, not full scene reflections, exact wheel-well
geometry or multi-bounce GI. See `WINDOWS_VALIDATION.md` for measured scope.

## Parity with Godot

- Stage layout, rivers, pace notes, heights, terrain chunks, trees and the
  road ribbon match Godot exactly, for every seed tried.
- Handling bench: within about 1% (0-100 3.67 vs 3.68 s, braking 42.4 vs
  42.7 m, top speed 213.6 vs 213.6 km/h, handbrake turn 143 vs 143 degrees).
- `--sim 45000 --seed 1 --km 2`: the same four stages; 6 finishes either
  way, 6 vs 7 recoveries, 228 vs 210 shifts. Bit-equal driving is not a goal
  (Jolt versus Rapier), the behaviour is.

Things learned on the way, each one a trap for the next port:

- `randf()`, `randf_range()` and `RandomNumberGenerator.randf()` use three
  different algorithms in Godot 4.5.
- Every GPUParticles3D takes a draw from the global RNG when created.
- Godot's concave shapes are two-sided; Rapier's trimeshes are one-sided,
  so the ground's winding is flipped.
- Parry's shape casts lose precision against huge boxes (the bench's 6 km
  ground made the suspension bounce); a half-space is exact.

## The iteration loops, measured

On a 2-core cloud container:

| Loop | Rust (kern) | Godot |
| --- | ---: | ---: |
| type check after an edit | 0.4 s | (parse on load) |
| rebuild after an edit | 0.6-0.9 s | none |
| retune a number and rerun the bench | 0.07 s | 3.3 s |
| handling bench (82 s simulated) | 0.07 s | 3.3 s |
| generate a stage layout | 1 ms | ~2 s (with startup) |
| build a stage's colliders | 0.7 s | ~10 s (with meshes) |
| 45000-tick race sim (4 stages) | 16 s | 112 s |
| find why the car went missing | one `--trace`, one `--probe` | prints, rerun |
| validate all shaders | 2 s (`cargo test -p render`) | run the game |
| render a frame on the software GPU (960x540) | 7 s; 16 TAA frames 41 s | about 35 s |
| find what draws a wrong pixel | one `--pick x,y` | inspect the scene tree |

## The renderer

- **Geometry:** every mesh becomes clusters of up to 128 triangles in a LOD
  DAG (meshoptimizer's clusterlod: groups simplified with locked borders,
  so any cut is crack-free). Per frame a compute pass culls and LOD-selects
  clusters per instance against a 1 px error; a cluster is drawn when its
  own error is small enough and its group's coarser replacement is not.
  Trees: about 4000 triangles up close, about 150 far away.
- **Visibility buffer:** survivors are drawn as one indirect draw (one
  instance per cluster, vertices pulled from storage buffers) into an
  R32Uint target: which triangle covers each pixel. Alpha-tested foliage
  is cut there, with coverage-preserving mips so crowns stay dense far away;
  under TAA the threshold changes every frame, so needles average out to
  soft partial coverage (alpha to coverage with many samples).
- **Shading** happens once per pixel in a compute pass: the view ray is
  intersected with the triangle for exact barycentrics; light in physical
  units (sun in lux, the HDRI scaled to the sky's lux and turned to match
  the sun, SH9 irradiance), a texel-snapped sun shadow map drawn through the
  same cluster path (cached while the sun box and scene stand still), distance
  fog. Terrain and road shading are ports of Godot's terrain.gdshader and
  road.gdshader.
- **Textures:** 2K albedo and 1K normal-map arrays, sampled with exact
  gradients from ray differentials (the neighbouring pixels' rays hit the
  same triangle's plane) and 16x anisotropic filtering: no blur at grazing
  angles. Normal maps (OpenGL convention) on terrain, rock, road, walls,
  bark and needles, in planar, triplanar and mesh uv frames.
- **Materials:** Lambert plus GGX sun specular, split-sum environment
  reflections of the sky (blurring into its irradiance with roughness),
  metalness, a clear coat for car paint (car_model.gd's numbers), Godot's
  foliage look (tint, per-tree shade, backlight).
- **Anti-aliasing:** TAA with a Halton(2,3) jitter, reprojection from depth,
  Catmull-Rom history, variance clipping in YCoCg, then contrast-adaptive
  sharpening. Only the camera's motion is reprojected so far: moving cars
  need per-instance motion vectors once the scene can move.
- **GI:** tree crowns enter a world-aligned, filtered density volume around
  the cameras. Opaque cars instead use bounded local contact proxies that
  follow their live transforms and do not shade their own body panels.
  Terrain and canopy tracing condition sky visibility, glossy reflected
  directions, distant sunlight and local ground bounce. This is approximate
  visibility and average blocked radiance, not full scene ray-traced GI.
- **Transparent models:** glTF BLEND glass and decals use a small per-view
  sorted forward pass into linear HDR. They do not write opaque depth or
  cast opaque sun shadows. Texture alpha and authored material alpha both
  contribute; no dither is used for glass.
- **HDR to screen:** exposure as EV100 (14 for clear weather, calibrated
  against Godot's frame), AgX, sRGB.
- **Split screen:** every view culled, rastered and shaded on its own, one
  shared shadow map, tonemapped into its rectangle.
- **Ground cover:** one photographed CC0 grass clump on three crossing cards,
  scattered on a common 0.45 m world grid around the active cameras. Density
  follows verge distance, patches and slope, then falls off over a broad
  distance range. Tuft size never depends on the camera. Roots sit on the
  rendered terrain triangles and the card footprint is excluded from the
  road. One, two and three/four views use radii of 80, 65 and 52 m with density
  multipliers 1, 0.68 and 0.42; duplicate cells are shared, and all views
  together have a hard limit of 60,000 tufts. Grass receives sun/canopy shadows
  and does not cast into the sun map. The existing Poly Haven diffuse/alpha
  originals and license are retained (`assets/licenses/grass-medium-01-cc0.txt`
  in the repository root). The selected clump is combined into an RGBA layer
  in memory before coverage-preserving mip generation.

- **Distant forest prototype:** high rolling background hills and up to 10,000
  trees reuse the three actual near-tree meshes and their existing cluster
  LOD, preserving species identity. Placement is fixed in world space. Native terrain/collision
  remains unchanged; the background extension has no new driving collision.
  Disable with `SIDEWAYS_DISTANT_FOREST=0` for matched comparisons. This is
  not Nanite assemblies or a virtualized geometry streaming implementation.
  The original generic exposed-face voxel pine trial is opt-in with
  `SIDEWAYS_GENERIC_VOXEL_TREES=1`; the source-derived density voxel bake is
  still an offline experiment.

The grass scatter is currently built on the CPU once for a frozen shot. It
does not yet stream as a live camera moves, animate in wind, or port shrubs,
ferns, stumps and mossy rocks. Whole tufts are selected by a world-cell hash
rather than a pixel dither; moving-camera pop-in and temporal stability still
need a real motion test. The split-screen budget bounds instance count, not
GPU time: each view still culls, rasterizes and shades its visible grass.

Not there yet: the remaining ground-cover types, car lights and particles,
wet road reflections, and the remaining interactive features listed above. GPU numbers come from
the runner (GPU_RUNNER.md); the container only has a software rasteriser.
