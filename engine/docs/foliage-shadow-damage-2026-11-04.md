# Animated shadow tile damage: producer and sparse clear

This is the next cache building block, **not yet connected to normal sun raster**.
The renderer still redraws moving forest shadows. There is no new opt-in backend
and no claimed performance win. Normal integration is required before the shadow
architecture gate can be completed.

`shadow_damage` consumes the existing world-space source instance bounds, voxel
bounds/transforms and common shadow representation choices after materialization.
A GPU invocation compares each caster to its exact previous snapshot (128 bytes
per source/voxel record). It marks both old and new projected footprints, including
removal, and records the new state. No CPU visible-caster list or counter readback
controls this. The mask is 512 bytes for 64x64 tiles of a 4096-square sun map.

The snapshot includes source attributes/flags, voxel page identity and shadow
representation selection. Geometry/material changes not expressed in those
records, including non-instance deformations and changed asset payloads, require
the owner's invalidation epoch. Projection or epoch changes invalidate all tiles.
First use also invalidates the full map. There is no approximate hash, pose
quantization or animation freeze. Any transform change dirties its conservative
footprint; dense wind may therefore dirty many tiles. This prioritizes correctness
and must be measured before claiming savings.

A sparse depth clear draws only marked tile quads. Clean tiles emit degenerate
triangles. The depth attachment uses Load and writes 1 only to damaged tiles;
source AND voxel raster must subsequently be masked to those tiles. Drawing the
whole scene over retained depth would leave removed/changed geometry incorrect.
The current implementation has a raw depth-buffer fixture to prove clean texels
are retained bit-for-bit; it is not a screenshot comparison.

## Remaining integration

1. Own this cache beside the shared sun target; run tracking after common forest
   materialization. Invalidate on abandoned submissions or failed raster.
2. Bind the same mask to source and voxel raster, including early caster/splat
   rejection where their projected footprint misses every dirty tile.
3. Replace full-map clearing with the sparse clear, preserve target allocation,
   and connect all external invalidation sources. Near/headlamp/legacy maps must
   retain their existing independent behavior.
4. Stabilize world-space page coverage during camera travel. A moving projection
   currently invalidates this whole map; this block alone is not virtual shadows.
5. Verify wind, removal/replacement, asset publication, sun changes, four cameras,
   overflow and source/voxel transitions against a full-redraw depth oracle.
6. Record per-pass GPU timings, mask population and updated texels on the normal
   route. Neither software-adapter timing nor this small fixture proves savings.

The fixture checks first-use invalidation, unchanged reuse, old/new source bounds,
voxel removal, source removal, epoch and projection invalidation. Every output
mask bit is cross-checked against the sparse clear's raw 64x64 depth readback:
dirty texels become 1, all other texels remain exactly 0.25. Validation outcome
is recorded in HANDOFF; hardware/normal integration remains outstanding.

Validation: release compilation, both Naga shaders and the raw-buffer/depth oracle
pass with Windows DX12 software rendering (`KERN_ADAPTER=software`,
`WGPU_BACKEND=dx12`): two tests passed. No normal-renderer/hardware performance
claim; the other games remained running on the physical GPU.
