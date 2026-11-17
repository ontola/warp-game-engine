# The fog's cone edge as a staircase near the camera, 14 November

Mancave (RTX 5070 Ti, Windows), `interior_rooms`, idle GPU, 1920x1080,
EV 6.3 fixed, fog on. Joep's F4 of 12 November (camera 0.458,1.65,-8.299,
yaw -15.859, pitch -0.230), the pendant swinging to the same pose (215
frames from start), 4x crop of the cone's edge below the bulb
(700,150,300,150). Suite spot `r2_fog_shade` (its new field `t=0.25` sets
the pendant's clock at the spot's start).

The edge of the cone under the shade, a metre from the camera, showed as
blocks of the cell's size (12 pixels at 1080p). Held still at that pose it
was smooth in this crop, so first the moving case was taken apart:

| tried | the blocks |
| --- | --- |
| history 0.5 kept instead of 0.9 (diagnostic) | stay, other shapes |
| the lamp's shadow averaged over four points along the cell | unchanged |
| no move of hidden samples to the tile's farthest ray (diagnostic) | unchanged |
| four samples per cell per frame (diagnostic, 4x the inject) | left side smoother, right side unchanged |
| cells behind a column's surface filled from their seen neighbours in the integration | unchanged |
| each pixel reading the volume up to half a cell off its spot, per frame | gone: a gradient |

So it is the grid's resolution where the edge crosses the cells
diagonally just in front of the camera, not the history, the shadow or
the marked cells. Kept the last (froxel_fog.wgsl `froxel_fog`: interleaved
gradient noise moved per frame, TAA averages it); a faint dither remains
in the 4x crop. The diagnostics were not kept.

Frame-to-frame luma change (mean / p99, 16 frames, pendant held):

| spot | main | jittered lookup |
| --- | --- | --- |
| room 2, pendant crop | 0.467 / 2.93 | 0.478 / 3.07 |
| outside, wall and horizon | 0.002 / 0.00 | 0.003 / 0.00 |
| the cone under the shade | 0.085 / 1.93 | 0.090 / 1.93 |

Seen on the way: with fog off, the swinging shade's inside still shows the
green grain of [2026-11-12](2026-11-12-moving-reflector-grain.md) in
this view (lit by the bulb, a metre away); the 14 November check
([pendant-shade-grain](2026-11-14-pendant-shade-grain.md)) measured with
fog on.
