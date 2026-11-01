# Quest lightweight graphics update — 1 November 2026

Built on main through `2256242`, with the earlier cost investigation in
[31 October profiling](quest-profile-2026-10-31.md). Physics, damage, eye
resolution and the requested 120 Hz refresh rate are unchanged.

## Changes

- Low/minimum HUD labels reuse bounded raster pictures and animate quads.
  Fixed raster buckets avoid warming an alphabet at every floating font size.
- Minimum lava bakes colour and crack shaping into a seamless 512×512 texture:
  one sample in the opaque pass, about 1.33 MiB with mipmaps.
- Explosion visuals cluster neighbours within 3 m and a 20 ms birth interval.
  Full/low/minimum respectively allow 3/2/1 full bursts per tick,
  25/12/4 per half-second and 256/96/12 live decorative puffs. Pending queues
  hold at most 13/8/8 distinct bursts. Overflow never changes damage.
- Full desktop keeps the newly merged baked volumetric smoke and adaptive
  smoke resolution. Low/minimum do not enable that animation. Minimum uses
  six-vertex analytic fire/smoke blobs and a projected coverage budget that
  prefers fire, shares nearby-eye selection, and leaves ion warning columns
  independent. A single large fire blob can exceed the coverage target;
  camera-inside remains the expensive case.
- Ion columns use conservative projected bounds rather than a fullscreen
  triangle, retaining the existing ray-integrated fragment shader.
- Bullet tracers have a short soft wake and an eye-facing round glow. Two
  quads replace one long shaft; the added quad reuses the muzzle-flash texture.
  No extra blur pass, lights or particle simulation. Minimum pixel sizing
  preserves the glow's aspect ratio at distance and steep view angles.

## Measurements and limits

Quest 2 Adreno 650, two 1440×1584 sRGB eye targets, no reconstruction/MSAA.
GPU intervals are both eyes combined. CPU wall time includes submission/wait.
Synthetic probes have no OpenXR compositor and do not establish live FPS.

| Isolated workload | Earlier | Updated |
| --- | ---: | ---: |
| Continuously scaled HUD label, CPU preparation median | 4.793 ms | 0.023 ms |
| Same small label, GPU median | 0.071 ms | 0.143 ms |
| Almost-full-view lava, paired incremental GPU cost | 2.671 ms | 0.232 ms |
| 512-request blast fixture, effects GPU median | 2.383 ms | 2.273 ms |
| Camera inside blast fixture, effects GPU median | 8.955 ms | 6.660 ms |
| Camera inside, effects GPU p95 | 10.294 ms | 6.666 ms |

HUD/lava measurements precede the final main merge; effects columns compare
before/after the final coverage limit and merge, not a single isolated code
change. Runtime clocks were not pinned. Large nine-copy outlined label quads
cost about 1.023 ms GPU versus 0.344 ms previously; their CPU preparation falls
from about 1.159 to 0.036 ms. Label caching trades GPU fill/memory for fewer
CPU stalls and is opt-in. Large text/new strings still need first-use work.

The final 512-request test creates four clusters (508 requests merged), one
full and three cheap bursts, six decorative puffs plus an independent ion
warning. Outside: geometry 2.236 ms, effects 2.273 ms, CPU wall 5.587 ms.
Inside: geometry 1.302 ms, effects 6.660 ms, CPU wall 9.859 ms. Thus this
worst-case synthetic scene still exceeds an 8.33 ms total-frame budget.
[Recorded statistics](data/quest-minimum-20261101.json) include CSV hashes.

## Validation

736 renderer tests pass (17 ignored), 103 Growing Guns app tests pass (2
ignored), and both lobby tests pass. Workspace/all-target compilation passes.
Stereo GPU tests cover end-on tracer roundness at three camera pitches,
explosion/ion visibility and expiry, and direct sRGB attachments. Explosion
unit pressure tests cover clustering, bounded queues/live puffs and retention
of ion warnings. The APK was signed, installed and launched on Quest; active
headset presentation and sustained 120 fps require the separate playtest.

To reproduce, cross-compile `minimum_blasts` with cargo-ndk for arm64 Android,
copy it alongside libc++_shared.so, and run `minimum_blasts OUT --bench --spam`
and `minimum_blasts OUT --bench --spam --inside` separately from the VR app.
Use `hud_font_churn OUT.csv --cached` for label reuse and `minimum_lava` for
paired lava measurement (see the previous investigation for its switches).
