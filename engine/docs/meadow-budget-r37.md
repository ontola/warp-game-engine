# Meadow geometry budget, r37

Status: accepted for interactive iteration after focused CPU checks, indexed
image parity exercising all three LODs and the matched r36/r37 comparison.
Results below describe a short one-player A/B, not completion of the four-player
target. The new playtest reached its first visible scene frame after10.056s;
its process identity and responsiveness were checked.

Manifest: `../../games/sideways/build/dev-runs/20261025T155253489Z-r37-play/iteration.json`.
PID236708, startUTCticks639285403742534550, executableSHA256
`99e02645b74f4c84eb1bd22d1c74c8969b3ea0e89abf6b1de826d3058fba884f`.
One player,3840x2160output,DLSSQuality,livewind0.6. These identify this launch,
not an indefinitely running process; verify active.json before stopping it.

## Changes

The r36 field remains 360 authored ribbon blades per 0.45 m world cell, with
three shared variants and four wind groups. The accepted source cuts remain
1800 / 1080 / 600 triangles per patch. No blade, accepted cut, coverage guard,
terrain support rule, material palette or projected-error threshold was removed
to reduce the counts. GPU placement, culling, anchored wind, static road-edge
filtering and the 80..100 m outer fade remain.

`vegetation_clod::RibbonErrorBound` retains the directions of source-to-cut
deviations. For recognized planar ribbon strips, corresponding surface points
have the same height and relative width. Checking both edges at the union of
source/cut height stations bounds the intervening linear surfaces in both
directions. The actual blade affine transform is applied to these deviations,
including a rounding allowance for packed f32 vertices. Unsupported or twisted
geometry retains the conservative scalar/operator-bound fallback.

Previously a small deviation across a blade's width inherited the much larger
height scaling through one scalar bound. The new physical bound removes that
overestimate. It does not change geometry or relax the one-pixel selection
threshold. Uniform growth and terrain-plane shear still enlarge the error at
GPU selection; camera projection uses the conservative nearest bound distance.
The maximum across shared variants is retained, and cut errors are monotone.

The focused seed17 test measured conservative assembly-space bounds:

| Cut | Previous bound, metres | Physical bound, metres | Source triangles/patch |
| --- | ---: | ---: | ---: |
| LOD0 | 0 | 0 | 1800 |
| LOD1 | 0.012409..0.012614 | 0.00122047..0.00122080 | 1080 |
| LOD2 | 0.025661..0.026084 | 0.00415719..0.00415871 | 600 |

These are geometry error bounds, not measured screen error or a density target.
The separate physical projected-area experiment is diagnostic only; it did not
change which cuts are accepted.

## Exact indexed submission

`meadow_index.rs` builds shared u16 index streams into the original flat recipe
buffer. A representative is shared only if the required attributes match
bit-for-bit in **all three variants**. Depth keys contain position, branch and
anchor; colour keys contain the complete vertex, including normals/materials.
Triangle order and winding stay identical. Normals are not averaged. A range
that cannot be addressed with u16 is rejected rather than truncated.

Depth and colour each use their own index stream, with the same selected patch
lists and indirect counts. The indirect record has five words and is valid for
both indexed and non-indexed submission. `KERN_MEADOW_INDEXED=0` selects the
original draw path; otherwise indexing is enabled. The extra shared index
allocation for this recipe is 41,760 bytes. The original shared source vertex
payload remains 2,004,480 bytes.

The vegetation-seed0 native parity capture reports these reference opportunities:

| Cut | Flat vertex references/patch/pass | Distinct depth indices/patch | Distinct colour indices/patch |
| --- | ---: | ---: | ---: |
| LOD0 | 5400 | 2520 | 4916 |
| LOD1 | 3240 | 1800 | 3238 |
| LOD2 | 1800 | 1320 | 1800 |

Distinct indices times selected patches are **ideal vertex-reference budgets**,
not measured vertex shader invocations, hardware cache hits or elapsed time.
In particular, this recipe has almost no colour-pass index reuse in its coarse
cuts. The source geometry buffer is not deduplicated by this change.

## Count and memory scope

Capture `meadow.views` contains actual GPU-selected patch counts per LOD and
their source triangles. A source triangle is counted once; the depth and colour
passes submit it twice. These counts precede per-blade road habitat rejection,
frustum clipping and raster depth rejection. Selected patches times 360 is an
authored-blade upper bound, not a measurement of visible blades or covered area.
Main `FrameStats.triangles` does not include these dedicated meadow draws.

The field still reserves 205,209 candidates/view and 25,445,916 bytes/view for
112-byte patch records plus three compact LOD-index lists. Terrain samples,
source recipes and hardware index streams are shared across views. The captured
seed1 field's terrain payload is 50,539,536 bytes. These figures exclude render
targets, uniforms, indirect buffers, pipeline resources and allocator overhead;
they are not total VRAM. Four views require four pools, not four copies of the
shared recipe.

## Validation

Evidence root: `C:/dev/perf/warp-sideways-20261025/r37-meadow-budget`.

- `tests-meadow.log`: 21 focused meadow tests passed, including shader/ABI,
  index expansion, all variants, current/previous rigid poses, anchored wind,
  bounds, deterministic recipes and physical-error regression.
- `tests-clod-final.log`: 12 CLOD tests passed. Dense independent f64 distance
  checks cover both directions under affine transforms; unsupported topology,
  malformed cuts and projected coverage changes are rejected. The f64 oracle
  avoids the false large distances found in a thin-triangle f32 diagnostic.
- Evidence tooling: 9 Python tests passed, including actual-mode detection,
  exact RGBA comparison independent of PNG compression, resolution rejection
  and proof that `--prepare-only` cannot launch a game.

These are targeted checks, not a rerun of the entire workspace suite.

`indexed-parity-a/report.json` passed for one unchanged executable, native
1280 x 720 windowed output, seed1/3 km at frozen tick6000, wind0.8/time1.9,
two cameras and captures1/32/33/64. Every decoded RGBA byte matched, every
selected source count matched, and observed `indexed` metadata changed from
false to true. The four images exercise first frame, settled frame and camera
cut. They selected **LOD2 only**: 16,430 chase patches and 150 close-cliff
patches. This first fixture alone establishes onlyLOD2 parity.

`indexed-lod-parity-a/report.json` additionally passed four native1280x720
captures with FOV15 cameras aimed at visible grass beside the same road.
The two sections selectedLOD0/1/2 counts0/154/25 and17/89/0 respectively;
the wrapper rejects a result lacking actualLOD1 orLOD0 selection. Every
decoded RGBA byte matched indexed off/on, including first and settled frames
on both sides of the camera cut. Root inspected both resulting close images:
grass is visibly rendered, rather than hidden behind terrain. Combined with
the initial fixture this exercises all three index streams. It does not prove
moving-wind or DLSS temporal parity; poses and time are fixed in these tests.

The tested candidate executable SHA256 is
`99e02645b74f4c84eb1bd22d1c74c8969b3ea0e89abf6b1de826d3058fba884f`.

## Matched r36/r37 comparison

`run-budget-comparison.py` compares the exact staged r36 executable
`9c5e31b6bec09b1e7bba125eb52a4e189d290597c97234445736b779f79337b5`
with r37. Both use one 3840 x 2160 output view, DLSS Quality, the same two
frozen cameras, world/sun/wind and tick6000. Each section has 64 frames with
24 warmup frames and an excluded final capture. `comparison-a/comparison.json`
confirms 39 clean GPU samples per section, zero missing samples, rendered input
2560 x 1440 and reconstructed output 3840 x 2160 in both runs.
Source-count reductions and indexed execution are combined in this comparison;
it cannot assign their individual timing gains.

| Camera | r36 selected patches LOD0/1/2 | r37 selected patches LOD0/1/2 | r36/r37 source triangles |
| --- | --- | --- | ---: |
| Driving chase | 1997 / 5030 / 9338 | 0 / 77 / 16288 | 14,629,800 / 9,855,960 |
| Close cliff | 149 / 1 / 0 | 0 / 84 / 66 | 269,280 / 130,320 |

The selected patch totals are unchanged: 16,365 and 150. Nominal authored-blade
density is unchanged. Source triangles fall by 32.63% and 51.60%, respectively,
because the same patches select coarser accepted cuts.

| Camera | r36/r37 meadow render ms | r36/r37 GPU covered interval ms | r36/r37 CPU frame body ms |
| --- | ---: | ---: | ---: |
| Driving chase | 15.368 / 12.619 | 36.506 / 33.852 | 36.470 / 33.721 |
| Close cliff | 1.312 / 1.144 | 16.452 / 16.283 | 16.550 / 16.367 |

The meadow render interval covers its depth and colour passes. Placement is
separate: 0.154 / 0.143 ms in chase and 0.150 / 0.165 ms at the cliff. Composite
is also separate: 0.028 / 0.028 ms and 0.0135 / 0.0134 ms. The other major chase
scopes remain roughly unchanged: shade 7.748 / 7.829 ms, reconstruction
1.571 / 1.571 ms. Meadow rendering is still a large part of the frame.

Both pairs of 1600 x 900 JPEG previews were independently inspected by the
implementation reviewer and root; see `comparison-a/visual-review.json`.
Broad grass coverage,
colour patches and silhouettes appear preserved, with no obvious newly bare
regions. Existing square verge steps remain. This is a preview-level visual
check, not full-resolution optical parity, a continuous camera sweep or a
moving-wind temporal test. Pixel equality was deliberately not required between
r36 and r37 because they select different geometry cuts.

GPU pass intervals and first-to-last instrumented GPU interval remain separate
from CPU wall time. One short serial A/B under concurrent CI cannot establish
statistical confidence, sustained frame rate or a four-player budget. The
samples are temporally correlated; there are no independent repeats or valid
performance confidence intervals in this evidence.

## Remaining limits

This is source-ribbon rendering to 100 m, not far voxel aggregation. Grass does
not cast its own shadows. The existing one-plane-per-patch terrain support and
conservative road-boundary fallback can still produce edge steps or incorrect
root support on sharp terrain. A tighter geometric error bound does not prove
lighting/normal/silhouette parity at every LOD transition. The target remains
four players in one 4K output at 120 FPS (8.33 ms), and is not met or certified
by these one-player comparisons.

## Next simplification

The chase is already overwhelmingly at the600-triangle cut:16,288of16,365
selected patches. That still submits19.71million triangles across depth and
colour, with roughly51.21million ideal distinct vertex references. Placement
costs0.143ms, so more placement optimization does not target the main cost.

The next substantial change should test a terrain-following sparse grass
coverage/density volume for unresolved distant blades, derived from the same
source shapes, habitat, material variation and normal distributions. Near
source meshes remain. Do not simply increase the existing bounded256-group
voxel prototype's loop limits: spatial traversal and aggregation are needed.
Coverage, lighting and wind must match source renders; four wind groups with
distributed blade anchors cannot be replaced by one rigidly moving patch.
This is a proposed next experiment, not an implemented far-voxel field.
