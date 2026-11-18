# Forest pop-in, 18 November 2026

Joep: level-of-detail pop-in is clearly visible, even in the small look suite
clips. This entry records the measurement so far and is the handoff for
whoever continues it. Nothing is fixed yet.

Machine: Mancave (RTX 5070 Ti), branch `claude/project-thread-s7vbe4` at
82e597d7 (main 34113f77 plus the storage-layout work and the tooling below).
Scene: `terrain_fly --seed 1 --style swiss --spruce-forest`, 2560x1440,
headless `--dump`, 300 still frames first unless noted.

## Tooling (82e597d7)

- `terrain_fly --dump N DIR --dump-cut DIR/cut.jsonl`: per frame every GPU
  forest tree's cut for camera and sun (`Runtime::cut_snapshot`) and its
  change since the last frame (`CutSnapshot::change`): Appear, Disappear,
  Fallback, WholeToCut, CutToWhole, WholeMip, Refine (the node set changed),
  VoxelSource, VoxelMip. Camera changes of trees whose root is on screen carry
  screen position and distance. Also per frame: scales (`state[79]` camera,
  `[78]` shadow, `[85]` source), used totals, refusals (`[76] [75] [86]`),
  fills (`[74] [77]`), shadow slots (`[81] [88]`), sun clipmap levels
  (`Renderer::sun_clipmap_log`: deferred, drawn, force, shift) and the voxel
  page cache counters.
- `cargo run --release -p render --example pop_meter -- DIR --radius 16
  --crops DIR/crops`: one-frame changes that a ±16 px block motion search does
  not explain, as spikes over the neighbouring frames. Writes `pops.json` (per
  frame score, 20 largest regions with luma before and after) and
  `grids.bin` (per frame residual and spike per 16 px block, f32).
- Scratch scripts (not in the repo; easy to rewrite): `pops.sh` runs a dump
  then the pop meter. `popjoin.py` joins `pops.json` regions with
  `cut.jsonl` changes whose root projects into or just below the region,
  at frame ±1.
- Data: `C:\dev\perf\2026-11-18-popin\{walk,fly,flyhigh,flyhighcold,treeclip,walk60r}`
  (PNGs, `cut.jsonl`, `pops.json`, crops). Delete when done.

## Runs

| run | path | frames | camera changes on screen per frame | largest pop region |
| --- | --- | --- | --- | --- |
| walk | `--at -21,-116,1.7,0.4,0 --speed 4` | 360 | Refine 44.5, Appear 13.4, Disappear 13.6, WholeMip 1.8 | frame 39, 576x208 px, score 11747 |
| flyhigh | `--at -400,300,80,2.4,-0.4 --speed 40` | 300 | Refine 90, Appear 23, Disappear 27, WholeToCut 4.5 | 48x64 px, score 146 |
| flyhighcold | same, no warm frames | 300 | Refine 219, WholeMip 56, WholeToCut 16, Appear 36 | 48x64 px, score 143 |
| fly (rejected) | 48 m, flew into crowns | 300 | | branches at the lens, not LOD |

- Camera scale `state[79]` never moved in any run (walk 6.128, fly.high
  4.000 = the floor). The error-cut scale is not the cause.
- Shadow scale `state[78]`: walk 0 steps; fly.high one step (frame 236,
  4.373 to 4.635, shadow fill crossing 0.92); blocked words always 0.
- Source scale `state[85]`: about 10 at 80 m; the 83 to 474 drift seen
  once came only from the rejected path through crowns.
- Tree changes are mostly invisible: looking up each change's screen blocks
  in the spike grid, only about 1 % of Refine, Appear and Disappear changes
  coincide with a spike of 4 luma or more. Appear/Disappear happen only at
  400 m and beyond (mostly 800 to 1600 m); 378 trees flip 10 or more times in
  360 frames (HZB occlusion flips, not yet confirmed).
- From 80 m the forest is close to clean, settled or cold: page streaming
  causes a lot of representation churn (shadow WholeMip 503 per frame cold)
  but no extra visible pops.

## The visible pops: near field

1. **Walk frame 38 to 39: the sun shadow repatterns at once.** The meadow
   shadow, the dappled spots and the blotches on a near trunk all change in
   one frame (luma 18 to 48 over 576x208 px). Reproduced in a second run
   (`walk60r`, frame 39, score 24080). At that frame: no change in
   the shadow cut (per tree shadow consumer), shadow scale constant,
   camera scale constant. `KERN_SUN_DAMAGE_REPORT=1`: no slot fully
   redrawn (4096 tiles) at 38/39; level 0 (slot 1) dirty tiles jump from
   127 to 399 and 467, and changed sources in row 0 from about 60 to 358.
   Slot 2 is fully redrawn at frames 42 and 43 (one frame offset of the
   report not checked). Open suspects (coordinator):
   - an outer clipmap level crossing a 32-texel tile with pending deferred
     damage: `shadow_damage.rs encode_maps` forces a full redraw;
     `frame.rs` `clipmap_level_due`, `clipmap_level_grid`;
   - `clipmap_bases` changing;
   - a policy invalidation (`frame.rs`, `invalidate_slot` for all slots);
   - stale tiles: `voxel_forest.wgsl volume_sun_splats_impl` picks the sun
     level from page residency (`volume_splat_cells[..].y==0` steps
     coarser) while `shadow_damage.wgsl track` only snapshots the tree
     record and choice word, so a finer page arriving does not dirty the
     tile; a later redraw then swaps a whole area at once.
2. **Walk frame 284: needles of a branch right in front of the camera
   appear in one frame** (Refine on a tree within 25 m).
3. Look suite lowland tree clip (`--style lowland --grass --seed 1 --at
   -390.4,573.0,10,-5.4018,0 --speed 38`, 300 frames after 150): rendered
   at 1440p with `cut.jsonl`; the pop meter has run, results not yet read.

## Next steps

1. Walk frames 20 to 60 with the new binary (82e597d7 has
   `sun_levels` in `cut.jsonl`, the `walk60r` run was made with the
   previous binary) with `KERN_SUN_DAMAGE_REPORT=1`, then the same with
   `KERN_SUN_STAGGER=0`: which level forces, scrolls or redraws at 38/39.
2. History: the same walk at a62ef085 and at 2c86ddf1 (worktrees
   `C:\dev\wt\pop-a62` and `C:\dev\wt\pop-2c86`, both built; remove them
   afterwards). Does frame 39 repattern there too?
3. Per frame page admissions (finer sun levels) against dirty tiles, for the
   stale-tile suspect.
4. Frame 284 tree and one voxel to needle switch: render the frame with
   parent and with children forced; luma, coverage and silhouette inside the
   tree's box (is it a brightness or density mismatch, or does it need a
   crossfade).
5. Read the tree clip's `pops.json` and join it with its `cut.jsonl`.

Do not hide pops with distances or thresholds; find the mechanism.
