# Froxel fog: dark edges, a flickering horizon, flicker under a lamp, 13 November

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
