# Vegetation cost, tiers and rejected candidates, 18 November

Machine: Mancave (Windows, RTX), 2560x1440 native, `terrain_fly --seed 1
--style swiss --spruce-forest`, headless `--fly`, medians over frames 101 to
300. Poses: crowns `-21,-116,25,0,-0.55`, near
`-121.3255,-5596.4775,6.3668,3.944799,0.062000`, over
`389.0229,-573.3995,48.2914,0.419595,-0.415399`. Branch
`claude/project-thread-s7vbe4`, base a62ef085.

## Which number is the frame

`--fly` frame p50 is the wall time of the headless loop: it encodes, submits
and reads the frame back every frame. `covered_ms` (the interval the timed
GPU passes cover) is the GPU cost. Baseline, crowns / near / over:

| | frame p50 | covered_ms p50 |
| --- | --- | --- |
| baseline | 32.8 / 30.4 / 30.3 | 21.0 / 19.2 / 18.2 |

Turning off `--voxel-raster-profile` and `KERN_SPLIT_SELECTION_TIMING` does
not change frame p50, so the difference is the loop, not the profiler. CPU
`render_ms` is 7.7 ms (crowns).

## Per pass (baseline, ms, crowns / near / over)

| pass | ms |
| --- | --- |
| voxel splats (generation) | 6.17 / 3.44 / 5.33 |
| GPU forest selection residency | 2.16 / 2.40 / 2.36 |
| GPU forest refine | 2.05 / 2.29 / 2.25 |
| shade terrain | 1.97 / 1.01 / 1.83 |
| occluder raster | 1.91 / 1.65 / 0.71 |
| voxel splat raster | 1.55 / 1.64 / 1.61 |
| remaining raster (needles) | 0.88 / 1.43 / 0.38 |
| shade | 0.83 / 1.52 / 0.51 |

Stored splats 3.14M / 3.45M / 3.24M; about 92% of discs are 1 to 2 px wide.

## Candidates

| change | result |
| --- | --- |
| Compute raster for small discs (`KERN_SPLAT_COMPUTE`, 4de99e13) | Slower in every variant: covered_ms +0.3 to +0.4 at radius < 1.5 px, +0.4 to +1.2 with every disc by compute. Removed. |
| Workgroup-aggregated publishes off (`KERN_REFINE_WAVE_PUBLISH=0`) | No difference. |
| `KERN_REFINE_ROUNDS` 8 / 16 | Not converged: source slots 0, 0.3M to 0.7M splats, every tree a whole-root proxy. |
| `KERN_REFINE_ROUNDS` 26 / 48 | Identical image to the default; 48 costs +1 to 2 ms frame time. A cold start needs 16 to 26 rounds; a round that changes nothing still costs about 0.07 ms. |
| Voxel leaf parity (d6fa71c2) | Voxel / mesh mean luma at crowns 49.7 / 42.1 to 43.7 / 40.1. Same ms. |
| Disc depth from the drawn disc (e059281e) | 1.5% fewer stored splats, luma -0.2, same ms. |
| Fine branch cells (`KERN_BRANCH_CELL_M=0.0375`, `KERN_BRANCH_BRICKS=1024`) | Crowns look right at the same cost. At the near pose voxel slots fall from 57k to 4.5k and source slots from 503 to 32: 5,794 missing pages, the page cache keeps resident far bundles ahead of the near tree's request, so near trees stay whole-root proxies. The earlier "near-black fine voxels" were these proxies. |

## Tiers (b21bd791 steps: medium error 1.5, 1M source, 8M splats, half the slots; low error 2.5, 256k source, 3M splats, a quarter of the slots)

covered_ms p50, crowns / near / over: high 21.1 / 19.2 / 18.2, medium 17.5 /
15.7 / 15.4, low 13.8 / 12.9 / 12.2. Source slots at crowns 492 / 223 / 32:
most of the saving came from fewer needles, which shows as proxy balls in
near crowns (first come, first served splat reservations refuse the near
trees' late refinements). f9b1613c keeps the full needle budget on medium
and half on low; those steps are not timed yet.
