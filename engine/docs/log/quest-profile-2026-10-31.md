# Quest 2 cost investigation, 31 October 2026

The current minimum profile misses the 8.33 ms target because both CPU HUD
preparation and GPU effects can exceed the budget independently. Lava is also
materially expensive when it covers most of the view. These measurements locate
costs; they do not establish a stable 120 fps build or implement the proposed fixes.

## Active gameplay

Measured builds use game revision `cf22e50` plus the instrumentation in this
change (before the later dash/card retirement). Physical Quest 2 / Adreno 650, runtime 1440×1584 per eye, 120 Hz, single-sample
direct sRGB attachments, minimum graphics, all gameplay effects enabled.
Opt-in timestamps cover both eyes and are read asynchronously with the existing
four-slot ring. No blocking profile readback is added to the live frame loop.
The ordinary XR GPU-completion wait remains. Instrumentation and driver
scheduling can perturb timings; timestamp intervals are not hardware utilization.

The first capture contains 8,235 GPU samples after skipping the first 240 frames,
10:05:40–10:06:57 Europe/Amsterdam. A second usable capture contains 5,570 samples,
10:13:13–10:14:04. Different player actions make these separate observations,
not a before/after optimization comparison.

| GPU scope, both eyes | First median | First p95 | First maximum | Second median | Second maximum |
| --- | ---: | ---: | ---: | ---: | ---: |
| Opaque geometry, including lava | 3.389 ms | 4.748 ms | 7.688 ms | 3.661 ms | 7.417 ms |
| FX (billows, embers, ion columns, streaks/motes) | 0.0005 ms | 1.669 ms | 33.804 ms | 0.0005 ms | 24.272 ms |
| Sprites (including muzzle flashes/bullet trails) | 0.042 ms | 0.124 ms | 2.457 ms | 0.042 ms | 2.619 ms |

The FX median reflects frames with no active FX; it is not the typical cost of
an explosion. Individual FX recipes are not separated by these timestamps.
Geometry includes blocks, cages, guns, characters and lava. The measurements
cannot assign the whole geometry duration to lava or assume scope maxima coincide.

### CPU hitches: repeated text preparation

The former aggregate HUD timer also included overlay swapchain acquisition.
Splitting it confirms that the large hitches are inside `HudRenderer::prepare`,
not acquiring the full-vision tint images. Examples from the second capture:

| Frame | HUD preparation | Glyph warmup/cache work | Visible atlas preparation | Overlay acquisition |
| --- | ---: | ---: | ---: | ---: |
| 1695 | 87.796 ms | 38.135 ms | 49.110 ms | 0.0108 ms |
| 4648 | 172.313 ms | 165.066 ms | 6.875 ms | 0.0084 ms |

These are CPU wall intervals, including library/driver calls and scheduling,
not GPU shader time. Warmup also includes atlas trim and text renderer allocation.
The shaping/geometry-upload phases are small in these two examples.

Code trace: `render/src/hud.rs::prepare_text` warms an entire alphabet for every
new `(font, font_size.to_bits())` up to 96 px. Growing Guns' `hud/mod.rs`
`damage_numbers` derives a continuously varying size from distance, and its
banner uses animated scale. `hud/style.rs::Label::draw` emits eight offset
outline copies plus the fill. Each new size can trigger new alphabet/glyph
rasterization and atlas traffic; exact-width/font-size cache keys also reshape
runs. The outline copies are not the engine's custom stroked-glyph path.

An isolated Quest reproduction (`hud_font_churn`, nine copies of one label,
1280×720 sRGB HUD, 120 measured frames after 16 warmups) isolates this behavior:

| Font sizing | Median CPU preparation | p95 CPU preparation | Maximum CPU preparation | Median GPU draw |
| --- | ---: | ---: | ---: | ---: |
| Continuous 60–80 px | 4.793 ms | 5.816 ms | 9.599 ms | 0.071 ms |
| Fixed 60 px | 0.030 ms | 0.033 ms | 0.045 ms | 0.052 ms |
| Rounded to 4 px | 0.030 ms | 0.036 ms | 7.625 ms | 0.066 ms |

Continuous sizing spends a median 3.909 ms in warmup. Quantization still hitches
when a new bucket is first used. A separate large-text probe above the warmup
limit measured 1.159 ms median preparation (mostly visible glyph atlas work),
versus 0.028 ms fixed. These are diagnostic labels, not a replay of the complete
HUD, and they do not predict the size of every gameplay hitch.

## Controlled lava comparison

`minimum_lava --bench --paired` alternates flat/detailed/detailed/flat (ABBA)
inside one renderer with identical targets, camera, animation time, mesh order
and material uploads. There are 40 warmup frames and 64 measured ABBA cycles.
Resolution is 1440×1584 per eye on the Quest GPU, without OpenXR/compositor.
No clocks are pinned; adjacent paired comparisons reduce clock drift, but the
absolute figures must not be substituted for runtime game measurements.

| Scene | Flat GPU geometry median | Detailed GPU geometry median | Mean paired extra GPU cost |
| --- | ---: | ---: | ---: |
| Almost full-view exposed lava | 2.738 ms | 5.425 ms | **2.671 ms** |
| Lava completely behind an opaque floor | 2.561 ms | 2.560 ms | −0.001 ms (noise) |

Exposed paired deltas range 2.620–2.701 ms. CPU submission-plus-completion deltas
agree at 2.684 ms mean. The earlier sequential-run estimates of +0.32/+1.62 ms
were not reliable estimates for the full-view case. An intermediate 80 m cover
still left distant lava visible and measured +0.337 ms; the corrected 900 m
cover fully hides the lava and removes the incremental cost.

## Optimization order

1. Reuse rasterized text sizes; animate placement/scale without requesting a new
   font raster every frame. Warm a bounded set of sizes before animation, and
   avoid whole-alphabet warmup for continuously sized text. Preserve readability.
2. Give minimum-profile FX cheaper screen-area-bounded recipes (or lower-resolution
   rendering with correct opaque occlusion). Keep explosions and ion warnings.
3. Replace the two-read/ALU-heavy lava material with a baked colour tile and cheap
   scrolling. Validate the visual result and paired timing before adoption.
4. Keep muzzle flashes and bullet trails: the sprite pass is usually inexpensive.

## Reproduce and evidence

Create `files/growing-guns/xr-profile` with `adb shell run-as
com.ontola.warp.growingguns touch ...` and restart the APK. Remove that marker and
restart to disable detailed live logging. It is development-only and off by default.
`Renderer::set_async_gpu_profiling` now works for minimum rendering too, with
`minimum_geometry`, `minimum_fx` and `minimum_sprites` per-eye intervals.
It requires timestamp queries inside encoders; unsupported devices return an error.

```sh
python3 engine/tools/analyze-quest-profile.py build/quest-native/profile3-live.log
cargo run -p render --example minimum_lava -- OUT --bench --paired
cargo run -p render --example minimum_lava -- OUT --bench --paired --covered
cargo run -p render --example hud_font_churn -- OUT.csv
cargo run -p render --example hud_font_churn -- OUT.csv --fixed
cargo run -p render --example hud_font_churn -- OUT.csv --quantized
```

Cross-build examples with cargo-ndk for the actual Quest. The paired lava probe
explicitly refuses unsupported timestamps (the tested Metal backend lacks
inside-encoder timestamps); it does not silently report CPU time as GPU time.

Local raw artifacts under `build/quest-native/`: `profile-live.log`,
`profile3-live.log`, their `*-summary.json`, `paired-exposed.csv`,
`paired-covered-full.csv`, and `hud-{continuous,fixed,quantized,large,large-fixed}.csv`.
Validation: Android build/signature/alignment, on-device paired GPU samples,
HUD probes, two minimum GPU regressions, 12 HUD tests, three XR tests and focused
all-target checks. Detailed profiler logging is removed from the final play setup.
