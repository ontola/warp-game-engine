# Native-resolution CPU and GPU profiling

`app::profile::bench` is the shared headless measurement path. It advances one
fixed simulation tick and renders a frame with the ordinary scene, HUD and
presenter. `Headless::render_at` submits without reading back pixels. Existing
`Headless::frame_at` still returns an image; legacy `app::bench` still measures
readback throughput and must not be used as gameplay FPS.

The report records individual frame IDs, CPU simulation/game draw/render
encoding/HUD/present encoding, completion wait, GPU intervals with view and
parent indices, and p50/p95/p99/max. Warmup is separate. Actual GPU output size
must match the requested native size. Dropped, pending and erroneous timestamp
samples are explicit. `KERN_CPU_SPANS=1` adds renderer phase detail, including
queue submission. There is no dynamic resolution or screenshot in the sample
set. The caller records game scenario, adapter, build and environment settings.

SpaceRacer exposes it through `--bench`; other games can call this API from
their benchmark command. This does not silently change every game's legacy
benchmark format.

```powershell
./games/spaceracer/tools/build-native.ps1 -Profile release
$env:KERN_CPU_SPANS='1'
./build/spaceracer-native/spaceracer.exe --demo --seed 35 --biome city --difficulty hard --players 1 --ticks 1200 --warmup 120 --bench 360 --width 3840 --height 2160 --out build/city.json
python engine/tools/profile-report.py build/city.json --out build/city.html
```

`games/spaceracer/tools/profile-native.ps1` runs a serial native-4K matrix of
seeds, biomes and player counts into a fresh directory. It records executable
SHA256, source revision/status and hardware, and rejects incomplete timestamp
coverage. Compile first, then profile. Do not run another GPU fixture or game
alongside it. The report viewer accepts multiple JSON files; it warns when
scenario metadata is missing or differs. Hover over a graph to inspect a frame
and its per-view GPU passes. It is self-contained and makes no network requests.

SpaceRacer's `--profile-no-reflections` is an explicitly recorded diagnostic
omission for cost attribution. Never call that a full-quality result. Environment
overrides are recorded too; compare the same configuration and use an optimized
build for final timings. `WARP_TARGET_FPS=0 WARP_RENDER_HEIGHT=0` disables
adaptation and uses native rendering in an explicitly requested windowed test.
The headless profile always uses native rendering regardless of those settings.

## Interpretation

CPU values are wall time, including OS scheduling and driver work, not core
utilization. GPU intervals can include preemption from another application.
Root pass sums are instrumented scope costs, not hardware utilization. Children
must not be added to parents. The measured GPU interval excludes HUD/presenter
GPU work. CPU and GPU can overlap: adding their costs or inverting completion
wait does not give interactive FPS. The headless runner intentionally waits for
completion each frame to bound queued work and attribute samples; it does not
reproduce a swapchain, controller latency or frame pacing.

Keep raw reports. Compare identical seed/biome/difficulty, race tick, player
count, sample count, resolution, effects and environment. Include later race
moments, boosts, dense effects and split screen. Inspect before/after imagery
separately; a faster missing effect is not an optimization. Record background
CPU/GPU activity with OS tools, without closing someone else's applications.

## Further tooling Warp needs

The engine already has asynchronous timestamp rings, F10 pass timings, F12
feedback and Sideways' frame-budget/hitch tooling. Build on those, rather than
creating independent profilers in each game. Next priorities:

1. Shared live CPU spans for input, fixed-tick catch-up count, Luau hooks/GC,
   draw preparation, driver submission, surface acquisition and present wait.
   Export frame IDs through the existing live-control channel.
2. Timestamp HUD and presentation as well as scene rendering, so full-output
   cost is covered. Preserve authored parent/view scopes and report gaps.
3. An opt-in rolling trace around F12 or a hitch, with resolution/quality,
   loading/paused/hidden/capture state and camera cuts. Use delayed readback;
   never stall a gameplay frame to obtain a profiler result.
4. Background process CPU deltas, GPU clocks/power and per-process GPU activity
   alongside traces, with privacy-conscious process-name-only reports.
5. Repeatable effect-heavy scenarios and hardware-specific regression budgets,
   image checks and confidence across repeated runs. Keep presented-frame
   latency separate from this serial headless cost measurement.

None of those future additions are claimed by the current headless API.
