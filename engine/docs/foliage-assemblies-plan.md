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

## Code map and findings (2 Nov 2026)

These are paths under `engine/crates/render/src/`. The numbers are for the
recipe spruce at 2560x1440 on the M5 Pro.

**How a full tree becomes instances today.** `vegetation_asset.rs`,
`SceneVegetation::append`, does the expansion:
- Each branch with a volume gets one owner `GpuInstance` (`PART_WIND`, no
  clusters), a `VolumeBinding`, an `aggregate_lod::Group` and a
  `foliage_representations` binding.
- Each placement gets its own `GpuInstance` and a 160 B `PartAttachment {pose,
  owner, rest}`.
- Each branch gets one `PartWind` pose.

A spruce comes to 5,169 placements, 61 poses and ~5,230 instances, about
1.8 MB resident per tree:

| Per placement | Bytes |
|---|---|
| `GpuInstance` | 80 |
| Previous-frame copy (motion.rs copies the whole buffer each frame) | 80 |
| Wind coefficients | 16 |
| `PartAttachment` | 160 |

| Per pose | Bytes |
|---|---|
| Descriptor | 304 |
| Pose | 96 |

**Caps.** These limit how many trees can be full:
- `recipe_forest::limits()` and `plant_streamed` fit the pool to
  `part_wind::MAX_ATTACHMENTS` = 65,536, which is 12 spruces.
- Behind that sit 4096 volume bindings (`foliage_volume_data::MAX_INSTANCES`,
  about 68 trees), aggregate group and volume ids < 4096, and the
  `foliage_selection` uniform `array<vec4<u32>,4096>` (cull.wgsl,
  voxel_forest.rs).
- `part_wind::RuntimeLimits`: 262,144 poses, 1M attachments, 512 MiB.

**part_wind** (`shaders/part_wind.wgsl`) already runs on the GPU:
- `schedule_assemblies` sleeps assemblies that no view sees and scales motion
  by projected size.
- `update_parts` evaluates the poses parent-first, but is dispatched over all
  assemblies, not indirectly.
- `update_attachments` writes `pose x rest` into every member `GpuInstance`
  of an awake assembly, even members whose branch group is on voxels.

Pose evaluation is therefore not the bottleneck; residency and per-placement
culling are.

**Culling.**
- `cull.wgsl` runs one 64-lane workgroup per instance.
- `instance_policy` hides a source member when its group's
  `foliage_selection` chose voxels.
- `cull_cluster` makes the DAG cut, using the aggregate's world triangle cut
  when one is set.
- There is no tree-level node above instances. `cull_groups.rs` has an
  optional Morton hierarchy over owners; `AssemblyDescriptor.sphere` is a
  per-tree bound.

**Instance reads.** `instances[...]` is read 41 times across 17 shaders. For
trees, these matter: cull (4), raster (1), shade (4), motion (1),
voxel_forest (10) and part_wind.

**Dead end: merged branch meshes.** I merged each branch's placements into
one cluster mesh, giving one instance per branch (branches are rigid under
part_wind).
- The image is the same (PSNR 43.9 dB).
- 12 full trees went from 23.5 to 27.2 fps, and 200 trees render.
- The cost is ~1 GB of geometry per spruce or pine variant (2.6 GB for six
  species), because the source is millions of separately modelled needles.
- 60 full trees ran at 16 fps.
- So: instanced shared parts as in Unreal's Assemblies, not merged geometry.
- The patch lives in session scratch only; it was not committed.

**Handover.** Full source geometry must hand over to part voxels
(aggregate_lod) much closer to the camera than today. Otherwise "full trees
everywhere" blows the raster budget, whatever the instancing.

**Measuring.** Per-pass GPU timestamps overlap on Metal and gave wrong costs:
"voxel composite" read 6-15 ms while removing it changed nothing. Measure by
fps with one thing switched off at a time (the `--bench` deltas at the four
playtest cameras in `build/screenshots/terrain-fly-*.json`).

## Status (2 Nov 2026, evening)

Done and on main:

- **Milestone 1:** virtual members, on by default in terrain_fly.
- **Milestone 3:**
  - Voxel splats go into the main visibility buffer and are lit by shade.
  - Branch voxels switch in at 32 px.
  - Whole-plant voxels shade themselves through their own cells.
- **Milestone 4 for terrain_fly:** the card ring is the low-end fallback only.
- **Group selection packed:** a view now holds 16384 groups, enough for 240 full trees by default.

**Milestone 2 (GPU per-tree selection, streamed full assemblies) is parked.**

- With the group budget fixed at about 270 resident full trees, letting the GPU choose among resident trees only replaces the CPU's nearest-240 assignment. That assignment now has hysteresis.
- The real gain needs streaming pages of assemblies, as Nanite does. That is a multi-day project whose payoff is detail at range, not frame time.
- Frame time is now dominated by shade and by the voxel pipeline's fixed passes. See the commit log from 0e1c803 on.
