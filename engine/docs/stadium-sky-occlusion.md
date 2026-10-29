# Stadium ambient visibility

Geometry-derived ambient occlusion for the covered Growing Guns stands. This
first stage is **not bounce lighting or full Lumen**. It attenuates diffuse ambient
and environment reflections; direct sunlight and local lights keep their existing
shadowing. No extra lights or geometry are authored.

The engine builds a CPU BVH over the 1260 transformed boxes already used for the
stands, steps, walls and roof. It traces 96 deterministic sphere directions per
probe, caching cosine-weighted visibility for six axes. This data is independent
of camera position and sky colour. The shader uses four-probe tetrahedral
interpolation, continuous across cell faces, and squared normal axis weights.

The 65 x 13 x 65 grid contains 54,925 probes at 6 metre spacing, x/z -192..192,
y -6..66. Storage reuses the existing RGB probe layout (3.95 MB), but the shader
reads scalar visibility. Header word 7 selects visibility mode; zero preserves
the existing indoor irradiance mode. Reservation follows game-owned aux data.

Only changed stadium geometry triggers rebaking. The last CPU cache is reused
within the session; GPU upload occurs on change. Renderer recreation restores
data from the scene. All views share one cache. There is no disk cache yet.
`GG_SKY_OCCLUSION=0` disables reservation/baking for comparisons; default is on.

## Limitations

This is ambient visibility, not reflected light. Rear regions are still too hard
and dark for a finished GI look. Coarse probes can leak through thin geometry or
darken surfaces near probes inside solids; DDGI distance moments and relocation
are not implemented. Ambient cubes do not exactly integrate a strongly directional
changing sky. Reflection occlusion uses diffuse-normal visibility. Dynamic people,
destructible arena geometry, foliage and transparency are outside this first cache.

## Validation and measurements, 29 October 2026

The roof/open-edge unit test passes, including parallel rays and an empty scene.
The final WGSL was compiled and executed by actual GPU captures and benchmarks.
Close rows and overview were inspected: open rows retain light, covered rear rows
become darker. No interactive playtest was launched.

Initial trilinear/RGB lookup was rejected after repeated 600-frame runs gave GPU
medians 4.77/7.28/7.28 ms versus 4.26/4.13/4.32 ms disabled. Fewer RGB reads alone
did not solve it. The final scalar four-probe path was visually checked against
the eight-probe version.

Final identical executable plus immutable shader snapshot, three alternating off/on
pairs, native 3840x2160, four views, seed 52, 30 warmup + 600 measured frames.
All six runs retain 600/600 GPU profiles with zero drops/errors. RTX 5070 Ti,
Vulkan. Other CPU compilations were active; wall/FPS results are qualified.

| Median per run | Off 1/2/3 | On 1/2/3 |
| --- | --- | --- |
| GPU ms | 4.07 / 4.19 / 4.12 | 4.55 / 4.57 / 4.39 |
| Serialized wall ms | 12.29 / 12.17 / 12.13 | 12.74 / 12.53 / 12.41 |

Median-of-run GPU medians: **4.12 to 4.55 ms (+0.43 ms)** in this fixture.
Serialized benchmark FPS: about 82 to 80, not interactive FPS. This meets the
initial ~0.5 ms additional GPU goal here, not a general 120 FPS guarantee.
Geometry is unchanged: 55,249 scene instances, 8,363 seated spectators.
First cache bake roughly 0.35-0.4 seconds, shared across views.

Per-pass means and run medians:
[stadium-sky-measurements.json](stadium-sky-measurements.json).
Evidence: `C:/dev/warp-bench/sky-20261029/`, including rejected iterations,
`final-{0,1}-{1,2,3}.log`, `view-0/steps.jpg`, `final-view/steps.jpg`.
Command: `growing-guns bench --views 4 --size 3840x2160 --frames 600 --seed 52 --build none`.
Set `GG_SKY_OCCLUSION=0` / `1`. Final shader snapshot was loaded using
`KERN_SHADER_DIR=C:/dev/warp-bench/sky-20261029/tetra-shaders`.
Executable SHA256: `bc74787bd587e66aef0c209cf844131d90ef1d194dc4cf17d3b74f137fb41a6c`.
Shader SHA256: `08e9c1918a5a695d7cefa54bb173117000a9ef2f090bd743acefa65da307afab`.
Source base: `0fad26b` plus this change. Captures: 1600x900, one player, noon,
`games/growing-guns/scripts/stadium-masonry.json`.
