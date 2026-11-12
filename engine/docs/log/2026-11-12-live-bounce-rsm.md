# Live bounce of moving lights (reflective shadow maps), 12 November

Machine: mancave, RTX 5070 Ti, Windows 11. Scene: `interior_rooms`.
Replaces the example's virtual point lights (one soft light where the
pendant's and the flashlight's beams land) and the pendant's uplight with
reflective shadow maps in the engine (`render::rsm`). Guide:
[interior-light.md](../interior-light.md#live-bounce-of-moving-lights).

## Measurements

Gather cost, 4K, flashlight on the white block in room 3 (r3_flash_box):

| Version | rsm gather | rsm filter |
| --- | --- | --- |
| quarter resolution, 16 texels, 0.1 m voxel walk | 3.74 ms | 0.33 ms |
| same, no visibility walk (`KERN_RSM=open`) | 0.63 ms | 0.33 ms |
| brick level (4^3) in the walk | 4.05 ms | 0.33 ms |
| Russian roulette on the walk | 3.60 ms | 0.33 ms |
| an eighth of the resolution above 1600 lines (480x270) | 0.87 ms | 0.19 ms |
| final (derived walk ends, voxel boundary fix) | 0.70 ms | 0.18 ms |

The brick level did not pay at quarter resolution (rooms are small, most
walks start and end in occupied bricks); it stays, as the walk is no
slower. The cost follows the number of walks: the eighth resolution
settled it. The bounce has no detail finer than its filter's reach.

Flicker (`--pan 240`, 480x270, fixed EV 5.5, room 3, flashlight): mean
frame-to-frame change of the mean luminance 0.028 (bounce off 0.029), max
0.153 (0.139).

Leaks (suite, 1920x1080, mean display luminance or metered EV):

| Spot | bounce off | no visibility | visibility walk |
| --- | --- | --- | --- |
| leak_r3_ceil (pendant, room 3 ceiling by room 2) | EV -2.59 | EV -0.07 | EV -2.59 |
| leak_door (flashlight in room 3 on the shut door, camera in room 4) | 0.000 | 0.090 | 0.027 |

`leak_door`'s rest: the lit jamb between the wall face and the door lies
within a voxel diagonal of the 4 cm door, where the walk's corner rule
(a solid run at an end, at most a voxel diagonal long, is the neighbouring
surface) lets it through. With a fixed 0.3 m end tolerance (the first
version) the rest was larger.

Ceiling against the lit spot (`ceiling_wall`, room 4, flashlight 2.5 m
from the wall): display values 20.6 % (the virtual point light: 22 %, before
that 53 %), linear after tone mapping 3.6 %. The spot sits near white, so
the display ratio overstates the ceiling.

## Faults found on the way

- A NaN normal on the gather's last row whenever the view height was not
  a multiple of the gather scale (the clamped neighbour was the pixel
  itself): one NaN spread through the filter and the exposure meter read
  EV 17.8 at 480x270. The normal now takes the other side at the edge and
  is zero where the depth gives none.
- A surface exactly on a voxel boundary (a wall face on the 10 cm grid)
  could round out of both voxels beside it in the triangle-box test, and
  the walk passed it. The test now uses a hair over half a voxel (1e-4);
  `leak_door` fell from 0.036 to 0.027.
- The gather first used the depth texture's size, which can be larger
  than the view; it uses the view's size, as shade does.

## Regression suite against 83180f4c

Unchanged (mean |difference| under 0.3 of 255): r1_door, look_down,
r2_bands, r3_chevron, r3_lampstreak, r3_sparkle, r4_dark_open,
r4_dark_shut, r4_door_view. Changed by the bounce: flash_room3 (10.1),
r3_flash_box (7.2), r2_corner (6.4), flash_wall (2.2), r2_ceiling, r2_box,
r2_doorway, r3_lamps (1.1 to 1.5).
