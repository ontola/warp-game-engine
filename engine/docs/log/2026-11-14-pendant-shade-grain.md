# The grain on the swinging pendant's shade, checked again, 14 November

Main at d4ab8782, Mancave (RTX 5070 Ti, Windows), `interior_rooms`, idle
GPU, 1920x1080. Follows the open item in
[2026-11-12-moving-reflector-grain](2026-11-12-moving-reflector-grain.md):
a green-grey grain on the inside of the pendant's shade, only while it
swings, from the traced reflections.

Joep's F4 of that evening (`warp--four-rooms-1794512455046`, camera
0.458,1.65,-8.299, yaw -15.859, pitch -0.230, fog on) with the pendant
where its inside faces the camera (215 frames from start, swinging) and the
same pose held (`ROOMS_HOLD_T=3.5833`, a new switch of the example), 2x
crops and a grain measure (mean |luma - mean of its 8 neighbours|, 0..255)
over 150x90 pixels of the inside right of the bulb:

| | grain |
| --- | --- |
| swinging | 0.530 |
| held at the same pose | 0.542 |
| swinging, `KERN_TRACED_REFLECTIONS=0` | 0.386 |

The green tint is gone, and the shade is no grainier swinging than still:
what the traced reflections add (0.14 of 255) is the same on a still
surface (the table top: 0.53 against 0.40 without them) and not visible in
the crops. Which change since 76f067b9 removed it was not bisected. The
motion-specific grain is not reproduced; the open item is closed in
[status](../status.md).

Seen on the way: in Joep's F4 and here, the edge of the fog's cone under
the shade is a staircase at the froxel size (12 pixels at 1080p) when the
lamp is within a metre of the camera; the grid's cells are larger than the
cone's edge there.
