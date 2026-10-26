# Forest fullness and current-frame culling, r38

Limited acceptance: renderer and tree-source CPU tests, static native6/6 and
four-view2/2 pixel parity pass. A matched three-camera DLSS run records lower
GPU time with current-frame culling. Changing-wind parity fails at one pixel in
frame22 in both repeated runs; its cause remains unresolved. At the user's
request, the existing tested pre-diagnostic release is running with that known
limit. This is not full temporal, all-route or 4K/four-player/120FPS acceptance.

Evidence directory:
`C:/dev/perf/warp-sideways-20261025/r38-forest-occlusion`.

## Two separate changes

The Sideways source change increases crown coverage without adding trees or
triangles. `SIDEWAYS_TREE_FULLNESS` defaults to 1.0, accepts finite values in
1.0..1.35, and selects 1.25 in this candidate. Each of the 136 connected needle
fronds expands around its authored stem anchor. Each frond has eight vertices
and eight triangles. Anchor bits, UVs, winding and wood positions remain fixed.
The existing conifer taper uses its original reference height, so larger leaf
geometry cannot inadvertently retaper trunks or move attachment points.

All three source archetypes retain 1088 crown triangles and total finest-source
counts 4117 / 3943 / 2969. Alpha textures and cutoff are unchanged. Positive uniform
frond scaling preserves its normals; the existing later warp still applies its
normal transform. Bounds and CLOD are rebuilt from the constructed source.
Near and distant source rendering share that geometry. Fuller source coverage
can naturally change shadows and visible overlap; it is an art change, not a
pixel-preserving optimization or a voxel-tree implementation.

The independent engine change, `KERN_TREE_OCCLUSION=1`, attempts to avoid drawing
trees completely hidden by geometry already rendered in the **current frame**.
It defaults off. It requires cluster-bucket rendering; an incompatible setting
reports `enabled=false` with `disabled_reason=cluster_buckets_required` instead
of pretending the requested mode ran. Source fullness and occlusion are
constructor settings, not live menu controls.

The regular development launcher now selects fullness 1.25 and culling on,
with explicit parameter/environment overrides. This launcher choice does not
change the underlying engine's opt-in default. Actual launch evidence follows.

## Current playtest identity

The root verified the requested playtest responsive and presenting its first
visible scene frame: PID267312, start UTC ticks639285453706207673, executable
`games/sideways/build/dev-runs/20261025T171609647Z-r38-play/sideways.exe` relative
to the repository root. [Runmanifest](../../games/sideways/build/dev-runs/20261025T171609647Z-r38-play/iteration.json)
records SHA256 `efb28754f4223aa1fb9258944308467b375848dc29447378d7d989b12e9b5c04`,
one3840x2160 output, DLSSQuality, livewind0.6, and observed constructor values
fullness1.25/culling enabled. The first scene frame is logged at10.575s from
play entry. The original launcher returned a post-frame verification error;
direct logs and process verification established the running state without
another restart. Preserve this process during investigation.

The new source-only `KERN_TREE_OCCLUSION_REJECT=0` diagnostic and capture
`rejection_enabled` metadata are **unbuilt/unvalidated and absent from this
executable**. They retain both draw phases and disable only Hi-Z rejection to
separate phase-order effects from incorrect rejection. No depth bias or pixel
tolerance change has been made.

## Current-frame two-phase visibility

For each camera, the normal main visibility/raster pass becomes:

1. Cull and rasterize non-tree geometry plus trees whose conservative sphere
   reaches within 32 m of the camera. The sphere includes the wind envelope.
   Tree identity uses the explicit geometry category, not a foliage/material
   guess. Source alpha testing remains active, so holes are real depth holes.
2. Build a minimum reversed-depth pyramid from this phase's depth buffer.
3. Cull remaining trees against that pyramid, first at the instance bound and
   then at surviving selected-cluster bounds. Rasterize surviving clusters into
   the same colour/visibility and depth targets.

The phases partition owners exactly. Phase2 appends visible records after the
retained phase1 prefix, preserving IDs already written into the visibility
buffer. Only the new range is scattered into phase2 draw buckets. Submitted
triangle and vertex totals accumulate across both passes. Immutable phase
uniforms and an ordered GPU preparation pass avoid a later CPU buffer write
changing the meaning of an earlier queued draw.

The occlusion test projects eight corners of the enlarged bound, conservatively
includes a pixel margin and compares its nearest reversed depth with **every**
covered texel at the chosen mip. A zero-depth sky or alpha opening prevents
rejection. Invalid projection, near-plane crossings and unproven cases retain
the geometry. The strict comparison includes a depth safety margin. Odd mip
dimensions use integer base-pixel shifts and clamping consistent with the
pyramid's edge reduction; proportional mip UVs are not substituted.

There is no previous-frame depth/history dependency or CPU visibility readback
in normal rendering. Each view owns a reusable pyramid at its actual input
resolution. Phase2 trees do not occlude one another in this implementation;
only phase1 depth supplies the evidence. This deliberately leaves some hidden
work in large distant forests. Shadow maps, foliage shadow pages, GI inputs,
physics and other cameras retain their independent caster/source data.

The R32F mip chain costs approximately 16/3 bytes per input pixel, excluding
allocator overhead and existing depth/visibility targets. At 2560x1440 this is
about 18.75 MiB for one additional pyramid; four native diagnostic views near 640x360
total about 4.66 MiB with the actual 720p gutters. Ordinary render targets, the existing meadow depth pyramid,
source buffers and all other resources are additional. No total VRAM claim is
made. A depth copy/reduction and two culling phases can cost more than they save
in an open view; only the matched GPU run can establish the local tradeoff.

## Diagnostics and accounting

`FrameStats.tree_occlusion` reports the constructed mode.
`Renderer::debug_tree_occlusion` is an explicit blocking capture API, not a
per-frame gameplay probe. Its snapshot includes:

- Frame ID, render sequence, view index, enabled/disabled reason and 32 m split.
- Disjoint phase1/phase2 submitted cluster counts.
- Instance tests/rejections and cluster tests/rejections.

Instance rejection avoids cluster traversal. Cluster counts therefore describe
the surviving owners that reached that stage, not the same denominator as
instance counts. They are not shaded pixels, saved triangles or distinct world
objects across views. Phase1 plus phase2 must equal the retained selected list.

Captures record actual `tree_source` construction values/archetype counts and
`tree_occlusion.views`. Geometry category snapshots remain available alongside
them. The source inventory is expected to match between same-fullness off/on
runs; the submitted visible work is allowed to decrease. No counter is inferred
from an environment variable.

The enabled timing scopes are `tree occluder cull`, `tree occluder raster`,
`tree occlusion pyramid`, `tree remaining cull` and `tree remaining raster`.
They replace the reference main `cull`/`raster` work. Use the whole instrumented
GPU interval and individual passes; do not compare only one of the new phases
with the old complete pass. CPU wall time remains separate.

## CPU validation completed

- `tests-render.log`: 326 render tests passed, 0 failed, 1 ignored GPU oracle.
  This includes phase partition, stable visibility IDs, counter ABI, composed
  shader validation and conservative wind/near-plane/alpha-hole behavior.
- `tests-crowns.log`: 8 tree-source tests passed. They verify bounded settings,
  exact stems/wood/UVs/counts, rebuilt source bounds and increased alpha union
  across 48 orthographic archetype/view cases. The coverage test
  uses the same baseline crown hull and source alpha cutoff 0.42. It is a
  finest-source CPU test, not GPU/LOD/lighting parity.
- `test_forest_evidence.py`: 7 evidence-tool tests passed. All three scenarios
  prepared without game execution; the unlaunched PowerShell script parsed.
- `build-release.log` and `build-timing.json`: release build succeeded, exit 0,
  in 65.914 s wall time (Cargo reports 1m 05s). Compilation is separate from
  scene construction, GPU timings and first-frame presentation.

## GPU acceptance commands and results

Use the bundled Python runtime with Pillow, from the evidence directory.
Run serially only after the parent has exclusive GPU access:

```powershell
python run-forest-comparison.py --mode parity --out parity-a
python run-forest-comparison.py --mode split-parity --out split-parity-b
python run-forest-comparison.py --mode performance --out performance-b
```

These output directories already contain evidence; use fresh names to rerun.

`--prepare-only` prints and records the plan without launching. The scripts
never build or stop an existing playtest; timeout handling affects only their
own child. The launch script separately verifies passed reports and matching
release SHA, refuses to run alongside a game, and checks construction logs plus
the first presented frame. Automated gates do not replace a separate image review.

Native parity uses the same candidate executable and fullness 1.25, toggling
only culling. Windowed 1280x720 is checked against actual dimensions. Full PNG
captures 1/32/33/64/65/96 cover first frames, settled frames and camera cuts among
the verified forest/holes, cliff and road-vista cameras. Four-view parity uses
the same cameras, with the forest repeated. At 1280x720 output, the engine's 3px
gutters produce view sizes 638x358, 639x358, 638x359 and 639x359, in that order.
The evidence runner validates those exact rects and native input sizes. Pixel bytes must
match exactly. Missing or unsupported observed modes fail, as do stale counters
or changed source inventories. At least one actual rejection is required to
prove the test exercised culling.

The performance sequence is exact staged r37, new fuller source with culling
off, and the same new source with culling on. The first difference includes the
art change; the second isolates the culling path. All use one 4K output view,
DLSS Quality, seed1/3km, frozen tick6000 and wind0.8/time1.9. Three 64-frame
sections with 24 warmup frames and an excluded capture require 39 clean matched
GPU samples each. Captured poses and source hashes are recorded in
`camera-provenance.json`; this is not an old controller/car trajectory replay.

`parity-a/report.json` passes all six single-view captures: decoded RGBA is
exactly equal with culling off/on, with zero changed pixels and zero maximum
channel error. This includes first frames and the two hard camera cuts.
`split-parity-b/report.json` passes both full-output captures at frames 1 and 32
with the same exact equality across the four views, including the odd 639x359
view. Actual rejection counters are nonzero; the comparison exercised the path.

The original `split-parity-a` stopped at an evidence-check assumption of
640x360 views that ignored the gutters. Its existing off capture was separately
CPU-revalidated in `split-parity-a-off-revalidation.json`; the original files
remain intact. The complete off/on GPU acceptance is **split-parity-b**.
`performance-a` failed on a missing DLSS runtime DLL before its first frame and
supplies no performance result. **performance-b** completed with actual DLSS
Quality evaluation and no native fallback.

### Measured GPU work

The following are arithmetic means of the covered GPU timestamp interval in
`performance-b/report.json`, in milliseconds. Each cell has 39 clean GPU samples
and zero missing samples for that camera, after the section's warmup and capture
exclusion. Output is 3840x2160, one view; DLSS input is 2560x1440. CPU frame-body
time is recorded separately in the report and is not added to GPU time.

| Camera | Staged r37, fullness 1.0 | Fullness 1.25, culling off | Fullness 1.25, culling on | Same-source off minus on |
|---|---:|---:|---:|---:|
| Forest / alpha openings | 24.799 | 24.784 | 23.974 | 0.810 |
| Terrain-occluded cliff | 16.280 | 17.165 | 10.756 | 6.409 |
| Open road vista | 33.317 | 34.821 | 27.858 | 6.963 |

Both fuller-source runs use executable SHA256
`efb28754f4223aa1fb9258944308467b375848dc29447378d7d989b12e9b5c04`;
the staged r37 reference is
`99e02645b74f4c84eb1bd22d1c74c8969b3ea0e89abf6b1de826d3058fba884f`.
Only the same-executable fuller-source off/on comparison isolates culling.
The r37 difference also includes the art change and its resulting CLOD, overlap,
shadow and meadow-depth effects.

Total main raster time, summing the two raster phases when enabled, changes
from 6.817 to 5.714 ms, 5.760 to 0.056 ms and 8.265 to 2.026 ms for those cameras.
The added pyramid costs 0.113 / 0.115 / 0.124 ms. Combined main culling costs
1.019 to 1.264 ms, 0.745 to 0.717 ms and 1.387 to 1.043 ms: culling overhead
can increase even when the full interval improves. These pass figures describe
components of the measured interval, not additive category costs or game FPS.

The finest source triangle counts remain constant, but fuller crowns change
the geometry selected by rebuilt CLOD. The captured main-visibility **tree**
triangle counts are therefore not constant:

| Camera | r37 selected tree triangles, millions | Fullness 1.25, off | Fullness 1.25, on |
|---|---:|---:|---:|
| Forest / alpha openings | 46.35 | 52.13 | 42.44 |
| Terrain-occluded cliff | 39.47 | 43.94 | 0.00 |
| Open road vista | 58.39 | 66.58 | 15.69 |

These are selected/submitted triangle counts at captures 64/128/192, not unique
source triangles, fragments, shadow work or a reduction in planted trees.
Same-source depth-winning tree pixel counts are unchanged off/on:
1,323,471 / 0 / 716,912 respectively. The cliff case had no visible tree pixels
before culling despite submitting 43.94 million tree triangles. Full-source
inventory matches within each off/on pair, while retained phase1 plus phase2
counts match the combined selected list.

### Preview review and remaining checks

The **root agent's reported visual review**, not a separate review by this
document editor, covered all three
`performance-b/full125-cull-on/captures/frame-000064.jpg`,
`frame-000128.jpg`, `frame-000192.jpg` previews and the prior forest preview.
The root observed fuller crowns, retained alpha openings, and no obvious missing
tree or terrain geometry. Forest patchiness, grass edges and pre-existing
lighting issues remain. These 1600x900 JPEG previews are an appearance check;
the native PNG reports supply exact pixel parity. The original performance
report's `visual_review_pending` field is preserved; this paragraph records the
subsequent human-visible review rather than rewriting the measured report.

**Temporal failure remains open:** `temporal-parity-b/report.json` and
`temporal-parity-c/report.json` both have matching inputs/checksums for all96
frames and31/32 exact native PNG pairs. Frame22 differs at exactly one pixel,
(217,232), maximum channel error62. Off/off repeats are exact. Visibility counts
identify a tree→terrain winner change, not just a lighting difference. Strict
`Greater` depth testing makes equal stored depth first-wins, so altered phase
ordering is a plausible explanation, but it is not proven; nondeterminism is
not established either. The exact temporal pixelgate remains failed. The
unbuilt rejection-bypass diagnostic above is the next isolation step, followed
by temporal/DLSS history validation. Static runs used wind0.8/time1.9; temporal
runs advance wind from simulation ticks and do not cover DLSS.

Latest grass-pop screenshots are under investigation; meadow aggregation,
own grass shadows and existing edge/pop issues remain open. This tree change
does not resolve them.

Timing samples are correlated and this short serial run, with concurrent CI
activity possible, has no independent confidence interval or general speedup
claim. Four-view native parity is complete, but four-player 4K performance and
the 8.33 ms / 120FPS target remain unproven.
