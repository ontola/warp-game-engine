# GPU asset demand and publication boundary

The normal forest selector now consults logical representation-page residency
before admitting a cut. It records used/missing pages on GPU and compacts unique
requests, including publication generations, into a bounded buffer. Cameras and
shadow consumers still contribute to the same selection union. A missing page
invalidates the whole root cut; residency publishes no partial branches and the
existing resident root volume remains visible. Root representation pages are
pinned and cannot be evicted through this publication API.

`Selector::publish_asset_pages` validates an entire batch before any queue write:
IDs, duplicates, nonzero/monotonic generations and pinned-root protection. It must
be called at a frame boundary, after the associated data upload has been ordered
on the same queue. New asset generations retire affected branch slots even if
root identity and representation ID did not change. The publication epoch also
invalidates direct-root motion history conservatively. Root/world generation
and asset generation are separate; they are not collapsed into a collision-prone
hash to determine slot validity.

This is **not yet bounded physical asset streaming**. The current renderer still
uploads all source and voxel catalogs. Logical pages describe representation
bundles, whose dependencies include the shadow representation; physical content
ranges and shared dependencies still need a page allocator/address translation.
No memory saving is claimed, and the architecture's asset-residency gate remains
open. This boundary is integrated into the normal selector, not an opt-in demo.

Next physical integration must cover source vertices/cluster indices and voxel
levels, dense grids, sparse cells AND splat-cell lists. Updating only the voxel
ray integrator would leave normal splat rendering using stale physical offsets.
The upload service needs bounded asynchronous request transfer, allocation,
generation-safe eviction, immutable pinned ancestors and ordered publication.
A full-catalog allocation with availability flags alone cannot close that gate.

## Validation and measured cost

All 111 foliage-filter tests pass, including exclusive GPU fixtures. Fault tests
unpublish detail, check unique GPU requests and visible full-root fallback,
republish a newer generation, reject stale/invalid mixed batches atomically, and
verify that only affected branch slots are reassigned after asset replacement.
The standalone residency fixture now includes its synthetic representation IDs
in its catalog; its reorder/reuse/overflow/duplicate coverage is retained.

Three alternating runs per unchanged executable; native 1713x964, one view,
80-frame alpine flight, first ten GPU samples excluded, median of run medians:

| GPU timestamp timing | Before ms | After ms |
| --- | ---: | ---: |
| Selection/residency | 0.479 | 0.581 |
| Shared poses | 0.038 | 0.038 |
| Materialization | 0.056 | 0.056 |
| Forest preparation total | 0.573 | 0.674 |
| Timestamp-covered frame | 16.885 | 16.861 |

This introduces about 0.10 ms of preparation overhead, **not a speedup**.
Whole-frame difference is within run variation. The first implementation wrote
an atomic request flag for every shared use and cost about 0.15 ms extra; an
atomic-load guard avoids repeated contended writes once a page is requested.
Reset occurs only in the later compact dispatch, so concurrent first writers
remain harmless. Initial evidence remains in build/foliage-pages/atomic-rmw-*.

Final normal-route counts match baseline: 62,274 active roots, 4,090 selected
records, 1,751 source slots, 2,339 voxel slots, 4,020 shared poses and no slot/hash
failures. Inspected paired images show no obvious missing trees or pose change.
RGB MAE excluding 180 HUD rows is 0.290-0.293/255; channel p99 difference is 7.
Images are not pixel-identical. Raw GPU fault assertions provide the correctness
evidence; these route captures only check the normal all-resident appearance.

[Per-run/pass evidence and executable hashes](foliage-asset-publication-2026-11-04-evidence.json).
Local executables, profiles and captures: build/foliage-pages. No interactive
playtest opened. This one-view run does not establish 4K/four-player/8.33 ms.
Physical asset paging, consumer-aware upstream recovery, animated shadow caching,
unified grass and extreme-distance aggregates remain open architecture gates.
