# Vegetation pop-in: handoff (18 November 2026)

For the next agent on vegetation and terrain primitives plus performance.
Branch `claude/project-thread-s7vbe4` at 82e597d7 (draft PR #61). Main is at
34113f77 (PR #59 merged). Earlier results of the day:
[2026-11-18-vegetation-cost-and-tiers.md](2026-11-18-vegetation-cost-and-tiers.md).

## Goal

Joep watched the look-suite clips and said the LOD pop-in is very visible,
even in small clips. That is the top priority. Rules he set: fix the cause,
no distance pull-backs, no clamps that hide it, fix the engine and not the
map. Overall goal stays: Nanite-style GPU vegetation, a realistic Swiss
forest, 60 fps, on all kinds of hardware.

## Where things stand

Performance (Mancave, 1440p native, `covered_ms` p50, frame 300; poses
crowns / near / over): 21.2 / 19.3 / 18.4 ms before, 16.9 / 17.1 / 14.8 ms
on main. Voxel page cache 332 to 146 MB.

Pop-in tooling is pushed (82e597d7), described in
[render-suite.md](../render-suite.md) under "Finding pop-in":
`terrain_fly --dump N DIR --dump-cut DIR/cut.jsonl` and the render example
`pop_meter`.

## Measured so far (Mancave, 1440p)

- Flight at 80 m, 40 m/s, settled and cold start: close to clean. Largest
  pop 48x64 px, score about 145, on trees at 850 to 1300 m. Page streaming
  causes much representation churn on a cold start (shadow WholeMip 59 to
  503 per frame) but no extra visible pops.
- Walk, 4 m/s, 360 frames: Refine 44.5 per frame, Appear 13.4, Disappear
  13.6 (all beyond 400 m, about 1% coincide with a visible spike, probably
  HZB cull flips), WholeMip 1.8; VoxelMip, VoxelSource, Fallback 0.
- Camera scale state[79] is constant in every run; shadow scale state[78]
  constant on the walk and one step in the flight. Scale stepping is not the
  cause of visible pops.
- The strong pops are near field:
  1. Walk frame 38/39, score 11747, 576x208 px, luma 18 to 48: the whole
     sun-shadow pattern on meadow and trunks changes in one frame while no
     scale or shadow selection moves.
  2. Walk frame 284: needles of a near branch (within 25 m) appear in one
     frame, a Refine on a near tree.
- An earlier flight at 48 m scored lens-adjacent branches, not LOD. Use
  paths without anything within a few metres of the camera.

## Suspects for the sun repattern (not confirmed)

Read in `render/src/frame.rs` and `render/src/shadow_damage.rs`:

- (a) An outer clipmap level (k > 0) moves by a 32-texel tile
  (`clipmap_level_grid`) while it has deferred damage pending. Then
  `Damage::encode_maps` sets `force` and the whole level redraws in one
  frame.
- (b) `clipmap_base` changes (redraws every level at new extents).
- (c) The policy invalidation near frame.rs line 5325.
- (d) Stale retained tiles. In `shaders/voxel_forest.wgsl`
  `volume_sun_splats_impl` the level a tree splats depends on page
  residency (`while level<ceiling && volume_splat_cells[...].y==0u`) and on
  `selected.y`. The damage history (`shaders/shadow_damage.wgsl`, `track`)
  snapshots only the tree record and the slot's choice word. A finer voxel
  page that arrives (per-level paging, `Spec::voxel_page_finer`) does not
  mark the tile, so the tile keeps the coarser level until something else
  redraws it, and a forced redraw swaps a whole area at once.

Prepared fix for (d), not written: give `track` the splat cell ranges
(`SplatRaster::cells`, binding 6, the pass then has 6 storage buffers) and
store a mask of resident levels per tree in `now.words[7].x`. A change marks
the tree's tiles in every map. Confirm (d) first with the test below.

## Queue on Mancave (was running, results not in yet)

1. Look-suite lowland tree clip at 1440p with the pop meter (the clip Joep
   watched).
2. Walk frames 0 to 60 at a62ef085 and 2c86ddf1: is the repattern older
   than last night's work?
3. Walk frames 0 to 60 with `KERN_SUN_DAMAGE_REPORT=1`, and again with
   `KERN_SUN_STAGGER=0`, with the per-level force/shift log: which slot goes
   fully dirty on frame 38/39?
4. Frame 38 normal against frame 38 with every slot invalidated on frame
   37, and diff the level's depth: stale tiles (d) or a different redraw.
5. Per frame: page cache generation, pages admitted or evicted, and trees
   whose resolved sun level changed while their tile was not dirty.
6. Frame-284 tree with the parent forced and with the children forced:
   luma, coverage and silhouette difference.

## Next steps

1. Fix the sun repattern at its cause once the test names it.
2. Near needles: make parent and child representations agree (luma,
   coverage, silhouette), then crossfade what remains.
3. Confirm the far Appear/Disappear churn is invisible HZB flipping.
4. Before and after clip with a pop graph for Joep, in the look suite.
5. PR #61: the per-pipeline-layout storage buffer work is done except the
   three voxel layouts ("layout voxel forest", "layout voxel sun splats",
   "layout cached voxel sun splats" fail `forest_layouts_under_mac_storage_limit`
   at 8). Then run the forest with `KERN_WEBGPU_LIMITS=mac` and `=spec`,
   update the PR body and merge.

Smaller open items: low tier balls near the camera, brown proxy albedo, a
shadow splat cap per tier if Joep finds the image too dark (about 7% darker
at the over pose), wind, rebuilding splats every frame, CPU submit 4.6 ms.

## Lessons

- Raising the shadow scale also coarsens root-proxy sun mips. Never use it
  for slot pressure (`fast_adapt` in `foliage_refine.wgsl`).
- Fine 3.75 cm cells everywhere: measured and parked (+576 MiB, +4 to 10 ms,
  looks like 7.5 cm).
- WebGPU counts storage buffers per pipeline layout per stage, not per entry
  point.

## Working notes

- Cloud container: no GPU, lavapipe
  (`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`).
  `cargo test --release -p render --lib` (920 pass); ignored GPU tests with
  `-- --ignored --test-threads=1 foliage` take about 7 minutes.
- Real numbers come from Mancave. No GPU runs there while Joep plays.
- CI fails because there are no runners. It is not a merge blocker.
- Poses: crowns `-21,-116,25,0,-0.55`; near
  `-121.3255,-5596.4775,6.3668,3.944799,0.062000`; over
  `389.0229,-573.3995,48.2914,0.419595,-0.415399`.
- Look suite: https://claude.ai/artifact/Em4tqNhszDKj7HH5PqrexR
