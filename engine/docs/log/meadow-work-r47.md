# Meadow work elimination and optical bricks: r47

Follow-up: [r48 actual empty-space jumps and headlight hierarchy](render-traversal-r48.md)
adds an immutable-field comparison and measured work avoidance. Results below
remain the historical r47 evidence; do not combine different experiments' gains.

26 October 2026. Interactive playtests are stopped at the user's request. Only
controlled, serial GPU runs are used. The target remains one 3840×2160 output,
four players and 8.33 ms. It has not been reached.
The Windows machine has an NVIDIA GeForce RTX 5070 Ti (driver 32.0.16.1714);
the fixtures explicitly request the NVIDIA adapter.

## Remove a geometry submission without reducing the source

`KERN_MEADOW_VISIBILITY=1` selects an experimental near-meadow visibility buffer.
One indexed draw writes the closest source primitive ID and depth. A subsequent
resolve reconstructs the source triangle and runs the existing grass lighting.
The reference submits source geometry for both depth and colour. Density, source
recipe, LOD selection, placement, wind and material settings are unchanged in the
comparison. The test recipe is the existing explicit 120-blade candidate, not
an assertion that the application default recipe changed.

The experiment requires the optional GPU primitive-index feature and remains
off by default. It runs after ordinary opaque shading, so it does **not yet**
avoid shading the ground that grass later covers. A separate experiment for
that omission must be measured independently.

Frozen executable:
[`sideways-r47-candidate-a.exe`](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/sideways-r47-candidate-a.exe),
SHA256 `f46afa98287243a251528e2907b9ba1a1eefef75fc7c71a80f1190bbf85232b6`.
[Source manifest](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/candidate-a.json).

| Controlled scene | Reference GPU interval | Visibility GPU interval | Change |
| --- | ---: | ---: | ---: |
| Native 1920×1080, near noon | 14.925 ms | 11.596 ms | −3.330 ms |
| Native, far noon | 16.897 ms | 13.988 ms | −2.909 ms |
| Native, near sunrise | 16.541 ms | 15.636 ms | −0.904 ms |
| Native, far sunrise | 19.452 ms | 18.081 ms | −1.371 ms |
| Four views, 4K output, noon | 43.559 ms | 38.696 ms | −4.863 ms / −11.16% |
| Four views, 4K output, sunrise | 49.628 ms | 46.778 ms | −2.849 ms / −5.74% |

The [native pair](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/visibility-native-a/report.json)
has 95 clean samples per section and is only a two-run check. The
[four-view ABBA](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/visibility-four-a/report.json)
has 127 clean samples per section/run. It uses four independent cameras and
DLSS Quality, 1277×717 internally to 1916×1076 per panel, including gutters in
the 4K output. Four-view values are the mean of two per-run medians. These are
controlled render intervals, not gameplay FPS or native-4K rendering. CPU/CI
contention and run variation still apply. Pass scopes are not additive gains.

Both paths preserve exact source counts and camera/sun selected-pair multisets;
the candidate reads back valid primitive IDs and changes actual source geometry
submissions from two to one. Native stable image pairs at frames 128, 256, 384
and 512 were visually inspected: no apparent coverage, colour or lighting
change, but strict decoded pixel equality fails. Across all eight native pairs,
the maximum channel difference is 5/255. Four-view stable pairs 192/384 were
also inspected. Their strict comparisons fail, including repeated reference
versus reference runs; DLSS amplifies small differences. The original reports
retain `accepted:false` and `promoted:false`. Visual similarity is not a strict
pixel-parity PASS, and fixed-camera tests do not prove temporal correctness.

The subsequent [native wind/camera-cut/night pair](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/visibility-motion-a/report.json)
checks 192 frames per variant at 1280×720, four sections and 30 clean samples
per section. Actual simulation ticks, wind poses, source selections and
headlight metadata match; night skips the sun passes. All four upward-sky
captures are pixel-identical; forest/night captures have small differences
(maximum 21/255 across the 16 image pairs). End-of-section frames 48/96/144/192
were visually inspected in both variants. This limited sequence does not
validate raw motion vectors, continuous driving or moving DLSS output. Strict
image acceptance remains false. Sparse/night views show no useful interval
gain in this two-run check; the optimisation primarily helps dense grass.

## Avoid background illumination hidden by fully opaque grass

`KERN_MEADOW_SKIP_HIDDEN_SHADE=1` is a separate opt-in and requires visibility
mode. Placement and the same single source visibility submission move before
ordinary shading. A source fragment certifies a skip only at opacity exactly
one and, if the far aggregate is enabled, before its blend start. The later
grass colour still fully replaces that pixel. Partial/transition grass keeps
background lighting. Current IDs are copied to a padded storage buffer because
the opaque shader already occupies all 16 sampled-texture slots. The copy and
counter costs are included in the GPU interval; memory is reported per view.

Frozen candidate C:
[`sideways-r47-candidate-c.exe`](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/sideways-r47-candidate-c.exe),
SHA256 `091c81ce8ecae35b0540fa77143b9333e569cbc7bd1d1d268cc1adfcaea98fe7`.
[Manifest](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/candidate-c.json).
Visibility stays enabled in both A/B cases; only hidden-shading skip changes.

The [native pair](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/hidden-native-a/report.json)
skips 1,357,359 pixels in the near view and 566,385 in the far view, using
16,589,056 extra buffer bytes per 1920×1080 view. Near noon/sunrise captures are
decoded-pixel identical. Far held captures differ at 509/447 pixels, max3/6;
all changes are in the upper distant scene, not the foreground grass or road.
Comparable variation exists between first and settled frames inside the
skip-OFF run. Aggregate ordering is plausible but not proven as the cause.

The [four-view ABBA](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/hidden-four-a/report.json)
uses the same 4K-output/DLSS-Quality configuration and 127 clean samples per
section/run as the visibility test:

| Scene | Skip off | Skip on | Change |
| --- | ---: | ---: | ---: |
| Noon | 38.607 ms | 38.055 ms | −0.553 ms / −1.43% |
| Sunrise | 46.776 ms | 46.421 ms | −0.355 ms / −0.76% |

Exact source, camera/sun selections and visible ID counts are preserved; no
invalid IDs and nonzero skips in the candidate. Counts are not an exact ID-map
fingerprint. Stable four-view frames 192/384 were visually compared, with no
apparent missing-background change. Strict images still fail, including
reference repeats, and remain recorded as failed. This feature stays opt-in;
it has not received a separate moving night/DLSS test. These incremental gains
must not be presented as a same-binary combined gain over candidate A.

## Real grass optical bricks: a bounded proof, not the live field

The new GPU producer uses the same source recipe, root-placement function and
branch deformation as ordinary meadow grass. It composes samples into world
interiors and the tree volume format: directional extinction and normal
moments, with four mip levels. Four views consume a shared current-pose field.
The consumer integrates active screen tiles through occupied cells; it does not
scan every grass root for every pixel.

The first fixture failed with magenta rays. Its DDA accumulated rounded cell
crossings that disagreed with the analytic exit, stepping outside the last cell.
The repaired version derives crossings from integer cell faces and integrates
the terminal cell through the analytic exit, without dropping a tail interval.
An f32 regression reproduces the original failure. Failure reports now retain
timings, provider data and per-view error counters before rejecting a run.

Repaired frozen fixture:
[`meadow-brick-r47-candidate-b.exe`](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/meadow-brick-r47-candidate-b.exe),
SHA256 `2ec1bbcaeb5af245d01acd21e214ea8b3a753b22c7fb32b0f034b273f4126dfc`.
[Evidence](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/brick-native-b/report.json)
and [changed-source manifest](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/brick-candidate-b.json).

Mechanical checks pass: four cameras, noon/low sun, two wind poses, directional
mass preservation across mips, finite output, bounded traversal, actual coarse
mip selection and rejection of invalid vertical bounds. The proof is a small,
fully enclosed habitat using four bricks. It has no production streaming,
source-boundary fallback or previous-pose/DLSS motion implementation.

**Optical acceptance is false.** The first reviewed source/volume pair is
noticeably blurred in the volume representation. Point quadrature and mip
homogenization preserve area approximately, not exact source union coverage or
silhouette. The consumer's current per-segment lighting is also expensive. This
is not a measured whole-field speedup and is not enabled in Sideways. Existing
far-meadow directional quads must not be described as this voxel field.

In the first noon fixture, four views traverse 8,441,224 cells for only 77,791
occupied samples (0.922%). Only 841 of 98,304 finest cells are occupied. The
fixed brick slabs schedule 369,792 pixel jobs and 233,988 intersecting rays.
Median integration is 1.456 ms, producer about 0.190 ms, tile planning 0.030 ms
and composition 0.011 ms. The source pass is about 0.080 ms for 16 roots and
9,600 finest source triangles per view. Instrumented pass sums are about
0.559 ms source versus 1.881 ms bricks, with 20 samples per case; those sums are
not a full render interval or FPS. This identifies empty-space traversal as a
concrete structural omission before spending effort tuning voxel density.

### Empty-cell read skipping: measured, not useful yet

Candidate C adds `KERN_BRICK_EMPTY_SKIP=1`, default off. Existing coarse mips
certify exact-empty regions; the consumer bypasses fine-cell fetches and optics
there. It deliberately retains the original fine-face arithmetic to preserve
occupied segment endpoints and terminal integration. This is **not yet** a
hierarchical jump that eliminates DDA steps.

The [same-binary four-run comparison](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/brick-skip-comparison-b.json)
uses frozen fixture C, SHA256
`009864a75f76eccfade8dc88c55f76012aa80c936e2b0622bc52c5ec81224342`.
Each run saves executable provenance, four sun/wind cases and 20 timing samples
per case. Noon-0 bypasses 8,312,409 of 8,441,224 selected-cell reads (98.47%),
through 1,110,301 empty regions. Logical cell steps and occupied samples are
unchanged. Median integration, averaged over two runs per mode:

| Fixture case | Skip off | Skip on |
| --- | ---: | ---: |
| Noon, wind 0 | 1.4860 ms | 1.4846 ms |
| Noon, wind 1 | 1.4820 ms | 1.4652 ms |
| Low sun, wind 0 | 1.4779 ms | 1.4694 ms |
| Low sun, wind 1 | 1.4825 ms | 1.4629 ms |

This is only 0.09–1.32%, not a useful demonstrated speedup. Keep it off. The
result rejects the assumption that avoiding cell reads alone solves this
consumer; actual traversal arithmetic and lighting still need work.

The strict comparison also remains failed: all source PNGs and non-skip
integer counters match, but regenerated provider moments vary even across
reference repeats. Relative integrated-mass drift is at most 1.62e-7, and six
of 40 image comparisons differ at one or two pixels by one byte, including
reference/reference comparisons. No tolerance was relaxed. A future exact
consumer-equivalence test should reuse one immutable GPU-produced field in
both consumers, avoiding independent floating reduction orders. All four
mechanical runs pass; this does not grant optical parity acceptance.

## Validation and current state

Candidate C passes 459 renderer tests, 2 brick-fixture tests and 164 game tests
(two existing ignored tests across renderer/game). Native, motion and four-view
evidence and earlier failures are retained. The render-path changes are opt-in;
the brick proof is fixture-only. No game or GPU fixture is left running, no
interactive playtest was launched, and physics/settings density were not
changed by these structural experiments. Work remains local and uncommitted.

## Next structural work

1. The hidden-background experiment now demonstrates a modest measured gain,
   but a unified visibility layout could avoid its extra ID-buffer copy. Keep
   partial fades and volume transmittance correct; never treat a brick bounding
   box as opaque coverage. Close the remaining temporal validation gaps before
   default promotion.
2. Make voxel consumption scale with projected occupied content, with compact
   traversal and reusable lighting rather than expensive lighting for every
   traversed segment. Preserve source-derived optics and wind; density changes
   are not a substitute for validating the representation.
3. Demand current-pose shadow work from visible receivers, with conservative
   filter margins, grass/transparent receivers and offscreen casters. Current
   instance grouping mostly saves selection work; it does not remove the tens
   of millions of selected sunrise caster triangles. See the
   [r46 work analysis](render-work-avoidance-r46.md).
