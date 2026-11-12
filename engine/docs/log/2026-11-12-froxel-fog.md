# Froxel fog, 12 November

Machine: mancave, RTX 5070 Ti. Scene: `interior_rooms`. Joep's F4: a cube
pattern in the pendant's shaft in the bounded volumetric light (a fixed 4x4
Bayer offset per half-resolution pixel, after TAA, so never averaged).

A quick fix (per-frame white noise and the fog's own history at half
resolution, 12 march steps) left coarse pixel noise in 4K crops: dropped.
Joep chose froxel fog. Guide: [volumetric-light.md](../volumetric-light.md#froxel-fog).

## Faults found on the way

- First image: the fog ten times too bright (meter EV 9.3 against 5.3
  without fog). Checks that held: a constant in-scatter of 100 nits gave the
  expected haze (integration right); the transmittance looked up per pixel
  matched exp(-density x distance) (lookup right); cell positions and
  orientation (uv and position patterns) right; the sun's shadow in the air
  indoors 0. The cause: a cell's sample lies anywhere in its slice (0.5 m
  thick at 5 m), and in a column that also shows the far side of the room,
  anywhere in front of that; air behind the wall or above the ceiling
  (outdoor sky and sun) entered the room's smoke.
- Reading the volume a slice short of each surface hid it partly and lost
  fog in front of every surface: dropped. The fix is at injection: each
  column skips cells behind its farthest visible surface, and each sample is
  tested against the depth buffer at its own pixel (with x/y jitter across
  the cell); a hidden sample leaves the cell's history unchanged, so the cell
  averages its visible air (not its visible share of it, which darkened
  edges). Meter: EV 5.03 with the smoke, 5.29 without; no band along the
  ceiling with the fog outdoors too.

## Measurements (4K)

fog inject 0.60 ms, integrate under 0.01 ms (r2_fog_shaft), shade 9.5 ms.
Suite: spots without fog unchanged against 511c3f5c; r2_fog_shaft,
flash_fog, r1_fog_door added (a "fog" field per spot).
