# Bounded crowd silhouette iteration — 29 October 2026

Growing Guns now uses shared low-poly spectator meshes with tapered torsos,
rounder heads, noses, hair/caps, separate legs and shoes. Three builds and skin
tones combine with eleven clothing colours for 33 looks. Seating randomness,
population and two instances per person are unchanged. No textures, CPU skinning
or crowd shadow casters were added.

The engine exposes `Crowd::arm_pivot(height)` to rotate the combined arm mesh
forward around its shoulder plane. This avoids the previous floating-arm cheer.
Zero preserves legacy translation. Existing GPU sway, hopping, fear and fleeing
remain. Both arms share one transform: this is not a skeletal rig or articulated
elbows. Close figures remain visibly stylized; this is a cheap improvement, not
photoreal people. Skin/build combinations are correlated, not independently sampled.

`GG_CROWD_MODE=blocks` retains the old crowd for same-executable comparisons;
otherwise the new figures are used. Crowd geometry counts and serialized benchmark
FPS are now printed by `growing-guns bench`.

## Counts

Seed 52 seats 8,363 spectators. Both benchmark variants end with 8,359 alive.
Total scene instance count is 55,249, including reserved slots. There are two
instances and four clusters per spectator in both modes.

| Geometry budget | Blocks | People |
| --- | ---: | ---: |
| Source triangles / person | 48 | 188 |
| Root triangles / person | 48 | 188 |
| Alive source triangle upper bound | 401,232 | 1,571,492 |

These are geometry budgets, **not visible/drawn triangle counts**. This small
mesh has no additional simplified root representation yet. It still uses cluster
culling; do not claim a new crowd LOD/impostor system was added.

## Measurements

RTX 5070 Ti / Vulkan, native 3840x2160, four views, seed 52, full lighting,
current masonry enabled. Same executable, three alternating blocks/people pairs.
30 warmup frames excluded, 120 measured frames per run; every run has 120/120 GPU
profiles, zero dropped/error samples. No simultaneous game or local compilation.
Background machine load can still affect CPU/wall times.

| Per-run median | Blocks 1/2/3 | People 1/2/3 |
| --- | --- | --- |
| Covered GPU ms | 4.00 / 3.83 / 3.85 | 4.14 / 4.35 / 4.20 |
| Serialized wall ms | 10.09 / 9.57 / 10.17 | 10.43 / 10.78 / 10.31 |
| Serialized benchmark FPS | 99.1 / 104.5 / 98.3 | 95.9 / 92.7 / 97.0 |

Median-of-run GPU medians: **3.85 → 4.20 ms (+0.35 ms)**. Wall medians:
10.09 → 10.43 ms. FPS is 1000 / wall median, including serialized GPU completion
wait. It is not interactive gameplay FPS or a sustained 120 FPS guarantee.

The crowd animation pass reports 0.018 ms mean in every run. Colour raster is
approximately 0.62 ms for blocks and 0.79 ms for people (per-run means). Added
geometry is not free even though instance/cluster counts remain unchanged.
Pass means must not be presented as per-pass sample medians.

Command:
`target/debug/growing-guns.exe bench --views 4 --size 3840x2160 --frames 120 --seed 52 --build none`

Images use `games/growing-guns/scripts/stadium-masonry.json`, one player, seed 52,
noon, 1600x900. Close rows and arena overview were inspected, including different
cheer phases. They show improved silhouettes and shoulder-attached arms. Fine
faces, articulated hands and independent limb animation remain absent.

Evidence: `C:/dev/warp-bench/crowd-20261029/bench-{blocks,people}-{1,2,3}.log`
and `{blocks,people}/{steps,overview,facade}.jpg`.
Executable SHA256: `77b8ae7f7a2dba702ba3a65e35ea0e75d811cadd150797b6fe7190772bf47d29`.
Source base `aedc1e8` plus this change. No interactive playtest was launched.

The six count/timing summaries are retained in
[growing-guns-crowd-measurements.json](growing-guns-crowd-measurements.json).
The figure budget/bounds/normal test and both engine crowd tests (WGSL/layout
validation and flee-slot lifetime) pass. Runtime GPU captures also compile and
execute the shoulder-pivot branch successfully.
