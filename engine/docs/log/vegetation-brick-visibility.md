# Parallel foliage brick visibility — 29 October 2026

`KERN_VOXEL_BRICK_VISIBILITY=1` is an opt-in renderer for the existing aggregate
fixture. It enables the bounded global visibility stream automatically. The
approved r4 source, material data, wind hierarchy and representation-selection
policy are unchanged. It does not switch the live Sideways forest.

## Architecture

1. `voxel_bricks.rs` builds shared local bounds from occupied 4×4×4 cells.
   Bounds include the half-cell trilinear reconstruction support. That support
   is partitioned on a common four-cell lattice: interiors do not overlap.
   Instance references contain a tree, selected level and shared region ID.
   They use the existing GPU owner pose; there are no additional wind bones.
2. GPU brick projection bins only tiles touched by the selected level. A
   per-tile bitset deduplicates branch candidates. After binning, its storage
   is reused for the reverse map from screen tile to compact pixel slots.
3. Independent brick jobs intersect pixel rays with local support bounds,
   clipping against opaque depth and the near plane. They append 16-byte
   visibility records to bounded per-pixel linked lists. This pass does not
   fetch cell materials or evaluate lighting.
4. A separate pass compacts each list and heap-sorts it front to back, with
   deterministic owner/depth ties. Integration retires completed intervals
   and stops scanning when it reaches a future entry. This replaces repeated
   whole-branch candidate scans with compact support intervals.
5. The subsequent integration pass still reconstructs directional extinction,
   material and normal moments, evaluates lighting, and combines overlapping
   media jointly. It outputs fractional colour/transmittance and motion; it
   never substitutes a false opaque first-hit depth.

The initial serial brick-intersection prototype is retained only as evidence
in `aggregate-lod/brick-stream-1`, not as the enabled implementation. It reduced
sample counts but performed millions of avoidable bounds tests per image. The
parallel producer replaces it.

## Relation to Nanite

[Epic's documented foliage architecture](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)
uses shared assemblies, compact voxel-cluster work, separate raster bins and
front-to-back depth buckets. This implementation follows the separation of
visibility work from material/lighting evaluation and retains shared poses.
The local support partitions are visibility work units, not new independently
selected assembly LOD candidates. Their count includes filtering support and
is distinct from the cooker's 128 occupied-brick limit per candidate level.

Warp still integrates a fractional medium. It does **not** implement Nanite's
opaque voxel raster/GBuffer path, virtualized cluster streaming, or voxel
shadow casting. Source triangles still cast shadows. The existing distant
source-versus-volume coverage bias is not fixed by a scheduling change.

## Budgets and fallback

- Shared bounds are 32 bytes each; each tree/level reference is 16 bytes.
  Catalog payload admission is 64 MiB. An inadmissible catalog or a scene
  without aggregate selection retains the complete original renderer and
  logs why. This is a payload bound, not a total CPU allocator bound.
- The visibility pool is 32 MiB per view, including metadata for at most
  262,144 compact pixels. A successful parallel intersection uses a temporary
  linked record and then a contiguous sorted record in that pool.
- At most 512 intervals are sorted per pixel. Allocation has a checked, bounded overshoot;
  neither capacity exhaustion nor a dense pixel wraps the allocator or
  silently drops foliage. These pixels take the complete branch fallback.
- Tile candidate capacity remains 64. Overflow uses the complete branch set.
  The deduplication mask costs `tiles × ceil(branch_instances/32) × 4` bytes
  per view and is checked against device buffer limits.
- GPU timing now reports `voxel visibility` separately from `voxel integrate`.
  Compare their sum with historical integration timings, which included the
  old visibility prepass. Record counts describe final sorted intervals;
  temporary linked records also consume pool capacity.

## Validation

CPU tests cover shared geometry, sparse/dense indirection parity, admission
failure and exactly-once reconstruction support across partition boundaries.
A GPU test executes the actual shader sort on 512 records with tied depths,
checks half-open ownership for a ray parallel to a shared region boundary,
and races 64 GPU reservations against a 16-record pool to verify overflow.
Native/temporal shader specializations are validated with the option on and
off. Final checks: 675 render tests passed, 16 ignored; the new ignored GPU
oracle was run separately and passed. Sideways: 189 passed, one ignored.
Workspace all-target checking and developer-launcher contracts passed.

## Captures and measurements

Evidence root: `C:/dev/perf/warp-sideways-20261028/aggregate-lod`.
All final fixture runs use eight measured frames after eight warmups on the
RTX 5070 Ti Vulkan adapter. Background GPU workloads are uncontrolled, so
these are diagnostic medians, not isolated benchmarks or gameplay FPS. The
fixture deliberately keeps both sun shadow maps focused on the forest.

| Run | Scene | Visibility ms | Integration ms | Covered GPU interval ms |
|---|---|---:|---:|---:|
| `brick-baseline-64` | 64 trees, one 1080p view, TAA off; original branch stream | 2.969 | 82.353 | 112.116 |
| `brick-parallel-64` | First parallel allocator, rejected | 188.559 | 20.162 | 236.001 |
| `brick-atomic-64` | Final parallel brick visibility | 0.642 | 20.767 | 49.206 |
| `brick-atomic-four` | Four trees, four 960x540 views, TAA off; summed scopes | 0.495 | 31.457 | 64.225 |
| `brick-atomic-one` | One tree, 1,000 m, stationary TAA | 0.126 | 5.964 | 10.263 |

The initial global compare-exchange allocator caused severe contention. The
final allocator uses checked atomic addition with a proven bounded overshoot
and complete fallback. No stream or step overflows occurred in the final
fixtures. The 64-tree run has 22 tile candidate overflows, handled completely.
Do not benchmark the rejected allocator or add visibility twice to old timings.

Matched 64-tree camera/shadow submissions are identical: 29,186 camera and
177,593,316 shadow triangles. Bounds tests fall from 42,401,827 to 2,188,121
(-94.8%), integration steps from 989,002 to 437,099, and volume reconstruction
calls from 4,747,443 to 888,103 (-81.3%). Occupied trilinear corner reads rise
from 1,966,032 to 2,272,607 because interval boundaries change quadrature steps;
this is not a claim that every kind of memory access decreased.

Exported 8-bit native opacity PNGs compare at fuzzy IoU 0.999627, relative L1
0.000373, and coverage ratio 1.000009; maximum difference is 2/255. The final
single-tree parallel opacity PNG is identical to the serial brick prototype.
These comparisons establish renderer consistency, not source optical accuracy:
at 1,000 m source coverage is 71.000 versus 79.780 for the volume (+12.37%).
At 50 m the pre-existing source cut retains 30,561 versus 31,649 native coverage.
Neither source-versus-LOD issue is repaired by visibility scheduling.

Catalog payload is 352,360 bytes for one tree, 703,240 for four and 7,720,840
for 64. Bounds and volume payloads are shared; instance references grow with
population. The existing 4,096 branch-selection ceiling still applies.

Review [actual GPU source and forest crops](../assets/brick-visibility-review.png)
and [machine-readable comparison](../evidence/brick-visibility-review.json).
The top pair uses the same 50 m camera and TAA phase; bottom images are identical
2.5x nearest-neighbour crops of the native 64-tree captures. No generated tree
imagery is used. Moving-camera/DLSS transitions and voxel shadow casting remain
unvalidated/unimplemented respectively. Integration still dominates the new
volume path; separate, cheaper material/lighting resolve is the next boundary.
