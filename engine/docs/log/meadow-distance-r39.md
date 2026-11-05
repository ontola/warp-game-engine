# Meadow distance continuity — r39, in progress

The candidate extends the existing dense procedural meadow beyond its visible
80–100m disappearance. It adds a **source-derived directional surface aggregate**
and a matching coarse terrain material. It is not a voxel marcher, Nanite, or a
replacement grass recipe. The first GPU candidate is rejected for excessive
cost. The optimized path has hardware-filtered atlas data and one shared lighting
resolve per covered pixel. Current candidate D moves the source-to-aggregate
blend to 60–80m. The measured package improves distant coverage but costs more
total GPU time than source-only rendering. Final acceptance is still pending.

## Candidate A: rejected on GPU cost

`native-a/report.json` records a serial same-executable off/on comparison at
1920×1080 native, one view, 39 clean GPU samples per section. The two actual F12
cameras changed from 19.185/18.569 ms total covered GPU time to 38.497/28.395 ms.
The new aggregate pass alone cost 18.942/9.592 ms. Elevated inspection cameras
cost 32–42 ms for that pass. Placement remained about 0.06–0.12 ms. These are
controlled scene measurements under active CPU CI, not typical gameplay FPS.

The images extend coverage over the previously pale slope, but the test enabled
both the aggregate and substrate material, so it does not isolate their visual
contributions. The implementation repeats manual directional atlas filtering
and full scene lighting for every overlapping patch. Low triangle counts did
not make this path cheap. Candidate A must not become the default playtest.
Its executable, images and timings are preserved under `candidate-a` and
`native-a` in the evidence directory. The next candidate uses hardware atlas
filtering and resolves lighting once per covered pixel; it requires fresh GPU
validation.

Evidence is being recorded in
`C:/dev/perf/warp-sideways-20261025/r39-grass-distance`. The initial five atlas tests
passed in the focused 32-test meadow run. That renderer run passed 337
tests with one existing GPU test ignored. These CPU/WGSL results do not prove
presentation, optical matching, temporal stability or a performance improvement.

## Measured optimization and current candidate D

Candidate C retains B's rendering changes and corrects the timestamp boundary.
`quick-isolation-c/report.json` has 39 matched steady GPU samples per section at
1920×1080 native, one view, with source blending over 80–100m. Values below are
mean milliseconds, not medians or gameplay FPS.

| Camera | Source-only total GPU | Aggregate + substrate total GPU | Aggregate render, including resolve |
| --- | ---: | ---: | ---: |
| F12-0 | 19.172 | 20.742 | 1.424 |
| F12-1 | 18.540 | 19.521 | 0.809 |
| Synthetic 100m-anchor inspection | 17.613 | 21.367 | 3.625 |

The component-isolation images show that the far aggregate supplies the main
visible coverage improvement; substrate-only colour continuity is insufficient.
These are visual observations, not a claim of optical equality with full source
geometry. The hardware/resolve rewrite removes candidate A's excessive far-pass
cost, but the complete package still increases total GPU time.

Candidate D adds `SIDEWAYS_MEADOW_SOURCE_RADIUS`, a finite override within
60–100m. Its default is 80m when the aggregate is enabled and 100m otherwise.
The source blend starts 20m before that radius, so the current aggregate default
is 60–80m; the aggregate's outer fade remains 160–180m. The override changes the
distance of the representation transition, not blade density or source geometry.

`quick-range-d/report.json` compares three settings of the same candidate D
executable, again with 39 matched samples per section at 1920×1080 native.
All values are mean covered GPU milliseconds:

| Camera | Source-only, 100m | Package, source 80m | Package, source 100m | 80m package extra vs source-only | Saving from 100m to 80m package |
| --- | ---: | ---: | ---: | ---: | ---: |
| F12-0 | 19.094 | 20.164 | 20.782 | +1.070 | 0.618 |
| F12-1 | 18.533 | 19.078 | 19.458 | +0.544 | 0.380 |
| Synthetic 100m-anchor inspection | 17.600 | 20.559 | 21.337 | +2.959 | 0.778 |

The 80m setting saves approximately 0.4–0.8ms relative to the 100m package in
these views. It still costs approximately 0.5–3ms more than source-only rendering
while covering a larger distance. **This is not an overall speedup.** These are
serial controlled comparisons under active CPU CI, without confidence intervals;
the synthetic anchor is not a measured grass-root distance.

Current executable: `candidate-d/sideways.exe`, SHA256
`46c964ebaf9bb260a47364cb81d637c4bb1de7c0d9a276e30abbd956a09ff9f2`.
The full native, night, motion/DLSS and four-view runs have now completed their
mechanical checks. Their results below supersede the earlier pending-run status;
final visual acceptance and playtest readiness are separate decisions.

## Candidate D: completed GPU suites

`native-d/compact-timings.json` records 11 static native 1920×1080 scenes, each
with 39 matched steady samples. These are mean covered GPU milliseconds. The
reference uses source grass to 100m; the package uses source grass to 80m plus
aggregate and substrate. Synthetic distances identify the camera setup, not a
verified distance to grass roots.

| Scene | Source-only total | Package total | Aggregate render, including resolve |
| --- | ---: | ---: | ---: |
| F12-0, noon | 19.041 | 20.240 | 2.198 |
| F12-1, noon | 18.451 | 19.110 | 1.537 |
| Synthetic 180m anchor, noon | 16.024 | 18.495 | 4.603 |
| Synthetic 130m anchor, noon | 16.755 | 19.858 | 4.737 |
| Synthetic 100m anchor, noon | 17.653 | 20.507 | 4.670 |
| Synthetic 80m anchor, noon | 18.595 | 20.881 | 4.435 |
| Synthetic 60m anchor, noon | 20.891 | 22.729 | 3.983 |
| Synthetic 130m anchor, sunrise | 22.220 | 24.909 | 4.271 |
| Synthetic 100m anchor, sunrise | 22.621 | 25.294 | 4.175 |
| Synthetic 80m anchor, sunrise | 23.126 | 25.581 | 4.116 |
| F12-0, night | 16.568 | 17.535 | 2.181 |

For the first F12 capture, the source count falls from 20,392,800 to 13,110,000
triangles, with 83,534 aggregate support triangles added. The second falls from
17,108,280 to 12,985,680, with 50,138 support triangles. These counts are before
per-fragment habitat rejection and count each source triangle once; they do not
describe pixel work or the additional depth submission. Lower source triangle
counts do not imply lower total GPU time.

`motion-d/report.json` records actual DLSS Quality at 3840×2160 output and
2560×1440 input, with no fallback and 67 matched steady samples per section:

| Motion section | Source-only total GPU ms | Package total GPU ms |
| --- | ---: | ---: |
| F12-0, changing wind, noon | 30.680 | 32.777 |
| F12-1, changing wind, sunrise | 29.384 | 31.366 |
| Chase, noon | 29.016 | 31.096 |
| Chase, sunrise | 31.248 | 33.555 |

These motion runs exercise presentation and reconstructed motion, but animation
poses are not established as exactly matched between runs. They are not an
image-parity oracle. The package costs approximately 2.0–2.3ms more in these views.

`four-d/report.json` records four actual views in one 3840×2160 output. Every
panel has 1916×1076 output and 1277×717 input using DLSS Quality, without fallback.
Each section has 71 matched steady samples and frozen 1.9s wind:

| Four-view section | Source-only total GPU ms | Package total GPU ms |
| --- | ---: | ---: |
| Chase, noon | 50.566 | 50.557 |
| Chase, sunrise | 60.580 | 60.377 |

The four-view totals are essentially unchanged; these small differences do not
establish a meaningful speedup. They are far above the 8.33ms budget for 120FPS.
All tables report controlled GPU intervals under active CPU CI, without confidence
intervals, not typical gameplay FPS or total application latency.

Review of four-view captures 96 and 192 found useful increased ground coverage.
An elevated native view still shows a crosshatch pattern on the far hillside
(`native-d`, capture 192). This is a known visible tradeoff, not a resolved
natural-grass appearance. Mechanical completion and these sampled reviews do
not prove continuous temporal stability or optical equality with full source.

## Source and distant representation

Near grass retains the r37/r38 geometry: 360 real opaque ribbon blades in each
0.45m world-cell patch, three shared variants and four branch wind groups. Its
accepted source cuts remain 1800/1080/600 triangles per patch. Every blade survives
source LOD selection; road habitat and normal visibility culling can still reject
work. No source density, fine geometry, colour palette or LOD error gate is relaxed
by this change. The distant representation submits two support triangles per
selected patch instead of the 600-triangle minimum source cut.

`meadow_aggregate.rs` cooks only the finest source triangles. For each of three
variants it uses 24 views: eight azimuths at elevations 0°, 30° and 60°. Every view
has a 32×32 base image with 4×4 subpixel samples per texel, plus six total mip levels
down to 1×1. Opaque nearest-surface rasterization measures visible union coverage;
hidden blades do not add density and projected holes remain empty.

Each covered texel stores a source point, normal, linear material colour,
roughness, discrete branch index and basal anchor. Source normals use the same
view-facing convention as the existing two-sided thin-leaf BRDF. The most covered
branch supplies the representative attributes; branch indices are never averaged
into invalid poses. Coverage averages across mip levels without amplification.
Runtime filtering blends coverage, material and normal samples across mip levels
and nearby directions, selecting a representative point/branch/anchor.

The CPU source oracle remains a 6,293,376-byte atlas, approximately 6.002 MiB,
with 5463 vec4 words per view and 131112 words per variant. Candidate B converts
its texels into two hardware-filterable RGBA16Float array textures, each with
144 layers and six mip levels. Each view occupies a pair of layers. One texture
contains coverage-premultiplied material and normal/roughness moments; the other
contains the representative source point/branch and basal anchor. Branch and
anchor use discrete covered-texel loads, never hardware interpolation of IDs.

Hardware bilinear sampling evaluates material moments at each of two mip levels,
followed by continuous mip blending. An analytic valid-footprint weight preserves
empty samples outside each mip's support when using portable ClampToEdge
samplers. No optional clamp-to-border or float32-filtering feature is required.
The moment division happens after filtering, so uncovered texels do not bleed
an arbitrary colour into covered grass. Half-float conversion introduces bounded
storage rounding; this is not bit-identical to the CPU float32 oracle.

The runtime payload is **3,148,608 bytes**: 3,144,960 bytes of texture data,
3,456 bytes of exact float32 projection headers and 192 bytes of source bounds.
The full CPU oracle is not retained as a GPU storage-buffer atlas. The bounds
separate each variant's fixed anchors from blade-relative extents so support
quads can account for growth and wind. Headers retain three words per view;
texture row zero lies toward negative `up`. Construction happens once per
renderer. Reported atlas cook time is CPU setup time, not GPU rendering time.

## Placement, lighting and the transition

The GPU uses the same deterministic 0.45m cell identity, native terrain triangles,
road exclusion, habitat, source variant, world pigment and branch wind rules for
both representations. There is no CPU per-root streaming. Terrain shear and its
inverse-transpose normal transform remain consistent with the near source.

Near and distant grass render into independent layers. Complete premultiplied
representations are mixed over 60–80m by default in candidate D, or 80–100m with
the 100m source-radius override. A faded near layer does not first occlude the
distant layer. The aggregate fades out over 160–180m. These are horizontal camera
distances used by the layer logic, not the arbitrary Euclidean distance to a
screenshot's camera target.

Overlapping aggregate patches use product revealage:

`alpha = 1 - product(1 - patch_alpha)`.

This preserves the union formula for the submitted fractional alpha values,
subject to render-target precision. It is not proof of the exact binary union
of correlated source silhouettes. Candidate B accumulates unlit material and
view-facing normal/roughness moments with that coverage, rather than evaluating
full lighting for every overlapping patch. A separate nearest surface supplies
depth and representative motion.

A fullscreen resolve divides the material moments by accumulated coverage,
normalizes the averaged normal, and evaluates the shared meadow lighting once
per covered pixel at the reconstructed nearest surface. This is an
order-independent approximation: lighting an averaged material/normal is not
the same as averaging individually lit surfaces, and nearest depth cannot
represent every contributing layer. It is **not exact depth-sorted colour
transport**. The source/aggregate merge chooses motion from the representation
with the dominant weighted coverage. The 160–180m fade is applied once to the
resolved union; applying it separately before overlap accumulation would keep
dense stacks too opaque.

The single resolve uses source-derived material moments and the engine's shared
thin-leaf response, sun, sky, scene occlusion and fog. It does not bake sun
illumination or invent a constant green emission. Current and previous branch
poses transform the representative source point and its basal anchor. This
supports reprojection; it does not make the stored rest-pose coverage a full
moving-blade silhouette. If accumulated normals cancel, the resolve uses a
view-facing fallback; it does not invent an upward ground normal.

`meadow_substrate.rs` separately reduces finest-source material and a 64×64
top-view opaque union inside the fixed 0.45m XZ footprint. The terrain shader
combines that data with the same stable habitat and world pigment. The uncovered
fraction retains the original terrain material. This is an albedo approximation:
terrain keeps its own normal, roughness and lighting. The coverage is measured
at rest and growth 1, without neighbouring-cell overhang, runtime wind or slope.
It must not be described as a dynamically exact optical ground layer.

## Allocation and work budget

The 180m aggregate grid currently allocates 651,249 candidate slots per view. At
124 bytes per candidate, its pool is 80,754,876 bytes, approximately 77.014MiB per
view. This is additional to the existing near-grass pool; allocated candidates
are not the number of visible patches. Four views would allocate approximately
308.06MiB for these additional pools before any image targets. This first
implementation is bounded, but it is not yet a compact clipmap or sparse pool.

Candidate B's additional aggregate, material-moment and merge image targets use
**48 bytes per input pixel** without temporal output, or **80 bytes per input
pixel** with the additional DLSS motion targets. The extra material and normal
targets trade memory for evaluating illumination only once per covered pixel.
For example, at 2560×1440 input the temporal allocation is 294,912,000 bytes
(281.25 MiB). Output 4K with DLSS does not imply those targets are allocated at
4K input. The actual per-view input sizes must be recorded, especially in
split-screen. Pool/atlas/target figures exclude other render targets, uniforms,
indirect buffers, pipeline resources, allocator overhead and the rest of the
scene; they are not total VRAM.

Reducing source triangles is only part of the cost. The aggregate adds root
generation, hardware-filtered atlas reads, unlit overlap accumulation, revealage,
depth/motion, one lighting resolve and a layer merge. A larger visible radius
can improve continuity while costing more GPU time. Candidate A established
that low triangle count alone is insufficient. The quick C/D comparisons quantify
the improvement over that rejected implementation and the remaining total cost.
The completed native and DLSS suites above measure broader scaling; final visual
acceptance remains separate. The 120FPS target is not met.

## Diagnostics and comparison

The candidate flags are `KERN_MEADOW_AGGREGATE=1` and
`KERN_MEADOW_SUBSTRATE=1`. The same-executable reference uses both 0. Comparing both
together measures their combined package, not either component in isolation.
The staged r38 executable is an additional historical reference; changes since
that executable make its comparison broader than the two flags. Candidate D
also records the actual source radius: enabled and disabled defaults differ,
so use `SIDEWAYS_MEADOW_SOURCE_RADIUS=100` when isolating only the two flags.

F12 and scenario captures explicitly read back `meadow.views[]`, including:

- `aggregate_enabled`, `aggregate_plants`, `aggregate_triangles`,
  `aggregate_capacity`;
- `aggregate_pool_bytes`, `aggregate_recipe_bytes`,
  `aggregate_target_bytes`, `aggregate_cook_cpu_ms`;
- `aggregate_transition_metres`, `aggregate_outer_fade_metres`;
- existing near source counts, ideal vertex-reference budgets and pool bytes.

`aggregate_triangles` is twice the selected patch count, counted once as source
support geometry. Separate depth and colour submissions are additional work.
Neither near nor aggregate triangles belong to the main visibility-buffer
`FrameStats.triangles`. A counted patch may have partial coverage or be rejected
by per-fragment road habitat; these are not visible-pixel counts. Capture
readback is synchronous and excluded from steady timing.

GPU timestamp scopes are `meadow placement`, `meadow render`,
`meadow aggregate placement`, `meadow aggregate render`, `meadow merge` and
`meadow composite`. The official frame-budget analyzer excludes startup, section
warmup and diagnostic/readback frames. Keep matched GPU sample counts and covered
GPU intervals separate from CPU wall time. CI is active on the machine; CPU
throughput is not a reliable rendering comparison.

From candidate C's timestamp correction, `meadow aggregate render` includes
the far depth/material work and the following single lighting resolve. Do not
report it as only atlas rasterization or add the resolve a second time when
summarizing GPU cost.

The prepared r39 runner replays both user F12 cameras and declared distance
steps, at noon and sunrise, with a separate motion sequence. Static runs freeze
wind at 1.9s. Default JPEG previews avoid large exports and do not prove pixel
parity. The distance anchor is explicitly synthetic until checked against actual
terrain in the first render. Native coverage/lighting and real motion must be
reviewed before accepting reconstructed playtest results.

## Remaining limits and acceptance work

- Rest-source silhouette coverage does not follow every animated blade. Source
  point and anchor motion is representative. Direction/mip mixing and growth
  projection are approximations, especially at shallow views and large wind.
- One nearest surface and representative branch cannot retain every depth layer,
  normal distribution or correlated occlusion inside a dense patch. The averaged
  normal/material resolve is a further lighting approximation and must be
  reviewed under grazing and back lighting, rather than assumed optically equal.
- The aggregate does not cast its own grass shadows. Receiving scene shadows is
  not the same as casting self-shadow or shading the ground beneath the grass.
- Road edges, steep slopes, atlas-direction changes, 60–80m source blending,
  160–180m fading, disocclusion and reconstructed motion are sampled by the GPU
  suites, not exhaustively validated. The far-hillside crosshatch visible in the
  elevated native capture remains an appearance issue.
- Memory scales per view. This is not full Nanite foliage, virtual geometry,
  radiance caching, or Lumen. The distant tree system is not replaced here.
- The previously observed single tree/terrain pixel difference in r38 changing-
  wind occlusion parity remains unresolved by this grass work.
- Native, night, motion/DLSS and four-player runs completed mechanically, with
  measured costs and selected visual observations above. Final visual acceptance
  remains separate. The 4K/four-player 120FPS target remains unachieved.

The immediate acceptance gate is a useful improvement in the actual user-camera
views without a new bright haze, dark distance ring or unstable patch coverage,
followed by honest GPU and memory measurements. CPU tests alone cannot close it.
