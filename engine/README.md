# Kern: an engine built for fast iteration

Rust workspace for the Sideways port. Designed around the loops that cost an
AI (or a person) time: compiling, tuning numbers, seeing results and finding
why two runs differ. This project has two equal goals: improve the engine's
APIs, performance and development loop, and apply those improvements in
Sideways. Validate engine changes in the game; current comparisons are
between Kern versions, not against the archived Godot implementation.

Windows iteration: `tools/play-dev-windows.ps1 -Autoplay` builds while the
current game keeps running and stages a uniquely named executable. Use
`-Restart` for the following iteration, or `-BuildOnly` to avoid launching.
See [the developer workflow](tools/README.md) for logs, process ownership and
explicit first-presentation confirmation. This is build/restart, not shader
hot reload. F12 captures the actual frame, camera/state and same-frame GPU
pass timings; those timings exclude HUD, presentation and CPU work.

| Crate | What |
| --- | --- |
| `kern` | engine core: deterministic `Sim` trait, state digests, first-divergence finder, traces as compact tables, timings per system |
| `gdcompat` | bit-exact Godot 4 pieces: PCG32 (`seed`, `randi`, `randf`, `randf_range`), `hash()`, FastNoiseLite |
| `sideways` | the game simulation: stage, terrain, road, barriers, car physics (Rapier), AI, race loop |
| `render` | the renderer: cluster LOD DAG (Nanite-style), GPU culling, visibility buffer, HDR shading in physical units, canopy-volume GI, AgX; headless, renders into textures |
| `cli` | the `sideways` tool |

## Commands

    cargo run --release -p cli -- play --views 1 --size 1920x1080               # explicitly open a playable window
    cargo run --release -p cli -- play --autoplay --fullscreen --size 3840x2160 # explicit AI driving demo for remote viewing
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

Car materials (see "Car materials" below) iterate without `cargo build`:

    KERN_SHADER_DIR=crates/render/src/shaders cargo run --release -p cli -- shot car.png --taa 1 --lineup 4 --cam yaw=90,dist=10 --watch
    cargo run --release -p cli -- shot car.png --cam yaw=40,dist=4,car=0 --car-set paint=candy --car-set rust=0.6
    cargo run --release -p cli -- shot roof.png --lineup 1 --cam glint=1,dist=3.2 --car-set swirls=1   # the sun's reflection on the roof
    cargo run --release -p cli -- shot masks.png --cam yaw=40,dist=4 --set debug=9                       # 8 curvature, 9 wear, 10 grime

    cargo run --release -p cli --example voxel_forest_gpu -- --out FRESH_DIR --trees 64 --size 960x540

The voxel example compares actual source crowns with a bounded GPU density
volume prototype. It writes source/voxel images, raw-alpha coverage metrics,
GPU pass times and overflow diagnostics. It is not a full-forest renderer or
gameplay FPS measurement. `SIDEWAYS_VOXEL_TREE_PATCH=1..256` explicitly enables
a fixed source-derived patch in development play/shot; the default stays off.
It has no automatic near/far transition. Current close voxels are too soft;
the first dense 256-tree fine-level fixture exceeded its sample budget, so
this prototype is not accepted for replacing the regular forest.

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

## Vegetation and wind

`play` and `shot` accept the same explicit construction/animation options:

| Option | Default | Meaning |
| --- | --- | --- |
| `--season spring\|summer\|autumn\|winter` | `summer` | Evergreen needle tint; grass tint and stature |
| `--growth 0..1` | `1` | Established juvenile to mature tree/grass size, applied when building the stage |
| `--vegetation-seed N` | `0` | Stable tree phenotype variation, independent of the layout RNG |
| `--wind 0..1` | `0.3` | Shared world wind strength |
| `--wind-time SECONDS` | simulation tick / 120 | Freeze the wind phase for reproducible images |

    cargo run --release -p cli -- play --season autumn --growth 0.7 --vegetation-seed 42 --wind 0.3
    cargo run --release -p cli -- shot autumn.png --season autumn --growth 0.7 --vegetation-seed 42 --wind 0.3 --wind-time 2.5 --taa 1

Summer/growth 1 preserves the existing vegetation geometry and color exactly;
gentle wind at 0.3 is a new CLI default. Use `--wind 0` for a stationary
baseline. `sim` accepts the construction options too. The engine's
`WindState::default()` and `gpu-run` jobs retain wind 0; jobs can opt in with
`wind`, `wind_time`, `growth`, `season` and `vegetation_seed` TOML fields.

The reusable game API is `sideways::vegetation::Vegetation::new(seed, growth,
Season)`, assigned to `race::Settings::vegetation` before constructing a race.
Native tree spots feed both colliders and visible instances; distant trees
reuse the same phenotype function. Roots, populations and layout RNG draws
do not change. Near grass size/tint and the far volume's source-area/color
moments use the same profile. Stage reconstruction is required to change it.

Call `Renderer::set_wind(&gpu, &WindState { time_seconds, strength,
..Default::default() })` before rendering. One GPU coefficient field drives
every view, source LOD, source shadow and deferred reconstruction, keeping
roots fixed without CPU uploads of every plant transform. This is bounded
whole-plant shear, not branch/leaf physics. Experimental packed voxel-tree
owners remain static; canopy and far-grass density use mean rest poses.
Pines do not turn into deciduous autumn trees. There is no continuous
biological growth, snow accumulation or seasonal terrain-material change:
the ground/verge texture keeps its existing color beyond the grass volume.
Snapshots and shot sidecars record the resolved profile and wind phase.

## Per-frame scene GPU profiling

    cargo run --release -p cli -- play --autoplay --views 4 --size 3840x2160 --smoke-render 90 --gpu-profile route-gpu.jsonl --gpu-warmup 30 --wind 0.3

`--gpu-profile` requires a new writable JSONL file and works in visible play
or bounded hidden smoke. It streams a header and each frame after the warmup
(30 by default), including adapter, route/tick, cameras, output size,
vegetation/wind settings, GPU pass durations and work counters. Missing GPU
timestamps remain explicitly unavailable. Hidden smoke reports no presented
frames; its route pacing is a test harness, not normal gameplay.

Profiling synchronizes same-frame GPU readback. `instrumented_gpu_ms` sums
only timed scene passes; it excludes HUD, presentation, CPU simulation and
uninstrumented work. Neither that sum nor the synchronized wall time is a
gameplay FPS measurement. Normal play retains the asynchronous render path;
`--hud-profile` measures the separate HUD scope described below.

| Counter / bound | Interpretation |
| --- | --- |
| `submitted_vertices` | Opaque cluster draw vertices, including padding; excludes later glass, transition-grass and volume passes |
| `shadow_submitted_vertices` | Actual far/near source-shadow draw vertices this frame; a cached far map contributes zero |
| `clusters`, `triangles` | Selected geometry counts, not covered pixels; triangles also include the forward glass pass |
| `shadow_clusters`, `shadow_triangles` | Retained shadow geometry, including a reused far map; not actual work on a cache hit |
| `early_grass_rejects` | Per-view instances skipped because their opaque raster path was already invisible; not plants removed from the transition pass |
| `canopy_updated_levels`, `canopy_pitch_m` | Per-view rebuild masks and world pitches; zero mask means no density rebuild |
| 70,000 / 115,000 grass slots | One-view / shared split-screen residency caps, not per-player densities or millisecond budgets |
| 27 MiB canopy per view | Density texture plus atomic working storage; up to 108 MiB for four views, excluding other renderer allocations |

`KERN_CLUSTER_BUCKETS=0` and `KERN_EARLY_GRASS_CULL=0` independently disable
the two draw-work optimizations for matched comparisons. Effective states
are recorded in each sample. Buckets submit 8/32/64/128-triangle capacities
instead of padding every cluster to 128; visibility IDs and source triangles
are retained. No performance improvement is implied by these options alone.

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
are implemented. Grass streams around the active views in a fixed pool of
70,000 slots (one view) or 115,000 shared slots (multiple views), retaining
world positions and sizes. `SIDEWAYS_LIVE_GRASS=0` disables it. The basic
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
sun-map extent grows in fixed buckets retained for a stage. Each camera now
keeps three world-aligned canopy density grids at 1.5/6/24 m, with a larger
coarse pitch when the initial scene height envelope requires it. They cover
distant/high forests as well as nearby crowns. Continuous blend anchors are
separate from snapped storage windows. Coarse cells retain integrated crown
mass instead of missing a small tree when its centre falls between samples.
All levels use additive ellipsoid extinction; this changes the previous
max-union near approximation. These are shadow/sky-visibility proxies, not
the experimental source-leaf voxel renderer.

Per-camera fields cost 27 MiB, up to 108 MiB for four views. Pipelines and
validation metadata are shared; unchanged fields emit no rebuild passes.
Car/grass movement does not invalidate static forest density. Structural
foliage edits do; adding/rescaling crowns requires rebuilding the renderer.
Every surface, grass and volume pass samples the same lighting representation
for its camera. Thin foliage can still alias with live TAA disabled. The
canopy coverage guard is for 50/60 m sun/sky queries, not a guarantee for the
full 250 m glossy-reflection query at the outermost visible forest edge.

Indirect lighting currently uses nine global RGB sky SH coefficients,
visibility queries and approximate local ground bounce. There is no spatial
radiance cache, scene color bleeding or multi-bounce light transport. A
cached extinction field is not a cached lighting solution.

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
| retune car paint/wear data, `shot --watch` (1200x420, 1 frame) | 2.9 s edit to PNG | |
| edit car/shading WGSL, `shot --watch` (900x500, 1 frame) | 0.7-0.9 s pipeline rebuild, 5.2 s edit to PNG | |
| the same WGSL edit through `cargo build --release` + `shot` (960x540) | 2 min build + 31 s shot | |

## The renderer

- **Geometry:** every mesh becomes clusters of up to 128 triangles in a LOD
  DAG (meshoptimizer's clusterlod: groups simplified with locked borders,
  so any cut is crack-free). Per frame a compute pass culls and LOD-selects
  clusters per instance against a 1 px error; a cluster is drawn when its
  own error is small enough and its group's coarser replacement is not.
  Trees: about 4000 triangles up close, about 150 far away.
- **Visibility buffer:** survivors are drawn in four indirect size buckets
  (one instance per cluster, vertices pulled from storage buffers) into an
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
  metalness, Godot's foliage look (tint, per-tree shade, backlight), and
  procedural car materials (below).
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

- **Distant forest prototype:** high rolling background hills and up to 200,000
  trees reuse the three actual near-tree meshes and their existing cluster
  LOD, preserving species identity. Density is calibrated to actual native
  trees per eligible horizontal square metre; logs report any budget or
  candidate shortfall. Placement is fixed in world space. Native terrain/collision
  remains unchanged; the background extension has no new driving collision.
  Disable with `SIDEWAYS_DISTANT_FOREST=0` for matched comparisons. This is
  not Nanite assemblies or a virtualized geometry streaming implementation.
  The original generic exposed-face voxel pine trial is opt-in with
  `SIDEWAYS_GENERIC_VOXEL_TREES=1`; the source-derived density voxel renderer
  is a separate bounded experiment, described above.

Frozen shots build the full grass scatter once. Live play caches candidate
plants in 3.6 m world tiles and updates residency masks at the moving border;
surviving plants keep their three cards, size and fixed slots. The pool has
70,000 slots for one view and 115,000 shared slots for split screen. Near
grass uses a nearest-layer opacity fade at 40-60 m (20-38 m in split screen),
with residency extending beyond the fade. Roots follow the native terrain
triangles and exclude the road. Plants receive sun/canopy shading without
casting shadows. Shared wind is available as described above; shrubs,
ferns, stumps and mossy rocks remain pending.
These budgets bound instance count, not GPU time.

`SIDEWAYS_FAR_GRASS=1` explicitly enables the new development grass volume
in live play. Four height bins follow the native 2 m terrain triangles and
road mask. Source-photo coverage and habitat rules estimate its density;
near and far representations blend before one composite over the scene.
Each bin also retains extinction-weighted source color and normalized plant
height, so its material response uses the same roots/tips as the near tufts.
Far grass fades out over 160-240 m. This is a statistical, half-resolution
volume, not individual blades or full Nanite. Its mean appearance is still
too smooth on some open slopes, so the normal default remains unchanged.
The fixed allocation cap is 4,194,304 native grid points; an oversized field
fails explicitly. See WINDOWS_VALIDATION.md for bounded GPU tests and costs.

Engine integration: `Scene::far_grass` owns the terrain/density field;
`volume_view::VolumeViewInputs` supplies typed shared lighting/depth inputs
to both grass and tree-volume passes. `Renderer::render_texture` stays free
of readback; `render_texture_profiled` explicitly waits for same-frame scene
counters/timings without a redundant image readback. It is for developer
captures, not continuous gameplay telemetry.

In developer play, F12 saves the current image including HUD and a metadata
sidecar to `engine/build/screenshots/`. On Windows it also copies the image
to the clipboard; paste it into the chat with Ctrl+V. No automatic upload
occurs. The metadata records the stage, cameras, cars, graphics settings and
same-frame scene GPU pass times. Missing timestamp support is marked as
unavailable. Per-view canopy pitches and rebuilt-level masks identify cached
frames. HUD, presentation and CPU time are outside these timings.
Hidden `play --smoke-render N --capture image.png` also writes
`image.png.metrics.json` for the final frame.

Not there yet: the remaining ground-cover types, car lights and particles,
wet road reflections, and the remaining interactive features listed above. GPU numbers come from
the runner (GPU_RUNNER.md); the container only has a software rasteriser.

## Car materials

Car surfaces are generated in the shader (`shaders/car_material.wgsl`,
appended to shade.wgsl) from the object-space position and normal, noise
filtered by the pixel footprint, and a curvature the loader bakes per
vertex (dihedral angles of the welded mesh, convex positive) into the
paint's unused uv. No texture assets. Layers, bottom to top: bare steel,
primer, paint, clear coat, then wear and grime:

- **Paint finishes:** solid; metallic (flop: darker edge-on, and flake
  glints, one random flake normal per pixel-sized cell, fading into a smooth
  lobe when flakes are far below a pixel); pearl (hue shift toward `second`
  at grazing angles, tinted mica); candy (tinted lacquer over bright flakes,
  deeper at grazing angles). Matte and satin are presets with less or a
  rougher coat. The coat has its own normal (orange peel) and roughness;
  its sun term never gets sharper than the sun's 0.5 degree disc.
- **Wear:** chips on convex edges, the nose, sills and arches (primer, then
  steel), small stone chips, straight scratches in the coat, polish swirls
  (short buffer arcs that only light up where they run across the direction
  to the sun's reflection), and sun fade on upward panels.
- **Grime:** rust from bare steel, low edges, creases and wheel arches, with
  blistered paint around it and brown run-off below; a dust film low down,
  behind the wheels and on top; mud splatter, glossy when wet.
- **Other parts**, by glTF material name: tyres (the tyre texture's rubber,
  disc and lettering told apart by brightness and radius; aged sidewalls),
  alloy rims, chrome, unpainted plastic that greys in the sun, and textured
  parts that keep their texture and only get grime. Glass keeps the forward
  pass's shading.

Parameters live in `engine/data/car_materials.toml`, read at run time:
paint presets, then `[all]`, `[model.<glb>]`, `[car.<index>]` and
`--car-set [N:][part.]key=value`, each overriding single keys. The file
documents every key. `shot --lineup N` puts N copies of car 0 nose to tail
on the road (cars 0..N-1 in the data); `--cam yaw=,dist=,height=,car=,fov=`
is a close camera and `--cam glint=1` looks down the sun's mirror direction.
`--set debug=8|9|10` shows curvature, wear and grime masks.

Without a rebuild:

- `KERN_SHADER_DIR=crates/render/src/shaders` reads wind_common, shade,
  car_material and transparency .wgsl from disk when the renderer is made
  (missing files fall back to the built-in copies), so a plain `shot`
  picks up shader edits. Other shaders stay built in.
- `shot --watch` keeps the scene and renderer alive and re-renders when the
  data file or those shaders change; `play` polls the same files every
  250 ms. Changed WGSL rebuilds only the shade and glass pipelines, under a
  wgpu validation error scope: broken code prints the compiler's message
  and the previous pipeline keeps drawing. Broken data keeps the previous
  materials. Changing a part's `match` or `kind` needs a restart.

Measured on the 2-core cloud container with lavapipe (CPU rasteriser), see
the iteration table above. The edit-to-PNG time is almost all lavapipe
rendering the frame; on a real GPU the same loop is the pipeline rebuild
(0.7-0.9 s here, mostly WGSL to SPIR-V and LLVM) plus a frame.
