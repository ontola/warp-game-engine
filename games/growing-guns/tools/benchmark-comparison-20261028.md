# Growing Guns: current Warp and installed Godot benchmarks

Measured 28 October 2026 on the same Ryzen 9 5900X / RTX 5070 Ti machine
(NVIDIA driver 617.14). Three repetitions per case, 18 measured runs total.
The one-view engine runs were interleaved, reversing engine order on repetition
two. Four-view Warp runs followed. No simultaneous game or build was launched
by this task. Background CPU work remained active.

Warp has shorter measured loop intervals in these stress fixtures. This is
**not an apples-to-apples engine speedup**: the workloads, effect budgets and
timing scopes differ. GPU work is relatively stable; CPU-side wall time and
scheduling have much larger tails. The current data does not establish
presented gameplay FPS or the cause of a particular playtest hitch.

## Results

All numbers are milliseconds. Each column is the median of the corresponding
per-run statistic across three runs, not a pooled percentile.

| Engine / workload | Output | Loop median | Loop p95 | Loop p99 | GPU median |
|---|---|---:|---:|---:|---:|
| Godot bullets | Native 1080p, one view | 15.72 | 34.34 | 57.85 | 0.09 |
| Godot explosions | Native 1080p, one view | 24.04 | 108.90 | 140.14 | 2.57 |
| Warp bullets | Native 1080p, one view | 3.42 | 6.58 | 9.32 | 1.58 |
| Warp explosions | Native 1080p, one view | 4.70 | 10.61 | 19.94 | 1.69 |
| Warp bullets | Native 4K, four views | 9.29 | 14.50 | 17.59 | 5.43 |
| Warp explosions | Native 4K, four views | 11.05 | 17.85 | 26.78 | 5.78 |

**Loop** means successive process-callback wall intervals for Godot, and a
serialized simulation/update/render/GPU-completion wait for Warp. These are
different measurements. Warp excludes audio, HUD and presentation. Godot's
existing scenes retain their audio and UI. Neither column is presented frame
time; do not convert this table into a comparative gameplay FPS or speed ratio.

**GPU** means Godot's delayed viewport timing or Warp's union of instrumented
GPU ranges. Godot samples are not synchronized to the callback intervals.
Warp's GPU interval including gaps is also retained in the raw results.
Do not subtract GPU time from a loop interval to manufacture CPU time.

Warp CPU-side submission/update section medians were 1.44 / 2.47 ms at
1080p for bullets / explosions, and 2.85 / 4.06 ms at four-view 4K.
These are CPU-side wall durations, including scheduling and possible API waits,
not exclusive active CPU execution time.

## What was actually tested

- Warp: exact previously launched release executable for
  `34a189509044325238bd3857366113e1f6307291`,
  `C:/dev/warp-bench/engine-efficiency-20261028/playtest-34a1895.exe`.
  Original knight/paladin/ch10 skins loaded successfully in every run.
- Godot: installed `C:/dev/growing-guns/MoreRounds.exe` and adjacent PCK,
  Godot 4.6.2, export files dated 22 October. This is **not** a fresh export of
  source main `3daf27e` from 24 October. Executable/PCK hashes are recorded.
- Native resolution throughout; no DLSS or automatic quality reduction.
  Both used the RTX 5070 Ti. Godot was uncapped, vsync off, viewport/render
  scale held at 1920x1080 / 1.0 and its adaptive governor disabled.
- Warp uses 180 warmup ticks and 900 measured ticks at simulated 60 Hz,
  rendering as fast as possible with a completion wait per tick. All four
  players are pinned alive, so this includes animation/skinning but **no
  ragdoll workload**. The four-view case repeats the same overhead camera in
  all four quadrants; it is a capacity test, not four distinct player views.
- Godot uses its authored warmup and 12-second bullet / 15-second explosion
  measurement phases. Draw counters advanced and GPU timings were positive:
  moving the window offscreen did not skip its rendering in these samples.
- Screenshots were inspected separately: Godot in capture-only runs, Warp
  after measurement. Readback and startup are excluded from timing samples.

The bullet workloads are particularly different. Godot renders one remote
shooter and tracers against an almost empty background, with roughly 472-483
live projectiles at peak. Warp renders a complete lit arena and four animated
characters, with a 64-visible-tracer cap. Both request about 210 bullets per
simulated second, but that does not equalize simulation or rendering work.
Godot produced 2,520 / 2,527 / 2,520 bullets: the middle run crosses one extra
seven-pellet trigger boundary. No run was discarded or repeated to hide this.
Warp produced exactly 3,150 shots in each 15-second simulated window.

The explosion workloads also differ. Godot has moving bots plus nominal
40/s fixed-point wall spray; terrain depletes and its full/cheap effect budgets
remain active. Warp targets intact visible terrain at exactly 40/s, resets
terrain/debris every three seconds or on target depletion, and includes that
reset cost. Every Warp run processed 600 explosions, removed 7,860 chunks over
15 resets, kept all four actors alive and used no empty-target fallback.
Godot's early explosion callback medians were 76-104 ms versus 16-20 ms in
the last three seconds: its aggregate median hides substantial phase changes.

## Bottlenecks and limitations

At four-view 4K, Warp's largest GPU passes are shading (about 1.9-2.0 ms),
main raster (1.1 ms), near-shadow raster (0.7-0.9 ms), followed by other
shadow work. Weighted character skinning is around 0.01 ms and runs once
for all views. Pass figures are approximate per-run means, not additive
speedup predictions.

On the CPU side, scene update and render submission dominate the recorded
sections. The explosion fixture's intentional arena resets cause some large
spikes. They must not be presented as proof of the cause of ordinary gameplay
hitches. GPU tails are much tighter than CPU/wait tails.

Host snapshots before/after runs showed 29-85% system CPU utilization and
substantial WSL and other application activity. This is consistent with the
user's warning about concurrent CI. It is not continuous telemetry, and cannot
attribute an individual slow frame to CI. GPU medians repeat closely; CPU
numbers should be treated as measurements under contention, not quiet-machine
limits. No unrelated process was stopped.

A stronger cross-engine comparison needs one shared scene/camera snapshot,
identical projectile/explosion replay and effect caps, plus matching timing
scope. Then add bots, audio, HUD and articulated deaths separately. The current
results are useful for locating costs and regression checks within each fixture;
they do not justify a claim that Warp is a particular factor faster than Godot.

All 18 runs passed their data/workload checks. Warp retained all 900 GPU
profiles per run, with no dropped profiles or validation errors. Godot's
shutdown resource/material diagnostics occur after its measurement-complete
message, and remain in the logs. The owned temporary Godot override was removed;
no benchmark game remained running after the captures.

## Evidence and reruns

[Compact checked results](benchmark-comparison-20261028.json) retain executable
hashes, all per-run acceptance checks, distributions, counters and reset-frame
isolation alongside this report.

Raw artifacts, full analysis and exact orchestration are preserved at:

`C:/dev/warp-bench/compare-20261028-163747/`

- `manifest.json`: executable identities, arguments, order and host snapshots.
- `results.json`, `summary.txt`: per-run distributions, event counts, GPU passes,
  pool occupancy, reset versus ordinary frames and diagnostic phases.
- `warp-*.csv`, `warp-*.csv.gpu.csv`: all measured CPU/GPU frame data.
- `godot-*/01-*/frames.csv`, `summary.json`: raw callback/render telemetry.
- `visual-manifest.json`, `godot-0-*/01-*/visual.jpg`, `warp-1-*.png`:
  fresh visual evidence. Visual-only runs are not pooled with performance runs.
- `run.ps1`, `analyze.py`: retained orchestration and validation. Use a fresh
  output directory when repeating; existing evidence is deliberately preserved.

Portable Godot launcher: [godot-stress/run-stress.ps1](godot-stress/run-stress.ps1).
Current Warp invocation, repeated three times for each scenario and layout:

```text
playtest-34a1895.exe bench --stress bullets --views 1 --size 1920x1080 --seed 721077 --csv <fresh-output>.csv
playtest-34a1895.exe bench --stress explosions --views 4 --size 3840x2160 --seed 721077 --csv <fresh-output>.csv
```

The archived `gg-bench.ps1` / `gg-versus.ps1` / `gg-perf.ps1` wrappers can
checkout an old branch, and the old stress wrapper hardcodes an older binary.
They were not used. No runtime or graphics defaults changed in this benchmark.
