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

The profiler adds `shadow tile damage and clear`. Source culling now rejects owners whose projected world bounds miss every dirty
tile before cluster traversal or virtual-member expansion. Voxel sun generation
uses the same predicate before level/cell iteration and splat allocation. Flat
and spatial source paths preserve their normal overflow fallback. Fragment
masks remain necessary for partially overlapping bounds. The map projection still follows the existing anchor policy, so
camera translation can invalidate every tile. These are material remaining costs.

`gpu_cached_sun_depth_matches_full_redraw` is an ignored hardware fixture. It
warms assets, renders four views, compares raw shared Depth32Float values against
forced full damage at wind changes, unchanged wind, root removal/restoration,
changed sun and camera movement, and requires actual nonempty caster depth.
It compiles but has not run while other games occupy the GPU.

## Remaining integration

1. Execute the complete renderer oracle on the physical GPU; the small software
   fixture does not prove all renderer bindings and invalidation cases.
2. Measure the integrated early source-caster/splat rejection, including its
   bounds-query overhead, rather than counting discarded fragments.
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


## Early dirty-bound work rejection

The production cached sun compute paths consume the same GPU mask as raster.
`shadow_damage_query.wgsl` projects all eight corners, expands by two texels,
and tests the resulting tile rectangle with at most two bitmask words per row.
The query is conservative, including unsupported projection fallback. A source
workgroup tests once in lane zero and shares the decision before expanding
members or visiting clusters. The spatial owner overflow fallback still scans
all owners through this predicate. Voxel generation tests its materialized world
AABB in lane zero before walking cells; its draw reset runs even if all owners
are clean. Regular/near/headlamp entry points do not reference the mask.
Voxel shader reload prepares and commits both regular and cached pipelines.

This is owner-level rejection, not per-cluster dirty testing or a compacted dirty
owner dispatch. Workgroups still launch for candidates; giant bounds can overlap
many clean tiles and wind may invalidate much of the map. Projection movement
still invalidates all tiles. No hardware speedup has been established.

The small `dirty_bounds_match_brute_force_tiles` fixture evaluates every tile as
a point plus 4096 deterministic rectangles, on four masks (empty, full, sparse,
and word-boundary stripes). It checks both direct rectangle queries and projected
world bounds against CPU tile enumeration: 65,536 result comparisons. This tests
the shared predicate, not the complete renderer's final depth or performance.

Early-bound validation: `cargo check --locked -p render --tests` PASS. Release
software DX12 `shadow_damage::tests --include-ignored --test-threads=1`: 3 PASS
in 11.30 s. Culling specialization matrix, three lighting snapshot/reload tests
and voxel empty-ceiling shader variants also PASS. Hardware complete-renderer
oracle and timing remain pending while Sideways uses the GPU.
