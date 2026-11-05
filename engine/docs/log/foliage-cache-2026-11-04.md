# Bounded voxel asset cache on the normal forest route

The GPU forest now uses a persistent physical voxel cache, not just resident
flags over a full upload. Three bounded arenas hold index words, sparse cells
and splat records; logical level/splat headers stay stable. Normal creation
uploads pinned root dependencies only. GPU-generated representation requests
arrive through a two-slot asynchronous readback ring. CPU service owns asset
bytes and allocations, never a near-tree pool or camera/branch visibility list.

Default maxima: 16 MiB grid storage, 64 MiB sparse cells (176 bytes each), 4 MiB
splat records, plus level/splat metadata and two small feedback buffers. Actual
arenas use the smaller of the catalog size and configured capacity. The exposed
allocated_bytes counts the four GPU arena/metadata buffers, not total renderer
VRAM, source geometry, transient splats or CPU asset data. Spec::page_budget is
a normal engine setting; terrain_fly --voxel-cache-mib changes the cell budget
for reproducible pressure tests without selecting another renderer.

Per frame at most eight page completions and 8 MiB of upload payload/headers.
Large uploads continue over multiple frames. Grid/cells/splat bytes are ordered
before the logical headers and residency publication; incomplete pages remain
unavailable. Initialization may upload pinned ancestors outside that per-frame
budget. Too-small budgets for pinned data fail explicitly rather than exceed the
budget. The current policy pins whole fallback mip ranges, which is conservative
and can be expensive; a coarsest-only resident ancestor contract is future work.

Representation admission reserves the total cost of a complete dependency
bundle before loading its mips. Already-ready bundles take priority; an eight
frame request lease bridges delayed feedback and incomplete-cut frames. Retained
unrequested pages can be evicted for admitted requests. Memory generation and
representation generation reject stale publication/use. Root dependencies cannot
be evicted. Capacity/fragmentation failure keeps the complete root fallback;
currently it does not choose the nearest intermediate loaded subtree.

Source branches also need their adaptive voxel shadow mip ranges. Those are
included even when the camera uses source needles. Source cluster/vertex/index
storage itself remains fully resident and is NOT covered by this cache. Optional
brick-HZB/brick-visibility experiments currently require immutable physical
catalogs; the GPU forest uses its normal whole-level splat path while paging.
Other non-GPU-forest voxel callers retain their existing static upload.

No synchronous wait is introduced in the live frame. A small extra queue copy
submission collects last-frame requests. When all pages of the immutable catalog
are resident, feedback service stops until that cache is destroyed/replaced.
Missing samples retain the previous demand; stale/out-of-order publications do
not replace newer requests. This is bounded asset streaming, not GPU-only asset
IO or a completed Nanite-equivalent architecture.

## Stable appearance

Physical page offsets and transient branch slots must not seed surface detail.
GPU materialization stores a stable root-generation/branch appearance seed in
the unused volume bounds w component. Splat records carry a compact seed; local
cell coordinates identify the cell. Both raster jitter and rounded shading
normals reconstruct the same centre from that key. A previous mismatch used
physical cell/slot addresses in shading, making lighting depend on load order.

## Validation

All 116 foliage-filter checks pass (release, including serial GPU fixtures).
The GPU two-page fixture forces multi-frame uploads with a 44-byte frame budget,
eviction/reload, pinned fallback and raw grid/cell/splat byte checks. A normal
Renderer cold-start fixture observes a root fallback then complete source draws
through the asynchronous service; partial cuts may never be published. Existing
occlusion/history/population fixtures explicitly prewarm their tiny assets so
they continue testing their original invariants separately from loading latency.

Source paging, consumer-aware upstream
history/publication, animated shadow caching, unified grass and far spatial-cell
aggregates remain open architecture gates.

## Next dependency: source pages

Source paging must compose availability with these voxel dependencies before
publication. Independent source and voxel writers to the selector's resident bit
would incorrectly publish a representation when only one payload is ready. Keep
one publication/generation owner and one feedback stream; extend it to all
required payloads. Stable logical cluster IDs can retain culling metadata while
first_vert/first_tri address bounded vertex/index arenas. Non-vegetation/skinned
geometry needs explicit pinned ownership. Publish a cut only after every required
source and voxel page is uploaded; eviction must invalidate the same history and
shadow dependants. This section is a dependency plan, not implemented source paging.

## Measured normal route and pressure

Three alternating unstreamed-control/cache pairs, same executable for each
variant, alpine ground route, 180 frames at 15 m/s, native 1713x964, one view.
Discarded first 80 timestamp samples; 99 remain per run. The control has the same
corrected appearance shaders and bypasses cache construction at compile time.
The temporary source edit was restored; there is no shipped bypass switch.
Different executable hashes and nonempty cache telemetry guard the comparison.

| Median of run medians, ms | Unstreamed control | Bounded cache |
| --- | ---: | ---: |
| Timestamp-covered GPU work | 10.583 | 10.625 |
| Forest selection/residency | 0.660 | 0.657 |
| Shared poses | 0.054 | 0.053 |
| Materialize | 0.103 | 0.103 |
| Voxel splat generation | 2.693 | 2.685 |
| Voxel splat raster | 0.999 | 1.001 |
| Voxel splat resolve | 0.650 | 0.655 |

This establishes no speedup. Separate feedback-copy queue submissions and upload
work are outside these main-frame timestamp regions, so warmup/pressure service
GPU cost is not fully measured. Fully resident steady-state stops feedback copies.
CPU render intervals were [5.791, 6.628, 6.109] vs [6.680, 6.536, 6.374] ms;
these are not isolated CPU compute measurements. The 4K/four-player/8.33-ms target
is not established by this one-view scene.

Default cache: 65,908,804 bytes allocated in the four data/metadata buffers,
27 pinned pages (46,322,268 uploaded bytes) initially; all 507 pages loaded by
frame 61. Maximum frame upload 695,420 bytes, zero evictions, zero misses. A
fully loaded small catalog therefore saves no final voxel memory versus control.

48-MiB cell-budget pressure case: 62,257,508 allocated bytes across all four
buffers, 327 resident pages by frame 39, zero evictions/reload churn over 180
frames, 21,004 denied dependency-bundle admission attempts (not missing trees).
Final source/voxel slots 1208/2865 versus 1428/2625 at the default budget; this is
reduced detail with complete fallback, not a quality-equivalent memory claim.
The tiny two-page GPU fixture separately forces real eviction and reload.

Full-size RGB comparisons exclude the first 180 HUD rows: MAE 0.02365–0.02371
on the 0–255 scale, channel p99 1, maximum 19. Visual review of control/cache
shows matching coverage and lighting, not bit-identical output. Both had the
same final logical cut and zero slot/hash/frontier overflow failures.

[Per-run/pass medians, hashes, image deltas and cache evidence](foliage-cache-2026-11-04-evidence.json).
Local artifacts: `build/foliage-cache`; `after-preview.jpg` shows the actual
headless renderer output. An earlier invalid run reused the control executable
because restoring a file preserved an old timestamp; those timings are excluded.
The final build explicitly refreshed the source timestamp and checked hashes.
