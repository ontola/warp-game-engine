# GPU foliage selection core, 3 November 2026

Experimental, callable only: no gameplay defaults changed. Files:
`render/src/foliage_select.rs` and `render/src/shaders/foliage_select.wgsl`.

## What this implements

Shared immutable preorder hierarchies (48 bytes/node), permanent instance IDs
(80 bytes/instance), GPU frustum rejection and geometric screen-error cuts,
compact selected records and compact deferred subtrees. Historical reversed-depth
HZB can only defer work, never finally reject it. The second phase retests against
current depth and appends recovered records. GPU-written indirect arguments live
in a separate buffer from read/write counters, as required by wgpu usage rules.
There is no synchronous readback or CPU nearest-tree shortlist in this path.
Readbacks exist only in the diagnostic tests.

The hierarchy uses escape indices, without a fixed-depth shader stack. Nodes are
shared; there is no per-instance expanded branch hierarchy. Traversal currently
uses one invocation per instance in phase one and per deferred subtree in phase
two. This is NOT a load-balanced persistent cluster work queue, page streaming,
or a complete source/voxel representation cooker.

Geometry/aggregate error and representation IDs are caller-supplied. A production
adapter must preserve the existing cooker's optical error policy, material,
volume and motion identity; arbitrary geometric error alone is not proof of
foliage appearance equivalence.

## Safety and consumer contract

- Parent bounds enclose children. Instance scale bounds and wind envelopes are
  conservative. Invalid preorder ranges, non-finite inputs and undersized scale
  bounds are rejected before allocation.
- Bounds crossing projection limits or outside the historical view cannot be
  declared occluded. Sky/zero depth in the footprint prevents rejection.
- Deferred-list overflow traverses without historical rejection. Output-list
  overflow marks that instance for its resident coarse root; consumers MUST skip
  all its partial selected records and use that root once. Capacity therefore
  changes fallback quality, not whether that tree exists.
- For this prototype, phase-one records may seed depth, but final color must be
  consumed after phase two and its fallback flags. Appending final color blindly
  would leave duplicate partial subtrees after a phase-two overflow.
- Each view owns its selector/scratch. A selector is encoded once per submitted
  frame. The caller supplies the matching previous matrix/HZB and current HZB,
  and invalidates unsuitable history. Visibility is never reused to drop
  off-camera shadow casters.
- Instances and recipes are immutable in this initial API. Live pose updates,
  resource residency, stable motion history and streaming invalidation still
  need integration.

## Validation actually run

`cargo test --locked -p render --lib foliage_select::tests -- --include-ignored --nocapture --test-threads=1`

Three tests passed on the Windows GPU setup: input/bounds validation, Naga shader
validation, and real GPU dispatch/readback. The GPU fixture used 1,025 instances
sharing one three-node hierarchy with distinct root/child representation IDs.
It checks exact selected IDs, generations, complete cuts and indirect counts.

| Fixture | Selected records attempted | Nodes tested | Root fallbacks |
|---|---:|---:|---:|
| No history, detailed cut | 2050 | 3075 | 0 |
| Historical blocker removed in current depth | 2050 | 4100 | 0 |
| Blocker remains in current depth | 0 | 2050 | 0 |
| Deferred capacity forced to one | 2050 | 3076 | 0 |
| Coarse screen-error cut | 1025 | 1025 | 0 |
| Output capacity forced to five | 2050 | 4100 | 1025 |

The last row retains at most five records in storage; all marked instances must
use their root instead. Atomic append order is intentionally unspecified.

These are synthetic selection correctness tests, not rendered forest images,
performance acceptance or a comparison with the old CPU pool. No game window was
opened. No new 8.33 ms claim. No shader timing is presented as whole-frame FPS.

## Remaining integration

The current completion contract is
[no CPU pool integration](foliage-no-cpu-pool-integration.md). Do not mark this
work complete on the strength of the tests below: `terrain_fly` still uses the
legacy CPU pool.

### GPU branch residency checkpoint

`foliage_residency::Residency` now consumes final selector records directly on
GPU, with retained slots keyed by tree/node/generation. GPU hashing, retaining,
free-list construction, allocation and final publication are separate ordered
dispatches. No host-visible slot map or nearest-tree decision is used. The
selector's indirect record count drives the request passes. Failed hash/slot
allocation extends the same per-tree fallback flags; final publication removes
all partial records of failed trees. Duplicate requests emit only once.

Slot state explicitly distinguishes occupation, previous published history and
current publication. Replacement and waking after a fallback frame invalidate
history, including the existing packed virtual-member epoch bit. Storage is
32 bytes per resident slot plus bounded request hash/maps and free-list scratch;
it does not expand every world instance's branches. Each view owns mutable
residency; source/voxel geometry may remain shared.

Validation: 100 foliage-filtered tests passed, including actual GPU allocation
for 3,600 synthetic branch records / 1,200 tree IDs, reversed request ordering,
half-population replacement, generation changes, forced five-slot exhaustion,
recovery after fallback, 512 duplicate requests and an empty frame. The existing
1,025-instance selector GPU fixture now directly chains both selection phases
into residency on one encoder, testing disocclusion and selection overflow with
no CPU readback between those operations. These are correctness fixtures, not
rendered forest images, FPS measurements or completed draw integration.

1. Cook shared selection/representation tables from real plant assets; preserve
   measured optical source/voxel errors and conservative animated envelopes.
2. Replace packed movable-owner identity with stable instance/member addressing
   across raster, resolve, motion and independent shadow selection. A bounded
   transient owner table must be produced on GPU, with explicit resident fallback.
3. Feed compact records to source and voxel indirect work, build actual current
   HZB between phases, and remove CPU `Pool::update` from the experimental route.
4. Compare fixed camera and moving scenes, including disocclusion, wind and
   overflow. Measure three alternating repeats before any default switch.
5. Add page residency and work scheduling; then lighting and shadow-cache budget
   work. The terrain CPU issue remains separate and unresolved.

## Real cooked-plant adapter and renderer ABI follow-up

`vegetation_asset::CookedPlant::shared_selection_recipe` exports a shared table
of `SourceBranch { branch, branch_id }` and `Voxel { branch, branch_id, level }`.
These are recipe identities, not per-camera owner slots. It uses the existing
cooked voxel coverage/color measurements and spatial/motion errors. Disallowed
voxel candidates are absent; exact source leaves remain. A root marked
fallback-only cannot satisfy regular LOD selection, even with a huge pixel-error
threshold. Its existing whole-subtree volume is reserved for explicit overflow.
The adapter currently omits measured simplified-triangle cuts, and does not
promise the same choice as the older selector when several alternatives qualify.

Parent spheres enclose branch envelopes including ancestor wind. Placement
scale bounds handle shear; cooked geometry and voxel bytes remain untouched.
Tests check source IDs, optical admission and original vertices over five
articulated poses. A real GPU test selects 1,200 copies of a small cooked recipe:
4 shared nodes yield 3,600 source records; 10 shared nodes with permissive optics
yield 1,200 whole-subtree voxel records. Zero fallbacks in either case.

`virtual_member.wgsl` is now the single immutable addressing/composition helper
for production cull, raster (including shadow raster), shading and motion.
The existing packed IDs and 80-byte instance layout have NOT changed. The former
`pad` field in raster/shade now consistently means `flags`. The helper is compiled
in, not independently hot-reloaded while other consumers keep an older ABI.

Validation: 49 shader-related tests passed. The real-GPU
`virtual_source_matches_expanded_gpu` test renders source instances versus shared
virtual branch templates at wind times 0, 1 and 1.4 seconds: RGBA MAE 0 and zero
changed pixels in each 128x128 image. This exercises actual cull/raster/shade;
it is not a full moving-camera/motion-vector/shadow regression suite or a forest
benchmark. No playtest window was opened.

The new selection remains disconnected from production draw ownership. Next is
GPU allocation of bounded owner/pose records plus stable identity/history across
reassignment; then connect source and voxel draws and remove CPU near-tree
assignment from the opt-in route. Lighting and animated shadow caching remain
separate unfinished milestones.
