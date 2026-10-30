# Rasterized foliage voxels: plan (30 October 2026)

Joep chose Nanite-style voxel bricks for distant trees (thread "Build
high-detail procedural terrain in Warp"). This plan replaces the ray
integrator of [brick visibility](vegetation-brick-visibility.md) with opaque,
stochastic voxel rasterization, so one representation feeds the camera
**and** the shadow maps. See [the performance analysis](forest-rendering-roadmap.md)
for why: tree geometry (87 M triangles over four views, 11 to 14 ms) does not
shrink with resolution, and distant trees still cast source-triangle shadows.

## What Nanite does, and what we copy

Epic's [Nanite Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)
voxelizes clusters into at most 128 bricks of 4x4x4 cells and picks voxels
where they have lower error than triangles. Voxel clusters are binned
separately and **rasterized** front to back into the same visibility/depth
buffers as triangles. Each voxel stores a normal distribution; the GBuffer
picks one normal per pixel stochastically. We copy exactly that split:
voxels are opaque fragments in the visibility buffer, never a separate
fractional medium.

## Data we already have

The cooker (`vegetation_asset`, `foliage_volume_data`) already produces per
level a sparse 4x4x4 brick directory and `GpuCell`s with albedo/roughness,
13 directional extinction coefficients and six weighted normal lobes. The
aggregate selector (`aggregate_lod`) already chooses source or a volume level
per branch group from projected error. Nothing in the cook changes.

## Design

1. **Splat list.** Per view, a compute pass walks the selected (tree, level)
   references, culls each brick against the frustum (later Hi-Z), and appends
   its occupied cells to an indirect draw: tree, level, cell.
2. **Raster.** One instanced quad per cell in the visibility raster pass,
   facing the view, one cell across, depth-tested with the triangles. The
   fragment keeps the cell with probability `1 - exp(-sigma(view) * cell)`
   from the cell's directional extinction, using a per-pixel, per-cell hash
   (TAA averages the dither). Its id carries a voxel bit and the splat index.
3. **Shadow.** The same splat draw into the sun maps and the foliage sun
   pages, with sigma along the sun direction. Source triangles of a branch
   group stop casting once its voxel level is selected.
4. **Shade.** `shade.wgsl` decodes the voxel bit, reads the cell, rebuilds the
   position from depth, and picks one of the six lobes in proportion to
   weight times extinction (Nanite's stochastic normal).
5. **Wind.** Splats use the owner pose; distant trees already stand still
   (`WindFreeze::DEFAULT`, 60 to 90 m), so voxel shadows can be cached.

## Milestones and acceptance

| Milestone | Done when |
|---|---|
| M1 camera splats in the aggregate fixture | 64-tree fixture renders voxels opaque; coverage within the existing IoU gates vs. the integrator; voxel raster ms below integration ms |
| M2 shadows | Shadow maps and sun pages take splats; shadow triangles of voxel groups drop to zero in the counters |
| M3 live forest | terrain_fly and Sideways trees select voxels past their error; camera and shadow triangle counts and frame time measured on the Mac and ManCave |
| M4 motion | TAA/DLSS motion for splats; moving-camera review |

Measure with GPU pass scopes and the triangle/splat counters. Report the
whole frame, not a pass sum.
