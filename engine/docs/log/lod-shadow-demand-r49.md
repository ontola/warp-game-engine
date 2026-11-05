# r49: matched grass optics and receiver-driven shadow work

26 October 2026. Controlled GPU experiments; interactive playtests remain stopped.
The 8.33 ms four-player goal is not reached. No production feature defaults,
plant density, physics rate, light intensity or shadow resolution changed.

## Grass selection and optical continuity

`KERN_BRICK_SMOOTH_MIPS=1` blends adjacent optical coefficient fields only during
the first quarter octave after the coarser mip meets the projected-cell budget.
Both levels meet that budget. Outside this narrow band only one level is read.
The implementation traverses the finer grid once; an empty fine child is not
skipped if its contributing coarse parent contains extinction. Exact-empty
ancestor jumps retain their existing reference path.

The current prototype's finest cell is 11.25 cm. Source blade detail is already
lost at that scale, before a mip transition. `brick_projected_error_bound` is a
conservative whole-brick spatial gate: callers must retain source geometry
when even the finest cell exceeds the requested pixel budget. Spatial
eligibility alone does not establish optical equivalence. This is not yet a
production source/brick replacement selector.

`meadow_brick_field_gpu --optical-lod --out FRESH_DIRECTORY` compares the exact
same frozen GPU-produced field against independent finest source geometry.
It uses three wind poses, noon/low-sun/night-ambient lighting, four cameras,
and 2/4-pixel budgets with hard/smooth mip selection. A 2x-linear-resolution
source reference preserves fractional coverage. Metrics are restricted to
the source/volume foreground union; background cannot dilute missing grass.
Eight steady native timing samples follow four warmups per block. Reference
supersampling, GPU readback and field generation are excluded from those
consumer times. No result automatically authorizes live replacement.

## Receiver demand

`KERN_RECEIVER_SHADOW_DEMAND=1` gathers current wind-expanded owner spheres
intersecting each camera plus procedural meadow and live-particle receiver
bounds. It builds 64-texel demand masks for the existing shared sun map and
four independent near maps, with a 2x2 OR hierarchy. Selected caster clusters
may be omitted only when their conservative light-space footprints miss
all demanded tiles. Offscreen upstream casters remain eligible.

This implementation differs from the more ambitious
[design](receiver-shadow-demand-plan.md): it gathers conservative current
owners before camera raster, rather than splitting the frame to reuse exact
selected receiver clusters. Terrain owners use all transformed resident cluster
bounds, across all LODs/materials, instead of one whole-route sphere. It avoids
duplicate raster/placement work and requires no gameplay readback, but coarse
hierarchy bounds and unoccluded receiver support can still saturate demand.
Published fully requested maps bypass per-caster projection and tile queries;
separate counters distinguish those retained casters from rejected work.
Actual mask occupancy, omitted triangles and net frame cost determine value.
This is neither virtual page allocation nor a persistent shadow-page cache.

Grass bounds include the aggregate shader's ray-reconstructed lighting point,
perspective billboard magnification, pixel padding, wind, growth, terrain
shear and atlas quantization. Invalid support or upload overflow requests
all tiles. Legacy far grass, volume/brick receivers, static split-sun caching,
part wind and baked animation explicitly retain all shadow work until their
providers are registered. Mask creation is current-frame work; sparse shared
maps cannot reuse old demand. Camera, headlamp and distant-cache culling
remain on their existing policies.

`receiver_shadow_demand_gpu --out FRESH_DIRECTORY` checks requested shadow
texels bit-for-bit against an independent full-shadow renderer, plus final
native images. Cases include first frame, wind, camera cut and four views,
real alpha holes, transparent and NO_SHADOW receivers, and an offscreen
blocker. The tiny fixture is a correctness gate, not a performance benchmark.

## Evidence location

`C:/dev/perf/warp-sideways-20261026/r49-lod-shadow-demand` contains immutable
candidate manifests, build/test logs and controlled output. Final GPU results
and visual acceptance are recorded below after inspection; implementation
or successful compilation alone is not evidence of appearance or speed.

### Corrected optical matrix, candidate C

`optical-c/report.json` and `optical-analysis-c/summary.json` retain 144 view
measurements (72 hard/smooth pairs), three wind poses and three lighting states.
The fixture now updates the actual sky texture and spherical harmonics together;
candidate A's low-sun/night images had retained the constructor's daylight sky
and are not valid evidence for those lighting states. Source exposure, geometry
and wind identity are held constant between representations.

At the 56 m, 4-pixel noon boundary with wind pose 0, smoothing changes coverage
error from +10.86% to +6.86%, conditional linear-HDR luma error from +11.42% to
+8.11%, and foreground-union RGB error from 10.683 to 8.462 / 255. This patch
occupies only about 36–38 fractionally covered source pixels, so these are local
optical measurements, not whole-game image-quality percentages. At low sun the
same view improves from +29.62% to +17.10% luma error, still unacceptable. The
8 m view has a 27.30-pixel finest-cell bound and remains visibly blurred even
at mip zero. Retaining source geometry nearby remains necessary.

Only rays crossing the quarter-octave band fetch both optical levels. In the
representative far view that is 438 blended segments and 1,320 integration
cell reads versus 740 for hard selection. These counters exclude ancestor
emptiness-certificate reads. The projected-error diagnostic itself still runs
outside the blend band. Neither fixture marks optical parity or production
replacement accepted.

The first candidate C camera sweep preserved its failed endpoint assertion:
at 35 m, 102 intersections already selected mip 1. The revised sweep starts
at 32 m and has a stronger whole-field-corner CPU bound; its final evidence
must independently confirm a full fine-to-coarse transition. A two-distance
bracket alone is not a measure of temporal popping.

### Receiver experiment B: exact fixture, no full-scene saving

`receiver-b/report.json` has bit-identical requested depth texels and final
native images in first-frame, wind, camera-cut and four-view cases. Submitted
shadow triangles fall by 16.7–37.6% in this small sparse fixture. Candidate A's
failed output is retained: positive map selectors also altered alpha-texture
mip gradients; negative selectors now have an actual shader regression.

`live-demand-b-resumed/report.json` contains completed OFF/ON/ON/OFF four-player
measurements, with an analysis-repair gap after the original OFF. The completed
OFF was revalidated and reused, not rerendered. Output is 3840×2160, DLSS Quality
from 1277×717 to 1916×1076 per panel, 127 clean samples per scene per run.

| Scene | OFF, mean of run medians | ON, mean of run medians |
| --- | ---: | ---: |
| Sunrise | 44.5691 ms | 45.7855 ms |
| Noon | 37.9484 ms | 38.9530 ms |

All 8,192 tiles are requested; fallback flags are zero and zero casters are
rejected. The builder alone costs about 0.56–0.57 ms. Shadow selections remain
exact, but all 16 strict preview comparisons fail, including same-mode repeats.
Visual inspection finds no broad ON-specific missing geometry or shadow; that
does not excuse the failures or establish parity. This experiment is slower
and remains off. No full-scene performance benefit is claimed.

The whole route is one terrain owner and terrain chunks are 64×64 m; a
frustum-retained owner's complete sphere can cover every shadow tile. The next
candidate requests transformed terrain cluster bounds across every resident
LOD and material. It deliberately does not drop coarse clusters, glass or
NO_SHADOW receivers. Coarse hierarchy clusters can still over-request; actual
occupancy, correctness and net timings decide whether the refinement helps.

### Complete camera sweep, candidate D

`sweep-d/report.json` passes the complete-transition and unchanged-provider
checks: 16 positions from 32–53 m, four angles, noon/low sun, one fixed wind pose,
640×360 native pixels per view. The finest reference remains 16 plants / 9,600
triangles in each view. Both variants begin entirely at mip 0 and end entirely
at mip 1. Hard/smooth raw layers are identical outside actual blending,
including the two endpoints. `sweep-analysis-d/summary.md` and hashed fixed-crop
contact sheets retain visual review; no optical acceptance is inferred.

Median absolute coverage error falls from 7.92% to 5.73%; worst remains 18.58%.
Low-sun median absolute HDR luma error barely changes, 13.11% to 13.04%, and the
worst remains 39.24%. Blending cannot repair the coarse representation's angular
lighting or silhouette bias. Source coverage is only 17–91.25 fractional pixels;
these are local comparisons, not quality percentages for the full screen.

Within the transition, optical integration reads reach 2.758× hard selection
and traversal steps increase by up to 6.31%. Outside it no second optical mip
is fetched. One sample per sweep position is diagnostic, not a stable timing
benchmark. The source/volume selector needs optical and cost acceptance as
well as its projected-cell gate before deployment.

### Receiver D: correctness passes, builder performance rejected

`receiver-d/report.json` passes all five cases, now including a kilometre-wide
owner split into actual small geometry clusters and an intentionally full-map
case. Requested raw depth and final native images remain exact. Partial cases
remove 16.7–37.5% of submitted shadow triangles in this sparse fixture. The full
case retains all casters and skips all projection/OR queries.

In `live-demand-d/report.json`, however, every map remains full. Across all four
matched runs, actual solar caster hashes and counts stay equal, with zero
rejection. Refinement examines 19,336 cluster bounds from 1,403 terrain owners.
Repeated projected-tile gathering is pathological: the demand pass itself is
89.464 ms at sunrise / 92.106 ms at noon. Full intervals are 44.360→133.248 ms
and 38.027→129.902 ms. These are two-run median means, 127 clean samples per
section/run, the same 4K-output/four-player/DLSS Quality setup as B. This is a
clear rejected regression; it is not enabled by default.

The subsequent repair stops redundant gathering once a map has complete
demand. A tile's first atomic zero-to-one transition alone increments its live
occupancy. Refinement may stop only after every relevant camera-map bit has
complete demand. Final publish still occurs after all gathering and reductions.
Its correctness and bounded full-scene cost require separate evidence below.

### Final receiver E result and disposition

`receiver-e/report.json` passes all five native cases with exact requested
depth texels and final images. `live-demand-e/report.json` is a complete fresh
OFF/ON/ON/OFF sequence, 127 clean samples per scene/run, no missing samples:

| Scene | OFF interval | ON interval | Demand pass alone |
| --- | ---: | ---: | ---: |
| Sunrise | 44.465 ms | 47.175 ms | 2.818 ms |
| Noon | 37.972 ms | 40.837 ms | 2.838 ms |

Each value is a mean of two run medians. Output remains 4K/four players/DLSS
Quality. The early completion repair removes D's pathological cost, but this
is still 2.71–2.86 ms additional full-frame cost, with no raster saving. Every
one of the 40 ON map snapshots requests all 8,192 tiles, fallback zero. The
caster fastpath retains 534,719 clusters at sunrise / 58,236 at noon, with zero
projection/OR tests or rejection. Solar selection hashes and triangle counts
match OFF exactly. Gathering examines 16,353–17,437 clusters; variation is
expected because threads stop after shared occupancy becomes complete.

All 384 input-state hashes match across runs, including captured geometry,
camera, wind and physical lighting. Strict preview parity fails all 16 pairs,
including OFF/OFF and ON/ON. Repeat differences do not establish acceptance.
**Receiver demand and smooth brick mips remain default-off. No live source
replacement, density reduction, shadow-resolution reduction or 8.33 ms claim
is made.** Better receiver selection must use actual visible geometry/depth
and register additional receiver support, rather than expanding this all-LOD
bound-gathering path further. Even accurate requests may still cover most of
some views' maps; future savings must be measured, not assumed.

Final validation: 12 receiver CPU/WGSL tests, 3 receiver-fixture CPU tests,
12 meadow-fixture CPU tests and 20 external harness tests pass. Earlier full
render/game test results are retained in the same artifact directory; they
do not substitute for the final focused GPU checks. Candidate E pins 278
runtime source files, with zero current mismatches at final verification.
Its game SHA256 is
`39ccb5e54d2b361a547ec205ce9b82c1107d35a8ebd8e9b06584ddc46ce05094`.
No interactive playtest, commit or push was performed. Controlled GPU
processes exited. Existing failed artifacts and the dirty reorganization
remain intact.

### Remaining structural cost: actual forest/group replacement

The tested game is not using automatic voxel trees:
`SIDEWAYS_VOXEL_TREE_PATCH=0` and final-state reports no experimental volume.
319,179 owners use three shared alpha-card CLOD assets. The noon capture selects
55,045,692 tree triangles across four independent views; the elevated view
alone selects 121,319 owners and 31,828,972 triangles, about 262 triangles/tree,
for 477,035 depth-winning tree pixels. This is not duplicate source-plus-voxel
rendering. Current CLOD simplifies, but its terminal meshes still contain
137/165/177 triangles per tree.

The next large cost lever is a world-group volume/aggregate hierarchy that
removes source owners before cluster traversal/raster when its image error
and work budget permit. Stable membership, matching coverage and directional
lighting, current wind, source fallback and independent shadow selection are
required. The old fixed 1–256-owner voxel experiment is not that system. The
same group-selection mechanism should serve grass and tree crowns; per-root
far-grass quads still cost about 5 ms in these captures despite low triangle
counts. Receiver culling alone cannot close a 38–44 ms baseline to 8.33 ms.
