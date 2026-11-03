# GPU forest integration: completion contract

User requirement, 3 November 2026: finish the terrain demo route without a CPU
near-tree pool dependency. **Status: incomplete.** A selector or allocator test
alone does not complete this work. The current `terrain_fly` still uses `Pool`.

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
Each mutable residency domain belongs to one selector/view. Geometry and recipe
buffers can be shared. Slots cannot be reassigned between queued views while
their draw or motion consumer still expects the old mapping.

## Required implementation, in dependency order

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

## Acceptance gate (all required; not yet passed)

- [ ] A real `terrain_fly` run uses the path, with no Pool construction/update
  and no synchronous visibility/readback-dependent allocation.
- [ ] Source, voxels and wind are rendered, not only a selection diagnostic.
- [ ] Near capacity is in branch records/bytes, not a hidden 240-full-tree limit.
- [ ] All world instances participate without per-world-tree expanded branches.
- [ ] Camera travel, rotation, teleport, disocclusion and instance generation
  replacement preserve correct geometry and motion history.
- [ ] Forced selection/hash/slot exhaustion renders one resident root for each
  affected tree, with no missing tree or duplicate partial source rendering.
- [ ] Multiple cameras and off-camera shadow casters remain correct.
- [ ] Matched source/voxel transition images and moving captures reviewed.
- [ ] Same executable/settings, at least three alternating GPU measurements;
  report resolution, reconstruction, view count, per-pass medians and CPU time
  separately. No FPS claim from readback-heavy route wall time.
- [ ] Commit/push implementation and evidence. Keep opt-in until accepted.

Lighting's 2.5 ms budget and animated shadow-cache performance are separate
milestones. Neither is completed by this allocation work. The 4K/four-player
120 FPS target remains unmet.
