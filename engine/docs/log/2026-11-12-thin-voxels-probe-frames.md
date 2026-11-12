# Thin bounce voxels, whole-probe captures, shade's texture count, 12 November

Machine: mancave, RTX 5070 Ti; `game-room.exe` (another project) held the
GPU at about 34 % throughout, so the timings below are relative pairs
from alternating runs, not absolute costs.

## Bounce through a shut door

- The voxelisation is thin (6-separating, Schwarz and Seidel 2010): one
  layer per flat surface. The walk starts half a voxel plus half a
  diagonal out (was a full diagonal).
- The engine picks the voxel size: the finest within 8.4 million voxels over
  the reserved bounds (5 cm over `interior_rooms`; the map passed 0.1 m).
- `leak_door` (flashlight in room 3 on the shut 4 cm door, camera in dark
  room 4), mean display luminance: 0.090 without visibility, 0.027 with the
  conservative 10 cm grid, 0.0025 now. Room corners unchanged.
- 5 cm voxels at 16 texels per pixel made the gather 3 to 5 times dearer
  (pairs: 3.4 to 14.6 ms against shade 17 to 65 ms). 8x8x8 bricks did not
  help. 8 texels per pixel did: 0.76 to 1.39 ms in the same runs, the same
  image after the temporal and spatial filters, `--pan` flicker 0.030
  against 0.029.

## Probes: all six faces in one frame

The swinging pendant showed at two places in a mirror-like floor: its
faces were captured frames apart. A probe's six faces now render in the
same frame (each with its own cull targets and view uniform), probes in
turn: four rooms all fresh within 4 frames. The pendant shows once.
Capture passes by timestamps: probe filter 0.08 ms in quiet runs.

## Shade's sampled textures

Chrome (browser house viewer on 6aab6c1) found shade's pipeline binding
26 sampled textures in the vertex stage while `portable` assumed 24, so a
device with 24 or 25 chose the full renderer and failed. The count now
comes from the layouts (`frame::shade_sampled_textures` over
`scene_layout_bindings` and `shade_layout_bindings`, which build them);
a test checks that a device one short takes the portable path.
