# Vegetation source LOD and volume work — r32

25 October 2026. Two changes pass the bounded native vegetation fixtures:
intermediate source geometry for closed blades/needles, and corrected empty-space
skipping at the selected voxel mip ceiling. The finest source remains unchanged.
This is progress on the shared procedural vegetation path, **not acceptance of
the full forest, source/voxel optical equivalence, or the four-player 120 FPS
target**. The [r31 distance report](vegetation-distance-budget.md) remains the
historical baseline.

## Source-derived intermediate geometry

[`vegetation_clod.rs`](../../crates/render/src/vegetation_clod.rs) recognizes the
closed triangular-ring sweeps used by the procedural blade and needle recipes.
Coarser levels remove interior rings, retaining the root cap, tip, material and
primitive placement. They do not remove plants, needles or branch members.
The original finest geometry, normals, material bindings and rest poses remain
available; unknown topology and textured primitives retain their existing path.

Each level has a conservative two-way surface-distance bound, including the
residual of the adaptive triangle cover. A longitudinal detail floor delays
simplification while face shading is still readily visible. Existing instance
transform bounds convert error to world space; the ordinary projected-error
selector chooses the level separately for each view. `Lighting.lod_pixels = 0`
forces the finest source, while the final sweep uses `1.0`.

A thirteen-direction projected-area guard rejects reductions outside ±10%.
That guard sums projected triangle areas; it does **not** prove equal visible
coverage or lighting. The GPU image tests below supply an additional, bounded
check. Coarse face normals are recomputed. Source geometry and its voxel asset
still share the branch hierarchy, current/previous wind poses and representation
decision. The existing source-shadow policy is retained.

The six-plant fixture contains three grass tufts and three conifer twigs:

| Quantity | r32 value | Meaning |
| --- | ---: | --- |
| Finest shared triangles | 354 | Unique source primitive geometry, before placement duplication |
| Finest shared vertices | 1,062 | Unique referenced vertices of the finest clusters |
| Resident shared vertices | 2,052 | All resident source LOD levels, including the finest |
| Shared geometry bytes | 79,808 | Resident geometry including the additional LODs |
| Primitive placements | 315 | Actual leaf/blade/needle instances; unchanged |
| Pose nodes | 30 | Shared branch hierarchy; unchanged |
| Volume assets / instances | 9 / 27 | Shared packed assets and attached branch volumes |
| Packed volume bytes | 360,420 | Separate from source geometry storage |

More resident geometry buys less selected raster work. These byte counts are
different payload categories, not a complete GPU memory or allocation total.

## Actual selected triangles by distance

The final sweep passed all 23 configurations. The table counts vegetation colour
triangles read from the actual GPU selection, excluding the fixture floor.
Distances are camera-path parameters, not universal LOD thresholds. FOV is 50°.
The four views are identical here to isolate the effect of viewport resolution;
the fixture also checks a mixed-distance four-view case.

| Distance | One native 3840×2160 view | One 1920×1080 view in split screen | Four-view sum |
| --- | ---: | ---: | ---: |
| 5 m | 6,960 | 6,960 | 27,840 |
| 15 m | 6,960 | 5,808 | 23,232 |
| 30 m | 5,808 | 5,232 | 20,928 |
| 50 m | 5,298 | 4,716 | 18,864 |
| 75 m | 5,082 | 4,512 | 18,048 |
| 100 m | 4,710 | 4,332 | 17,328 |
| 150 m | 4,512 | 4,152 | 16,608 |
| 250 m | 4,152 | 2,040 | 8,160 |
| 500 m | 1,760 | 0 | 0 |
| 1,000 m | 0 | 0 | 0 |

At 150 m, one 1080p view selects 40.3% fewer triangles than the 6,960-triangle
finest source; one 4K view selects 35.2% fewer. This is a geometry-work result,
not a measured gameplay speedup. Zero colour triangles means the selected plants
use the volume path: rays, sparse-cell queries and source shadow work still cost
GPU time. Volumes are raymarched, not rasterized as “box triangles.”

## Per-plant source image validation

All 72 source-only image comparisons passed: six distances (5, 15, 30, 50, 100
and 150 m), two orientations, three held wind poses and native 1080p/4K views.
The reference forces the finest source. Visibility-owner masks identify each
plant separately; RGB error is measured inside the union of its two masks,
dilated by two pixels. Empty background cannot dilute the plant's error.
Four-view scheduling is checked separately from these single-view optical tests.

- All twelve 5 m image pairs are byte-identical.
- For reference coverage of at least 100 pixels, the largest observed coverage
  loss is 3.47%, and the largest mean absolute RGB error is 0.651 on a 0–255
  scale. Acceptance permits 10% coverage change and mean RGB error at most 2.
- Smaller resolved plants use a three-pixel coverage tolerance and RGB error
  ceiling of 5. One 12-pixel plant becomes 9 pixels: three pixels is still a
  25% relative loss at that size. Plants below eight reference pixels are
  outside the resolved-plant optical gate.

These checks cover one fixture, sun direction and material setup, with three
stationary wind poses. They do not establish continuous-motion stability,
day/night equivalence or optical parity between source geometry and voxels.

## Correct empty-space skipping at the voxel mip ceiling

The volume integrator previously tested an upper mip even after reconstruction
had reached its selected maximum mip. Its next-LOD boundary could already lie
behind the current sample, causing repeated checks and preventing useful empty
skips. The corrected path returns the current empty-brick result once that
selected ceiling is reached. Below the ceiling it still checks both mip
supports, since the upper level can contribute before the next brick boundary.

This correction is enabled by default. `KERN_VOXEL_EMPTY_CEILING=0` retains the
baseline for comparison. It does not increase extinction, recolour the plants,
drop primitive placements or lower the selected volume resolution. Skipping can
nevertheless change subsequent sample positions slightly, so bit equality is
tested rather than assumed.

### Controlled ABBA result

RTX 5070 Ti; native 3840×2160 output with four 1920×1080 views; the same six
plants; wind strength 0.8 at time 1.9. Runs are `base-a`, `ceiling-b`,
`ceiling-c`, `base-d`. Each run/distance has three warmup frames and eight
measured frames. The values below pool the sixteen actual per-frame GPU pass
sums per condition, then take their median; they are not averages of run
medians. Blocking diagnostic readbacks and synchronous CPU wall time are excluded.
Both conditions use `source_error_pixels=0.0` to isolate the empty-skip change
from source CLOD.

| Distance | Baseline GPU pass sum, median | Corrected GPU pass sum, median | Reduction |
| --- | ---: | ---: | ---: |
| 250 m | 6.154720 ms | 4.530528 ms | 26.4% |
| 500 m | 5.730256 ms | 3.514080 ms | 38.7% |
| 1,000 m | 1.778864 ms | 0.940048 ms | 47.2% |

These are sums of instrumented GPU passes for a small diagnostic scene, **not
whole-game frame times or FPS**. They cannot be extrapolated to a full forest or
added to the source-triangle reductions. Even the corrected 250 m fixture still
uses 4.53 ms of measured GPU work for six plants across four views.

The baseline/corrected four-view 4K captures at 250, 500 and 1,000 m are
byte-identical. In the separate mixed-distance acceptance image, two pixels
change by at most one 8-bit channel value. Source-reference images remain exact.
A camera cut from far volumes to four close source-only views again matches the
independently expanded source, with zero raw volume output, traced rays, active
tiles and scheduled pixels. Current/previous poses, attachment transforms,
volume inverses and conservative bounds pass their numeric checks.

An additional r32 variant with smaller private ray caches was measured and removed because its
performance did not justify retaining it. That rejected experiment is separate
from the earlier r30 pixel-compaction experiment. Neither is an enabled runtime
optimization.

## Evidence and validation

Evidence root: `C:/dev/perf/warp-sideways-20261025/r32-vegetation-optimization/`.

The real Sideways `unified-vegetation.toml` scenario also completed all 120
presented frames at native 1600×900, with 23 grass plants and two saplings through
the same code. Four JPEG captures and the profile are in `stage-auto/`.
Physics remains 120 Hz/four substeps. This is an integration preview; the sparse
recipe and source/volume optical transition are still unaccepted. The normal
one-player 4K-output/DLSS Quality playtest was restored with the prototype off;
its verified process identity is in `restored-playtest.json`.

| Evidence within that directory | What it establishes |
| --- | --- |
| `validation.json` | Pooled ABBA medians, image differences, optical summary and source/executable hashes |
| `base-a/report.json`, `ceiling-b/report.json`, `ceiling-c/report.json`, `base-d/report.json` | Raw per-frame GPU pass samples and controlled-run settings |
| `sweep-final/report.json` | 23 passing configurations, actual per-view selections and work counters |
| `source-visual-final/source-lod-visual.json` | 72 passing image cases, per-plant masks/metrics and explicit optical gates |
| `acceptance-final/report.json` | Source oracle, GPU poses, bounds, history and source-only camera-cut checks |
| `render-tests-final.log` | 283 renderer tests passed; one GPU test ignored |
| `workspace-check.log` | Workspace check passed |
| `patch-budget-test.log` | One targeted Sideways procedural-patch test passed; 142 other tests filtered out |
| `build-final.log` | Release build completed; compilation alone is not a playtest |

The shared procedural path remains a bounded native prototype applied to a
mixed grass/sapling patch. General forest residency, coarser assembly hierarchy,
source/voxel optical parity and full-route four-view performance remain open.
Branch-pose history exists, but motion-vector correctness across representation
changes and DLSS support for this path are not accepted. Growth is still a set
of cooked recipe parameters, not biological growth simulation. This report does
not promote the prototype to the default full-forest renderer or claim that the
4K/four-player/120 FPS target has been reached.
