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

## Not done

- Kernels compile per call (three contexts per state, about 0.3 s each).
- Lamps are baked one after another, each with its own readback.
- The smoothing passes are the CPU bake's, kept so the two agree. With rays
  now cheap, more of them may make the two extra passes against probe-
  spacing blotches unnecessary; not tried.
