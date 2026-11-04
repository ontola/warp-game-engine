## 2026-11-04 Normal GPU forest shared sun tile cache connected

frame.rs now runs shadow_damage after common forest materialization, clears only
marked tiles, loads retained source depth and supplies the same mask to source
and voxel cached-depth fragment entry points. No opt-in. Near/headlamp/ordinary
entry points remain independent. Legacy static/dynamic split still redraws its
dynamic map. Cached source pipeline only constructed for GPU forest scenes.

GPU forest wind no longer unconditionally invalidates the full normal map. Actual
source/voxel pose/choice changes feed damage; legacy vertex-wind casters still
invalidate globally. Cache force conditions include invalid/unsubmitted content,
recreated targets, alpha/LOD changes, receiver-demand mode and skinning/crowd.
Projection and shader generation are damage keys. Content remains pending until
submission, so an abandoned frame forces the next update. Real map size is part
of the key/uniform and raster specialization (4096 desktop, 2048 Android), with
fixed 64x64 tile grid. Profiler label: shadow tile damage and clear.

CHECKS: cargo check --locked -p render --tests; final release shadow_damage::tests
with software DX12: 2 PASS (5.72 s). Naga covers damage, clear, complete source
raster and voxel splat raster. Raw depth oracle checks preserved texels with the
new map-size contract. Last small Rust specialization edit only supplies the
new tile-size override to cached source fragments; final cargo check verifies it.
Normal renderer hardware oracle was added and compiles but NOT RUN:
foliage_runtime::tests::gpu_cached_sun_depth_matches_full_redraw. Four views,
prewarmed assets, wind, unchanged poses, root removal/restore, sun/camera change;
raw shared depth versus invalidate_shadow/full redraw, with nonempty depth guard.

GPU occupied: former Growing Guns/Spellstorm sessions ended, but Sideways PID
51952 started 2026-11-04 17:37:11.739408 and was left alone. No hardware benchmark,
full-renderer acceptance or speedup claimed. Do not run the full renderer on WARP
again: previous attempt grew beyond 17 GB. Small shader fixtures are fine.

Still open: early dirty-bound rejection (current cull/vertex/splat generation runs
all candidates), world-stable/scrolling shadow coverage (projection changes force
all tiles), full normal-renderer raw depth + hardware timing validation, further
asset/terrain mutation coverage. Then remaining upstream two-phase publication,
unified meadow and far cells. This is integrated code, NOT a completed shadow
architecture gate. See docs/foliage-shadow-damage-2026-11-04.md.

## 2026-11-04 Shadow tile damage and sparse clear building block

Added shadow_damage.rs + shaders/shadow_damage{,_clear}.wgsl. NOT CONNECTED to
normal raster yet: no render savings or finished animated shadow cache claimed.
Do not simply disable frame.rs forest invalidation: both source and voxel raster
must honor the same tile mask after sparse clearing or stale depths remain.

GPU compares exact 128-byte source/voxel snapshots (including common shadow
choices) and marks old/new world bounds in a 64x64 tile mask (512 bytes). Source
sphere is already world-space, voxel low/high are world bounds. Removal, changes
and initial/epoch/projection invalidation covered. CPU does not select casters.
Sparse clear uses Load and only writes far depth in dirty tiles. No pose freeze
or quantization. Dense wind may dirty most tiles. Projection changes currently
force all tiles; stable/scrolling world coverage is still required.

Release compile, both Naga shaders, DX12 software-adapter functional test PASS:
`KERN_ADAPTER=software WGPU_BACKEND=dx12 cargo test --locked --release -p render
--lib shadow_damage::tests -- --include-ignored --test-threads=1 --nocapture`
2 passed, 1.41 s execution; build 4m40s including shared-target contention.
The oracle compares every pixel of a raw 64x64 Depth32Float buffer with the mask:
marked tiles clear to 1, all others remain exactly 0.25; also checks old/new
positions, hidden voxel/source, unchanged reuse, epoch and projection changes.
No hardware timings or production integration evidence. Both other games remain
live (CIM verifies PIDs 32908/22888). Own build/test sessions 16011 and 76378 have
exited 0; no owned test remains. See docs/foliage-shadow-damage-2026-11-04.md.

Next: use this beside shared shadow_cache, encode after GPU materialization;
attach identical mask to source raster and voxel splat raster (plus early bound
rejection); clear only dirty tiles, load retained depth; wire asset/terrain/
material/target invalidation epochs and fail/abandon invalidation. Source raster
currently groups0/1, voxel splat raster group0; uniform mask needs no extra
storage binding. VoxelRenderer::tree_buffer exposes current volumes; common
runtime choices contain shadow policy at word16384+slot. Source/voxel/page
mutation must be included. Keep near/lamp/legacy behavior independent. Full
redraw comparison + measured normal integration still required before gate pass.

## 2026-11-04 Complete resident intermediate cuts

Normal shared selector gates refinement on all visible immediate children being
resident. Missing detail requests all siblings, keeps the nearest loaded parent,
and leaves independent sibling subtrees detailed. Count and publication use the
same rule. Whole-root overflow fallback remains. No CPU visibility shortlist.
Updated raw-buffer regression covers four consumer views, eviction/reload,
intermediate parent loss and whole-cut budget rejection. Cold-renderer test now
expects direct resident ancestor selection rather than an error flag.

Release build and Naga selector validation PASS. Windows software DX12
(KERN_ADAPTER=software, WGPU_BACKEND=dx12) passes new resident-ancestor fixture,
shared-pose matrix/hash-overflow oracle (including indirect args introduced last
commit), wavefront vs serial cut / queue overflow, and union budget priorities.
The larger historical disocclusion/overflow software test timed out at the
30-second GPU poll limit (not a validated pass; physical rerun required).
No physical-GPU performance claims. Full renderer cold-start software test was
stopped deliberately after working set exceeded 17 GB; it did NOT finish or pass.
Owned PID 48336 creation 16:52:56.9713010 confirmed before Stop-Process; parent
session 15514 exited 1. Growing Guns 22888 and Spellstorm 32908 remain untouched.
Use CIM + Format-List or JSON for mixed process output: PowerShell table column
selection briefly hid live game rows; subsequent CIM confirms they remain live.

Still pending: hardware cold-start render/regression and alternating performance
runs for source paging + indirect poses + resident cuts. Software tests are a
useful functional check, not image/timing acceptance. Full architecture remains
open: animated shadow pages, upstream history publication, tighter ancestor pins,
unified grass and far spatial cells. See docs/foliage-resident-cuts-2026-11-04.md.

## 2026-11-04 GPU-sized shared pose depth dispatch

Normal forest pose evaluation now consumes GPU-written indirect dispatch counts
per ancestry depth instead of launching the full slot budget at every depth.
The 768-byte argument buffer is written in a separate preparation pass and is
not bound as storage during indirect evaluation. Zero-demand arguments are
rewritten each frame; parent order and serial hash-overflow fallback remain.
Root preparation still scans reserved capacity. Animated shadow caching is open.

Release test build + Naga validation of materialization and the new dispatch
shader PASS (`cargo test --locked --release -p render --lib
foliage_runtime::tests::materialize_shader_validates -- --exact`). Extended the
existing ignored GPU matrix oracle to check all 64 dispatch triples and stale
arguments after asset-fault fallback. GPU test and before/after measurements
NOT RUN: other games still live (Growing Guns 22888, Spellstorm 32908), untouched.
Run `cargo test --locked --release -p render --lib
foliage_runtime::tests::gpu_shared_poses_match_serial_wind_and_hash_overflow
-- --exact --ignored --test-threads=1` once GPU free, then normal renderer tests
and alternating measurements. No speedup claimed, no playtest opened.

## 2026-11-04 Source paging integrated with common forest publication

Normal GPU forest now creates bounded source vertex/index/triangle arenas before
binding the renderer. Stable cluster IDs address relocated payloads. Source and
voxel dependencies share GPU feedback, complete-bundle admission, generation
publication and frame upload budgets. Source page misses keep complete voxel
fallbacks; no CPU tree shortlist. Source admission still loads all source-branch
CLOD pages, not fine-grained selected-cluster faults. Host catalogs stay resident.

120 release foliage-filter tests PASS at 16:21:52. Includes raw source GPU payload
checks through partial uploads and twice eviction/reload, combined publication
where loaded voxels cannot publish missing source geometry, genuine normal source
cold-start, terrain fixed-address writes and compact static pins. Initial visual
probe exposed a terrain-address regression; corrected by preserving its reserved
vertex prefix (also skinning). Shared terrain topology is retained once per page;
contiguous cluster headers upload together and identity prefixes avoid CPU maps.

Default: source buffers 76,508,384 bytes including pinned terrain/ordinary assets;
1 -> 16 source pages, voxels 27 -> 507, all complete by frame 63. Joint upload peak
1,204,984 bytes. 62-MiB vertex pressure leaves 2 source pages, uses coarser voxels,
no churn. 61 MiB explicitly rejects pinned-data overflow. Images MAE ~0.022/255.
Docs/evidence: docs/foliage-source-pages-2026-11-04*. Local build/foliage-source.

IMPORTANT: final performance runs overlap OTHER agents' games. Spellstorm PID
32908 started 16:22:32; Growing Guns PID 22888 started 16:25:07. They were left
untouched. Our final session 31085 has completed, no owned GPU jobs remain.
Do not start GPU tests/benchmarks until checking those processes are gone (or
explicitly authorized coordination). Previous corrected version measured
10.225 -> 10.293 ms covered, but final CPU-startup-cleaned binary requires a new
uncontended measurement. Final timings rejected, saved historical summary retained.

NEXT: confirm final source paging timings when GPU free. Open gates remain:
intermediate ancestor fallback/tighter pins, shared consumer-aware upstream
history/publication, animated shadow caching, unified meadow/trees, far cell
aggregates and growth/content lifecycle. Current dynamic shadow invalidation is
every frame; future caching MUST invalidate on asset generation changes too.
Architecture/8.33-ms target NOT complete. Goal active.

## 2026-11-04 Bounded normal-path voxel cache and stable surface identity

Normal GPU forest now owns bounded index/cell/splat arenas, async GPU demand
feedback, whole-dependency-bundle admission, chunked uploads, LRU eviction and
generation-safe publication. Root fallback stays pinned. CPU services asset IO;
GPU still selects roots/branches. Source geometry and CPU catalogs remain fully
resident. All 116 foliage-filter checks PASS, including real two-page eviction,
44-byte frame upload budget/raw payload checks and normal-renderer cold start.

Fixed another address-dependent appearance defect: splat raster and rounded
surface normals now share root-generation/branch/local-cell identity. Across
three alternating same-shader control/cache pairs RGB MAE 0.02365-0.02371/255,
p99 1. Native 1713x964 one-view covered medians 10.583 -> 10.625 ms; no speedup
claim. 4K/four-player 8.33 ms remains unproven. Separate feedback/upload queue
cost is outside main-frame timestamps. Default loads 27 -> 507 pages by frame
61, 65,908,804 allocated cache bytes. 48-MiB cell pressure: 327 resident pages,
62,257,508 bytes total, zero churn over 180 frames, reduced-detail fallback.

Evidence: docs/foliage-cache-2026-11-04.md and matching evidence JSON; local
build/foliage-cache. Temporary unstreamed control edit restored. A stale mtime
initially reused the control binary; excluded those results, rebuilt with fresh
mtime and guards on executable hashes/nonempty cache telemetry.

NEXT: source paging needs ONE publication owner combining source + voxel
availability. Independent writers to selector resident bits would be unsafe.
Keep logical cluster metadata stable, relocate vertex/index payloads, explicitly
pin non-vegetation/skinned ownership. Also open: tighter ancestor pins and
intermediate fallback, upstream shared history/publication, animated dirty-page
shadows, unified meadow/trees, far cell aggregates, growth/content lifecycle.
Goal remains active; architecture is not complete. No interactive playtest opened.

## 2026-11-04 Relocatable voxel pages on the normal renderer upload

New foliage_pages cooks self-contained dense/brick levels, local cell payloads
and splats. Stable logical level IDs address independently allocated physical
ranges. Normal GPU forest upload uses these pages for camera/sun/ray consumers.
Generation-checked bounded arena rejects stale frees, coalesces fragmentation.
All 114 foliage-filter checks pass, including exact cell/normal/splat relocation.

Image comparison found physical cell IDs seeded voxel jitter and faint thinning.
Both now use local coordinates. Same-shader unrelocated control vs relocated
images: RGB MAE 0.288-0.292/255, channel p99 7; no obvious missing trees/shadows.
Three alternating native 1713x964 one-view runs: covered 17.018 -> 16.998 ms,
forest prep 0.682 -> 0.676 ms (no performance claim). Docs and evidence:
docs/foliage-page-layout-2026-11-04.md; artifacts build/foliage-page-layout.
Temporary control-build edit was restored. No shipped alternative/opt-in route.

STILL OPEN: initial upload keeps all pages resident (no memory saving). Retain
page ownership, map representation dependencies, async requests, bounded uploads
and eviction; source geometry paging also missing. Adaptive shadow choices use
voxels even for camera-source branches: keep all applicable mip dependencies.
Root fallback ranges pinned. Instance seed still uses the existing transient
volume index; stable world identity on slot replacement needs follow-up.
Other gates remain: upstream consumer-aware history/publication, animated shadow
cache, unified grass and far cell aggregates. Goal active, not complete.

## 2026-11-04 GPU logical asset demand and generation-safe publication

Normal forest selection now records/compacts unique logical representation-page
requests and rejects incomplete cuts when a page is missing. Root pages are pinned.
Asset generation changes retire affected branch slots; direct-root history uses
a publication epoch. Batch validation has no partial GPU effects. All 111 foliage
checks pass, including explicit missing-page/fallback/republish/invalid-batch tests.

Physical source/voxel catalogs are STILL fully resident. This is a prerequisite,
not a bounded physical page cache or memory saving. Next: relocate source geometry
and voxel grids/cells/splat lists with shared dependency tracking, bounded upload
service and generation-safe eviction. Keep architecture gate open.

Three alternating native 1713x964 one-view routes: preparation 0.573 -> 0.674 ms;
covered frame 16.885 -> 16.861 ms (no speedup claim). Unique request atomic-load
guard reduces initial overhead. Same cut counts; image RGB MAE 0.290-0.293/255.
See docs/foliage-asset-publication-2026-11-04.md + evidence; build/foliage-pages.
No playtest opened. Other remaining gates: upstream shared history/publication,
animated shadow cache, meadow migration, far cell aggregates. Goal active.

## 2026-11-04 Shared GPU ancestor poses on the normal forest route

GPU demand hash + per-depth queues now deduplicate selected branch ancestry.
Root poses are shared; source, voxel and shadow transforms consume identical
poses. Fixed bounded per-depth dispatches use GPU queue counts; no CPU shortlist.
Hash overflow recomputes the current chain, never reads stale poses. A forced
one-slot cache and three nonzero-wind phases pass the independent matrix oracle.
All 111 foliage-filter tests pass. No interactive playtest opened.

Three alternating native 1713x964 one-view flights: forest preparation 0.533 ->
0.579 ms, shared poses alone 0.038 ms; covered frame 17.313 -> 16.938 ms. This is
not a proven speedup (other passes vary). Final 4,020 unique/evaluated poses, zero
hash failures, same selection cut. See docs/foliage-shared-poses-2026-11-04.md and
its evidence JSON. Baseline/current binaries and captures: build/foliage-poses.

Remaining: upstream consumer-aware history/publication; asset paging with real
memory budgets; animated shadow cache; grass migration; far cell aggregates.
Pose depth dispatch/root processing could be compacted further. Goal active.

## 2026-11-04 History-first source recovery and sparse-coverage policy

Normal GPU forest cameras now use pixel-history scheduling and a bounded GPU
queue of deferred source clusters, with current-depth recovery and complete
complementary scan on overflow.64 KiB/view history allocation (32 KiB Bloom
payload);6 MiB deferred tail/view. GPU-only policy renders the complete valid cut
when occlusion saves nothing, probing again every32 frames. Shadows independent.

110 foliage +10 history-filter tests pass (overlap), including forced capacity1
RGBA parity. Three alternating baseline/final native1713x964 one-view flights:
17.206 ->17.047 ms covered GPU median, effectively neutral; first attempts19.2–
19.3 ms regressed. No four-player/120FPS claim. Small draw-order pixel differences
remain; image inspection found no obvious missing trees. No interactive game.

See docs/foliage-history-2026-11-04.md and evidence JSON. Source recovery is
DOWNSTREAM of shared selection: upstream consumer-aware history/publication,
asset paging, shared pose scheduling, animated shadow cache, grass migration and
cell aggregates remain open. Goal remains active; do not declare full Nanite.

## 2026-11-04 Stable streamed population, no renderer rebuild

Normal terrain route uses WorldRoot stable IDs, reserved root slots/generations,
worker-prepared population transactions and bounded GPU scatter publication.
Unchanged roots retain branch residency and motion history; stale transactions,
invalid roots and capacity excess are rejected. CPU does world generation, not
per-frame selection. Forest/ground/sun recenter independently of renderer lifetime.

109 foliage-filter tests pass including exclusive GPU fixtures. 300-frame native
1713x964 one-view route crosses three forest updates: 53xx–69xx roots changed,
48k–57k retained. Final CPU publication 5.20/4.19/4.36 ms versus the first synchronous
36–61 ms implementation (engineering observations, not paired frame benchmarks).
GPU covered median 10.113 ms on this moving route excludes the separate scatter
submission. No 8.33 ms/four-player claim; terrain/readback hitches remain.

See docs/foliage-population-2026-11-04.md and its evidence JSON. Root capacity is
fixed (demo reserves 2x initial, minimum16k); no asset paging or automatic growth.
Next: safe history-first traversal/publication across camera+shadow consumers,
then asset pages, shared pose/shadow caching and grass migration. Do not claim
these gates complete. No interactive playtest opened.

## 2026-11-04 GPU subtree work queues on the normal route

Selector::union uses GPU-generated bounded frontiers for both demand and emission.
One workgroup processes a shared child span; only refining subtrees re-enter the
queue. No serial per-root subtree walk or CPU work-count readback on this route.
The historical-HZB fixture remains separate; do not claim history-first production
selection, cell streaming, page streaming or cached animated shadows are complete.

Six selector + three runtime tests pass, including exact cut parity, forced queue
overflow/recovery, teleport, four views and off-camera casters. 62,274-root scene,
three repeated 80-frame native1713x964 one-view runs: selection/residency/poses
0.474 ms, GPU covered17.015 ms. Zero queue/slot/hash failures, peak child frontier
3,048. The first per-node-queue attempt overflowed and is rejected, not a speedup.
See docs/foliage-wavefront-2026-11-04.md and its evidence JSON. Images inspected;
no interactive playtest opened. Next architecture work: stable cell/root updates
without renderer rebuild, and safe history-first publication across consumers.

## 2026-11-04 Architecture first: normal GPU forest route

User explicitly requires coherent production architecture before visual polish,
not another opt-in checkpoint. See docs/vegetation-architecture.md for ownership,
frame graph, implementation order and honest connected/missing status.

terrain_fly no longer constructs or updates recipe_forest::Pool, sorts nearest
trees on CPU, or carries an alternate fir/card backend. All roots use the GPU
runtime. --branch-slots controls the shared resident branch budget (4096 normal,
512 low profile). Retired CPU-mode arguments fail explicitly. RootPlacement now
belongs to foliage_runtime; recipe_forest::Tree is a compatibility alias.

This does NOT complete page streaming, work-queue traversal, shared grass,
incremental world-cell updates or animated shadow-page caching. Scene recentering
still rebuilds the renderer. Existing game callers are not silently migrated.
Historical benchmark reports below retain their original experimental status.

## 2026-11-04 Actual GPU forest runtime (historical opt-in measurement)

`WARP_TERRAIN_GPU_FOREST=1` in terrain_fly now connects shared recipes,
GPU demand/complete-cut budgeting, persistent slots, articulated branch wind,
source/voxel draws and separate sun voxel choices. No Pool construction/update
in that route. Default is unchanged. 4096 branch outputs, not 240 full trees.
Full scene tested with 62,274 roots. A parallel blocked prefix replaced the
serial budget scan; selection/residency/poses median 0.789 ms for that population.

Three alternating pairs per size, same executable, native1713x964 one view:
500 m old/new GPU covered 16.078/15.927 ms; 2500 m 16.347/17.335 ms. No overall
speedup or 120 FPS claim. Root fallback still causes visual differences. Full
report, per-pass JSON, tests and exact executable hash:
`docs/foliage-runtime-2026-11-04.md`. Raw captures `build/gpu-forest-20261104-parallel`.
No interactive playtest launched. Video reference uR5EvBWxN14 is user-supplied
architecture context, not independently verified imagery/performance.

Corrected stale voxel-composite timestamps, source needles mistakenly used for
near shadows, inflated canopy bounds, and branch-pack limits applied to roots.
Tests cover actual source/voxel wind rendering, budget exhaustion, teleports,
four views, off-camera sun casters, generations and deterministic near priority.
Remaining: workload-balanced/history-first cluster traversal, animated shadow
page caching, far spatial-cell aggregates without per-tree records, broader
optical/temporal acceptance. See the integration gate; do not claim full Nanite.

## 2026-11-01 SpaceRacer native 4K profiling and reflections

City reflection BVH uses surface-area splits + threaded escape indices (aux
node word 3 is now subtree escape, not right child). No stack or 256-node cutoff.
Matched native4K GPU median 8.59 -> 6.18 ms, repeat 8.56 -> 6.14; reference image
pixel-identical. Final release tested five biomes 1/2p, city seeds 35/80385 4p.
Four-player city GPU ~9 ms: no locked120 claim. Windows media service/daemon and
other apps consume many CPU cores; left running. No interactive game launched.
User explicitly said NOT to run a playtest after this work.

Shared app::profile::bench separates CPU phases, GPU scopes and completion wait,
excludes warmup/readback and checks native reconstruction. profile-report.py
provides offline graphs; SpaceRacer profile-native.ps1 runs a serial matrix.
build-native.ps1 now defaults release; -Profile dev remains available. Tests:
5 stage tests, 23 app tests, all headless pipelines/captures. See
engine/docs/spaceracer-native4k-2026-11-01.md and native-profiling.md; raw results
at build/spaceracer-perf-{native4k,final4k,final-biomes4k}. Portable release ready.

## 2026-11-01 SpaceRacer adopts current Growing Guns smoke

Latest shared baked-smoke implementation already present after origin fetch;
SpaceRacer now opts in via effects.luau baked_explosion_smoke. Its common Effects
instance covers crashes and non-EMP weapon explosions. No copied asset/shader.
Five baked-smoke tests + explosion-smoke.json pass; native 1080p city/forest/two
views reviewed at build/spaceracer-smoke-{city,forest,split}. No perf target claim.

## 2026-11-01 SpaceRacer blue root / dim tail

Forward jet emission blue (.12/.4/1), bright near nozzle then exponential falloff;
7.2 m max authored plume before surge. AgX handles near-white highlights. Blue
engine light, wisps .12x, wake .065x/.24 s. Tunables jets.luau, shape jets.wgsl.
Eight effect tests + city/forest 1080p onset/sustain captures pass, at
build/spaceracer-blue-core-*. Handling and global tone mapper unchanged.

## 2026-11-01 SpaceRacer compact glow / source occlusion

Sprite.depth_test defaults Fragment; Source checks centre depth for optical
halos (misc.y now additive/source bit flags, GPU layout unchanged). Nozzle,
glare and flare opt in; road contacts remain Fragment. Smaller engine halos
and contact pools; distant minimum 14/~20 px. Eight effects tests and normal/
minimal GPU source-occlusion regression pass. 1080p city/forest captures:
build/spaceracer-bloom-tight-*. See game parity review and rendering.md.

## 2026-11-01 SpaceRacer grounded lightning and engine halo

Game jets use varied eight-segment arches to supported deck contacts, max 48
segments/ship; onset decays, no bolt history. Boost plume width/radiance and
local light increased. Effect 13 adds source-depth-gated Screen-facing halo,
24 px minimum / ~35 on boost, far fade 650-1200 m. Tunables in jets.luau.
Eight effect tests + 1080p thirteen-tick city/forest sequences and sustain pass;
see boost-arcs-ground.json, game parity review and build/spaceracer-arcs-*-final.

## 2026-11-01 SpaceRacer varied AI and finish racing

Luau personality/plan/boost_plan give six seeded driving styles, smooth lateral
plans, optional pickup pursuit and independent boost clocks/cooldowns. Lua must
check optional pickup type: serde null is truthy userdata, not nil. Finished
cars run race-speed/boost choreography with predictive braking, bank/fins and
world motion blur; finish times/results and mounted hardware survive. Normal AI
is skipped for finished drivers. 41 simulation tests pass; 1080p finish-racing
scenario captures at build/spaceracer-finish-racing. See game parity review.

## 2026-11-01 SpaceRacer menu selections

Local World/Difficulty/Seed default to Random. Start new race resolves only
Random values; fixed choices and player count survive. Resume never rerolls.
Initial menu hides Resume; seed recipe label updates immediately. Selection
policy/navigation in game.luau, native entropy only at new-race creation.
Focused tests + 1080p menu-settings.json capture flow; see game parity review.

## 2026-11-01 SpaceRacer level shadow cache and sharp arcs

Scene.stage_sun opts into a 4096-square/64 MiB shared static source-alpha map;
Slots::reserve registers dynamic ownership. Static solid geometry now included,
full-scene fit, fixed bake LOD, receiver-plane precision/mesh-error correction.
Graphics.sharp_sprites moves world motion blur before sprite/exhaust/particle
composition; target COPY_SRC is required. Bloom remains after emission.
See SpaceRacer parity-review.md for captures, checks and limits. Near-detail
transitions remain; distant wind is rest pose; this is not GI or an FPS claim.

## 2026-11-01 SpaceRacer flight/combat feedback

Finite road/tunnel/guard sweeps replace nearest-section crash inference; shared
ribbon geometry and oriented scenery proxies. F12 seed 46990/hard/cell covered
by sidewall regression; actual previous input trajectory was not recorded.
EMP now 100 m, fading ship arcs, local missile cancellation. Pickup spacing
1300 m. Sentry newest-only interpolated tracer with surface sparks; boost arc
onset/decay and contact glows; stronger world/per-view warp refraction.
37 sim + 24 app tests; native capture scenario `flight-combat-effects.json`.
No Android export or performance certification. See SpaceRacer parity review.

## 2026-11-01 SpaceRacer split-road F12 correction

Seed 89502 hard forest at 4854 m: coarse midpoint cell omission made sawtooth
holes; inner split guards were missing despite simulation constraint. Restored
continuous fork strips, matching inner walls/guards/neon, surviving-branch boosts
and exact launch lip boundary. No track recipe/physics changes. Two geometry
regressions and native 1080p entry/wide/exit captures; see parity-review.md.

## 2026-11-01 SpaceRacer respawn and energy wear

Restored source-inspired smooth camera return and bottom-up hull construction,
plus retained energy-linked paint wear/scorch and low-energy smoke. Game-owned
WGSL visibility mask is injected via `render::set_game_mask` before construction;
all default games retain an always-false mask. `Renderer::update_aux_words` writes
a checked pre-reserved range. SpaceRacer uses header word 5 for six per-pilot
records (pose, energy, scan progress), uploading only 580 bytes/frame. World
fragments outside the contiguous ship slots return before the six-record lookup.
No per-frame geometry allocation. 32 sim + 20 app + 1 renderer test; native build
and staged solo/split 1080p captures. See games/spaceracer/parity-review.md.

# SpaceRacer weapon restoration - 31 October 2026

Restored source warp ribbon autopilot/phase/gap/EMP handling, 20-Hz mounted rotary
sentry and powered predictive glide bomb. Luau combat/warp policies, native sweeps
and bounded per-frame presentation. Source-shaped weapon mounts have separate
head/barrel/rotor/wing joints. Frame-consumed shot IDs prevent persistent tracers.
Generic exhaust compositor adds depth-aware world lenses and explicit per-view
radial/chromatic lenses (kinds 3/4); opt-in, before HUD, no effect on other games.
Details/tests/capture limits: games/spaceracer/parity-review.md.

# Handoff: Warp engine (Sideways)

For the next agent. Read this, `README.md`, `../games/sideways/README.md`
and `../AGENTS.md`. Cargo workspace and lockfile are at the repository root.

Repo: https://github.com/ontola/warp-game-engine: the engine
in `engine/`, the native game in `games/sideways/`. Godot code and its oracle
comparison are removed; older sections below remain historical evidence.

## SpaceRacer source restoration - 31 October 2026

Restored retained Open Sans SemiBold/logo, source-proportioned HUD and actual visor glass
(curvature/glow/ghost), segmented energy/minimap/buttons/results. Generic HUD
`Optics` is opt-in (four bounded rectangles), offscreen at native resolution;
menus are flat. New `hud_optics.rs`/WGSL, serial renderer regression.
Cruise now follows banked track geometry, launches from the rack, accelerates,
retargets until terminal lock, hits eligible leader after earlier finisher,
fuses targetless shots and responds to EMP. Modern source loft mesh, corrected
winding, interpolated pose, bright engine and continuous smoke ribbon.
Victory is independent of combat/normal AI, preserves items, continues after
results, animates fins and arcs over gaps; four cinematic shots, relative entry
blend and fitted split FOV. Policy is Luau (`missile`, `victory`).
See games/spaceracer/parity-review.md: explicit outstanding weapon, EMP, damage,
menu, warning, content/audio/platform gaps. Native portable build refreshed;
no Godot/Android build or new physical-controller run. Tests: sim 25, app 16,
HUD 13. Only owned SpaceRacer/render files belong to this change.

## SpaceRacer city playtest follow-up - 31 October 2026

4K playtest found a city light radius of 8 m violating renderer's 1 m cap:
fixed, with 2,400-tick interpolated light regression. UI scales from 900 logical
pixels and text is outlined. Billboard/window reflection lookup now uses a
near-first balanced BVH instead of discontinuous receiver-cell top-six lists.
Bounded analytic proxies, not full scene RT. Stronger Luau-tuned motion blur,
confirmed on consecutive 4K boost captures. `scenarios/city-playtest.json` and
`build/spaceracer-city-playtest` hold the flow/captures; visual-review.md has
limits and one 1080p GPU-pass sample. Fourteen app tests including hierarchy
coverage and all-light validation. No Android build.

## SpaceRacer native GameNight integration - 31 October 2026

Shared client adds read-only `State::player(id)` for renderer profile access.
SpaceRacer caches one avatar/skin atlas and projects it onto original nacelle
paint; live clothing colour repaints hull/debris. Settings policy/schema lives
in `games/spaceracer/rules/party.luau`: ten keys, next-race/live timing, all native
biomes. Preparing batches are applied before Ready; profiles stay live while
paused. Fixed rosters use the host Prepare boundary for seat/player changes.

12 app + 16 SDK tests and nine isolated installed Windows Preview-daemon checks
pass, including real authenticated launch, two/four views, profiles, settings,
Undo, pause/resume, seat re-prepare and SDK6 new-race action. Reproduce with
`games/spaceracer/tools/test-gamenight.py`; screenshots/report are under
`build/spaceracer-gamenight-final`. This does not test physical pads or desktop
foreground transitions. `register-local.ps1` points only local SpaceRacer at
`build/spaceracer-native`; restart GameNight to reload. No catalog release or
Android build was produced. Public canonical integration docs were regenerated.

## SpaceRacer motion blur and crash debris - 31 October 2026

Native effects complete for this iteration: per-view HDR motion-vector blur
and source hull fragments with soot, gravity/bounces and small fire/smoke.
Artistic policy in `games/spaceracer/rules/effects.luau`; bounded mechanics and
GPU work native. Shared FX smoother billows with opt-in continuous smoke.
See `games/spaceracer/visual-review.md` for captures, tests and 1/4-view timings;
`engine/docs/motion-blur.md` for the optional renderer API and limitations.
Portable Windows build refreshed. No interactive, Android or hardware test.

## SpaceRacer visual corrections — 31 October 2026

Native visual revision: see `games/spaceracer/visual-review.md`. Game-owned
`app/shaders/surfaces.wgsl` restores opaque ship livery, road detail, varied
metre-sized city glazing and biome materials. Correct 3x2 poster atlas/orientation,
soft additive exhaust with calibrated interpolated local lights, thin foliage,
physical daytime sky, facade/track structure. Gameplay/asset bytes preserved.
`--bench N --out PATH` records a repeatable headless report; not interactive FPS.
20 focused tests and portable scripted controller flow pass; all five biomes
rendered. Full visual fidelity and scene reflections are still incomplete.

## SpaceRacer native/Luau development build — 31 October 2026

SpaceRacer now has `games/spaceracer/app` (`spaceracer-app`, binary `spaceracer`).
Pulled main through ae0b1c8 before this work. Read its README and port-status.json:
it is playable, **not a visual/content-parity replacement**. Existing GameNight
catalog and releases remain unchanged. No Godot runtime or export tooling used.

`games/spaceracer/rules/game.luau` owns track recipes/design ranges, AI decisions,
rank-weighted items, weapon activation, balance, menu and biome parameters.
Rust retains geometry, handling/flight, queries, projectile integration,
bookkeeping and rendering. Rules hot reload; geometry applies next race.
Analog triggers were added to `app::Controls`, and seat merging now preserves
the right stick. Shared GameNight adapters carry trigger pressures too.

Validation: workspace all-target check; 18 sim + 2 native app + 21 app tests;
1207-tick native two-controller flow with captures; five biome renders; a
1920×1080 four-view capture. Mock-host lifecycle uses a real loopback WebSocket,
sparse seat IDs and opaque controller tokens. No real controller/host, mobile
or full-content performance acceptance yet. All assets/provenance retained.

Windows: `games/spaceracer/tools/build-native.ps1` builds/packages only;
`build/spaceracer-native/spaceracer.exe` resolves adjacent rules/assets.
Local test cache used `C:/dev/warp-spaceracer-target`; captures/logs are in
`build/spaceracer-review`. Final captures expose the provisional scenery and
lighting: preserve this honesty, do not advertise it as finished or switch
GameNight's released game without completing the remaining port.

## Main integration and next-agent entry point - 29 October 2026

The recovered vegetation work is integrated with current main, including its
newer terrain and custom game-surface changes. Start with
[the runnable vegetation hand-off](docs/vegetation-next-agent.md). It includes
fixture commands, acceptance limits, architecture gaps and portable evidence.
Older notes below about the missing original Git directory describe the
pre-recovery situation; the original checkout remains a backup, not the main
checkout to use. Do not reset or repair that old directory to continue work.

Integration validation: 682 render tests passed (16 ignored), 189 Sideways
tests passed (one ignored), the explicit GPU visibility/sort/boundary oracle
passed, and the developer launcher contract checks passed. A fresh 1920x1080
one-tree 1000 m aggregate smoke capture completed; this is a correctness smoke
check, not a performance benchmark. The workspace all-targets check passed again
after merging upstream through `cb005a6` (terrain-fly and wire-entity changes).
The only additional integration fix updates a stale contact-AO shader assertion
to cover both diffuse-sky lighting paths; rendering behavior is unchanged.

## Parallel brick visibility implemented - 29 October 2026

See [architecture, validation and actual captures](docs/vegetation-brick-visibility.md).
`KERN_VOXEL_BRICK_VISIBILITY=1` enables shared disjoint support bounds, existing
GPU poses, parallel visibility construction, bounded per-pixel sorted intervals
and front-to-back integration. It automatically enables the global stream.
The live forest/default remains unchanged. No new species geometry or fake bones.

The final 64-tree diagnostic (`aggregate-lod/brick-atomic-64`) measures 0.642 ms
visibility plus 20.767 ms integration, versus 2.969 + 82.353 ms in the matched
branch baseline. Covered interval 49.206 versus 112.116 ms. Eight samples each,
uncontrolled competing GPU work: not game FPS. Camera/shadow triangle counts
match; exported opacity IoU 0.999627. Four views and source/LOD captures pass
without stream/step overflow. The rejected CAS allocator (`brick-parallel-64`)
measured 188.559 ms visibility; do not use it as the final result.

675 render tests pass, 16 ignored; the new GPU sort/boundary/concurrent-overflow
oracle passed separately. 189 Sideways tests, workspace all-target check and
launcher contracts pass. New tests and evidence do not certify DLSS motion or
source LOD appearance. The prior +12.37% far source coverage bias remains.
Integration/material lighting and triangle shadow maps remain expensive; this
is not full Nanite voxel raster/GBuffer shading or voxel shadow casting.
Keep the existing 4,096 selection limit and bounded complete fallbacks explicit.
The original common Git directory is still missing; no commit/merge to main.

## Sideways r4 source and opt-in aggregate LOD — 29 October 2026

The user rejected the coarse v1 species gallery. The denser r4 source is now
the approved baseline to preserve, particularly the spruce; this does not
approve distant LOD, transitions or a live forest. V2 uses shared detailed
wood/foliage sprays. See [architecture, evidence and limits](docs/vegetation-species.md).

**Do not describe the current consumer as Nanite voxel rasterization.** Warp
ray-integrates branch volumes and evaluates BRDFs at occupied samples. Epic's
documented route separately bins compact voxel clusters, uses front-to-back
depth buckets and samples a normal distribution into the GBuffer before
shading. Warp's tile bins and `KERN_VOXEL_GLOBAL_INTERVALS=1` worklist remain
integrator optimizations. For each new issue or fix, compare the actual Warp
mechanism with [Epic Nanite Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)
and state the gap; matching 128×4³ storage is not renderer equivalence.

`cook_source`/`finish_source` retain unique geometry and compact placements;
`tree_species_compare` pairs that source with the textured reference at common
scale, and `plant_spray_gallery` exposes its components.

`cook_aggregate`/`finish_aggregate` now add branch/subtree candidates with at most
128 bricks of 4³ cells per level and GPU per-view selection. Directional opacity
and material errors compare against a finer source-derived medium: sampled
optical-model evidence, not exact raster visibility. Per-volume tile bins retain
deterministic candidate order, exact ray interval sorting and complete overflow
fallbacks. A chosen aggregate mip is sampled exactly; the legacy world consumer
keeps its footprint/ceiling behavior without a new selection ABI. Finer source
cuts now execute in the fixture but lack optical/temporal acceptance. Source shadows still
rasterize triangles; forest residency, DLSS transitions and optical parity remain
open. This experiment is not live Sideways. The rejected flattened `mixed`
route stays explicitly disabled; mixed grass is a separate opt-in experiment.

Dynamic pose buffers and all-camera/dynamic-shadow GPU motion scheduling now
dispatch active attachment pages indirectly. 64 poses per assembly remains;
legacy admission defaults can be raised explicitly. Runtime ceilings are
262,144 poses, 1,048,576 attachments and 512 MiB internal animation buffers,
also bounded by device limits. These are not measured forest capacities.
Source-instance/attachment residency still scales with placements.

V2 r4 evidence: `C:/dev/perf/warp-sideways-20261028/species-v2` contains all six
reference pairs, component closeups and the far source-vs-CLOD oracle. The
spruce places 32.1M source triangles from 1.312 MiB shared geometry excluding
instances/poses; its 975 MiB expanded counterfactual is not allocated memory.
Normal 1 px far CLOD loses much of the crown that full source retains. Native
grain and distant optics remain unresolved despite source approval. Earlier
render/CLI tests passed 649/189 before r4; focused r4 recipe/catalogue tests and
workspace check passed. Those historical tests do not certify aggregate edits.

`C:/dev/perf/warp-sideways-20261028/aggregate-lod/smoke-2/report.json` records the
completed single-tree source/auto fixture: native 1920×1080, one view, TAA off,
two warmups/two measured frames per case, RTX 5070 Ti/Vulkan. At 50/150/400 m all
61 groups remain full source and RGB/native opacity match exactly. At 1,000 m
auto selects 60 mip-0 branch volumes and submits 458 colour triangles including
ground versus 32,099,798 source triangles. Both shadow maps retain 14,619,068
source triangles. Coverage becomes 79.266 versus 68.000 pixel-equivalents,
fuzzy IoU 0.630; this transition is not accepted. Unrelated background GPU work
and the tiny sample window prevent a performance conclusion. Offscreen is not
isolated: separate idle observations still showed roughly 40% competing GPU
activity. No speedup,
four-view result or DLSS proof. This run predates ongoing source-cut refinements.

Later reports under the same evidence root supersede the initial selection
experiment without changing approved r4 source shapes:

- `fine-global-1/report.json`: 0.025 m reference grid, refinements and global
  intervals. The 0.08 optical gate keeps full source at every tested distance,
  including 1,000 m. Preparation is 139.47 s, packed volumes 23,388,756 bytes.
  Configuration rejected. Source/auto TAA phases differed; image and opacity
  metrics from this run are invalid as LOD-quality evidence. Capture sampling
  was subsequently corrected using `reset_capture_sampling()`.
- `global-reference-1/report.json`: corrected capture phase, 0.075 m grid,
  refinements/global stream, TAA with 32 frozen-pose capture frames. At 50 m,
  source cuts reduce camera triangles from 32,099,798 to 22,260,566; raw coverage
  falls 3.44%. At 1,000 m, 60 volumes leave 458 camera triangles, raw coverage
  exceeds source by 12.37%, and fuzzy IoU is 0.656. Shadow maps retain 14,619,068
  source triangles. No volume overflow is reported. Eight contended samples
  show 14.56 ms integration alone versus 12.90 ms full-source covered interval
  (auto covered interval 20.27 ms). This is no isolated speed estimate or
  performance win; the global stream is not promoted.

The [source/auto review image](C:/dev/perf/warp-sideways-20261028/aggregate-lod/global-reference-1/source-auto-review.png)
shows the corrected reference comparison. Subsequent diagnostic evidence:

- `global-cheap-brdf-1` / `global-density-1`: 13.44/15.25 ms integration versus
  14.56 ms full shading; eight contended samples each. Opacity PNG SHA-256 and
  all traversal counters are identical. No material/lighting win demonstrated.
- `four-tree-four-view-1`: four trees, four 960×540 views in one 1920×1080 output,
  TAA off, four samples. Camera triangles fall 513,596,744→7,304; shadow triangles
  remain 226,596,066. Covered interval source 138.40 ms, auto 145.15 ms; auto
  integration 101.30 ms. No ready four-player result follows.
- `forest-64-guard-1`: 64 trees, one native 1080p view, TAA off, two samples;
  auto completes with 3,840 volume groups, 64 source groups, 29,186 camera and
  177,593,316 shadow triangles. Integration 98.73 ms, covered interval 135.14 ms.
  Twenty-five tile-list overflows use complete fallback; step/pool overflow zero.
  5,088 rays perform 4,747,443 volume samples and 42,401,827 AABB tests.

All these timings have uncontrolled competing GPU work. The fixture deliberately
pins both near and main shadow maps to the forest, unlike camera-relative
gameplay: report source-shadow stress counts, not game FPS. The current feature
is far from ready. `taa-null-2` uses `--error 0.01` to force finest source in
both modes: native opacity matches exactly (IoU 1, L1 0, coverage 31,649).
RGB differs in 24/2,073,600 pixels, MAE 0.00000916 on the 0–255 scale. The
capture-phase fix has opacity parity, not bit-exact RGB or temporal-motion proof.

Latest checks: `render-tests-5.log` has 672 pass/15 ignored;
`selector-gpu-2.log` separately passes the GPU hierarchy/exact-cut/scale oracle;
`cli-tests.log` has 189 pass/one ignored. Workspace all-target checking and
developer-launcher contracts pass. These checks do not accept optical quality,
forest scale or moving TAA/DLSS.

The next milestone is compact spatial voxel clusters and separate visibility/
material resolve followed by shading, compared directly with Epic's documented
boundary and the current source oracle. Do not spend the next iteration polishing
branch ray integration or changing the approved source to hide its cost.

`vegetation_forest_lod` excludes warmup and image/debug readbacks from async
GPU timestamp windows. Begin with one source/auto tree before longer isolated
runs. Four full-source trees are about 128M triangles per camera before light
passes. The completed 64-tree guard case is not a universal capacity guarantee:
preflight both consumers against 4,194,304 visible clusters/pass. Do not silently coarsen the
source oracle, disable shadows or report truncated work as a gain. Four-view
sun focus now has one anchor per view. Shared-helper and timestamp regressions
are covered by the passing latest suite; do not reuse an earlier failing log.
Recheck GPU process state before measuring; do not assume other workloads stopped.

Working source is `C:/dev/warp-sideways-plant-species-20261028`. Its original
common Git directory `C:/dev/warp-game-engine/.git` disappeared during this task;
files remain, but metadata must be recovered before merging. Do not reset or
recreate history blindly. No commit/push/merge was made here.

### Rescue snapshot before further source work

The final rescue snapshot is the root agent's responsibility; this instruction
does not claim it exists. Use a fresh directory outside the worktree. Copy the
source tree, workspace manifests/lockfile, shaders and docs, including
`plant_species.rs`, `plant_recipe.rs`, `plant_spray.rs`, assembly/cook/animation/
selector code and preview fixtures. Preserve the r4 gallery and all aggregate reports
with their binaries/manifests where available. Include required ignored assets
explicitly; source alone or the broken `.git` pointer cannot reconstruct them.

Write a relative-path/byte-count/SHA-256 manifest and verify copied files before
repairing Git metadata or editing approved source. Record the original worktree,
capture commands, known build/test state and missing Git directory. Do not
overwrite older snapshots or fabricate a revision. Append the actual snapshot
destination and verification result only after completion; recovery/merge is
separate work.

## Growing Guns feedback fixes — 28 October 2026

See [the feedback checklist](../games/growing-guns/docs/playtest-20261028.md).
Quarried stone prototype, stable card hover, spawn cages, live stadium screens,
human ghost recovery above lava, and debris collision re-upload are implemented.
The stone surface still needs less regular fracture relief; this is explicitly
unfinished visual refinement, not an accepted final asset or a measured speedup.

**User requested no new interactive playtest: Claude handles the next launch.**
Only offscreen captures and focused tests ran after that request. Build from
current main (`cargo build --locked -p growing-guns-app`); the old local
`gg-play.ps1` switches to an obsolete slice branch. GG network ALPN is now `/2`
because cage state was added to the positional snapshot format; update both
peers. The reusable GPU screen feed appears in profiler output as `live screens`.

## Integration into current main — 28 October 2026

The grass agent's source is preserved in `ac29b0f` and integrated with current
renderer features and Growing Guns CPU work. Read
[integration notes](docs/grass-integration-20261028.md) for binding/view-index
repairs, checks and limits. The r64 playtest below is historical and stopped.
Optional contact/filter controls remain off; distant source geometry is still
expensive and the proposed far representation remains unfinished.

## Previous grass status — r64 opt-in distant shading and edge contact

28 October 2026. Read [r64 evidence and remaining checks](docs/meadow-edge-grain-r64.md).
`KERN_MEADOW_SPATIAL_FILTER=1` adds a native-only depth-aware 3×3 RGB filter
at source composite. Alpha is exact; near pixels and empty coverage stay
unchanged. Onset uses the coarse triangle's mean width W/2 times live growth;
TAA/DLSS, motion and aggregate disable the filter. Current source wind and
lighting remain live. This does not fix silhouette aliasing or reduce the
250 m source geometry cost, and is not the proposed distant terrain layer.

`KERN_MEADOW_EDGE_SHADOW=1` builds a source-accepted 151² root grid per view,
1,459,520 bytes shared across four views. The final receiver shares 25 records
between sky/sun queries, radius 0.4941 m for this source (bounded to 0.55 m),
sun reach 0.35 m and fade 24–32 m. It approximates basal contact on terrain/road,
not wind-animated blade silhouettes. Both flags remain opt-in.

First-probe OFF is pixel-exact with r63e in three native 4K captures. The W/2
filter-only exploratory distant crop has 76.4% less high-pass display-luma
variance but 9.0% higher mean display luma; this is static contrast evidence,
not final appearance or temporal acceptance. Workspace/CLI/launcher checks,
556 renderer tests (eight ignored) and the 2312-pixel filter GPU oracle passed.
The contact producer oracle also passed its final rerun after the receiver
radius change. Final combined static captures and six
selected wind poses completed. One-view native 4K covered GPU medians are
23.311/26.326/25.410 ms for edge-noon/hillside/edge-morning, 72/72/69 samples;
contact remains subtle and road controls are unchanged. No final appearance
or temporal acceptance. Frozen `sideways-r64-native.exe` SHA256:
`afc89e31a0e16af50dd2a62450aa7371ff914264310a63f272b653b082a607d0`.

The final native 4K, one-view preview reached a confirmed visible first frame
at 16:56 UTC, PID 23356, using the frozen final executable and both flags.
Wind strength is 0.8 with advancing time. The new launcher
`-DebugUiScript games/sideways/scenarios/windswept-grass-edge-settings.json`
requests size 1.6/density 1.0, as verified in the static scenarios. No live F12
was captured to independently verify those controls; the saved 2.0/.55
baseline is unchanged. The active executable's path and hash match the manifest.
Manifest: `../games/sideways/build/dev-runs/20261028T165617934Z-5eab5a82/iteration.json`.
**Do not run GPU fixtures while the playtest is active.** Recheck process state
before later work; earlier handoff statements below are historical. Evidence:
`C:/dev/perf/warp-sideways-20261028/r64-grass-edge-grain/`.

## Previous status — r63d validation and interim r63e source range

28 October 2026. Read [the r63 report](docs/meadow-windswept-r63.md).
The GPU meadow now defaults to `SIDEWAYS_MEADOW_PROFILE=windswept`; choose
`classic` explicitly for the previous recipe and r62 shading experiment.
Pinned Quick_Grass equations supply curved blades, fourth-power tip colour
and travelling noise wind. r63d evaluates pigment/AO per fragment and morphs
six segments onto the actual one-triangle cut over growth-scaled 3–6 m. The
cooked three-segment cut is not selected. Aggregate remains forced off for
windswept; group cache is invalid. The first aggregate-based live attempt was
prohibitively expensive and remains diagnostic evidence.

Validated r63d, one-view native 3840×2160 on RTX 5070 Ti: normal/close/overview
fixed cameras measure median covered GPU 23.310/14.321/20.514 ms, with meadow
6.511/2.620/3.527 ms; 72/72/69 samples after warmup. The interval excludes CPU
and work outside the instrumented endpoints, including HUD/presentation.
547 renderer tests (five ignored) and four explicit GPU oracles passed. The
61-frame approach sweep also completed. The user's r63d freeplay closed normally;
**no game is active at this handoff**. Earlier PID 48036 is historical.

The user rejected the 100 m disappearance. Current r63e code defaults to a
250 m source radius with a 200–250 m fade; the explicit
`SIDEWAYS_MEADOW_WINDSWEPT_RADIUS` control accepts 60–256 m and ignores the
classic radius/band flags. This is an interim source comparison,
not the accepted distant representation. The completed 100/250 m hillside
comparison fills the bare hill but costs more; its timings precede compaction.
The subsequent homogeneous windswept profile-2 pool uses 75,129,660 bytes/view
at 250 m instead of 155,267,964; classic/mixed profiles retain 28 plant words.
550 renderer tests (six ignored), 190 CLI tests (one ignored) and the explicit
compact-pool GPU sentinel and developer-launcher contracts passed. The release
build and final matched native-4K hillside capture completed successfully;
see the r63 report for the compact run's measurements. Frozen executable:
`sideways-r63e-native.exe`, SHA256
`088132BAD7274098EA4F9D5565DA60948E43869204AC9212AAFA098045336248`.

The user prefers filtered distant coverage over individual far blades. The
proposed representation is a source-derived, directionally prefiltered thin
layer following the terrain, with shared wind and current lighting. It is
not implemented; do not re-enable the old photo field or per-patch atlas as a
substitute. No four-player/120 FPS, final visual or temporal-quality claim. Evidence:
`C:/dev/perf/warp-sideways-20261028/r63-windswept-grass/`.

## Previous status — r62 rounded grass shading, opt-in source experiment

28 October 2026, branch `codex/grass-rounded-shading-20261028`. Read
[the r62 report](docs/meadow-rounded-shading-r62.md). The supplied grass
tutorial prompted smooth centreline/rounded transverse shading normals for
every meadow source LOD and its rebuilt atlas. Select explicitly through
`SIDEWAYS_MEADOW_BLADE_SHADING=rounded`; unset/flat retains the existing source.
Geometry, placement, material, wind, saved baseline and registration are unchanged.
The launcher records the mode; F12 replay treats missing historical mode as flat.

Workspace check, 535 renderer tests (one ignored), CLI/replay tests and launcher
contracts passed. Eight paired native1280×720 GPU cases retain exact source
alpha and selected draw counts, exercising all source cuts and the aggregate.
Two native1920×1080 Sideways camera views also completed A/B capture. Close
blade shading is softer but the improvement is subtle and geometric silhouettes
remain angular. Motion/DLSS/all-weather quality and performance are unverified;
aggregate geometric shadow bias still uses its approximate shading normal.

Evidence and frozen native executables:
`C:/dev/perf/warp-sideways-20261028/r62-rounded-grass/`.
Both bounded scene runs exited. No interactive playtest was started or left
running. GameNight still uses its earlier registered package and profile.

## Previous status — r61 frame hitch fix, opt-in GameNight preview

Current branch: `codex/fourview-budget-20261026`. Read
[the r61 report](docs/frame-hitches-r61.md). Foliage shadow page updates were
forcing counter readback and `gpu.wait()` even during ordinary gameplay.
`KERN_ASYNC_SHADOW_VALIDATION=1` now copies counters to a bounded asynchronous
queue, retains overflow/error checks and defers page updates on saturation.
Engine default remains off. No shader, shadow quality, LOD or physics changes.
There is no dedicated validation drain at renderer destruction; explicit
captures drain earlier batches, as all six measured runs did.

Six alternating A/B runs of the SAME r61 executable: one view, 4K output,
DLSS Performance (1920x1080 internal), fixed-camera then deterministic drive.
The two repeatable shadow-update intervals fall from median 59.234/56.918 ms
to 15.489/14.386 ms. All six baseline page-update spikes disappear. Total
intervals above 33.33 ms fall from 12 to 3 across 10,068 intervals per variant.
GPU covered median stays approximately 15.75/15.83 ms; this fixes a wait,
not average rendering cost. Other spikes remain unexplained. No four-player
or 8.33 ms claim. JPEG A/B differences are comparable to repeat-run A/A;
sampled visual review found no missing shadows, not raw/motion equivalence.

New `engine/tools/frame-hitches.py` reports cadence tails and preceding CPU
work with real-frame-ID GPU joins. 27 Python fixture tests, 12 focused render
tests and two transport tests passed; DLSS release built and six visible
3,600-frame runs completed. GameNight transport failures now report structural
causes and timing without tokens/payloads; a prior disconnect is not yet
explained or claimed fixed.

Package `games/sideways/build/gamenight-r61/` and frozen test binary SHA256:
`3ae3e74087829185873f13862a1d526aa3ec90d24ab0cecf52061bacb31f8a3c`.
Local registration enables the async preview while retaining GPU meadow,
120 blades, 25–35 m band, tree fullness 1.25, AO off and the r60 grip change.
At 2026-10-27 13:40 UTC GameNight session
`c49511ad-d234-416d-8ada-015df3d2ec40` was running this exact executable;
PID 201308, seat 0 occupied, controller `ordinal:1`. The game logged a visible
3840x2160 scene frame. No physical controller-input/scanout proof. Do not run
GPU fixtures while this playtest is active. Evidence and managed launch proof:
`C:/dev/perf/warp-sideways-20261027/r61-hitch-investigation/`.

GameNight can overwrite an explicit local launch with a cloud CI preview;
that previously selected an old Godot executable. Process-scoped
`GAMENIGHT_OFFLINE=1` was used when starting the host for this investigation;
check live executable paths after any later host restart. Do not change or
delete old cached packages as a workaround. The host priority issue is not
fixed here. Use `active_session`/`warm_session` in live party state.

## Previous status — r60 handling trial in GameNight

The user requested more grip after feedback that the car feels like a boat.
Read [the r60 report](../games/sideways/docs/handling-grip-r60.md). Game-owned
car data now uses tyre_grip_scale=1.12 and lateral_peak_scale=0.80. Physics
remains 120 Hz/four substeps; steering, inertia and relaxation are unchanged.
Matched 60 km/h flat-gravel impulse tests recover 50 ms earlier with about
10% less sideways travel. Braking improves about 11%; subjective acceptance,
powered slides and rough/wet-road validation remain open. 43 sim library and
two transient tests passed, two existing diagnostics ignored. Grip-dependent
bench limits were explicitly adjusted and documented; this changes handling.

DLSS release built successfully. Package `games/sideways/build/gamenight-r60/`
contains the updated executable AND cars.toml; SHA256:
`672d2380424687c1342e3857d3dcf80ecccfb06a0358426b554aa148f1ce1099`.
The managed registration retains the complete procedural-grass preview profile
from r58, with AO off. Runtime confirms the GPU meadow, no photographed grass,
and a visible 3840x2160 scene submission. GameNight session
`92154014-f51e-42e5-a621-6a8f5bf2fcc3` was confirmed running with two occupied
seats and the prior controller ordinal bindings. This is not physical-input or
display-scanout verification. Do not run GPU benchmarks during the playtest.
Live host state uses `active_session`/`warm_session`, NOT `active`/`warm`.
Registration is loaded on host launch; avoid redundant next/relaunch commands.

## Previous status — managed GameNight preview profile repaired

The user requested a GameNight session. The first r59 registration pointed to
the correct new executable but omitted the developer graphics environment.
Actual logs confirmed legacy photographed grass, tree fullness 1.0 and the
old sky selection. This was a launch-profile regression, not a source rollback.
The registration now copies the complete visible r58 playtest profile from
`games/sideways/build/dev-runs/20261027T100418779Z-44907e32/iteration.json`, with
packaged game/IR/DLSS paths. r59 executable hash remains unchanged.

Managed runtime now confirms GPU meadow with 120 opaque ribbon blades/patch,
25–35 m transition, no photographed grass/CPU tuft pool, tree fullness 1.25,
and a visible 3840×2160 scene frame. New registration helper and isolated
tests preserve the profile, unrelated games, backups and disabled experiments.
Tests pass on PowerShell 7 and Windows PowerShell 5.1. GameNight's controller
seats and settings remain host-owned; check live party state before transitions.

Local package: `games/sideways/build/gamenight-r59/`; registration proof is
`registration-proof.json` beside it. The user has resumed playtesting. Do not
run GPU benchmarks concurrently or restart merely to gather proof.

## Previous status — r59 grass contact AO, opt-in only

Read [r59](docs/meadow-contact-ao-r59.md). Runtime `e8634da` adds default-off
`KERN_MEADOW_CONTACT_AO=1`: four bounded canonical-root footprints attenuate
terrain diffuse skylight only, with no new pass or texture lookup. Direct
sun/headlights/specular remain unchanged. Tests: 528 render + 185 CLI passed,
one ignored each; launcher contracts passed. Frozen r59 binary and SHA256
are in the report.

Six OFF/ON runs, same executable, four-view 4K-output/DLSS Performance:
GPU 28.007/27.997 ms first, 34.110/34.203 ms continued. Opaque shading
4.092/4.085 and 3.920/3.936 ms. No large cost increase resolved above run
variation; not proof of zero cost or speedup. Pruning was OFF in both modes,
so do not combine r58 and r59 deltas as a measured package result.
Separate PNG QA completed at frames 60/240/480, all four views. Contact is
subtle in daylight, not a directional grass-shadow replacement. Motion and
all-weather acceptance remain open. No default promotion.

The user stopped the interactive playtest before leaving. It is closed and
both QA runs have ended. Do not restart an interactive session unless asked.
Evidence: `C:/dev/perf/warp-sideways-20261027/r59-meadow-contact-ao/`.

## Previous status — r58 full-resolution sample pruning measured, opt-in only

Read [the r58 report](docs/meadow-sampling-r58.md). Runtime `d5fea46` adds
default-off `KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE=1` and explicit raw aggregate
captures. Six OFF/ON runs: aggregate 4.169→3.307 / 6.729→5.694 ms; all grass
7.284→6.502 / 9.398→8.452 ms; total 27.645→27.005 / 33.734→32.976 ms. Same
binary, four-view 4K-output/DLSS Performance, 25–35 m band, full-resolution
aggregate, 179 clean frames/section/run. No default or target change.

Raw A/A/B checks cover 12 view/frame cases: all depth/motion bits match, zero
coverage mask differences, finite layers. Color/OIT differences also occur A/A;
their similar scale does not prove exact optical parity. All-view still review
found no new blur or silhouette issue. Motion/all-weather acceptance remains
open. 524 renderer + 185 CLI tests passed, one ignored each. Executable:
`C:/dev/perf/warp-sideways-20261027/r58-meadow-sampling/sideways-r58.exe`, SHA256
`ac95dfa6632f4236583989aa2ab4db1d53ec88ad8795ce7770d7a5a74389c203`.
Evidence is beside it (`timing-v2`, `raw-qa`, `raw-analysis-a`). Original failed
harness evidence remains; only the corrected output-vs-viewport validator is
used for the six repeated runs.

The user briefly playtested this binary with one player, 4K/DLSS Quality and
pruning ON; visible presentation was confirmed. It was closed at the user's
request (owned PID 192560). No interactive game should be restarted unless
requested. The subsequent grass-ground AO experiment is recorded above.

## Previous status — r57 earlier/cheaper grass measured, opt-in only

The user chose an earlier cheap-grass follow-up after the r56 decision gate.
Read [r57 implementation, measurements and image review](docs/meadow-half-resolution-r57.md).
Runtime source `72db92f` adds `KERN_MEADOW_AGGREGATE_HALF_RES=1`: half-linear
aggregate depth/material/illumination, full source/merge and conservative
depth reconstruction. Source band 25–35 m, unchanged 160–180 m outer range.
No density/placement/physics changes; no default promotion.

Nine serial A/B/C/B/C/A/C/A/B runs, same exe and four-player 4K-output/DLSS
Performance: current A 27.828/34.098 ms; earlier-only B 27.914/34.181 ms;
earlier+half C 25.938/30.229 ms. C grass is 5.001/5.782 ms versus
6.895/9.545 ms in A. Medians of three run medians; 179 clean frames per
section/run. All 4,320 GPU records validated. No historical-binary gain claim.

Full-output PNG QA completed (52 captures, 28 pixel-crop boards reviewed).
C visibly smears distant ground/grass and lower trunk boundaries, particularly
at frame 480. **Not visually equivalent, not accepted for default use.** Close
edges are broadly similar; motion remains unaccepted (sparse still review of
a recorded 0.2-second sequence is not a temporal pass). Earlier-only B does
not save GPU time here. 25 ms decision gate, 4 ms grass and 8.33 ms total targets
remain unmet. Do not start deeper tree/shading rewrites or change targets on
the strength of this experiment alone. First isolate aggregate filtering and
DLSS/motion contributions or investigate cheaper full-resolution aggregate work.

518 renderer + 185 CLI tests pass, one ignored each; DLSS release build 94 s.
Frozen exe `C:/dev/perf/warp-sideways-20261027/r57-cheaper-grass/sideways-r57.exe`,
SHA256 `faac17848bf3fec16707b50c0a1808dcb3f91cfbd640be0900cd6afa43af1a83`.
Evidence root is its containing folder. No game/benchmark remains active.
The later metadata-only prose correction does not replace this frozen exe.

## Previous status — r55/r56 four-view budget decision gate

27 October follow-up: the user requests DLSS rather than native 1080p per panel.
The measurements below already used DLSS. Developer launcher `-Views 4` now
resolves an omitted `-Upscale` to `dlss-performance`, enables its build support,
and records the selection source. Explicit overrides and 1-3-view defaults stay
unchanged. Generic packaged/GameNight builds are not automatically DLSS-capable.
No new GPU result or reduced graphics tier is implied; deeper-work choice is
still pending. No interactive playtest was started for this launcher change.

Read [the ordered plan](docs/four-view-budget-plan-r55.md),
[resolution sweep](docs/resolution-sweep-r55.md), and
[grass experiment](docs/meadow-band-r56.md) before doing further GPU work.
Branch `codex/fourview-budget-20261026` contains the diagnostics and opt-in
source-band control; implementation commit `9329d2f` built the frozen r56 exe.
PR 4 is draft, PR 6 remains unmerged/opt-in with raw visibility/depth comparison
pending, and corrected PR 5 merged separately to main as `8c62e47` with all
experimental switches still off. Neither PR 4 nor PR 6 is in the r56 executable.

Nine no-code DLSS runs and six grass A/B/A/B/A/B runs completed. Four-player
4K-output/DLSS Performance with a 40-50 m source band measures 26.947/33.351 ms
in the two route sections; default 60-80 m in the same executable measures
27.604/34.126 ms. Grass alone is 6.500/8.950 ms. All are medians of three run
medians of hidden synchronized GPU intervals, not typical gameplay FPS.
Do not attribute the old 48.8 ms baseline drop to code: identical input/exe
measurements shifted between sessions with no historical clock/load telemetry.

The user's decision gate is now triggered: discuss a quality tier or target
before steps 3-4/deeper renderer rewrites. 120 FPS remains the target. Apart from
the developer DLSS selection above, no new graphics defaults are accepted.
The aggregate's world resources are shared,
but its depth/coverage/lighting remain per-camera; shared source pooling is
deferred, not implemented. Preview A/B checks are limited to downsized stills;
moving transitions remain unaccepted. Do not start an interactive playtest.

Validation: 514 renderer and 185 CLI tests pass, one ignored in each; release
build took 95 s. r56 executable SHA-256:
`4878f5971e4b41b385e3ae9dc337fd3be9d0eac42ee118fb89592c8cdcbe2be8`.
Evidence: `C:/dev/perf/warp-sideways-20261026/r55-resolution-grass/` and
`C:/dev/perf/warp-sideways-20261026/r56-meadow-band/`. Both experiments finished;
no game/benchmark session is intentionally left running. Execution/playtest
instructions in older sections below are historical, superseded by this status.

## Previous status — r53 false roadside sun shadow

See [r53 diagnosis and validation](docs/roadside-shadow-r53.md). The disappearing
black wedge is a separate road-caster CLOD mismatch: a coarse triangle bridged
45 m above the exact shoulder. Warp now offers generic cooking-time vertex
constraints; Sideways preserves the full joining shoulder in every cut.
Source/collision geometry and sun bias/filter/strength are unchanged.
CPU false-hit counts drop 69→0 and 106→0 in the two captures. Matched GPU
views at 0/8/16 m, grass-off, and the exact second camera remove the wedge.
6 engine cluster, 183 CLI and 22 replay tests pass; one CLI diagnostic ignored.
F12 replay now accepts explicit bounded --pick x,y and reports shadow counts.
The single-view 4K-output/DLSS Quality hidden ABBA measures shadow cull+raster
0.913→0.919 ms and total covered interval 21.794→22.419 ms; no FPS improvement
or four-player result is claimed. See the doc for sample counts and limits.
Evidence: `C:/dev/perf/warp-sideways-20261026/r53-roadside-shadow/`.
Game SHA256: `ccfbb7c9376db5800e4470edbbada3c984095617a5545cd37eb23f37c0a7bcb6`.
Interactive launch: `games/sideways/build/dev-runs/20261026T192639759Z-155c7fd7/iteration.json`.
Do not compete with that playtest for the GPU. Prior r52 process has exited.

## Previous status — r52 roadside terrain wedges

See [r52 diagnosis and validation](docs/roadside-verge-r52.md). A 55 cm ditch
was too narrow for the 2 m terrain grid, causing the repeated roadside pits
and black headlight-shadow wedges in the user's F12 captures. It is replaced
by a shallow 4 m smooth verge transition, preserving the drivable road/ruts.
Flat-road grid waves drop from 344.8 mm to 8.5 mm. Matched native captures
with grass on and off remove the large wedges at the reported location.
The small jagged grass-clearance boundary remains separate work.

F12 capture_replay now restores applied tree/grass/bloom/local-light controls
and rejects mismatched source recipes; --no-meadow is a recorded diagnostic.
38 simulation, 182 CLI and 19 replay tests passed; normal suites have three
explicitly ignored diagnostics. Release game SHA256:
`82d4d08a2973df43a411a25f221bb6b0bbeb26c9b51ea7b3de05373f36466921`.
Evidence: `C:/dev/perf/warp-sideways-20261026/r52-roadside-terrain/`.
Latest interactive playtest: PID 32728, first visible frame confirmed;
`games/sideways/build/dev-runs/20261026T185740242Z-424ec09c/iteration.json`.
Keep the interactive GPU free from competing fixtures.

## Previous status — r51 saved baseline, vehicle lights and bloom

See [r51 implementation and open checks](docs/local-lights-and-vegetation-r51.md).
The user's tree/grass settings are saved in the Sideways data preset, applied
before renderer construction and retained across new stages. The engine now
supports a separate short-range local-light budget for tail/brake/reverse and
event-synchronized exhaust illumination, without new shadow maps. Exhaust
gas/flame/heat has fixed-tick RPM motion; the metal pipe is not animated.
The corrected release and a192-frame real night bloom on/off/on replay passed;
bloom's three filters average0.096-0.097ms at4K output / DLSS Quality, one
held view. Total covered GPU time remains16.6-16.8ms, not the8.33ms target.
HDR/exposure/AgX existed; new subtle bloom and its strength control are under
Lighting. Lamp cores are brighter while red environment spill is weaker.
CPU:182 CLI tests passed, one ignored; focused engine shader/settings/light
checks passed. See the linked doc for exact manifests and remaining GPU checks.
The native local-light fixture passes8/8 including last slot, clearing and
unchanged headlight depth/casters. Current interactive launch manifest:
`games/sideways/build/dev-runs/20261026T174416078Z-ced5e454/iteration.json`.
Near-grass shapes, grass shadow casting and the reported silhouette border
remain open with evidence and a concrete reproduction procedure in that doc.
The first r51 playtest ran and was closed by the user. It has been superseded
by the corrected executable in dev-runs/20261026T173428410Z-fe6ffcbd.

## Previous status — r50 and requested playtest

Main was pushed in 79cb339 (engine/game split, Godot removal, docs and accumulated
engine work). See [r50 results](docs/world-foliage-r50.md). The tiny world-volume
GPU fixture replaces 64 trees correctly but is rejected for cost/blur: 23.779 ms
integration at 128x128. Do not scale/promote it on mechanical correctness alone.

The user explicitly requested a playtest after the stopped-playtest instruction
below. It now runs the validated r49 candidate E at 4K output/one player/DLSS
Quality; world volumes remain off. Launch manifest:
`games/sideways/build/dev-runs/20261026T161351534Z-c9f58110/iteration.json`.
Leave this playtest running and do not compete for its GPU.

## Publication and in-progress world foliage — 26 October 2026

The local engine/game separation, retired Godot files, diagnostics and r49
work are included in this snapshot after an extended local-only backlog.
Root README is the current layout; historical documents are evidence, not
instructions to resume Godot comparisons.

New `foliage_world`, `source_replacement` and `foliage_world_render` modules
add shared whole-plant assets, a bounded XZ reference grid, GPU selection and
early source omission. `forest_group_gpu` is the controlled native fixture.
This backend is **experimental and off by default**: CPU/WGSL checks pass,
but GPU correctness, optical acceptance and performance are not yet established.
Its current integrator reports explicit failure if its ray-work safety cap is
exceeded; this is not a production fallback or proof of scalability. DLSS/TAA
retain source meshes because temporal volume correspondence is not wired yet.
The actual game still uses source trees; the new game cook adapter is available
but not enabled. Continue controlled validation, not interactive playtests.

Validation for this snapshot: workspace/all-targets compile passes; 708 Rust
CPU/WGSL tests and 43 Python tool tests pass, with three explicitly ignored
Rust tests. The PowerShell developer-launcher checks pass. GPU validation of
the new world backend remains pending.

## Current goals

### Current work — r49, LOD optics and receiver-driven shadows

Interactive playtests remain stopped. Only serial controlled GPU fixtures and
bounded replay benchmarks are authorized. See
[r49 implementation/evidence](docs/lod-shadow-demand-r49.md). Artifact root:
`C:/dev/perf/warp-sideways-20261026/r49-lod-shadow-demand`.

The default-off receiver-demand prototype has exact requested-depth and
final-image parity in `receiver-d/report.json` (five cases: first/wind/cut/four
views/full-map fastpath). **Do not promote it from the sparse fixture result.**
Live B requested every tile and ran about 1 ms slower. Terrain cluster-bound
refinement in D still requested every tile and introduced a severe 89–92 ms
builder cost (full intervals 133/130 ms). All rejected evidence remains saved.
Candidate E stops gathering when unique tile counts prove full demand and
passes all five native GPU fixtures. Fresh live ABBA removes the pathological
cost, but still saves no casters: sunrise 44.465→47.175 ms and noon
37.972→40.837 ms; the builder alone costs 2.818/2.838 ms. All maps stay full.
Strict previews fail even within same-mode repeats. Default remains off;
do not iterate this conservative all-LOD gather further without addressing
actual visible receiver support. Candidate E source manifest has 278 matching
runtime files. All controlled processes exited; no playtest, commit or push.
Candidate A failed near alpha shadows because the positive map selector also
changed material mip gradients. Candidate B uses negative selectors and an
actual composed-shader regression; keep both original and repaired evidence.

Optical source/voxel comparisons and a bounded quarter-octave smooth-mip path
are implemented. Finest 11.25 cm voxels remain visibly too blurry near the
camera. The conservative spatial source-retention gate is currently a helper
and diagnostic, not automatic production replacement. Initial optical A night
and low-sun comparisons used stale day-calibrated sky SH/texture; the corrected
fixture uploads actual sky data for every case. Do not use the old night
images as valid nighttime evidence. Corrected `optical-c` has 144 measurements;
`sweep-d` covers the complete first mip transition at 16 positions/four angles/
two lights. Median coverage error improves 7.92%→5.73%, but low-sun maximum
brightness error remains 39.24% and the blend costs up to 2.758× integration
cell reads. Raw layers are identical outside blending. No optical replacement
accepted and no defaults promoted. See `sweep-analysis-d/summary.md` and zooms.

Important code/evidence audit: live benchmarks have voxel trees OFF. They use
319,179 owners of three alpha-card CLOD assets, with a terminal 137–177 triangles
per tree. Noon selects 55.05M tree triangles across four views (31.83M in the
elevated view). There is no automatic whole-forest group-volume replacement;
the fixed 1–256-tree voxel experiment is not that implementation. Actual grouped
source suppression before traversal/raster is the major next structural step,
shared with grass, with matched optics/current wind/source fallback.

### Previous work — r48, real traversal and headlight work reduction

**Interactive playtests are stopped at the user's request.** Only controlled,
serial GPU runs are authorized until the user asks for another playtest.
No Sideways or brick-fixture process remains running. Historical instructions
to keep a playtest alive below no longer apply.

See [r48 implementation and evidence](docs/render-traversal-r48.md). True empty
ancestor jumps remove 85.32% of fine voxel traversal iterations; four-pose
immutable-field GPU ABBA/BAAB measures 13.34–14.41% less consumer integration
time, with exact raw RGBA16 and RGBA8. This tiny fixture still has unaccepted
source-to-voxel blur and is not the live meadow. `KERN_BRICK_EMPTY_JUMP` defaults
off. Previous read skipping alone was not equivalent work elimination.

`KERN_HEADLIGHT_INSTANCE_GROUPS=1` reuses refitted spatial bounds with separate
per-lamp frusta, lists and counters. Four-player ABBA, one 4K output/DLSS Quality,
127 clean samples per section/run: night 34.3526→31.9929 ms, sunrise
46.9173→44.4260 ms. Exact caster selection and shadow-depth hashes pass; strict
full-preview equality fails, including same-mode repeats. It remains default
off. These are render intervals, not 120 FPS or native-4K results. Source
density/LOD/shadow resolution are unchanged between variants.

Candidate A source and executable manifest:
`C:/dev/perf/warp-sideways-20261026/r48-work-reduction/candidate-a.json`.
Sideways SHA256 `d6b17f79f001251998d036f444d26990fe18cae9e57dc44f3d2da01dcc4e04a3`;
brick fixture `9377a86f92f8090fa452454f000ae8aadbb0729a82ab0d0c42be36f8bdc42897`.
462 render + 5 fixture + 164 game + 13 harness tests pass, two existing ignored.
All 272 source hashes still match. No commit/push. Preserve dirty reorganization
and all evidence. Native harness analysis recovered a completed OFF run after
a startup-null-frame bug, without rerunning its successful GPU capture.

Next structural gap: directional shadow raster still dominates some scenes.
[Receiver-demand plan](docs/receiver-shadow-demand-plan.md) is design-only:
current-pose receiver demand across all views, including grass/transparency/
non-surface lighting consumers; unknown demand must fall back conservatively.
Do not equate the existing rest-pose distant page cache with dynamic VSM.

### Previous work — r47, measured structural rendering improvements

The user has stopped interactive playtests: **do not launch a playtest** until
asked again. Use controlled GPU runs, serially, with saved images and actual
work counters. Historical process IDs and "keep running" instructions below
do not describe the current session. Last process check found no Sideways or
brick fixture process running.

See [r47 results](docs/meadow-work-r47.md) and
[r46 work analysis](docs/render-work-avoidance-r46.md). The same-source meadow
visibility experiment removes a second geometry submission. Four-view ABBA:
43.559→38.696 ms noon, 49.628→46.778 ms sunrise; 4K output with DLSS Quality,
1277×717 per 1916×1076 panel. It remains opt-in, strict image parity fails,
and 8.33 ms has not been reached. Native wind/cut/night checks preserve source
and selection identity. The real GPU grass brick fixture passes mechanical
checks but has unaccepted blur and excessive empty traversal; it is not the
live field. Separate candidate-C hidden-ground-shading ABBA measures another
38.607→38.055 ms noon and 46.776→46.421 ms sunrise with visibility enabled in
both cases. It stays opt-in. Empty-cell read skipping bypasses98.47% of reads
but improves its small fixture by only0.09–1.32%; keep it off. It does not yet
skip fine DDA arithmetic. CandidateC:459 render+2 fixture+164 game testsPASS,
two existing ignored. Strict image gates remain failed and preserved; these
are not identical combined-binary before/after measurements or120FPS.

Evidence: `C:/dev/perf/warp-sideways-20261026/r47-meadow-work/`. Preserve frozen
binaries and failed evidence. Source defaults are not performance acceptance.

### Historical playtest — r40, rounded grass and night sky

User asked to start immediately; tested candidate A is running and independently
verified responsive with first visible frame. PID118076, startUTCticks
639285520930716743, `../games/sideways/build/dev-runs/20261025T190811046Z-a344157b/`.
SHA256 `28f1a1798492357df1a256079be2e7f1162a3b88654ce1b8ce2084225893539d`.
One3840x2160 output, DLSSQuality, wind0.6, aggregate/source80m unchanged.
Keep this playtest running. No GPU fixture remains active.

Reported square grass holes are removed in matched F12 images: actual360-blade
source has overlapping rounded feathered density, bounded world-root jitter,
full yaw, smooth spatial growth, no random deletion of suitable whole patches.
Same source recooks distant atlas. Road safety can still make stepped bare edges.
Night has a faint sky-SH floor and stable depth-occluded procedural stars. The
night ground is still VERY dark; user requested immediate playtest before any
further tuning. Do not silently brighten it and claim candidate A was retested.

350 render and149 CLI tests pass, one existing ignored test in each suite.
Release52.83s. Eight one-view4K/DLSS visual sections and three four-view sections
completed; all actual resolution/settings metadata verified. Four-view GPU
means48.410ms noon,57.686ms sunrise,52.767ms night (71samples each); nowhere near
120FPS. GPU/CPU contention caveats remain. Evidence and per-scene timings:
`C:/dev/perf/warp-sideways-20261025/r40-grass-night/`; grass review is
`grass-visual-review.json`. See `docs/meadow-night-r40.md`.

### Historical playtest — r39, distance continuity with visible tradeoffs

The new one-player playtest is running, verified responsive with first visible
scene frame. PID242232, startUTCticks639285500936486110; executable and manifest:
`../games/sideways/build/dev-runs/20261025T183451763Z-519c1346/`.
SHA256 `46c964ebaf9bb260a47364cb81d637c4bb1de7c0d9a276e30abbd956a09ff9f2`.
One3840×2160 output with DLSSQuality, live wind0.6, source radius80m and aggregate
enabled. First visible scene at9.154s from play entry. Runtime logs verify the
constructed60–80m transition and60–180m aggregate. Keep this playtest running
during ordinary CPU work; verify its PID/path/start time before stopping it.

The source-derived directional surface atlas uses hardware filtering and one
lighting resolve per pixel. It is NOT voxel transport. Near source roots,
density and blade geometry remain unchanged; expensive source geometry blends
to the aggregate earlier over60–80m. The aggregate fades to matching substrate
colour over160–180m. Launcher default is now enabled; explicit switch or env
can select the reference. `SIDEWAYS_MEADOW_SOURCE_RADIUS=100` retains the old
range. `KERN_MEADOW_SUBSTRATE=1` works independently for diagnosis.

341 renderer tests and3 grass-field tests passed. Native11-scene, DLSS motion,
and four-player GPU suites completed serially. Two F12 native views cost
19.041→20.240 and18.451→19.110ms. Four-player4K/DLSSQuality has71 samples/section:
50.566→50.557ms noon,60.580→60.377ms sunrise—essentially unchanged, nowhere near
120FPS. Inputs are1277×717 per1916×1076 output pane. CPU CI remains active.

Limited visual acceptance: all22 native stills and48 motion-sampled JPEGs were
reviewed; coverage improves but open distant slopes can show a faint grid.
Stepped near-road grass edges remain. JPEG samples are not continuous temporal
proof, and wall-clock wind poses differ between runs. Night fixed-camera images
were black and do not validate headlights. The old one-pixel r38 tree-occlusion
failure is still open. Candidate A was rejected for9.6–42.5ms aggregate cost;
its evidence is retained. Full details: `docs/meadow-distance-r39.md` and
`C:/dev/perf/warp-sideways-20261025/r39-grass-distance/`.

### Historical playtest — 25 October, r38, limited acceptance

User requested the playtest; the root verified a responsive process and first
visible scene frame from the existing tested pre-diagnostic release. Active at
this update: PID267312, startUTCticks639285453706207673, executable
`../games/sideways/build/dev-runs/20261025T171609647Z-r38-play/sideways.exe`.
Manifest: `../games/sideways/build/dev-runs/20261025T171609647Z-r38-play/iteration.json`.
SHA256 `efb28754f4223aa1fb9258944308467b375848dc29447378d7d989b12e9b5c04`.
One3840x2160 output, DLSSQuality, livewind0.6. Constructor logs confirm
fullness1.25 and tree occlusion enabled. First scene frame:10.575s from play
entry. The original launcher returned a post-frame verification error; direct
log/process verification established success without another restart. Keep
this game running; verify path/PID/start ticks before any later lifecycle action.

Current-frame two-phase main-colour tree occlusion now uses actual alpha-tested
non-tree/near-tree depth, then a conservative pyramid for remaining trees.
Shadows retain independent casters. Live trees remain three shared alpha-card
sources,136fronds/1088crowntriangles each, with fuller stem-anchored fronds; this
does not deploy the full procedural assembly/voxel prototype.

Limited acceptance:326render tests and8crown tests passed; static native6/6
and four-view2/2 image pairs are exactly equal. `performance-b` has39 clean
GPU samples/camera, one4Koutput/DLSSQuality: same-source culling off→on means
24.784→23.974ms forest,17.165→10.756ms cliff,34.821→27.858ms road vista.
These are provisional real measurements without a confidence interval or
four-player120FPS claim. Fuller source changes CLOD-selected geometry despite
unchanged finest-source triangle counts. Release build65.914s.

**Open temporal failure:** `temporal-parity-b` and `-c` each match all96 inputs
but fail1of32 PNG pairs at frame22: one pixel(217,232), maxchannelerror62,
tree→terrain winner. Off/off is exact; nondeterminism is not established.
Depth-tie/draw-order behavior is plausible, not proven. Exact gate stays failed.
New `KERN_TREE_OCCLUSION_REJECT=0` and capture `rejection_enabled` metadata are
source-only, unbuilt/unvalidated and **not in this executable**. The diagnostic
retains both phases while suppressing Hi-Z rejection; no depth bias or tolerance
was added. Next diagnostic must distinguish ordering from false rejection.

Latest user grass-pop screenshots are being investigated. Meadow far-blade
aggregation, own grass shadows and visible edge/pop issues remain open; r38
does not solve them. Details: `docs/forest-occlusion-r38.md`; evidence:
`C:/dev/perf/warp-sideways-20261025/r38-forest-occlusion`.

### Meadow budget playtest — 25 October, r37

Historical playtest; superseded by the running r38 build above.

User asked for simpler grass/earlierLOD/fewer polygons while preserving r36's
dense coverage. Physical affine ribbon-error bounds now avoid multiplying
blade-width deviations by the much larger blade-height scale. Accepted source
cuts1800/1080/600 and360blades/patch stay unchanged; one-pixel threshold stays.
Exact indexed depth/colour streams share only bit-identical required attributes
across all variants, with KERN_MEADOW_INDEXED=0 as reference fallback.

21meadow and12CLOD tests PASS; finalrelease60s. Native indexed off/on fixtures
exercise all threeLOD levels and preserve everyRGBAbyte in eight captures,
including first/settled/camera-cut frames. Frozen wind, not temporal acceptance.
Matched4Koutput/DLSSQuality r36/r37 has39cleanGPU samples/camera underCI:
chase source triangles14,629,800→9,855,960(-32.63%), same16,365patches;
meadow15.368→12.619ms(-17.89%), coveredGPU36.506→33.852ms. Two-camera
JPEG review retains dense cover; square verge edges and missing own grass
shadows remain. This is oneA/B, not sustainedFPS or four-player120FPS proof.
No far voxel aggregate yet: nearly all chase patches already select600triangles,
so next major savings require aggregation of distant blades. Details and
bounded next direction: `docs/meadow-budget-r37.md`.

New playtest first visible scene frame confirmed after10.056s, responsive:
`../games/sideways/build/dev-runs/20261025T155253489Z-r37-play/iteration.json`.
PID236708, startUTCticks639285403742534550, SHA256
`99e02645b74f4c84eb1bd22d1c74c8969b3ea0e89abf6b1de826d3058fba884f`.
One4Koutput/DLSSQuality/livewind0.6. Verify active manifest/path/start ticks
before stopping. Previous r36PID3128 was identity-checked and stopped before
serialGPU tests. No commit/push; preserve existing repository reorganization.
Evidence: `C:/dev/perf/warp-sideways-20261025/r37-meadow-budget`.

### Dense meadow playtest — 25 October, r36

Historical playtest; stopped and superseded by r37 above.

User rejected r35 coverage and asked for roughly10x density. New MeadowPatch
has360 narrow ribbon blades across each fixed0.45m square versus36; nominal
1778 vs178 blades/m² before habitat/road/visibility filters. Individual root
anchors stay fixed under growth and GPU wind. Three shared shapes, four wind
groups, source-derived ribbonLOD1800/1080/600 triangles; no CPU blade stream.
Candidate capacity205,209 and root/owner pool25,445,916bytes/view unchanged.
Packed source vertices64B; shared recipes2,004,480B. Road-edge eligibility is
per blade where its anchor footprint fits one native triangle, with a safe
whole-patch fallback elsewhere; some visible square boundary steps remain.

16 focused meadow/source/anchor/bounds/ABI/shader tests plus3 ribbonLOD tests
pass. Final release55.23s. Exact serial r35/r36 comparison-b has matching seed,
two cameras/tick6000/light/fixed wind and actual4K-output/DLSSQuality. Both
images reviewed: much denser cover, still imperfect boundaries.11 clean GPU
samples/camera underCI: chase grass11.776→15.261ms, coveredGPU32.382→35.498ms;
r36 selects14,629,800 source triangles (before per-blade rejection, depth and
colour submit twice). No120FPS/4view claim; own grass shadows/far voxel field
still absent. A single terrain plane per patch can bury/float distributed
roots across native triangle boundaries; the road guard does not fix this.
See `docs/meadow-density-r36.md` and
`C:/dev/perf/warp-sideways-20261025/r36-dense-meadow/comparison-b`.

New playtest first scene frame confirmed after10.075s:
`../games/sideways/build/dev-runs/20261025T151030601Z-r36-play/iteration.json`.
PID3128, startUTCticks639285378313637208, SHA256
`9c5e31b6bec09b1e7bba125eb52a4e189d290597c97234445736b779f79337b5`.
One player/3840x2160 output/DLSSQuality/live wind0.6. Verify current manifest,
PID/path/start ticks before stopping. Old r35 was stopped after successful
compilation and before serial GPU comparison. No commit/push; preserve all
staged/dirty/untracked repository reorganization and unrelated work.

### Meadow appearance playtest — 25 October, r35

Historical: user subsequently rejected this density. Superseded by r36 above.

Latest feedback: r34 looked too uniform in colour and left too much bare ground.
The GPU field now has stable fresh/olive/dry material patches, stratified roots,
terrain-conforming current/previous wind poses with corrected normals, and a
reworked low/middle/tall source canopy. Corrected edge-on ribbon cross-sections
and reduced soil inset to5mm. Still36 blades/792 finest triangles per tuft;
candidate capacity remains205,209/view. No per-frame CPU root list.

Three actual fixed-camera image iterations: A/B rejected as too sparse; C is
accepted for interactive iteration with visible limits, not finished coverage
or Nanite acceptance. Low ribbons now overlap more ground; sunlit basal cover
remains pale/flat, cliff tufts remain discrete. Own grass shadows and far voxel
aggregation are still missing.11 focused tests pass; release build54.49s.
Chase C selects12,025,656 source triangles /15,878 roots (source counts once;
depth+colour submit twice). Short one-view4K-output/DLSSQuality A/B has11 clean
samples/camera under concurrentCI: no performance certification;120FPS unmet.
See `docs/meadow-look-r35.md` and
`C:/dev/perf/warp-sideways-20261025/r35-meadow-look/comparison-c/validation.json`.

New playtest first visible frame confirmed after9.844s, process identity checked:
`../games/sideways/build/dev-runs/20261025T121139630Z-r35-play/iteration.json`.
PID252292, SHA256
`ccdc8d52fc36799c22e10080098565969f0ea15f6eb822ce06ffd02d7eb3c120`.
One player/3840x2160 output/DLSSQuality/live wind0.6. Verify manifest path/start
ticks before stopping. Previous r34 playtest below was stopped for this update.
No commit/push; preserve existing staged/dirty reorganization and other work.

### Full-terrain GPU meadow playtest — 25 October, r34

Latest user request: replace the old grass with covering procedural grass,
designed for variation and wind with GPU placement/rendering instead of CPU
per-root streaming. The user reports concurrent CI CPU load: treat all timing
comparisons cautiously; keep CPU/GPU, startup and capture costs separate.

`SIDEWAYS_PROCEDURAL_GRASS=field` now replaces photo/far grass over supported
terrain. CPU uploads native height/road-clearance data plus three shared
MeadowGrass source recipes once. GPU derives stable world-cell roots, habitat,
frustum/Hi-Z rejection, projected-error source LOD, indirect lists and four
branch wind poses per root. Source depth/colour/normals/current+previous motion
share the poses. Full field supports native and DLSS; it is not the old bounded
23-tuft patch. Physics120Hz/four substeps unchanged.

Current limits: source meshes only; far voxel field aggregation and its own
grass shadow casting still absent. Radius100m/fade80m; max205,209 candidates
per view. SourceLOD triangles792/576/576 per tuft,36 blades each. Per-view
root/owner buffer25,445,916bytes; shared terrain50,539,536bytes for seed1/km3.
The whole GPU field has not solved open-terrain triangle cost. Shader reload
is explicitly rejected in this opt-in until transactional meadow reload exists.

New reusable current-frame minimum reversed-depth pyramid skips fully hidden
wind bounds, keeps sky holes and folds odd edge texels. It is applied only to
meadow, not the tree culler. `KERN_MEADOW_OCCLUSION=0` disables query rejection
for reference tests while still building the pyramid. Native1280x720 off/on/off
four-capture test (cliff, chase, first frame/camera cut/settled; fixed wind1.9)
is byte-identical. Actual4K DLSS close-cliff source triangles17,909,784 ->118,008;
this workload count is not a general frame-time speedup. ActualGPU synthetic
motion157cases pass. Renderer304tests pass/oneignored; game144pass/oneignored.

Real4K DLSS Quality one-view and four independent-view/camera-cut scenarios
each rendered/presented120frames, no errors/fallback. Four-view measured GPU
coveredp95 about34.0–34.2ms (38steady samples/section), CPU36.9–38.9ms under
concurrentCI. Target8.33ms remains unmet. Evidence:
`C:/dev/perf/warp-sideways-20261025/r34-meadow-field`;
read `docs/meadow-field-r34.md`, parity-report and validation.json.

Playtest started and first visible frame confirmed in18.15s:
`../games/sideways/build/dev-runs/20261025T085637578Z-r34-play/iteration.json`.
PID29644; inspect manifest start ticks/path before stopping. Executable SHA256
`0d64ca137d7fad3742d2cdf518e27a15661901be0c745cf57ae5e1e7f57b135b`.
One player,3840x2160 output/DLSS Quality, field mode, live wind0.6. Antipodal
voxel-normal candidate remains disabled. No commit/push this iteration; preserve
the large existing repo reorganization and unrelated dirty work.

### Vegetation source LOD and empty-space traversal — 25 October, r32

Latest request: fix the excessive source triangles and far-volume cost found
in r31. Added `vegetation_clod`: source-derived closed blade/needle reductions
keep every placement, root/tip, finest source vertex/triangle and branch pose.
Unknown geometry uses the existing cooker. Source/full-level and resident-LOD
vertex counts are now separate. The six-plant recipe remains 354 unique finest
triangles, 315 placements and 30 poses; resident geometry is 79,808 bytes.

Actual four-view native-4K colour triangles at 150 m fall 27,840 -> 16,608;
250 m falls 13,536 -> 8,160. No voxel boxes are rasterized. Full sweep covers
23 configurations, with real GPU owner/cluster/volume-selection readback.
72 source-only optical pairs cover 1080p/4K, two angles and three wind poses.
5 m images are byte-identical. Well-resolved plants lose at most 3.47% coverage;
maximum local ROI RGB MAE is 0.651/255. Explicit coverage/error gates pass.

Fixed empty-space skipping consulting an upper mip that the selected maximum
mip forbids. Default ON; `KERN_VOXEL_EMPTY_CEILING=0` retains the baseline for
diagnostics. ABBA pooled16 instrumented frame sums at 500 m: 5.730 -> 3.514 ms
(-38.7%); 250 m -26.4%, 1000 m -47.2%. These are six-plant fixture results,
not gameplay FPS. Three full-4K comparisons are byte-identical; the mixed
source/volume acceptance image differs by at most one u8 step. Source, pose
and camera-cut oracles pass. Compact private ray caches were measured and
removed because they gave no convincing gain. No full-forest/default opt-in.

Evidence: `C:/dev/perf/warp-sideways-20261025/r32-vegetation-optimization`;
`validation.json` records source/executable hashes, ABBA and image metrics.
Read `docs/vegetation-optimization-r32.md`. 283 renderer tests pass/one ignored;
workspace check and Sideways maximum-patch budget test pass. Real-stage auto
scenario renders/presents all120 frames at native1600x900, fixed23 grass+2
saplings, with physics120Hz/four substeps. Four JPEG captures are in
`stage-auto/captures`; this integration preview is not optical acceptance.

Normal playtest restored with previous lighting/cache settings, one-player
4K output/DLSS Quality/wind0.6, prototype OFF. Manifest:
`../games/sideways/build/dev-runs/20261025T014248475Z-c98ff7af/iteration.json`.
PID72736; start UTC ticks639284893704903121. SHA256
`733bee7477b826c19059dbeeace849a7f483610678c09428c671037defc258d7`.
First visible frame and process identity verified. Check identity before
stopping; never benchmark the GPU alongside the playtest. The shared-primitive
path remains a bounded native-only patch; optical voxel parity, DLSS motion,
spatial binning, larger plant aggregation and full-forest integration are open.

### Vegetation distance/work accounting — 24 October, r31

Latest request: prevent excessive triangles/boxes and count by draw distance.
Added `vegetation_accounting::Accounting`, actual GPU foliage-selection readback
on colour captures, and exact 64-bit voxel steps/samples/cell-read/AABB totals.
The standalone `vegetation_distance_budget` example writes JSON/CSV for 23
configurations at native 4K, one full view or four 1920x1080 views, 5–2000 m.
No image readbacks in the timed sweep. See `docs/vegetation-distance-budget.md`.
Evidence: `C:/dev/perf/warp-sideways-20261024/r31-vegetation-budget`.

Key finding: six plants draw 6,960 vegetation triangles per view; four 1080p
views keep all 27,840 through 150 m, draw 13,536 at 250 m, and zero at 500 m.
The 500 m volume path still costs 5.882 ms instrumented GPU pass sum (8 samples).
Zero triangles is not low cost. Volumes are integrated fields, not drawn cubes.
The 354 shared triangles are storage, not per-frame drawing. No defaults changed.
The patch maximum test guards 31,456 triangles/view, 1,428 primitive placements,
180 poses and 148 volumes. Source shadows still remain after voxel colour LOD.

272 renderer tests pass, all-target check/release fixtures pass; source/voxel
acceptance and camera-cut checks pass. Five decoded images exactly match r30.
The first attempted sweep failed a missing COPY_SRC buffer-usage validation;
fixed before `sweep-final`, which passes all counting/overflow guards.
The focused maximum-patch regression test also passes. `validation.json` records
source/fixture hashes; GPU images are unchanged versus r30.

Restored normal playtest: `../games/sideways/build/dev-runs/20261024T215055292Z-9db9cb62/iteration.json`.
PID 160656, start UTC ticks 639284755158762648, first visible frame and identity
confirmed. Executable SHA256 `9ad5a8de42d580add9748457c75fda1f35e640ca8cb1dbfa075481f32a3fac5a`.
One player, 3840x2160 output, DLSS Quality, wind 0.6; prototype remains off.
Build 59.46 s, launcher startup proof 11.11 s. Check identity before stopping;
no GPU benchmarks alongside this playtest.

### Unified vegetation source/pose integration — 24 October, r30

Latest request: work on unified vegetation. Implemented `vegetation_asset` in
the engine: `CookedPlant` stores unique primitive meshes; `SceneVegetation`
attaches compact placements, branch proxies, source groups and source-derived
volumes transactionally. A separate GPU scatter pass applies each shared branch
pose to its primitive members. Grass and conifer saplings use the same path.
This is not a full-forest replacement or a completed Nanite implementation.

Actual seed-1 patch: 23 grass + two supported conifer saplings, 16 shared meshes,
966 primitive instances, 125 poses and 102 volume owners. Geometry payload is
40,928 bytes; expanded unclustered source comparison is 2,297,376 bytes; packed
volumes are another 973,540 bytes. These exclude other CPU/GPU allocations.
The near recipes are visibly sparse; botanical variety and optical LOD parity
remain unaccepted. Default production foliage and the DLSS guard are unchanged.

Read `docs/unified-vegetation.md` and the updated forest roadmap. Evidence root:
`C:/dev/perf/warp-sideways-20261024/r30-unified-vegetation`.
The GPU fixture `examples/unified_vegetation_gpu.rs` compares shared primitives
with independent expanded source at three wind states; decoded source images
are identical, source/volume poses and bounds agree below 3e-7, and four views
independently select 0/0/26/27 volume branches. The real-stage scenario is
`games/sideways/scenarios/unified-vegetation.toml`; required environment is in
its comments. Source and auto runs completed all 120 frames at fixed wind time.

Final validation: renderer 267 tests pass (one ignored), Sideways app 142 pass
(one ignored), locked all-target workspace check and native release fixture pass.
`gpu-final/report.json` also tests a camera cut from volume views back to source:
all four images exactly match expanded geometry, with zero stale volume pixels.

The critical remaining cost is the bounded multi-volume integrator: about
3.0–3.4 ms in the small four-view fixture for only 71 traced rays. A candidate
pixel-compaction path reduced heavy invocations from 2,048 to 71 but was about
24% slower in serial ABBA, with identical decoded images. Rejected code and
hashes are in `rejected-pixel-compaction/`; `compaction-decision.json` records
the result. The runtime path is removed; useful active-tile/pixel counters stay.
Do not assume pixel compaction is a speedup or promote voxels because poses pass.

Next: jointly traverse/aggregate plant geometry, investigate the expensive
volume kernel, improve recipes and validate optics/motion before broad use.
Limits remain 256 poses/volume owners, 64 branches per plant, native-only.
Do not reintroduce Godot comparison or conflate mesh-memory savings with FPS.

After serial GPU validation, the normal playtest was restored with the previous
accepted lighting/cache flags: one player, 3840x2160 output, DLSS Quality,
wind 0.6, procedural-grass prototype off, far grass on. First visible frame and
process identity confirmed. Latest manifest:
`../games/sideways/build/dev-runs/20261024T213502691Z-928d796a/iteration.json`.
PID 190680, start UTC ticks 639284745734068157; executable SHA256
`787a246e539c7441108e9abcfeed121f2dff0c51e66c8a97e4d2ccd1bc402a3f`.
Build took 69.26 s, startup to launcher proof 13.49 s. The new shared plant path
is available in the native fixture/scenario, not enabled in this DLSS playtest.
`r30-unified-vegetation/final-validation.json` records final source/fixture hashes;
`restored-playtest.json` records the checked identity. Recheck identity before
any later stop. Do not run more GPU benchmarks alongside this playtest.

### Repository split and measured category costs — 24 October, r28/r29

Two equal goals remain: develop Warp and apply it in Sideways. Reusable crates
are engine/crates/{kern,render,audio}; game app/sim/determinism/assets/data/tools
are games/sideways. Package names remain cli/sideways/gdcompat. The latter is
active deterministic math/RNG, not a Godot runtime. Game assets resolve from
SIDEWAYS_GAME_ROOT, an executable-adjacent bundle, or the source game root.
Audio receives explicit presets; the engine owns default fonts and test fixtures.

274 retired Godot/duplicate files were hash-verified into
`C:/dev/perf/warp-sideways-20261024/r29-repo-cleanup/legacy-godot.zip` before
removal. Historical logs/builds were preserved. Empty-directory removal was
blocked by automatic review and was not retried. Native Windows CI replaces
Godot import/export. No commit/push was made during this cleanup.

New category diagnostics preserve shadows/GI/physics/effects while omitting
colour geometry. Full evidence: `r28-categories/matrix/comparison/report.json`
under the perf root. Four-player 4K-output DLSS Quality: baseline 35.52 ms,
trees hidden 16.95 ms, grass hidden 33.65 ms. These are marginal diagnostic
effects, not additive costs or quality-preserving improvements. Frame 240
selects 123.30M tree triangles across four views. Read
`docs/render-categories-2026-10-24.md` and the updated forest roadmap.

Priority: non-expanded shared plant assemblies, hierarchical branch/voxel
representations for conifers and grass, optical/LOD/wind consistency. The
experimental voxel/part-wind path is still not a full-forest replacement.

Relocated all-target workspace tests: 731 passed, zero failed, eight ignored;
diagnostics Python tests: 42 passed. Launcher checks and release build passed.
Packaged 45,000-tick simulation exactly preserved hash `f811a948ee4d3689`
and all summary values. Two packaged four-view GPU runs completed; both match
all 240 baseline render states and capture counters. Final-state differences
are only measured startup/CPU timings. See `r29-repo-cleanup/migration-verification.json`
and `docs/repository-layout.md`. These prove relocation, not a performance win.

Restored playtest: `../games/sideways/build/dev-runs/20261024T204621962Z-51d61229/iteration.json`.
PID 160248 and matching executable path verified at launch; first visible frame
confirmed. One player, 3840x2160 output, DLSS Quality, wind 0.6, full scene (no
diagnostic omissions). Standalone, not a new physical-controller/GameNight
acceptance. Check its saved process identity before any later stop/restart.
No more GPU benchmarks while it runs. The packaged verification executable is
separate from this final source launch; the intervening code change only fixed
the licensed renderer-test fixture path.

### Historical accepted runtime optimization and pipeline cache — r27

Historical accepted playtest: `build/dev-runs/20261024T194651269Z-cd17a581/iteration.json`,
PID162080 at verification, SHA256
`a11cc943f5127149ec8299b407e599ffaa081920c1f2c7e23cfa6403a4b9d126`.
First visible frame confirmed, 1 player, 4K output, DLSS Quality, wind0.6.
Warm graphics preparation3.61s, whole startup16.81s; cached build1.88s.
Check the saved process identity before any later stop/restart. No other GPU
work alongside this live playtest. Older PID264524 was closed for benchmarking.

Grass depth prepass is now default ON (`KERN_GRASS_DEPTH_PREPASS=0` opts out).
Actual four-player 3840x2160-output DLSS Quality ABBA covered-GPU medians:
44.529/35.538/35.603/44.578ms; grass interval ~9.89 to1.02ms. Same route,
all four car/camera traces, unchanged grass density/range/lighting. Native PNG
differences match baseline repeat variability. Exact-zero shading early exits
also accepted: shade ~7.57 to7.36ms in a separate ABBA. Target8.33ms unmet.

`KERN_INSTANCE_BROADPHASE=1` and `KERN_RASTER_LARGE_FIRST=1` remain experiments,
default OFF: the former adds ~0.16ms net culling cost in the measured route;
the latter had no useful raster gain. Their existence is not a speedup claim.
Broadphase GPU fixture v2 passed exact pixels/IDs/counts including70,003 visible
owners and four views. The original debug-reader buffer-index mistake was
separate from, and did not affect, the frame profiler. The newer active-view
snapshot diagnostic API passed the final build's v3 GPU fixture, including
stale submission and four-to-one active-view rejection.

Evidence: `C:/dev/perf/warp-sideways-20261024/r27-runtime/`.
Measured ~22s renderer preparation is mostly pipeline creation (~18–19s),
with warm texture arrays <1s. Persistent Vulkan pipeline cache is now default
ON when supported, with driver/adapter/ABI checks and nonfatal cache fallback.
Same-executable four-view off/cold/warm renderer:23.55/23.36/1.36s;
graphics worker26.05/25.82/3.86s; complete scenario startup45.05/45.03/22.63s
(includes stage generation and6000-tick replay). Warm loaded1,603,120 bytes,
zero errors. GPU steady35.326/35.251/35.285ms, unchanged scene traces and
image differences consistent with repeat DLSS noise. Cache is not an FPS gain.
See `pipeline-cache-comparison.json` under the evidence root. Latest combined
GPU median35.285ms versus original44.55ms is ~21% lower, still far above8.33ms.
Playtest cache directories are explicitly recorded in the launch manifest.
See `docs/iteration-speed-2026-10-24.md` for accepted/rejected experiments.
Scenario runs now hash supported shader overrides before/after execution;
iteration reports distinguish same-executable shader-content changes.

### Historical iteration tooling result — 24 October, 18:55 UTC

Previously accepted executable SHA256:
`166389215d08592336ba42727fb0031fb847fb07458b2bf880d420d5d7607f18`.
Previous playtest (now closed): `build/dev-runs/20261024T185443647Z-ee0ca6f8/iteration.json`.
This historical launch
uses the persistent cache at `C:/dev/perf/warp-sideways-20261024/r26-regression/texture-cache-v1`.
Do not run another GPU workload alongside a live playtest.

The asynchronous frame profiler and TOML scenario runner are integrated and
GPU-tested at actual 3840x2160 output with DLSS Quality, one and four views.
See `tools/frame-budget.md`, `tools/scenarios/`, and
`docs/iteration-speed-2026-10-24.md`. Fullscreen is the runner default; it verifies
actual output size. The earlier `scenario-noon-001` window was OS-clamped to
3840x2019 and must not be called 4K. Capture/next-cadence/warmup samples are
excluded from the default steady report; raw data retains their cost.

Scenario captures now default to JPEG width1600/quality85, GPU-downsampled
before readback. Explicit `[capture] format='png', max_width=0` retains native
pixel evidence; F12 is unchanged. Matched frame120: 2991.88ms/13,813,153 bytes
native PNG vs73.94ms/483,571 bytes preview. Four-format gradient/HUD/gamma GPU
fixture PASS. Output/game render resolution is unchanged by preview settings.

Content-addressed processed texture/mip cache is ON by default, opt-out
`KERN_TEXTURE_CACHE=0`, custom `KERN_TEXTURE_CACHE_DIR`. Matched same-binary
off/cold/warm renderer constructor:3593/3953/1284ms; warm38/38 hits, no errors.
Whole startup remains16.57s warm in this run; do not imply a proportional
whole-startup or FPS improvement. CPU mip parity/corruption/concurrency tests
pass; independent DLSS final PNGs have tiny differences and are not byte-identical.

Four-player route GPU-covered median44.53/45.47ms by section, p95~47ms.
Target8.33ms remains unmet. Largest GPU passes: raster14.5–16.3ms,
grass copy/nearest8.6–9.9ms, shade7.4–7.6ms. These are controlled route data.
No renderer-quality tradeoff was introduced by texture caching or preview encoding.
Next-stage actual GPU smoke PASS: round1→2,567 loader frames, ready; initial
loader62 frames, physics tick0 at install. PR#3 audio remains merged below.

Every dev launch now records startup phase JSON in its manifest and logs.
`tools/iteration-report.py` aggregates explicitly supplied run roots without
mixing different configurations. Evidence root remains `r26-regression`.

### Regression diagnosis preceding the completed tooling

The 17:52 demo below was closed by the user. It is **not running**. Do not
restart that stale executable: it contains the wheel-triggered foliage-cache
invalidation regression. PR #3 is now merged on GitHub and incorporated
locally at `d4245753737b939d028ade6be9b64cb3f5efd4d4`; eight audio tests pass.
The existing unrelated staged/unstaged graphics changes were preserved.

The fix distinguishes moving receivers from changed static shadow casters.
Wheels no longer reset every foliage page. In the same deterministic 4K-output,
DLSS Quality, one-view 100-frame test, synchronous measured graphics wall
median fell from 99.784 to 38.382 ms, GPU pass sum from 48.657 to 29.639 ms.
The 40 post-warmup frames have zero page rebuilds/updates/missing pages.
These are diagnostic timings, not gameplay FPS; 120 FPS / 8.33 ms remains unmet.
Evidence: `C:/dev/perf/warp-sideways-20261024/r26-regression/`.

Startup now presents a real animated loader before building the Renderer on a
worker. Actual GPU image `after.png.startup.png` and startup report pass; the
race stayed on tick0 throughout loading. Initial CPU scene generation remains
before window creation. Renderer preparation was 14.76 seconds; final upload
wait 5.53 ms. Constructor instrumentation and processed texture cache are now
integrated; the older14.76s observation is not the matched cache baseline.

User explicitly wants continual iteration-time measurements, aggressive safe
caching, asynchronous CPU/GPU budget tooling, and text-defined deterministic
scenarios (seed/camera/ticks/sections/lighting/captures). Active source work:
`--frame-profile`, `--scenario`, `tools/frame-budget.py`, scenario runner and
examples. Async GPU 4-slot fixture PASS, startup CPU5 PASS, foliage CPU8 PASS.
Integrated scenario/profile validation is complete as recorded above.

### Current lighting demo — 24 October, 17:52 UTC

**Historical demo PID189948, now closed.** First visible frame was confirmed.
Manifest `build/dev-runs/20261024T175235340Z-a3048d19/iteration.json`.
SHA256 `2c3912c46d8baa213383f658859aaf43fb577e12d39dd79a3ba59d9c457f7b48`.
One player, fullscreen4K output, DLSS Quality, wind0.6. Do not run GPU tests
beside this playtest. User can change sun presets via Start/Esc > Time of day;
pages invalidate/rebuild over bounded frames after the shared sun update.

Enabled `KERN_WORLD_TERRAIN_VISIBILITY=1`, `KERN_PAGED_FOLIAGE_SUN=1`,
`KERN_FOLIAGE_SUN_PAGE_METRES=256`; legacy `KERN_FAR_SUN_SHADOW=0`.
Engine page-size default remains64;256 is the selected demo setting, with
0.508m depth texels outside the existing near/medium shadow maps. Memory stays
256MiB. No real gameplay radiance-cache bounce is integrated yet.

The17-buffer grass-selection failure was repaired with a dedicated six-storage
layout (four actually used), without increasing the16-buffer device contract.
Renderer231 tests PASS/1 ignored, capture77 PASS/1 ignored, pagefixture4 PASS.
Real GPU `paged-foliage-fixture-256` PASS: alpha/depth/open gaps, cold missing,
approach/orbit/four-view cache. `reload-paged` PASS for transactional shader
reload after the new scene bindings.

Actual F12 `noon-paged-256` has151 requested/resident pages,0missing,0debug
magenta. Problem pixel1152,162 changed14/16/5 ->115/120/91 versus independent
local-source-shadow reference118/124/94 (`noon-source-oracle-b`). Pixel537,183
is truly blocked in both paged/source oracle-a, not a reason to brighten all
crowns. Near1459,205 differs by1byte. Sunset `rocks-paged-256` approaches
0/20/40/60m all retain154/154pages,0missing after warmup. All under
`C:/dev/perf/warp-sideways-20261024/r25-world-visibility/`.

This is a playable direct-lighting improvement, NOT whole-forest optical or
4K/four-player120FPS acceptance. Restpose/wind differences, coarse page
sampling/self-shadow, cold update costs, four separated full-stage cameras,
remaining indirect-canopy darkness and missing live GI remain open. Source
page requests use conservative CPU bounds; future hierarchy/priority and
per-page owner compaction are in the roadmap. Preserve all dirty work.

### Earlier r24 replay and forest plan — 24 October, 17:36 UTC

User requested an immediate playtest while the new lighting was still under
validation. The previously accepted r24 executable ran as PID235264;
it closed normally with window_closed after7896 rendered frames. Manifest:
`build/dev-runs/20261024T173645295Z-replay-stable/iteration.json`.
SHA256 `460d9fc02ea4e421ef1b4687537e114ec34a04162e617af09a38ab258aa1b12c`.
One player, fullscreen 3840x2160 output, DLSS Quality, wind0.6. This was explicitly
NOT a new forest-shadow/GI playtest; the newer run is above.

The living [forest rendering roadmap](docs/forest-rendering-roadmap.md) tracks
current/prototype/missing techniques, M0-M6 dependencies and acceptance. The
user asked whether Decima is a better reference: take Guerrilla's published
GPU placement and visibility/deferred shading ideas, retain our procedural
triangle/voxel source and branch poses. Claims of zero memory/universal speed
or unverified current Decima details are not accepted engineering facts.

New shared static foliage sun pages are integrated behind
`KERN_PAGED_FOLIAGE_SUN=1`: 64m pages, 512 squared including guards, 256 pages,
8 updates/frame. CPU renderer229 PASS/1 ignored; capture replay77 PASS/1 ignored;
page fixture3 CPU tests PASS. Real GPU fixture
`C:/dev/perf/warp-sideways-20261024/r25-world-visibility/paged-foliage-fixture-b/`
PASS: alpha gaps/depth, deferred missing, approach/orbit, four-view reuse.
The first fixture's diagnostic colour oracle failed to recognise tonemapped
magenta; corrected without relaxing missing-page assertions; failed evidence
is preserved in fixture-a.

Actual noon F12 replay `noon-paged-a` failed BEFORE any rendered image: the
grass transition selection pipeline inherited17 storage bindings, device
limit16. Its narrowed selection layout and subsequent actual replays are
described above. Full forest optical acceptance is still pending. Future tests must
check page misses/256-page capacity, source-alpha self-shadow, fixed restpose
versus wind, and steady/cold cost. No full VSM/GPU feedback or Lumen claimed.

World terrain visibility CPU/GPU and actual cliff approach are validated
opt-in (`KERN_WORLD_TERRAIN_VISIBILITY=1`); radiance-cache CPU/GPU sampling
fixtures PASS but live GI remains heuristic. See the roadmap evidence and
`docs/live-terrain-bounce-plan.md`. All work remains local, dirty; preserve it.

### In progress — world visibility and Lumen-inspired lighting, 24 October

The user explicitly chose virtual shadows / distance-field techniques and then
asked to study and emulate Lumen, accepting a longer architectural iteration.
The original r24 playtest PID192948 was stopped for GPU diagnostics. A later
restart of that same binary is described above; the following section is history.

Root owns runtime integration. Agent work produced bounded hierarchical terrain
ray queries, world radiance-cache fixtures, capture replay and shader reload. No new
Godot comparison. Preserve all existing dirty work; no commit/push this turn.

The large static far-shadow-map experiment is REJECTED and remains OFF
(`KERN_FAR_SUN_SHADOW=0`). It corrected some distant terrain lighting but
introduced ground self-shadow artifacts and failed the open-gap GPU fixture.
Evidence is in C:/dev/perf/warp-sideways-20261024/r24-forest/. Do not promote it
or claim the distant forest / sunset cliff issue solved.

Actual regression captures: sideways-1792855853301-192948-0 (noon dark distant
crowns), sideways-1792856050487-192948-1 (sunset cliff becomes dark on approach).
The latter has camera solar visibility0 already: surface sunlight and camera
fog glare are distinct bugs. Existing distant terrain march only reaches
48.16m; it cannot establish whether a farther mountain blocks the sun.

New direction: camera-independent geometry visibility, explicit hit/clear/
unknown semantics, then real material/light transport in a shared world cache.
Terrain heightfields are the first backend, not a claim of completed mesh SDFs,
virtual shadow maps or Lumen. See docs/world-lighting.md for scope and acceptance.

### Previous playtest — 24 October, r24 camera sun visibility and rolling mountains

**Historical PID 192948, stopped**, first visible frame was confirmed.
Manifest `build/dev-runs/20261024T152921929Z-05fc671e/iteration.json`.
Executable SHA256 `460d9fc02ea4e421ef1b4687537e114ec34a04162e617af09a38ab258aa1b12c`.
Release build45.76s/startup17.81s; local PR3 b761c05 plus preserved dirty work.
No commit or push. One player, 4K output, DLSS Quality, wind0.6. Same accepted
r23 flags below, plus `KERN_CAMERA_SUN_OCCLUSION=1` (new default). All rejected
r23 shadow/geometry experiments and both canopy experiments remain OFF.
PID99004 had already exited before GPU diagnostics; no live game was interrupted.
Do not run GPU diagnostics while the new playtest is active.

The user reported sunlight glow through terrain. Root fixed the direct solar
term of shared fog scattering: allocation-free per-camera heightfield horizon,
triangle-exact DDA, analytic partial sun-disc coverage, rotation-independent.
Only direct solar haze is gated; diffuse haze/extinction/material light remain.
No extra GPU pass. See `docs/camera-sun-fog.md` for limits and test evidence.
Orange F12 `sideways-1792853911138-99004-1` is visibly corrected. White afternoon
F12 `sideways-1792854017421-99004-2` is unchanged: sun28deg clears the sampled
terrain horizon there. Do NOT claim both images were changed or that this
solves full along-ray volumetric occlusion, foliage/cars or surface shadows.
191 renderer tests PASS/1 ignored; real GPU blocked/rotated/exposed/split tests
PASS; exposed images byte-identical ON/OFF. Native replays omit cars/HUD.
Evidence: `C:/dev/perf/warp-sideways-20261024/r24-sun-fog/`.

Background mountains now have512m foothills (was128m), broader rounded ridges,
same source terrain triangles and the same measured299 trees/ha eligible land.
273993 distant trees vs256417 for captured stage (+6.85%), below320k cap.
Steep excluded far land0.42% vs6.81%; native1.32%. High peaks215m retained.
13 CPU tests PASS; exact captured camera reviewed in
`r24-mountains/replay-after/lit.png`: bare pyramid replaced by wooded rolling
hill. This is still native source tree CLOD, not a newly completed voxel forest.
Extra tree cost has not been accepted against 4K/four-player budget.
See `docs/distant-forest.md`.

The requested real-vs-fake forest comparison is complete but **NOT fixed**.
`r24-forest/legacy-00`, `compact-01`, `bounds-10` and `visual-review.json` use
24 identical source trees, fixed camera and canopy anchors. The diagnostic-only
`set_sun_shadow_focus` moves just the sun maps, not GI. Real/source vs real/CLOD
ground sun images match exactly; approximate/source vs approximate/CLOD match
exactly; sky visibility is unchanged. Current approximation shadows425/1215
ground samples vs102 with real maps. Compact-only401, bounds-only480: neither
is acceptable and source bounds darkens more. No candidate promoted.
CPU source coverage audit also identifies directional gap and scale-law errors
in the ellipsoids: `r23-tree-root-audit/crown-coverage-README.md`.
Next: source-derived directional coverage/transmittance, not arbitrary random
holes or one global density multiplier. Existing foliage_shadow.rs remains a
CPU prototype; actual voxel visibility/streaming/wind integration is unfinished.
That earlier delegation completed; newer active tasks are listed above.

### Previous playtest — 24 October, r23 material variation and caster depth

**Historical PID 99004, now exited**, first visible frame was confirmed. Manifest:
`build/dev-runs/20261024T145259334Z-5ee7d383/iteration.json`.
Executable SHA256 `bd38139550cbc7c5fc598555425092a9b6a692546ea60f51c86e6268d16540ef`.
Source remains local PR3 `b761c05` plus preserved dirty graphics work; no push.
One player, 3840x2160 output, DLSS Quality, wind0.6, normal/extended grass,
far grass and wheel effects ON, procedural grass and crowds OFF.
Cached launch build0.90s/startup20.37s. User already captured
`sideways-1792853644814-99004-0.png` in this run. Do not run GPU diagnostics
concurrently or interrupt this playtest for further investigation.

Enabled for this playtest: `KERN_ROCK_VARIATION=1` and
`KERN_SUN_SCENE_DEPTH=1`. The same rock asset now varies its mapping in world
space, including consistent normal-map transforms. It reduces recognisable
repetition; it does NOT provide the missing large-scale cliff geometry.
Sun maps now cover the loaded scene's depth instead of clipping casters
399m sunward of the focus. A real 600m blocker GPU fixture passes with these
exact playtest flags, and matched daytime/sunrise rock captures were reviewed.

**Rejected for this playtest:** `KERN_SUN_RECEIVER_PLANE=0`,
`SIDEWAYS_ADAPTIVE_CLIFF=0`, `KERN_SUN_TERRAIN_ERROR=0`,
`KERN_SUN_SURFACE_SOURCE=0`. Candidate implementations remain behind these
explicit flags. Do NOT call the orange tree roots, near/far sun transitions,
or full rock-depth work fixed. Receiver-plane comparison removes the captured
near orange root leak, but introduces dark/noisy cliff seams; triangle-domain
clipping passes a real V-crease GPU test yet does not fix the actual wall.
Source-only surface casters remove large false road polygons; simply reducing
terrain error to1.5625cm does not remove the far tree-root leak. Do not promote
these experiments on the strength of a synthetic fixture alone.

Evidence: `C:/dev/perf/warp-sideways-20261024/r23-lighting/`, including
`rock-playtest/lit.png`, `wall-playtest/lit.png`, `sun-fixture-playtest/report.json`,
and rejected `rock-clipped`. The native1920x1080 replays omit cars/HUD and
reconstruct the recorded stage/camera/light, not original DLSS pixels.
No new gameplay FPS or 4K/four-player performance acceptance claim.
Full earlier render181 and CLI117 tests passed (one ignored each); after the
last diagnostic edits, composed shader checks and focused plane/geometry
regressions passed, plus the final GPU fixtures and release-DLSS executable.
See `docs/sun-shadow-diagnostics.md` for limitations and exact causal findings.

Latest user report: some tree bases were intensely black/untextured, possibly
geometry. The original orange root and adjacent dark bark hit the same valid
outward-facing triangle; that does not rule out a separate black-base defect.
CPU-only asset/UV/winding audit is assigned to road_terrain_seam while the
user plays. Preserve the active game and return the evidence honestly.

For genuinely larger rock forms, change the shared Stage cut profile with
bounded, coherent buttresses/recesses outside the protected road/shoulder,
so render basis, heightfield and collision agree. Raising only visual Y-offset
is insufficient on nearly vertical walls and breaks the lighting bounds.

### Local PR #3 merge — 24 October

User requested a local merge only. `main` fast-forwarded from `09500d2` to
`b761c05` (idle ringing, quieter idle, granular tyre sound). No remote push
or GitHub merge. All pre-existing graphics changes and staging state were
restored; 136 non-overlapping files are byte-identical, and the overlapping
README gained only the PR's audio paragraph. Safety stash retained:
`f022a4c8099d77627c5962b627a039552165c73e`.

Validation: audio 8 passed; release CLI with DLSS 115 passed / 1 ignored;
the tyre probe completed, including silent standing wheels. Evidence:
`C:/dev/perf/warp-sideways-20261024/pr3-local-merge/`.
After the user's "doe", rebuilt and started the PR #3 version. Current live
playtest PID **134304** (historical, closed for r23 diagnostics), first visible frame was confirmed;
`build/dev-runs/20261024T134441628Z-e7be65c0/iteration.json`.
Source `b761c05` plus preserved graphics; executable SHA256
`8475f49557674b45057d73851fa16dbd1358b9f723ce944b30f0679a0cbec847`.
Build40.83s/startup12.38s. One player, 4K output, DLSS Quality, wind0.6,
wheel effects ON. Audio started at48kHz; XInput reports one connected
controller. This replaces the r22 PID below. No new GPU tests were run.

### Current iteration — 24 October, r22 wheel tracks and ejecta

User requested persistent tyre paths plus surface/slip-driven dust, gravel,
soil and grass pieces. Implemented the existing fixed-tick wheel emitter in
the actual play/shot renderer, plus a shared bounded material atlas for tyre
marks. Each wheel uses contact/load/slip and COM point velocity; water is
queried per contact. Pauses, recovery and stage changes are covered. Physics
and race RNG are unchanged. Preserve the r21 cliff-normal correction below.

Release candidate SHA256
`9e554e2ba35e89f614036a85f21400d4175f64f3426d9b1fee6d575a01611087`,
`C:/dev/perf/warp-sideways-20261024/r22/sideways-candidate.exe`.
r21 PID89152 was identity-checked and closed for exclusive GPU tests.
**Live r22:** PID **267092**, first visible frame confirmed and responding.
Manifest `build/dev-runs/20261024T124338269Z-713f0758/iteration.json`;
executable hash matches the validated candidate above. One player, 4K output,
DLSS Quality, wind0.6, wheel effects ON. Cached build1.81s /startup33.29s.
Controller inputs were not independently exercised on this restart.
No concurrent GPU checks while this playtest runs; use active.json for ownership.

Validation: CLI115/render172/sideways30 passed, one ignored per suite.
Both real GPU fixtures PASS: `r22/track-gpu/report.json` and
`r22/particle-gpu/report.json` (including DLSS-history parity). Exact actual
on/off gameplay pair `trail-front-on/off.png` shows subtle compressed tyre
paths and dust behind the cars; first rear camera hid most of the trail.
Native4K/four-player on/off pair also succeeds, 11,499 stamps /31 tiles /
465 live births with no queue loss. Four-player raster still ~20.7ms and
shade10.7/10.9ms off/on, so no120FPS or general performance acceptance claim.

Tracks: 512 fixed world tiles, 8cm texels, 33MiB GPU including identity lookup;
LRU eviction clears identities, height guard rejects upper bridges/cliffs.
Material compaction/rubber is applied before lighting. No geometry/grip
deformation or flattening of live grass. Particle births4096 shared, visible
per-view2048 plus fill cap, seven kinds, GPU analytic motion, sun shadows and
opaque depth, post-temporal composition reuses exhaust output. No local
headlight illumination of particles, dust self-shadowing or full collision.
See docs/wheel-effects.md, persistent-tyre-marks.md and wheel-particles.md.

Default effects ON; SIDEWAYS_WHEEL_EFFECTS=0 provides a matched diagnostic.
Procedural grass experiment remains OFF; normal grass/far grass ON, crowds
OFF. Nanite-style shared foliage work remains in the queue after this user
request; do not silently replace it with the new wheel effects work.

### Current iteration — 24 October, r21 rock-wall black triangles

**Historical r21 (closed):** PID **89152**, first visible frame was confirmed.
Manifest `build/dev-runs/20261024T121150316Z-bd7376ab/iteration.json`;
SHA256 `dcc36f1f392c8099e222325cddab6946108f3c4ee47a067b7cee5ab07ca2a36b`.
4K output / one player / DLSS Quality / wind 0.6. Cached build 1.29s,
startup 16.65s. Same executable as the r21 validated candidate. The earlier
r20 PID 114080 was identity-checked and closed before exclusive GPU testing.
Use active.json for current ownership; historical PIDs below are not live.
No concurrent GPU tests while the playtest runs. Controller input was not
independently exercised on this restart. Dirty-checkout build, no new commit.

The reported black pits in the geological cliff were caused by shading normals
derived from a different, unclamped relief field than the actual triangles.
Some pointed into the visible surface and made skylight probes self-occlude.
Normals now follow the final geometry, with shared area-weighted normals across
chunk boundaries and outward-facing constraints. Positions, UVs, indices,
collision and protected road normals are unchanged. No material darkening,
ambient floor, heightfield bias or shader workaround was introduced.

Evidence `C:/dev/perf/warp-sideways-20261024/r21`: matched native
`wall-after.png` and `wall-after-sky.png` remove the black pits; actual car
`rear-dlss-after.png` was also reviewed. Source and selected-CLOD audits have
zero reversed shading normals in the captured region. Full CLI suite:
110 passed, one ignored. Renderer unchanged since r20 (163 passed, one ignored).
The common normal pass cost 279ms during this stage's construction, with no
new per-frame GPU pass. See `docs/terrain-surface-normals.md` for bounded
coverage, retained geometry and the four offscreen vertical CLOD slivers.
This does not certify every rock shape/LOD or the separate masonry guardrail.

Foliage and exhaust work from r20 remains unchanged. Procedural grass experiment
OFF; regular extended and far grass ON; crowds OFF. Continue shared voxel
foliage optimization from the r20 notes below after this focused defect repair.

### Current iteration — 24 October, r20 foliage and exhaust

The user has resumed the shared procedural/voxel foliage work and explicitly
requested a subagent for exhaust gas, smoke, brief backfire and hot-air image
refraction. Root owns renderer selection/bindings; kern_grass_port owns the
playable procedural grass attachment; road_terrain_seam owns representation
validation and same-renderer multi-view fixtures; rock_terrain owns exhaust.
Do not replace these with further unused prototype modules.

**Historical r20 (closed):** PID **114080**, first visible frame was confirmed.
Manifest `build/dev-runs/20261024T113402278Z-5daa1a11/iteration.json`;
SHA256 `41d0ab51987674d48b73842fb91823b49778ea612cb691ea7f15acc9dadb0fc7`.
4K output / one player / DLSS Quality / wind0.6; 120Hz physics / four substeps.
Build59.13s, startup17.97s. Exhaust enabled. Procedural grass experiment OFF,
regular extended grass and far statistical grass ON. Crowds OFF. No concurrent
GPU tests while this game runs. Controller inputs in this restart have not
been independently exercised. This is a dirty-checkout build, not a new commit.

The previous live playtest was r19: PID **265232**, first visible frame confirmed,
manifest `build/dev-runs/20261024T105250242Z-d4e37a8a/iteration.json`, executable
SHA256 `7c68dabdb95fade012482a7dbbd0d9dc1081c607b61140ed3b77238eae4f08f3`.
4K output, one player, DLSS Quality. PR1 engine audio merged remotely and locally
as `09500d264ca80c4962d833388b06ad8f24ccca90`; the graphics checkout is dirty.
It was identity-checked and closed for exclusive r20 GPU validation. Do not
mistake this historical PID for a running game; use `build/dev-runs/active.json`.
Keep safety stashes `7fed750c0d003428ae18ee48657dd662489b18cb` and
`d7792e253211afc1d5e773f08889d4333bb2d7e2`. No concurrent GPU tests while it runs.

r19 evidence is in `C:/dev/perf/warp-sideways-20261024/r19`: exact morning-rock
replay removed the false black faces caused by a coarse heightfield occluding
its own refined surface; camera-yaw pair checked physical receiver anchoring.
These do not close all low-sun or terrain artefacts. Native radius is now 1152m,
with 256417 far and 45186 native trees in the initial stage; no free-cost claim.
The real next-stage test displayed 644 loading frames while simulation paused,
then resumed round 2 without catch-up: 12.69s total, including 0.92s race/colliders,
7.56s landscape and 3.46s graphics preparation. Audio opened at 48kHz again.
Render tests: 157 passed, one ignored; CLI: 97 passed, one ignored; audio: 8.

Compact canopy support remains OFF: the held-frame noon comparison raised
GPU time from 8.87 to 18.65ms while leaving the halo unresolved. Rear lamps
have CPU checks but visual acceptance is open: `rear-night.png` accidentally
faces the FRONT of the car and cannot validate rear lenses. Volumetric beams,
local indirect bounce and road aggregate depth remain unfinished.

r20: reduced voxel lighting layout removes unused mesh/car bindings
(17 storage bindings formerly exceeded the requested 16). Per-view 4KiB
selection uniforms connect the same source branch and voxel to actual raster,
culling and volume integration. Output-pixel bounds, independent view history,
stable branch thresholds and source shadows are retained. No new pixel dither.
The playable grass experiment is a bounded fixed patch, not whole-world
replacement, and remains native-only until fractional-volume motion is solved.
Default grass is not replaced by this experimental patch.

r20 evidence: `C:/dev/perf/warp-sideways-20261024/r20`. Final render library
tests: **163 passed, one ignored**; CLI **109 passed, one ignored**. Release CLI, foliage fixture and exhaust
fixture built. `foliage-runtime/report.json` and
`foliage-runtime-active/report.json` both pass: one persistent renderer switches
near/far/near and four independent views with exact selected-endpoint images,
58 groups / 70 poses, source and volume pose oracle within 4.8e-7. Source
shadow geometry remains. This is ownership/pose acceptance, NOT source-versus-
voxel optical parity. The far fixture covers only a few dozen pixels.

Active tile scheduling removes almost all empty-view integration overhead
(near runtime 3.05 to 0.0075 ms in this fixture), but the far 26-ray case still
costs 3.17 ms and four-view integration 6.87 ms. These single fixture samples
are not gameplay FPS or performance acceptance. Do not enable globally.
The fixture forces finest mip0. The active dispatch itself is bounded correctly;
read-only follow-up identified expensive six-lobe BRDF/environment evaluation
inside every fine step and 2.25KiB of dynamically indexed private ray data per
lane. Register spill is only a hypothesis, not profiler evidence. Next bounded
experiment: bypass zero-weight sky lookups at roughness>=0.55, pair equivalent
folded normal lobes, record already-existing step/cache counters, and compare
with the unchanged endpoint and pose oracles. No such optimization is delivered.

The actual seed1/3km stage accepted the bounded patch: 34 plants, 12 shared
assets, 170 poses, 136 volume owners, 723528 packed bytes. Evidence
`grass-stage-auto.png.json`; fixed cells [-177,-723]..[-169,-717]. The default
shot starts at 300m, and this initial patch is mostly outside its frustum;
this capture proves scene integration, not near/far appearance.
`grass-stage-close.png` is a rejected diagnostic camera inside the hillside.
`grass-stage-close-opposite.png` actually shows the same procedural source
clump beside the rock wall at --at364; the coarse source/voxel transition's
appearance is still not accepted. Do not use the rejected image as evidence.

Exhaust is integrated into play and shot, with measured pipe mounts, bounded
smoke, 16.7ms backfire and real background refraction. Native fixture passes
heat/smoke/flame, full opaque blocker, independent four views and unchanged
TAA history. Two actual DLSS car shots render successfully; held-frame images
do not establish moving appearance or cost. See `docs/exhaust-effects.md`.
No volumetric smoke shadows or projected flame light are claimed.

### Delivery queue — 24 October, after live sky/menu request

**A CPU prototype is not a delivered feature.** Close an item only after it is
present in an identified playable build and its real rendered result is checked.
The user explicitly reported that crowds and road depth were being left unfinished.
Keep new requests in this queue rather than silently replacing earlier work.

| Item | Actual status | Next acceptance step |
| --- | --- | --- |
| Car headlights at night | r18 live: two measured physical lamp origins per car, grass receiver fix, eight-light/four-view real GPU fixture PASS | Moving night DLSS and four-player cost acceptance remain; no grass cast shadows or volumetric beams yet |
| Sunrise light disappears on approach | Open: low-sun near/far shadow coverage and short canopy fallback under investigation | Capture matching near/far positions, isolate map/detail/fallback contributions; do not claim fixed by the road normal correction |
| Volumetric fog and indirect lighting | User requested after first starting PR2 playtest. Current fog is analytic height fog; SH9 is global sky, not local bounced-light cache | Low-resolution per-view froxels with shadowed sun/headlamps, followed by shared world-space diffuse irradiance cache. Measure four-view total; no speed claim yet |
| Live procedural sky/time menu | Delivered in PID 269560. In-place GPU update passes; actual menu observed and sunset revision 2 applied in live F12 | Complete atmosphere switching / paused-position matrix later; no clouds/rain yet |
| Roadside crowds | User rejected prototype appearance; disabled in current playtest (`SIDEWAYS_CROWDS=0`) | Deferred by user to a future update; do not re-enable as part of graphics iteration |
| Unified rock wall | Delivered in PID 269560; separate overlays removed; matched actual #1/#4 renders reviewed | One consistent surface now, but further shape/lighting refinement remains |
| Black triangles in geological cliff | r21: reconstructed normals from final geometry; matched native and DLSS captures reviewed; live PID 89152 | Reported captured pits fixed; other angular rock features and distant LODs remain separate |
| Terrain pyramid/straight crest | River-bank source correction delivered in PID 269560; matched #4 render reviewed | Left toe/right rail grid artefacts remain open; not all terrain faceting solved |
| Black stone guardrail triangles | New screenshot captured; lighting/normal diagnosis active | Prove source cause and inspect corrected same-location render |
| Road surface depth/aggregate | Existing shallow geometric ruts/ridges; detailed gravel is largely texture/normal detail | Audit and deliver visible near-road stones and convincing surface depth; not complete |
| Distant forest shadow continuity | Ellipsoid fallback still loses real gaps. Source coverage tile is CPU prototype only; source crown bounds opt-in/off | Connect source-derived visibility/depth to live rendering and compare user's near/far pair |
| Unified procedural/voxel grass and branch wind | Source/voxel optics and animation prototypes exist; not yet accepted as live grass replacement | GPU optics, motion and cost acceptance before replacing current reference grass |

The immediate build contains only features that pass their actual render checks.
No further Godot comparison is requested. The 4K-output / four-player / 120 FPS
target remains unmet/unproven by this work; prototype counts are not FPS evidence.

### Current playable build — 24 October, 11:54 Amsterdam

Owned live process **231924**, visible first frame confirmed and process responding.
Controller operation in this specific restart has not been independently tested.
Manifest `build/dev-runs/20261024T095403418Z-c35d30f1/iteration.json`;
staged executable in the same directory. SHA256
`799600998ca262645e0f4806d0b06d2c0409ae70525d431b873029722e187cb7`.
4K output / one view / DLSS Quality; 120 Hz physics and four substeps retained.
Source is the dirty checkout on merged PR2 commit
`0f4baa8652cbd081a6c6810033cc466325c6f6b3`, not an immutable source snapshot.
Final release compilation took 78 s; launch's cached build took 1.62 s and visible
startup 20.08 s. Prior PID 219656 was deliberately closed before exclusive r18
GPU validation. PR2 is merged remotely; local work was preserved through safety stash
`7fed750c0d003428ae18ee48657dd662489b18cb` and reapplied. Keep that stash.
All eight merge conflicts resolved; 149 render library tests passed, one ignored;
four car-lamp, four capture-replay and four final composed-shader checks passed.

Enabled: PR2 procedural car materials, procedural sky, unified rock, river-bank
correction, finite height fog, local terrain sun occlusion, and night headlights.
Crowds remain disabled at the user's explicit request; preserve that setting.
Actual r17 haze/mist GPU images no longer show the former horizon stripe, but
mist still has no volumetric shadowing. r18 same-location road/grass renders and
a DLSS night shot were visually reviewed; moving night acceptance remains open.
Local car material binding is 19;
car diagnostics are 20/21/22, preserving renderer diagnostics 8..14.
Reference extended grass remains; new shared voxel grass and optional source
crown bounds are **not** promoted. No new road-depth implementation in this build.
The user is actively driving now: do not run concurrent GPU tests or take over
the controller/menu. Start/Esc opens the time/atmosphere menu; F12 captures state.

PR2 integration follow-ups: optional voxel scene layout now totals 17 storage
buffers against the requested device limit of 16; fix before enabling that path.
Hot reload updates opaque/transparency but not grass/far/voxel helpers and is not
transactional across the two passes. Use a full restart after shared-lighting
shader changes. No full-Nanite, four-player performance or night-quality claim.

Road stripe correction (r18): explicit `NormalEncoding::SrgbTransferred` only
for gravel decodes normal RGB before mips; other maps remain linear. Actual
matched-camera `r17/afternoon-road/lit.png` versus `r18/user-road/lit.png` shows
the three black bands removed. Prior shadow/albedo diagnostics were uniform,
but mapped normals were striped: this was an import encoding error, not a
reason to change ambient light or roughness globally.

Headlight grass correction: all three reference-grass paths share inclined-blade
light response rather than rejecting a light below the fragment via world-UP
NdotL. Actual `r18/user-grass-night/lit.png` replays F12
`sideways-1792834206688-219656-0` with its original lamp poses held fixed. Native
diagnostic view omits cars/HUD, so it is not pixel-parity with the DLSS capture.
New paired mounts were also viewed in `r18/night-dlss.png` (held frame, 1920x1080
output / 1280x720 input). No moving/4K/four-player performance inference.

`r18/headlights-final/report.json` passes: off/blocked/open RGB 0/0/56.6,
grass above lamp 196.32, eighth light alone 4.4, and zero different pixels in
all four views against the eight-light single-view reference. This exposed and
fixed a real pass-classification bug: split view marker 1 was mistaken for a
perspective shadow pass, removing NO_SHADOW grass. Headlight shadow uses marker
2 now; main-view and culling predicates distinguish both. Production scene cost
remains unmeasured. All evidence is under `C:/dev/perf/warp-sideways-20261024`.

Sunrise follow-up (read-only source audit, still OPEN): `shadow_at` blends to
`transmittance(..., 50)` at shared-map boundaries. Its 12 expanding samples end
at 48.1683 m, and the last untrimmed integration step reaches 69.844 m. Increasing
max_t alone does not add samples. At a 2-degree sun a 20 m crown can cast a
573 m ground shadow; the canopy layout only reserves a 60 m ray guard beyond
the receiver domain. These are known approximation/coverage limits, not proof
of the user's particular near-distance switch. Both sun maps use depth range
-500..+399 m toward the sun relative to their distinct centres. Near/far map
resolution, caster LOD and normal bias also differ. The latest saved low-sun
F12s are sunset (2 degrees / azimuth 270), not sunrise (azimuth 75).
Next exclusive-GPU diagnosis: same captured camera with debug=9; debug=9 and
near_shadows=0; then also gi=0. Inspect mapped/source NdotL (13/14) if unchanged.
Compare the same world receiver from two camera distances before changing
production defaults. Do not interrupt the now-live playtest just for speculative
distance tuning, and do not report sunrise fixed by the normal-map change.
Independent bias audit: far/near texels are 4.6875/1.5625 cm and receiver normal
offsets 7.03125/2.34375 cm. On a horizontal plane with 2-degree, azimuth270 sun,
receiver plus slope-scale raster bias corresponds to approximately 4.746/1.582 m
along the light ray (excluding constant depth bias). This can erase distant
contact shadows and restore them near, but is not yet a measured diagnosis of
the user's location. Saved low-sun camera targets are within far receiver
coverage, so the edge fallback alone does not explain those exact target points.
Include separate bias and caster-depth ablations in the exclusive-GPU matrix.

Evidence under `C:/dev/perf/warp-sideways-20261024/r16`:
- `sky-live-final`: one actual renderer updated morning/noon/sunset/night/noon;
  return-to-noon pixels equal, invalid update preserves output, four small views
  rendered. This is functional validation, not four-player performance evidence.
- `crowds-gpu-final`: 20 real images across native 1920x1080 and DLSS Quality
  1280x720 input; all 1,215 fixture part poses/feet match CPU reference. Characters
  visibly have prototype faceting and separated joints; no polished-art claim.
- `replay-final-1`, `replay-final-4`, `replay-final-5`: strict matching stage seeds,
  captured camera, actual lit/albedo/direct/indirect frames. No cars/HUD; native
  diagnostic images are not route performance or DLSS parity claims.
- Live `build/screenshots/sideways-1792830832981-269560-0.json`: user already
  applied sunset, revision 2, clear, elevation 2/azimuth270, pending false,
  no error, no new race round. Paused position preservation has CPU coverage;
  a controlled before/after paused live capture pair is still pending.

Black guardrail remains open. Actual #5 replay has intact nonzero albedo on
black pixels, direct=0 on both black and control regions, but indirect=0 only
in black regions. Same owner/cluster/material; next investigate five-cone
terrain sky visibility and receiver-modulated bounce. Do not mask with an
unproven material/normal/ambient-floor change.

Black-wall follow-up: F12 `sideways-1792829525441-102808-5` is round 4,
stage noise/detail `1049722424 / -85257117`. The CPU-only
`wall_lighting_audit` reproduced that stage and found all 1,150 source wall
triangles selected at its camera; Geometry and TexturedGeometry cuts match.
Across 148 sampled black pixels, source normals are finite (length >=0.9989)
and point toward the sun (dot approximately 0.89–0.92). A wall LOD/normal
patch is therefore not justified by this evidence. The scene wall has no
SOLID contact proxy. Evidence: `C:/dev/perf/warp-sideways-20261024/r16/wall-lighting-audit.json`.
`capture_replay` now includes exact black/control pixel picks and lit, albedo,
direct and indirect views to distinguish the actual frontmost surface and
light attenuation from composition. The black-wall bug remains open; do not
hold the requested playtest for speculative material changes.

#### Road surface depth — audited, next playable iteration still pending

This is a source audit, not a new rendered feature or a performance result.
`sideways::builder::road` already supplies the driving mesh and collider from
23 across-road samples at 2 m longitudinal spacing. Its coefficients are
4.5 cm rut depth, 2.5 cm centre ridge and 4 cm windrow, modulated by wear and
wandering track centres. Sampling makes the actual relief smaller: the
windrow peak at across=0.8 lies between samples 0.72 and 0.86, so that term
reaches only 2.28 cm at the source vertices. `visuals::road_mesh` adds no
small aggregate geometry and uses the positional-only `Geometry` CLOD
policy; preservation of these shallow grooves at selected LOD is unmeasured.
The ROAD shader uses real texture normal maps but its rut mask stays at
abs(across)=0.25 instead of following the builder's wander and wear. Thus
dark painted tracks and the geometric depressions are not exactly aligned.
There is currently no individual road-stone geometry or dynamic tyre-track
accumulation in this renderer. The separate cliff work does not implement it.

The user's road proposal is design input, not verified evidence of how DiRT
or WRC implements these effects. The first bounded deliverable starts with
one typed road profile: per-row track centres/wander, wear, rut depth and
ridge/windrow parameters shared by source geometry, collision and shading.
Preserve current driving heights initially; make the shader read the same
profile rather than approximate it with an unrelated fixed dark stripe.
Actual deeper deformation is a separate intentional handling change requiring
matching collision/tests. Verify the CLOD-selected surface preserves the
important groove shape before changing tessellation or shader complexity.

Then add a seeded road-aggregate builder using a small shared library of
closed, irregular stone templates, baked into short CLOD tiles on the exact
existing road triangles. First candidate budget: 8 m tiles,
up to 48 stones/tile, 12–20 triangles/stone; approximately 72–120k source
triangles/km before the existing CLOD selection, and 125 tile instances/km.
Those are proposed limits, not measured GPU costs. Template topology is
reused during generation; transformed tile vertices consume unique storage.
Use 3–10 cm diameters, mostly embedded with <=2 cm protrusion, seeded visual
RNG, and fewer stones in the worn grooves/more in the windrows. This preserves
the existing driving mesh/collider and avoids stones floating on the analytic
height field. Do not silently enlarge physical obstacles or add large
per-stone GI proxy volumes. Ordinary raster/shadow passes supply actual
silhouette/depth; visibility of their small shadows still needs GPU proof.

Later, if these two improvements still leave a clear gap, evaluate a bounded
world-space deformation cache driven by authoritative wheel contact/load/slip
signals. Toroidal storage must track its world origin and clear newly exposed
strips on integer-cell scroll; modulo UVs alone alias old tracks into new
ground. Four separated players need coordinated windows or shared sparse
world tiles, with wheel stamps applied once per simulation update rather
than once per camera. Make eviction/loss of old visual tracks explicit.
A proposed 2048-square, 40 m window has ~1.95 cm texels and costs 8 MiB in
R16F or 16 MiB in RG16F; four independent windows cost 32/64 MiB before mips,
temporary copies or other attachments. These are storage arithmetic, not a
choice of default or evidence of runtime affordability. Keep channel meanings
explicit: displacement, raised edges and wetness cannot silently share one
quantity or refer to a nonexistent B channel in an RG texture.

Do not read GPU deformation back into the 120 Hz driving simulation. Initially
it is bounded visual detail; any later physical rut tracking needs a separate
deterministic CPU-authoritative representation and matching collision design.
For microdetail, limited near-view POM/height blending is an optional measured
experiment after the real geometry is accepted. Bound its screen coverage,
step count and distance/angle transition, with a stable normal-map fallback.
POM is not free, does not provide actual silhouette/collision, and its local
self-shadow approximation does not replace ordinary scene shadowing. Keep
height/albedo/normal/roughness sampling coherent when rotating/blending tiles.
Avoid camera-dependent high-frequency sparkle: the user already reported
noisy, shimmering ground. Wetness must respond to material and lighting rather
than forcing every track to near-mirror roughness or a world-up normal.

Check deterministic seeds, exact triangle support, tile bounds/winding and
selected-CLOD profile error on CPU. Acceptance requires close oblique and
driving-camera native/DLSS images, low-sun lighting, moving LOD continuity,
and four-view GPU cost at 4K output against the total 8.33 ms frame target.
No per-view duplicate world updates or unmeasured "cheap" claims. Do not mark
this done from CPU checks or source counts. Ship the already-ready
sky/rock/crowd work first; this road iteration must not delay that playtest.

### 24 October, current playtest and pending r16 work

- Joep authorized restarting when updates were ready. Managed PID **102808**
  (`20261024T073758926Z-bdb57264`) now runs the new cliff/road-material iteration.
  Release build took 88.656 s; visible first frame and actual window image were
  confirmed. Xbox controller detected; user was already driving at capture.
  Executable SHA256: `b157001d7e4ffdfde60ef61f53cb57e4e16686173a21fb85f7208e7cbde2e3f1`.
  One player, 3840x2160 output, DLSS Quality, clear, summer, wind 0.6,
  extended reference grass. Crowds and procedural sky remain disabled.
  New foliage optics/dispatch are compiled but not enabled in the live world.
  Prior PID 109828 was closed by the managed launcher after successful build.
  Keep this new session running during CPU work; no concurrent GPU benchmarks.
- The new **Roadside crowds** task is assigned to the freed
  `road_terrain_seam` agent (its visible chat title was changed). Two attempts
  to spawn a fresh agent were rejected by the agent-thread limit. Its scope is
  now reusable baked GPU animation and opt-in roadside spectator groups;
  it must not resume old seam/sky work. User context is the pasted text in
  attachment `c1f75c08-c425-4878-9075-603a2a6d8e8a`.
- Root added `Scene.voxel_active_tiles` (default **false**). Tile binning
  appends nonempty tiles to a GPU worklist; indirect dispatch visits their
  pixels with the unchanged integration function. Empty pixels are cleared by
  a lightweight pass included in the tile GPU timestamp. Argument bytes are
  copied into a separate indirect buffer to avoid storage/indirect aliasing.
  Dispatch supports more than 65535 tile jobs via bounded two-dimensional
  workgroups. Nine relevant CPU/WGSL tests pass, including exact pixel coverage
  for sparse, odd-sized and multirow views. Log:
  `C:/dev/perf/warp-sideways-20261024/r16/voxel-active-tiles-tests.log`.
  **No GPU image equivalence or speedup has been measured yet.**
  The one-leaf fixture accepts `--active-tiles true|false` and saves raw
  little-endian RGBA16F volume layers for a later bit-exact comparison.

Joep explicitly wants both a better engine and its application in Sideways.
Improve APIs, performance and iteration time while using the game to expose
real needs. Current work shares typed volume-view bindings between voxel
trees and distant grass, and adds a staged Windows build/restart workflow.
F12 developer captures include same-frame scene GPU metrics. Do not claim
token savings or runtime speed gains without measurements. Shader hot reload
exists for the shading chain only (shade, car_material, transparency,
wind_common: `KERN_SHADER_DIR`, `shot --watch`, live in `play`); the other
shaders and broader frame-resource reuse are still future work.

Car materials (branch `car-materials`, README "Car materials"): procedural
paint (solid, metallic, pearl, candy), wear and grime in
`shaders/car_material.wgsl`, parameters in `data/car_materials.toml`,
reloaded live. Tuned only on lavapipe in the cloud container: check flake
sparkle, swirl arcs and the paint's brightness under TAA on the real GPU.

Joep also wants future GPU wind animation at every foliage distance, and
procedural trees parameterized by growth and season. Preserve one source of
species, seed, branch structure, rest pose and seasonal leaf coverage for near
geometry and distant representations. A shared world wind field and bounded
GPU branch deformation should animate both. Wind phase must be separate from
structural changes to a tree; do not rebuild every cached canopy volume for
every moving leaf. Evaluate conservative motion bounds, sampling a deformed
rest-density field and infrequent shadow updates against actual error/cost.
The r14 first implementation below now supplies shared GPU whole-plant wind
and stage-authored growth/season profiles. Branch structure, biological growth,
seasonal terrain, animated density and per-leaf motion remain future work.

During the 24 October playtest Joep explicitly reaffirmed one common foliage
mechanism for trees, grass and shrubs: near source geometry, derived voxel
groups selected by projected error, shared GPU wind phase/transforms, and
preserved density/color/normal distributions and sun transmittance. Current
four-bin mean grass columns are an intermediate approximation, not that final
system. Avoid building incompatible separate grass and tree animation paths.
Do not reuse the obsolete subagent task name `godot_comparison_baseline`;
its visible completion messages confused Joep despite its actual grass work.
No further Godot comparisons are wanted or being performed.

Joep explicitly stopped further investment in extending the old grass-card
rendering path on 24 October. Its current extended-distance development
profile is a reference only. Continue with shared procedural opaque plant
parts, derived voxel levels, coherent normals/materials and GPU branch poses.
Source trees remain the normal forest; do not claim the bounded voxel fixture
already replaces the whole live forest or grass field.

Current focus and missing systems are now stated in both READMEs: procedural
nature and sky, vehicle physics, AI workflows, compilation/iteration speed;
no human editor, indoor/building/people/water system or general animation
system. Plant-wind work is distinct from general animation. This direction
can change. No new Godot comparison is wanted.

## In progress: shared assemblies, fog and procedural sky (r16)

- The user's far-mountain screenshot is
  `build/screenshots/sideways-1792824184064-268264-0.png`. Its fully fogged
  mountain is flat RGB 145,148,151 while the background varies with direction.
  Materials used a constant horizon average, but the background used the HDRI.
  Shared background/fog helpers now make the full-fog endpoint identical;
  this is coherent HDRI distance fog, not physical aerial perspective.
- Camera-only culling of fully obscured bounds is being validated. Preserve
  shadow/GI/reflection casters and independent split-screen visibility. This
  reduces retained geometry, not necessarily the number of indirect draw calls.
- `foliage_assembly`, `part_wind` and `procedural_foliage_gpu` are the bounded
  next-path implementation: actual blade/needle geometry, branch-local voxels
  from that exact source, common GPU poses and previous/current history.
  Procedural materials share thin-leaf reflection/transmission, source normals
  and voxel normal distributions. A covered-area leaf fraction keeps bark from
  automatically becoming translucent after aggregation.
- Limits remain explicit: 256 volume owners, six normal lobes, approximate
  Beer medium opacity, no whole-world streaming/merged assembly hierarchy,
  no automatic source/voxel handoff, and no DLSS motion integration for these
  animated parts. Old grass cards are not being further optimized.
- Agent `sky_and_weather` owns the opt-in procedural atmosphere foundation;
  see `ATMOSPHERE.md`. Bounded CPU transmittance/sky LUTs supply shared
  direct sun, background and SH9, with cache reuse across stage rebuilds.
  `SIDEWAYS_SKY=procedural`, `SIDEWAYS_SUN_ELEVATION` and
  `SIDEWAYS_SUN_AZIMUTH` opt into shot/play. Defaults remain HDRI. Clouds,
  multiple scattering, aerial perspective and live day/night progression
  remain future work. Do not call this a complete weather system.
  Complete render CPU/WGSL validation: 111 passed, one GPU-only test ignored;
  two procedural-sky CLI/metadata tests passed. Logs are `sky-render-tests.txt`
  and `sky-cli-tests.txt` under the r16 evidence directory. No GPU captures
  have been run for the atmosphere while Joep is playing.
  The release `atmosphere_gpu` example compiles successfully (41.39 seconds;
  `sky-example-build.txt`). Run its executable with `--out FRESH_DIRECTORY`
  only during a free GPU window, then inspect the four actual images.
- Evidence directory: `C:/dev/perf/warp-sideways-20261024/r16`.
  `iteration-summary.json` links the actual GPU reports. The 38-frame native
  960x540 assembly fixture (14 plants, 70 nodes) passed GPU source/volume pose,
  inverse, bounds and previous-pose checks (max pose error 2.69e-7), with zero
  traversal overflows. Both zero-light images are exactly black. This is
  mechanical correctness, NOT visual acceptance: close voxels are blurred,
  coverage is inflated and integration alone costs 6.95-11.00 ms here. Keep
  the prototype out of the live grass/forest replacement.
- Follow-up findings: the heavy integrator dispatches over the whole image
  with ~2.25 KiB dynamically indexed arrays per invocation. Naga initializes
  function locals; driver spilling is only a hypothesis, not measured. Test
  an active-tile worklist before changing optical fidelity. Finest 2.5 cm
  cells are also too coarse for 4.5 mm needles, and the existing surface baker
  counts both sides of closed leaves. A one-leaf projected-coverage oracle
  is needed before changing extinction; do not hide this with tint/density
  boosts. Updated fixture JSON now includes coarse-mip diagnostics, but the
  first report predates those extra fields.
- `fog-a/report.json`: fully fogged geometry and background were pixel-exact
  (0 different pixels), partial fog remained visible, culling removed camera
  triangles/vertices while retaining all six shadow triangles; independent
  camera distances passed. This GPU run preceded the subsequent bilinear
  sky-lookup refinement, whose full GPU regression is still pending.
- Live playtest restored and first visible frame confirmed: PID109828,
  `build/dev-runs/20261024T070554235Z-edce00cd/iteration.json`, exe SHA256
  `50be5339b3c8d779931628b34f40bd23b7d7e81c43c77c875c0c73cedfb122cc`.
  One player, 3840x2160 output, DLSS Quality, wind0.6, summer, extended reference
  grass profile. Fog fix is live; procedural sky/assembly replacement is not.
  Joep is playtesting now: leave this session running while CPU work continues.
  The explicitly named `sky_and_weather` agent owns the atmosphere follow-up;
  the old `road_terrain_seam` name was confusing and is no longer active.

- Follow-up rock/road work is source-ready, **not yet visually accepted or
  present in PID109828**. F12 captures `sideways-1792825863196-109828-0` and
  `sideways-1792826096534-109828-1` show broad flat native cut faces alongside
  the twelve localized detailed exposures, plus a bright road/soil cut.
  Native cliff triangles now receive welded adaptive source refinement only
  at steep slopes and 5-35 m beyond the road edge. Continuous world-space
  fractured relief and curved surface normals are bounded to 0.30 m from the
  unchanged collision triangles; the first 5 m beyond the road edge stays
  exact. Rolling terrain keeps its original topology and fast construction
  path. Existing cluster LOD handles distance, with no extra patch instances.
  Rock mapping now uses arbitrary rotations and fixed 3.7/6.1 m scales,
  and irregular strata replace the periodic vertical sine bands. Road and
  terrain converge to the same world-mapped soil colour/normal at the join;
  road-edge normals now match the native terrain. Handling/colliders unchanged.
  Six geometry tests pass, including bit-exact height/normal agreement and
  identical boundary segments across two neighbouring steep-cliff chunks.
  Existing rock tests 5/5, road geometry/collision tests 3/3 and composed
  shader validation 4/4 also passed. The cliff-heavy CPU fixture increases
  2,048 native triangles to 12,880 source triangles (6.29x in that fixture,
  not a GPU cost claim); scene construction logs native/source/root counts.
  Actual visual quality and GPU/CLOD cost still need a free-GPU capture.
  Evidence: `r16/rock-terrain-geometry-tests.txt` and `r16/rock-terrain.json`.

- **Unified-rock correction supersedes the preceding rock candidate.** New
  F12s `sideways-1792827681185-102808-0`, `1792827777800-102808-1` and
  `1792827988108-102808-4` expose alternating detailed/flat spans and large
  cut-toe forms. The twelve exposure overlays and their runtime `add` API
  are removed. They sampled the old native ground while the rendered ground
  had already changed. The useful fracture field now supplies only the one
  native terrain surface. A second real discontinuity was the old rock
  mapping: variant B used 6.1 m, but became A at 3.7 m when its integer selector
  changed. Rock now has one fixed 3.7 m world projection, restrained continuous
  warp with matching analytic normal/texture gradients, and mild weathering.
  Terrain uv.y stores `1 + macro normal.y` (zero is legacy fallback), preserved
  by terrain CLOD and distant terrain, so small upward-facing rock details do
  not turn into artificial soil/grass patches.
  Relief is expressed as up to 0.24 m normal to the face, converted to a
  heightfield Y offset; rounding stays bounded to 0.16 m Y and the total Y
  deviation is capped at 1.10 m. This replaces the overly shallow 0.30 m Y
  bound. The road guard is narrowed to 2 m beyond the road edge, with 21,894
  actual rendered shoulder contacts verified across three generated stages
  and 12 refined chunks. Eight visuals tests, two shared-geology tests and
  four composed shader validations pass. Exact X/Z chunk boundaries and a
  single terrain material with preserved macro-up through CLOD pass too.
  The cliff-heavy fixture is 2,048 -> 13,109 source triangles. Actual GPU cost
  and matched-capture visual acceptance are still pending. Large cut-toe
  pyramids can also originate in the broad Stage height/ledge envelope; do
  not claim this bounded renderer refinement fully fixes that source shape.
  Evidence: `r16/unified-rock.json` and `r16/unified-rock-*-tests.txt`.

- **Authoritative macroterrain follow-up, same F12 #4.** The broad clipped
  crests were not just low-poly ledges: `Stage::carve` imposed a river-bank
  plane up to 40 m from a river, including its endpoint, then used a hard
  minimum against the cliff. At row 1142 / edge 11 m this cut a 15.58 m
  crest down to 6.80 m; at row 1174 / edge 12 m it cut 22.62 m to 17.98 m.
  The river is respectively about 17 m and 31 m away. No decorative noise
  was added. Dry bank influence now fades continuously between river half
  width + 2 m and + 12 m, with a bounded 0.1875 m soft intersection. The
  wet channel and water levels remain exact. Road + 2 m is protected with
  an additional native-grid diagonal halo, preserving triangles as well as
  point samples. This changes the authoritative off-road height and hence
  collision together; it is not another visual overlay. RNG, road layout,
  river paths and native grid resolution are unchanged.
  Nine Stage tests and three existing road-seam tests pass, including
  78,550 bit-identical road/shoulder heights, 12,568 unchanged native grid
  corners and 1,195 unchanged wet-channel/bank samples. Continuity at the
  protected-bank edge, outer influence and old 40 m boundary is tested.
  On the captured upper-crest camera ray, native-grid error against the
  continuous field drops from 1.250 m to 0.109 m; across 8,262 nearby
  section samples p95 only changes from 0.491 m to 0.483 m, so this is not
  a general grid-resolution fix. The leftmost toe (0.303 m) and right rail
  (0.210 m) ray errors remain. Source frozen; matched GPU review is pending.
  Evidence: `r16/macroterrain.json`, `macroterrain-before/after.json`, and
  `macroterrain-*-tests.txt`.

## Nanite foliage research applied to our next steps, 24 October

Primary sources checked at Joep's request:

- [Epic Nanite Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage):
  the system combines assemblies, aggregate voxels and skeletal wind. Voxel
  substitution follows representation error; its shading retains a distribution
  of surface normals. Animation also needs a screen-size policy because animated
  shadows remain costly. Our fixed-distance mean grass field is not this system.
- [Epic Nanite Assemblies](https://dev.epicgames.com/documentation/unreal-engine/nanite-assemblies):
  reusable parts avoid duplicating geometry. The hierarchy merges distant parts
  so every twig does not remain a separate render instance. Current skeletal
  assembly parts follow bone pivots rigidly; individual part vertices are not
  independently skinned. Procedural variation should preserve part reuse.
- [Heitz et al., SGGX](https://research.nvidia.com/labs/rtr/publication/heitz2015sggx/):
  projected area parameterizes an anisotropic normal distribution, supporting
  interpolation and prefiltering. One averaged normal cannot preserve the same
  directional appearance. This is a useful shading model, not an automatic cure
  for correlated surface coverage becoming a homogeneous medium.
- [Epic's analytic shading status](https://forums.unrealengine.com/t/nanite-foliage-and-twosidedsign/2835970):
  the Witcher demo used analytic SGGX, while that path was not fully integrated
  into the UE 5.8 release. Stochastic normals and two-sided orientation have
  different limitations. Do not equate the released feature and demo or add
  random temporal noise to hide our existing grass transition.

Our implementation decisions, rather than claims about Epic's implementation:

1. Preserve the actual clump silhouette, holes, directional coverage, colour and
   normal distribution across representation changes. The CPU source-sheet
   prototype has better measured opacity fidelity than our Beer medium, but has
   no runtime GPU path yet. Extending source cards is only a controlled reference
   and temporary fidelity experiment, not a finished voxel solution.
2. Shared foliage assets need reusable parts plus a hierarchy that can collapse
   groups. Do not allocate a permanent scene instance per twig throughout the
   entire forest. Select representations using projected error in each actual
   split viewport; share poses and asset residency across views.
3. For visible branch wind, derive a small authored rig from the existing crown
   bundles. A bounded first experiment can use rigid bundle groups, a stiff trunk
   and parent branch pivots. Source assets have no complete branch skeleton;
   this is not a biological growth simulation. Stronger whole-tree shear would
   not meet Joep's request.
4. Before enabling branch animation, retain stable source/bone IDs through CLOD,
   current and previous poses, transformed normals, shared shadow deformation
   and conservative bounds. The current whole-plant inverse-shear motion pass
   cannot reconstruct independently animated branches. Later voxels must inherit
   the same source groups and motion rather than becoming a second tree model.
5. Compare frozen/moving near and far views, sun-facing/backlit views and four
   separated cameras. Measure visibility, animation, shadows and shading as well
   as total scene GPU time. Published demo bone timings do not establish our
   four-player 8.33 ms frame budget.

The current live renderer still uses whole-plant GPU shear and static far-grass
statistics. The CPU shared foliage index and voxel baker are foundations, not
evidence that production Nanite-equivalent foliage or branch wind is complete.

## Original goal

Joep asked for an engine designed around fast AI iteration loops, with the
Sideways rally game (Godot 4.5) as the benchmark: port it, and beat Godot
on graphics, physics and developer velocity. Modern rendering from the
start: Nanite-style cluster LOD, visibility buffer, HDR in physical units,
GI. Keep the loops short and measured (README, "The iteration loops").

## Rules from Joep

- Talk to him in Dutch, without em-dashes. Code, comments and docs in English.
- Open a visible game only for an explicitly requested playtest/demo. Joep
  now requests visible autonomous driving for remote watching. Otherwise
  render offscreen; stop the demo before isolated GPU measurements.
- Commit to `main` of ontola/warp-game-engine and push. Attribute only the
  actual contributors/session; do not copy another agent's attribution.
- When mentioning PRs, link them with a short description.
- Show actual pictures and honest limitations. Compare current Kern variants;
  Joep explicitly ended Godot comparisons on 23 October.

## In-progress Windows r15, 2026-10-24

Latest 24 October steering, road/cliff changes now in running r15e:

- One road type for now. `Stage::make_road_surfaces` now fills every row with
  Gravel; random Dirt/Mud/Asphalt bands were removed. The same data drives
  visuals, tyre grip and AI. A generated-stage regression across three seeds
  and 0.8/3/8 km verifies row/AI/wheel agreement. Loose shoulders, offroad grass
  and physical river water remain their own ground responses. Route-generation
  RNG is unchanged because the removed bands used a separate private RNG.
- Future road types need a geographical reason: mud from wet soil in rainy
  lowlands; paved/unpaved transitions at a real branch/junction, with a blocked
  unused branch retaining one race route. Road width can change with road type.
  Do not bring back arbitrary distance bands or implement junctions prematurely.
- F12 `sideways-1792820558094-57532-0` shows uniform far grass. CPU terrain rays
  locate sampled flat areas at 73-111 m and near/far transition at 53-57 m
  (`r15/grass-r15d-capture-probe.json`; ignores foreground objects/render CLOD).
  The live global four-bin mean field still loses individual clump structure.
- `r15/grass-clump-voxel-cpu-b` compares real projected sizes: the area-density
  Beer medium retains only 81-87% source opacity. An exact-source-sheet
  coverage-mip candidate gives 100.5-101.5% at the screenshot's 73-111 m Quality
  views, but has residual small-viewport error up to 7%. Fourteen CPU tests
  pass. No runtime switch: a generic API must distinguish surface coverage
  transport from a homogeneous medium; conserved area is not conserved opacity.
- F12 `...57532-1` and `...57532-2` show repetitive cliff-toe pyramids. The 2 m
  native grid undersamples a 9-22 m cliff rising in 4.6 m, before render LOD.
  The revised toe/crown varies coherently over roughly 6-9 m; ledge strength
  now interpolates continuously between road rows. Full cliff height and steep
  faces remain. Screenshot #1 source-grid error p95 fell 0.969 to 0.467 m,
  maximum 1.545 to 0.747 m. This reduces, not eliminates, the coarse-grid error.
  Screenshot #2 also contains a sharp river carve, which remains unresolved.
  All 24 Sideways unit tests pass, including bitexact drivable foundation,
  shoulder/collider, original main-road hash and the new road-surface checks.
  Evidence: `r15/cliff-sampling-source.json`, `cliff-sampling-profile-a.json`
  and `cliff-sampling-profile-a-dry.json`. No new GPU acceptance claim.

Current r15e manual playtest reached its first visible frame after a managed
restart: `build/dev-runs/20261024T060113217Z-6d2a4397/iteration.json`,
PID 194472 at launch. One player, 3840x2160 output, DLSS Quality, wind 0.6,
summer. Archived executable `C:/dev/perf/warp-sideways-20261024/r15/sideways-r15e.exe`,
SHA256 `0086bcc162b7f2fcf22bb32e6ed220d03dd8c30f176478544631d50e8dfd0496`.
Build 92.49 s; startup 20.76 s. The road is now continuous gravel and the cliff
toe/profile is revised. Grass rendering remains the r15d mean-field path.
First-frame confirmation is execution evidence, not visual acceptance.

The new CPU-only `render::foliage_groups` index is source-complete and its six
tests pass (`r15/tests-foliage-groups.log`). Shared bounds, stable identity and
generation, inverse poses, conservative wind/filter bounds, explicit memory
limits and half-open DDA segments are representation-independent. Asset,
instance, cell and owner-reference records are 64/144/32/4 bytes. It has no
runtime/GPU integration yet. Its final preallocation-budget correction happened
during the r15e release build and is not claimed to be present in that executable;
the module is unused by r15e rendering either way. No further source changes
occurred after its final targeted tests.

Previous r15d manual playtest reached its first visible frame after a managed
restart (user explicitly requested the latest version when ready):
`build/dev-runs/20261024T054027200Z-7193dab5/iteration.json`, PID 57532 at launch.
One player, 3840x2160 output, DLSS Quality, wind 0.6, summer. Archived executable
`C:/dev/perf/warp-sideways-20261024/r15/sideways-r15d.exe`, SHA256
`5849dbbe00da53d267289a62dc425b1b560cbac859d53baa3286ca7e128a572a`.
First-frame startup confirms execution, not visual/performance acceptance.
No isolated GPU benchmarks ran concurrently with the manual playtest.

Changes in r15d:

- Trees share three source archetypes (canopy/conifer/stout) and deterministic
  juvenile scale. Missing C-trunk UVs use cylinder unwrap and tileable bark.
  Textured wood LOD retains normals/UVs and bounds newly reversed faces below
  two projected pixels. This retains more near wood triangles; cost unmeasured.
- Terrain has shared world-halo normals and attribute-aware LOD, including a
  source-triangle lock within 5 m of the road. The outside 80 cm road shoulder
  follows actual terrain triangles with a 3 mm overlap; main lane unchanged.
  CPU ray/geometry checks pass; see `r15/road-seam-cpu.json`.
- Terrain texture variants keep physical scale and blend world-anchored rotated
  offsets instead of fading to a 4.3x enlarged texture with camera distance.
  Cliff relief uses the same terrain material and normals blend at the actual
  terrain intersection. Fully rocky pixels skip zero-weight soil samples.
  Painted green road-center streaks were removed. GPU visual review pending.
- Grass clump data is now derived from the exact shared near source through
  `render::foliage_voxel`, preserving local coverage/color and stable grouped
  placement. CPU fixture `r15/grass-clump-voxel-cpu-a` has one 365,768-byte
  shared asset, seven mips and 145 exact live plants. Twelve tests pass.
  Finest 2 cm cells cover only 82-84% of the source silhouette in three
  directions; fix projected-opacity fidelity before runtime acceptance.
  This is CPU/offline work; live far grass remains the mean field.
- `sideways::wheel_fx` is a tested material/slip/load-dependent spawn API at
  120 Hz, bounded to 16 wheels and 96 births/tick. It is not yet connected to a
  GPU particle pool or live play. Do not claim visible debris/dust is done.
- `chase_camera.rs` holds <=1.5 cm/0.03 degree body rest noise only below
  0.03 m/s and releases on driving/turn/teleport. Unified raw right-stick
  input uses 0.24/0.16 radial hysteresis and wrapped yaw; four CPU regressions
  pass. Vehicle physics and driving inputs are unchanged. This addresses the
  measured neutral drift, not proof that every reported wiggle is resolved.

Joep is actively playtesting. Keep the current session running and do not run
competing GPU benchmarks. The previous managed session
`build/dev-runs/20261024T050859254Z-cb791f7d/iteration.json` reached its first
visible frame at 3840x2160, one player, DLSS Quality, wind 0.6 (PID 171044 at
launch) was gracefully stopped for r15d. It ran the r15c executable below with
all three grass constructor flags enabled, as r15d does. Its controller
is connected through XInput as `Xbox Controller`. Revalidate the PID/path/start
identity in `build/dev-runs/active.json` before any later restart.

The current r15 work is uncommitted on top of r14. Earlier tested executable:
`C:/dev/perf/warp-sideways-20261024/r15/sideways-r15c.exe`, SHA256
`4bf8ff48eaf151a6626b64b6f10c335631b0bded4c2db1d1c62d3b5e701fd265`.
141 CPU tests passed, with one unrelated ignored benchmark (`tests-c.log`).
All 22 actual GPU motion cases passed (`motion-c/report.json`). The c build
adds alpha-aware linear cutout RGB filtering, a shared inclined-leaf grass
BRDF, and bounded local grass sun transmittance. Tree archetype runtime
changes are being prepared separately and are NOT in this executable.

One short, same-binary 4K one-view DLSS Quality grass A/B (`grass-old-route`
versus `grass-new-route`) measured 25 frames after ten warmup frames, with
identical car/camera traces. Instrumented GPU sum increased from 15.465924
to 16.445108 ms (+0.979184 ms). All three grass flags changed together, so
this is neither an isolated feature cost nor a robust performance result.
The new image was reviewed before manual launch. HUD, simulation,
presentation and uninstrumented work are excluded; no gameplay FPS claim.

The 60-image `grass-c` fixture completed, but its default 2x1 constant sky
exposed a separate SH integration bug: fixed step-4 sampling aliases it to
one horizon direction. The predicted inclined-leaf/UP irradiance ratio
5.05157 matches measured far indirect HDR 5.05337. The game's 2K HDRI does
not have this specific undersampling problem. The fallback now integrates
small sky texels analytically (CPU constant/hemisphere tests pass); rerun
the fixture before using its lighting measurements as acceptance evidence.
The albedo-only mismatch narrowed, but this does not prove all grass pop-in
or temporal artifacts are solved. No additional GPU runs during playtesting.

Earlier r15b executable SHA256 was
`cc8f8060f8b9e334c7b9db54806597a2604bcb0f00067b0c51853d2b40568fcd`.
Real NGX DLSS Quality completed 30 moving frames at 853x480 -> 1280x720 in
`first-dlss-b.png` / `first-dlss-b-state.json`; there was no fallback. The
visible 4K session has also submitted frames successfully. This is functional
evidence, not a four-player performance or image-quality acceptance result.
The earlier r15a failed before evaluation because wgpu 30 forbids mixing raw
and tracked commands in one encoder. r15b uses separate ordered command buffers
in one queue submission. Preserve failed and successful evidence separately.

DLSS setup: `NVIDIA_DLSS_SDK=C:/dev/tools/nvidia-dlss-sdk`,
`VULKAN_HEADERS=C:/dev/tools/vulkan-headers-dlss`,
`KERN_DLSS_RUNTIME_DIR=C:/dev/tools/nvidia-dlss-sdk/lib/Windows_x86_64/rel`.
Build with `--features render/dlss`, target `C:/dev/warp-target-20261023`.
Native remains the default. CLI/helper/GameNight accept `native`,
`dlss-quality`, `dlss-balanced`, `dlss-performance`. The helper records the
official runtime hash; see `crates/render/native/DLSS.md` for SDK/license and
integration details. Experimental voxel crowns are rejected with DLSS until
they have suitable motion data. HUD remains at output resolution.

`r15/tests-b.log`: 132 CPU/shader tests pass (52 CLI, 70 render, 10 game).
`r15/motion-a/report.json`: 20 GPU motion cases pass, max error 0.003795 pixels.
The rebuilt motion probe adds two cases and corrects merged depth on reset;
the final 22-case run passed in `motion-c`. `render/examples/dlss_probe.rs`
is a lifecycle/raw-output fixture that still needs execution.
Remaining DLSS validation: lifecycle, missing-runtime
native/error paths, real 4K/four-view modes and same-route native/DLSS timings.
Do not claim a speedup, full visual acceptance or the 120 FPS target yet.

Latest user grass report: darkening/pop-in around 50 m. The normal single-view
handoff is 40-60 m, split-screen 20-38 m. Far grass is already a terrain-following
density volume. `r15/grass-a` contains 60 real GPU captures of the same resident
photo-grass patch, five distances, near/far/blended, and four lighting components.
At 50 m near coverage is 0.98873 and far alpha is 0.99820, but their colors differ
substantially. Investigation is focused on texture filtering and shading, not
adding density or arbitrarily darkening the far field. This sunny fixed-patch
fixture does not yet certify forest shadow parity. Production grass has no
full visual acceptance yet; r15c contains the first correction below while
the statistical near/far handoff itself remains in place.
`Lighting.grass_reference=1/2` forces near/far only for diagnostics; use 0 for
normal play and performance measurements. Finish validation, document results,
then commit/push main. Joep has authorized a managed restart when the next
coherent iteration is ready; avoid concurrent GPU tests during manual play.

The grass RGB filtering candidate is now in `render/src/cutout_mips.rs` plus
the small `texture_array` hook. Five CPU oracles pass, evidence
`r15/grass-a/cpu-candidate.json`. Existing alpha/coverage is preserved byte for
byte; only actual sRGB cutouts use alpha-weighted linear RGB from the original
image, with color extended into transparent texels. This is in r15c; the
combined runtime comparison and remaining fixture limitation are above.

Actual user F12 references, both from the running r15b DLSS Quality session:
`build/screenshots/sideways-1792817001611-266592-0.png` and
`build/screenshots/sideways-1792817202919-266592-1.png`, with JSON sidecars.
The first looks across a wooded slope; the second clearly shows nearly-white
grass facing the sun. Both near/far lighting currently use one UP normal and
coherent GGX, and grass is excluded from shadow casting. The mean density field
only attenuates viewing rays, not sun rays. The RGB filtering fix alone cannot
resolve these lighting errors. A shared grass normal-distribution and local
density sun-transmittance candidate is being developed separately; its GPU
cost and visual quality are unvalidated. Avoid simply darkening grass or
promising that a particle system will fix transparency/lighting costs.
The first capture measured 1.3905 ms in the dedicated near-grass pass and
0.1791 ms far volume at 2560x1440 -> 3840x2160. Main raster/shade also contain
grass work, so these are NOT the complete cost of grass or a native/4P result.

The source lighting correction is in r15c and has run on the GPU, with the
limited comparison above. `grass_lighting.rs` and `grass_lighting.wgsl` share one
projected-area weighted, two-sided blade-normal response between opaque,
transition and far grass. Source color, F0 and roughness are retained. Four
quadrature directions follow view azimuth rather than fixed world axes;
this is a macroblade approximation, not a measured leaf-normal distribution.
Near/far grass and terrain/road also query sunlight extinction through the
existing four-height-bin grass field, with the native terrain diagonal and
road clearance. The trace is explicitly bounded to eight cell/tile visits
and 16 m; empty tiles are skipped and omitted space remains unoccluded.
No individual grass shadow geometry or full-resolution target was added.
This does not animate the far density or replace the statistical field with
source-derived voxels. Sky visibility still uses one shared canopy query.

Constructor-time ablations: `KERN_GRASS_LEAF_BRDF=0` restores the single-UP
normal response; `KERN_GRASS_SUN_OCCLUSION=0` disables the new medium shadow.
Both default on unless their value is exactly `0`. Shadow strength zero also
disables the medium shadow. `KERN_CUTOUT_LINEAR_RGB=0` independently restores
the old texture RGB filtering. Use the same executable/camera/source patch
for baseline (all three zero), mip-only, leaf-response-only, medium-shadow-only
and combined images/timings. The eight-visit per-pixel trace is not assumed
free; measure main shade, transition grass and far-volume costs separately
before accepting the candidate. The 24 October user subsequently authorized
launching the updated playtest when the coherent candidate is checked.

## Windows continuation r14, 2026-10-24

Current graphics executable: `C:/dev/perf/warp-sideways-20261024/r14/sideways-r14a.exe`,
SHA256 `2f9f4ef830df9b8d630e90da7c943000bb2a321f0b88f28731140c7e58ddf7a3`.
The previous r13 binaries/evidence remain intact. User asked to optimize the
budget and work on wind, growth and seasons. No Godot comparison is required.

Geometry work is now bucketed into 8/32/64/128-triangle indirect draws. Original
visibility IDs and LOD policy are retained; live grass rejected by the existing
opaque fade is culled before drawing. Per-camera cluster buffers persist.
`KERN_CLUSTER_BUCKETS=0` and `KERN_EARLY_GRASS_CULL=0` are constructor-time A/B
switches. Per-frame metrics distinguish actual submitted vertices from retained
shadow geometry. Buckets require up to 16 MiB additional mapping storage per
view/map (144 MiB upper bound for four main + four near + one shared sun map).

Native 4K/four-player ABBA, 80 measured route frames per run after 20 warmup:
GPU scene-pass mean 39.4352 -> 34.4169 ms (12.73% less), raster 19.5981 -> 13.3308 ms.
Sorting raises cull cost; the net improvement includes it. Exact car/camera
traces match. Final captures differ in one pixel by at most 5 RGB8 levels, also
seen between repeated baseline runs. This is bounded route evidence, not full
gameplay FPS or proof over all routes. 120 FPS remains unmet.

`render::wind::WindState` + `Renderer::set_wind` drive a single compute field
shared across source geometry, every camera/LOD and depth shadows. Roots stay
fixed; normals and CLOD bounds account for the shear. This is whole-plant sway,
not skeletal branch animation. Canopy/far-grass densities remain mean rest-pose
fields and experimental packed voxel-tree owners stay static. Wind does not
invalidate the canopy; source sun shadows update with changed wind. The GPU
differential fixture matches CPU-baked geometry and direct shadows exactly.
Separate four-view wind ABBA: 34.4684 -> 34.6230 ms at strength 0.3, +0.1546 ms;
the shared compute itself is 0.0238 ms. Not an isolated total GPU-clock claim.

`--season spring|summer|autumn|winter --growth 0..1 --vegetation-seed N`
select a stage phenotype. Shared native tree Spots keep colliders aligned;
near/far grass use the same stature and tint. Pines stay evergreen. Growth is
authored juvenile-to-mature scale, not procedural branch generation; terrain
does not yet recolor with seasons, and there is no snow. Summer/growth 1 keeps
the prior geometry. CLI play/shot now default to gentle `--wind 0.3`; engine
and existing gpu-run jobs default to zero. `--wind-time SECONDS` freezes phase.
`--gpu-profile NEW.jsonl --gpu-warmup N` records per-frame route/GPU evidence;
it synchronizes for readback and excludes HUD/presentation/CPU from pass sums.

Use `tools/play-dev-windows.ps1 -Restart -Autoplay` for the visible remote demo.
It accepts `-Season`, `-Growth`, `-Wind`, `-WindTime` and `-VegetationSeed`, and
records the options in its staged run manifest. Keep far grass enabled and
experimental voxel crowns disabled. Stop the owned demo before GPU tests.
Full evidence and remaining limitations are in WINDOWS_VALIDATION.md.

## Previous Windows continuation r13, 2026-10-24

Latest r13c build SHA256:
`c8092e3def8f464949cc6fc10738e1ae9828604a4f1a46bd388aa96212851565`.
The r13 graphics iteration adds persistent per-camera canopy clipmaps for
distant sun/sky occlusion, plus source-color/relative-height moments for far
grass. One shared canopy was rejected because separated players changed
lighting. Each view now has its own 1.5/6/24 m field levels, 27 MiB per view,
108 MiB for four. Metadata reports per-view pitches and dirty masks.
The canopy is a static ellipsoid-density approximation, not source-leaf
voxel geometry, animated trees or a spatial radiance cache.

The corrected 55-frame GPU fixture passes, including high forests, distant
sunlight, split camera invariance and the former spread-threshold jump.
12 grass and 11 tree-volume optical cases pass. The r13b graphics executable
ran native 4K one/four-view 100-frame tests with matching r12b player states.
Final scene GPU pass sums were 20.240/38.883 ms versus 17.977/36.954 ms before;
this quality improvement costs GPU time. These are captured-frame timings,
not gameplay FPS. The 120 FPS target remains unmet. r13c only raises the
validated tree-count capacity to accommodate the existing 8 km stage range;
the 8 km scene itself has not been measured. Read WINDOWS_VALIDATION.md.

Keep `SIDEWAYS_FAR_GRASS=1` and `SIDEWAYS_VOXEL_TREE_PATCH=0` for the requested
visible autoplay. Restart through `tools/play-dev-windows.ps1`; its active
manifest owns the current PID/path. Stop it before isolated GPU measurements.
Motion shimmer, approximate far silhouettes and broader route/weather
coverage remain open. Future GPU wind/growth/season requirements are above.

## Previous Windows continuation, 2026-10-23

Previous r12b validated candidate SHA256:
`05ec994693442cc72ec7d2cb10641666a616fa0d13db25138a860e75c479097e`.
The visible demo is now managed by `tools/play-dev-windows.ps1`; use its
`-Restart -Autoplay` workflow and inspect `build/dev-runs/active.json`.
It runs with `SIDEWAYS_FAR_GRASS=1`, `SIDEWAYS_VOXEL_TREE_PATCH=0` and NVIDIA.
The new far grass fills the former missing representation past 60 m; it
remains opt-in because open slopes look too smooth and the field has a fixed
allocation bound. 101 CPU/shader/oracle checks and 12 far-grass / 11 tree
GPU optical cases passed. Paired 100-frame 4K one/four-view smoke runs passed;
final-frame far-grass overflow was zero in all views. This is not full-route
temporal acceptance. Same-frame far pass samples were 0.190 ms single,
0.768 ms combined four, and 1.190 ms on a later open-slope live capture.
The four-view scene sample total was 36.954 ms excluding HUD/presentation/CPU;
120 FPS is not achieved. Read the r12 section of WINDOWS_VALIDATION.md.

Read `WINDOWS_VALIDATION.md` for measured results, evidence and limitations.
The `play` command now provides a Windows driving loop, native XInput,
GameNight lifecycle/controller transport and one to four rendered views.
Only explicit `play` opens a window. Managed preparation stays invisible;
automated GPU validation uses `play --smoke-render N` and reports zero
visible presented frames. It is not a substitute for a visible playtest.

Windows uses gilrs's XInput backend. The attached 8BitDo Ultimate is named
"Xbox Controller" by that backend. A physical input probe recorded both
sticks, full throttle/brake, handbrake, recovery, camera and Start.
The installed GameNight Preview daemon speaks protocol 1 / SDK 5, so
SDK 6-only actions and room logs must be capability-gated.

The photographed grass uses shared world cells and terrain-triangle roots.
Frozen shots have a 60,000-instance cap; live play streams cached candidates
into a fixed 70,000-slot pool for one view or 115,000 shared slots for multiple
views. Existing plants keep their poses/sizes; only changed slots upload.
The r9 grass uses static habitat density, retaining every tuft and all three
cards. Near grass stays opaque; a nearest-layer opacity pass fades from
40-60 m in one view or 20-38 m in split screen. CPU residency extends beyond
that fade. This is not a distant geometry LOD yet. Deeper partial grass
layers are omitted; glass ordering remains approximate and live TAA stays off.
The private HDR/depth targets require about 95 MiB at combined native 4K;
measure the extra GPU passes before making performance claims.
`SIDEWAYS_LIVE_GRASS=0` disables it for comparisons. Live TAA is still off
until per-instance motion vectors exist. The native-resolution HUD now uses
glyphon and shared cached Barlow text, with timer, instruments, pace notes,
progress, recovery, countdown and results. Audio, an in-game settings menu,
particles and the remaining ground cover are still missing.

One 4K output has four 1916x1076 views with an eight-pixel gap. The 120 FPS
target has not been met. Frozen GPU pass timings are not live-game FPS.
Keep the Godot source beside `engine/` as the unchanged reference.

The Windows lighting update fixes indirect visibility around cars,
direction-aware glossy reflections, locally conditioned ground bounce,
absolute shadow-texel snapping, filtered world-aligned canopy density and
reversed stone-wall normals. Read the lighting section of WINDOWS_VALIDATION
before claiming full shadow stability or 120 FPS. The subsequent r5 playtest
adds forward glTF glass transparency, smooth terrain visibility normals and
a bounded distant forest on rolling background hills. The r7 default reuses
the three actual near-tree meshes and their cluster LOD to retain species
identity; the original exposed-face voxel pines now require
`SIDEWAYS_GENERIC_VOXEL_TREES=1`. The hills have higher warped skyline ridges.
The following density iteration measures actual native trees per eligible
horizontal terrain area and matches that density on the hills, bounded to
200,000 background instances with any shortfall explicitly logged. This uses
the current cluster renderer, not Unreal Nanite.
The separate sparse density-voxel bake is still experimental and inactive in
play. Its second CPU comparison reduces oblique coverage error using 13
directional projected-area samples, but coarse levels remain soft and this
has not been accepted as a near-tree replacement.
Joep explicitly prioritizes runtime source-derived voxel trees next. Preserve
the current near crowns and species identity; procedural plant generation is
later work. Do not call the existing alpha-card cluster renderer voxel foliage.
As of the late 23 October session, Joep no longer wants Godot comparisons:
evaluate current Kern mesh and voxel versions against each other. He is done
playtesting tonight but subsequently asked to watch gameplay remotely. An
explicit `play --autoplay` uses the existing race AI and keeps driving without
focus; it does not override managed GameNight inputs. Show occasional actual
images in chat. Stop the visible demo before GPU measurements and restart it
afterwards; do not mix its GPU load into comparison timings.
Joep requested quick build/restart rounds with his live visual feedback. Do
not run competing GPU benchmarks during his playtest. Build/shader checks
and first-frame confirmation remain appropriate; do not imply a full GPU
acceptance suite or a 120 FPS result for these rapid iterations.

F12 in live play writes the current final 4K (or chosen window size) composite,
including HUD, to `engine/build/screenshots/sideways-<timestamp>-<pid>-<serial>.png`.
A JSON sidecar records the camera, cars, stage, settings and adapter. Windows
also copies the image to the clipboard for the user to paste into the chat.
This does not transmit anything automatically or advance the simulation.

## Earlier renderer baseline

- Simulation: stage, terrain, road, barriers, cars (Rapier), AI and race
  loop ported. Stage layout matches Godot bit for bit; handling bench is
  within about 1%. See README "Parity".
- Renderer (`crates/render`), all headless wgpu 30:
  - cluster LOD DAG (meshopt clusterlod) and GPU cull;
  - visibility buffer and compute shading;
  - sun shadow map (cached while nothing moves) and canopy-volume GI;
  - TAA plus CAS sharpening;
  - textures: 2K albedo and 1K normal-map arrays, sampled with
    ray-differential gradients and 16x anisotropic filtering;
  - materials: GGX, sky reflections, clear-coat paint, Godot's foliage look.
- Joep's last verdict before the latest fixes: "very pixelated, especially
  the trees, ground and wall very blurred". That is fixed now: TAA,
  anisotropic gradients, normal maps.
- The cloud container only had lavapipe (CPU), about 40 s for a 16-frame
  TAA shot. On Windows you have a real GPU: use it, the loop should be
  milliseconds.

## Setup on Windows

    winget install Rustlang.Rustup LLVM.LLVM Git.Git
    rustup default stable
    git clone https://github.com/ontola/warp-game-engine C:\dev\warp-game-engine
    cd C:\dev\warp-game-engine\engine
    cargo test -p render          # shader validation, no GPU
    cargo run --release -p cli -- shot out.png --seed 1 --km 3 --at 300 --weather clear --size 1920x1080

`engine/.cargo/config.toml` sets clang + lld for Linux. If linking fails on
Windows, scope that config to the Linux target (or use `rust-lld`). Do not
just delete it: the Linux container needs it.

`KERN_ADAPTER` picks a GPU adapter by name substring. The shot prints the
adapter and per-pass GPU ms from timestamp queries. `gpu-run` (see
GPU_RUNNER.md) runs `gpu-jobs.toml`. The polling runner described there is
not needed now that you run on the GPU machine itself.

Godot reference frame: `tools/shot.sh out.png --frames 30 --seed 1 --at 300
--cam chase` from the repo root (needs Godot 4.5 in PATH). It renders
offscreen.

## Ground-cover reference and remaining work

This is the biggest visible gap with Godot: the verge is bare, with a
mown-lawn tiling, where Godot has grass tufts and berry shrubs. Port
`src/grass_field.gd` and its shaders (`grass_cards.gdshader`,
`grass_field.gdshader`, `vegetation_density.gdshaderinc`,
`grass_coverage.gdshaderinc`), plus `src/ground_cover.gd` (ferns, mossy
rocks, stumps; the chunks already carry `ferns`, `rocks`, `stumps` and
`grass` spots in `builder.rs`).

What Godot does:

- **Carpets.** Rings of tufts around each camera, built on a world-cell
  grid with `[kind, cell, inner, radius, grow]`:
  - high detail: grass at (0.3 m, 0-20 m), (0.7 m, 18-60 m) and
    (1.6 m, 52-110 m); shrubs at (0.55 m, 0-30 m);
  - medium and low presets for 2 and 3-4 views.
- **Placement per cell id.**
  - jitter: `(hash(id), hash(id+17)) - 0.5) * 1.6` cells;
  - size: `grow * mix(0.7, 1.3, hash(id+9))`;
  - angle: `hash(id+3) * 2π`;
  - hash: `fract(sin(dot(p, (127.1, 311.7))) * 43758.5453)`.
- **Density, from the ground map** (height, metres from road edge, normal Y):
  - grass = `max(verge, wood) * (1 - shade * 0.45) * level`, where:
    - verge = `smoothstep(0.5, 1.4, d) * (1 - smoothstep(10, 22, d))`;
    - wood = `smoothstep(0.3, 0.6, noise(xz/7)) * 0.85 * smoothstep(1, 3, d)`;
    - level = `smoothstep(0.7, 0.82, up)`.
  - shrubs: patch noise, times `smoothstep(2.5, 6, d) * (0.7 + shade * 0.5)`.
  - `shade` is Godot's canopy map (a soft shadow per tree). Approximate it
    from tree spots, or from our canopy volume.
- **Coverage.** Each tuft is dissolved by a fixed per-tuft hash pattern
  (`grass_coverage_sample`), with smooth ring crossfades.
- **Grass mesh.** 3 crossing cards, 0.50 m high and
  `0.36*(257/135)` m wide. The UV is an atlas sub-rect of
  `assets/grass/grass_medium_01_diff_2k.jpg`, with the alpha in
  `grass_medium_01_alpha_2k.png` (red channel); combine them into one RGBA
  layer. Atlas min `(580.5, 889.5)/1024`, max `(836.5, 1023.5)/1024`,
  v flipped.
  - albedo: `tex * mix(0.8, 1, h) * 1.25`, normal straight up;
  - AO: `mix(0.45, 1, h) * (1 - canopy * 0.5)`;
  - translucency: `(0.12, 0.16, 0.05) * max(-n·l, 0)`.
- **Shrub mesh.** Procedural, `_shrub()`: 4 twigs × 6 leaves, RNG seed 11.
  - colour: `mix(green, dry, smoothstep(0.5, 0.75, tone))`, with green
    (0.2, 0.32, 0.1), dry (0.52, 0.15, 0.07) and
    `tone = noise(xz/5)*0.6 + hash(id+2)*0.4`;
  - Godot colours are sRGB: convert them to linear.
- **Wind.** Tips sway; there is no time in shots yet, so it can wait.

Original design, first step now implemented in `cli/src/ground_cover.rs`:

- **First step: build the tufts as instances on the CPU around the
  cameras, in `cli/src/visuals.rs`.** Make the decision for a fractional
  coverage per tuft: keep a tuft when `hash < coverage`.
  - Each tuft is one tiny cluster, so ~60k instances is fine for the cull.
  - This needs a heightfield with road distance and normal Y: extend
    `heightfield()` (the chunks have `heights` and `road_d`).
  - Add material flags for grass cards and shrubs. In `shade.wgsl`, derive
    tuft height from the instance origin (`inst.r1.w`) and `inst.scale`,
    and per-tuft tone from the instance origin.
- **Future design: GPU-driven.** A compute pass can spawn visible tufts into
  an instance list to reduce CPU streaming work. Split screen still has
  per-view culling, raster and shading costs; measure them independently.

## After that

- **Per-instance motion vectors for TAA.** Only camera motion is
  reprojected now, so moving cars would smear. Shade knows the instance;
  keep a previous-transform buffer and write velocity.
- **Complete the playable loop.** Moving instances, shadow invalidation,
  native input, GameNight and explicit window presentation now exist. Next
  are audio, settings, additional small vegetation and scene performance work.
- **Visual differences with Godot that remain:**
  - distant forest denser and darker than Godot's (Godot uses mid meshes
    and impostors beyond 150 m, `src/forest.gd`);
  - canopy GI darkens crowns a lot (try `--set gi=0`);
  - the rock wall's vertical streaks come from a faithful port of Godot's
    terrain shader; full triplanar would fix them in both.
- **Also from the game:** car lights, dust and gravel particles, wet roads,
  tyre marks (`track_map.gd`).
- **Performance on the real GPU:** measure with `gpu-run`. Shadow raster
  was the heaviest pass on lavapipe. Sim cost: Rapier step 175 µs and car
  forces 120 µs per tick.

## Handy

- `shot --pick x,y` says which instance, mesh, cluster and triangle covers
  a pixel.
- `--set key=value` retunes `Lighting` without rebuilding:
  - light and look: ev100, sun_lux, sky_lux, sun_elevation, fog_*, gi,
    debug=1 (sky visibility), debug=2 (albedo);
  - TAA: taa, taa_blend, taa_gamma, sharpen;
  - foliage: alpha_cut.
- `--taa 1` gives one frame without TAA.
- Every new WGSL file goes in the `shaders_validate` test in `render/src/lib.rs`.
- Traps already hit are listed in README. Two more:
  - In the view uniform, `params.z` is the ortho flag for culling; the alpha
    threshold lives in `extra.x`.
  - `shot` takes the output path positionally; it has no `--out`.
