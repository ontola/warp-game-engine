# Status

Where each part of Warp stands now: what is the default, what is opt-in and
what is still open. This page is rewritten as things change, not appended
to. Dated measurements and the reasoning behind decisions are in the
[development log](log/README.md). Last reviewed 5 November 2026.

## Target

One 3840 × 2160 output, four players, 120 FPS (8.33 ms per frame). **Not
reached.** The latest controlled four-view, 4K-output Sideways route measured
about 38 to 44.5 ms of GPU time with DLSS Quality. Smaller scenes such as
Growing Guns' arena are much cheaper (see its README). Always report the real
resolution, reconstruction mode, sample count and scene with a number.

## Renderer core

- **Default:** cluster LOD, GPU culling, visibility buffer and compute
  shading, TAA, AgX ([rendering overview](rendering.md)).
- **Render suite** (5 November): `engine/tools/render-suite.py` measures one
  world from fixed cameras and compares runs ([guide](render-suite.md)).
- **Occlusion culling for every category** (5 November): history-first
  two-phase Hi-Z, on by default (`KERN_OCCLUSION=0` turns it off). Any rigid
  opaque geometry hides anything behind it, including the GPU forest; there
  is no wall or tree special case. Tested by
  `gpu_hidden_geometry_of_any_category_is_not_drawn`.
- **Opaque clusters raster without `discard`** (5 November): opaque buckets
  draw first through a pipeline with early depth, alpha-tested ones after.
  Same pixels as before on lavapipe (`gpu_opaque_and_masked_pipelines_draw_the_same_pixels`).
  **Open:** not timed on a real GPU yet; A/B with `KERN_RASTER_SPLIT=0` on the
  Mac and the RTX 5070 Ti.
- **Open:** fixed per-frame work for unused features; `frame.rs` still refers
  to Sideways features; new meshes or materials need a new renderer;
  animated pixels reject TAA history (no deformation motion vectors).
- Many experiments remain behind default-off switches; see
  [renderer switches](renderer-switches.md). They are candidates for removal.

## Terrain and world generation

- Procedural terrain from one height function with CPU and WGSL copies,
  GPU patches by projected error ([terrain](terrain.md)).
- Done on 5 November: [regions](terrain-regions.md) (step 1 of the
  constrained-generation roadmap), [zones and footprints](terrain-zones.md)
  (step 2), [world overrides](world-overrides.md) (step 3, CPU only).
- Ditches, streams and rivers keep their share of water at every LOD
  (`Sample::film`); ditches are also drawn per pixel up close.
- `terrain_fly --town` builds buildings (`GeometryCategory::Structures`) on
  zoned plots.
- **Next:** step 4, hierarchical GPU placement (with a GPU id table for
  overrides). **Open:** the far painted forest and its colour ignore regions,
  zones and overrides; streams and rivers are still drawn per vertex (only
  ditches per pixel); Sideways and SpaceRacer do not use regions. GPU parity
  for regions and zones has been checked on lavapipe only, not on real
  hardware.

## Vegetation (trees and grass)

Owned by the foliage work; its working checklist is
[vegetation-architecture.md](vegetation-architecture.md).

- **Default in `terrain_fly`:** one GPU route for trees and terrain grass:
  shared assemblies, GPU incremental complete-cut selection, bounded source
  and voxel paging, shared poses and a shadow tile cache. There is no CPU
  near-tree pool. Selection rounds follow the need read back from the GPU.
- **Open:** requested detail is not met (submitted voxel cells up to about
  5.5 px against a 1 px target in the normal scene, more in the full scene),
  upstream two-phase visibility, independent per-consumer detail and target
  performance. Source triangle and cluster work is accounted for but a tight
  budget was rejected. Games (Sideways) still use their own meadow path.
- Distant trees freeze to their rest pose between 60 and 90 m
  (`KERN_WIND_FREEZE`), so they match cached sun shadows.

## Shadows and lighting

- Physical units (lux, EV100, nits), procedural atmosphere, height fog with a
  finite-distance aerosol layer, irradiance volumes for interiors, opt-in
  [volumetric light](volumetric-light.md).
- **Opt-in static/dynamic sun split:** retains GPU source and voxel tree
  casters; exact split/full depth oracle covers still, wind, removal and
  restoration. Does not yet skip forest cull/splat work on unchanged pages.
- Sun shadows: dynamic near maps (180 m reach), a camera-independent cached
  sun map for stage scenes (`Scene::stage_sun`) and a camera-anchored far map
  for streamed GPU-forest worlds, plus the forest's shadow tile cache.
- [Automatic exposure and HDR output](hdr-and-exposure.md) (5 November): both
  opt-in. **Open:** a visual check on a Windows HDR and a macOS EDR screen
  before turning them on by default; Sideways' own presenter is SDR only; no
  HDR10/PQ metadata.
- **Open:** the edge of the near sun shadow map shows as a square from the
  air; far shadows should be drawn once per tile rather than every frame (one
  cached, paged sun shadow system).

## Games

- **Sideways** (rally): playable; the main performance scene.
- **Growing Guns** (arena shooter): a port of the Godot game; its README's
  status tables say how far the port is. Native Quest build (OpenXR) is
  experimental: about 107 fps average at full eye resolution and 120 Hz, not a
  stable 120 ([native Quest](quest-native.md)).
- **SpaceRacer:** playable native development port with Luau rules; visual
  and content parity in progress.
- **Spellstorm** and GameNight's **Game Room** lobby: being ported.
- Luau-only games (`hello-warp`, `neon-siege`, `neon-trails`) run with the
  [`warp` executable](warp-cli.md).

## Platforms

- Windows/Vulkan with an RTX 5070 Ti and DLSS is the main measurement
  machine; macOS (Metal, M5 Pro) is used for interactive work. On Metal a
  fullscreen window in another Space is not presented, so use headless GPU
  pass times there.
- Browser (wasm32, WebGPU): `terrain_fly` builds for the browser
  (`engine/tools/build-web.py`) as a prototype that loads and runs its
  frame loop in Chrome. The cull shader compiles there now; a drawn frame
  is not confirmed yet. No threads, gamepads, F10 menu or
  HDR; trees are opt-in (`?trees=1`) because they are cooked at every load
  ([Running in a browser](browser.md)). It needs 16 storage buffers per
  shader stage, which about 80 % of WebGPU devices offer but only about
  29 % of Macs (most give 10).
- The cloud container has no GPU: Mesa lavapipe runs GPU tests in software.
  It proves correctness, not performance.
