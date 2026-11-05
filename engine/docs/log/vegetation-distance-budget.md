# Vegetation work by distance — r31

**Historical baseline retained.** The [25 October r32 follow-up](vegetation-optimization-r32.md)
adds intermediate source geometry and a measured voxel empty-skip correction.
Its updated counts and timings supersede the performance diagnosis for that
bounded fixture; the r31 tables below are deliberately unchanged. Full-forest
acceptance and the four-player 120 FPS target remain open.

24 October 2026. This is an accounting result, **not acceptance of the voxel
path for the full forest**. The source geometry stays too detailed over a long
range, and the existing volume integrator remains expensive after triangles
disappear. Default production foliage is unchanged.

## Repeatable measurement

```powershell
cargo test --locked -p render --lib
cargo run --locked --release -p render --example vegetation_distance_budget -- --out FRESH_DIRECTORY
```

Run with no concurrent game. Output is JSON and CSV; no full-resolution image
readbacks or PNG encoding. The fixture uses three grass tufts and three conifer
twigs, the same six plants as `unified_vegetation_gpu`. Native output is
3840×2160, with either one 3840×2160 view or four 1920×1080 views. FOV is 50°;
wind is held at strength 0.8/time 1.9. Three warmup frames precede eight timed
frames per distance, followed by blocking counter reads outside the measured
interval. Distances are camera-path parameters; exact camera-to-target lengths
are also recorded. Four equal views isolate viewport resolution; an additional
5/50/250/1000 m configuration tests independent view choices.

Evidence: `C:/dev/perf/warp-sideways-20261024/r31-vegetation-budget/sweep-final/`.
`report.json` records all 23 configurations and per-view data;
`distances.csv` is the compact table. RTX 5070 Ti, native rendering.

## Actual counts

Vegetation colour triangles only; the fixture's floor is excluded here.

| Distance | One 4K view | Four 1080p views, total | Selected volume groups per 1080p view |
|---:|---:|---:|---:|
| 5–150 m | 6,960 | 27,840 | 0 |
| 250 m | 6,960 | 13,536 | 18 |
| 500 m | 3,008 | 0 | 27 |
| 1,000 m | 0 | 0 | 27 |
| 2,000 m | 0 | 0 | 27 |

The immutable geometry is only **354 unique triangles**, but instancing does
not make 6,960 per-view triangles become 354 draws. The six plants have 315
primitive placements and 30 branch poses. The current recipes place each blade
or needle in a small cluster with no cheaper intermediate geometry. It retains
source detail until its branch can use the experimental volume. Selection is
based on projected error, so a 4K view retains detail farther than a 1080p view.

There are **no rasterized voxel boxes**. The selected groups are ray-integrated
volumes. For one of the four 1080p views:

| Distance | Triangles | Selected mip cells¹ | Raymarch steps | Volume samples | Occupied cell reads² |
|---:|---:|---:|---:|---:|---:|
| 150 m | 6,960 | 0 | 0 | 0 | 0 |
| 250 m | 3,384 | 3,949 | 6,097 | 14,458 | 818 |
| 500 m | 0 | 2,264 | 1,810 | 5,025 | 190 |
| 1,000 m | 0 | 770 | 209 | 354 | 0 |
| 2,000 m | 0 | 257 | 0 | 0 | 0 |

¹ Nonempty cells available in the selected maximum mip, weighted by placement.
The ray can sample finer levels. This is neither visible cell count nor actual
work. Shared packed-level storage is reported separately, without multiplying
it by plant or view count. Selected groups may also miss all pixel rays.

² Actual nonempty sparse-cell records loaded during reconstruction, counting
both mips, repeated reads and zero-weight corners. Not unique voxels or hardware
memory transactions. Empty grid checks are not included in this count. Steps
include gaps/skips; volume samples include empty reconstruction results.

The four-view instrumented GPU pass sums were 0.611 ms at 150 m, 6.434 ms at
250 m, 5.882 ms at 500 m, and 1.780 ms at 1,000 m (medians, eight samples).
These are different projected workloads, not an isolated optimization A/B or
gameplay FPS. They demonstrate why zero triangles is insufficient: this tiny
scene with the volume path consumes much of the **8.33 ms whole-frame target**.
Do not extrapolate these timings linearly to a forest or call this scalable.

Tile selection also visits every resident volume per screen tile: 220,320
selection checks per 1080p view for these 27 volumes, including source-only
groups. This deterministic loop count is derived from dispatch dimensions,
separate from GPU-counted ray/AABB tests. Active tile integration does not remove
the setup scan. The current 102-volume stage patch would imply about 3.3 million
such checks across a 4K output if this same algorithm were used.

## Guards and reusable diagnostics

`vegetation_accounting::Accounting` snapshots immutable scene data and consumes
`Renderer::debug_visible_clusters` captures. Captures now include the **actual
GPU selection buffer** for that view; no independent CPU planner guesses the
LOD or loses hysteresis history. Counts distinguish source placements, unique
geometry, actual clusters/triangles, padded vertex invocations, source/volume
groups and per-mip residency. Four-view sums intentionally count four draws.

`VoxelViewDiagnostics` adds exact 64-bit totals for march iterations, volume
samples, occupied cell reads and ray/AABB tests. Per-ray local accumulation
avoids atomics in the sampling loops; low-word atomic sums with carries do not
require a GPU 64-bit-atomic feature. Instrumentation can affect kernel costs;
timings above are explicitly instrumented results.

The fixture rejects duplicate owner/cluster pairs, invalid proxy draws, colour
triangles exceeding full source, any tile/ray/active/step overflow, and remaining
source triangles at 1,000 m. It checks the frame's triangle counter against the
actual GPU-selected pairs. Six-plant recipe ceilings are 6,960 triangles,
315 primitive placements and 30 poses. These checks detect regressions;
passing them is not a performance or optical-quality acceptance.

The existing Sideways maximum-patch test now guards **31,456 full-detail
triangles per view, 1,428 primitives, 180 poses and 148 volumes** for 32 grass
plants plus four saplings. The observed smaller 23+2 patch has 21,272 source
triangles per view. Limits are reviewed recipe budgets, not a runtime feature
that silently drops plants when overloaded.

Shadow counts are separate. `shadow_cache_triangles` may count retained geometry
without drawing it again; `shadow_vertices_submitted_this_frame` identifies new
raster work. The source geometry still casts shadows after colour switches to
voxels. Current shadow totals include the floor and are not per-plant costs.

Validation: 272 renderer tests pass (one ignored); all-target workspace check
and release fixtures pass. The focused maximum-patch budget test also passes.
The mixed source/voxel GPU acceptance and camera-cut
checks pass. Five decoded images match r30 byte-for-byte, recorded in
`image-invariance.json`; the counters do not alter these fixture images.

## Consequence for the unified renderer

Before enabling this across the forest: add intermediate branch/plant geometry
aggregation, replace the all-volumes-per-tile setup with spatially bounded work,
and reduce empty reconstruction/traversal cost. Measure the same counters and
full GPU intervals after each change. Preserve silhouette, coverage, lighting,
wind and per-view selection; simply lowering draw distances or triangle counts
would not establish equivalent quality or cheaper rendering.
