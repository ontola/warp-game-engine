# Relocatable voxel page payloads

The normal GPU forest's voxel upload now cooks independent level pages and
allocates their physical index/cell ranges separately from logical level IDs.
Pages are placed small/coarse first rather than in the source catalog order.
Tree and branch level IDs remain stable. Source geometry is not relocated by
this change.

Each page preserves its original dense or brick encoding. Brick-directory
addresses, sparse-cell references and splat references are rebased together.
The normal camera/sun splat upload uses the page-produced splat table; the ray
integrator uses the same relocated grid/cell buffers. The optional brick-HZB
catalog is rebuilt from the relocated data. Material, extinction and actual
normal records retain their original bytes. No density/colour/LOD tuning.

The bounded range allocator coalesces free spans, fails without mutation when
fragmentation/capacity prevents allocation, and checks allocation generations
when freeing. A stale handle cannot free a newly reused physical span. Empty
allocations are explicit. Physical address overflow and malformed source grids
fail before publication.

This is a connected physical-address prerequisite, **not yet a streaming cache**.
The initial pool is sized for all cooked pages, and all remain resident. There
is no claim of reduced GPU memory or completed asset-residency gate. Next work:
retain cooked pages and allocation ownership, map representation dependencies
(including shadow mips), consume GPU requests asynchronously, upload/evict with
budgets, and publish residency only after all required bytes are uploaded.
Source cluster/vertex/index paging must follow the same transaction contract.
The materializer currently publishes adaptive voxel shadow choices (765) even for
camera-source branches; their dependency bundle must retain the applicable whole
mip range, not only the camera's selected mip. Root fallback ranges remain pinned.
Precise shadow-level requests can reduce that conservative dependency later.

## Address-independent appearance

The first image comparison exposed a genuine relocation dependency: splat
jitter/dither and faint-cell thinning used the physical sparse-cell index as a
random seed. Relocating correct bytes therefore changed coverage and shadows
(RGB MAE about 5.22/255). Both now seed from the local packed cell coordinate.
The existing instance component remains; stable world identity across transient
branch-slot replacement is a separate remaining concern.

A controlled baseline is built with the same corrected shaders but original
unrelocated upload, then compared with the normal relocated upload. This temporary
control is not a shipped toggle or second production route. Normal construction
is restored before building/testing the committed executable.

## Validation and measurements

All 114 foliage-filter tests pass. New tests compare every source cell against
independently relocated dense and brick pages (including partial bricks), exact
material/normal bytes, and the splat-cell oracle. Allocator checks cover full
capacity, fragmentation, coalescing, invalid frees and stale handles after reuse.
Invalid source/overflow cannot produce a partially published page.

Three alternating runs per unchanged executable, native 1713x964 one view,
80-frame alpine flight at 15 m/s, first ten GPU samples excluded:

| Median of per-run GPU medians | Control ms | Page upload ms |
| --- | ---: | ---: |
| Forest preparation | 0.682 | 0.676 |
| Timestamp-covered frame | 17.018 | 16.998 |

No demonstrated speedup/regression at this measurement precision. Scene-build
wall times vary (control 1054/1267/1138 ms; pages 989/1079/619 ms); these are not
steady rendering timings and do not prove faster cooking. The page cook currently
runs at construction and is not persisted in an on-disk cache.

Final cut counts match: 62,274 active roots, 4,090 selected records, 1,751 source
slots, 2,339 voxel slots, zero slot/hash failures. Inspected final image pairs
show no obvious missing vegetation or shadow changes. Corrected-seed control
versus relocated RGB MAE is 0.288-0.292/255 (excluding 180 HUD rows), channel p99 7.
Not pixel-identical; raw page-cell/splat correctness is asserted separately.

[Executable hashes and every per-pass/run median](foliage-page-layout-2026-11-04-evidence.json).
Artifacts: build/foliage-page-layout. No playtest opened. This still does not prove
bounded streaming, full architecture completion or 4K/four-player/8.33 ms.
