# Real traversal avoidance: r48

26 October 2026. Interactive playtests remain stopped. This iteration tests
structural work elimination with unchanged source density, LOD, lighting and
shadow resolution. Target: one 4K output, four players, 8.33 ms; not reached.

Evidence directory:
[`r48-work-reduction`](C:/dev/perf/warp-sideways-20261026/r48-work-reduction/).
Both candidate-A executables were built together from an unchanged source
snapshot in 65.03 seconds including source checks/copying; Cargo reports 63 s.
[`candidate-a.json`](C:/dev/perf/warp-sideways-20261026/r48-work-reduction/candidate-a.json)
records the dirty checkout's individual source hashes and executable SHA256s.
No source reset, interactive launch, commit or push was performed.

## Jump across empty voxel regions

The r47 experiment avoided cell reads while still visiting every fine cell.
`KERN_BRICK_EMPTY_JUMP=1` now uses exactly empty ancestor mips to jump to their
exit. It retains the chosen optical mip, source field and occupied segment
equations. A bounded integer boundary search recovers the selected-grid cell;
it does not round a reconstructed world position or introduce epsilon steps.
Uncertain progress falls back to the reference traversal. The old read-bypass
flag remains separate; both flags default off.

Actual work counters distinguish fine iterations, coarse jumps and numerical
fallbacks. Their sum is the executed iteration count; skipped fine cells are
not invented as if the shader had visited them. No new GPU binding or field
allocation is required for jumping.

The fixture's new `--empty-jump-ab` mode produces each field once, checks its
publication, then freezes the exact GPU buffers and generation. Reference and
candidate consume those same bytes. This avoids r47's independent producer
reduction rounding differences. Four independent native 640×360 views form a
1280×720 output. Two sun settings and two wind poses each receive alternating
ABBA/BAAB blocks, four warmup and eight steady samples per block, 16 samples
per mode/pose. Startup, image writes and readbacks are excluded; producer passes
are forbidden in these consumer-only timing samples.

| Pose | Reference integration | Coarse jumps | Change |
| --- | ---: | ---: | ---: |
| Noon, wind 0 | 1.4633 ms | 1.2606 ms | −13.85% |
| Noon, wind 1 | 1.4719 ms | 1.2599 ms | −14.41% |
| Low sun, wind 0 | 1.4689 ms | 1.2606 ms | −14.18% |
| Low sun, wind 1 | 1.4638 ms | 1.2686 ms | −13.34% |

In each case executed iterations fall from 8,441,224 to 1,239,116 (85.32%
less). All first-frame and steady-block comparisons pass exact raw RGBA16
premultiplied colour/alpha and composited RGBA8 equality, with unchanged
occupied samples, ray/mip identities and provider hashes. No invalid rays or
jump fallbacks occurred. See the
[`brick-jump-a` report](C:/dev/perf/warp-sideways-20261026/r48-work-reduction/brick-jump-a/report.json).

The frozen fixture is `meadow-brick-r48-candidate-a.exe`, SHA256
`9377a86f92f8090fa452454f000ae8aadbb0729a82ab0d0c42be36f8bdc42897`.
These are integration-pass medians for a tiny enclosed four-brick field, not
whole-frame/gameplay timings. Exact consumer equivalence does not validate
source-to-voxel optics: the existing blur remains unaccepted. There is no live
Sideways replacement, production residency, boundary fallback or DLSS motion
support in this fixture. The reviewed noon preview still shows that limitation.

## Select headlight casters through shared spatial bounds

`KERN_HEADLIGHT_INSTANCE_GROUPS=1` independently traverses the existing spatial
hierarchy in each lamp's perspective frustum. Refitted bounds include actual
current source poses and wind. The hierarchy is shared, but the retained owner
list, scratch/indirect dispatch and counters belong to each lamp. Camera fog,
camera Hi-Z and camera-selected owners never certify a shadow rejection.

The original per-instance policy, perspective cluster LOD and shadow raster
remain unchanged. This removes rejected-owner dispatch/cluster-culling work,
not intended caster geometry. It defaults off. Bounds allocation works with
any combination of camera, sunlight and headlight grouping enabled.

Capture diagnostics now record independent lamp selections plus SHA256 of each
actual 1024×1024 Depth32Float map. Copies, waits and hashes occur only on explicit
diagnostic captures, excluded from steady timing. Timestamp capacity includes
up to eight independent headlight and five sun group-selection passes.

Candidate: `sideways-r48-candidate-a.exe`, SHA256
`d6b17f79f001251998d036f444d26990fe18cae9e57dc44f3d2da01dcc4e04a3`.
The [native comparison](C:/dev/perf/warp-sideways-20261026/r48-work-reduction/headlight-native-b/report.json)
uses 1920×1080 native output and 95 clean GPU samples per section/variant.
This is a limited two-run comparison, not repeat-controlled performance evidence.

| Native scene | Flat selection | Headlight hierarchy |
| --- | ---: | ---: |
| Night, actual chase camera | 8.1739 ms | 7.6645 ms |
| Night, far forest camera | 10.3625 ms | 9.9729 ms |
| Sunrise, near camera | 15.6702 ms | 14.7631 ms |
| Sunrise, far camera | 17.9804 ms | 17.4549 ms |

All 16 captured lamp depth maps are byte-identical by SHA256. Selected caster
fingerprints/counts/triangles and all camera/source inputs match. The near night
and sunrise previews are exactly equal; distant night previews differ in at
most 15 pixels by 1/255, distant sunrise in at most 451 pixels by 6/255. The
strict full-preview gate remains failed; the far sunrise pair was visually
reviewed with no apparent missing shadow. Shadow-map equality does not silently
turn the complete-image failure into a pass.

In the night chase capture, each reference lamp scans 321,825 owner slots.
The hierarchy visits 2,688 owner bounds and dispatches 275/266 retained owners
to cluster culling. It preserves the exact 5,383/5,343 selected clusters and
604,906/600,355 source triangles. Per-lamp owner scratch is 1,287,300 bytes;
shared bounds/ordering occupy 1,468,372 bytes, already needed for camera/sun
grouping in both variants. No hierarchy overflow or incomplete-traversal
fallback occurred.

The initial OFF run completed successfully; analysis failed on a startup
`cpu_frame` record with `frame_id:null`. The specific guard and regression test
were repaired. `--reuse-off` revalidates executable, environment, scenario,
inputs, captures, depth and source fingerprints, profiles and original file
hashes before reusing that run; only ON was launched for recovery. Original
failure evidence remains in `headlight-native-a`. Thirteen harness tests pass,
including reanalysis of those real completed files. This saved repeating GPU
work and startup after an analysis-only failure.

### Four-player repeated measurement

The [four-view ABBA report](C:/dev/perf/warp-sideways-20261026/r48-work-reduction/headlight-four-a/report.json)
records OFF A / ON A / ON B / OFF B, with 127 clean GPU samples per section per
run. One 3840×2160 output contains four independent views, each reconstructed
from 1277×717 to 1916×1076 with DLSS Quality, plus gutters. This is not native
4K. All eight physical headlights remain active. Both variants use the same
120-blade test recipe, meadow visibility/hidden-shading experiments and camera/
sun hierarchy settings; only headlight grouping changes. No density, draw
distance, lighting or shadow-resolution adjustment is part of this comparison.

| Four-player scene | Flat selection | Headlight hierarchy | Change |
| --- | ---: | ---: | ---: |
| Night | 34.3526 ms | 31.9929 ms | −2.3596 ms / −6.87% |
| Sunrise | 46.9173 ms | 44.4260 ms | −2.4913 ms / −5.31% |

These are means of two per-run GPU render-interval medians, not gameplay FPS.
Night OFF medians are 34.1390/34.5661 ms; ON 31.9222/32.0637 ms. Sunrise OFF
46.9156/46.9191 ms; ON 44.5263/44.3258 ms. Active CI remains a possible source
of contention. No CPU speed improvement is claimed from quantized Windows
execution ticks. The 8.33 ms target is still far away.
An independent audit verifies 1,016 clean samples without missing, duplicate,
or orphan frames and 128 exact lamp-depth/caster pair comparisons. Night ON B
has a 41.38 ms p95 versus 39.75 ms for OFF B: the repeated central improvement
does not establish an improvement to long-tail latency.

The actual eight-lamp cull scopes fall from about 2.83–3.27 ms to 0.211–0.217 ms,
plus 0.116–0.117 ms of hierarchy selection. Shadow raster stays about 0.70–0.72 ms.
These scopes are already contained in the render interval and are not additional
gains. Exact caster selections and raw depth hashes pass, with no hierarchy
fallbacks. This isolates eliminated selection work rather than fewer shadows.
Across eight lamps the hierarchy tests 22,272 owner bounds instead of scanning
2,574,720 flat owner slots; 2,310 owners reach cluster traversal. All 5,053,124
selected caster triangles remain.

All 16 strict preview comparisons fail, including OFF/OFF and ON/ON repeat
controls. Steady night OFF/ON differs in at most 1,460 pixels by 6/255; sunrise
in at most 9,473 pixels by 29/255. Same-mode sunrise repeats differ in up to
10,739 pixels by 50/255. The first sunrise frame has larger outliers, up to
124/255, also present in an ON/ON comparison. These are 1280×720 PNG previews,
not full-output pixel comparisons. Root inspected the stable OFF A/ON A night
and sunrise pairs: no apparent missing shadow or broad coverage change, but
the scenes retain dark foliage and coarse grass. Repeated baseline differences
do not prove their cause or waive the strict failure. The feature remains
experimental and default off; exact shadow-map equality is the narrower pass.

## Validation and remaining cost

462 renderer tests, five brick-fixture tests, 164 game tests and thirteen headlight
harness tests pass. The renderer/game retain one existing ignored test each.
Independent source reviews found no blocker in coarse jumps, lamp routing,
snapshot/depth readbacks or immutable-field timing isolation.

Headlight grouping cannot remove the larger directional-shadow raster cost.
The r47 sunrise capture measured roughly 7.43 ms shared plus 3.16 ms near-map
raster, with 47.84M shared and 11.71M near selected triangles. The next structural
gap is current-pose shadow demand derived from receivers across all four views.
It must include source grass, partial volumes/aggregate and transparent
receivers, light-space offscreen casters and filtering guards. Restricting it
to opaque camera depth would lose valid shadows. Existing rest-pose distant
pages are not a substitute for that mechanism.

The [receiver-driven shadow demand plan](receiver-shadow-demand-plan.md) scopes
that follow-up, including non-surface consumers and conservative fallback.
It is a design, not an implemented VSM system or another measured saving.

At closeout all 272 runtime/build source hashes still match candidate A. No
Sideways or brick-fixture process remains running. Changes are local and
uncommitted; frozen executables and failed image evidence remain intact.
