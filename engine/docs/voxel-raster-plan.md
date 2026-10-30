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

## Status

**M1 built (opt-in, `KERN_VOXEL_RASTER=1`).** `voxel_forest.rs` lists every
occupied cell of every level once at load (`splat_cells`). Per view,
`voxel_splats` keeps the cells of each tree's selected level that pass the
frustum and have opacity `1 - exp(-sigma * mean chord)` above 1/255, and
appends them to an indirect draw. `voxel_splat_raster.wgsl` draws three
camera-facing faces per cell into an R32 id and reversed-Z depth target and
discards each pixel with the cell's opacity (hash of pixel and cell, so a
still image does not shimmer). `voxel_splat_resolve` shades the winning cell
once per pixel with the integrator's own `volume_radiance` and writes the same
colour/alpha layer, so the existing composite is unchanged. Opaque scene
depth in front wins. GPU scopes: `voxel splats`, `voxel splat raster`,
`voxel splat resolve`.

First headless check (lavapipe, CPU rasterizer, so only relative), one spruce
at 150 m, `--error 16` so all 60 branch groups pick level 0, 960x540:
coverage 938 versus 931 pixels for the integrator (under 1% apart); voxel
passes 88 ms versus 153 ms for brick visibility plus integration. The raster
pass is most of that on a software rasterizer; on a real GPU it is the cheap
part. Without TAA the result is dithered; TAA averages it (M4 adds motion).

Not yet: the 64-tree acceptance run on a real GPU, Hi-Z culling, shadows (M2).
