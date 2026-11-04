# Warp vegetation: implementation checklist

Updated 4 November 2026. **This is the single authoritative checklist.**
Older reports and `engine/HANDOFF.md` contain supporting history, not separate
plans. Architecture first; no interactive playtest until requested.

## Where we are

**Overall status: incomplete.** Nested geometry groups, GPU incremental complete-cut
admission, paging, shared poses and shadow caching are integrated. G2.1 is done;
fine-detail admission, upstream two-phase visibility and scaling/performance
acceptance are not. The current crowns and performance are not accepted.

**Current work: G2 selection cost and non-monotonic refinement costs**, followed
by G2.2/G2.3 validation and G3 consumer visibility/detail. No new backend or
appearance tuning to hide missing architecture.

Latest committed route result (c619265): native1280x720, one view, lowland+grass,
240frames at1m/s,120warm GPU samples. Three alternating old/new repeats measured
new11.382/11.574/11.511ms covered GPU, versus11.909/11.917/12.072ms. Selection
still costs~5.24ms. Camera splats1,361,047,0 missing records,0 emergency fallbacks;
near crowns remain too coarse and source slots are0. This is NOT4K/four-view
acceptance. Root-stage compaction was measured and rejected; see the experiment below. Older measurements are historical, not the current build's performance.

## Immediate performance decision (existing G1.4/G8 evidence, not new gates)

- [ ] Measure voxel rasterization at ~1 pixel projected support, including splat
  generation, overdraw, visibility/resolve, and shadow work. Report submitted and
  surviving counts; compare the identical view with a detailed reference.
- [ ] Same executable, alternating native/DLSS Quality/Balanced/Performance route
  runs, at least three per mode. Record actual input/output sizes, per-pass medians,
  reconstruction cost and comparable images. No silent fallback.
- [ ] Apply the following provisional TOTAL frame allocation to these measurements,
  then proceed to G2. These are engineering targets, not achieved measurements.

| Category (all four views combined at 4K output) | Budget ms |
| --- | ---: |
| Vegetation selection, paging, poses and materialization | 0.50 |
| Vegetation raster, visibility and resolve | 1.50 |
| Vegetation shadows | 1.50 |
| **Vegetation subtotal** | **3.50** |
| Terrain | 1.50 |
| Lighting / indirect illumination | 2.00 |
| TAA / upscale / post | 1.30 |
| **Allocated frame total** | **8.30** |
| Remaining margin to 8.33 ms | 0.03 |

User-proposed allocation adopted. Non-vegetation geometry, shadows, effects and UI
must be explicitly accounted for within the total; they are not free or omitted.
The margin is effectively zero, so these are ceilings rather than comfortable
steady-state targets. Per-category instrumentation must avoid double-counting.

A pass missing its allocation is a recorded gap, not a reason to hide or discard
its measurements. 720p single-view runs are diagnostic only. Share matched images
with measurements; continue to label the current appearance as unaccepted.

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

- [x] G2.1 — Start from a complete resident cut; refine by projected benefit and
  incremental cost. Retain intermediate parents when refinement cannot fit.
  Replace the current whole-plant all-or-nothing admission.
- [ ] G2.2 — Publish a replacement only when ALL required children are resident
  and allocated. Report actual achieved error when budgets prevent requested detail.
  Do not hide this behind larger pools or relaxed optical thresholds.
- [ ] G2.3 — Test undersized budgets, deterministic priority, teleport and changing
  demand. Show fallback counts and representation distributions during motion;
  emergency proxies must not remain the normal appearance of the forest.

G2 implementation in progress: normal consumer-union selection now starts from
resident proxies and replaces complete parents incrementally. Stable priority
prefixes reserve added branch slots and conservative camera/shadow splat counts;
all visible child pages and output ranges must be available before publication.
Missing pages or exhausted budgets retain the current intermediate parent.
Terminal leaves publish directly; non-increasing unary chains collapse locally.
Camera refinement precedes shadow-only upgrades. No CPU tree shortlist or mode
switch is introduced. Diagnostics report reserved raster work, budget stops and
allocation faults; branch-demand counters now explicitly mean published branches.

Shared cooked nodes carry camera/shadow splat bounds in their existing GPU
metadata. Whole-root fixed-mip choices, articulated adaptive mip ranges, source
branches with voxel shadow casters, and separate emergency proxies are accounted
independently. Streaming preserves proxy costs. These are voxel-record bounds,
NOT source triangle costs or GPU-time estimates. If even the resident baseline
exceeds capacity, the allocator reports that fault; far aggregation is still needed.

G2.1 is implemented; G2.2/G2.3 remain open pending performance, images, achieved-error
reporting and broader pressure validation. A complete retained ancestor is not
claimed visually equivalent to the old separate emergency proxy. The 720p
normal-route overflow must be checked again against actual indirect draw counts.

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

- G1.4: GPU threshold sweep (130 cuts, forward/back) preserves every original
  source member exactly once and naturally reaches source, internal group, mixed
  and whole-root cuts. `shared_selection_nested_cut_preserves_source_coverage` PASS.

## DLSS diagnostic sweep � 4 November

Same DLSS-enabled executable, 12 serial runs in native/Quality/Balanced/Performance
order repeated three times. Lowland+grass, one view, 1280x720 output, 240 frames
at1m/s; 120 warm GPU samples per run. No frame generation. NOT a 4K/four-view
acceptance test. Actual reconstructed sizes reported by successful evaluations.

| Mode | Internal size | Covered GPU medians ms (three runs) |
| --- | --- | --- |
| native | 1280x720 | 15.435 / 15.916 / 16.505 |
| dlss-quality | 853x480 | 17.106 / 13.347 / 15.362 |
| dlss-balanced | 742x418 | 16.306 / 12.618 / 15.208 |
| dlss-performance | 640x360 | 16.240 / 15.703 / 14.940 |

Per-pass median of the three run medians (ms; do not sum medians as a frame time):

| Pass | Native | Quality | Balanced | Performance |
| --- | ---: | ---: | ---: | ---: |
| GPU forest selection residency | 3.603 | 3.531 | 3.543 | 3.524 |
| GPU forest shared poses | 0.105 | 0.104 | 0.104 | 0.104 |
| GPU forest materialize | 0.142 | 0.142 | 0.141 | 0.141 |
| shadow tile damage and clear | 0.306 | 0.307 | 0.306 | 0.306 |
| shadow cull | 0.529 | 0.530 | 0.528 | 0.529 |
| shadow raster | 0.134 | 0.134 | 0.134 | 0.134 |
| voxel sun splats | 0.746 | 0.750 | 0.742 | 0.746 |
| voxel sun splat raster | 0.543 | 0.543 | 0.543 | 0.544 |
| near shadow cull | 0.323 | 0.323 | 0.323 | 0.323 |
| near shadow raster | 0.033 | 0.032 | 0.033 | 0.033 |
| voxel near sun splats | 0.221 | 0.221 | 0.220 | 0.220 |
| voxel near sun splat raster | 0.070 | 0.070 | 0.070 | 0.070 |
| tree occluder cull | 0.521 | 0.526 | 0.528 | 0.536 |
| tree occluder raster | 0.214 | 0.206 | 0.204 | 0.202 |
| tree occlusion pyramid | 0.089 | 0.075 | 0.074 | 0.073 |
| tree recovery dispatch | 0.003 | 0.002 | 0.002 | 0.002 |
| tree remaining cull | 0.010 | 0.010 | 0.010 | 0.010 |
| tree remaining raster | 0.001 | 0.001 | 0.001 | 0.001 |
| tree visibility history | 0.030 | 0.015 | 0.012 | 0.010 |
| voxel splats | 3.124 | 3.027 | 3.018 | 3.153 |
| voxel splat raster | 2.877 | 2.826 | 2.835 | 2.924 |
| voxel splat resolve | 0.104 | 0.056 | 0.046 | 0.037 |
| terrain coarse | 0.100 | 0.060 | 0.050 | 0.043 |
| shade terrain | 0.425 | 0.304 | 0.247 | 0.194 |
| shade voxels | 0.073 | 0.063 | 0.053 | 0.046 |
| shade | 0.079 | 0.061 | 0.044 | 0.038 |
| motion vectors | 0.034 | 0.019 | 0.016 | 0.014 |
| taa | 0.045 | 0.000 | 0.000 | 0.000 |
| exhaust | 0.001 | 0.000 | 0.000 | 0.000 |
| wheel particles | 0.000 | 0.000 | 0.000 | 0.000 |
| bloom | 0.031 | 0.031 | 0.031 | 0.031 |
| tonemap | 0.039 | 0.021 | 0.022 | 0.021 |
| dlss | 0.000 | 0.478 | 0.477 | 0.440 |

Image review: matched final views retain rounded coarse crowns and heavy black
shadows in every mode. Lower internal resolution reduces shade/resolve work but
not the dominant vegetation selection/splat/raster costs enough to help this
720p scene. DLSS has its own cost; this does not prove it cannot help at4K.
Next: near-one-pixel raster feasibility, then target-resolution/split validation.
Evidence: build/playtest-unified/upscale-*.json.gpu.json, logs and PNG pixel
references; JPEG contact sheet upscale-comparison.jpg. No gameplay-FPS claim.
Executable SHA256: `d0ecd969686f5e48216a6d624a963b20b6e84815780be3dbf7d3ede3210c8b2e`.

## One-pixel raster feasibility � 4 November

`render/examples/voxel_raster_budget.rs` uses the unchanged production voxel
vertex/fragment shader and main-pass depth/visibility formats. RTX5070Ti, Vulkan,
1920x1080 target (one native quadrant of4K); three serial repeats,12 GPU timestamp
samples after3 warmups per case. Includes target clears. Excludes selection, splat
generation, materials, lighting, shadows and reconstruction: NOT whole-game FPS.
Packed cells form a synthetic grid; four-million case repeats four coplanar layers.
Larger supports can clip against the viewport; footprint changes also alter the
covered region. No real-forest quality equivalence is implied. Raw checks verify
every written visibility ID agrees with the splat ID and expected depth. JPEG
previews are ID colors, not shaded vegetation.

| Submitted splats | Triangles | 0.5px ms | 1px ms | 2px ms | 4px ms |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 65,536 | 131,072 | 0.0338 | 0.0344 | 0.0350 | 0.0348 |
| 262,144 | 524,288 | 0.1316 | 0.1323 | 0.1329 | 0.1327 |
| 1,048,576 | 2,097,152 | 0.5227 | 0.5235 | 0.5233 | 0.5227 |
| 4,194,304 | 8,388,608 | 2.0878 | 2.0895 | 2.0912 | 2.0891 |

Cost scales almost linearly with submitted splats and barely with pixel diameter
in this fixture. This is consistent with vertex/submission-side work dominating;
not proof from hardware counters. Four views each submitting1M splats would
exceed the1.5ms vegetation raster allocation on a linear extrapolation, BEFORE
generation/resolve. Actual four-view behavior still requires a measurement.
Next: correlate real submitted counts and projected footprints; investigate a
small-splat compute path and/or reducing redundant submissions, preserving raw
depth/visibility correctness. This does not complete the broader raster decision:
full generation/shadow cost and source/voxel image comparisons remain open.
Evidence: build/playtest-unified/voxel-raster-probe/results.json and support-*.jpg.

### Normal-route correlation: confirmed raster buffer overflow

Final-frame diagnostic of the normal240-frame lowland+grass route (720p,oneview):
4,495,774 requested/indirect-drawn splats, buffer capacity4,194,304,
26,974,644 submitted vertices. **301,470 missing records (6.7% of demand)**.
The production generator increments the draw count before checking storage bounds;
the vertex shader guards invalid accesses but cannot draw records never stored.
This is a correctness failure, not merely a slow draw. Earlier zero frontier/slot
overflow counters did NOT cover this independent raster buffer. Do not accept
normal-route fidelity until fixed. Synthetic probe fits its buffer and therefore
cannot establish whole-renderer overflow correctness.

G2 admission must budget raster records/work as well as transient branch slots.
Keep a complete cheaper resident cut when a refinement exceeds the raster budget;
never silently truncate, merely clamp statistics, or claim that larger buffers
solve bounded scaling. Also establish which projected-size bins consume the
records before choosing a small-splat compute path. No new gate is needed.
Evidence: build/playtest-unified/raster-count-route.log.

### Stored-splat footprint profile

Normal240-frame720p route, explicit `--voxel-raster-profile` offline readback:
4,495,775 requested,4,194,304 stored,301,471 missing.20,147 distinct stored volumes.
Bins are screen-aligned diameter; omit the production sub-cell jitter displacement.
Overflow records have no payload and are excluded, so this is NOT a histogram of
all demand. This diagnostic adds no per-frame CPU selection/readback dependency.

| Diameter px | Articulated groups | Whole roots (direct + fallback) |
| --- | ---: | ---: |
| <=0.5 | 11,459 | 3,766,133 |
| 0.5�1 | 89,424 | 217,864 |
| 1�2 | 18,359 | 73,124 |
| 2�4 | 1,091 | 12,082 |
| 4�8 | 0 | 3,643 |
| >8 | 0 | 1,125 |

97.4% of stored splats are <=1px. Whole-root records dominate, not articulated
groups. Root materialization publishes exactly one shared-selected mip, so camera
raster cannot adapt that choice to its own resolution. G3 consumer-detail remains
a candidate cause, NOT yet isolated: geometric error also uses2.598*cell+motion,
which can demand sub-half-pixel cells even with one consumer. Do not attribute all
excess detail to shadows without a camera-only comparison. Optical/support error
constraints must survive any changed rule; simply relaxing the error is not proof.
Next: isolate camera vs shadow demand, then implement per-consumer resident cuts
and bounded raster admission within existing G2/G3 requirements.
Evidence: build/playtest-unified/raster-profile-route.log.

### Consumer isolation � shadow demand is not the primary raster cause

Same executable, three alternating normal/camera-only240-frame runs at720p,120
warm GPU samples. Camera-only sets sun_lux=0 and shadow_strength=0, confirmed
shadow_only_roots=0. It changes lighting, so total-frame time/image is NOT a
like-for-like visual comparison. Compare selection and camera raster workload only.

| Mode | Final requested splats | Selection medians ms | Raster medians ms |
| --- | ---: | --- | --- |
| Combined | 4,495,774�4,495,775 | 2.516 / 2.658 / 2.508 | 2.252 / 2.505 / 2.251 |
| Camera only | 4,651,051 | 1.704 / 1.699 / 1.680 | 2.575 / 2.439 / 2.331 |

Removing shadow consumers does not solve excess camera raster work; it increases
splat demand. G3 remains necessary but must not be claimed as the primary cure.
Code inspection: ordinary root aggregate cook inherits just3 branch-local levels,
although the packed representation ABI supports16 and reduction stops at1cell.
Far roots can exhaust their coarse choices while still emitting subpixel records.
A full root pyramid is under test; unchanged optical admission, cook output caps,
branch/internal-group mip counts. Do not claim overflow fixed until normal runs.
Evidence: build/playtest-unified/consumer-{combined,camera}-{1,2,3}.log/json.gpu.json.

### Rejected root-pyramid experiment

Extended only whole-root aggregate cooking from inherited3 levels to the existing
validated8-level limit, stopping at1cell. Optical error policy and all cook caps
unchanged.25 asset tests and15 runtime tests pass. Normal route nonetheless still
requests4,380,565 splats (186,261 missing), versus4,495,774 previously: only2.6%
fewer requests. Three covered GPU medians11.663/11.932/11.941ms; selection
2.600/2.775/2.704ms, splat generation2.366/2.395/2.503ms, raster2.346/2.440/2.451ms.
Timing comparisons are not alternating builds and must not be claimed as a gain.

Visual check rejects the change: fewer detailed near crowns. Coarser support
expands the shared root/chain bounds, changes visible demand and budget allocation:
source slots802 vs880, camera roots41,196 vs40,697, emergency fallbacks46,310
vs46,270. Asset correctness is insufficient for normal-renderer acceptance.
**Production source restored to the pre-experiment default.** Reproducible code
retained in `root-pyramid-experiment.patch`; it is NOT an active backend or option.
Do not reapply as a fix without separating LOD/error support from allocation priority
and implementing bounded incremental refinement. Existing G2/G3 remain the work.
Evidence: build/playtest-unified/root-pyramid-{1,2,3}.log/json.gpu.json/png;
root-pyramid-tests.log, root-pyramid-runtime.log.

### G2 incremental publication: first normal-route measurement (4 Nov)

Normal consumer union now uses GPU complete-cut refinement, not whole-plant
all-or-nothing admission. 139 foliage tests pass, including all exclusive GPU
fixtures. Tests cover deterministic mixed cuts, missing sibling pages, tiny slot
and splat budgets, camera priority, zero-error forced-root priority, unary chains,
frontier exhaustion, teleport, four views, paging and cached-shadow depth.
G2.1 is implemented; G2.2/G2.3 and visual/performance acceptance remain open.
The old proxy-image equivalence assertion no longer applies when a different,
complete ancestor is intentionally retained; the renderer test now requires
complete source-equivalent caster coverage and bounded nonempty raster work.

Before submission batching, three alternating old/new executable route repeats,
identical native1280x720 one-view lowland+grass, seed1,240frames,120 warm samples:

| Pass ms (medians per repeat) | Previous allocator | Incremental allocator |
| --- | --- | --- |
| Covered GPU interval | 11.961 / 12.009 / 12.025 | 12.665 / 12.798 / 12.714 |
| Selection + residency | 2.525 / 2.536 / 2.500 | 6.468 / 6.592 / 6.534 |
| Camera splat generation | 2.494 / 2.541 / 2.404 | 0.681 / 0.672 / 0.682 |
| Camera splat raster | 2.326 / 2.252 / 2.251 | 0.680 / 0.680 / 0.680 |

First diagnostic run:1,361,047 actual camera splats,0 missing (previous4,495,775
and301,471 missing). Reserved camera/shadow bounds4,183,045/4,183,917, both within
4,194,304. No allocation faults, no emergency proxy fallbacks. However, all visible
crowns remain coarse;0 source slots and758 voxel slots, while94,366 whole roots
remain. Costly intermediate adaptive voxel bundles can block the path to source
detail: removing overflow is NOT proof of visual quality. The comparison shows
less excessive black shadow coverage, but round coarse crowns remain unacceptable.
CPU render encoding also regresses (example final sample5.53 to13.73ms), motivating
batching dependent compute dispatches within a pass. No120FPS/4K/four-view claim.
Evidence: build/playtest-unified/incremental-{old,new}-repeat-{1,2,3}.*,
incremental-summary.json, incremental-comparison.jpg, incremental-final-tests.log.

#### Submission batching follow-up (same allocation and quality rules)

Dependent compute dispatches now share a compute pass; indirect-argument copies
remain explicit boundaries. No policy, cap or geometry change. All139 foliage
tests pass again (including21 exclusive GPU tests). Three fresh alternating
old/batched runs with the same route and120 warm samples each:

| Pass ms | Previous allocator | Batched incremental allocator |
| --- | --- | --- |
| Covered GPU interval | 11.909 / 11.917 / 12.072 | 11.382 / 11.574 / 11.511 |
| Selection + residency | 2.512 / 2.506 / 2.525 | 5.216 / 5.257 / 5.237 |
| Camera splat generation | 2.349 / 2.494 / 2.596 | 0.689 / 0.685 / 0.672 |
| Camera splat raster | 2.251 / 2.306 / 2.500 | 0.680 / 0.680 / 0.680 |
| CPU render encoding | 4.878 / 5.149 / 5.101 | 5.991 / 6.750 / 6.920 |

Actual camera records1,361,047 in every new run, within4,194,304; previous4,495,774
overflowed. Batched/unbatched final scene PNG difference excluding HUD has mean
absolute channel error0.0000215/255: observational image check, not raw-buffer
identity or full motion acceptance. Both still show coarse round crowns.
The final allocator is modestly faster in covered GPU time than the prior one,
but selection and CPU encoding remain too expensive and fine source detail is
blocked. Do not extrapolate this720p one-view result to4K/four-view120FPS.

Next substantive work: avoid repeatedly processing shadow-only pending roots and
inactive queue entries; reduce fixed dispatch/scan work; remove non-monotonic
intermediate raster-cost barriers (adaptive mip bounds vs the actually selected
consumer mip). Preserve complete parents and page safety. Add achieved-error
reporting and motion checks before accepting G2.2/G2.3. No new gate or backend.
Evidence: build/playtest-unified/incremental-final-{old,batched}-{1,2,3}.*,
incremental-final-summary.json, incremental-final-comparison.jpg,
incremental-batched-tests.log and incremental-batched-build.log.

## Historical G1.3 integration measurement (superseded)

Historical G1.3 measured route: lowland + grass, 1280x720, one view, 240 frames at 1 m/s,
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


## Rejected root-stage queue compaction (4 November 2026)

Compacting camera-owning roots and shadow-only roots into separate refinement
stages reduced visited nodes and peak frontier, but increased GPU selection cost.
Three alternating before/after repeats used the same native 1280x720 one-view
lowland+grass route, 240 frames, with frames 120�239 measured.

| Repeat | Before covered ms | After covered ms | Before selection ms | After selection ms |
| --- | ---: | ---: | ---: | ---: |
| 1 | 11.526 | 11.967 | 5.224 | 5.666 |
| 2 | 11.388 | 11.755 | 5.250 | 5.558 |
| 3 | 11.386 | 11.880 | 5.240 | 5.504 |

All 139 foliage tests passed, including exclusive GPU fixtures. Camera splats
remained 1,361,047. The matched scene image outside the HUD was virtually
unchanged (each channel MAE below 0.000005 on the 0�255 scale); this is not
a raw visibility/depth equivalence claim or visual acceptance. Extra compaction
and changed publication work did not produce a net gain. Production changes
were removed; `root-stage-compaction-experiment.patch` preserves the experiment.

Evidence: `build/playtest-unified/root-stages-{before,after}-{1,2,3}.*`,
`root-stages-summary.json`, `root-stages-comparison.jpg`, and
`root-stages-tests.log`. Current production remains c619265's allocator.
Next: investigate conservative adaptive-mip reservation and expensive intermediate
representations that block finer detail; do not hide this by increasing budgets.

## Exact camera-mip admission experiment (4 November 2026, rejected)

Camera materialization was changed to render the selector-admitted mip exactly,
rather than choosing a finer mip again. Camera reservations therefore used that
mip's occupied-cell count; shadow reservations remained adaptive. This tested
whether conservative camera reservations were the main fine-detail blocker.

All 139 foliage tests passed. Three alternating native 1280x720 one-view
lowland+grass route repeats (240 frames, frames 120�239 measured) gave:

| Repeat | Before covered ms | After covered ms | Before selection ms | After selection ms |
| --- | ---: | ---: | ---: | ---: |
| 1 | 11.559 | 12.360 | 5.212 | 6.067 |
| 2 | 11.559 | 12.521 | 5.235 | 6.236 |
| 3 | 11.532 | 12.352 | 5.179 | 6.111 |

The first after-run published 4,096 voxel slots instead of 758, but source slots
remained zero. Tested nodes increased from ~532k to 700,391. Actual camera
splats were 1,356,414, versus 1,361,047 before; raster faults/fallbacks remained
zero. Thus lowering camera reservation alone exposes more intermediate traversal,
not useful source detail or a speedup. Matched images were inspected: crowns
remain coarse; RGB MAE outside HUD was 1.014/1.075/0.659 on a 0�255 scale.
This is not visual acceptance or raw-buffer equivalence.

Production changes removed; preserved in `exact-camera-mip-experiment.patch`.
Evidence: `build/playtest-unified/exact-mip-{before,after}-{1,2,3}.*`,
`exact-mip-summary.json`, `exact-mip-comparison.jpg`, and
`exact-camera-mip-tests.log`. Next architectural work must address intermediate
refinement traversal and achieved-error reporting, not merely reduce reservations.

## Selected-cut error audit (5 November 2026, integrated diagnostics)

An explicit diagnostic GPU dispatch evaluates the retained cut against the same
consumer projections and spatial/optical thresholds used by selection. It runs
only when `Runtime::diagnostics` is requested, not during normal frame scheduling.
Reports separate maxima for all consumers and camera consumers, plus counts
over target. The normalized target ratio is max(spatial error / requested pixel
error, optical support / one pixel). Emergency proxies, unmatched direct
representations and forced-refinement nodes are counted separately; their missing
error guarantees must not be interpreted as zero-error acceptance.

These are selected-cut estimates from authored bounds, not measured image error
or proof that downstream residency/materialization rendered every record. Thus
G2.2 remains open until output publication and error evidence are reconciled.

Normal lowland+grass route (native 1280x720, one view, 240 frames) reported
40,530 camera-over-target and 80,061 all-consumer-over-target representations,
plus 15,063 forced nodes. Emergency proxies and unmatched direct representations
were both zero. Selected spatial bound maximum was 95,418.82 pixels; target
ratio maximum 132,132.28. These are conservative projection bounds (including
near-plane amplification), not measured image displacement. They establish that
zero emergency fallbacks alone is not evidence of adequate detail.
Artifact: `build/playtest-unified/cut-error-route.log`; normal selected counts
remain 758 voxel slots / 94,366 direct roots / zero source slots.

Validation: all 139 foliage tests pass (`cut-error-tests.log`). The strengthened
GPU mixed-cut oracle additionally checks zero error for fully refined leaves,
exact over-target parent counts under slot/raster limits and missing pages, and
direct-root error lookup (`cut-error-oracle.log`, PASS). Normal frame dispatches
are unchanged: the new audit runs only on explicit diagnostic readback. No
performance improvement is claimed.
