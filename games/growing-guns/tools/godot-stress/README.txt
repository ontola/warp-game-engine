Growing Guns: external Godot stress harness

From the repository root (Windows PowerShell):
  powershell -ExecutionPolicy Bypass -File games/growing-guns/tools/godot-stress/run-stress.ps1

This runs three alternating bullet/explosion pairs using the original exported
C:\dev\growing-guns\MoreRounds.exe and adjacent MoreRounds.pck. Override GameExe
for another installation. Results default to C:\dev\warp-bench\godot-stress-<timestamp>,
outside the repository. Override OutputRoot for another new output directory.

One run: add -Repeats 1 -Scenarios bullets
Separate JPEG runs: add -Repeats 1 -CaptureOnly

The probe is loaded from beside this launcher. No legacy game source or PCK is
copied or edited. A temporary override.cfg selects the existing packed scene
and adds the probe autoload. Existing override.cfg is never overwritten; only
identical owned content is removed. Each process is bounded to 120 seconds,
and timeout termination targets only its owned process handle. If the launcher
itself is forcibly terminated, inspect the saved owned-override.cfg.txt before
cleaning up its configuration manually.

SCENARIOS
Bullets: res://scenes/remote_minigun_benchmark.tscn,
build=minigun,minigun,minigun,shotgun,shotgun,shotgun.
Original warmup 2 seconds, measure 12 seconds, seed 721077 before scene ready.
Three miniguns offset three shotgun fire-rate penalties. Original projectile
caps and physics firing cadence remain active: measured 210 bullets/second.

Explosions: res://scenes/perf_benchmark.tscn, mode=wall_spray.
Original warmup 3 seconds, measure 15 seconds, scene seed 721077. Four bots
use minigun/explosive/explosive, plus nominal 40/s simulated-time wall spray.
Terrain is not regenerated. Manual spray is absent from ev_explosions; consult
blast_calls/full/cheap too. Full/cheap are game branch counters, not a count of
simultaneously visible effects. Existing VFX caps and time budgets are retained.

MEASUREMENT
Fixed 1920x1080 windowed, offscreen (-4000,0), no focus, uncapped, vsync off,
render scale 1 and time scale 1. The screen-claim coroutine is cancelled by
advancing its epoch. PerfGovernor is disabled and quality held at 1, explicitly
preventing adaptive workload reduction. Trace is disabled with GG_NO_TRACE.

frames.csv records consecutive monotonic process-callback wall intervals only
inside the observed measure phase. Boundary intervals, startup and final result
printing are excluded. These are not presented frame times or total CPU time.
Viewport GPU and render-CPU telemetry is separate, may lag, and must not be
subtracted from wall time to infer CPU time. Zero timings are unavailable.
Probe and GPU timestamp instrumentation overhead is included.

summary.json includes interval percentiles, actual window/viewport dimensions,
render/quality min-max, event totals, peak projectiles and blast profiles.
manifest.json preserves exact arguments, process exits, executable/PCK/probe
hashes and the actual renderer adapter startup line. Image readback occurs only
in separate CaptureOnly runs, never inside measured runs.

RECORDED EVIDENCE, 2026-10-28
evidence-20261028.json is a compact derivative of six completed runs on an
NVIDIA GeForce RTX 5070 Ti, Godot 4.6.2 official 71f334935, Vulkan Forward+.
Bullets: 2520 in each 12-second measure; peak 474-476 active. Loop median
10.33-11.49 ms, p95 22.50-26.39 ms; viewport GPU median 0.089-0.090 ms.
Explosions: 713-843 total blast calls per 15 seconds. Loop median 12.62-18.12 ms,
p95 40.70-75.50 ms, p99 103.76-115.37 ms; GPU median 2.32-2.48 ms.

The difference suggests host work/waits dominate over viewport GPU saturation;
it does not establish exclusive CPU attribution. Early measurement was slower
than late measurement: fixed warmup did not establish a stationary workload.
Cache settling, bot activity, terrain depletion and effect budgets can all
contribute. Fixed seed does not guarantee identical timing-dependent workloads.
No errors/warnings preceded probe completion; shutdown errors are preserved in
the original logs. The low-load one-minigun pilot is excluded.

Original raw evidence is retained at C:\dev\warp-bench\stress-20261028\godot-final;
full derived analysis at C:\dev\warp-bench\stress-20261028\godot-results.json.
These local paths are provenance, not dependencies for running the harness.
The probe is byte-identical to the tested probe. The portable launcher changes
only the default output directory. No cross-engine speedup claim is supported.
