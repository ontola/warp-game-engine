# Rendering less work: r46

26 October 2026. Warp supplies reusable visibility, geometry, volume and lighting
mechanisms; Sideways supplies the terrain, vegetation recipes and test scenes.
The target remains one 3840×2160 output, four players and 8.33 ms per frame. It
has not been reached. Reducing detail settings is not the missing architecture.

Follow-up: [r47 measurements](meadow-work-r47.md) now quantify the near-meadow
visibility experiment and the bounded real-brick fixture. The state table below
describes the earlier r46 measurement, not an assertion those tests are pending.

## Evidence and actual cost

The frozen r46 executable is
[`sideways-r46-candidate-a.exe`](C:/dev/perf/warp-sideways-20261026/r46-work-reduction/sideways-r46-candidate-a.exe),
SHA256 `7badc5fdf630c96e6fcc255af37781d3f3defbcb16fc76c5fed38b23e5288675`.
The [four-view sun-selection ABBA](C:/dev/perf/warp-sideways-20261026/r46-work-reduction/sun-four-a/report.json)
uses four independent cameras, fixed wind, and DLSS Quality: each panel renders
1277×717 into 1916×1076, with gutters in the 4K output. Each section/run has
127 matched GPU samples after 64 warmup frames and capture exclusion.

| GPU scope, median ms | Noon | Sunrise |
| --- | ---: | ---: |
| Render interval, grouping off, mean of two run medians | 42.258 | 47.626 |
| Render interval, grouping on, mean of two run medians | 40.948 | 46.661 |
| Meadow source, on-a | 13.036 | 9.140 |
| Meadow directional surface aggregate, on-a | 5.572 | 5.114 |
| Remaining-tree raster, on-a | 6.689 | 6.707 |
| Main shading, on-a | 6.634 | 4.446 |
| Shared sun raster, on-a | 1.400 | 7.277 |
| Near sun raster, all four views, on-a | 0.364 | 3.246 |

Pass values come from the clean
[on-a budget](C:/dev/perf/warp-sideways-20261026/r46-work-reduction/sun-four-a/on-a/budget.json).
They are scopes inside the measured render interval, not independently additive
FPS improvements. The interval excludes presentation and uninstrumented work;
CPU wall time includes waits. Main-thread OS execution time is separate and
coarsely quantized. These controlled scenes are not typical gameplay claims;
CI contention and run variation limit precision.

The retained sun instance-group selection shows a modest **1.311 ms / 3.10%**
noon and **0.965 ms / 2.03%** sunrise interval reduction. It preserves the exact
selected caster-pair multiset; it primarily avoids culling work. Raster work is
unchanged. The strict PNG comparison still fails, including differences between
repeated references. The saved reports therefore remain `accepted:false` and
`promoted:false`; retaining the implementation is not a pixel-parity PASS.

The [native cluster-BVH pair](C:/dev/perf/warp-sideways-20261026/r46-work-reduction/clusters-native-recovery-a/report.json)
has 95 clean samples per fixed 1920×1080 camera: **20.185→20.327 ms** and
**19.153→19.357 ms**. It preserves selection but provides no useful measured
gain; it remains off in the sun ABBA. The separate
[native sun pair](C:/dev/perf/warp-sideways-20261026/r46-work-reduction/sun-native-a/report.json)
and [wind/camera-cut pair](C:/dev/perf/warp-sideways-20261026/r46-work-reduction/sun-motion-a/report.json)
are limited two-run checks, not repeat-controlled speed claims. Their strict
image failures remain recorded.

## What exists, and what does not

Actual flags are recorded in the
[run manifest](C:/dev/perf/warp-sideways-20261026/r46-work-reduction/sun-four-a/on-a/run.json).

| Mechanism | Actual state in this measurement |
| --- | --- |
| Current-bound instance hierarchy and tree Hi-Z | On. Sun grouping additionally selects in light space, preserving offscreen blockers. |
| Per-mesh cluster BVH | Implemented; off here after the native pair showed no gain. |
| Meadow indexed source, source LOD and opaque-depth occlusion | On. The explicit Sideways recipe uses 120 blades; its coarsest patch still has 200 triangles, submitted for depth and colour. |
| Far meadow aggregate | On. Source-derived directional surface samples with per-root quads; **not voxels**. |
| Shared meadow surface-page cache | Implemented experiment; off here. It is not accepted field-scale voxel transport. |
| Paged foliage sunlight | On, 256 m pages. Cached **rest-pose** source-alpha shadows, not reuse of current animated-tree shadows. |
| Whole-stage far sun map | Off. |
| Exact static/dynamic shared-sun cache | Implemented; off. r45 achieved high cache reuse but no useful forest interval gain because animated foliage dominates. |
| Zero-sun skip and retained shadow allocations | On. These already avoid night sun work; they do not remove daytime raster. |
| GPU meadow optical bricks | New bounded implementation/proof under validation; not the production grass field and not a measured speedup. |
| Near-meadow visibility and deferred resolve | Agent experiment in progress: replaces meadow's own depth/colour geometry submissions with one visibility submission and grass resolve, still after opaque shading. No acceptance or timing result yet. |
| Current-pose receiver-driven shadow allocation | Absent. Existing distant rest-pose pages do not supply this mechanism. |

At the captured sunrise, the shared sun pass selects **47,839,921 triangles**
and the four near passes together **11,710,983**. Noon selects **6,103,036** and
**352,944** respectively. These are actual selected source triangles, not visible
screen pixels. Light-space grouping keeps them unchanged. See the synchronized
[sunrise capture](C:/dev/perf/warp-sideways-20261026/r46-work-reduction/sun-four-a/on-a/captures/frame-000384.json)
and [noon capture](C:/dev/perf/warp-sideways-20261026/r46-work-reduction/sun-four-a/on-a/captures/frame-000192.json).

## Priorities and acceptance

1. **Make distant plant work follow projected detail, not the number of roots.**
   The new [producer](../../crates/render/src/meadow_brick_field.rs) composes shared
   source samples into world-owned optical cells; the
   [consumer](../../crates/render/src/vegetation_brick_render.rs) integrates occupied
   screen tiles without scanning every root per pixel. The first proof is capped
   at eight bricks, 262,144 cells, eight million sample jobs and 128 MiB producer
   allocation. It shares root placement, road tests, branch deformation and the
   tree volume cell format. It still needs optical, motion and cost validation,
   scalable residency and shared four-view scheduling. Point quadrature preserves
   area approximately; it does not prove exact opaque coverage. Invalid or stale
   pages require reference fallback. World-interior ownership must also govern
   source removal: dropping whole roots would break crossing boundary blades.
   First compare an entirely enclosed habitat, mass across mips, current poses,
   forced invalid generations and local source/volume coverage before extending
   the field. See the [volume-field plan](meadow-volume-field.md).

2. **Avoid the second near-grass geometry submission.** The pending experiment
   replaces meadow's own depth-plus-colour geometry passes with one visibility
   submission and deferred grass illumination. It still runs **after opaque
   shading**; it does not yet avoid lighting ground hidden by grass. Preserve
   true blade normals, road clipping, wind, lights and motion, and compare the
   complete added/removed meadow passes rather than triangle counts alone.
   Test sparse grass, cliffs, road edges, camera cuts and night. A subsequent,
   separate shared ground/grass surface resolve could shade the nearest opaque
   surface once and avoid hidden ground shading. Fractional far volumes must
   still composite rather than pretending their bounds are opaque.

3. **Allocate current-pose shadow work from actual receivers.** Begin with
   near-map demand after camera depth, including explicit support for grass and
   transparent receivers. Then combine all four views for shared sunlight.
   Offscreen casters remain eligible in light space. Filtering halos, missing
   pages, changing sun direction and animated bounds need conservative handling.
   A current-pose page system must not silently substitute the existing frozen
   distant-tree cache. Prove receiver coverage and shadow images before claiming
   fewer rasterized triangles or faster frames.

The existing source-plus-far grass scopes total about **18.61 ms noon / 14.25 ms
sunrise** in on-a: an absolute scope ceiling, not predicted savings from bricks.
The current visibility experiment works within the **13.04 / 9.14 ms** meadow
source scope and adds a deferred grass resolve; it cannot remove that entire
scope. Only a future shared ground/grass resolve could avoid the grass-covered
part of the separate **6.63 / 4.45 ms** main-shading scope. Near-shadow raster
alone has a ceiling of **0.36 / 3.25 ms**; shared-plus-near raster about **1.76 /
10.52 ms**. These ceilings overlap other proposed work and must not be summed.
Neither one small culling improvement nor deleting all near-shadow work would
close the gap to 8.33 ms.
