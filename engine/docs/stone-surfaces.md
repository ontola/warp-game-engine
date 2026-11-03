# Weathered stone prototype

The November 1 iteration replaces the shared Voronoi plate pattern with porous
stone. `stone_surface` bakes three irregular pit-size bands (radii 0.6-2.5 mm,
1.6-6.5 mm, and sparse 7-20 mm cavities), granular height and mineral variation.
Height, tangent normal and roughness come from the same periodic 1024-square
source. Near-field parallax and mip filtering are retained. Albedo is neutral
and contains no directional lighting; pores do not add real geometry or true
self-shadowing. GPU texture-array sizes and shader sampling budgets are unchanged.

`weathered_block::weathered_block(material, wear, seed)` starts from a 768-triangle sampling grid
for a noise-modulated rounded-box implicit field with sparse planar corner breaks, shared vertices
and field-gradient normals. The modulated field is not an exact distance bound;
meshing brackets its surface instead of assuming safe sphere-tracing steps.
A topology-preserving simplification targets a 3 mm error in the unit recipe;
unsafe sliver collapses retain the sampling grid. The ordinary cluster hierarchy
then supplies distance detail reduction. The game caches
four variants at three damage stages once per process; arena resets reuse them.
Noise seeds persist across damage, and all geometry remains within the unit
collision cube. Instance scaling also scales the erosion, including on thin slabs.

For shape/material comparisons, `stone_surface_gpu` accepts
`--geometry quarried|weathered` alongside its existing material modes. Hold
geometry fixed when comparing material modes. The game comparison script is
`games/growing-guns/scripts/weathered-stones.json`; the prototype is the default, and `GG_STONE_SHAPE=quarried` selects angular blocks. Neither option
changes simulation or collision. Validation results are recorded below.

## November 1 validation

Windows / RTX 5070 Ti, optimized dev build, native offscreen 3840x2160,
four views, seed 2, overhead camera, normal weapons (`--build none`). Three
alternating baseline/new runs, 30 warmup frames and 120 measured GPU samples
per run. The baseline executable retains the old angular geometry and old
plate texture, so these numbers compare the whole change, not only the material.

| Run | Before GPU median / p95 (ms) | New GPU median / p95 (ms) |
| --- | --- | --- |
| 1 | 6.97 / 7.36 | 7.74 / 8.22 |
| 2 | 6.69 / 7.30 | 7.62 / 8.20 |
| 3 | 6.69 / 7.26 | 7.94 / 8.62 |

The change costs approximately 1 ms of active GPU time in this scene. The new
p95 is 8.20-8.62 ms, so this does not establish a locked 120 FPS budget.
Serialized CPU/wall timings are recorded separately and are not presentation FPS;
this change does not resolve the earlier movement/combat stutter investigation.
The first 1200-triangle iteration measured 8.27-8.57 ms GPU medians; reducing the
cooking grid and simplifying reduced that cost. Seed 17 now has 522 source
triangles and 64 root triangles over four hierarchy levels. Other variants may
retain the bounded 768-triangle grid if simplification would fold a sliver.

Validation: two geometry tests covering 32 seeds and five wear levels (finite,
closed, outward, collision-bounded, deterministic, monotone sampled erosion,
actual coarser clusters); six material tests (physical normals/height, periodic
features, deterministic bake and real mip chains); two arena occupancy tests.
The headless script passes with all players alive and matching before/after
positions. Captures inspect close wall/tower masonry, sunset haze and a raised
morning-mist view. The tiny pores are material relief, not geometric holes.

Local evidence: `C:/dev/warp-gg/build/sdf-stones-validation.json`,
`sdf-optimized-{before,after}-{1,2,3}.log`, `sdf-stones-before-final/`,
`sdf-stones-optimized/` and `sdf-porous-optimized/`. The fullscreen playtest
bundle is `sdf-porous-playtest-20261101/` and its executable hash matches the
validated build. Screenshots show the native renderer, not generated mockups.

## Previous angular-surface iteration (October 28)


Growing Guns uses this first visual iteration as of 28 October 2026. It replaces
the smooth arena-block shape and the procedural colour-only rock shader with
angular shared meshes and a shared mipmapped surface. Gameplay collision and
damage thresholds are unchanged.

## Engine API and cost model

- `render::quarried_block::quarried_block(material, wear, seed)` cooks an inward-cut
  unit block, with planar fracture normals and some more closely dressed seeds.
  Flat central supporting faces and the original collision envelope are retained.
  A small palette is shared by instances; there is no per-frame mesh generation.
- `render::stone_surface::add_to_scene(scene)` inserts or reuses one texture layer.
  A process-wide `OnceLock` caches the deterministic 512-square source bake.
  Colour has no baked lighting. Normal direction comes from the physical height
  gradient; normal-map alpha carries a linear roughness multiplier. Normal B
  packs the same height in a signed 32 mm range; normal Z is reconstructed
  from XY. Custom ROCK maps can use B=1 to disable parallax. Albedo alpha
  stays opaque so foliage coverage processing cannot alter the roughness data.
- A `ROCK` material with a nonnegative layer uses this surface. The material tint
  and damage remain game choices. A negative layer retains the previous shader.
  The repeat is 1.5 metres and follows the instance, including moving rubble.
- Screen-footprint gradients choose texture mips. Normal relief fades continuously
  between 2.5 and 10 cm per pixel; the far branch skips the normal fetch entirely.
  Near shading uses bounded parallax occlusion mapping (8-20 steps with linear
  intersection refinement), then samples color and normal at the same displaced
  coordinate. It fades over 12-24 metres, coarse footprints and extreme grazing
  angles; subpixel offsets skip the march. Far shading uses one sample. This replaces the
  previous rock shader's nine noise octaves, not the rest of the lighting pass.
- The current renderer allocates 2048-square albedo and 1024-square normal layers,
  about 26.7 MiB including mips for this shared pair. There is no texture per block.

This is **material detail LOD**, not a new geometric LOD system. Quarried blocks
use at most 92 source triangles, versus 108/192 in the previous intact/worn
recipes. Such small meshes may fit a single cluster and retain the same triangle
count at distance. The fixture reports actual selected triangles and hierarchy
depths; do not infer distance-dependent geometric simplification from mipmaps.

## Repeatable inspection

```powershell
cargo build --locked -p render --example stone_surface_gpu
target/debug/examples/stone_surface_gpu.exe --mode textured --out stone-new --camera gallery --captures --frames 120 --views 1 --size 1920x1080
target/debug/examples/stone_surface_gpu.exe --mode legacy --out stone-old --camera gallery --captures --frames 120 --views 1 --size 1920x1080
```

The fixture holds geometry, texture uploads, exposure and lights constant and
changes only the material path. It records GPU pass samples/medians, selected
geometry, mesh hash, and close/grazing/far normal and albedo diagnostics. Fixed
world-patch comparisons validate ownership through GPU picks. Captures/readbacks
are outside timed frames. `--views 4 --size 3840x2160` exercises four viewports.
Use fresh output directories and at least three alternating repetitions for
performance claims; run serially without an active game.

For a whole-game comparison, `GG_STONE_SURFACE=legacy` restores old arena meshes,
the old arena material and plain stadium/rubble materials in the same executable.
That comparison changes several things and must be reported as a complete package.

## Validation and remaining work

The first new-surface gallery and real Growing Guns capture rendered successfully
on an RTX 5070 Ti. Evidence is locally in
`C:/dev/warp-bench/stone-20261028/quarry-preview/` and `gg-stones.png`. These are
visual previews, **not** a completed performance comparison. They show the early
geometry version. The subsequent convex-mesh robustness fixes are included in
the rebuilt executable; the earlier interactive playtest has stopped. No new
interactive session was launched, at the user's request.

The fracture pattern currently outlines too many similarly sized polygons,
giving some faces a smaller-stones-within-stone appearance. Next visual iteration:
reduce the continuous facet contribution and concentrate relief in sparse,
unequal, interrupted chisel cuts, retaining quiet areas and fine grain. Hard
dominant-face projection can also reveal seams on bevels. The prototype is not a
finished match to the user's hand-chiselled masonry and fissured stone references.
The Poliigon image was used as a visual reference only; no licensed texture or
preview is included in the repository.

Focused tests cover deterministic cooking, periodic values/slopes, physical
normal direction, actual texture-array mip filtering, shared-layer reuse, closed
convex meshes, outward normals, supporting faces, triangle counts and wear states.


## Parallax review (31 October 2026)

The `textured` fixture mode retains normal mapping with height disabled;
`parallax` enables the packed height field on identical geometry. `detail`
adds a near grazing view for checking cavities. This changes material UVs,
not silhouettes, collision, depth-buffer values or cast-shadow geometry.
No extra texture allocation is required. Material self-shadow rays are not
implemented. Camera-relative relief uses instance-space mapping so moving
rubble carries its detail. The mobile/minimum forward renderer is unchanged.

Validation: eight stone-surface tests pass, including packed-height fidelity
and actual mip-chain filtering. Vulkan/RTX 5070 Ti, 3840x2160, four close
grazing viewports, 30 warmup + 60 measured frames per run, three alternating
pairs: median GPU covered-union time 1.465 ms without POM and
1.868 ms with POM (+0.403 ms). These are the fixture's
instrumented GPU intervals, not full-game frame time or CPU wall time.
Local evidence: `C:/dev/warp-gg/build/pom-4k-{textured,parallax}-{1,2,3}/`.
The user's F12 replay is also rendered at its original 3840x2019 resolution
for an in-game visual check. Motion stability remains an interactive review.
