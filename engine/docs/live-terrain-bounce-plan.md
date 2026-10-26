# Next live diffuse-bounce step — design only

Status, 2026-10-24: **not implemented in gameplay**. The CPU transport reference,
GPU cache sampler and world-terrain visibility tests are foundations, not a
completed scene GI system. This note records the next bounded integration;
the immediate visible priority remains distant forest lighting.

## What the current code can and cannot supply

`terrain_trace` provides real native-heightfield hit position and geometric
normal. `Scene.heightfield` has no material, road UV, terrain road-distance or
macro-slope attributes. The rendered terrain uses those attributes plus five
texture layers, continuous rock mapping, weather and road joins. Sampling a
texture average or assigning a green constant would not reproduce its surface.

`radiance_cache` has real Lambertian hit transport, SH9 and distance moments,
but requires all 96 directions resolved before a probe is valid. A terrain
`AboveTerrainBounds` result does **not** certify sky through trees, rocks,
barriers, bridges or cars. Wiring it directly to `Trace::KnownSky` is wrong;
marking every possibly obstructed direction unknown instead would invalidate
most forest probes. `radiance_cache_gpu` currently uploads/samples that CPU
reference; it neither traces this world nor evaluates game materials.

## Recommended first live slice

Keep a separate **single terrain-bounce channel**. Preserve current SH sky
lighting and specular. Only verified positive terrain-bounce contributions
enter this channel; unresolved paths contribute no added energy and retain an
explicit coverage mask. Do not renormalize partial directions to a complete
hemisphere. This is incomplete transport, not a promise of full scene GI or a
strict mathematical lower bound after SH reconstruction/interpolation.

1. Extract the existing TERRAIN/ROAD material evaluation into one shared WGSL
   function with explicit world position, geometric/authored normal, source UV,
   material, wetness and texture footprint. The normal camera pass must use the
   same helper, unchanged. Cache sampling uses a declared world footprint;
   it must not borrow a player's distance or duplicate the material in Rust.
   The current road `far_d` is view-dependent, so make its filtering argument
   explicit and test the camera output remains identical before adding GI.

2. Build a small GPU surface atlas from actual static terrain/road source
   triangles, using an orthographic downward visibility pass and that shared
   evaluator. Store linear diffuse albedo, actual triangle normal and world
   height, with world-tile identity/revision. This reuses the real textures,
   rock blend, road UV and material bindings without a CPU mesh BVH. Select
   terrain/road owners from a small stage-static chunk list, not all 400k-plus
   scene instances every tile. Discarding non-terrain owners here does not
   declare them transparent to transport.

   A downward atlas is a heightfield surface cache, not support for overhangs.
   Native trace hits may differ from refined rendering by the declared 1.10m
   shell. Require compatible cached height/normal and explicit uncertainty at
   steep discontinuities, multiple layers, bridges, missing tiles or stale
   identities. Do not silently borrow road/bridge albedo for the ground below.
   For the initial slice, exclude ambiguous cells rather than claiming exact
   detailed-mesh intersections.

3. Trace bounded primary rays on the existing GPU terrain hierarchy. On a
   compatible hit, evaluate the cached actual surface and a sun ray. Add
   `albedo / pi * sun_rgb * max(dot(normal,sun),0)` only when that path is
   verified. Keep native geometric and shading normals distinct; avoid using
   a filtered normal map as the intersection surface. Record rays rejected for
   surface mismatch separately from node-budget exhaustion.

4. Build a conservative object-uncertainty index from **all omitted loaded
   geometry bounds**, including foliage with wind margins, barriers, rocks,
   bridges and current cars. A sparse XZ grid with lower/upper height ranges
   and a min/max hierarchy is sufficient for the first conservative rejection
   test; it is not an opaque tree proxy. If the primary segment before its
   terrain hit, or the sun path before world escape, overlaps an omitted
   object's bounds, the contribution is unknown. Never turn that overlap into
   a fabricated black hit/albedo. Dynamic bounds invalidate affected cache
   entries. Dense forest will initially have low usable coverage; expose it.
   Global scene bounds, not the current camera range, determine escape.

5. Use a new typed bounce-only ray result
   (`VerifiedBounce`, `KnownNoBounce`, `Unknown`) and independent SH/coverage
   storage. Do not relax `Trace::KnownSky` or the existing strict cache tests.
   A finite clear primary segment is merely no hit within the chosen range,
   not known sky. Share residency/update budget over the union of four views.
   In the accepted region replace the old heuristic *bounce component* with
   the measured channel; leave the SH sky term intact and avoid counting both
   heuristic sun bounce and measured sun bounce. Missing coverage uses the
   existing approximation, with the transition and coverage visible in debug.

## Proposed first experiment bounds (not measured performance)

- 4m probe spacing, radius `[2,1,2]`: at most 75 requested probes per separated
  player, 300 total. Four probes updated per frame globally: 384 primary rays
  and at most 384 terrain sun rays; 256-node budget each. CPU/GPU unknown states
  remain explicit. This is a ceiling on work, not a millisecond estimate.
- 32m primary range. Surface tiles 16m wide, 32×32 texels (0.5m footprint),
  hard capacity 160 shared tiles. Albedo RGBA16F + normal RG16F + height R32F
  costs 2.5MiB, excluding identity/depth/transient visibility. Tile allocation
  must cover each probe's trace reach; budget eviction makes affected probes
  invalid, never aliases another tile. At most four tile refreshes per frame.
- 300 probes at the current packed SH9 + 96 moment + key sizes are about
  0.5MiB. Lighting/material/geometry revision changes invalidate old energy.
  No gameplay/physics GPU readback; no readback needed for update scheduling.
- Do not copy the diagnostic sampler's eight binary searches into every 4K
  pixel. Use constant-time integer world-cell lookup, reject outside resident
  bounds first, and measure the eight-corner SH/moment evaluation separately.
  Keep cache data packed or texture-backed so the scene storage binding limit
  is not exceeded. First consumer scope: opaque terrain/road only, opt-in.

## Suggested file boundaries and acceptance

New `terrain_surface_cache.rs/.wgsl`: atlas, static owner list, bounded tile
identity. New `terrain_bounce.rs/.wgsl`: uncertainty, GPU gather and scheduling.
`radiance_cache.rs`: separate bounce-only result/packing; retain strict existing
transport. `shade.wgsl` plus a shared material include: extraction and terrain
consumer. `frame.rs`: once-per-world update before per-view color passes,
statistics, revision invalidation. `shaders.rs`: include the new shared source
in transactional reload. CLI supplies no new shading constants.

Require real renderer tests for colored terrain-to-wall bounce, wrong-side and
closed-wall leakage, tree-bound uncertainty, bridge-height rejection, cache
eviction, live sun changes and four-view sharing. Compare source albedo/normal
against the camera material pass at controlled world points. Measure atlas
refresh, gather and sampling separately on/off, then inspect actual driving.

The captured 2° sunset valley has nearby terrain already in shadow; this first
single-sun-bounce slice may correctly add little light there. It must not be
marketed as the solution to the entire dark forest or as multiple-bounce GI.
Sky-lit surface bounce, foliage/mesh transport and non-heightfield geometry
remain later work.
