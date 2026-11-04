# Warp vegetation: implementation checklist

Updated 4 November 2026. **This is the single authoritative checklist.**
Older reports and `engine/HANDOFF.md` contain supporting history, not separate
plans. Architecture first; no interactive playtest until requested.

## Where we are

**Overall status: incomplete.** The normal renderer uses GPU vegetation, but it
still lacks the integrated geometry hierarchy, budget refinement and visibility
pipeline needed to scale. The current image and performance are not accepted.

**Current item: G1.2 — validate cooked intermediate geometry groups.**
Next: G1.3 (GPU integration), then G2 (budget refinement). Do not add another
rendering backend or spend this phase tuning colors to disguise missing detail.

Latest measured route: lowland + grass, 1280x720, one view, 240 frames at 1 m/s,
120 warm GPU samples per run. Three runs: covered GPU medians
15.57 / 15.93 / 15.77 ms; selection 5.22 / 5.15 / 5.02 ms.
The preceding pre-filtered baseline was approximately 11.87 ms covered GPU.
This is a regression, not progress toward the performance target.
Final diagnostics: 151,058 active plant roots (including grass cells), 4,096
branch slots, 82,097 emergency fallbacks. Evidence:
`build/playtest-unified/projection-cache*.json.gpu.json` and corresponding logs.
These local artifacts are ignored by Git; measurements are recorded here.

## Rules for checking boxes

- `[x]` means the stated, narrow item has evidence. It does not complete its parent gate.
- `[ ]` means missing, in progress, or not sufficiently verified. Label ongoing work explicitly.
- An integrated gate needs normal-renderer evidence, lifecycle/overflow checks,
  images in motion and pass timings. A standalone module or passing unit test
  cannot complete an integration gate.
- Keep IDs stable. Record the commit, relevant test/run, result and remaining
  limitation when checking an item. Reopen it if a regression contradicts it.
- Every progress update names the item worked on, what changed, what was checked,
  and the next item. Do not substitute activity or new modules for completed gates.
- Do not change the target or silently remove requirements to make the checklist green.

## Foundations already connected

- [x] F1 — Normal terrain demo uses GPU selection without a CPU near-tree detail pool.
  Evidence: `foliage_runtime.rs`, `terrain_fly`; normal route measurements above.
- [x] F2 — Explicit camera/shadow consumer roles, including orthographic cameras
  and perspective lights. Evidence: `c563b33`, selector consumer-role GPU fixtures.
- [x] F3 — Bounded GPU frontier with tested overflow recovery and local unary-chain
  traversal. Evidence: `bb889e9`, `c8b2b61`; all 11 selector tests passed.
- [x] F4 — Stable root IDs/generations and queued cell replacement without rebuilding
  the whole renderer. Evidence: `foliage_population.rs`, `foliage_world.wgsl`,
  lifecycle fixtures; capacity growth remains G7.2.
- [x] F5 — Terrain grass enters the same population/selection machinery as trees.
  Evidence: `5dce9e3`; 3x3 patch cells and mixed species in normal route.
  This does NOT establish acceptable grass LODs, lighting or performance.

## G1 — Nested geometry hierarchy (NOT COMPLETE)

- [ ] G1.1 — Cook a deterministic spatial hierarchy independently of wind joints.
  Parent membership must be the exact disjoint union of children; original source
  IDs and shared primitives are retained. **Implemented locally; not committed yet.**
- [ ] G1.2 — Cook internal-group voxel assets, optical errors, support and discarded
  motion bounds in a shared ancestor frame. **In progress:** new coverage test and
  19 CPU asset tests passed; production lowland/grass cook still running.
- [ ] G1.3 — Export nested groups into the NORMAL GPU selector, asset page tables,
  materialization and shadow representations. No synthetic extra wind joints,
  duplicate geometry or new backend. Current selector still flattens branch chains.
- [ ] G1.4 — Validate mixed coarse/fine cuts against original source membership,
  animated bounds, depth and image sequences. Repeated normal-route measurements
  must show the cost and fidelity of this integration, not just the cook.

## G2 — Budget-aware refinement (NOT COMPLETE)

- [ ] G2.1 — Start from a complete resident cut; refine by projected benefit and
  incremental cost. Retain intermediate parents when refinement cannot fit.
  Replace the current whole-plant all-or-nothing admission.
- [ ] G2.2 — Publish a replacement only when ALL required children are resident
  and allocated. Report actual achieved error when budgets prevent requested detail.
  Do not hide this behind larger pools or relaxed optical thresholds.
- [ ] G2.3 — Test undersized budgets, deterministic priority, teleport and changing
  demand. Show fallback counts and representation distributions during motion;
  emergency proxies must not remain the normal appearance of the forest.

## G3 — Consumer-aware two-phase visibility (NOT COMPLETE)

- [x] G3.1 — Downstream per-view source history/recovery exists.
  Evidence: `foliage-history-2026-11-04.md`; this runs AFTER shared residency.
- [ ] G3.2 — Move safe history-first visibility into shared hierarchy traversal;
  build current depth and recover disoccluded groups before final publication.
- [ ] G3.3 — Preserve off-camera shadow casters and independently choose consumer
  detail. One demanding shadow/camera must not force every consumer to rasterize
  its finest cut. Shared residency remains a single ownership domain.
- [ ] G3.4 — Prove recovery for four cameras, turns, teleports, hidden back rows,
  queue overflow and rejected publication. No depth from a failed partial cut
  may survive. Compare raw visibility/depth, not only screenshots.

## G4 — Bounded paging and lifecycle (NOT COMPLETE)

- [x] G4.1 — Bounded physical source/voxel caches, async demand, incremental uploads,
  generation-safe eviction and resident-ancestor fallback are connected.
  Evidence: `foliage-cache-2026-11-04.md`, `foliage-source-pages-2026-11-04.md`,
  `05042af` root pinning and normal cold-cache fixture.
- [ ] G4.2 — Page the new geometry groups through the same owner; account for all
  bytes and keep permanent fallback pins bounded. Validate an undersized cache.
- [ ] G4.3 — Refine page granularity beyond whole branch/CLOD/mip bundles where
  needed; validate fragmentation/reclamation and asset-content replacement.
- [ ] G4.4 — Verify no holes, stale generations or full renderer rebuilds across
  eviction, removal/reinsertion, empty world and heavy oversubscription.

## G5 — Shared wind and animated shadow caching (NOT COMPLETE)

- [x] G5.1 — Demand-deduplicated ancestor poses and indirect depth dispatches are
  connected. Source, voxel and shadow paths share poses; bounded fallback exists.
  Evidence: runtime pose fixtures; cached-sun raw-depth oracle passed six steps.
- [x] G5.2 — Shared sun tile damage, sparse clear and exact integer XY scrolling
  are connected. Evidence: runtime cached/full sun depth oracle and prior handoff.
- [ ] G5.3 — Extend pose/frame mapping and shadow invalidation to geometry-group
  replacement and animated group bounds; verify source/voxel/motion agreement.
- [ ] G5.4 — Complete virtual shadow coverage and independently budgeted caster
  detail; measure cache reuse with wind, changing sun and moving consumers.
- [ ] G5.5 — Compact inactive pose/root work and scale animation work by visible
  error while retaining stable distant wind. Prove savings with pass timings.

## G6 — Unified vegetation and extreme-distance aggregates (NOT COMPLETE)

- [ ] G6.1 — Accept grass/tree source-to-voxel transitions in motion: coverage,
  silhouette, color, normals and shadow density must remain comparable. No green
  sheets, visible ball crowns, abrupt darkening or unexplained flicker.
- [ ] G6.2 — Integrate hierarchical spatial-cell aggregates at extreme distance,
  removing mandatory per-tree records there. Stream through the same lifecycle.
- [ ] G6.3 — Validate grass attachment on curved/sloping ground and basal wind;
  migrate remaining game meadow paths to the common representation machinery.
- [ ] G6.4 — Integrate reusable species/growth/season variants and asset updates;
  verify primitive sharing, stable identities and animated representation bounds.

## G7 — World scalability (NOT COMPLETE)

- [ ] G7.1 — Demonstrate bounded GPU/CPU memory and work as world population grows;
  publish root/group/cluster/voxel counts at multiple distances.
- [ ] G7.2 — Grow/stream root capacity safely without reassigning surviving IDs or
  rebuilding the renderer. Current reserved capacity rejects excess roots.

## G8 — Acceptance and delivery (NOT COMPLETE)

- [ ] G8.1 — Run normal-route regression matrix: empty world, oversubscription,
  teleport, four cameras, off-screen casters, cell replacement, eviction, wind,
  changing sun, source/voxel transitions and motion history.
- [ ] G8.2 — Capture comparable moving views and raw buffers for suspected
  correctness faults. Fix missing geometry and invalid lighting before polish.
- [ ] G8.3 — Measure traversal, residency, poses, source/voxel raster, shadows and
  shading separately; exclude warmup/startup/readback. Use at least three serial
  repeats and alternating before/after runs for performance acceptance. Record
  hardware, resolution, reconstruction, scene and executable/commit identity.
- [ ] G8.4 — Demonstrate **4K output, four players, 120 FPS / 8.33 ms**, with DLSS
  mode and internal resolutions stated. Current single-view 720p data does not
  establish this. Any target change requires explicit user agreement.
- [ ] G8.5 — Commit/push the integrated work to main, update this checklist with
  evidence, and audit every unchecked requirement before claiming completion.

## Non-negotiable design contracts

CPU work may author/cook species, generate deterministic placements, upload assets
and stream cells. It must not choose detailed trees per frame or await GPU counts.
Geometry-group identity, articulated joint identity, asset page identity and
transient draw-slot identity are distinct. Missing detail retains a complete
resident ancestor. Growth variants share primitive assets. Physics remains
120 Hz/four substeps. No renewed Godot comparison.

## Progress log

- 4 Nov, `c8b2b61`: shared unary-bound projections; 11 selector tests passed;
  three measured runs remain too slow. Identified missing geometry groups.
- 4 Nov, local/uncommitted: G1.1/G1.2 topology and aggregate cook added in
  `vegetation_geometry.rs`; source partition fixture passes. Production cook
  pending. G1.3 is explicitly not implemented; no visual/FPS improvement claimed.