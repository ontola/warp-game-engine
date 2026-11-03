# Terrain fly: retained meadow and voxel culling, 3 November 2026

## Acceptance

The black-frame regression is fixed in 6520abf: the temporal voxel surface pass
now uses the temporal layout, not the native layout. A real GPU screenshot was
captured after the repair. Reload now also updates the surface and selection
pipelines transactionally.

The new culling is **opt-in, not accepted as a default performance improvement**.
It is a conservative prototype, not Nanite parity or completed world streaming.

## Implemented

`KERN_SPLAT_BRICK_HZB=1` enables:

- Shared 4x4x4 cell catalogs per asset level; no duplicated catalog per tree.
- GPU-compacted visible-tree lists, bounded by the existing raster-unused tile
  allocation. Indirect dispatch processes those trees. Overflow falls back to
  the full list, never silently truncates it.
- Two phases using the current frame: nearby voxel trees seed depth alongside
  the opaque scene; a reversed-depth minimum pyramid tests the remaining bricks.
  This does not use the planned previous-visible-brick history. The 150 m seed
  partition changes scheduling only, not the selected LOD, density or material.
- Conservative world bounds include camera-facing billboard size and jitter,
  also for nonuniform scale/shear. Near-plane intersections stay visible. Sky
  holes prevent occlusion. Odd mip sizes use shift-and-clamp indexing.
- Separate GPU scopes for the seed/list pass, pyramid, second list/cull pass,
  and second raster pass. List dispatch and copies are included in the scopes.
  The second raster currently redraws the retained seed records: correct but
  extra work, worth removing in a later iteration.

`WARP_TERRAIN_STREAM_GROUND=1` retains meadow buffers, geometry and pipelines
while a worker samples the new ground. It avoids 100 m near-tree scene rebuilds
(the existing full-tree pool already moves). Outer forest refresh at 300 m still
rebuilds the renderer, and meadow ground is still a full 23 MB upload. These
remaining costs can cause movement stalls; neither is solved by voxel culling.

Snapshot `--from` is applied before the initial scene build. Previously a replay
started one scene, marked it stale, then built another during measurement.
`WARP_BENCH_WARMUP=60` permits a longer warmup; default remains two frames for
compatibility. Bench wall FPS includes readback; do not use it as gameplay FPS.

## Verification and measurements

26 voxel unit tests passed, including packed cell preservation, device binding
counts, shader specialization, bounds, overflow and existing volume contracts.
Both native and temporal GPU paths were exercised by terrain_fly captures.

The first un-compacted prototype regressed the hill camera from about 4.73 to
5.54 ms covered GPU time. The compact prototype also failed to improve the tested
hill/ground scenes. Thus culling remains off; reduced geometry quality must not
be credited as an optimization of this path. See the [evidence JSON](evidence/voxel-brick-culling-2026-11-03.json)
for final executable hash, per-run medians, per-pass medians and sample counts.

Hardware: RTX 5070 Ti, driver 617.14, Windows. One view, 1713x964 internal/output
for headless timing, alpine seed 1, grass and track enabled. This is not the
4K/four-player target. Three alternating off/on pairs per camera, same executable,
160 frames per run, first 60 excluded. GPU async-log tail can contain fewer than
100 samples; counts are recorded. User was watching a film, so background GPU
load was not controlled. No interactive game window was opened for these runs.

Camera hill: position (771,1030,-27), yaw 35 degrees, pitch -15 degrees. Camera
ground: terrain_fly's track start. Initial 8-frame, 5-degree-per-frame hill pan
was pixel-identical outside the HUD between off/on; that is a short correctness
check, not full motion/wind/disocclusion coverage. Final ground pan uses 30 degrees
per frame to exercise newly visible regions and near geometry.

Local raw results are C:/dev/warp-bench/final-{hill,ground}-[0-5].{log,jsonl},
C:/dev/warp-bench/final-ground-motion-{0,1}/, and the earlier compact-* runs.

Final median of three run medians (covered GPU scopes):

| Camera | Default | Brick HZB | Delta |
|---|---:|---:|---:|
| Above forest | 4.741 ms | 5.545 ms | +0.804 ms |
| Ground / road | 9.967 ms | 10.130 ms | +0.163 ms |

All eight final ground-pan PNGs were pixel-identical outside the top 180 HUD
pixels. This is tone-mapped image agreement, not a raw visibility/depth proof.
The native `--taa 0` capture also rendered successfully. The current scene still
has dark slopes and noisy foliage; fixing the black pipeline does not establish
acceptable lighting or image quality. Screenshot HUD FPS is not benchmark evidence.

## Next useful work

1. Persistent forest/heightfield resources and budgeted partial meadow uploads:
   avoid complete renderer construction during movement.
2. Near source-geometry work: raster cost around 4 ms dominates the ground view;
   the tested distant-voxel culling cannot remove those triangles.
3. Make the culling scheduler cheaper for coarse levels: testing tiny coarse
   bricks can cost more than emitting them. Reuse selected levels/tree lists,
   avoid drawing seed records twice, then measure representative dense views.
4. Only then evaluate finer voxel error and previous-visible-brick scheduling.
   The existing 4M-splat cap still applies; this work does not establish safe
   one-pixel voxels or GPU-streamed full assemblies.
