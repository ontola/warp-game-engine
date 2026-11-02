# Foliage as assemblies: one tree path from 1 m to the horizon

Status: design, November 2026. Replaces the tiered forest (recipe_forest:
a pool of full assemblies, alpha card trees, whole-plant voxel proxies, the
painted far layer) with one representation per tree, after Unreal 5.7's
Nanite Foliage and Assemblies.

## Why

Joep's terrain_fly playtest (2 Nov 2026, four cameras, 2560x1440, M5 Pro)
measured by fps with one part switched off at a time. Per-pass GPU
timestamps overlap on Metal and are not usable for this. Card trees cost
9-23 ms, voxel proxies about 9 ms and the painted far layer about 5 ms. The
cards are also the main source of shimmer when the camera turns. Every tier
boundary is a visible change of look. That is the pop-in Joep reported, and
forest_ladder measures the same boundaries.

## What Unreal does, and what we take from it

- **Voxels for distant foliage.** Clusters of up to 128 bricks of 4x4x4
  voxels, rasterized like opaque geometry with early Z, each voxel holding a
  normal distribution. We take this: our rasterized splats (voxel_forest
  raster) already draw stochastic opaque voxels. Since fda385c the opaque
  shade skips the pixels they hide.
- **Error, not distance, picks the representation.** A cluster switches from
  triangles to voxels when the voxel error is below the triangle error.
  aggregate_lod already does this per branch group of a full assembly; it
  has to cover every tree, not only the pool.
- **Assemblies.** Branches and sprays are instanced part clusters in the
  tree's hierarchy. Their transforms come from the bone (part) poses during
  cluster culling, and each part simplifies down to a single cluster.
- **No alpha-masked cards.** We keep the card mesh (plant_mesh_lod) only as
  the low-end merged fallback, Unreal's non-Nanite path.
- **Wind per part (rigid), off below a screen size.** Bounds stay tight and
  distant shadows stay cacheable.

## Milestones

Each milestone ships on main with forest_ladder (coverage, mass and tone
against the source, within ±10 % at every distance) and fps at the four
playtest cameras (`terrain_fly --track --from build/screenshots/terrain-fly-*.json --bench 120`).

1. **Assemblies as virtual instances** (a, about 4 days). Mapping the code
   (2 Nov) showed that part_wind already evaluates poses on the GPU, and only
   for assemblies that some view sees (`schedule_assemblies` sleeps the
   rest). The ~12-tree cap comes from residency instead: every placement of
   a full tree is its own `GpuInstance` plus a 160 B `PartAttachment`. A
   spruce is 5,169 placements and 61 poses, about 1.8 MB, and
   `MAX_ATTACHMENTS` is 65,536 / 5,169 = 12 trees. Two further caps sit
   behind it: 4096 volume bindings, aggregate groups and the
   `foliage_selection` uniform array.
   The change:
   - Placement rest transforms and mesh refs go into one table per variant.
   - Each tree keeps only its root, phase and a block of branch poses
     (61 × 96 B ≈ 6 KB).
   - A reserved instance-id range means (tree, placement).
     `instance_at(id)` composes `pose[tree, branch] × rest[variant,
     placement]` wherever a shader reads `instances[id]` today: 41 reads
     across 17 shaders, of which cull, raster, shade, motion and
     voxel_forest matter for trees.
   - Cull walks tree → branch group (skipping groups the aggregate selection
     put on voxels) → that group's placements, so only visible, non-voxel
     branches reach the cluster cut.
   - `update_parts` is dispatched indirectly over awake trees only.
   Expected fps: unchanged at equal tree counts, but full trees out to the
   distance where branch groups switch to voxels, with no pool.
2. **GPU instance hierarchy** (b, about 4 days). Trees become instances of an
   assembly (parts × placements) culled hierarchically: tree bounds, then
   branch groups, then part clusters. Built on the GPU from the placement
   output (render::placement) for about 5k trees in view. Instance memory per
   tree stays O(1): seed, pose index and variant. Expected fps: similar to the
   card tier at 150 m, without its shimmer.
3. **Per-cluster triangle→voxel switch to whole-plant voxels** (c, about
   4 days). Extend aggregate_lod's projected-error choice above branch groups
   to whole-plant voxel levels, so one tree goes smoothly from source
   triangles through part voxels to whole-plant bricks. The voxel bricks
   raster into the main visibility buffer and are lit by shade (no resolve or
   composite pass), with one normal per pixel from the stored distribution
   (a stable hash by voxel and pixel). Expected: −8 to −15 ms against cards
   plus proxies at the high cameras; ladder coverage within ±10 % at 40–600 m.
4. **Retire the tiers** (about 2 days). Cards become the low-end merged
   fallback (off by default) and proxies the deepest voxel level. The painted
   far layer stays only beyond the last voxel level, where a tree is under a
   pixel; forest_field already matches it statistically. Sideways moves over
   via its plant_population policy (the coordinator owns games/sideways).

## Low-end profile (Quest-like minimum)

Each layer keeps a cheap setting:

- Trees: the card mesh as a merged fallback, voxels from about 40 m.
- Terrain: KERN_SIMPLE_LIGHTING=1, or terrain_coarse with a lower threshold.
- Adaptive quality: render scale only, floor 0.67.
- Shadows: the shared map only.

## Risks

- **Pose buffer bandwidth** with 5k trees × 32 branch groups is about 10 MB
  per frame. Freeze distant poses and update them at a lower rate.
- **Voxel normal noise.** Unreal names this as their noise source; our splat
  hash is already per voxel and pixel. Measure shimmer with the pan test from
  the playtest (`playtest-diag`).
- **Shadow consistency** when representations change. Every representation
  casts from the same voxels past the near map, as now.
- **Hot-reload and four-view budgets.** The cull pass grows; the r55
  four-view budget plan has to be rechecked.
