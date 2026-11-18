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

## Later the same night

| change | result |
| --- | --- |
| Splat loop reads the tree's pose and grid once per thread (32151383) | No effect: voxel splats 6.30 / 3.52 / 5.43 before and after. |
| Splat loop reads three extinction values per cell instead of the whole 128-byte cell (ca99d058) | voxel splats 6.30 / 3.86 / 5.41 to 1.72 / 1.19 / 1.51 ms; covered_ms 21.2 / 19.6 / 17.8 to 16.4 / 16.7 / 13.9. Image diff 0.19 / 0.002 / 0.05, same splat count. The pass was memory bound. |
| Tier steps of f9b1613c | covered_ms high 21.15 / 19.17 / 18.27, medium 19.95 / 19.24 / 16.83, low 16.54 / 15.70 / 13.96 (before ca99d058). Medium near equals high in the image; low near keeps a few balls where the 4M splat cap refuses 8k camera requests. |
| Page admission by urgency with a resident bias (not committed) | No effect stationary; flying, ten times the evictions for a few more source slots. The cause was the dependency set, not the order. |
| Error-ordered gate in fast_round (not committed) | Worse: more refusals at tier low near, an even/odd alternation between frames, +0.8 to +1.3 ms from the extra rounds. |
| Branch voxel slots page their selected mip and coarser only (1c134042) | Default: pages and slots unchanged, covered_ms within 0.5; image differs 4.5 at crowns and over (under investigation: the shadow consumer can no longer pick a finer level than the camera's). Fine cells: resident pages 251 to 394 and voxel slots 4.2k to 14.7k at near, but near trees still miss pages; the arena fills at about 400 pages. |
| Shadow levels after 1c134042 (`KERN_LIGHT_DEBUG=3`, 1dd1a436 built in a worktree against ca99d058) | Near identical (0.002). Crowns and over differ 4.48 and 4.76 against same-commit noise 0.24 and 0.04; mean luma 32.41 to 31.81 and 65.68 to 64.24 (about 2 % darker). Shadows in and under mid-distance crowns are larger and blobbier; at over the shadow splats drop 2.77M to 1.81M and the clipmap splat raster 0.119 to 0.054 ms. Slightly worse, not just different. |
| Fine cells (3.75 cm), what fills | The index arena: 16.78M of 16.78M words wanted at all three poses, while cells and splats sit at 44 to 49 % of 2.1M. Page bytes are about 70 % directory (indices and four words per splat brick). At near, 12 of 13 wanted 3.75 cm pages (43 MB) and 35 of 69 7.5 cm pages (73 MB) are missing. |
| Warm start from last frame's records (`KERN_REFINE_WARM`, not committed) | Without collapse a held record keeps its booking: slots and shadow slots fill, the cut scale runs to 1e5 and the image differs 9.8 / 4.4 / 8.0, although a static view needs 2 to 3 rounds instead of 21. With a per-record check (every node from the root still refines under fast_round's rule, otherwise that tree's role restarts at the root): 54 / 415 / 272 records restart per frame, rounds stay 26 / 21, the frame gets 5 to 9 ms slower (the path walk per record) and the cut coarser (stored splats 2.4-2.9M against 3.1-3.7M). Rejected for now. Making it work needs (a) collapsing to the nearest acceptable ancestor with a per-node claim bit instead of restarting the tree, (b) the path verdict cached per node instead of walked per record, (c) the hysteresis band kept on held records so threshold nodes do not flip. |

## Pages, cells and the shadow cut (a02383f0 to 9ac72016)

Mancave, 1440p native, frame 300, crowns / near / over, back to back runs.

| change | result |
| --- | --- |
| Every voxel page cooked as compact bricks (a02383f0) | Fine levels were dense index grids: an age-12 spruce at 3.75 cm had 512k cells and 4.2M index words. Default cells: index arena 12.3M to 0.94M words wanted, page cache 332 to 270 MB, image within noise (0.32 / 0.002 / 0.03), covered_ms 16.28 / 16.63 / 13.72. |
| Stored cell 128 to 64 bytes (2c86ddf1): extinction as half floats, colour unorm16, normal moments snorm8 | Image within noise (0.15 / 0.004 / 0.06), covered_ms 16.41 / 16.44 / 13.81, voxel splats at near 1.20 to 0.98 ms. 2.1M cells are now 128 MiB, WebGPU's default binding limit. |
| Fine cells (3.75 cm) with larger arenas (local) | Index, then cells, then splats fill in turn (a page stores one splat record per cell). Cells and splats at 2.1M / 4.2M / 8.4M: missing pages 4751 / 4995 / 7734, 5555 / 5521 / 4967, 3495 / 6899 / 2314; covered_ms 18.9 / 16.8 / 15.5, 19.7 / 20.2 / 16.9, 22.2 / 26.5 / 17.9. At 8.4M (576 MiB more) crowns have no proxy blobs and look about like 7.5 cm cells. Parked: memory grows 8x per halving across the whole forest; only the nearest trees could afford it. |
| Finer levels admitted with leftover page budget (14a7d773), generation moves with them (2c86ddf1) | No effect on the shadows: residency was the same in every variant (1.407M cells wanted). Kept, since nothing waits for those pages. |
| Why shadows were coarser since 1c134042 | Not paging. fast_adapt took the shadow fill as the maximum of shadow slots, shadow splats and the source cluster fill, a budget shadows never book. Source clusters sat at 0.912, inside the 0.85 to 0.92 dead band, so the shadow scale stayed where startup left it: 55.7 with full-range dependencies, 94.2 with the smaller set (over), 302 and 2321 at crowns and near. |
| Source clusters out of the shadow fill (db39d2dd) | Deterministic (two runs equal to the unit). Shadow scale 18 / 29 / 21, but shadows then held 14.4k of their 16.4k slots and the camera lost them: crowns camera voxel slots 58,962 to 43,216, darker crowns. |
| Shadow scale rises with pool pressure (97d494b4) | Camera restored, but the scale ran to its 1e5 ceiling at crowns and near, which also coarsens the whole-root proxies' sun mips: needle shadows became soft blobs. Replaced. |
| Shadow slots as an allowance, scale from shadow splats only (9ac72016) | Camera equal to 2c86ddf1 (voxel slots 58,961 / 59,561 / 53,566). Shadow voxel slots 0 / 0 / 1,883, shadow scale 4.55 / 6.84 / 4.29, shadow splats 14.8M / 15.3M / 15.0M of 16.8M (1dd1a436: 1.5M / 2.0M / 2.8M). Crisp dappled needle shadows at crowns, denser and sharper at over. covered_ms 16.70 / 16.99 / 14.43 (+0.26 / +0.57 / +0.69, the clipmap splat raster 0.02 to 0.09 becomes 0.30 to 0.52 ms). p99 follows p50; two runs identical. Medium and low at over: 11,497 and 3,075 shadow slots, covered 13.48 and 12.02. The shadow splat cap is the lever if that raster time is wanted back. |

Storage buffers per pass after ced098e0, 58f37149, 68191813 and 7affd6fa
(work queues, counters and state, records, catalog each in one buffer): refine
fast_round 7, drain 8, select 8. Open: residency allocate_slots 10 and
retain_slots 9, materialize 15, voxel splats 10. Image and covered_ms
unchanged at every step.
