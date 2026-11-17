# What the probes' gradients cost in shade, 14 November

Mancave (RTX 5070 Ti, Windows), `interior_rooms` from room 2 (`--at
-2.5,1.65,-5.0 --yaw -0.79`), `--bench 300` at 3840x2160, the GPU idle
(nothing else on it, 3 % before the runs). `KERN_PROBE_GRADIENTS=0` against
the default, alternated, each three or four times; the runs agreed to
0.1 ms.

| shade | gradients off | gradients on |
| --- | --- | --- |
| main (264b4e2e) | 10.49 ms | 13.70 ms |
| slopes returning 0 (diagnostic: C1 weights only) | | 10.16 ms |
| a slope only along the axes the normal faces, only for probes that count | 10.43 ms | 11.89 ms |
| and the gradients packed three channels to a word, shared exponent | 10.49 ms | 11.77 ms |

The whole cost was the slopes: every one of the eight probes read 27 half
floats per value (sky transport and lamp light), for all three axes the
normal picks, also where the normal faces one axis (every wall here) and
for probes behind a wall that do not count. Skipping those is exact (the
images are identical). Packing three channels into one word with a
shared exponent saved 0.1 ms more but was rejected: the sky transport's
three channels are sky, bounce and sun, orders of magnitude apart, and the
small ones lost their precision (mean 0.8 of 255 off on `r2_bands`).

Kept: +1.4 ms (was +3.2 ms).

The 13 November measurement (+2.8 ms) shared the GPU with the demo window;
with the window open on 13 November shade ranged 23-34 ms, so that number
was contended.
