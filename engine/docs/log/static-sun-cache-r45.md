# Static/dynamic sun cache: r45 measured experiment

Status: implemented behind `KERN_STATIC_SUN_CACHE=1`, **default off**. The first
native GPU comparison exercised reuse correctly but showed no useful frame-time
improvement in this forest. Exact final-image parity is not established. Do not
promote the cache or claim progress toward 8.33 ms from these results.

## Implementation and projection

`static_sun.rs` partitions original source owners into static and dynamic lists.
Terrain and rocks can be cached; cars, animated attachments, trees and other
wind foliage remain dynamic. A miss rasterizes static depth. Every dynamic
update copies the cached depth into the existing final map, then rasterizes
dynamic depth with Load/Less. This computes the intended minimum of both depth
fields. Source, membership, shader and raster-key changes invalidate the cache.
Night skips solar work; dawn restores it. Existing near shadows are unchanged.

`KERN_SUN_CACHE_GUARD_METRES=32` is a separate, default-zero projection option,
usable with cache either off or on. It holds a conservative light-space anchor
through car translation. In this one-view run, shared half-extent is 176 m and
texel pitch 8.59375 cm, versus the ordinary 96 m / 4.6875 cm. This quality
tradeoff has **not** been accepted by comparing guard 0 against 32. The current
pair used guard 32 on both sides, isolating the cache implementation.

## Evidence and timing

Evidence: `C:/dev/perf/warp-sideways-20261025/r45-static-sun/native-a/report.json`.
The executable is the immutable
`r45-instance-groups/sideways-r45-controls.exe`, SHA-256
`0464edfa2a5c309be800df9fc3be8f6fa765657cf5b6b7d5053fe2d090d7765b`.
The build passed 410 renderer and 164 game tests. Five CPU harness tests cover
the deterministic scenario and timing/phase validation.

One native 1920×1080 view, 120-blade recipe, same stage and inputs, actual AI
physics at two 120 Hz ticks per rendered frame. Wind is held at 1.9 s; this does
not validate animated wind. OFF then ON ran 416 frames each. Every render-state
hash and all capture identities matched. GPU intervals are asynchronous
timestamps joined by frame ID. Warmup and synchronous captures are excluded.
CPU frame wall time is separate. This is one ordered pair, not an ABBA timing
estimate; concurrent host activity can still affect the result.
`budget-clean.json` in each run excludes diagnostic captures. The original
`budget.json` is retained unchanged and includes expensive readback frames;
its CPU means must not be presented as ordinary frame cost. The table below
uses the separately validated clean GPU samples from `report.json`.

| Section | Clean samples per run | GPU interval OFF → ON, ms | Static redraws / eligible frames ON |
| --- | ---: | ---: | ---: |
| Moving chase, noon | 94 | 16.423712 → 16.525712 | 2 / 128 |
| Held camera, noon | 95 | 15.836320 → 16.027840 | 1 / 128 |
| Moving chase, night | 47 | 14.224288 → 14.069984 | No solar work in either run |
| Moving chase, sunrise | 71 | 23.146432 → 23.106240 | 2 / 96 |

The cache therefore reused static depth in 126/128 moving-noon frames,
127/128 held-camera frames and 94/96 sunrise frames. The very first dawn frame
rebuilt static depth. It did not freeze moving cars. The small night difference
cannot be a saving in solar passes: neither run submitted any of them.

The reason this cache does not pay here is the caster mix: **2,641 static
owners versus 319,184 dynamic owners**. At the first dawn capture the dynamic
pass still selects 52,662,509 triangles, versus 54,303 static triangles. At
noon frame 64 it selects 8,541,583 versus 47,371. Wind foliage dominates the work
that remains. Sunrise reference solar raster median is 7.118336 ms; dynamic
raster alone remains 7.022080 ms. Individual pass medians are not additive.

Each seeded frame copies 67,108,864 bytes. Median copy cost is approximately
0.071–0.087 ms. The actual incremental allocation is **121,307,056 bytes
(115.69 MiB)** including the static map, owner lists and retained raster/cull
targets. This is not just the 64 MiB depth image.

## Image review and limits

All six full-resolution PNG pairs were checked numerically. The two exact
pairs and all four differing pairs were visually inspected. No broad missing
shadow, changed car silhouette or obvious geometry omission was visible.

| Frame | Phase | Changed pixels / 2,073,600 | Maximum channel difference, 8-bit | Mean absolute channel difference |
| --- | --- | ---: | ---: | ---: |
| 64 | Noon chase | 0 | 0 | 0 |
| 128 | Noon chase | 32 | 1 | 0.000005144 |
| 256 | Noon held camera | 0 | 0 | 0 |
| 320 | Night | 249 | 1 | 0.000041152 |
| 321 | First sunrise frame | 1,224 | 26 | 0.000258327 |
| 416 | Sunrise | 458 | 7 | 0.000083912 |

Differences are confined to distant vegetation bands. The night discrepancy
establishes that at least some final-image variation occurs without solar
raster or solar sampling. The grass pipeline uses GPU atomic compaction and
half-float order-dependent accumulation; that is a plausible source, not a
diagnosed explanation for every daylight difference. No same-setting repeat or
raw shadow-depth comparison was performed. In particular, the first-dawn
maximum of 26 must not be excused as proven harmless numerical noise. The
strict PNG parity check remains failed; source render identities passed.

The hashed review is `native-a/visual-review.json`. No extra eight-run suite is
scheduled for this marginal result. A future correctness investigation should
first compare same-setting repeats or raw final shadow depth, separately from
grass accumulation. It must not silently relax the existing parity gate.

## Next engine work suggested by the measured bottleneck

Prioritize **receiver-driven, shared sun pages with current animated casters**,
rather than extending the static-only cache to more copies. The existing
`foliage_sun_pages.rs` provides bounded world-light page residency, but its tree
producer explicitly uses rest wind. It cannot replace current animated shadow
maps while claiming equivalent lighting.

1. Reuse conservative spatial owner groups for light-space caster lists, and
   deduplicate requests from all four views. Include off-screen blockers along
   the complete light ray. The current 2048² near map is rerendered separately
   for every view; culling dispatches against the source owner population.
2. Request only pages containing visible receivers, render current wind/car
   poses into the requested dynamic pages and preserve complete global depth
   coverage. Keep the existing maps as a fallback for missing/overflow pages.
   This targets repeated dynamic raster, not just the small static fraction.
3. Only then experiment with animated-page reuse using an explicit projected
   displacement bound and a declared error budget. Exact mode must invalidate
   on any relevant pose change. Near-map pitch is 1.5625 cm, so a small wind
   movement can invalidate it quickly; a fixed-wind benchmark would overstate
   the real benefit. Rest-pose or time-averaged foliage is an approximation and
   needs separate moving-wind image acceptance.

The bounded first acceptance should compare one and four views at low sun and
noon, camera motion/cuts, wind on, and dawn. Record dynamic triangles, requested
and reused pages, overflow/fallback, memory and actual GPU interval/pass costs.
Keep guard resolution changes independent. No performance benefit from that
future architecture is claimed here.
