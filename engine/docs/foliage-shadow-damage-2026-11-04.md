# Animated shadow tile damage: producer and sparse clear

The normal GPU-forest shared sun route now connects damage detection, sparse
clearing and source/voxel fragment masks. No opt-in backend is added. **Complete
renderer depth-oracle execution and hardware measurements remain pending**; the
shadow architecture gate is not complete and no performance win is claimed.

`shadow_damage` consumes the existing world-space source instance bounds, voxel
bounds/transforms and common shadow representation choices after materialization.
A GPU invocation compares each caster to its exact previous snapshot (128 bytes
per source/voxel record). It marks both old and new projected footprints, including
removal, and records the new state. No CPU visible-caster list or counter readback
controls this. The mask is 512 bytes for a 64x64 tile grid. Projection margin and raster tile
size use the actual sun-map size (4096 desktop, 2048 Android).

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
source AND voxel raster now use the same mask through dedicated cached-depth
entry points. Ordinary, near and headlamp entry points remain independent.
The current implementation has a raw depth-buffer fixture to prove clean texels
are retained bit-for-bit; it is not a screenshot comparison.

## Normal renderer connection

The cache runs after forest materialization and before shared sun raster. Source
raster loads the retained depth instead of clearing it; voxel raster also loads
it. Both cached fragment entry points reject clean tiles before alpha sampling.
Moving GPU forest poses no longer invalidate the whole map through set_wind;
legacy wind casters still do. First use, target recreation, sun/projection changes,
alpha/LOD policy, explicit content invalidation and failed/unsubmitted frames force
full damage. Receiver-demand mode and non-instance skinning/crowd deformation
conservatively invalidate all tiles. The legacy static/dynamic split path retains
its previous full dynamic redraw behavior.

The profiler adds `shadow tile damage and clear`. Source culling, vertex work and
voxel splat generation still run for all candidates; dirty-bound rejection is not
yet connected. The map projection still follows the existing anchor policy, so
camera translation can invalidate every tile. These are material remaining costs.

`gpu_cached_sun_depth_matches_full_redraw` is an ignored hardware fixture. It
warms assets, renders four views, compares raw shared Depth32Float values against
forced full damage at wind changes, unchanged wind, root removal/restoration,
changed sun and camera movement, and requires actual nonempty caster depth.
It compiles but has not run while other games occupy the GPU.

## Remaining integration

1. Execute the complete renderer oracle on the physical GPU; the small software
   fixture does not prove all renderer bindings and invalidation cases.
2. Add early source-caster/splat rejection when projected bounds miss every dirty
   tile, then measure saved work rather than counting discarded fragments.
3. Extend full-renderer coverage to asset publication/eviction and procedural
   terrain changes; preserve near/headlamp/legacy behavior.
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

Integration follow-up checks: `cargo check --locked -p render --tests` passes.
The release software test filter `shadow_damage::tests` passes both tests; shader
validation now includes the full source raster and voxel splat raster modules.
The raw depth oracle uses the actual map-size input. Full renderer hardware
validation remains pending; no performance acceptance is inferred from this.
