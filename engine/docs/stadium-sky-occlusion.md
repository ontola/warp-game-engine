# Stadium diffuse sky transport

## Correction, 29 October 2026

The previous visibility-only iteration (a11c3e3) was rejected visually: the
stands were too black. It multiplied existing indirect light and reflections
by sky access and included probes embedded in solid masonry as black samples.
The old measurements remain in stadium-sky-measurements.json, but are not
quality acceptance evidence.

The correction excludes embedded probes and renormalizes interpolation weights.
It attenuates sky illumination only, preserving the existing bounce term and
reflection model. It adds one cached diffuse sky return: each probe ray that
hits a static box samples incident sky visibility at that surface, weighted by
the stadium stone material's luminance reflectance. This is a neutral material
proxy, not an artist-selected minimum brightness. Direct light and exposure
are unchanged.

The reusable engine API is bake_diffuse / set_sky_diffuse. Aux header mode 2
stores direct visibility, diffuse return and validity in the existing RGB
layout; modes 0 (irradiance) and 1 (visibility) remain supported. Four-probe
tetrahedral interpolation reads these coefficients. 54,925 probes, 6m spacing,
96 deterministic directions and 1,260 static oriented boxes. All four views
share the cache, rebuilt only when geometry or material reflectance changes.
First bake is about 1.3 seconds; no per-frame rays and no extra geometry.
GG_SKY_OCCLUSION=0 disables it for comparison. There is no disk cache yet.

## Visual result and limits

Real GPU captures at identical positions show restored masonry/row detail and
a darker covered rear. This is an improvement, not finished Lumen-like GI:
under-roof regions remain dark because direct-sun bounce and multiple bounces
are missing. The neutral coefficient approximates sky transport and does not
exactly integrate a directional sky. It has no per-surface RGB bleeding,
dynamic-object bounce, probe relocation or visibility moments. Coarse probes
can still leak across thin walls; if all tetrahedral probes are invalid the
shader falls back to the existing lighting. Glossy sky reflection occlusion
needs a separate directional solution; diffuse AO is no longer applied to it.
The correction applies to ordinary opaque materials; specialized foliage
lighting is not changed. No interactive playtest was launched.

## Validation

Two engine tests pass: roof/open-edge visibility, plus material-dependent
return and rejection of embedded probes. The corrected shader compiled and
ran on the GPU in three capture views and all six benchmark runs.

Same frozen executable and shader, three alternating disabled/enabled pairs,
native 4K four views, seed52, 30 warmup +600 measured frames:

| Run median | Disabled 1/2/3 | Enabled 1/2/3 |
| --- | --- | --- |
| GPU ms | 4.14 / 4.17 / 4.15 | 4.58 / 4.62 / 4.57 |
| Serialized wall ms | 12.31 / 11.89 / 11.96 | 12.50 / 12.86 / 12.19 |

Median-of-run GPU medians: **4.15 to 4.58 ms (+0.43 ms)**. This fixture is
not a general 120 FPS guarantee. All runs have 600/600 GPU profiles, no dropped
profiles/errors. Pass timings are means, not medians. Scene has 55,249 instances
and 8,363 seated spectators. GPU geometry counts did not increase.

[Raw run medians, pass means and hashes](stadium-sky-bounce-measurements.json).
Evidence: C:/dev/warp-bench/sky-bounce-20261029/, view-1/{facade,steps,overview}.jpg,
bench-{0,1}-{1,2,3}.log, shaders/shade.wgsl and frozen growing-guns.exe.
Previous rejected images: C:/dev/warp-bench/sky-20261029/final-view/.
Command: growing-guns bench --views 4 --size 3840x2160 --frames 600 --seed 52 --build none.
Use GG_SKY_OCCLUSION=0/1 and KERN_SHADER_DIR pointing to the recorded snapshot.
