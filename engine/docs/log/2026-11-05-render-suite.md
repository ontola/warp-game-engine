# Render suite and first iterations, 5 November 2026

Machine: Mancave, RTX 5070 Ti, Vulkan, driver 617.14, native Windows build
driven from WSL. Scene: `engine/tools/render-suite.json`, 2560x1440. Baseline
commit 793fa10, three repeats, median of the last 60 frames. CI jobs ran all
evening (WSL load 20 to 90), so wall times are inflated; GPU times repeated
within 0.3 ms anyway.

## Baseline

| View | GPU ms | Wall ms | Camera triangles | Largest passes (ms) |
| --- | --- | --- | --- | --- |
| near | 19.90 | 45.1 | 33.5M | selection 7.13, occluder cull 2.57, shade terrain 2.52, occluder raster 2.35 |
| forest | 23.12 | 43.1 | 67.3M | selection 6.49, occluder raster 5.68, occluder cull 3.14, shade terrain 2.31 |
| wall | 5.60 | 25.3 | 30k | selection 1.68, shade terrain 0.78, shade 0.68 |
| town | 17.78 | 45.1 | 3.5M | selection 6.67, shade terrain 2.29, occluder cull 2.12, remaining cull 2.06 |
| hill | 25.38 | 55.7 | 26.4M | selection 10.64, remaining cull 3.46, occluder cull 2.46, shade terrain 1.73 |
| polder | 14.23 | 38.9 | 8.2M | selection 7.47, shade terrain 1.51 |
| far | 19.38 | 42.0 | 5.1M | selection 7.69, remaining cull 2.45, occluder cull 2.22, shade terrain 2.18 |
| flight | 23.22 | 112.5 | 36.8M | selection 7.58, remaining cull 2.82, occluder cull 2.49 |

Allocated GPU memory is 1.78 GiB in every view, 2.1 GiB reserved. Largest:
voxel splats 294 MiB, albedo 128, visible 109. Sun shadows are small here:
`shadow cull` and `voxel sun splats` are about 0.1 ms each on the wall view,
where the M5 Pro spends 1.28 and 1.04 ms.

## Kept

| Change | Result |
| --- | --- |
| Terrain patches upload as one range; far sun sources as one write, not one per instance | Flight terrain update 31 to 4.8 ms |
| Quadtree keys use a cheap hasher; a settled selection plans nothing | Flight terrain update 64 to 31 ms; still views 2 to 0.003 ms |
| Phase-two queue 262,144 to 1,048,576 clusters (+36 MiB) | `remaining cull` -2.0 town, -3.3 hill, -2.4 far, -2.7 flight; no overflow in any view |
| Grass, soil and gravel blend to triplanar on faces steeper than about 45 degrees, and their bump map fades out | No GPU cost; `wall` changes 0.62% of pixels as intended, other still views within noise |

Together the flight's wall time fell from 112 to 40 ms and its GPU time from
23.2 to 20.7 ms. Still views stayed within the noise floor of their pictures
(0.000% of pixels over 16 levels).

Patch selection itself is not the cost: about 1 ms per reselect at 1280x720.
The 60 ms was thousands of small `queue.write_buffer` calls.

## Measured for the shading thread

PR #10 at fc759df on `--town`: frame time unchanged (wall 7.01 to 6.98 ms, hill
18.61 to 18.65), `shade terrain` down at most 0.06 ms. Pixels: wall identical,
hill at most 22 levels on no measurable share of pixels.

## Findings left open

Foliage, not changed here:

- Selection is the largest pass in every view, 1.7 to 10.6 ms.
- The forest cut does not settle quickly on a still camera. On `polder`, voxel
  slots went 64, 98, 50 at 300, 600 and 1200 frames on one build, and 95, 50, 50
  on a faster one; 1.1% of pixels differ between those states. Both builds
  agree at 1200 frames (at most 31 levels, 0.00005% over 16). The cut depends on
  wall time as well as frames, so a player can see trees change while standing
  still.
- `forest` draws 67M triangles and `occluder raster` takes 5.68 ms. This fits
  handoff problem 1 (source bypass) but was not tested here.
- Ball-shaped clusters and ball-shaped tree shadows at 20 to 40 m on `forest`.

Terrain and shading, still to do:

- Faint vertical bands remain on steep plot cuts after the fix below: the colour
  breakup noise is still mapped from above.
- Faint rings in the sky on `forest`.
- The building slab shadow is stepped on `wall`.
- `render_cpu_ms` is 8 to 16 ms, now the largest CPU cost measured. With
  `KERN_CPU_SPANS=1` on `town`, one sampled frame spent 6.9 ms in `submit` and
  under 1.5 ms in everything else, so encoding is not the problem.
