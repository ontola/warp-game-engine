# GPU forest integration: completion contract

User requirement, 3 November 2026: finish the terrain demo route without a CPU
near-tree pool dependency. **Status, 4 November: integrated and opt-in, visual/performance acceptance pending.**
`WARP_TERRAIN_GPU_FOREST=1` now renders actual source and voxels in `terrain_fly`
without constructing/updating `Pool`. The default route remains unchanged.
See [runtime evidence and remaining limits](foliage-runtime-2026-11-04.md).

## Available building blocks

- `CookedPlant::shared_selection_recipe`: shared hierarchy, actual source branch
  identities, measured voxel admission and animated bounds.
- `foliage_select::Selector`: two-phase GPU cuts, current-depth recovery,
  conservative list overflow and indirect record counts.
- `foliage_residency::Residency`: consumes the completed GPU record stream,
  retains stable `(tree, node, generation)` slots, allocates/reclaims on GPU.
  Bounded hash failure and slot exhaustion mark the entire tree for its resident
  root. Final publication suppresses partial trees. No CPU allocation decisions.
- `virtual_member.wgsl`: shared source address composition in cull, raster,
  material resolve and motion. This still uses the legacy packed owner address.

The residency state is 32 bytes per branch slot, not per world tree/branch pair.
Temporary hashing is sized by selected-record capacity. Duplicate requests emit
once; generation changes and previously unpublished slots invalidate history.
The integrated runtime uses ONE frame-wide residency domain: a conservative
union of all cameras and shadow consumers selects the required cut. This keeps
live source/volume buffers and motion snapshots coherent across queued views.
Per-view culling still follows. Separate view domains would need separate live
buffers/history; simply reassigning shared slots between views is incorrect.

## Implemented integration

`foliage_runtime` exports shared templates/rigs, keeps permanent roots, selects
and budgets whole cuts on GPU, retains/allocates branch slots, evaluates wind
ancestry, and writes actual source/voxel owners and separate shadow choices.
Whole-root voxel selections bypass the transient branch slots. Oversubscription
selects a resident root, never partial geometry. Demand priority is projected
size, then stable root ID; queue thread arrival does not decide quality.

Important deviation from the original step 4: the common caster cut cannot use
one camera's HZB to reject off-camera shadows. Runtime selection is a conservative
consumer union followed by the existing per-view two-pass source cluster culling.
The standalone history-HZB selector remains tested, but a persistent load-balanced
cluster queue and history-first production traversal are NOT completed here.

## Original implementation requirements (retained for review)

1. Export shared source template ranges, branch bind/parent/wind data and packed
   voxel spans from each cooked recipe. Root placements are permanent world data.
   Allocate bounded GPU owner/pose output buffers; do not expand every branch of
   every tree or reserve a fixed number of full trees per species.
2. Materialize assigned branch slots on GPU into source owner matrices and voxel
   inverse/bounds records. Use the same articulated pose for both representations.
   Initialize unused slots as hidden. Propagate published status and history
   validity into the actual motion consumers, not just diagnostics.
3. Connect the materialized records to `Renderer` source culling and voxel splat
   work. Update owner classification / dynamic bounds and replacement mapping;
   do not let construction-time placeholder bounds cull moving slots. Existing
   shared voxel level/cell catalogs can remain shared by asset, but capacity
   estimates based on placeholder instances must cover dynamic assignments.
4. Encode phase-one visibility, current depth/HZB, phase-two recovery, final
   residency/publication and final draw consumption in order. Phase-one depth
   is provisional: a later overflow can invalidate a tree's partial cut. Final
   color must obey final fallback flags. Avoid duplicate source + root draws.
5. Handle every player camera and independent shadow consumer. Camera invisibility
   must not suppress shadow casters. Wind, sun changes and slot replacement must
   invalidate affected cached shadow work. Do not silently freeze wind or remove
   shadows to make the integration appear successful.
6. Add the real opt-in scene path in `terrain_fly::recipe_spruces`, returning no
   `recipe_forest::Pool`, and omit `plant_streamed_reserved` / `Pool::update`.
   Cooking and initial root placement may run on CPU. Per-frame camera-to-tree
   selection, slot assignment and source/voxel draw counts must not.

## Acceptance gate (full acceptance still pending)

- [x] A real `terrain_fly` run uses the path, with no Pool construction/update
  and no synchronous visibility/readback-dependent allocation.
- [x] Source, voxels and wind are rendered, not only a selection diagnostic.
- [x] Near capacity is in branch records/bytes, not a hidden 240-full-tree limit.
- [x] All world instances participate without per-world-tree expanded branches.
- [ ] Camera travel, rotation, teleport, disocclusion and instance generation
  replacement preserve correct geometry and motion history.
- [x] Forced selection/hash/slot exhaustion renders one resident root for each
  affected tree, with no missing tree or duplicate partial source rendering.
- [x] Multiple cameras and off-camera shadow casters remain correct.
- [x] Matched route images and eight pan captures reviewed; visible tradeoffs
  remain. This is not full optical/temporal acceptance.
- [x] Same executable/settings, at least three alternating GPU measurements;
  report resolution, reconstruction, view count, per-pass medians and CPU time
  separately. No FPS claim from readback-heavy route wall time.
- [x] Commit/push implementation and evidence. Keep opt-in until accepted.

Lighting's 2.5 ms budget and animated shadow-cache performance are separate
milestones. Neither is completed by this allocation work. The 4K/four-player
120 FPS target remains unmet.

## Additional reference supplied 4 November

User supplied https://www.youtube.com/watch?v=uR5EvBWxN14 and its description:
CLOD virtual geometry, far voxels without impostors, screen-space/voxel hybrid
GI, and extremely distant proxies without per-instance data. Video retrieval
was throttled; no visual, hardware or performance claims have been verified.

Useful follow-on architecture: spatial-cell aggregates beyond the per-tree cut,
with stable species/coverage/material/normal statistics and shared wind state.
Our current roots remain individual tree records, so this is NOT implemented.
Hybrid GI is a separate lighting milestone, not a consequence of GPU residency.
