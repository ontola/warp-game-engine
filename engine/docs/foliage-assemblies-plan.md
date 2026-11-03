# Foliage as assemblies: one tree path from 1 m to the horizon

Status: design, November 2026. Replaces the tiered forest (recipe_forest:
a pool of full assemblies, alpha card trees, whole-plant voxel proxies, the
painted far layer) with one representation per tree, after Unreal 5.7's
Nanite Foliage and Assemblies.

**Handoff:** read [Handoff (3 Nov 2026, evening)](#handoff-3-nov-2026-evening)
first. It is the current state, the open issues and the next tasks; the
sections after it are the history that led there.

## Windows follow-up (3 Nov 2026, retained meadow groundwork)

The Windows terrain_fly playtest on dcc3f1d confirmed movement stalls, even at
1713x964 internal rendering / 3840x2160 output. A single 355-frame headless route
at 240 m/s measured p50 71.7 ms, p99 711.4 ms and max 755.3 ms; without trees,
38.8 / 123.7 / 203.9 ms. These include synchronous image readback, are not gameplay
FPS or isolated GPU timings, and are diagnostic runs, not an accepted A/B.
Raw logs: C:/dev/warp-bench/terrain-diagnosis-{baseline,motion,motion-no-trees}.log.

Prepared changes (GPU execution deferred: user is playing another game):

- `WARP_TERRAIN_STREAM_GROUND=1` opts into updating meadow ground samples in the
  retained GPU allocation. Geometry, pipelines, root seed and renderer survive.
  Ground sampling remains on a worker. Near full-tree slots already follow the
  camera through Pool::update; they no longer force 100 m scene rebuilds in this
  mode. The existing 300 m outer forest refresh still rebuilds the renderer.
- The 120 m altitude threshold no longer constructs/destroys the entire meadow
  in this mode. Source visibility still depends on the camera and draw range.
- Compare requested vegetation settings with requested settings, not biome-scaled
  density: lowland/polder/desert previously kept identifying their scene as stale.
- Pack the brick deduplication mask after the 22 diagnostic words. Native and
  temporal voxel layouts now require 15 and 16 storage buffers respectively,
  rather than 16/17. Preserve the 88-byte diagnostic readback prefix. This fixes
  the layout budget without increasing device requirements.

CPU validation passed: `cargo check --locked -p app --example terrain_fly`,
`temporal_layout_fits_sixteen_storage_bindings` (declared layout count, not just
used shader resources), and `volume_entry_points_fit_reduced_scene_binding_contract`
(Naga validation of native/temporal and brick/normal specializations). No GPU
was initialized by these checks. Build and shader/device creation on actual
hardware, image comparisons and timing repeats are still pending.

This is NOT complete world streaming or two-pass splat HZB occlusion. No measured
performance improvement or visual acceptance is claimed for these changes yet.
Next: validate retained ground at the source boundary, altitude changes and
recentring; inspect upload spikes (still a complete 23 MB ground upload); move
forest/heightfield refresh to persistent paged resources; then implement the
brick HZB work below. Do not run GPU validation concurrently with the user's game.

## Handoff (3 Nov 2026, evening)

Vegetation and terrain_fly frame time, as left by the session that did the
work from 2 to 3 Nov. Everything below is on `main`.

### State per step

| Step | State | Commits |
|---|---|---|
| Virtual tree assemblies (milestone 1), voxel splats lit by shade (milestone 3), cards as low-end fallback (milestone 4) | Done | see "Status (2 Nov)" below |
| GPU-streamed assemblies (milestone 2) | Parked, see below | |
| Shade split into terrain / voxels / rest pipelines, compile-time features | Done, same image | 6e420da, b6f7119, ce4f1d0, d368807, a986272 |
| Distant ground's sun march once per terrain_coarse quad | Done, 53-54 dB | e9afabf |
| Shared sun map 180 m with a 55 % fade; full/card tier hysteresis (Joep: shadow seam, blinking trees) | Done | 2fb7e25 |
| Voxel proxies 6 px loosening from 50 m (12 px at 200 m; was 25 px: "cubes") | Done | bd6ed85 |
| Round, jittered splats sized by opacity; normals rounded per pixel | Done, Joep still finds far trees blobby | ead0617 |
| Terrain: per-frame patch build budget, offscreen_factor 2.5 + 15 deg cone margin, reselect thresholds; terrain_fly yaw/pitch interpolated (mouse-turn hitches) | Done, CPU-tested, NOT measured on the GPU | 540c40e |
| Red trees: MOLTEN_DEBRIS (Growing Guns lava) shared bit 7 with part_wind's HISTORY_EPOCH | Fixed: molten only without PART_WIND | e5d216d |
| fps counter: scene frames only, mean frame time, worst frame of the last second | Done | c7f1dc8, 2afdffc |
| Crop screenshots: Shift+F12 or F12 twice, then drag a rectangle | Done, not tried by Joep yet | 063b5a6, 94f16a5, a979660 |
| Nanite-style voxels at about 1 px (HZB two-pass brick culling) | Designed below, not started | |

### Measured (M5 Pro, terrain_fly 2560x1440, `--bench 120`)

- **fps at the four playtest cameras:** 54.4 / 46.7 / 45.7 / 41.7 before
  bd6ed85. The finer proxies cost 5-10 %.
- **Cameras:** `build/screenshots/terrain-fly-1793630336696.json`,
  `-1793630348490`, `-1793630353351`, `-1793630402380`.
- **Joep's playtest spots:** `-1793694348617` (x 713 z 83) and
  `-1793695131755` (x 164 z -163).
- **Per-pass trace:** see "Frame time state" below. Terrain shade about
  5 ms is the largest pass, then voxel surface, splats and splat raster.
- **Sideways (2 views, 1440p):** about 14 ms per view-frame. Raster
  (4.8 ms) and cull (3.0 ms) outweigh shade there.
- **Joep's window (5120x2880, 120 Hz):** adaptive quality aims at 120 fps
  and falls to render scale 0.67 within 3 s. It renders 1713x964 and
  upscales 3x (bilinear, `render/src/present.rs`). Every voxel looks 3x
  larger than in a 1440p shot.

### Flags and environment

- **Render switches:** `KERN_SHADE_CLASSES=0`, `KERN_TERRAIN_COARSE=0`,
  `KERN_VOXEL_SURFACE=0`, `KERN_VOXEL_PASS=0` and `KERN_SKY_VIS_CACHE=0`
  switch the respective splits off.
- **Shadows:** `KERN_SHADOW_REACH` (180) sets the shared sun map's reach.
- **Pool log:** `KERN_POOL_LOG=1` logs tree pool churn.
- **A/B only, off by default:** `KERN_COARSE_FOOTPRINT_M` and
  `KERN_VOXEL_SELF_SHADOW_COARSER`. Neither was worth its image change;
  the shots are in `build/screenshots/ab-tradeoffs/`.
- **terrain_fly options:**
  - `--proxy-px` (6) and `--proxy-from` (50);
  - `--cards N` (the fallback card ring);
  - `--taa 0/1`;
  - `--profile low` (simple lighting, 720p, 30 full trees).
- **Render height:** `WARP_RENDER_HEIGHT=N` fixes the render height
  (0 = native).

### How to test headless

- **fps:** `terrain_fly --track --from build/screenshots/terrain-fly-<id>.json --bench 120 --size 2560x1440`.
- **A shot:** `--shot out.png` instead of `--bench`. Add `--size 5120x2880`
  with `WARP_RENDER_HEIGHT=965` to see what Joep's window shows.
- **Per pass:** use `xctrace record --template 'Metal System Trace'` and
  sum the encoder intervals; the parse scripts are described under
  "Measuring" above. `--bench` timestamps overlap on Metal, so compare by
  fps or xctrace.
- **Motion:** `--pan DEG --dump N DIR` writes N frames turning DEG per
  frame. Use it for shimmer, and with the HUD's "worst ms" for turn
  hitches.
- **Terrain churn on the CPU:** run `cargo test -p render --lib
  procedural_terrain::tests`.
- **Sideways A/B:** `sideways shot X.png --views 2 --size 2560x1440 --taa 64`.

### Known traps

- **Metal timestamps:** per-pass GPU timestamps overlap on Metal. Measure
  with fps deltas or xctrace.
- **Shade cost is kernel size:** it comes from register pressure, not
  runtime branches. Win by compiling code out (override constants, separate
  pipelines); a runtime skip gains nothing.
- **Shared instance flags:** all 32 bits of `GpuInstance::flags` are taken
  (see scene.rs). A new flag must share a bit safely, as `is_molten` does.
- **Joep's Mac:** someone may be playing on it.
  - Never open, focus or relaunch a game window unless he asks.
  - No GPU runs while he plays: they share his GPU, and his 5 fps report
    was partly that.
  - Ask before closing a window.
- **Cloud sessions:** they have no GPU, so they can only do CPU tests.
- **Splat budget:** 4M splats per view. Finer than about 4 px proxies
  overflows at wide views and trees vanish. That is the reason for the
  culling work below.

### Next tasks, in order

1. **GPU-check 540c40e (turn hitches) and e5d216d (red trees)** when
   Joep allows a GPU run:
   - `--pan 30 --dump 60` at his spots, and the worst ms against 1df3ca9;
   - check that the hidden-build swap does not pop visibly.
2. **Adaptive quality on 5K/120 Hz.**
   - Decide with Joep: target 60 fps, or an adaptive base above 1440p.
   - At 0.67 times 1440p a 5K screen shows everything 3x blurred.
3. **Nanite-style voxels at about 1 px of the presented image.** Joep:
   "Nanite is 1 voxel per pixel". The steps:
   - Group each level's splat cells (`voxel_forest.rs::splat_cells`, a flat
     list per level) into bricks of 4x4x4 cells, each with a local AABB and
     a cell range.
   - In `voxel_splats`, test bricks instead of cells:
     - frustum first;
     - then occlusion against an HZB, two-pass like Nanite. Pass 1 draws
       the bricks visible last frame (a per-view bit per brick) into the
       visibility buffer. Build the HZB from that depth plus the opaque
       scene (`pyramid.encode` in frame.rs already builds one). Pass 2
       tests the rest against it.
     - The splat count is then bounded by screen pixels times overdraw,
       not by tree count.
   - Choose the level so a voxel is about 1-1.5 *presented* pixels: divide
     `proxy_pixels` by the render scale.
   - Check that the cooked proxy has fine enough levels at 150-400 m
     (cell size in metres per level).
   - At 1 px, quads per voxel may cost too much in the vertex stage.
     Measure; a compute rasterizer of bricks is the fallback.
4. **Indirect visible-tree list for voxel splats and sun splats** (about
   +1.5 fps). With nothing drawn they cost 0.59 + 0.50 ms for 131k
   one-tree workgroups. A list needs a storage binding in voxel group 2,
   which is near the 16-buffer limit. Superseded if step 3 restructures
   voxel_splats.
5. **The near ground material** (`field_surface`, about 4 ms). No block
   dominates. Only a cheaper formulation of its layers, or image changes,
   remain.

## Why

Joep's terrain_fly playtest (2 Nov 2026, four cameras, 2560x1440, M5 Pro)
measured by fps with one part switched off at a time. Per-pass GPU
timestamps overlap on Metal and are not usable for this. Card trees cost
9-23 ms, voxel proxies about 9 ms and the painted far layer about 5 ms. The
cards are also the main source of shimmer when the camera turns. Every tier
boundary is a visible change of look. That is the pop-in Joep reported, and
forest_ladder measures the same boundaries.

## What Unreal does, and what we take from it

- **Voxels for distant foliage.** Clusters of up to 128 bricks of 4x4x4
  voxels, rasterized like opaque geometry with early Z, each voxel holding a
  normal distribution. We take this: our rasterized splats (voxel_forest
  raster) already draw stochastic opaque voxels. Since fda385c the opaque
  shade skips the pixels they hide.
- **Error, not distance, picks the representation.** A cluster switches from
  triangles to voxels when the voxel error is below the triangle error.
  aggregate_lod already does this per branch group of a full assembly; it
  has to cover every tree, not only the pool.
- **Assemblies.** Branches and sprays are instanced part clusters in the
  tree's hierarchy. Their transforms come from the bone (part) poses during
  cluster culling, and each part simplifies down to a single cluster.
- **No alpha-masked cards.** We keep the card mesh (plant_mesh_lod) only as
  the low-end merged fallback, Unreal's non-Nanite path.
- **Wind per part (rigid), off below a screen size.** Bounds stay tight and
  distant shadows stay cacheable.

## Milestones

Each milestone ships on main with forest_ladder (coverage, mass and tone
against the source, within ±10 % at every distance) and fps at the four
playtest cameras (`terrain_fly --track --from build/screenshots/terrain-fly-*.json --bench 120`).

1. **Assemblies as virtual instances** (a, about 4 days). Mapping the code
   (2 Nov) showed that part_wind already evaluates poses on the GPU, and only
   for assemblies that some view sees (`schedule_assemblies` sleeps the
   rest). The ~12-tree cap comes from residency instead: every placement of
   a full tree is its own `GpuInstance` plus a 160 B `PartAttachment`. A
   spruce is 5,169 placements and 61 poses, about 1.8 MB, and
   `MAX_ATTACHMENTS` is 65,536 / 5,169 = 12 trees. Two further caps sit
   behind it: 4096 volume bindings, aggregate groups and the
   `foliage_selection` uniform array.
   The change:
   - Placement rest transforms and mesh refs go into one table per variant.
   - Each tree keeps only its root, phase and a block of branch poses
     (61 × 96 B ≈ 6 KB).
   - A reserved instance-id range means (tree, placement).
     `instance_at(id)` composes `pose[tree, branch] × rest[variant,
     placement]` wherever a shader reads `instances[id]` today: 41 reads
     across 17 shaders, of which cull, raster, shade, motion and
     voxel_forest matter for trees.
   - Cull walks tree → branch group (skipping groups the aggregate selection
     put on voxels) → that group's placements, so only visible, non-voxel
     branches reach the cluster cut.
   - `update_parts` is dispatched indirectly over awake trees only.
   Expected fps: unchanged at equal tree counts, but full trees out to the
   distance where branch groups switch to voxels, with no pool.
2. **GPU instance hierarchy** (b, about 4 days). Trees become instances of an
   assembly (parts × placements) culled hierarchically: tree bounds, then
   branch groups, then part clusters. Built on the GPU from the placement
   output (render::placement) for about 5k trees in view. Instance memory per
   tree stays O(1): seed, pose index and variant. Expected fps: similar to the
   card tier at 150 m, without its shimmer.
3. **Per-cluster triangle→voxel switch to whole-plant voxels** (c, about
   4 days). Extend aggregate_lod's projected-error choice above branch groups
   to whole-plant voxel levels, so one tree goes smoothly from source
   triangles through part voxels to whole-plant bricks. The voxel bricks
   raster into the main visibility buffer and are lit by shade (no resolve or
   composite pass), with one normal per pixel from the stored distribution
   (a stable hash by voxel and pixel). Expected: −8 to −15 ms against cards
   plus proxies at the high cameras; ladder coverage within ±10 % at 40–600 m.
4. **Retire the tiers** (about 2 days). Cards become the low-end merged
   fallback (off by default) and proxies the deepest voxel level. The painted
   far layer stays only beyond the last voxel level, where a tree is under a
   pixel; forest_field already matches it statistically. Sideways moves over
   via its plant_population policy (the coordinator owns games/sideways).

## Low-end profile (Quest-like minimum)

Each layer keeps a cheap setting:

- Trees: the card mesh as a merged fallback, voxels from about 40 m.
- Terrain: KERN_SIMPLE_LIGHTING=1, or terrain_coarse with a lower threshold.
- Adaptive quality: render scale only, floor 0.67.
- Shadows: the shared map only.

## Risks

- **Pose buffer bandwidth** with 5k trees × 32 branch groups is about 10 MB
  per frame. Freeze distant poses and update them at a lower rate.
- **Voxel normal noise.** Unreal names this as their noise source; our splat
  hash is already per voxel and pixel. Measure shimmer with the pan test from
  the playtest (`playtest-diag`).
- **Shadow consistency** when representations change. Every representation
  casts from the same voxels past the near map, as now.
- **Hot-reload and four-view budgets.** The cull pass grows; the r55
  four-view budget plan has to be rechecked.

## Code map and findings (2 Nov 2026)

These are paths under `engine/crates/render/src/`. The numbers are for the
recipe spruce at 2560x1440 on the M5 Pro.

**How a full tree becomes instances today.** `vegetation_asset.rs`,
`SceneVegetation::append`, does the expansion:
- Each branch with a volume gets one owner `GpuInstance` (`PART_WIND`, no
  clusters), a `VolumeBinding`, an `aggregate_lod::Group` and a
  `foliage_representations` binding.
- Each placement gets its own `GpuInstance` and a 160 B `PartAttachment {pose,
  owner, rest}`.
- Each branch gets one `PartWind` pose.

A spruce comes to 5,169 placements, 61 poses and ~5,230 instances, about
1.8 MB resident per tree:

| Per placement | Bytes |
|---|---|
| `GpuInstance` | 80 |
| Previous-frame copy (motion.rs copies the whole buffer each frame) | 80 |
| Wind coefficients | 16 |
| `PartAttachment` | 160 |

| Per pose | Bytes |
|---|---|
| Descriptor | 304 |
| Pose | 96 |

**Caps.** These limit how many trees can be full:
- `recipe_forest::limits()` and `plant_streamed` fit the pool to
  `part_wind::MAX_ATTACHMENTS` = 65,536, which is 12 spruces.
- Behind that sit 4096 volume bindings (`foliage_volume_data::MAX_INSTANCES`,
  about 68 trees), aggregate group and volume ids < 4096, and the
  `foliage_selection` uniform `array<vec4<u32>,4096>` (cull.wgsl,
  voxel_forest.rs).
- `part_wind::RuntimeLimits`: 262,144 poses, 1M attachments, 512 MiB.

**part_wind** (`shaders/part_wind.wgsl`) already runs on the GPU:
- `schedule_assemblies` sleeps assemblies that no view sees and scales motion
  by projected size.
- `update_parts` evaluates the poses parent-first, but is dispatched over all
  assemblies, not indirectly.
- `update_attachments` writes `pose x rest` into every member `GpuInstance`
  of an awake assembly, even members whose branch group is on voxels.

Pose evaluation is therefore not the bottleneck; residency and per-placement
culling are.

**Culling.**
- `cull.wgsl` runs one 64-lane workgroup per instance.
- `instance_policy` hides a source member when its group's
  `foliage_selection` chose voxels.
- `cull_cluster` makes the DAG cut, using the aggregate's world triangle cut
  when one is set.
- There is no tree-level node above instances. `cull_groups.rs` has an
  optional Morton hierarchy over owners; `AssemblyDescriptor.sphere` is a
  per-tree bound.

**Instance reads.** `instances[...]` is read 41 times across 17 shaders. For
trees, these matter: cull (4), raster (1), shade (4), motion (1),
voxel_forest (10) and part_wind.

**Dead end: merged branch meshes.** I merged each branch's placements into
one cluster mesh, giving one instance per branch (branches are rigid under
part_wind).
- The image is the same (PSNR 43.9 dB).
- 12 full trees went from 23.5 to 27.2 fps, and 200 trees render.
- The cost is ~1 GB of geometry per spruce or pine variant (2.6 GB for six
  species), because the source is millions of separately modelled needles.
- 60 full trees ran at 16 fps.
- So: instanced shared parts as in Unreal's Assemblies, not merged geometry.
- The patch lives in session scratch only; it was not committed.

**Handover.** Full source geometry must hand over to part voxels
(aggregate_lod) much closer to the camera than today. Otherwise "full trees
everywhere" blows the raster budget, whatever the instancing.

**Measuring.** Per-pass GPU timestamps overlap on Metal and gave wrong costs:
"voxel composite" read 6-15 ms while removing it changed nothing. Measure by
fps with one thing switched off at a time (the `--bench` deltas at the four
playtest cameras in `build/screenshots/terrain-fly-*.json`).

## Status (2 Nov 2026, evening)

Done and on main:

- **Milestone 1:** virtual members, on by default in terrain_fly.
- **Milestone 3:**
  - Voxel splats go into the main visibility buffer and are lit by shade.
  - Branch voxels switch in at 32 px.
  - Whole-plant voxels shade themselves through their own cells.
- **Milestone 4 for terrain_fly:** the card ring is the low-end fallback only.
- **Group selection packed:** a view now holds 16384 groups, enough for 240 full trees by default.

**Milestone 2 (GPU per-tree selection, streamed full assemblies) is parked.**

- With the group budget fixed at about 270 resident full trees, letting the GPU choose among resident trees only replaces the CPU's nearest-240 assignment. That assignment now has hysteresis.
- The real gain needs streaming pages of assemblies, as Nanite does. That is a multi-day project whose payoff is detail at range, not frame time.
- Frame time is now dominated by shade and by the voxel pipeline's fixed passes. See the commit log from 0e1c803 on.

## Frame time state (3 Nov 2026)

terrain_fly at Joep's four playtest cameras, 2560x1440, M5 Pro:

| | cam 1 | cam 2 | cam 3 | cam 4 |
|---|---|---|---|---|
| Morning, 2 Nov | 18-27 fps | | | |
| a986272 (shade classes) | 49.1 | 43.2 | 43.0 | 39.2 |
| e9afabf and later (main) | 54.4-54.9 | 46.6-46.7 | 45.7-46.0 | 41.5-41.7 |

GPU time per pass, ms per frame (Metal System Trace, summed encoder
intervals, so passes that overlap count twice):

| Pass | cam 1 | cam 2 | cam 3 | cam 4 |
|---|---|---|---|---|
| total | 17.37 | 21.83 | 21.12 | 22.99 |
| shade terrain | 5.23 | 5.10 | 4.94 | 5.43 |
| voxel surface | 1.91 | 1.96 | 1.78 | 2.95 |
| voxel splats | 0.95 | 1.35 | 1.27 | 2.42 |
| terrain coarse | 1.92 | 1.96 | 1.80 | 1.92 |
| voxel splat raster | 1.12 | 1.93 | 1.95 | 1.90 |
| shade voxels | 0.75 | 1.44 | 1.44 | 1.50 |
| cull | 1.16 | 1.95 | 1.65 | 1.35 |
| raster | 0.55 | 1.92 | 2.16 | 1.30 |
| taa | 0.74 | 0.74 | 0.74 | 0.74 |
| voxel sun splats | 0.54 | 0.66 | 0.52 | 0.55 |

**Shipped:**
- **a986272:** the terrain shade class covers every TERRAIN material (Sideways
  included). Terrain and voxels are shaded before the main pass. Same image.
- **e9afabf:** the sun march past the shared map runs once per
  terrain_coarse quad and is compiled out of the terrain shader. +2-4 fps,
  53-54 dB.

**Tried and dropped** (cam 1 / cam 4 unless noted):
- **Sun transmittance cached in canopy slabs** (+25 MiB per view): 57 / 44.5
  fps, but distant ground came out too bright. The 6 m cells smear the
  vertical profile; against the real shadow map the correlation was 0.22,
  where the per-pixel march gives 0.41.
- **The quad march with the march still compiled into the terrain shader:**
  +0. Runtime skips do not free registers.
- **A lean terrain shader plus the full one on listed tiles:** identical
  image but slower, 54.8 -> 50.3 and 41.6 -> 38.7. About 60 % of the terrain
  work is in tiles that need the near ground material.
- **The voxel self-shadow one level coarser**
  (`KERN_VOXEL_SELF_SHADOW_COARSER=1`, off): +0.4 / +0.8 fps for a visible
  crown change, 32-48 dB.
- **A lower coarse-terrain footprint threshold** (`KERN_COARSE_FOOTPRINT_M`,
  off): no change even at 0. The near ground never takes the coarse path.
  A/B shots are in `build/screenshots/ab-tradeoffs/`.
- **Ground material ablation:** each block costs 0.3-1.1 fps (rock, far
  forest, roads, detail slope, grass canopy, scree, water, snow); all
  together cost 3.4. None can be compiled out per style without changing the
  image, because snow and water follow altitude and position.
- **Voxel splats:**
  - Skipping cell culling for trees fully in view: +0.
  - One global atomic per 64 cells: +0.
  - 64 trees per workgroup: +0.7 / -2.3 (cam 3) / -0.8 (cam 4), from load
    imbalance.

**Remaining levers:**
- **GPU-driven tree LOD and streaming (milestone 2).** Fewer voxel trees per
  frame shrinks splats, splat raster and voxel surface together, and this
  is the largest lever.
- **An indirect visible-tree list for voxel splats and sun splats.** With
  nothing drawn the two still cost 0.59 + 0.50 ms: one workgroup for each of
  131k trees. Compacting the list should give about +1.5 fps. It needs a
  storage binding in voxel group 2 (near the 16-buffer limit), or a list
  carved from the splat records buffer.
- **The near ground material** (`field_surface`, about 4 ms in terrain
  shade): only image changes or a cheaper formulation of its layers remain.
