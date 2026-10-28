# Quarried stone surface prototype

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
  gradient; normal-map alpha carries a linear roughness multiplier. Albedo alpha
  stays opaque so foliage coverage processing cannot alter the roughness data.
- A `ROCK` material with a nonnegative layer uses this surface. The material tint
  and damage remain game choices. A negative layer retains the previous shader.
  The repeat is 1.5 metres and follows the instance, including moving rubble.
- Screen-footprint gradients choose texture mips. Normal relief fades continuously
  between 2.5 and 10 cm per pixel; the far branch skips the normal fetch entirely.
  Near shading uses two texture samples; far shading uses one. This replaces the
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
