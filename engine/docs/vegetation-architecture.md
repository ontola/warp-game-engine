# Warp vegetation: implementation checklist

Updated 4 November 2026. **This is the single authoritative checklist.**
Older reports and `engine/HANDOFF.md` contain supporting history, not separate
plans. Architecture first; no interactive playtest until requested.

## Where we are

**Overall status: incomplete.** The normal renderer uses GPU vegetation, but it
now has an integrated nested geometry hierarchy, but still lacks budget-aware
refinement and upstream visibility needed to scale. The current image and performance are not accepted.

**Current item: G1.3 — integrate cooked groups into GPU selection and paging.**
Next: G1.4 (integrated correctness/performance), then G2 (budget refinement). Do not add another
rendering backend or spend this phase tuning colors to disguise missing detail.

Latest measured route: lowland + grass, 1280x720, one view, 240 frames at 1 m/s,
120 warm GPU samples per run. Same executable, three serial runs: covered GPU
medians **14.79 / 14.70 / 15.11 ms**; selection **3.42 / 3.48 / 3.51 ms**.
Previous three runs: 15.57 / 15.93 / 15.77 ms; selection 5.22 / 5.15 / 5.02 ms.
These were not alternating before/after runs, so no accepted comparative speedup.
The older pre-filtered baseline was ~11.87 ms: the current path is still slower.
Final first-run diagnostics: 151,058 active roots, 4,096 branch slots, 46,270
emergency fallbacks (previous 82,097). Image still has rounded coarse crowns and
heavy black shadows; quality is not accepted. Evidence:
`build/playtest-unified/nested-groups-times.json.gpu.json`, `nested-groups-repeat{2,3}.json.gpu.json`,
matching logs and images. These artifacts are ignored; results are recorded here.
Executable SHA256: `60ad922af9c3ef247e574cf2b7c676731aebfa084ac1b0ec5fa697804c17d71a`.

Per-pass upper-middle medians (ms; medians need not sum to the covered median):

| Pass | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| GPU forest selection residency | 3.422 | 3.482 | 3.512 |
| GPU forest shared poses | 0.102 | 0.103 | 0.106 |
| GPU forest materialize | 0.140 | 0.141 | 0.141 |
| shadow tile damage and clear | 0.302 | 0.304 | 0.306 |
| shadow cull | 0.527 | 0.530 | 0.530 |
| shadow raster | 0.134 | 0.134 | 0.134 |
| voxel sun splats | 0.737 | 0.745 | 0.747 |
| voxel sun splat raster | 0.540 | 0.543 | 0.543 |
| near shadow cull | 0.321 | 0.323 | 0.323 |
| near shadow raster | 0.032 | 0.033 | 0.033 |
| voxel near sun splats | 0.220 | 0.222 | 0.222 |
| voxel near sun splat raster | 0.070 | 0.070 | 0.070 |
| tree occluder cull | 0.530 | 0.508 | 0.518 |
| tree occluder raster | 0.213 | 0.213 | 0.214 |
| tree occlusion pyramid | 0.087 | 0.087 | 0.089 |
| tree recovery dispatch | 0.002 | 0.003 | 0.003 |
| tree remaining cull | 0.010 | 0.010 | 0.010 |
| tree remaining raster | 0.001 | 0.001 | 0.001 |
| tree visibility history | 0.030 | 0.030 | 0.030 |
| voxel splats | 2.903 | 2.722 | 3.035 |
| voxel splat raster | 2.735 | 2.799 | 2.821 |
| voxel splat resolve | 0.103 | 0.105 | 0.105 |
| terrain coarse | 0.100 | 0.100 | 0.100 |
| shade terrain | 0.424 | 0.426 | 0.427 |
| shade voxels | 0.073 | 0.073 | 0.073 |
| shade | 0.079 | 0.080 | 0.080 |
| motion vectors | 0.034 | 0.034 | 0.034 |
| taa | 0.045 | 0.045 | 0.045 |
| exhaust | 0.001 | 0.001 | 0.001 |
| wheel particles | 0.000 | 0.000 | 0.000 |
| bloom | 0.030 | 0.031 | 0.032 |
| tonemap | 0.039 | 0.039 | 0.039 |

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

- [x] G1.1 — Cook a deterministic spatial hierarchy independently of wind joints.
  Parent membership must be the exact disjoint union of children; original source
  IDs and shared primitives are retained. Evidence: `97cf333`, deterministic split and exact source-partition tests.
- [x] G1.2 — Cook internal-group voxel assets, optical errors, support and discarded
  motion bounds in a shared ancestor frame. Evidence: `97cf333`; all 19 CPU asset tests
  passed, including animated displacement and cache round trip. Normal production
  CPU cook passed for six tree species and two grass cells. This checks the cook,
  NOT GPU integration or rendered LOD quality.
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
- 4 Nov, `97cf333`: G1.1/G1.2 checked. Balanced species-local groups preserve
  exact source partition and reuse existing articulated frames (no extra joints).
  Intermediate groups have voxel assets and measured error/motion bounds.
  19 CPU asset tests pass; four GPU tests not run for this cook-only change.
  Production tree group counts: 113/121/121/121/81/81; grass: 71/71.
  Cold scene cook 88.48 s; cached scene load 2.73 s. Logs:
  `build/playtest-unified/geometry-groups-cook.log` and `geometry-groups-cached.log`.
  G1.3 remains unchecked: the current selector still uses the old flat branch cut.
- 4 Nov, G1.3 integration: normal selector exports recursive geometry groups; paging
  and materialization address their own volume assets while reusing existing poses.
  Fixed source-pick diagnostics incorrectly decoding voxel visibility IDs as triangle
  IDs. Cold-page fixtures wait for residency without advancing wind time; original
  image/source assertions retained. All 23 asset and 15 runtime tests pass.
  First route sample: 14.79 ms covered GPU, selection 3.42 ms, 46,270 emergency
  fallbacks. This is preliminary, not accepted performance or visual quality.
  Test evidence: build/playtest-unified/nested-groups-asset-final.log; normal-route
  evidence: nested-groups-times.json.gpu.json and nested-groups-gpu.log in that folder.

- 4 Nov, G1.4 evidence started: 11 selector tests pass; two additional serial
  normal-route runs produce 14.70 / 15.11 ms covered GPU. Final JPEG inspected:
  coarse rounded crowns and harsh black shadows persist. Mixed-cut motion/depth
  acceptance and alternating comparisons remain outstanding; G1.4 is NOT checked.
