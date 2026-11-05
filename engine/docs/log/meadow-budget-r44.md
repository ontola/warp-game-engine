# Meadow budget, r44

Status: the six-case native diagnostic matrix completed, but **none of these
settings is promoted to a new default**. Removing half the patch roots makes
visible bare channels. Enlarging the remaining clumps does not restore the
reference coverage. Earlier source-to-aggregate replacement remains a separate
experiment, with a small saving in one camera and a cost increase in the other.

## Evidence and measurement scope

Evidence directory: `C:/dev/perf/warp-sideways-20261025/r44-meadow-budget/native-b`.
`report.json`, the scenario, each run manifest, frame profile and capture
sidecars retain the exact settings and counters. Report SHA256:
`2631a77f754481d6c6025b985e0d7a4e886858f5b78426a12e50167854722758`.
All six runs use executable SHA256
`4f532779285201b87097ed3d38a107ab00f851fdf50cce7bebe26019fa49888a`,
with embedded shaders and no shader-directory override.

This is **one view at native 1920 × 1080**, clear noon, seed 1 / 3 km,
paused at simulation tick 6000, wind strength 0.6 and fixed wind time 1.9.
Both cameras come from retained scene captures. Each section has 192 frames:
64 warmup frames and the final capture frame are excluded, leaving **127
matched GPU samples per camera and case**, with no missing selected samples.
Captures are 1600 × 900 JPEG previews. The debug overlay is closed before the
measured interval; the pause panel remains visible in the previews.

The GPU interval below is the earliest measured GPU start to the latest
measured GPU end, including gaps. It is not a sum of passes, hardware active
time, CPU execution time, or presentation latency. `meadow render` is source
depth plus colour submission; `meadow aggregate render` is the distant
representation's measured scope, not all meadow work. Placement, composite and
other passes remain separate in the raw report. This single serial matrix has
no confidence interval or repeated ABBA trial; small differences need repeat
measurement. It does not establish the native 4K / four-player / 8.33 ms target.

## Six settings, two cameras

Every value is a median in milliseconds, **n = 127**. Reference settings are
size 1, patch count 100%, source LOD error 1 px and source-to-aggregate blend
60–80 m. All cases retain the 160–180 m outer fade and unchanged pigment.

| Case | Changed setting | Near GPU interval | Near source | Near aggregate | Far GPU interval | Far source | Far aggregate |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `reference` | None | 18.548 | 10.722 | 0.041 | 20.273 | 9.254 | 2.737 |
| `lod2` | Source LOD error 2 px | 18.372 | 10.528 | 0.041 | 20.295 | 9.294 | 2.746 |
| `count50` | Retain 50% of patch roots | 14.673 | 6.809 | 0.042 | 16.129 | 6.411 | 1.579 |
| `size125` | Size 1.25 | 19.959 | 12.036 | 0.041 | 20.923 | 9.366 | 3.362 |
| `larger-half` | Size 1.25, patch count 50% | 15.754 | 7.928 | 0.041 | 16.750 | 6.682 | 1.898 |
| `source60` | Source-to-aggregate blend 40–60 m | 18.731 | 10.702 | 0.207 | 19.486 | 7.436 | 3.975 |

The reference near view already selects 6,261 of 6,345 patches at LOD2
(98.68%). Raising the threshold to 2 px moves the other 84 patches to LOD2,
reducing source triangles by only 1.05%. The far reference already selects
all 22,778 source patches at LOD2, so its source count cannot improve through
that threshold alone.

| Case | Near source triangles | Far source triangles | Far aggregate patches |
| --- | ---: | ---: | ---: |
| `reference` | 3,847,320 | 13,666,800 | 45,189 |
| `lod2` | 3,807,000 | 13,666,800 | 45,189 |
| `count50` | 1,904,520 | 6,823,800 | 22,549 |
| `size125` | 4,171,200 | 13,676,280 | 48,944 |
| `larger-half` | 2,072,040 | 6,829,800 | 24,420 |
| `source60` | 3,847,320 | 7,408,800 | 52,490 |

These are actual GPU post-cull indirect counts at each section's capture.
Source triangles are counted once, although depth and colour submit them
twice. Counts precede per-blade road rejection; they are not shaded pixels,
measured vertex invocations, shadow triangles or total scene triangles.

## Image review and decision

Near `frame-000192.jpg` and far `frame-000384.jpg` were inspected for
`reference`, `larger-half`, `source60`, and additionally `count50`.

- The reference near hillside maintains continuous overlapping grass with
  green and dry colour patches. Both 50%-root cases expose connected bare
  soil channels and isolated clumps; size 1.25 does not compensate. Their
  far verges and distant hillside also become visibly patchier. Their lower
  GPU times therefore purchase a substantial coverage loss.
- `source60` retains the broad coverage of the reference in these static
  previews, without the large bald areas of the count reductions. Its far
  source saving is partly consumed by the aggregate pass. Near source work
  is unchanged while 1,039 aggregate patches are added, so that camera does
  not benefit. This is not pixel parity or moving-transition acceptance.
- The JPEGs cannot establish wind continuity, temporal stability, small
  blade silhouettes or nighttime response. Existing stepped road margins
  and the distant layer's flatter appearance remain limitations.

The next experiment is an **opt-in recipe with fewer, wider blades inside
each clump while retaining every world patch root**. It addresses source
work without repeating the demonstrated whole-patch holes. That recipe is
not a result of this matrix: coverage, source-derived aggregate agreement,
wind and actual GPU cost still need their own matched evidence.

## Validation boundary

The associated UI/control source checks passed **396 render tests and 162
game tests**, with one ignored test in each suite (`render-tests.log` and
`game-tests.log` in the r44 evidence root). Those are CPU/source checks,
not a performance certification. The noon grass matrix does not prove the
separate shadow changes; their native night/dawn evidence follows below.
No runtime source or defaults were changed for this review.

## Shadow work: native ABBA verification

Evidence: `C:/dev/perf/warp-sideways-20261025/r44-meadow-budget/shadow-a/report.json`,
SHA256 `ae7124d16ca7406b6803d598899fed837295467d9c0d06c682943acc373da77d`.
All four run manifests identify the immutable `sideways-r44-controls.exe`,
SHA256 `4f532779285201b87097ed3d38a107ab00f851fdf50cce7bebe26019fa49888a`,
with embedded shaders. The order is `off-a`, `on-a`, `on-b`, `off-b`.
Both `KERN_REUSE_SUN_TEXTURE` and `KERN_ZERO_SUN_FAST_PATH` are explicitly
0 for off and 1 for on; this measures their combined package.

Each run uses **one native 1920 × 1080 view**, the same fixed camera and
seed-1 stage, paused tick 6000, wind strength 0.6 and fixed wind time 1.9.
Night, sunrise and noon follow within that same process, with 128 frames per
section. Excluding 32 warmup frames and one JPEG/readback frame leaves **95
matched clean GPU samples per section and case**, with no selected samples
missing. These remain GPU render intervals, not pass sums or gameplay FPS;
CPU wall and execution timing remain separate and may be affected by CI.

| Phase | Off A interval (ms) | On A interval (ms) | On B interval (ms) | Off B interval (ms) |
| --- | ---: | ---: | ---: | ---: |
| Night | 13.502 | 10.580 | 10.576 | 13.533 |
| Sunrise | 18.754 | 18.455 | 18.405 | 18.612 |
| Noon | 18.698 | 18.615 | 18.656 | 18.640 |

The controlled night interval falls by approximately **22%**. Its shared sun
raster alone measured 2.486/2.492 ms in the off runs and is absent in both on
runs. Small daylight differences are not a reliable general speedup claim.
This fixed-pose suite does not independently measure allocation reuse under
moving casters or separate that contribution from zero-sun work pruning.

The pass records confirm actual work removal and restoration: all 128 night
frames submit shared sun shadows when off, and **zero** night frames submit
any solar shadow pass when on. Near shadows are absent at night in both
conditions. All **128 sunrise frames and 128 noon frames** restore shared and
near sun passes in every run. The captured night has exactly zero direct sun
lux; sunrise/noon restore positive direct illumination.

All **12 decoded 1600 × 900 JPEG comparison pairs are exactly equal**, with
zero changed pixels and maximum channel error 0. These include off/off,
on/on and off/on comparisons in all three phases. That establishes equality
of the retained previews, not full-resolution uncompressed or temporal
parity. The native result does not establish four-view performance; the
separate four-view evidence follows below.

## Shadow work: four independent views with DLSS

Evidence: `C:/dev/perf/warp-sideways-20261025/r44-meadow-budget/shadow-four-b/report.json`,
SHA256 `f9f8683e1e78eb91c61fa700dc0676fa14f9fa9c8884234cecac0b1f2a6ef624`;
`visual-review.json` records the inspected images and their hashes. This uses
the same immutable `4f532779…a49888a` executable and original **360 blades per
patch**, not the later reduced-blade experiment.

The explicit scenario supplies **four distinct fixed cameras**, producing
one **3840 × 2160 output**. Each panel renders at **1277 × 717**, reconstructed
by **DLSS Quality to 1916 × 1076**, with no fallback. The four panel outputs
plus dividers form the single 4K image; each player does not render at 4K.
Night then sunrise run for 128 frames each, with 32 warmup frames and the
final capture excluded: **95 clean matched GPU samples per section and run**,
no missing selected samples. Stage, cameras, tick 6000 and wind time 1.9
match across the off/on/on/off sequence.

| Phase | Off A interval (ms) | On A interval (ms) | On B interval (ms) | Off B interval (ms) |
| --- | ---: | ---: | ---: | ---: |
| Night | 47.462 | 42.851 | 42.905 | 47.284 |
| Sunrise | 56.847 | 56.412 | 56.305 | 56.276 |

Night saves approximately **4.4–4.6 ms, or 9–10%**, in this four-view scene.
The off runs spend 3.811/3.800 ms in the shared sun raster; that pass is
absent when on. All 128 night frames obey the requested solar-work policy,
and all 128 sunrise frames restore shared and near sun passes, including
the first dawn frame. Sunrise differences overlap repeat-run variation;
there is **no reliable daylight speedup claim**. These measured GPU render
intervals are still far above **8.33 ms** and exclude unmeasured presentation
work. CPU wall and execution costs remain separate and may be CI-contended.

The off-A/on-A night and sunrise previews were visually inspected. Road
illumination, tree silhouettes, stars and the restored sunrise shading remain
consistent, with no obvious missing-caster or stale-night-map artifact. Unlike
the native test, **decoded JPEG parity is false**. Identical-mode repeat runs
also differ:

| Phase | Off/off mean absolute byte error | On/on mean absolute byte error | Off/on errors, two pairs |
| --- | ---: | ---: | ---: |
| Night | 0.01456 | 0.01447 | 0.01412 / 0.01304 |
| Sunrise | 0.15606 | 0.15318 | 0.15613 / 0.15665 |

The maximum off/on channel error is 24/255. Comparable whole-preview errors
support the static visual review, but do not prove that every off/on difference
is DLSS nondeterminism. No full-resolution pixel equality, moving-camera
stability, shadow shimmer or moving-caster allocation stress is claimed.
The flatter distant grass and near-grass texture remain separate limitations.

The earlier `shadow-four-a` attempt failed before any scene frame because the
immutable executable's directory lacked the DLSS runtime. It contributes no
timing or image evidence. The successful suite explicitly sets
`KERN_DLSS_RUNTIME_DIR=C:/dev/tools/nvidia-dlss-sdk/lib/Windows_x86_64/rel`;
actual reconstructed view dimensions are checked after every run.

## Blade budget inside every clump: native results

The follow-up keeps **all eligible world patch roots** and reduces the number
of authored blades inside each clump. This differs from the rejected 50%
world-root experiment above. `SIDEWAYS_MEADOW_BLADE_BUDGET` selects 360, 180 or
120 blades; corresponding widths are 1×, 2× or 3×. Source geometry, LOD errors,
directional aggregate and substrate are recooked from that recipe. Preserving
summed blade area does not prove equal projected coverage or appearance.

Evidence: `C:/dev/perf/warp-sideways-20261025/r44-meadow-budget/blades-native-a/report.json`,
SHA256 `a5230fbd5d4866294edb30cd9089ddd794e23e3cdf0f169357d0a9068c15c24a`.
All four runs use executable SHA256
`cc38a48bbd37323ad34ebc396c1edce3d7c3934d79ff0dc6963caddd52922ded`.
The sequence is **360 / 180 / 120 / 360 reference repeat**, not an ABBA
comparison of either candidate. Each uses the same two native 1920 × 1080
cameras, paused tick 6000, noon, wind 0.6/time 1.9 and size/count/colour
defaults. Source-to-aggregate blend stays 60–80 m, outer fade 160–180 m.
Again, each section has **127 clean matched samples** after 64 warmup frames
and capture exclusion. Every displayed value below is a median in ms.

| Blades per clump | Near GPU interval | Near source | Near aggregate | Far GPU interval | Far source | Far aggregate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 360 reference | 18.461 | 10.696 | 0.041 | 20.222 | 9.277 | 2.738 |
| 180, twice the width | 16.300 | 8.455 | 0.041 | 17.969 | 7.085 | 2.685 |
| 120, three times the width | 15.038 | 7.237 | 0.041 | 16.880 | 6.034 | 2.607 |
| 360 reference repeat | 18.594 | 10.729 | 0.041 | 20.146 | 9.169 | 2.740 |

Against the first reference, 180 saves approximately **2.16 / 2.25 ms** in
the near/far GPU intervals; 120 saves **3.42 / 3.34 ms**. Both exceed the
observed reference-repeat variation, but there is no statistical confidence
interval or general gameplay speedup claim. These are one-view native results;
the separate four-view ABBA follows below.

| Blades | Triangles per clump, LOD0/1/2 | Near submitted source triangles | Far submitted source triangles | Far aggregate patches |
| --- | --- | ---: | ---: | ---: |
| 360 | 1800 / 1080 / 600 | 3,847,320 | 13,666,800 | 45,189 |
| 180 | 900 / 540 / 300 | 1,921,260 | 6,832,500 | 45,088 |
| 120 | 600 / 360 / 200 | 1,272,480 | 4,556,000 | 44,743 |

Counts retain the earlier once-per-source-triangle scope, before road habitat
rejection; depth and colour each submit that geometry. Every world patch
remains eligible, but actual submitted patch counts need not be identical:
recipe bounds, road support and LOD errors are recooked. Shared source vertex
payload falls from 2,004,480 to 1,002,240 / 668,160 bytes. The aggregate recipe
remains 3,148,608 bytes; this is not a proportional saving in total GPU memory.

### Visual decision

All six original near/far JPEGs for 360, 180 and 120 were inspected.
`blades-native-a/visual-review.json` records their hashes and observations.

- **Both reductions preserve continuous broad coverage** in these cameras.
  Neither recreates the connected bare channels of the earlier half-root
  cases. The far hillside and road verges remain covered.
- **180 is the recommended visual compromise.** Its foreground blades are
  broader and less finely textured than 360, but remain closer to the
  reference. It removes about half the source triangles while retaining
  every world clump.
- **120 is the more aggressive performance playtest option.** Wider ribbon
  shapes are clearly visible in the near foreground and the road-edge
  silhouette becomes chunkier. It still meets the broad continuous-coverage
  requirement in these stills. Following the user's willingness to trade
  some quality for speed, it is selected for the next performance playtest,
  subject to the separate four-view checks; this is not visual parity.

The source default remains **360** until motion and user acceptance. Frozen
wind, two noon cameras and downscaled JPEGs cannot validate moving blades,
camera transitions, full-resolution fine detail, DLSS history or night
lighting. Existing stepped road margins and the smoother distant aggregate
remain visible; no new broad bald region or obvious large road spill was
found in the reviewed views. No runtime source was edited during this review.

## Blade budget: four-view DLSS ABBA and visual tradeoff

Evidence: `C:/dev/perf/warp-sideways-20261025/r44-meadow-budget/blades-four-a/report.json`,
SHA256 `0447e5f01281de1a01580510b97b6600c7e40631e2c919e9c67506a0129485a4`.
All four runs use the same executable SHA256
`cc38a48bbd37323ad34ebc396c1edce3d7c3934d79ff0dc6963caddd52922ded`.
The order is **360 / 120 / 120 / 360 blades per clump**. This is a separate
comparison from the shadow optimization above, not another contribution to
add to its timing delta.

One **3840 × 2160 output** contains four distinct fixed cameras. Each panel
uses **DLSS Quality, 1277 × 717 input to 1916 × 1076 output**, without fallback.
The clear-noon scene, tick 6000, fixed wind strength 0.6/time 1.9 and all live
size/count/colour settings match. Every 192-frame run excludes 64 warmup
frames and its final JPEG/readback, leaving **127 clean matched GPU samples**,
with no missing selected samples. Values are medians in milliseconds.

| Run | Blades per clump | GPU render interval | Meadow source depth + colour | Meadow aggregate |
| --- | ---: | ---: | ---: | ---: |
| A1 | 360 | 52.106 | 21.855 | 5.781 |
| B1 | 120 | 43.312 | 13.264 | 5.623 |
| B2 | 120 | 43.319 | 13.209 | 5.624 |
| A2 | 360 | 52.093 | 21.834 | 5.779 |

The paired render-interval reduction is **8.77–8.79 ms, approximately 17%**.
The source-grass scope falls by approximately **39%**. These are measured
scopes, not independent additive savings: neither pass sums nor CPU wall time
are used to construct the GPU interval. CI may affect CPU wall/execution,
which remains separately reported. The result is still approximately **43.3
ms**, well above the **8.33 ms** target; it is not a delivered-gameplay-FPS claim.

Across the four view lists at frame 192, source triangles fall from
**43,012,800 to 14,320,040**. This counts source geometry once; depth and colour
each submit it. It is not a unique-world triangle count, since different
views can select the same clump. Submitted source patches are 71,620 versus
71,533 and aggregate patches 185,558 versus 184,822. The small count changes
come from recooked bounds and selection, while every suitable world root
remains eligible. This is not the rejected removal of half the world roots.

All four original `frame-000192.jpg` previews were inspected.
`blades-four-a/visual-review.json` retains their hashes and observations.
The 120-blade candidate keeps broad continuous cover in the near uphill view
and both road views, without the earlier connected bare-soil channels. The
elevated fourth view also retains broad ground coverage. **Close grass is
visibly coarser:** wide, angular ribbons replace the reference's fine strands,
especially in the top-left foreground and nearby road margins. The repeated
images show the same tradeoff. This is a performance/appearance choice, not
equal silhouette coverage or optical parity. Existing stepped road margins
and the smoother distant aggregate remain visible.

The candidate is selected only through the explicit launch setting
**`SIDEWAYS_MEADOW_BLADE_BUDGET=120`**; the source default remains **360**.
The separate bounded wind-step check is still pending at this review.
These fixed-wind, downsampled JPEGs do not validate moving blades, temporal
shimmer, camera transitions, fine full-resolution detail or other lighting
and weather. The pause menu obscures part of two views. No runtime source or
defaults were changed during this review.
