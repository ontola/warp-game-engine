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

Recorded measurements from 2026-10-28:

- [Cached stadium ambient visibility](../../../engine/docs/stadium-sky-occlusion.md):
  Geometry-derived darkening under roofs, shared across views. Measured +0.43 ms
  GPU at native 4K/four views; no indirect light bounces yet.
- [Crowd silhouettes and measurements](../../../engine/docs/growing-guns-crowd.md):
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
