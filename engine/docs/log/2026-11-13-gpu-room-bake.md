# The room bake on the GPU, 13 November

Mancave (RTX 5070 Ti, Windows), `interior_rooms`: 14,260 probes, 576
triangles, two door states baked at startup in parallel, sky 2,048 rays,
bounces 512 rays, 3 sky bounces, 4 lamps with 2 bounces each. Startup bake
times with the demo window running on the same GPU.

## Where the CPU bake's time went (26.4 s)

Per door state, measured inside the two parallel bakes: sky transport
10.5 s, lamp bounces 11.7 s, reach 1.4 s, visibility 1.1 s, placement 0.6 s,
planes 0.4 s. Each bounce ray's hit traced its eight corner probes and the
sun or lamp (about ten rays per hit, every pass), and `probe_positions`
(256 rays, up to four moves per probe) ran in every stage, six times per
state.

## Steps

| Change | Startup bake |
| --- | --- |
| CPU, before | 26.4 s |
| `Placement` once per state (9d8e0f48) | 22.2 s |
| Transport on the GPU, every pass re-tracing its shadow rays | 13.2 s |
| Any-hit for shadow rays | 12.2 s |
| Corner, sun and lamp visibility traced once per ray into a mask (93dd5afd) | 5.2 s |
| Placement, visibility maps, reach and planes on the GPU | 3.5 s |

In the second row from the bottom the GPU part was sky 1.7 s and lamps
6.5 s per state. In the last: geometry 0.46 s, sky 0.9 s, lamps 2.1 s.

Agreement with the CPU bake (`room_bake_gpu` tests, a 4 m room with a
door, window and partition): transport to 1.2e-7 of the brightest value
(sky) and 3.4e-7 (a lamp); positions to 3e-8 m; visibility maps and reach
bit for bit; 4 of 1,792 separation planes picked the other of two equally
near surfaces (a ray along a box edge).

Benchmark without the demo window (`gpu_bake_speed`, 14,000 probes, 288
triangles, default settings): GPU 0.16 s, CPU 1.33 s.

## Lamps in one pass, kernels once, more rays

Up to eight lamps share a pass (each bounce ray traced once for all, a
probe set per lamp; three lamps at once agree with the CPU bake to 3.5e-7),
and the passes compile once per startup instead of three times per door
state: startup bake 3.5 s -> 2.2 s.

More rays against the two final smoothing passes, the whole suite (29 spots,
mean luminance difference per spot, 0..255):

| Variant | Startup bake | Against |
| --- | --- | --- |
| A: 2,048 + 512 rays, 2 passes (default) | 3.8 s | |
| B: 8,192 + 2,048 rays, 2 passes | 14.0 s | A: 0.03 to 3.8 |
| C: 8,192 + 2,048 rays, no passes | 14.2 s | B: 0.09 to 22.8 |
| D: 8,192 + 2,048 rays, 1 pass | 14.1 s | B: 0.05 to 8.5 |
| E: 2,048 + 512 rays, no passes | 3.1 s | A: 0.1 to 22.8 |

(A to E before lamps shared a pass.) Four times the rays move the result
about as much as noise; dropping the passes moves it a lot with either ray
count (C against B as E against A), most in dark room 4 (22.8) and on room
2's walls. In the 4x crop of `r2_bands` a step at the probe spacing comes
back without them, and rooms get darker: they spread light that trilinear
interpolation would show cell by cell, not ray noise. Kept, as the rays.

## Open: the step at the probe spacing

The two passes hide an engine fault rather than fix it: without them a wall
in room 2 (`r2_bands`) shows a step at the probe spacing, the same kind of
cell edge Joep saw on a wall before. Smoothing the probes makes neighbouring
probes alike, so the step's height shrinks; the lookup that makes a step at
all is unchanged. The interpolation in shading should be continuous whatever
the probes hold.

Best guess for the cause, not yet measured: a weight in the eight-probe
lookup that switches per cell or per voxel instead of varying smoothly, most
likely the reach test (a probe counts for a point only if the point's voxel
of its 8x8x8 reach map is set: on or off at voxel and cell boundaries),
possibly the separation-plane fallback. Next: a debug view of each probe's
weight at `r2_bands` with the passes off, to see which term jumps at the
step; then make that term continuous (for reach: the fraction of reached
voxels around the point, trilinear over the map, instead of the one voxel
it falls in), and drop the passes once the suite's crops show no step.
