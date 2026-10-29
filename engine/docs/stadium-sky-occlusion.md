# Stadium diffuse sky and sun transport

## Sun return follow-up, 29 October 2026

The sky-only correction below was also rejected as too dark. The current version
adds a shadowed first diffuse bounce of sunlight. For each probe ray hitting
static stadium geometry, trace from that surface toward the sun; accumulate
material reflectance * normal-dot-sun * visibility with cosine hemisphere
weights. No exposure change, brightness floor or authored fill lights.

Aux mode 3 retains sky visibility and sky return in R/G; B stores the sun
transfer coefficient (2 marks an invalid embedded probe). Direct sun colour
and intensity are applied in the shader, independently of sky colour. Changes
to sun direction rebuild the cache, including horizon rejection. Geometry and
material changes and renderer recreation invalidate the appropriate state.
All views share one cached volume; no extra geometry or GPU ray tracing pass.

This is one neutral diffuse bounce from static boxes, not full GI. Roof and
steps use the same stone reflectance proxy; dynamic characters, central arena
rocks/lava and local lights do not contribute bounced lighting. Multiple bounces,
per-surface colour, distance-weighted visibility and asynchronous cache updates
remain missing. The 6m probe grid may leak light across thin geometry.

**Sun changes currently incur a synchronous bake (~1.1–1.35 seconds during
CPU compilation in the capture runs).** Suitable for static sun presets; not
ready for smoothly animated day/night. Colour/intensity changes need no bake.
Below-horizon transfer is zero (26 ms in the direction-change capture).

Actual captures show visible roof undersides and increased face/masonry detail.
Direct-only and indirect-only captures separate the contribution. Low sun and
below-horizon direction changes were exercised. Evidence:
C:/dev/warp-bench/sun-bounce-20261029/view/ and directions/.
No interactive playtest was launched.

### Validation and cost

All three focused sky-occlusion tests pass, including a sunlit floor, the same
floor blocked by a roof, and zero sunlight return below the horizon. GPU
captures confirm direct-only, indirect-only and combined views. Same frozen
executable, three alternating off/on runs, native 3840x2160, four views, seed52,
30 warmup plus 600 measured frames:

| Run median | Disabled 1/2/3 | Enabled 1/2/3 |
| --- | --- | --- |
| GPU ms | 4.61 / 4.36 / 4.33 | 7.55 / 7.46 / 7.36 |
| Serialized wall ms | 13.44 / 12.44 / 13.13 | 14.01 / 14.01 / 14.00 |

Median-of-run GPU medians **4.36 to 7.46 ms (+3.10 ms)**. This is substantially
more expensive than sky-only and not an achieved performance target. Pass means
place shade at about 2.9–3.0 ms enabled versus 1.7–1.8 disabled; pass means must
not be added/subtracted as if they were frame medians. All six runs retained
600/600 profiles, zero errors/drops. Shared-system/CPU load can affect intervals.
An axis-zero read-skipping shader trial was rejected: shade 3.404 ms, GPU median
7.55 ms, no improvement. The original tested shader is retained.

[Raw timings and executable hash](stadium-sun-bounce-measurements.json).
Disable the whole cache with GG_SKY_OCCLUSION=0. No gameplay FPS claim.

## Historical sky-only correction (superseded), 29 October 2026

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
