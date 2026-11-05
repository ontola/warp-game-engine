# Controlled colour-geometry diagnostics — 24 October 2026

Windows/Vulkan, RTX 5070 Ti, one 3840×2160 output, four views, DLSS Quality.
Each view rendered internally at 1277×717. Fixed seed 1, 3 km stage,
noon/clear, ticks 6001–6240; 60 warmup frames and the capture excluded.
179 GPU frame pairs per comparison. All 240 camera/light/car checksums matched.

Evidence: `C:/dev/perf/warp-sideways-20261024/r28-categories/matrix/`.
`comparison/report.html` and `report.json` preserve raw manifests, exact pass
timings, counters, frame identity and missing-sample accounting.

| Colour geometry omitted | GPU median ms | Median paired reduction ms |
| --- | ---: | ---: |
| Nothing: baseline | 35.52 | — |
| Trees | 16.95 | 18.58 |
| Grass | 33.65 | 1.87 |
| Terrain, road and rocks | 33.12 | 2.39 |
| Cars | 34.62 | 0.89 |
| Other | 35.24 | 0.33 |
| Nothing: repeated baseline | 35.16 | 0.30 |

These are non-additive marginal omissions, not acceptable quality reductions
or exclusive category timings. Removing occluders exposes other work. Shadows,
GI, physics and effects remain. The 0.30 ms repeat drift makes the tiny Other
result inconclusive. The first baseline had a 171.5 ms outlier; medians do not
describe that stall. Raw mean/p95/max remain in the report. This short route
does not establish typical gameplay performance.

Trees explain the dominant raster load: omission reduces raster by 14.24 ms,
culling by 2.52 ms and shading by 1.47 ms (paired medians). At frame 240,
across four views, tree clusters contain 123,302,820 selected source triangles
and submit 511,958,376 padded vertex invocations. All categories together
contain 124,551,027 selected triangles. These are selected/submitted counts,
not hardware counts of visible triangles. Trees win 969,949 input visibility
pixels before forward grass/glass/effects. There are 319,179 source tree owners,
referenced independently by multiple views.

Prioritize shared procedural plant assemblies and efficient distant tree
representations, then cluster LOD efficiency, opaque/cutout paths and atomic
aggregation. Grass is not the largest remaining cost in this scene. A universal
4×4-pixel minimum triangle size would damage foliage silhouettes; opaque canopy
shadow hulls risk restoring the dark distant-forest problem. Validate appearance.

Code audit: PBR already shades one winning surface per input pixel through
compute; textures have full mips but are uncompressed. There is no main Hi-Z
pyramid or POM loop. Opaque/cutout geometry shares a raster pipeline; selected
clusters perform global atomics. These are measured-experiment candidates,
not guaranteed gains.

The opt-in diagnostic uses explicit source categories in reserved instance bits;
the 80-byte GPU ABI is unchanged. `KERN_DIAGNOSTIC_HIDE=none` enables baseline
accounting; category names hide colour geometry. Normal play leaves it unset.
Readback is capture-only and excluded from steady timing. Shared counters exclude
dedicated near/far grass, glass, effects and HUD, which remain in the total
colour-omission timing. Unsupported experimental foliage paths reject the mode.

Validation: 257 renderer CPU/shader tests passed (one existing ignored test),
10 report tests passed, and a native GPU fixture verified all five exclusions,
zero excluded visibility, unchanged shadow counts and a received tree shadow,
four views, frozen diagnostic identities and removal of blended glass.
Target remains 8.33 ms; no quality-preserving speedup is claimed.
