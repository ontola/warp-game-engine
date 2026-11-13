# Froxel fog: dark edges, a flickering horizon, flicker under a lamp, rooms, 13 November

Mancave (RTX 5070 Ti, Windows), `interior_rooms`, 1920x1080, fixed
exposure (`ROOMS_LIGHT=auto_exposure=0,ev100=N`), fog on. Joep's F4
captures outside (EV 16): a dark band just below the horizon that
flickers, and dark edges on the grass beside the door post and the
building's edge. Both go with the fog (Tab).

Flicker is measured with `interior_rooms --shot ... --flicker 16 --crop
x,y,w,h`: 16 more frames standing still (the pendant held), the mean and
99th percentile change of luma (sRGB 0..255) per pixel from frame to
frame, and `<out>_diff.png`, the largest step per pixel x16.

## Dark edges and the horizon band

Cause: a column of cells stored, behind its farthest visible surface,
"no light, full extinction". No pixel of that column reads those cells,
but shade's trilinear lookup does for the pixels of the neighbouring
column: past a wall's edge the column beside it sees the ground 20 m
farther, and on the ground below the horizon every row of cells sees
farther than the row beneath it. Those pixels mixed in dark air. Injecting
the real medium there instead removed both (checked), but would let the
air behind a wall leak into a room's fog through the same filtering.

Kept: those cells are marked (alpha -1, carried through the integration)
and every lookup (shade, glass, the history) filters trilinearly over the
unmarked cells only. A sample hidden at its own pixel no longer writes
black when it has no history: it moves to the ray of the tile's farthest
pixel. History is not read from marked cells (newly seen air starts from
this frame's sample).

| spot | crop | before mean / p99 | after mean / p99 |
| --- | --- | --- | --- |
| outside, wall and horizon (Joep's F4) | 0,440,1040,140 | 0.395 / 2.93 | 0.032 / 0.93 |
| outside, wedge and horizon (Joep's F4) | 740,470,1180,120 | 0.419 / 3.00 | 0.005 / 0.07 |
| room 1, horizon through the door (EV 13) | 760,380,400,300 | 0.186 / 2.00 | 0.095 / 1.00 |

Not changed: the haze beside the building in the first F4 is the fog's
bounds box (one box over the L-shaped house includes the outdoor corner
by room 4).

Suite: `out_fog_wall`, `out_fog_wedge`, `r1_fog_horizon`.

## Flicker under a lamp

Room 2 under the pendant (the pendant held still), crop 760,480,320,320:
0.683 / 5.14 with fog, 0.336 / 2.21 without (TAA and shading), 0.444 /
2.35 with the fog's jitter frozen. The jitter's variance, taken apart:

| change | mean / p99 |
| --- | --- |
| main (after the edge fix) | 0.683 / 5.14 |
| the lamp's inverse square as its exact mean along the cell | 0.640 / 4.86 |
| without the lamp's shadow in the fog (diagnostic) | 0.633 / 4.86 |
| no xy jitter (diagnostic) | 0.575 / 3.86 |
| each cell's light split into up to 8 parts along it near a lamp | 0.596 / 4.07 |
| history read at the cell's centre, not the jittered point | 0.479 / 2.93 |
| the same without the split | 0.492 / 3.07 |
| and the jitter stratified: golden-ratio steps per cell, interleaved gradient noise along the slice | 0.468 / 2.93 |

The blocks in the difference images were cells: the history was read
trilinearly at the jittered sample point, so every frame each cell took a
random share of its neighbours, a random walk. Kept: history at the
cell's centre, the inverse-square mean (exact, one atan pair per lamp) and
the stratified jitter; the split, which bought 0.013 for up to 8 light evaluations, was dropped.

| spot (EV) | main | after |
| --- | --- | --- |
| room 2, pendant crop (6) | 0.683 / 5.14 | 0.468 / 2.93 |
| room 2, whole frame (6) | 0.181 / 1.79 | 0.158 / 1.21 |
| room 3, ceiling lamps (6) | 0.132 / 1.00 | 0.118 / 1.00 |
| room 4, flashlight beam (6) | 0.053 / 1.00 | 0.010 / 0.72 |
| outside, wall and horizon (16) | 0.032 / 0.93 | 0.009 / 0.28 |

What is left at the pendant (0.47 against 0.44 with the jitter frozen) is
mostly the frame's own (0.34 without fog).

That 0.34 without fog goes to 0.002 with `gi=0` and does not change with
traced reflections, TAA, contact or reflection occlusion, near shadows or
the live bounce (`KERN_RSM=0`) off: it is in the GI term, next to find.

Suite against main (mean luma difference, 0..255): the fog spots' stills
within 0.26, and the cone under the pendant loses its grain while turning
(`r2_fog_shaft_turn`). Spots without fog, which this change cannot touch,
still differ by up to 3.8 (`r3_lamps_still`) and far more in their turn
shots: dynamic resolution follows the GPU, which the demo window shared,
so those differences are run to run, not the change.

## The haze beside room 4, and the flicker without fog

The fog's one bounds box covered the yard in the L of the house. The
medium now takes the rooms the scene declares for its reflection probes
(`FogBounds::Rooms`), with the probes' own fade: one definition of a room
(`room_presence`, rsm.wgsl). The demo's room boxes stopped at the walls'
faces, so a doorway lay in no box; they now reach through each door to the
middle of its wall, as `architecture` builds rooms. The haze in Joep's
first F4 is gone; the smoke runs on through the doorways.

The flicker without fog (0.34 / 2.2 under the pendant) is not lighting:
`gi=0`, or the irradiance volume alone off, floods the room with the open
sky and the frame clips to white at EV 6, so its 0 meant nothing. Between
two frames 97.7 % of the pixels change by at most one step of 255; the
rest are the bulb's and the shade's edges (bloom and TAA on the bright
bulb), not visible at 1x. Left as it is.

The gradient interpolation's shading cost could not be measured: with the
demo window on the same GPU, shade took 23-34 ms per 4K frame from run to
run (8-10 ms on an idle GPU).
