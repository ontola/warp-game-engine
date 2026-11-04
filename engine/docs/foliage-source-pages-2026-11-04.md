# Source geometry in the common forest asset cache

Detailed source branches now join voxel pages in one representation publication.
GPU requests are consumed through the same asynchronous readback ring. Admission
checks both source and voxel capacity for complete dependency bundles, and both
use the same per-frame byte/page-completion budget. A source representation is
unavailable until all of its source and adaptive voxel-shadow dependencies are
resident. One publisher advances generations and retires old draw history.

Source admission conservatively requests every cluster page in a source branch,
including its CLOD levels. Fine per-selected-cluster faults and intermediate
ancestor recovery are still future work. Source pages contain up to 64 logical clusters. Their culling/material metadata
keeps its logical ID; first_vert and first_tri address physical arenas. Page-local
vertex remaps preserve positions, normals, UVs and packed triangle topology.
Repeated topology within a page is shared (particularly terrain patch topology).
Three bounded source arenas hold vertices, cluster vertex indices and triangles;
the logical cluster metadata buffer remains resident. The default limits are
64 MiB vertices, 16 MiB cluster indices and 16 MiB triangles, plus metadata.

Ordinary scene geometry is pinned and shared with the normal renderer, not
copied into a separate foliage draw backend. Static pinned objects may compact.
Skinning and the procedural terrain generator write fixed vertex addresses, so
their original vertex prefix is reserved and preserved. A first visual experiment
missed the terrain writer and showed large corrupted triangles; its timings are
invalid. The corrected contract explicitly preserves that destination range.
Interleaving dynamic vertex writers after forest assets can still leave pinned
holes: an explicit independently addressed dynamic geometry heap is future work.

Pressure evicts unrequested source pages with the same complete-root fallback
as voxel misses. Source and voxel caches cannot independently set the selector's
resident bit. The normal cold-start renderer regression requires genuinely
unloaded source pages at frame zero. The combined cache fixture loads all voxel
pages first and proves that unavailable source pages still block publication;
it then evicts/reloads source pages and checks generation-safe replacement.

Other focused checks reconstruct every authored triangle/attribute, compare raw
GPU vertices/indices/cluster headers after partial uploads and eviction, preserve
pinned geometry, compact late static meshes and preserve procedural terrain's
fixed-address writes and shared topology. Current host catalogs and immutable
asset cooking remain resident. Texture paging and runtime species content edits
are outside this implementation. Intermediate-ancestor fallback, shared upstream
history publication, animated shadow caching and unified meadow remain open.

Contiguous cluster headers upload together; preserved dynamic vertex ranges use
identity addressing without constructing a large CPU remap table. Cooking still
retains host asset copies and is not a disk-streaming implementation.

## Validation and measurements

All 120 release foliage-filter checks PASS, including serial GPU tests. Finished
at 16:21:52 before other agents' fullscreen games started. Normal-route final
images compare with the prior voxel-cache renderer at native 1713x964, one view,
180 route frames, 15 m/s. RGB MAE is approximately 0.022/255, p99 1, excluding the
first 180 HUD rows. This is coverage/lighting equivalence, not bit-identical output.

Default source data: 76,508,384 allocated bytes across its four buffers, including
74,398,560 initially uploaded pinned bytes (chiefly the reserved procedural terrain
pool and its topology/metadata). Source pages go from 1 pinned page to all 16.
Voxel pages go from 27 to 507. Both finish loading by frame 63; the largest joint
frame upload is 1,204,984 bytes, with no evictions or denied requests. These counts
exclude CPU catalogs, feedback buffers, textures and other renderer allocations.
A fully resident small species catalog therefore does not establish a VRAM saving.

62-MiB source vertex budget: 75,548,256 allocated source-buffer bytes and only
2/16 source pages resident; 27,805 bundle admission denials, zero reload churn.
Final source/voxel slots are 68/4005 versus 1428/2625 with the default budget.
This is visibly coarser, complete replacement, not quality-equivalent memory
savings. The 61-MiB configuration is rejected because pinned geometry cannot fit.
The tiny GPU fixture separately forces real source page eviction/reload twice.

An earlier corrected build, before the final CPU-only initialization cleanup,
had three alternating pairs with covered GPU medians 10.225 -> 10.293 ms and
forest prep 0.745 -> 0.746 ms (99 post-warmup samples per run). This is no speedup.
Its saved per-run/pass report is retained in the evidence JSON. **Final executable
performance remains unverified**: another agent started Spellstorm at 16:22:32,
and Growing Guns at 16:25:07, overlapping the final repetitions. Those timings
are rejected. No other agent's game was stopped; no further GPU jobs were started.
The earlier raw run paths were reused by the final repetitions, so the saved
historical summary, not those current files, identifies that earlier result.

Separate asset-feedback copy submissions/upload work are outside main-frame GPU
timestamps. CPU render intervals are not isolated CPU compute. These one-view
runs do not establish the 4K/four-player/8.33-ms target. Re-measure the final build
when the GPU is free; source paging is functionally validated, not a claimed
performance improvement.

[Evidence, hashes and cache traces](foliage-source-pages-2026-11-04-evidence.json).
Local artifacts: `build/foliage-source`; `first.png` preserves the rejected terrain
address failure, while `after-*.png` and `pressure.png` show the corrected renderer.

