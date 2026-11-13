# Room bake: bias of the smoothing, noise of small openings, kinks, 13 November

Branch `hermite-probes`. Mancave (RTX 5070 Ti, Windows), `interior_rooms`,
14,260 probes, startup bake of two door states with the demo window
running on the same GPU. Follows the open item in
[2026-11-13-gpu-room-bake](2026-11-13-gpu-room-bake.md).

## The kink at the probe spacing

The "step" on room 2's wall (`r2_bands`) without the bake's final
smoothing is a kink: along a row of the crop the wall is flat (31-33 of
255) and then rises linearly. Every weight of shading's eight-probe lookup
is continuous (trilinear, reach filtered trilinearly, backface, DDGI), and
the probes along that wall are smooth (0.2 0.2 0.2 0.2 0.3 0.4 0.5 0.6
0.8, lamp bounce): piecewise-linear reconstruction of light that curves.
Gradient interpolation (each probe continued along its monotonized-central
gradient to the neighbours it sees, blended with C1 weights) removes it:
fixed exposure, no smoothing, the row runs 29 30 29 30 29 31 ... 32 34 33
35 38 36 39 against trilinear's 29 30 30 31 ... 44 43 46 48 51. Minmod
(tried first) put the flat part back; one-sided differences at corners
did not overshoot.

## The smoothing's bias

At fixed EV 6, with each image term on its own: the wall is lit by
indirect daylight from room 1 through the doorway (lamps off: no change;
RSM off: little). The smoothing passes moved bounced sky up 1.25x and
bounced sun up to 2x at room 2's probes. Against brute-force paths on 24
surface points per crop row (20,000 paths each, same reflectance and
bounces, `room_bake::reference_probe`), mean over the row:

| | reference | trilinear + smoothing | trilinear | gradients + smoothing | gradients |
| --- | --- | --- | --- | --- | --- |
| `r2_bands` bounced sky | 0.00020 | 0.00035 | 0.00025 | 0.00032 | 0.00020 |
| `r2_bands` bounced sun | 0.00003 | 0.00014 | 0.00006 | 0.00012 | 0.00003 |
| `r2_corner` bounced sky | 0.00023 | 0.00041 | 0.00029 | 0.00041 | 0.00029 |
| `r2_corner` bounced sun | 0.00003 | 0.00006 | 0.00003 | 0.00006 | 0.00003 |

The dark band in `r2_corner`'s corner is real: the reference drops
towards the corner too.

## The noise the smoothing also hid

Without it the suite showed blotches at the probe spacing (`r2_box`,
`r2_pendant_table`, room 4's ceiling). They fade with more bounce rays
(512 -> 2,048 -> 8,192; 23 s), but a variance-guided filter on the last
bounce did not remove them: each spans several probes. Their source is the
direct sky: 32,768 sky rays with 512 bounce rays gave smooth walls. Room 2
sees the sky through the door and then room 1's window, a tiny solid angle
2,048 rays hit a few times; that noise spread through every bounce's
gathers.

Kept: a second direct-sky estimate with 8x the rays for probes within 4
steps of open space of one that saw any sky (as clean as 32,768 rays for
all probes; 16x cost 7.2 s, 8x 5.1 s), and the variance-guided filter
after every bounce instead of the blind average, for sky and sun and for
each lamp (room 4's ceiling blotches were the lamps' bounce through the
shut door's gaps).

## Tests

- CPU bake against paths in a 4 m room with a door: within 1-4% per
  channel.
- GPU bake, two rooms (window with sun patch; a room lit only through the
  door): within 0-7%.
- GPU lamp bake, lamp in room 1: room 1 249.1 against 246.5, room 2
  (through the door) 17.36 against 17.21.
- GPU and CPU bakes agree to 8.7e-5 of the brightest value (half-float
  standard deviations in the filter).

## The last patch

With the variance filter a soft light patch stayed on room 2's back wall
(`r2_box`). A reference row across it (24 points, 20,000 paths each)
shows it is real: bounced daylight through the doorway from room 1, sky
bounce about 0.0010-0.0014 opposite the door against 0.0003 at the ends;
the bake follows it, a little wider at the 0.5 m spacing. The noise
around it went with twice the bounce rays for the probes near sky (probe
volume only, RSM off, fixed EV: 1x cloudy, 2x and 4x clean; startup bake
5.6 / 6.4 / 9.0 s). Kept 2x. Startup bake 6.3 s with the demo window on
the same GPU.
