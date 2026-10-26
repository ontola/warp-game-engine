# World-fixed vegetation volume field

Implementation plan, 25 October 2026. **The field renderer described below is
not implemented or accepted.** The immediate objective is to replace the
grass representation changes at large distance with progressively coarser
volumes of the same explorable world. A distant hill must not become a
special terrain-colour substitute when the player approaches it.

## What exists

- The live meadow uses `MeadowGeometry`: three shared recipes, 360 blades per
  root, fixed basal anchors and four wind groups. GPU generation selects roots
  from a world seed and the native terrain. Source CLOD preserves every blade.
- The current distant production candidate is a source-derived directional
  atlas. It blends from geometry at 60–80 m and fades at 160–180 m. Its terrain
  substrate is a material approximation, **not a voxel field**. This remains
  the reference until a replacement passes validation. See
  [r39 results](meadow-distance-r39.md) and [r40](meadow-night-r40.md).
- New [meadow_voxel.rs](../crates/render/src/meadow_voxel.rs) cooks those finest
  recipe streams into three ordinary `foliage_voxel::VoxelAsset`s. The explicit
  default is 4 cm cells, up to six mips and `SheetCrossings`. Anchored uniform
  growth is applied before cooking. Materials include the source height
  pigment; each triangle's mean pigment approximates its contribution within
  individual cells. CPU tests cover mass, normals, materials and packing;
  their presence is not a claim of a completed GPU field test.
- [foliage_volume_data.rs](../crates/render/src/foliage_volume_data.rs) already
  packs sparse cells, dense indirection and mip metadata. Its `GpuCell` is
  176 bytes and stores 13 directional extinction samples plus six actual
  normal-centroid lobes. The existing
  [voxel_forest.rs](../crates/render/src/voxel_forest.rs) renderer is a bounded
  proof with at most 256 volume owners. Raising that limit does not create
  world streaming or make one volume instance per grass root affordable.

The live trees still use shared source meshes/CLOD and alpha-tested foliage.
General tree voxel and procedural assembly fixtures are separate from that
live path. This plan reuses their data and traversal contracts; it does not
claim that all trees and grass already share a production volume renderer.

## Proposed data and residency contract

Use a sparse, world-fixed brick hierarchy. A page key contains integer world
brick coordinates, mip level, terrain generation and vegetation recipe/season
generation. Camera position is never part of plant identity. Root identity
remains the existing signed integer 0.45 m cell plus field seed; variant,
jitter, yaw, growth and material pigment must match
[meadow_generate.wgsl](../crates/render/src/shaders/meadow_generate.wgsl).

Start with 8³-cell bricks and explicit world-space cell sizes. A practical
first experiment is 6.25 cm base cells with dyadic coarser levels. This size
is a proposed test setting, not a measured quality limit. Reconstruct all
mass intersecting each brick, including source overhang from neighbouring
roots. Halo cells support filtering but must not duplicate physical mass in
adjacent interiors. An empty child contributes zero mass to its parent; it
must never be omitted from a density denominator.

All four cameras request pages from one residency cache. Deduplicate requests
by the full key before generation/upload. Each camera retains its own
unjittered projected-error decision and hysteresis, while poses, immutable
recipes and resident pages are shared. Sun/headlight visibility is a separate
consumer and cannot inherit a camera's geometry rejection.

Initial **experimental caps**, to be revised from actual measurements:

| Resource | Proposed hard cap and behaviour |
| --- | --- |
| Shared sparse cells, all mips and cameras | 262,144 cells; 44 MiB at the existing 176-byte layout |
| Shared dense indirection | 2,097,152 indices; 8 MiB |
| Shared resident asset/page payload | 64 MiB including metadata; renderer image targets reported separately |
| Page generation | Explicit sample/work limit plus a small configurable update count; GPU timestamps decide whether it fits |
| Camera cuts / distant players | Preserve a resident coarser ancestor; report missing pages and generation age |

Never silently drop plants or draw a green terrain patch when the cache is
full. Keep a valid coarser volume ancestor until a finer page is committed.
If no ancestor exists, the experiment must report that missing coverage and
fail its acceptance gate. The first bounded stage should prepare its coarse
coverage before showing a frame. General unbounded-world streaming is later
work, not implied by finite stage coverage.

## Generation, wind and terrain

Extract shared pure placement/deformation functions from the existing source
path before adding a second implementation. The page producer needs exactly
the same native-triangle interpolation, invalid-ground handling, slope
support, packed road-plane test and per-blade road mask. A page may span many
terrain triangles: applying one affine slope to the whole page is wrong.
Apply each root's actual terrain transform before accumulating world voxels.

Uniform scaling of a complete patch also moves its distributed anchors.
Instead use the source order: fixed blade anchor, growth about that anchor,
root yaw, the blade's wind-group rotation, then the local terrain shear.
Normals use the corresponding inverse transpose. The current whole-patch
`MeadowVoxelAsset` is useful for a rest-pose proof; one rigid transform cannot
reproduce four independently animated groups with different fixed anchors.

The first animation milestone should compose a small page from shared source
primitives at explicit current and previous wind times and compare it against
expanded source geometry. Reuse [part_wind.rs](../crates/render/src/part_wind.rs)
pose/history conventions, not its bounded owner count as a world architecture.
Evaluate poses once for the union of requested world regions, not four times
for four views. Do not regenerate every distant blade every frame.

For farther pages, schedule updates from a conservative projected deformation
error bound. A retained pose is allowed only while its accumulated error stays
below the declared limit; moving closer must request a finer/current page or
source representation before exceeding that limit. Publish page contents,
generation and previous-pose validity together. Missing previous data requires
an explicit temporal reset; stale motion vectors are not an acceptable shortcut.

## Optical and compositing contract

Voxel surface area and directional projected area are **additive mass**.
Opaque projected union coverage is not additive, and Beer transmittance of
averaged extinction is not automatically equal to that union. Keep both
measurements in validation. Do not fix a mismatch by multiplying density or
darkening all distant albedo. The existing 13-direction interpolation and
six normal-centroid lobes are useful approximations, not proof of optical
equivalence at every angle.

Trees and grass can share the sparse layout, mip reduction, empty-space
traversal, work diagnostics and bounded optical tests. Their source semantics
remain explicit: open grass ribbons use `SheetCrossings`; only validated
closed geometry may use `ClosedSurfaceEntries`. Authored material and actual
two-sided normals must survive reduction. Preserve the common
`thin_foliage_response`, sun/sky visibility and normal transformation; changing
to a terrain-up normal would recreate the current lighting discontinuity.

Integrate volume radiance and transmittance before the shared fog step.
Truncate against actual opaque scene depth. A brick AABB or a partially
occupied cell must never write a solid depth occluder over its holes. Fractional
volume motion/depth needed by DLSS remains separate from opaque scene depth,
following the existing `motion_layer`/opacity contracts in `voxel_forest`.

Use exclusive hierarchical representation cuts: every root/group is covered
exactly once, by source or by one selected page level. If a transition uses
blending, preserve complementary optical weight and test its compositing order;
painting a full terminal layer underneath explicit grass gaps is double
counting. A geometric pixel-error bound is insufficient without coverage and
lighting evidence. Reuse `OpticalEvidence` and per-view policy from
[foliage_representation.rs](../crates/render/src/foliage_representation.rs).

## Concrete milestones and gates

1. **Recipe asset proof.** Register/test `meadow_voxel`; report actual finest
   triangles, occupied cells per mip, bytes and source material/normal moments.
   Render a bounded rest-pose patch through the existing general volume path.
   Compare matched source masks, grazing angles and sun/sky lighting. This is
   the next small proof, not a full-field launch.
2. **World page producer.** Build a bounded multi-root page using identical
   root IDs, anchors, yaw, growth and road masks. Verify sparse mass across
   brick borders, mip parents and terrain seams. Test deliberate capacity
   overflow. No camera-dependent root membership or missing overhang.
3. **Shared four-view residency.** Add a bounded page cache and hierarchy walk;
   measure requested/resident/missing keys, duplicate requests, evictions,
   peak bytes and generation time. Identical views must share page payload;
   four separated players must stay within the same declared global cap.
4. **Wind, motion and representation cuts.** Test three wind states, current/
   previous poses, camera cuts and view resize. Keep independent branches and
   fixed roots. Source-only → volume → source must leave no stale output.
   Native and DLSS have separate motion and image acceptance.
5. **Replace the distance cliff.** Replay the user's 160–180 m approach and
   continue beyond 250/500 m, then drive back into the same vegetation. Remove
   the atlas/substrate substitution only when the volume field passes coverage,
   local luminance, silhouette and temporal review. Shadows remain a separate
   explicit gate; volume colour does not establish correct shadow casting.

Use fixed-seed, fixed-wind source references at native 4K and four 1080p-sized
panels. Include low sun, noon and night; compare vegetation masks and local
regions rather than whole-frame averages. Record maximum and distributional
coverage/luminance errors, actual selected mips, traced pixels, traversal
steps, volume samples, occupied-cell fetches and AABB tests. Root/pixel counts
must distinguish shared residency from per-view shading work.

Performance acceptance uses steady GPU pass timings and complete frame cost
on the same route, with generation/uploads inside the measured workload.
Readbacks, startup and warmup are separate. CPU CI load is not a GPU speed
claim. The target remains one 4K output, four players and 120 real FPS; this
plan does not establish that target or a complete Nanite implementation.
