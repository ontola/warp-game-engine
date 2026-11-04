# Vegetation architecture: production contract

4 November 2026. This is the current direction, superseding opt-in rollout plans
in earlier reports. Architecture and integration precede visual tuning. The
terrain demo uses GPU forest selection normally; it no longer builds a CPU
near-tree pool. This document does **not** claim the entire design is implemented.

## Ownership and lifecycle

The CPU authors species, deterministic growth parameters, world cells and root
placements. It may cook/cache/upload assets and stream cells. It must not select
which trees receive detailed geometry each frame. A CPU worker generating world
data is not a CPU visibility pool.

One cooked species contains a parent-first articulated assembly, shared geometry
clusters, measured geometric error, material/coverage data, and voxel equivalents
of its subtrees. Growth stages are recipe variants sharing primitive assets, not
copies of the world population. Grass, shrubs and trees must use this contract;
different cooking strategies are allowed, separate visibility/residency systems
are not the destination.

World roots have stable cell/local identities and generations. Root deletion or
replacement invalidates dependent residency, motion and shadow cache entries.
Streaming cells must not rebuild the renderer or reassign all surviving IDs.
Asset pages are addressed independently of transient draw slots. Missing pages
request uploads and select an already resident ancestor, never an empty hole.

## Required frame graph

1. Apply queued world/asset changes; retain previous transforms and generation IDs.
2. Build camera and shadow consumer requests. One shared residency domain serves
   all players. Camera occlusion must never remove an off-camera shadow caster.
3. Cull roots and traverse shared cluster hierarchies with bounded GPU work queues.
   Select cuts by projected error and measured representation error. Reuse previous
   visibility for phase one, build current depth, then recover disoccluded work.
4. Combine requests and allocate persistent slots on GPU. Budget admission must
   publish complete cuts. Overflow chooses a resident ancestor deterministically.
5. Evaluate the needed articulated poses once; source, voxels, shadows and motion
   consume the same pose and epoch. Wind bounds cover the entire permitted motion.
6. Build consumer-specific indirect draw lists. Shared residency does not mean
   every shadow view must rasterize the finest camera representation.
7. Update dirty shadow pages; retain unaffected pages. Invalidation includes sun
   changes, cell edits, representation replacement and animated caster bounds.
8. Render source clusters or voxel regions, resolve materials, light and reconstruct.
   Debug readbacks are asynchronous/explicit; CPU decisions never await GPU counts.

History-first visibility cannot be bolted onto the shared selection pass using
one camera's HZB: that would drop casters required by other consumers. Likewise,
provisional depth from an incomplete cut cannot survive publication failure.

## Connected implementation versus remaining work

| Contract | Current implementation | Required next work |
| --- | --- | --- |
| Shared species assemblies | CookedPlant, shared_selection_recipe, shared source/voxel catalogs | Growth variant reuse and runtime asset content edits; source/voxel payloads now relocate behind stable logical IDs |
| Permanent population | Stable WorldRoot IDs, reserved slots and generations; worker-prepared deltas publish without renderer rebuild | Automatic capacity growth and asset paging; current root capacity is fixed and excess is rejected |
| GPU selection | Consumer union, deterministic complete-tree budget; GPU work queues distribute refining subtrees across workgroups | Integrate history-first traversal with safe publication across consumers |
| Occlusion | Normal per-view source route uses pixel-history scheduling, bounded deferred-cluster recovery and current depth; queue overflow re-scans the complementary cut | Feed safe visibility into shared assembly traversal/publication; current camera recovery is downstream of residency |
| Residency | GPU branch slots and logical requests; bounded physical source/voxel arenas, asynchronous shared bundle demand, chunked uploads and generation-safe eviction; complete resident intermediate cuts on detail faults, pinned root fallback | Tighter ancestor pins; defragmentation and asset-content lifecycle |
| Wind | GPU demand-deduplicated ancestor poses, GPU-sized indirect parent-depth dispatches and shared root poses; source/voxel/shadow transforms agree | Hardware validation/timing of indirect dispatch change (software oracle passes), active-root compaction, recipe growth/season lifecycle |
| Shadows | Independent source/voxel choices; normal shared sun uses GPU tile damage, sparse clear and source/voxel fragment masks plus early source-owner/voxel bounds rejection | Full renderer depth validation and timing; world-stable page coverage; current projection changes invalidate all tiles |
| Far population | One resident voxel record per tree, painted terrain beyond population | Spatial-cell aggregates without per-tree records at extreme distance |
| Grass | Separate meadow path | Cook meadow patches into the same assembly/selection/residency contract |

The common cut currently satisfies the most demanding consumer. This is correct
but can over-refine other views. The branch output budget is bounded separately
from the number of world roots. Source and voxel payloads use bounded caches and one publication owner. Host
catalogs remain resident; non-forest/dynamic geometry has explicit pinned ownership.
Source admission currently requests all cluster pages of a source branch, including
its CLOD levels, rather than faulting individual selected clusters. See
[voxel cache](foliage-cache-2026-11-04.md) and
[source paging](foliage-source-pages-2026-11-04.md).

## Implementation order and completion gates

1. **One production route:** remove the demo's CPU pool and alternate population
   modes; keep historical evidence and compatibility APIs for existing callers.
2. **Traversal and identity:** persistent cell/root update API, bounded GPU queues,
   two-phase recovery and publication. No full-renderer rebuild on cell changes.
3. **Asset residency:** page tables, request compaction, upload budgets, ancestor
   fallback and generation-safe eviction. Test an intentionally undersized cache.
4. **Animation and shadows:** evaluate shared ancestry once, dirty-page scheduling,
   independently budgeted shadow representations, moving sun and moving wind.
5. **Unified vegetation:** migrate grass to the same production machinery, then
   add species/growth variation and extreme-distance cell aggregates.

Each gate requires code connected to the normal renderer, focused GPU regression
tests, bounded-memory/overflow evidence and frame-pass measurements. A new module,
shader fixture, environment flag or attractive screenshot alone does not finish
a gate. Do not hide unfinished integration behind another backend switch.

Required regression cases: empty world, heavy oversubscription, teleport, four
cameras, off-screen casters, cell replacement, page eviction, wind, changing sun,
source/voxel transitions and motion history. Measure traversal, residency, poses,
source/voxel raster, shadows and shading independently. Historical one-view
1713x964 results do not establish the 4K-output/four-player/8.33-ms target.

## Present limitations

The normal route and bounded GPU traversal are connected. The rest of the
second gate now has stable population updates and downstream history-first source
recovery; shared assembly publication remains open. See
[history/recovery measurements](foliage-history-2026-11-04.md).
Existing games using legacy forest
APIs are not silently migrated. The compatibility Tree alias points to the new
RootPlacement type; the GPU runtime no longer depends on recipe_forest::Tree.
Visual tuning is deferred, but missing geometry and invalid lighting remain
correctness failures. See foliage-runtime-2026-11-04.md for prior measurements;
its opt-in status describes the historical experiment, not the current default.
