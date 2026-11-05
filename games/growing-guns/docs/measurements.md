# Growing Guns on Warp: measurements

Moved from the README on 2026-10-30. These are point-in-time results; each entry says what it measured and on which commit.

The opt-in stress fixtures run 180 warmup ticks and 900 measured ticks at the
simulation's 60 Hz, with one offscreen render and GPU completion wait per tick.
They sustain 210 rifle shots or 40 gameplay explosions per simulated second;
these rates are not wall-clock presentation FPS. Explosion targets are restored
every three simulated seconds, or sooner when depleted, and those costs remain
in the measurements. Players are pinned and kept alive. The stress path flushes
arena changes, debris and effects just as play does; audio and HUD are excluded.
It checks actual event counts, writes raw CPU/GPU timings and reports effect
pool limits. Screenshot readback happens after measurement. The ordinary bot
benchmark is a different fixture; these results are not interchangeable with
its old results or with Godot's differently staged stress scenes.

## Continuation validation, 30 October 2026

The interrupted roster/render/network work was resumed on top of `a3de830`.
The shared shade shader initially failed WGSL parsing (`let _ = p`), even
though the Rust binary compiled. Removing the invalid declarations restored
699 passing renderer tests (16 ignored). Another 162 app/assets/game/network
tests passed (one ignored), and the workspace all-target check passed after
updating the two remaining `app::Config` initializers.

After merging upstream through `22d9a5d`, the same checks pass: 701 renderer
tests plus 162 app/assets/game/network tests, 17 ignored in total. The release
binary and 97-page website also build; the website reports pre-existing links
to absent historical capture artifacts. Timings below belong to the pre-merge
continuation, not a new performance comparison against the integrated engine.
The integrated `bench --sim` run reports 0.075 ms median / 0.285 ms p95 total
CPU tick cost, but a 308.397 ms maximum around arena setup. That synchronous
benchmark spike is not fixed by retaining roster visuals and is not a measured
windowed-game stall; round-transition latency needs separate profiling.

The `ui-hotjoin.json` GPU script passes all expectations across one, two and
three local seats, leaving, and disabling split screen. The reusable output
texture is replaced when viewport layout changes, clearing stale pixels from
the unused fourth quadrant. `ui-bots.json` adds and removes a bot through the
actual pause menu. Each script creates its renderer once, at initial capture;
roster edits retain the scene. Unit tests additionally require the scene
allocation and renderer-rebuild flag to remain unchanged through these edits.
`nettest` passes local join, movement, destruction and arena/card propagation.
It is loopback evidence, not a two-machine or jitter/latency acceptance test.

Apple M5 Pro, Metal, full lighting, four views, ordinary bot benchmark,
30 warmup frames followed by 120 measured frames, one run per size:

| Native output | CPU submit median / p95 | GPU median / p95 | Serialized wall median / p95 |
| --- | --- | --- | --- |
| 1920 x 1080 | 1.26 / 1.67 ms | 8.88 / 9.17 ms | 10.60 / 11.55 ms |
| 3840 x 2160 | 1.21 / 1.39 ms | 20.78 / 21.20 ms | 22.52 / 23.27 ms |

Both runs retained 120/120 GPU samples with zero errors or drops. Renderer
construction was 246/236 ms in these warm runs; the first scripted capture
before warming the driver took 2123 ms. No cold-start or before/after speedup
claim follows. These are offscreen timings with serialized GPU completion,
not presented FPS, and exclude adaptive quality, HUD and audio. The 4K/120 FPS
target is not met. The largest 4K pass is shade at 11.746 ms.

The compact arena DFAO grid is 52 x 18 x 52 at 2 m voxels, baked at arena setup.
This avoids repeated baking during destruction but leaves approximate stale
occlusion around removed walls until the next arena setup. It is not dynamic
DFAO correctness. Cached model-derived gibs and adaptive quality are also part
of the inherited work; physical controller feel and display-driven quality
changes were not validated by these headless tests.

Commands:

```sh
cargo test --locked -p render --lib -- --test-threads=1
cargo test --locked -p app -p assets -p growing-guns -p growing-guns-app -p net --lib --bins
cargo check --locked --workspace --all-targets
target/release/growing-guns script games/growing-guns/scripts/ui-hotjoin.json --players 1 --out build/hotjoin
target/release/growing-guns script games/growing-guns/scripts/ui-bots.json --players 1 --out build/bots
target/release/growing-guns nettest
target/release/growing-guns bench --views 4 --size 1920x1080 --frames 120
target/release/growing-guns bench --views 4 --size 3840x2160 --frames 120
```

Recorded measurements from 2026-10-28:

- [Cached stadium ambient visibility](../../../engine/docs/log/stadium-sky-occlusion.md):
  Geometry-derived darkening under roofs, shared across views. Measured +0.43 ms
  GPU at native 4K/four views; no indirect light bounces yet.
- [Crowd silhouettes and measurements](../../../engine/docs/log/growing-guns-crowd.md):
  8,363 spectators, shared low-poly figures and GPU shoulder pivots. Three
  alternating 4K/four-view pairs measure about 0.35 ms added GPU time;
  geometry budgets and serialized benchmark FPS are documented separately.
- [CPU spike optimization comparison](../tools/cpu-spikes-20261028.md):
  24 alternating runs with exact workload parity; substantially cheaper
  explosion reset frames, but no general hitch-elimination claim. Includes
  reusable comparison tools and an opt-in `--cpu-thread` diagnostic.
- [Current roster benchmark and installed Godot comparison](../tools/benchmark-comparison-20261028.md):
  exact playtest commit `34a1895`, three repeats per workload/layout, with current
  animated characters, raw CPU/GPU timings and background-load evidence. The
  Godot and Warp fixtures differ; the report does not claim an engine speed ratio.
- [Sustained bullet/explosion results](../tools/stress-results-20261028.json):
  three alternating pairs per layout, retained executable identity and workload
  counts. These measurements precede the rebase onto the newer renderer; do not
  present them as measurements of the latest commit.
- [Godot probe and evidence](../tools/godot-stress/README.txt): existing Godot stress
  scenes, with resolution, event counts and timing scope recorded separately.
- [Renderer configuration sweep](../tools/perf-results-20261028.json): measured
  commit `74ea8f0`, ten successful Vulkan configurations and two failed DX12
  attempts. DX12 failed compiling the `cull` pipeline in FXC (X3663), before any
  timing data. The renderer now links DXC on Windows (wgpu `static-dxc`), which
  wgpu picks before FXC; this is not yet re-measured on DX12. This sweep is one fixed-order repetition, not evidence for a
  default change. At 4K/four views the default GPU median is 4.15 ms and serialized
  CPU-plus-GPU-wait median/p95 is 7.57/12.85 ms; this is an offscreen fixture, not
  a sustained 120 FPS gameplay claim. Each measured run retains all 600 requested
  GPU profiles. The sweep includes complete scene/effect uploads, unlike older
  ordinary-benchmark results.
