# Warp vegetation: implementation checklist

Updated 6 November 2026. **This is the single authoritative checklist.**
Older reports and `engine/docs/log/handoff.md` contain supporting history, not separate
plans. Architecture first; no interactive playtest until requested.

## Where we are

**Overall status: incomplete.** GPU incremental complete-cut selection, bounded
paging, shared poses and shadow caching are integrated. Requested detail,
upstream two-phase visibility, independent consumer detail and target performance
are still unaccepted. There is no CPU near-tree pool.

The camera splat pass holds one compact list of visible tree indices per view.
Its count bounds all reads each frame. Only the ray marcher reserves tile
candidate and interval storage. Brick occlusion tests the eight corners of the padded bound
against the depth pyramid and keeps the brick as soon as a depth sample leaves
it visible. Camera and sun splats use four-vertex strips.
[Raster and culling measurements](log/2026-11-21-voxel-render-cull.md)

The default forest selector preserves the order of the initial root queue.
Later rounds store camera and retained-shadow tasks at one end of the queue
and remaining shadow tasks at the other. Each phase dispatches over its
own count. The separate budgeted selector allocates its proposal storage
when first used. [Terrain Fly measurements](log/2026-11-21-terrain-fly-performance.md)

## 6 November (evening): near-pixel splats, urgent streaming

Measured on the RTX 5070 Ti (Vulkan) after each round; numbers are GPU ms.

- **Area thinning** (`volume_splat_append`): a camera cell whose disc covers
  under 1 px is kept with probability = area and drawn that much larger
  (records carry alpha/64). Splat raster halved (lowland 2.35 -> 1.10 ms).
  The test runs before the cell fetch. Sun maps are not thinned: filtered
  depth turned it into grainy, lighter tree shadows.
- **Calibrated booking:** bookings bound every view direction, alpha and
  pixel size and ran ~3.3x the stored splats, so the camera error scale
  never moved. The first view's stored count is copied to refiner state[82];
  `fast_adapt` keeps a smoothed booked/stored ratio (state[83], 1..8, 15%
  margin) that scales the camera splat limit. Branch slots now bind: 65536
  gave scale 15 -> 11 for +4..6 ms, the default was 49152; with pages settled 32768 saves ~3 ms in forest and near for 541 of 2.2M splats over 2 px, so it is now 32768.
- **Urgent streaming:** pages were uploaded in id order, so recipe 2360's
  finer pages came last and near bushes stayed 31 px balls for ~290 frames
  (every earlier "largest cell" in near/forest was measured mid-streaming).
  `ready()` now requests with an urgency (waiting cut's px error x16, camera
  x4) and the page cache admits and uploads highest first; 32 uploads per
  frame (was 8). At frame 240 near/forest largest cells 31/19 -> 3.3/2.7 px.
- **Render suite** now runs 600 frames so streaming settles before measuring
  (`tools/render-suite.json`). Settled: every splat 1-2 px in all four views.
- **From high up** (Joep's flight at 626 m): the ~1 km ring was the 180 m sun
  map's edge (reach now grows with altitude, at most 1500 m); the ~2.1 km
  change is the handover to the painted `far_forest` (terrain_surface.wgsl);
  its stair-stepped stands come from the stand edge itself (tap filtering
  only softened them). The default style overflows the shadow splat budget
  at 1500 m reach.
- **Next:** cost per view (selection, voxel splat walking, occluder cull),
  far shadows at height, the far forest stand edge, the grass-on-grass wedge.

## 6 November (afternoon): booking, shadows on voxels, exact camera mips

Mancave RTX 5070 Ti; lowland 720p --grass and render suite 1440p (no grass), real
defaults (65536 slots, 8M splats per class), single runs under CI load.

- [x] Splat bookings scale by the share of the bound inside each class's
  frustums (state[80] marks the error cut). Safe: 0 missing records in every view.
- [x] terrain_fly --branch-slots really defaults to 65536 (a hardcoded 4096 overrode
  World::default()). Default splat cap per class 8M (was 4M), clamped to the device.
- [x] Shadow consumers hold at most a quarter of the slots (state[81]); binds in
  grass-only at 16M (shadow 45,485 -> 13,998 voxel slots, camera 1.89 px).
- [x] Shadows never refine into source leaves: hill 45.9 -> 16.2 ms (near shadow
  triangles 297M -> 4.6K), near 22.2 -> 15.6, forest 31.0 -> 20.5. Thin shadow
  lines are softer.
- [x] Voxel spatial error covers its cell; the camera raster draws exactly the
  selected mip (selection bit 12) and books only it. Over 1 px: lowland 20.7 ->
  18.7%, near 17.3 -> 14.2%, forest 23.0 -> 17.6%, hill 5.8 -> 2.1%.
  Selection +0.7 ms (25-40% more records).
- [ ] Camera scale still x7-15 at 8M: 17-43% of splats are under 0.5 px, mostly
  far roots whose coarsest mip is already sub-pixel. Needs coarser far
  aggregates or coverage-preserving stochastic thinning of sub-pixel cells.
- [ ] Recipe 2360 bushes: 16-31 px cells at 12 m (both whole-root and
  articulated) follow from that loose scale.
- [ ] Ground-cover "wedge" is lighting, not LOD: the camera cut is identical at
  three sun hours while the wedge changes; bare ground under it is not darker.
  Likely grass shadowing grass at the x69 shadow scale. Not
  KERN_GRASS_SUN_OCCLUSION.

## 6 November: error-driven cut (Nanite-style), replaces budgeted admission

Handoff: [log/2026-11-06-vegetation-handoff.md](log/2026-11-06-vegetation-handoff.md).
Measured on Mancave (RTX 5070 Ti, Vulkan), lowland diagnostic, native 720p,
ONE view, 1200 stationary frames, last-120 medians, single runs.

- [x] Error-driven cut (`fast_*` in foliage_refine.wgsl, default): each task
  refines while its projected error exceeds the target and every replacement
  page is resident. Caps are atomic reservations; no priority scans, prefix
  loops or dormant queue. When a cap binds, `fast_adapt` coarsens that
  class's target (state[79] camera, [78] shadow; kept across encodes) and
  relaxes it when the caps are slack. `KERN_FOLIAGE_CUT=budget` for A/B.
  GPU test: gpu_error_cut_refines_by_error_and_coarsens_when_a_cap_binds.
- [x] Splat booking counts the expected kept splats (alpha thinning at 0.5,
  drop under 1/255), not every occupied cell: about 3x lower.
- [x] Voxel page cache 256 MiB by default (64 MiB missed ~600 pages a frame
  forever and kept 9.8 px cells at 14 m). Arenas shrink to the device's
  storage binding limit. terrain_fly default branch slots 65536.
- [x] Results: main 27.05 ms / selection 11.32 ms / largest 5.48 px / 7.4% of
  splats over 1 px. Error cut, 256 MiB, 32k slots: 11.94 / 1.21 / 3.24 px /
  15.3%. 64k slots: 18.87 / 1.35 / 2.68 px / 5.1%. 128k: 19.66 ms, same picture.
  Near grass reads as blades, tree self-shadow clean, no holes.
- [ ] 1 px everywhere: camera target settles ~12x loose at 64k slots, held by
  the camera splat reservation (14.4M booked against 8.1M stored; the gap is
  per-cell frustum rejection). Book visible fraction, or raise the budget.
- [ ] Near ground cover shows a patch boundary (speckled wedge against smooth
  grass) that more slots do not move. Not traced.
- [ ] Shadow detail costs ~2 ms of near shadow raster at 64k; consider a
  coarser shadow target or caching.
- [ ] Raster still may draw finer mips than selected for articulated groups
  (footprint override), so their pages stay whole mip bundles.
- [ ] Order-dependent admission when a cap binds within a frame (atomics):
  watch for flicker in motion; the adaptive target keeps caps slack.

## 5 November: static split retains GPU forest casters

- [x] Repair static/dynamic sun split: reserved GPU branch owners remain in
  the dynamic candidate list even when their CPU placeholders have no geometry.
  Voxel sun rendering now runs after either the shared or split source path,
  with the same source-replacement selection contract.
- [x] Raw full-map depth oracle: split versus normal matches bit-for-bit for
  still, wind, root removal and restoration. Existing cached/full oracle PASS;
  all9 static-sun tests PASS. No performance/default-cache change claimed.
- [ ] World-anchored static pages for terrain/rocks/buildings/voxel foliage,
  with dynamic casters composited without losing static depth.
- [ ] Page-specific shadow cuts independent of camera refinement; unchanged
  pages must skip selection/cull/splat generation, not only rasterization.
- [ ] Near-map page reuse, bounded invalidation from changed caster bounds,
  sun/terrain revisions, and an explicit animated near-band policy.
- [ ] Handoff acceptance: raw cached/full depth through camera movement,
  zero static-page cull/splat counters when still, local damage for one moving
  caster; wall/town/skyline/hill images and per-pass measurements.

## 5 November: compact voxel storage

- [x] Pack six normal-moment/weight records as signed normalized16; retain
  full f32 colour and extinction. GPU cells128 bytes versus176 (27.27% smaller).
  No voxel coarsening or occupancy changes. CPU cooked assets stay unquantized.
- [x] Bounded cache offsets, partial uploads, eviction and uncached/world uploads
  use the packed format. Same64MiB cell pool holds524288 rather than381300 cells.
- [x]166 foliage tests and29 voxel renderer tests PASS, including direct GPU
  decode versus CPU and signed-normal/weight error<=1/65534+float rounding.
- [x] Normal1200-frame native720p ONE-view capture inspected: largest5.482px,
  no missing raster records/refinement faults; one-pixel requirement still FAILS.
 935 resident pages,88.19MB cache; previous capture712 pages. Covered GPU
  median33.287ms (last120 frames). No isolated speedup claim: upstream bbc3095
  also changes opaque raster/fog. Compact storage does not solve splat demand.

## 5 November: PR8 integration and rejected selected-mip experiment

- [x] Integrate PR8 through6a4002f, preserving the other agent's occlusion,
  terrain, far sun map and per-depth-target voxel shadow headers. Both failed
  source bypass gates were withdrawn upstream; no10%-height gate remains.
- [x] Test publishing only the selected mip and preventing whole-source bypass
  when a fine voxel endpoint is already pixel-acceptable. Combined foliage
  tests161/161 pass; release builds. This does NOT establish visual acceptance.
- [x] Reject that experiment after the normal1200-frame stationary capture:
  source slots32766 ->335, voxel slots2 ->32433, maximum submitted cell
  5.482 ->31.660px. Page misses602862, page-blocked refinements7001, splat-blocked
  40357; no missing raster records or refinement faults. Image visibly regresses.
  GPU covered median22.629ms is not a speedup claim: detail is worse and the
  newly integrated far sun consumer changes the workload. Revert90d51dd's
  selected-mip and endpoint-source restriction; retain PR8 and prior fine cook.
- [x] Source work accounting/admission implemented (details below). A tight
  default is NOT accepted: the normal-scene image regresses to larger spheres.
- [x] Retained integration after reverting the experiment:161/161 foliage
  tests PASS (pr8-retained-tests.log).
- [x] Retained release build and normal1200-frame capture inspected. Largest
  cell5.482px, zero missing raster records/refinement faults, no page misses;
  strict quality still FAILS. Final120 covered GPU median36.504ms, native720p
  ONE view/TAA. This includes PR8 far-shadow demand; not an isolated comparison.
- [x] Source work metadata: count repeated templates separately from unique
  cached meshes; validate catalog ranges and arithmetic overflow. Nodes carry
  conservative source cluster work in flags bits8..31. SourceWork also reports
  triangle upper bounds across the full CLOD catalog. These are conservative
  potential costs, not actual drawn triangles. New regression: three repeated
  templates share one resident mesh but cost three instances of work.
  All162 foliage tests PASS (source-work-foliage-tests.log).
- [x] Source work admission now participates in reservation, priority-prefix
  admission, refunds, retries and diagnostics, counting each visible consumer.
  Proposal publication mode is separate from its fourth (source) cost dimension.
  164 foliage tests PASS, including collective budgets, view multiplicity and
  refund-driven complete-endpoint retry. No new CPU selection or storage binding.
- [x] Test a262144 source-cluster-visit cap in the normal1200-frame capture.
  Reserved262119,2010 source slots,317 voxel slots; no refinement faults.
  Largest cell18.993px versus retained5.482px: visual regression, REJECTED.
  Final120 covered GPU median20.636ms versus36.504ms; NOT an accepted speedup.
  Selection alone10.735ms; camera voxel raster2.909ms, splat setup2.470ms.
  Both16M splat budgets saturate;635670 page misses and25832 page-blocked
  camera refinements. A source cap exposes unresolved fine voxel capacity; it
  does not solve it. Screenshot source-budget-1200.png confirms large spheres.
- [x] Also reject16777216 cap with source-cost priority: saturated16777159
  visits and largest cell14.448px. Preserve previous refinement priority; source
  accounting is an independent admission constraint, not a new quality heuristic.
- [ ] Calibrate a lower source work default only after compact fine voxel
  publication/paging passes the image requirement. Retain accounting with a
  conservative1073741824 safety ceiling, not the rejected262144 default.
  Cluster visits are a work upper bound, not measured triangles or milliseconds.
- [x] Final accounting build:164 tests PASS and normal1200-frame capture
  inspected. Original priority restored,1073741824 safety cap nonbinding;
  reserved25699650 conservative source visits,32736 source/32 voxel slots,
  zero refinement faults. Largest cell5.482px restores prior maximum; still
  FAILS one-pixel acceptance. Covered GPU final120 median33.287ms; single run,
  not a repeated speedup claim. Source-budget-final-1200 artifacts.
- [ ] One-pixel camera/shadow, bounded work, repeated timing and4K/four-view
  acceptance remain incomplete. Do not launch this rejected capture as playtest.

Evidence: vegetation-fine-endpoint-measurements.json, pr8-integrated-* artifacts.
Diagnostic: native1280x720 ONE view/TAA,151058 active roots,32768 slots,16M splats.
The source-shadow baseline lacks PR8's far sun consumer; not an isolated A/B.

## 5 November: retain fine voxel candidates and preserve shadow selections

- [x] Preserve the reference voxel grid as candidate zero when it fits the
  existing brick cap. Previously every asset discarded that grid immediately.
  Spatial groups now cook at their authored resolution; coarse whole-root
  fallback volumes remain separate. Cache keys include the changed cook source.
- [x] Complete endpoint admission first tries a pixel-acceptable mixed
  source/voxel frontier, then the complete source alternative. A GPU regression
  proves two fine voxel groups can fit where their four source leaves cannot;
  missing required pages retain the complete ancestor.
- [x] Preserve source geometry when the shadow consumer selects source. The
  materializer previously substituted adaptive voxels even for source choices.
  Source representations no longer require unrelated voxel mip pages or reserve
  voxel splats. Source triangle work is still real work, not free rendering.
- [x] All158 foliage tests pass, including actual GPU publication across camera,
  shadow and combined roles, wind times, night/day, teleport and four views.
  Aggregate tests10/10 and asset tests24/24 passed for the fine cook. Release
  terrain_fly builds; normal fixed-camera1200-frame capture inspected.
- [ ] ONE-PIXEL acceptance: still FAIL. Fine cook alone regressed to18.993px
  and535911 page misses. Correct source-shadow publication removes page misses
  in this capture and reduces the largest submitted cell to5.482px. Nearby
  source detail improves visibly; coarse grass/tree regions and shadows remain.
- [ ] Complete fine-voxel admission and consumer fairness. All32768 slots fill:
  32766 camera source slots,2 camera voxel slots, no detailed shadow slots.
  Camera refinements:30428 slot-blocked,0 page-blocked,9972 individually
  affordable but not collectively published. Increasing source capacity is not
  the desired solution. Shadow raster reservation also nearly fills16M.
- [ ] Performance acceptance. Single-run final120-frame covered GPU medians:
  mixed endpoint29.114ms; fine cook41.269ms; source-shadow correction30.534ms.
  All native1280x720 ONE view/TAA,151058 active roots,32768 slots,16M splats.
  These are diagnostics, not alternating repeat acceptance or4K/four-view results.

Evidence: vegetation-fine-endpoint-measurements.json; build/playtest-unified/
{mixed-endpoint,fine-cook,source-shadow}* logs and captures. No interactive
playtest started. The capture HUD's fixed simulation fps is not GPU performance.

## 5 November: deferred proposals reconsider complete endpoints

- [x] Reject increasing refinement waves fourfold. Same fixed-camera1200-frame
  diagnostic: largest submitted cell stays8.564px; detailed shadow branches
  stay zero; final120-frame GPU covered median25.460 ->54.293ms. Restore the
  original2*(hierarchy_depth+1) wave bound. This is not repeated perf acceptance.
- [x] Reproduce a collective-admission failure with128 GPU roots: each
  intermediate cut fits alone, only one fits collectively, all complete source
  endpoints fit together. Before the fix, some roots retain intermediate[1,4]
  cuts because parked proposals cache the expensive route and serialize releases.
- [x] Deferred camera proposals now reconsider the complete resident source
  endpoint and its priority. Wake eligibility also considers that endpoint if
  the cached intermediate splat cost cannot fit. Same atomic complete-cut
  reservation/publication, no CPU root selection. All157 foliage tests pass.
- [x] Release build and normal stationary captures inspected, original wave bound.
  32768 slots: largest cell7.009px (previous8.564),32768 source slots, no detailed
  shadow slots. GPU covered median22.761ms in this single diagnostic run.
  131072 slots: largest cell4.440px,131071 source slots, one shadow voxel slot;
  GPU covered median35.025ms. Both strict pixel checks FAIL; no missing voxel records
  or refinement faults reported. Nearby grass improves, spheres remain farther
  out and grass shadows remain coarse. Defaults were NOT increased.
- [ ] Efficient complete ONE-PIXEL voxel cuts: larger slot capacity largely
  buys whole source endpoints. It does not solve fine-voxel admission or
  consumer budget fairness. At131072 slots,31816 camera parents are slot-blocked,
  no camera parents are page-blocked, and6017 are individually affordable but
  not published finer. Investigate finer complete cuts and their conservative
  work accounting; do not call repeated source-pool expansion the final design.
- [ ] Shadow refinement and quality: camera-first admission still leaves almost
  every shadow on whole-root representations. Visible grass-shadow spheres
  remain a failed requirement, independent of the camera-cell measurement.

Reports: vegetation-refinement-wave-experiment.json and
vegetation-parked-source-measurements.json. Build/test/capture artifacts in
build/playtest-unified/{refinement-depth,parked-source}*. No interactive game
was opened during these experiments. F1-F8 delivery still needs playtest evidence.

## 5 November: branching splat barrier and playtest feedback

- [x] GPU regression proves a temporary-cost barrier: four source leaves cost
  four slots/zero camera splats, but the immediate branching voxel cut needs
  200 splats against a five-splat budget. Before the fix it retains the root.
- [x] Camera admission can bypass that barrier with the complete resident
  source frontier. Reservation remains collective; missing source pages or
  insufficient slots retain the parent. Intermediate voxel pages are not
  required for a source-only publication. Propose/apply and diagnostic audit
  agree; cached proposals preserve their publication mode. No CPU selection.
- [x] All156 foliage tests pass including the new regression; release renderer
  builds. Normal fixed-camera1200-frame strict check still FAILS at8.564px.
- [ ] One-pixel camera and shadow acceptance: UNRESOLVED. User playtest still
  shows many vegetation spheres and sphere-shaped grass shadows. Source-only
  budget relief is not the requested pixel-voxel architecture's completion.
- [ ] Full-scene scheduling convergence:42515 camera parents remain over target,
  now individually affordable/resident according to the final-budget audit.
  Individual affordability does not prove collective affordability or admission
  within the finite wave schedule. Camera source slots20285 ->20678; camera
  voxel slots1852 ->1594. No visible sphere fix or speedup is claimed.
- [ ] Investigate biome/jump keys with playtester. Existing log shows scene
  rebuilds but did not identify received keys. Add immediate action/building
  status and input logging; do not label that as a proven input-route repair.

Playtest was accidentally launched with the benchmark camera's1m/s speed;
normal flight is40m/s. Explicit --speed now also overrides captured interactive
speed. The inspection demo disables adaptive quality by default: the playtest
had silently fallen to856x482 internal pixels, undermining visual assessment.
Foliage defaults and physics are unchanged. These app adjustments require the
next playtest to verify keyboard delivery; no interactive restart is automatic.

Evidence: vegetation-source-frontier-measurements.json and
build/playtest-unified/source-frontier-{tests,build,1200,playtest} logs/captures.

## 5 November: fixed-camera convergence separates loading from selection

Added terrain_fly --fly N --stationary: zero translation input while ticks,
wind, rendering and asynchronous page service continue. This makes streaming
convergence measurable without moving the camera into a different workload.
The HUD speed is the configured movement speed; stationary input does not move.

Same native1280x720 ONE-view camera at(0,8,0),151058 roots,32768 slots,16M splats:
- Frame240:1893 pages resident, largest cell18.797px,12713 camera proposals
 blocked by pages. Deferred queue overflows391390 requests vs262144 capacity.
- Frame1200:2250 pages resident, last upload at frame288. No page misses,
 evictions, pending uploads, or camera proposals blocked by pages. Largest cell
 still8.564px;42703 camera cuts remain over target. Deferred queue no longer
 overflows (167113 requests). The pixel-quality requirement STILL FAILS.
- Worst settled cell is grass root107384, recipe2440, node2441. Both immediate
 child pages are resident. GPU replay reports an eligible2-slot/85-splat
 replacement for its0-slot/69-splat parent. Published camera budget is16776996
 of16777216 splats;36758 resident proposals are individually affordable.
 This does not mean all proposals fit collectively. The scene still saturates
 its splat reservation even after asset loading is complete.

Images inspected: nearby blades replace some coarse grass, but a prominent
coarse-cell band remains. This disproves incomplete page loading as the sole
cause of the persistent sphere problem. Do not change eviction logic on the
basis of the earlier240-frame nonresident snapshot alone.

Next correctness work: refinement scheduling and complete target-quality cuts
under budget contention. The encoder currently reseeds roots each frame; a
static camera does not accumulate refinement across frames. Distinguish final
individually-affordable proposals from a collectively feasible complete cut.
Investigate temporary-cost barriers before repeating rejected lookahead changes.

Release build and both headless runs complete; strict quality exits nonzero.
Single-run final120-frame GPU covered medians25.669/24.986ms are diagnostics,
not repeated performance acceptance. Evidence: vegetation-stationary-convergence.json
and build/playtest-unified/stationary-{240,1200}.*. No gate closed.

## 5 November: whole-root grass ownership is now observable

Offline raster profiling now decodes whole-root volumes, including separate
camera and shadow publication ranges after an allocation-sized branch prefix.
It reads the actual published representation, maps it back into that root's
recipe subtree, reports child-page residency, and replays the GPU proposal.
No CPU root/branch selection or per-frame readback was introduced.

Normal16M/32768-slot capture identifies the worst15.300px cell as camera root
109617, recipe2440, selected node2441: a0.32m grass voxel. No fallback fault.
Its two immediate children are2442 (resident) and2564 (nonresident). Replay
would replace69 splats/zero slots with85 splats/two slots, but returns inactive
control because a required child is missing. This proves a page-readiness stop
at the captured instant, NOT why the page was absent or whether the proposal
could fit once resident. Slot/splat constraints remain independently saturated.

The cache reports1893 resident pages, zero evictions/misses, no partial upload,
and ongoing frame uploads. Its per-frame upload-count budget is8. A240-frame
capture is therefore insufficient evidence of a permanently stuck page: test
convergence with a fixed camera and inspect generation-qualified demand before
changing the eviction/request algorithm. Do not label this a confirmed paging bug.

Release build and155 foliage tests PASS, including GPU lifecycle/refinement
fixtures and root-consumer range boundary test at65537 branch slots. The normal
capture finishes and intentionally fails strict pixel quality. RGB MAE below
HUD versus prior16M run is0.01114/0.01083/0.00792; not raw-buffer equivalence.
Evidence: vegetation-root-attribution.json, root-attribution-16m.* and
root-attribution-tests.log in build/playtest-unified. No quality gate closed.

## 5 November: normal-renderer capacity evidence and shared splat budget

Runtime-sized consumers are now verified in normal headless renders, including
32,768 branch slots above the old uniform limit. Default TAA/native1280x720,
ONE view, 151,058 active roots including grass; no 4K/four-view acceptance.
The ordinary4096-slot image is effectively unchanged below the HUD (RGB MAE
0.000261/0.000239/0.000199); this is PNG comparison, not raw visibility proof.

The new diagnostic --splat-budget N controls the SAME renderer and selector
capacity. Both subtract per-tree record headers from the actual device buffer
limit. Previously independent4M clamps prevented an honest larger-capacity test;
selector reservations could also exceed a hardware-clamped target. Added unit
checks for header exhaustion, device truncation and overflow-safe arithmetic.
Default splat/branch budgets and one-pixel policy remain unchanged.

Single diagnostic runs (NOT repeated performance acceptance):
-4096 slots /4M splats: largest cell21.308px;41,944 over-target camera cuts.
 GPU covered median15.468ms. No missing records or refinement faults.
-32768 slots /4M: same21.308px maximum;41,925 over-target cuts. Camera splat
 capacity exhausted, no branch-slot stops. GPU covered median16.281ms.
-32768 slots /16M: near trees gain source detail, but largest grass cell15.300px
 at about(-17.70,0.14,-6.68), world cell width0.32m.42,878 cuts over target.
 All32768 slots occupied; camera splats saturated;12,048 camera proposals
 await pages. Deferred-cluster queue OVERFLOWS (431256 requested vs262144).
 GPU covered median92.252ms. Selection19.325ms, recovery cull14.721ms,
 voxel listing8.100ms and voxel raster8.099ms. The larger budget is NOT a fix.

All strict pixel checks FAIL, as they should. Images inspected; detailed nearby
tree shapes return with more capacity, but grass remains oversized and spherical.
No FPS gain or full-forest correctness claim. Median timestamps use frames>=120,
not wall time. Evidence: vegetation-dynamic-capacity-measurements.json and
build/playtest-unified/{dynamic-choice-4096,dynamic-choice-32768,shared-budget-16m}.*.
Release build,29 voxel tests and154 foliage tests PASS, including exclusive GPU
choice-boundary, lifecycle, paging, refinement and shared-pose fixtures.

Next: decode whole-root grass attribution and its pending refinement, preserve
camera and shadow correctness under shared-slot contention, and resolve deferred
queue capacity/recovery. Raising budgets alone does not satisfy the requirement.

## 5 November: allocation-sized producer choice tables

GPU materialization, camera/shadow transfers, offline diagnostics and animated
shadow damage now derive their consumer-class stride from the actual slot count.
Removed fixed16384/32768/131072 producer addressing; choices allocate8*slots bytes.
The shadow damage bounds check likewise validates the actual two-table size.
A one-slot GPU oracle verifies camera changes leave shadow tiles alone and shadow
changes dirty the right tiles; runtime fixtures check exact two-table extent.
154 foliage and6 shadow_damage tests pass; release build and normal captures pass.
Trees-only normal image is pixel-identical outside HUD to the previous baseline.
Its pixel-quality check still FAILS at153.62px. Trees+grass separately FAIL at21.31px.
A two-root close fixture with128slots passes camera pixel quality; it uses a different
budget than the earlier16384-slot fixture and is not an image equivalence claim.

Correction: the earlier pixel-policy-full capture omitted grass; it has71112roots.
The new trees+grass capture has151058roots. Keep these populations distinct.
Evidence: vegetation-choice-stride-measurements.json; original report annotated.

Historical producer-only checkpoint (superseded by runtime-sized consumers above).
At this point cull/raster/voxel used64KiB uniform choice tables and retained the
16384 checked limit. Next: migrate those consumers to dynamically sized storage.
Cull already uses16 storage bindings: consolidate traversal counters with its
existing counter buffer before adding the choice storage binding. Preserve current
indirect arguments, raw diagnostics and shadow-demand ownership. Then test tables
beyond16384 and revisit full-scene correctness, without relaxing1px policy.
No architecture gate closed, no frame-time gain claimed, no playtest opened.

## 5 November: correctness-first pixel policy and explicit failure

User priority: establish pixel-scale voxel correctness before optimizing speed.
Removed terrain demo's32px geometric-error override and6px/distance-growing
proxy policy. Normal demo now requests1px with no distance relaxation. Resource
limits are NOT enlarged or silently reinterpreted as visual acceptance.
`--require-pixel-vegetation` on --fly captures checks final submitted cell diameter
<=1px, missing splats, near-plane crossings, camera over-target nodes and physical
fault/fallback counts. It saves captures/timings before returning an error. This
is a final-frame conservative submitted-work check, not full visibility/coverage,
motion, all-weather or four-view acceptance. Sampled optical policy is unchanged.

The normal full scene FAILS (153.62px maximum,19782 over-target camera nodes).
The100m tree test also FAILS (15.06px;16384 slots exhausted). Do not present either
as corrected rendering or use their faster frames as a performance gain.
A fixture bug ignored tree radius for shrubs: shrubs now respect min(radius,400m).
Default2500m scene population is unaffected; tiny fixtures now have two roots.
Two-root close view renders58 source branches, no voxel slots,0 over-target nodes;
image inspected. At1000m elevation the same tiny domain renders1885 voxel splats,
max0.1965px,100 voxel slots,0 over-target nodes. Both final-frame checks pass.
These are component proofs ONLY; they do not satisfy the forest-scale goal.

Reproduce isolated checks with terrain_fly --style lowland --fly 240 --speed 1
--size 1280x720 --tree-radius 10 --branch-slots 16384 --voxel-raster-profile
--require-pixel-vegetation --from engine/docs/fixtures/vegetation-pixel-near.json
(or vegetation-pixel-far.json); CLI options and values require spaces.
Evidence: vegetation-pixel-correctness.json. Release build, pixel-policy unit test
and headless pass/fail routes completed. No interactive playtest started.
Next: remove publication's small fixed branch-slot/owner constraints while keeping
bounded residency and complete source/voxel coverage. Performance comes AFTER
normal full-scene pixel correctness; G1/G2/G3/G6/G8 remain incomplete.

## 5 November: GPU proposal replay; reject complete-cut lookahead candidate

Offline cell attribution now replays the actual GPU proposal kernel against final
budget/residency, in scratch state/pages/queues/counters. A GPU fixture checks
returned costs and byte-identical live counters, cut, pages, demand and state.
Root33573/node1116 still proposes an affordable resident replacement at frame end:
old [1slot,171splats], new [2slots,199splats], status1 (eligible), bucket23.
This proves final eligibility, NOT historical rejection reason or global fit of
all eligible replacements. The observed immediate quality regression remains real.

Candidate complete-cut lookahead skips coarser spatial children, admits their
complete finer frontier atomically, and retains parents on missing pages/budget.
155 foliage tests pass, but normal rendering REJECTS it: branch slots fill4096,
source slots261->4074, lowland largest submitted cell21.31->72.08px. Three
alternating native720p ONE-view covered GPU medians are15.99/16.01/16.08 before,
18.63/18.72/18.71 after. Alpine14.98/15.10/15.13 before,17.18/17.14/17.26 after.
Production traversal restored; candidate/test/diagnostic diff preserved against
7974a88 in vegetation-complete-lookahead-experiment.patch. Per-pass and proposal
evidence: vegetation-complete-lookahead-measurements.json. No visual fix claimed.

Next architecture gap: quantify which assemblies consume transient source slots
and separate reusable/direct source instances from temporary refinement slots.
Quality-improving traversal alone starves foreground detail under the current
shared slot cap. Do not simply raise the cap or accept larger rendered cells.
Upstream tree recovery, per-view cuts and far-cell aggregates remain incomplete.

## 5 November: attribute oversized cells to the published GPU cut

The explicit --voxel-raster-profile report now attaches articulated-slot owner,
root/node/generation, selected error/cost, immediate children and GPU page state.
Only explicit offline capture reads back; no CPU selection or normal-frame waits.
Whole-root volume attribution is explicitly marked not yet decoded.
154 foliage tests (including GPU fixtures), release build and normal native720p
ONE-view route succeeded. Image RGB MAE
excluding HUD <0.000074 against retained baseline (diagnostic-only change).
Largest submitted cell was21.308px, root33573/node1116. Selected authored error
1.7875/optical footprint0.9492; immediate child1117 is worse at3.3464/3.4293.
Both children are resident. This exposes nonmonotonic intermediate quality, but
unary candidate skipping must be traced before claiming the exact admission cause.
Next: capture the actual replacement proposal for this owner and fix complete-cut
lookahead/cooking as justified; do not merely raise rounds or shrink splats.
Evidence: vegetation-cut-ownership.json. Spheres remain unresolved; no gate closed.

## 5 November: reject node-depth candidate; oversized voxel cells remain

Current-terrain visibility was extended from roots to hierarchy nodes before
reservation, with an array of current per-camera depth pyramids and independent
shadow demand. All 155 foliage tests passed (including GPU tests), and the release
example built. Three alternating baseline/candidate runs per route, native720p
ONE VIEW, showed no selection improvement and nearly identical final images:
lowland median covered GPU 15.092 -> 15.683 ms; alpine 14.158 -> 14.583 ms.
The candidate is NOT integrated. Its source/tests are retained in
vegetation-node-depth-experiment.patch and per-pass results in
vegetation-node-depth-measurements.json. Normal renderer source is restored to
2f38bf1. This does not fix the user's visible spheres: measured cells up to24px
remain, whereas pixel-scale distant representation is the intended goal.
Next work must demonstrate actual final cell footprint reduction and near-source
coverage, not count visibility infrastructure as a visual fix. G1.4/G2/G3/G8
remain incomplete. No interactive playtest launched.

Latest integrated change (5 November): current-frame TERRAIN seed depth now feeds
GPU root visibility BEFORE normal shared refinement. Camera rejection preserves
shadow demand and unions every camera; no historical-depth rejection. 154 foliage
tests pass. Three alternating runs per route show ADDED cost, not a speedup:
lowland about +0.23 ms, alpine +0.47 ms (native720p ONE view). Tree/cluster seed
recovery is still missing; oversized cells remain. G3.2 is NOT complete.

Earlier integrated change (5 November): camera and shadow classes now choose
independent GPU cuts with shared residency/poses, compact live root tasks,
consumer-local fallback and consumer-specific raster dispatch. 152 foliage tests
pass. Final routes have zero emergency fallbacks but are NOT faster; pixel-scale
voxel detail remains unaccepted (oversized rounded cells persist). See evidence
below. Per-view cuts within each class and upstream recovery remain open.

Earlier integrated change (5 November): whole-root resident ancestors and emergency
fallbacks now preserve camera/shadow roles, root motion history and cached-shadow
damage. 147 foliage tests plus six shadow-cache tests pass. Normal route images
match; variable timing results are recorded below without a speedup claim.
Independent camera/shadow cuts and upstream recovery remain open.

Earlier integrated change (5 November): bounded voxel pages now publish shared
4x4x4 occupied-cell group ranges with their resident payloads. Metadata shares
page allocation, upload readiness and eviction; no per-instance cell copies.
146 foliage tests pass, including raw GPU descriptor checks after reload.
Normal rendering remains effectively unchanged in matched high/low camera routes.
The attempted downstream group culling was slower and is NOT integrated; see the
latest evidence below. G3 upstream recovery and independent cuts remain open.

Earlier integrated change (5 November): a separate GPU dormant queue retains
budget-rejected refinements until active work drains and capacity/new pending
work changes. Stable parallel wake compaction gives cameras priority while
keeping eligible shadow work pending, including zero-cost camera batches.
Dormant overflow publishes a complete resident parent. No CPU scheduling/readback.

This is a correctness checkpoint with a measured regression: three alternating
native720p SINGLE-VIEW runs measure 14.927-15.047 ms covered GPU, selection
8.575-8.701 ms, against 12.038-12.091 / 5.746-5.838 ms before. Do not call this a
performance optimization or 4K/four-view acceptance. 144 foliage tests pass.

The final normal-route camera cut now has 124 source branches and 66 voxel
branches (previously zero of both). 40,424 camera nodes still exceed requested
error; 40,418 are camera-splat constrained and 21,608 also shadow-splat constrained
(reasons overlap). Seven have a missing next-detail page. No affordable/resident
next refinement remains in this audit. Nearby crowns remain visibly coarse.

Next: quantify actual oversized cell footprints and their retention reasons,
then address them with finer source/voxel coverage. G3 upstream two-phase
visibility and per-view consumer cuts remain necessary. Shared-slot reservation
is still conservative before deduplication. None of this closes G1.4, G2 or G8.

## Immediate performance decision (existing G1.4/G8 evidence, not new gates)

- [ ] Measure voxel rasterization at ~1 pixel projected support, including splat
  generation, overdraw, visibility/resolve, and shadow work. Report submitted and
  surviving counts; compare the identical view with a detailed reference.
- [ ] Same executable, alternating native/DLSS Quality/Balanced/Performance route
  runs, at least three per mode. Record actual input/output sizes, per-pass medians,
  reconstruction cost and comparable images. No silent fallback.
  Record actual projected voxel-cell footprint percentiles/maxima and counts
  above the pixel-scale target; identify why coarse representations survive.
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

Consumer-publication prerequisite (5 November): camera/shadow role bits now
survive request deduplication, residency and materialization. Both consumers
share one payload slot and pose; source/voxel draws exclude records belonging
only to the other consumer. Newly added consumers invalidate motion history.
147 foliage tests, six shadow-cache tests and composed shader validation pass.
Direct whole-root ancestors and emergency fallbacks now preserve consumer roles
as well. Shadow-only roots invalidate cached shadow tiles; newly added consumers
reset root motion history. Camera/shadow classes now choose separate cuts; G3.3 remains open for
independent per-view cuts within each class and tighter shared-slot accounting.
G3.2 upstream visibility is unchanged.

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

The plant card baker preserves four area-weighted source normals across each projected procedural needle spray. Relative depths in the source spray darken sheltered texels, while the existing coverage mask keeps gaps and exposed tips. Both the shading and the mask are baked once from the growth geometry.

## G5 — Shared wind and animated shadow caching (NOT COMPLETE)

List reusable whole-tree instance slots in `Scene::tree_wind_ranges` to apply
root-anchored sway and update their shadows as they move. For articulated
branches, create a `PartWindRig` when the plant becomes resident and call
`sample_into` each frame to reuse its validated bind poses and the output buffer.

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

## DLSS diagnostic sweep — 4 November

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

## One-pixel raster feasibility — 4 November

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
| 0.5–1 | 89,424 | 217,864 |
| 1–2 | 18,359 | 73,124 |
| 2–4 | 1,091 | 12,082 |
| 4–8 | 0 | 3,643 |
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

### Consumer isolation — shadow demand is not the primary raster cause

Same executable, three alternating normal/camera-only240-frame runs at720p,120
warm GPU samples. Camera-only sets sun_lux=0 and shadow_strength=0, confirmed
shadow_only_roots=0. It changes lighting, so total-frame time/image is NOT a
like-for-like visual comparison. Compare selection and camera raster workload only.

| Mode | Final requested splats | Selection medians ms | Raster medians ms |
| --- | ---: | --- | --- |
| Combined | 4,495,774–4,495,775 | 2.516 / 2.658 / 2.508 | 2.252 / 2.505 / 2.251 |
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
lowland+grass route, 240 frames, with frames 120–239 measured.

| Repeat | Before covered ms | After covered ms | Before selection ms | After selection ms |
| --- | ---: | ---: | ---: | ---: |
| 1 | 11.526 | 11.967 | 5.224 | 5.666 |
| 2 | 11.388 | 11.755 | 5.250 | 5.558 |
| 3 | 11.386 | 11.880 | 5.240 | 5.504 |

All 139 foliage tests passed, including exclusive GPU fixtures. Camera splats
remained 1,361,047. The matched scene image outside the HUD was virtually
unchanged (each channel MAE below 0.000005 on the 0–255 scale); this is not
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
lowland+grass route repeats (240 frames, frames 120–239 measured) gave:

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
remain coarse; RGB MAE outside HUD was 1.014/1.075/0.659 on a 0–255 scale.
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

## Non-monotonic unary refinement (5 November 2026, integrated)

GPU candidate traversal inspects through expensive or unavailable intermediate
unary LODs and selects a resident deeper endpoint within the original candidate's
slot/camera/shadow bound. It stops before a sibling split. Missing final detail
retains the complete ancestor. No raster budget or error threshold was changed.

All 140 foliage tests pass (unary-lookahead-tests.log), including a new GPU
fixture with costs 10 -> 100 -> 2 and cap 5: cheap endpoint admission, missing
intermediate page, missing endpoint fallback and endpoint recovery.

Three alternating native 1280x720 one-view lowland+grass route repeats
(240 frames; frames 120-239 measured): covered GPU before
11.544/11.503/11.568 ms, after 11.519/11.532/11.465 ms. Selection before
5.230/5.231/5.229 ms, after 5.216/5.232/5.273 ms. No performance win claimed.
Tested nodes (532,044), camera-over-target count (40,530), and selected cut
counts are unchanged. Matched images inspected; per-channel MAE outside HUD
below 0.00004/255. Not a raw-buffer identity or visual acceptance claim.
Source-page uploads increased from 3 to 6 due to deeper demand discovery; no
source slots became selected. Preserve the admission fix, but do not claim it
resolves the normal route's refinement bottleneck. Next investigate global
priority reservation and its distribution across hierarchy levels.
Artifacts: build/playtest-unified/unary-{before,after}-{1,2,3}.*,
unary-summary.json, unary-comparison.jpg, unary-lookahead-tests.log.

## Individually unaffordable proposal filtering (5 November 2026, correctness integrated; performance regressed)

Before priority-prefix reservation, reject upgrades whose positive incremental
slot/camera/shadow work cannot fit the current remaining budget even alone.
Otherwise impossible requests consume prefix space and starve cheaper proposals
that follow them. Rejected upgrades retain their resident parent. Zero-cost and
work-reducing replacements remain eligible, including an over-budget baseline.
This does not implement fully optimal packing: several individually affordable
proposals can still collectively exhaust a prefix and exclude smaller later ones.

Validation: 141 foliage tests PASS, including repeated impossible-expensive /
affordable-cheap root admission and exact reservation checks. Three alternating
native 1280x720, one-view lowland+grass route repeats (240 frames, frames
120-239 measured) gave before covered 11.524/11.571/11.614 ms and after
13.078/13.046/13.023 ms. Selection before 5.219/5.282/5.253 ms and after
6.678/6.653/6.645 ms. This regression is retained as an explicit correctness
tradeoff, NOT accepted performance or visual progress.

Final route counts: tested nodes 532,044 -> 586,510; source slots 0 -> 15;
voxel slots 758 -> 806; camera over-target 40,530 -> 40,529. Faults, fallback
and missing direct-error matches remain zero. Matched images inspected;
RGB MAE outside HUD 0.00712/0.00744/0.00399 on a 0-255 scale. No material
visual improvement. Source residency grows from 6 to 20 pages.

Evidence: build/playtest-unified/affordable-{before,after}-{1,2,3}.*,
affordable-summary.json, affordable-comparison.jpg, affordable-tests.log.
Next: global cross-depth priorities and bounding scan overhead, rather than
more isolated changes to cost estimates. G2/G3/G8 remain incomplete.

## Shared-memory priority scan (5 November 2026, integrated)

Stage each 256-proposal block once with coalesced lanes, then perform the
existing stable 32-bucket prefix scan in 4 KiB of workgroup memory. Preserve
saturating sums, stable order, priority, reservations and publication. This is
an execution optimization, not a change in detail policy or a cross-depth
scheduler. Measure before accepting; no quality/performance claim yet.

Validation: all 141 foliage tests PASS (shared-prefix-tests.log). Three
alternating native 1280x720 one-view lowland+grass repeats (240 frames, frames
120-239 measured) gave before covered 12.824/12.884/12.907 ms and after
12.117/12.114/12.090 ms. Selection before 6.693/6.687/6.697 ms, after
5.972/5.968/5.904 ms. Saving is 0.71-0.82 ms covered GPU; no 4K/four-view claim.

Selected counts, tested nodes (586,510), error counts, reservations and fallback
counts are unchanged. Matched images inspected; per-channel scene MAE outside
HUD below 0.000011/255. This is not a raw visibility/depth identity claim or
visual acceptance. Retains the prior allocator correctness fix.
Evidence: build/playtest-unified/shared-prefix-{before,after}-{1,2,3}.*,
shared-prefix-summary.json, shared-prefix-comparison.jpg.

## Priority frontier across depths (5 November 2026, rejected)

Only the highest eligible priority bucket advances per refinement wave. Lower
priority tasks remain on the frontier, retaining complete resident parents, so
high-value descendants can compete before lower-value unrelated upgrades.
The existing bounded wave count and final drain remain; therefore this is not
a proof of globally complete scheduling. Test deferred-task starvation as well
as admission, error, fallback and performance before accepting.

All 142 tests passed, including the new cross-depth budget oracle. Normal route
REJECTS this implementation: three alternating native 1280x720 one-view
lowland+grass repeats (240 frames, frames 120-239 measured) gave covered
before 12.242/12.117/12.165 ms, after 24.164/23.560/23.599 ms. Selection
before 5.896/6.003/5.977 ms, after 18.338/17.816/18.014 ms.

Repeatedly rescanning deferred tasks increases tested nodes 586,510 ->
4,089,943. Every shadow-only root (53,990) reaches final drain as emergency
fallback. Camera maximum spatial bound improves 95,418.82 -> 1,115.76 pixels,
but camera-over-target count worsens 40,529 -> 40,740. Selected source slots
fall 15 -> 2. Matched images inspected; scene RGB MAE 2.685/2.693/1.889
on the 0-255 scale. No quality/performance acceptance.

Production changes removed, preserved with test in priority-frontier-experiment.patch.
Evidence: build/playtest-unified/priority-frontier-{before,after}-{1,2,3}.*,
priority-frontier-summary.json, priority-frontier-comparison.jpg,
priority-frontier-tests.log.

Next scheduling design must cache/dequeue deferred proposals instead of
reprojecting them each wave, and guarantee camera/shadow service independently
within bounded work. Increasing the wave limit alone would worsen the problem.
Retain complete ancestors and error diagnostics; do not hide starvation.

## Deferred proposal cache (5 November 2026, integrated)

Ping-pong proposal storage alongside frontier queues preserves an eligible
proposal when its task is deferred. Reuse only within the same selection encode:
consumer transforms, recipe nodes and page residency are immutable then. Every
reuse rechecks current remaining budgets; per-wave prefix/output fields reset.
Seeded tasks and newly generated children have no cache flag, preventing stale
reuse across frames or changed consumers/pages. No CPU readback schedules work.

Additional persistent scratch is 80 bytes per frontier slot (80 MiB at the
current one-million-slot cap). Buffers stay individually within the original
proposal buffer size bound; the kernel now uses 14 storage bindings plus its
uniform. A diagnostic counter reports reused proposals; explicit error-audit
clearing preserves this counter. Require measured savings and output parity.

Validation: WGSL direct validator PASS; all 141 foliage tests PASS
(proposal-cache-tests-final.log), including strengthened consumer-order fixture
asserting actual proposal reuse. Original replacement-child count is preserved
separately from the one-entry deferred queue count.
Three alternating native 1280x720 one-view lowland+grass route repeats
(240 frames, frames 120-239 measured) gave covered before
12.235/12.278/12.328 ms, after 12.089/12.159/12.031 ms. Selection before
5.911/5.945/5.938 ms, after 5.719/5.765/5.715 ms. A modest 0.12-0.30 ms
covered saving, not a large optimization or four-view performance claim.

Fresh node evaluations 586,510 -> 261,552; reused proposals 324,958.
Published counts, reservations, error counts and fallback counts unchanged.
Matched images inspected; scene RGB MAE below 0.000022/255. No raw-buffer
identity or visual acceptance claim. This cache is infrastructure for deferred
scheduling; compact queue consumption and memory reduction remain needed.
Evidence: build/playtest-unified/proposal-cache-{before,after}-{1,2,3}.*,
proposal-cache-summary.json, proposal-cache-comparison.jpg.

## Camera/shadow root cohorts with cached proposals (5 November 2026, rejected)

Stable compact camera-owning roots, refine with the prior two-depth bound, then
compact shadow-only roots and give them a dedicated one-depth stage. Shared
baseline reservations are retained. This guarantees service to shadow roots
without repeatedly carrying them through camera waves, but is too coarse a
consumer partition for mixed-visibility branches.

All 141 foliage tests PASS; extended shadow-only optical cases PASS with an
empty camera stage. The mixed-consumer fixture confirms zero deferred cache
uses for its separated shadow root. Initial benchmarks overlapped test
compilation; discard them for performance conclusions. Clean serial repeats
after build/tests ended (native 1280x720, one view, lowland+grass, 240 frames,
frames 120-239 measured) report:

| Repeat | Before covered ms | After covered ms | Before selection ms | After selection ms |
| --- | ---: | ---: | ---: | ---: |
| 1 | 12.015 | 12.647 | 5.716 | 6.364 |
| 2 | 12.058 | 12.605 | 5.761 | 6.266 |
| 3 | 11.814 | 12.587 | 5.785 | 6.364 |

Fresh evaluations 261,552 -> 262,361, reused proposals 324,958 -> 1,018,
peak frontier 302,116 -> 53,990. Source slots 15 -> 2,948; selected records
821 -> 4,096. Camera-over-target remains 40,529; no emergency fallbacks.
Matched images inspected: per-channel scene MAE below 0.000014/255, effectively
unchanged. More allocated detail is not established camera quality improvement.

Removed production changes; preserved consumer-root-queues-experiment.patch.
Evidence: build/playtest-unified/consumer-queues-clean-{before,after}-{1,2,3}.*,
consumer-queues-clean-summary.json, consumer-queues-clean-comparison.jpg,
consumer-queues-tests.log, consumer-queues-shadow-only.log.

Next must inspect per-branch consumer masks and separate camera/source detail
from shadow representation demand (G3.3), rather than more root-cohort scheduling.
Production remains the cached-proposal allocator from 7ca3df6.

## Branch-level consumer diagnostics (5 November 2026, integrated)

Explicit selected-cut audit now classifies each node's actual conservative
frustum demand, separately for source branches, voxel branches and direct
whole-root voxels: outside, camera_only, shadow_only, both. Cooked node flag
bit 2 identifies source representations for the audit; selection still only
uses the original forced/root bits. No normal-frame dispatch was added.
The audit scratch grows 128 -> 192 bytes and preserves proposal-reuse counters.
Existing root-owned publication counters remain for compatibility and are now
documented as ownership rather than branch visibility.

Normal native 1280x720 one-view lowland+grass route, final frame of 240:

| Selected representation | Camera only | Shadow only | Both | Outside |
| --- | ---: | ---: | ---: | ---: |
| Source branches | 0 | 15 | 0 | 0 |
| Voxel branches | 0 | 806 | 0 | 0 |
| Whole-root voxels | 18,806 | 53,835 | 21,723 | 0 |

The old camera-root-owned branch count is 780, although no selected branch
intersects any camera frustum. This explains why more source slots in prior
queue experiments did not establish improved camera detail. Camera-visible
whole-root count is 40,529; camera-over-target count is also 40,529.
These counts precede downstream occlusion/materialization and do not measure
actual drawn pixels or prove page publication correctness.

Validation: all 141 foliage tests PASS (branch-consumers-tests.log), including
exact audit histograms for mixed retained cuts and direct roots. Direct WGSL
validation PASS. Normal route source/voxel/direct histogram totals match
15/806/94,364 published representation counts, with no unknown proxies or
unmatched direct nodes. Evidence: build/playtest-unified/branch-consumers-route.log
and corresponding route capture/JSON files. No performance improvement claimed.

## Capacity-refund scheduler experiment (5 November 2026)

- [x] Reproduce premature finalization: a camera refinement costs four extra
  splats, but is finalized before another complete cut refunds seven. A GPU
  fixture proves a retry admits the correct full cut without exceeding caps.
- [x] Measure bounded retries with cached proposals and unchanged complete-cut
  rules; all 142 foliage tests pass in the experimental version.
- [x] Measure stable GPU eligibility compaction; all 142 tests pass, but it
  regresses time further. Preserve both findings; neither path is integrated.
- [x] Replace full-frontier retry scanning with bounded dormant-work scheduling
  that wakes eligible work after relevant capacity changes. Keep stable priority,
  complete sibling admission, consumer roles, overflow and no-progress draining.
  Integrated below with 144 passing tests; performance acceptance remains open.
  This is part of G2.2/G3, not an additional completion gate.

| Native720p / one view | Covered GPU medians ms | Selection medians ms |
| --- | --- | --- |
| Baseline, first alternation | 12.020 / 12.029 / 12.073 | 5.753 / 5.797 / 5.755 |
| Cached full-frontier retries | 21.780 / 21.803 / 22.047 | 15.437 / 15.471 / 15.701 |
| Baseline, second alternation | 12.148 / 12.141 / 12.027 | 5.812 / 5.763 / 5.810 |
| Retry plus eligible-index compaction | 29.957 / 29.518 / 29.599 | 23.354 / 23.178 / 23.196 |

Each row uses 120 warm timestamp samples per run, same route and settings.
These are covered GPU timestamps, not 4K/four-view acceptance or gameplay FPS.
Retries change camera-over-target 40,529 -> 40,431 and affordable-resident
40,529 -> 0. Remaining nodes are blocked by camera splat capacity (all 40,431),
with 21,615 also shadow-capacity blocked. Thirteen voxel branch records become
camera-visible; source camera records remain zero. Conservative maximum camera
spatial error falls 95,418.82 -> 1,758.83 pixels, not measured image error.
Cached proposal reuses rise 324,958 -> 3,066,339. No missing pages, emergency
fallbacks, allocation faults or slot overflow. Small visual improvement only:
matched non-HUD RGB mean absolute differences ~0.86/0.86/0.62 on 0-255 channels.
The near crowns remain coarse. Compaction adds 4 MiB at current queue capacity.
An empty-block priority shortcut was also tried (single diagnostic run 22.28ms)
and did not rescue retries; eligible work is scattered across frontier blocks.

Code and refund regression fixture are preserved in
`refinement-retry-compaction-experiment.patch`, based on b4581d4; do not apply as
a default optimization. It includes the retry path and subsequent compaction.
Local evidence: `build/playtest-unified/refinement-retry-summary.json`,
`refinement-compact-summary.json`, paired comparison JPEGs and individual route
PNG/GPU/CPU/log files. Tests: `refinement-retry-tests.log` and
`refinement-compact-tests.log`. No extra scratch or retries remain in production.

Tracked per-pass medians and final counters: `refinement-retry-measurements.json`.

Restored normal executable rebuilt and route-verified: 12.093 ms covered GPU,
821 branch records, 324,958 cached proposal reuses, zero faults/fallbacks,
40,529 affordable-resident camera refinements. Evidence: refinement-restored.log
and its GPU sidecar. Production limitation is deliberately reported, not hidden.

## Integrated dormant refinement queue (5 November 2026)

- [x] Retain rejected complete proposals outside active hierarchy queues. Wake
  only after active work drains and allocation/pending demand changes. Cached
  camera/projection/page costs remain immutable within one frame encoding.
- [x] Stable parallel wake compaction, with separate camera and shadow prefixes.
  Do not wake shadows merely to repark them while camera work has priority.
  Remember shadow backlog even if the camera batch consumes zero extra capacity.
- [x] Bounded dormant storage, complete-parent fallback on overflow, reuse of a
  parked slot on another rejection, and reset between frame encodings.
- [x] GPU regression fixtures: capacity refund, two-record dormant overflow,
  repeated-frame reset, and zero-cost camera/shadow wake/no-progress behavior.
  Existing complete-cut, paging, generation, pose, shadow and lifecycle fixtures
  still pass: 144 total, including ignored GPU cases, serial execution.
- [ ] Performance acceptance. Integrated result is slower than baseline and
  far outside the 0.5 ms four-view selection allocation. No gate is completed.

This replaces one 80-byte-per-entry proposal scratch buffer with a 96-byte
parked task+proposal buffer: net +16 MiB at the current 1,048,576-entry capacity.
Active proposals use one buffer. Dormant indices are stable within the frame;
consumed records are inactive, and a rejected woken task reuses its record.
`dormant_records` counts records allocated this frame, not currently sleeping
entries. `dormant_wakes`, `dormant_park_events`, `dormant_wake_passes` and
`dormant_overflows` expose the actual scheduler activity. Explicit diagnostics
preserve these counters; normal scheduling never reads them back to the CPU.

| Native720p / one view | Covered GPU medians ms | Selection medians ms |
| --- | --- | --- |
| Baseline (alternating) | 12.081 / 12.091 / 12.038 | 5.814 / 5.746 / 5.838 |
| Integrated dormant queue | 15.026 / 15.047 / 14.927 | 8.627 / 8.701 / 8.575 |

120 warm timestamp samples per route; three repetitions each, identical route
and settings. This is covered GPU time, not wall-frame time or target acceptance.
Earlier dormant implementations took 18.25-18.43 ms with serial wake scans and
15.87-16.04 ms with parallel scans that woke/reparked shadows unnecessarily.
Both optimizations preserve the final selected representations in this route.

Final counters: 149,699 allocated dormant records, 259,096 wakes across eight
wake passes, 353,953 park events, zero dormant overflow. 1,701 branch records,
382 source slots (124 camera-visible), 1,319 voxel slots (66 camera-visible).
No emergency fallback, slot failure, allocation fault or active-frontier overflow.
Camera splats reserved 4,194,303; shadow splats 4,194,304. Conservative maximum
camera spatial error 654.98 px (not actual image error); 40,424 over-target nodes.
Camera coverage and error remain unaccepted. Matched non-HUD RGB image MAE is
~0.690/0.698/0.504 on 0-255 channels; visual comparison confirms only a small gain.

Source: `foliage_refine.rs`, `shaders/foliage_refine.wgsl`, diagnostics in
`foliage_runtime.rs`, fixtures in `foliage_select.rs` and `foliage_refine.rs`.
Evidence: tracked `dormant-refinement-measurements.json`; local
`build/playtest-unified/dormant-consumers-{before,after}-{1,2,3}` route logs,
PNG/GPU/CPU sidecars, `dormant-consumers-comparison.jpg` and
`dormant-consumers-tests.log`. Final test runtime 22.69 s; release example build
1m58s. Release compilation remains a substantial iteration cost.

## 5 November: consumer-scoped publication prerequisite

Camera/shadow roles survive selection, duplicate request merging, shared
residency and materialization. A newly added consumer rejects old motion
history. Cull and voxel choices suppress records used only by the other
consumer, including perspective light views. Shared payload ownership and
poses are retained. GPU buffer-limit validation now requires 15 storage bindings.

145 serial foliage tests and composed cull shader validation PASS; the normal
route also ran after the device-limit guard correction. Three alternating
baseline/new runs, native1280x720 ONE VIEW, 240 route frames, warm samples
frame>=120 (120 samples/run):

| Run | Before covered GPU ms | After covered GPU ms | Before selection ms | After selection ms |
| --- | ---: | ---: | ---: | ---: |
| 1 | 15.022 | 15.030 | 8.596 | 8.611 |
| 2 | 15.100 | 15.005 | 8.651 | 8.536 |
| 3 | 14.775 | 15.019 | 8.504 | 8.600 |

No meaningful speedup claimed. The final cut remains 1701 branch records;
camera publication reports124 source and66 voxel records, excluding258 source
and1253 voxel shadow-only records. Previous source_slots counted all consumers;
use selected_source_consumers for like-for-like comparisons across this change.
No fallback, slot/hash failure, refinement fault or dormant overflow.
Non-HUD RGB mean absolute difference for matched first-run final images is
[0.0000244,0.0000159,0.0000342] on a 0-255 scale; visual inspection found no
new difference. Existing coarse crowns and unaccepted appearance remain.

Full per-pass medians and final diagnostics: consumer-publication-measurements.json.
Local images/logs: build/playtest-unified/consumer-publication-{before,after}-{1,2,3}.*;
comparison: consumer-publication-comparison.jpg. No interactive playtest started.

G3.2 and G3.3 remain OPEN. This preserves consumer ownership; it does not provide
independent hierarchy cuts or upstream history-first/current-depth recovery.
Whole-root direct/fallback paths remain shared. No 4K/four-view acceptance.

## 5 November: rejected paged whole-volume occlusion experiment

Found an integration gap: the existing spatial two-phase brick-HZB path is
disabled for paged forests. Connected current-frame whole-volume rejection
without assuming the legacy static brick catalog exists in paged storage.
Near volumes (<150m, inherited spatial seed policy) raster first; current depth
then rejects fully hidden far-volume bounds before cell emission. Bounds include
billboard/jitter support. Added an actual GPU oracle for blocked/visible boxes,
near/far-plane crossings and a sky hole that opens/closes immediately.

Validation: 145 serial foliage tests, composed shader validation and new GPU
occlusion oracle PASS. Three alternating native720p ONE-VIEW normal routes,
240 frames and 120 warm GPU samples per run:

| Run | Before covered GPU ms | Candidate covered GPU ms |
| --- | ---: | ---: |
| 1 | 14.839 | 15.178 |
| 2 | 14.971 | 15.100 |
| 3 | 15.072 | 15.151 |

Final submitted splats stay 1,364,203 in every run. First-run camera generation
plus raster changes from0.689+0.683ms to0.093+0.051+0.081(HZB)+0.592(cull/generate)
+0.681(second raster)ms: no saving. Matched final images have RGB MAE
[0.0000256,0.0000208,0.0000317]/255 and visually match, retaining coarse crowns.
This is one route, not proof that whole-volume rejection never helps.

REJECTED as the normal implementation: restore production0741ccb, keep tested
patch paged-volume-occlusion-experiment.patch and complete per-pass measurements
in paged-volume-occlusion-measurements.json. No new opt-in mode or gate.
Local captures: build/playtest-unified/paged-occlusion-{before,after}-{1,2,3}.*.

Next implementation needs paged sub-volume/brick bounds and compact work ranges,
so coarse tree bounds do not force all cells to run. Reuse the same page ownership,
pose and conservative world-space support. This experiment is downstream of
selection and cannot solve the ~8.6ms hierarchy bottleneck; G3.2 upstream
history/current-depth recovery and G3.3 independent consumer cuts remain OPEN.

## 5 November: bounded paged voxel groups integrated; culling rejected

Each voxel asset page now groups occupied cells by deterministic 4x4x4 origin.
The original contiguous cell-range header remains valid for camera and sun
consumers. A second logical header addresses group descriptors stored in the
same bounded index arena: physical cell-range start/count and local group origin.
All instances share these asset records. Metadata is included in pinned sizes,
allocation and demand budgets. Grid, cells, ranges and group descriptors upload
before readiness is published; relocation and eviction use one ownership domain.
The atomic publication payload grows from 40 to 48 bytes; minimum upload budget
is 52 bytes. Existing default byte budgets are unchanged.

146 foliage tests PASS on the final production code. New sparse-cell coverage
oracle checks every occupied cell appears exactly once, groups contain <=64
cells and grouping is deterministic. The bounded two-page GPU fixture reads
actual group headers/ranges after repeated eviction/reload, checks partial pages
stay unpublished, and checks upload limits, pinned ancestors and payload bytes.

Three alternating baseline/new runs per scene, native 1280x720 ONE VIEW, 240 route
frames, 120 warm GPU samples/run. Full per-pass medians and counts are in
paged-brick-measurements.json. Baseline is a90a1f9 (renderer0741ccb).

| Final data-only integration | Before covered GPU ms | After covered GPU ms |
| --- | --- | --- |
| Elevated route | 14.705 / 14.639 / 14.734 | 14.809 / 14.709 / 14.700 |
| Low camera, pos [0,8,0] | 15.517 / 15.533 / 15.784 | 15.582 / 15.504 / 15.636 |

No speedup claimed. Final hierarchy counts and images are effectively unchanged:
non-HUD per-channel RGB MAE below 0.00006/255. Visual inspection still shows
coarse crowns and unaccepted appearance. No interactive playtest started.
Local comparisons: build/playtest-unified/paged-groups-{high,low}-comparison.jpg.

Two actual renderer integrations were tried and rejected before retaining only
the asset data. The 150m spatial seed / current-depth group tests reduced final
low-camera splats from 1,451,829 to 1,016,945, but took 16.952-17.022ms versus
15.940-15.958ms baseline. The elevated route rejected no final splats and slowed
from 14.832-15.048ms to 16.371-16.533ms. A min/max depth hierarchy classified whole
volumes as hidden/visible/uncertain before testing groups. It reduced overhead,
but remained slower: elevated 15.980-16.253ms vs 14.751-14.890ms, low 16.493-16.821ms
vs 15.771-15.875ms. Matched images remained effectively unchanged. Both variants
passed foliage tests and an actual GPU sky-hole/crossing/recovery oracle.

Preserved final culling/classification implementation plus oracle:
paged-brick-culling-experiment.patch (applies on top of this data integration).
It is not an enabled option or an acceptance gate. Do not repeat unconditional
per-brick eight-corner HZB queries: many far groups are cheaper to rasterize than
to test. Future scheduling must account for actual saved work and amortize tests
across hierarchy levels; upstream visibility must also prevent paying selection
cost for hidden detail. G3.2/G3.3 and full G4 acceptance remain OPEN. No 4K/four-view
or 8.33ms acceptance. Continue the full architecture, not more standalone culling
threshold experiments.

## 5 November: whole-root consumer publication and shadow-cache correctness

Direct resident ancestors and emergency root fallbacks now carry camera/shadow
roles through the existing GPU demand record into materialization. No new buffer,
CPU selection, per-instance asset copy or opt-in mode. Camera-only roots do not
cast shadows; shadow-only roots do not render into camera visibility. Adding a
consumer invalidates root motion history without leaking role bits into source
visibility flags. Shadow damage now includes shadow-only root volumes and clears
old caster tiles when they become camera-only or hidden.

Validation: 147 foliage tests (including ignored serial GPU oracles), six shadow
cache tests, composed shader validation and DLSS-enabled release example build
PASS. The new raw GPU test covers direct ancestors, explicitly forced fallback,
consumer transitions and motion epochs. Existing missing-page ancestry test now
requires camera-only publication. Shadow damage tests read actual cleared depth
as well as dirty bits for shadow-only/camera-only/hidden transitions. Existing
cached sun versus full redraw depth test also passes.

Normal integration: two batches of three alternating before/after runs for each
of high and low camera routes. RTX5070Ti/Vulkan, native1280x720 ONE VIEW, lowland
with grass, 240 frames, medians of120 warm GPU samples per run. Covered GPU ms:

| Camera/batch | Before | After |
| --- | --- | --- |
| High1 | 14.068 / 13.612 / 13.655 | 13.716 / 13.623 / 17.132 |
| Low1 | 16.201 / 15.340 / 15.160 | 17.778 / 15.128 / 16.934 |
| High2 | 13.760 / 13.794 / 15.692 | 13.860 / 13.807 / 14.785 |
| Low2 | 15.754 / 15.285 / 16.124 | 15.747 / 16.877 / 15.747 |

Selection timing varies substantially; the second batch does not reproduce a
consistent slowdown. Keep both batches: this is NOT proof of a speedup or firm
performance neutrality. Raster, terrain shading and materialization are broadly
stable. Selection counts/error pressure are unchanged. Matched final images have
non-HUD mean absolute error below0.000057/255 per channel; visual review still
shows coarse rounded crowns, hard dark shadows and repetitive coarse grass.
No visual-quality acceptance and no 4K/four-view/120FPS claim.

Full per-pass run medians and counters: root-consumer-measurements.json.
Local evidence: build/playtest-unified/root-consumers-{high,low}-{before,after}-*
and root-consumers-repeat-* (logs, GPU/CPU JSON, PNG and JPEG comparisons).
Tests: root-consumers-{tests,shadow-tests,shaders}.log. G3.2 upstream recovery
and G3.3 independent consumer cuts remain OPEN; this closes their root publication
prerequisite, not traversal or detail selection. No interactive playtest opened.

## 5 November: independent camera/shadow class cuts integrated

GPU tasks now refine camera and shadow demand independently. Each root has a
camera and a shadow draw record referencing the SAME asset, pose and canonical
source owner. Matching branch requests deduplicate into shared physical residency.
Whole-root histories invalidate their shared owner when either publication changes.
No CPU selection, CPU near-tree pool or opt-in route was added.

Live consumer tasks are compacted in stable root/role order on the GPU; inactive
world capacity does not enter refinement scans. Initial raster reservations are
summed per workgroup instead of one contended atomic sequence per root. Raster
passes dispatch only their own root range plus the shared branch prefix.

Important fixes from the first normal-route check:
- Camera priority now defers only shadow upgrades requiring MORE SHARED BRANCH
  SLOTS. Spending the separate shadow raster budget cannot starve camera work.
  Resident shadow ancestors no longer wait behind unrelated camera refinement.
- A consumer deadline/fallback suppresses ONLY that consumer's cut. Physical
  allocation/hash/record overflow still invalidates both partial cuts atomically.
  Residency filters consumer roles before publication, including retained slots.
- The rejected intermediate caused76,049 emergency root fallbacks in the low
  route and huge near-camera spheres. Final high AND low routes have zero
  emergency fallbacks. This does not imply acceptable projected voxel detail.

Validation:152 foliage tests,2 vegetation-cell tests,6 shadow-cache tests and
composed shader validation PASS. New raw GPU oracles cover different camera/
shadow detail, missing pages, stable sparse/empty-world compaction across workgroup
boundaries, exact initial reservations, exhausted parked storage, consumer-local
fallback, and shadow raster admission while cameras wait for shared slots.
Existing cached-versus-full shadow depth, population lifecycle, four-camera
teleports and shared articulated pose tests also pass. Release DLSS build passes.

Final normal renderer:RTX5070Ti/Vulkan, native1280x720 ONE VIEW, lowland+grass,
240-frame route,120 warm GPU samples/run. Three alternating baseline/final runs:

| Camera | Before covered GPU ms | After covered GPU ms |
| --- | --- | --- |
| High | 13.899 / 13.902 / 13.927 | 13.948 / 13.969 / 13.946 |
| Low | 14.781 / 14.784 / 14.778 | 15.032 / 15.044 / 15.056 |

This is NOT a speedup. High route is roughly unchanged; low is about0.26ms slower.
High selection median changes8.019->7.815ms, low7.978->7.962ms. Materialization
rises0.142->0.228ms; shadow damage0.286->0.402ms. Low-route shadow work increases
because resident caster detail replaces52,726 baseline shadow-only emergency
fallbacks. Camera raster stays~0.728ms. Low camera source records169->183, voxel
records282->127; over-target camera nodes42,083->41,943. Physical pose requests
503->192. These are route diagnostics, NOT4K/four-view or gameplay claims.

Image review: mixed source/coarse crowns and hard shadows persist. High/low
non-HUD RGB MAE are~3.24/~12.75 at worst channel, not image equivalence. Near-ground
shapes/shadows change; oversized rounded cells are explicitly NOT accepted.
The user correctly points out that20-pixel spheres violate the pixel-scale voxel
goal. Current fixed selected-level publication can retain such a coarse level
under budget/deadline pressure; the rasterizer cannot invent finer payloads.
Do NOT shrink splats to hide this: preserve coverage with finer cells/source
geometry. Actual projected-cell footprint distribution and retention reasons
must be measured under existing G1.4/G8 work, then fixed; shader appearance polish
is not a substitute.

Remaining G3 gaps: independent PER-VIEW cuts within each class, upstream
history/current-depth recovery and conservative shared-slot accounting (matching
requests deduplicate physically but admission still charges each request).
G2/G3/G8 remain open. Extra root publication/history storage is128 bytes per
reserved root before secondary renderer/cache bookkeeping; assets are not copied.

Full final and rejected-intermediate per-pass medians/counters:
independent-consumer-cut-measurements.json. Local evidence:
build/playtest-unified/independent-cuts-final-{high,low}-{before,after}-*;
comparison JPEGs and test/build logs use independent-cuts-* names. No playtest.


## Actual raster cell footprint diagnosis (5 November)

User-visible oversized spheres are confirmed in the normal renderer, not only
in a synthetic fixture. Evidence: [vegetation-cell-footprints.json](vegetation-cell-footprints.json).
Native 1280x720 ONE view, lowland with grass, 240 frames at 1 m/s, final-frame
offline GPU record readback. 1,453,836 stored splats, no record overflow or
near-plane crossings. 5,563 submitted quads exceed 8 px; 836 exceed 16 px.
Largest measured diameter is 24.04 px, from an articulated group with a
0.730 m world cell. Thus this is not exclusively a whole-root fallback problem.
This measures submitted quad diameter before depth testing, not visible pixel
area, and excludes sub-cell jitter. It is not a performance comparison.

The selector reserved 4,193,945 camera splats (limit 4,194,304), while the
raster list contains 1,453,836. 41,943 camera cuts exceed the requested error;
10,495 cannot afford the next camera raster reservation, 34 have a missing
page, and 31,414 have a resident, affordable next step at final audit. These
are final-cut classifications, not per-cell causal attribution. The next
architecture work must address traversal stopping with affordable work left
and upstream visibility before reserving complete cell payloads. Increasing
budget alone does not resolve both problems. Do not shrink discs to hide
this failure: preserve coverage using finer payloads/source geometry.

The explicit --voxel-raster-profile diagnostic now retains the 16 largest
submitted cells (world position, cell size, volume, opacity) and bins through
16/32/64/128 pixels. No frame-time readback or CPU selection was introduced.
Release build and normal-route readback pass; rendering is unchanged. G1.4,
G2/G3 and G8 remain open; no pixel-scale visual or 8.3 ms acceptance.


### Traversal deadline experiment (rejected, 5 November)

Doubled encode rounds from 2*(hierarchy depth+1) to 4*(depth+1), with the
same asset/branch/raster budgets. One lowland native720p single-view diagnostic
route each; not an alternating repeated performance claim. Evidence:
[vegetation-refinement-deadline-experiment.json](vegetation-refinement-deadline-experiment.json).

Affordable unfinished camera cuts fell from 31,414 to zero, but over-target
camera cuts stayed at about 41,949. Largest submitted cell remained 24.04 px;
787 articulated-group cells still exceeded 16 px. The image still has blatant
spherical near-tree cells. The final budget now has only three camera records
free: individually affordable requests compete for the same remaining capacity.
The earlier affordable-count diagnosis did NOT mean all 31,414 replacements
could fit together. Longer traversal alone does not fix the image.

Production round count restored; no new mode. Next work must address upstream
visibility/budget reservation and screen-error priority across near/far cuts,
rather than multiplying traversal passes or shrinking coverage discs.


### Screen-error-first allocation experiment (rejected, 5 November)

Replacing error-improvement-per-reserved-record priority with current projected
error priority improves the two nearby trees, but shifts coarse spherical detail
to grass and other trees. The high-camera comparison shows notably worse coarse
grass coverage. Do NOT integrate as a visual fix. Original priority restored.
Patch plus focused GPU allocation test preserved in
[screen-error-priority-experiment.patch](screen-error-priority-experiment.patch);
full per-pass medians in
[screen-error-priority-measurements.json](screen-error-priority-measurements.json).

Three alternating before/after runs at native720p, ONE view, 240-frame routes,
GPU samples frame>=120. Low-camera covered GPU: before 14.780/14.764/14.737 ms,
after 14.878/14.877/15.177 ms. High: before 14.050/14.011/13.755 ms, after
13.974/13.707/13.721 ms. No overall speedup claim. Existing 153 foliage tests
and the new focused cost-versus-error priority test pass. Low-camera cells above
16 px drop from 836 to 112, but maximum rises from 24.04 to 27.24 px and the
coarse-grass failure remains. MAE is substantial; not visual equivalence.

The integration obstacle for G3.2 is concrete: frame.rs calls
forest.encode_timed before current camera depth, then tree_cull_and_raster
builds current depth only AFTER residency/materialization. That downstream
culling cannot refund upstream reservation. A correct early path must draw a
valid seed cut at CURRENT poses, build per-camera depth, recover deferred groups
and publish complete cuts with shadow-class demand retained. Historical depth
alone must never discard a group; moved/recycled owners and rejected partial
cuts cannot remain occluders. Preserve old cut/page lifetime through the seed
phase and final publication. Continue G3.2 here rather than more budget, round
count or priority-only changes.


## Current terrain depth before refinement (5 November)

Normal frame.rs now draws a bounded terrain-only seed per camera, builds current
reversed-minimum depth, and classifies persistent GPU roots before their detail
reservation. This is the first upstream visibility stage, NOT full two-phase
vegetation occlusion. No old tree slots, old poses, historical camera depth or
CPU shortlist are used as evidence. Visible root bits union all camera consumers;
a camera looking away contributes nothing. Offscreen-edge uncertainty and
near-plane/wind-bound crossings fail open. Shadow roles are never removed.

Each camera uses its actual current jittered projection/internal viewport.
Root bounds include authored wind support. Missing cameras or mismatched matrices
reject the API call. Terrain seed capacity is 8192 clusters: missing occluders
reduce rejection rather than create holes. Normal final terrain rendering stays
complete. Terrain hidden by geometry diagnostics also leaves seed depth empty.
The seed's visibility/depth/pyramid targets and aligned camera uniforms/bind
groups are reused across frames, recreated on resize. A four-byte visibility
word per reserved root is added; the refiner now requires 16 storage bindings.
Seed render targets and cull scratch are additional memory/work, not free.

Validation: 154 foliage tests, release terrain_fly build, six alternating runs
per camera route (three each baseline/candidate). New GPU oracle checks actual
root-demand masks for clear/blocked transitions, two/four cameras disagreeing,
a camera looking away, missing consumers and wind bounds crossing the near plane.
Existing lifecycle/cached-shadow/consumer tests pass. No allocation/frontier
faults or emergency fallbacks in measured routes. Evidence:
[vegetation-current-depth-measurements.json](vegetation-current-depth-measurements.json).

Native720p ONE view, 240 frames, samples >=120:
- Lowland baseline 15.060/15.009/14.807 ms; current seed 15.337/15.057/15.241 ms.
  Camera roots 42189 -> 42186; shadow-only roots 52726 unchanged.
- Alpine baseline 13.838/13.851/13.846 ms; current seed 14.314/14.295/14.315 ms.
  Camera roots 29512 -> 28764; shadow-only roots 46714 unchanged.
- CPU wall frame time is not used as GPU time. These measurements predate the
  unrelated incoming HDR/exposure/VR/stone changes.

Images inspected: lowland gains some nearby detail as reservation changes, but
spherical cells remain; alpine broadly similar. RGB MAE about 5.25/2.32 at worst
channel, not image equivalence or a raw-depth correctness proof. This is a
correctness/architecture checkpoint with a measured cost regression.

Remaining G3.2: seed a safe current-pose vegetation cut as well, recover deferred
clusters against current depth, preserve page/owner lifetime through publication,
and avoid spending refinement on hidden branches inside otherwise visible roots.
Current root-only terrain rejection is insufficient for the dense forest. It
must not be presented as completed two-phase visibility, full per-view cuts, a
fix for 24px cells, or acceptance of the 4K/four-view 8.3ms target.

Integration check: rebased onto effa40f (HDR/exposure, VR, stone changes);
release DLSS-enabled terrain_fly build and a 60-frame headless lowland GPU smoke
run PASS. Smoke is not a steady-state performance sample. Logs:
build/playtest-unified/seed-rebase-build.log and seed-rebase-smoke.log.


## 5 November: runtime-sized consumer choices (capacity checkpoint)

Correctness first: oversized coarse cells are still a failed requirement. This
checkpoint removes the fixed 64 KiB/16,384-choice consumer table restriction;
it does not establish pixel-scale correctness for the normal forest.

- [x] Cull, source raster and voxel lookup use runtime-sized storage arrays.
  GPU forest view allocations grow with the physical slot budget; legacy
  authored tables retain their 16,384-entry capacity. Producer camera/shadow
  strides remain allocation-sized. Default branch budget remains unchanged.
- [x] Remain within sixteen storage bindings, including temporal voxels. Cull
  traversal diagnostics share the phase-counter allocation at byte 128. Voxel
  indirect draw arguments share diagnostics at byte 88, before the tile mask.
  Partial clears preserve independent lifetimes and draw offsets are explicit.
- [x] Real GPU boundary oracle uses each production consumer's declaration and
  decoding functions at indices 0, 16383, 16384, 32767, 32768 and 65536. All
  match CPU unpacked values. GPU partial-clear lifetime oracle passes.
- [x] Focused release tests: voxel_forest 28, cluster_traversal 6,
  tree_occlusion 7; includes ignored exclusive-GPU tests on NVIDIA/Vulkan.
- [ ] Normal renderer capture above the former capacity limit, native and
  temporal paths, with source/shadow/voxel counters and image inspection.
- [ ] Full-forest strict one-pixel acceptance and motion/coverage validation.
- [ ] Repeated per-pass measurements and four-view target acceptance.

No visual fix or speedup is claimed. The relaxed GPU population guard still
checks the virtual owner encoding; hardware buffer limits remain applicable.
Tests above ran before rebasing onto concurrent main changes. The obsolete
uniform-declaration assertion failed initially and was removed from the
unrelated per-ray arithmetic test; the actual GPU boundary oracle replaces it.
