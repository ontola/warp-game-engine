# Frame budget and repeatable scenarios

Run from `engine/`. `frame-budget.py` uses Python 3.10+ and the standard library.
The scenario runner needs Python 3.11+ for TOML. Neither script builds the game.
The target is **8.333 ms for one complete frame at 120 FPS**. The checked-in
cases request **3840×2160 total output with DLSS Quality**, including the
four-view case; these are not native-4K measurements or four separate 4K outputs.

## One-command capture and report

Use an already built executable with DLSS support and its configured runtime:

```powershell
python tools/run-frame-scenario.py ../games/sideways/scenarios/noon-foliage.toml --exe C:/dev/warp-target-20261023/release/sideways.exe --env-file ../games/sideways/scenarios/lighting-pages.json --out C:/dev/perf/noon-001
```

This explicitly opens a **visible, fullscreen, bounded** play window, exits at the scenario
frame limit, then analyzes its JSONL with the same Python executable. It does
not stop another game. Run one GPU workload at a time. Output must be a new
directory. `--timeout 300` is the default; timeout terminates only the process
spawned by this runner and reports failure. Closing the window early retains
partial evidence and does not report a complete scenario.

`--hidden` instead uses synchronized smoke rendering: useful for image/contract
checks, **not gameplay cadence**. `--visible` explicitly selects the default.
`--windowed` allows OS window chrome/clamping; requested and actual GPU output
sizes are checked, and a mismatch is not accepted as an exact-resolution run.
`--build-ms N` can record an externally measured build duration; it never builds.
The optional JSON environment file overrides only nonsecret renderer settings.
Other renderer values are inherited and recorded, so compare manifests before
claiming identical runs. `KERN_STARTUP_PROFILE=1` is always enabled by this runner.

Outputs:

- `run.json`: command, exact executable/scenario hashes, Git revision and dirty
  status, selected environment, exit/timeout status, elapsed time, verified frame
  limit, optional external build time and startup state.
- `scenario.toml`, optional `environment.json`: exact input bytes.
- `frames.jsonl`, `final-state.json`, `stdout.log`, `stderr.log`: original evidence.
- `startup.json`: all `cli_startup` and `renderer_startup` structured records.
  CLI events are cumulative snapshots: `partition_end_ms` freezes at the first
  scene frame, while `run_end.elapsed_ms` also includes gameplay and writer flush.
  Renderer constructor and worker timings nest within that timeline; never add
  them to startup elapsed time. Full subprocess elapsed also includes teardown.
- `captures/`: JPEG previews (default maximum width 1600, quality 85) and metadata.
  Render `source_output_size` and image `capture_size` are separate and verified;
  a 1600-wide preview does not imply a 1600-wide rendered workload.
- `budget/summary.json`, `budget/report.html`: numeric result and standalone HTML;
  by default excludes diagnostic captures and the following cadence interval.
  Raw profiles and the iteration report retain their full cost. No internet,
  scripts or external assets are needed to view the report.

The preset `lighting-pages.json` selects world terrain visibility and 256 m
foliage pages, with the old far map disabled. It requests the current NVIDIA/
Vulkan setup. Copy/edit the case and preset for native or another device; record
that as a different configuration. This is a measurement preset, not evidence
that the enabled features satisfy the target.

For exact pixel comparisons, explicitly opt into full-size PNGs in the case:

```toml
[capture]
format = "png"
max_width = 0
```

The checked-in cases use `format = "jpg"`, `max_width = 1600`, `quality = 85`.
Full-resolution PNG encoding is expensive; capture frames are diagnostic samples.
F12 bug-report captures remain full-resolution PNGs.

## Cases and boundaries

| Case | Purpose | Measured sections |
| --- | --- | --- |
| `scenarios/noon-foliage.toml` | Fixed forest camera from seed 1, 3 km, first-stage F12 `1792855853301-192948-0` | 180 frames; first 60 warmup |
| `scenarios/sunrise-rocks.toml` | Fixed wall camera from first-stage F12 `1792849958527-134304-0`; sunlight/cache invalidation | Sunrise and afternoon, each 180 frames / 60 warmup |
| `scenarios/fourplayer-route.toml` | Four actual chase views with deterministic AI movement | Start at tick 6000, then two contiguous 240-frame sections / 60 warmup each |

Fixed cameras also drive grass residency and lighting focus. Cars remain at the
scenario's actual simulation state: these are scene-camera tests, **not exact
replays of old controller/car/lamp poses**. The fixed cases use zero physics ticks
per rendered frame. The route uses one 120 Hz simulation tick per rendered frame;
it is a repeatable workload, not real-time player input. A run does not prove
performance for all seeds, separated four-player locations or whole stages.

Section names, `section_frame` and `warmup` are written to CPU records. The analyzer
joins delayed GPU records by global `frame_id`, excludes each section's warmup,
and reports sections independently. It intentionally omits aggregate budget
statistics when several sections exist. Images during warmup still document cold
cache behavior. A fixed 60-frame warmup is not proof that every cache settled;
inspect the recorded cache counters and images.

## Analyze existing profiles

```powershell
python tools/frame-budget.py C:/dev/perf/noon-001/frames.jsonl --out C:/dev/perf/noon-001/budget-clean --exclude-diagnostics
python tools/frame-budget.py C:/dev/perf/rocks-001/frames.jsonl --out C:/dev/perf/rocks-001/sunrise-only --section sunrise-wall
python tools/frame-budget.py C:/dev/perf/old-gpu.jsonl --out C:/dev/perf/old-report
python tools/test_frame_budget.py
```

Optional `--skip-frames N` excludes another N eligible rendered frames **per
section**, after declared warmup. `--include-paused` includes paused rendering;
cadence still splits at phase boundaries. `--target-fps` changes the comparison
budget, not the data. Existing reports are protected unless `--force` is explicit.
Malformed/truncated JSON fails with its line number; do not silently discard a
partial final line and label the result complete.

## What the numbers mean

- **CPU main-thread execution** is OS user+kernel time spent executing the calling
  thread, excluding waiting and descheduling. Worker threads are not included.
  It is separate from CPU body wall time. Because OS counters are coarse, the
  report shows its multi-frame mean. Missing samples stay unavailable, never
  replaced by wall time or zero. The raw field is `cpu_execution_ms`.
- **CPU body** is frame-body wall time, including measured waits. Sections must
  partition that interval; invalid overlapping partitions are omitted from
  contributor statistics. JSON construction/enqueue happens after this interval,
  but is included in the next cadence. Startup/loading/host-idle calls have their
  own phase table and are excluded from gameplay summaries.
- **GPU covered interval** is the first GPU timestamp to the last, including any
  gaps between them. Work outside those endpoints (currently including HUD and
  presentation) is excluded. It is not inferred from the sum of passes.
- **GPU pass sum** is diagnostic only: passes may overlap. There is no FPS or
  overall budget verdict calculated from that sum, nor from CPU plus GPU.
- **Cadence** is real, unclamped entry-to-entry wall time between adjacent
  presented calls in the same measured section/phase. No interval crosses
  warmup, a loading call, a dropped call, an unpresented frame or a diagnostic
  frame excluded by the user. Entry rate is `interval_count / total_duration`,
  not the reciprocal of median pass duration. It is not display scanout timing.
- Each metric shows its own `n`. Missing GPU records/covered intervals stay
  missing, never zero. An absent conditional pass within an otherwise complete
  valid GPU record is zero; its denominator is all such measured frames. This
  makes occasional page rebuilds visible without averaging only rebuild frames.
- GPU-ring drops/errors are cumulative **within one renderer**. The JSON keeps
  per-round/reset epochs and their observed totals; it never adds repeated polls.
  Writer drops and CPU call gaps remain explicit. Final GPU samples may still be
  pending because shutdown does not wait for the timing ring. A missing footer is
  reported as incomplete evidence.
- Synchronous image readbacks or old profiling paths can change pacing. They are
  counted and can be excluded in a separate report; keep the unfiltered report
  to show their actual cost. Async profiling also costs timestamps, copies,
  polling and logging; it is not declared free.

Percentiles use linear interpolation (type 7). Forty frames can show a regression,
but provide weak p99 evidence. The timeline uses maximum per bucket above 900
frames so narrow spikes are not discarded. Different sizes/view counts produce
a mixed-configuration warning.

Legacy `--gpu-profile` JSONL is supported as **synchronous diagnostic data**.
Its clamped `frame_time_ms` is ignored; CPU, covered GPU interval and actual
cadence remain unavailable unless explicitly measured. The real r26 before/after
logs produced valid 40-sample reports: pass-sum medians 48.657 → 29.639 ms, with
page raster/cull removed. That observation alone does not establish gameplay FPS.

## Iteration history and cache evidence

```powershell
python tools/iteration-report.py --dev-root build/dev-runs --scenario-root C:/dev/perf/warp-sideways-20261024/r26-regression --out C:/dev/perf/iterations-001
python tools/test_iteration_report.py
powershell -NoProfile -File tools/test-play-dev.ps1
```

Only explicitly supplied roots are scanned. The new output directory contains
JSON and standalone HTML, with each run's build, startup, constructor, process,
analysis and capture costs kept separate. Configurations include actual GPU
resolution, scenario hash and renderer settings; executable hashes remain visible.
There are no averages across workflows or inferred cache speedups. Cache values
are recorded snapshots, not summed repeated counters; missing evidence stays
unknown. A developer manifest's first-visible-frame status is historical evidence,
not a claim that its process is still running. No background monitor is installed.

Scenario manifests also record `shader_provenance` before launch and
`shader_provenance_after` after exit. Only the renderer's 13 registered WGSL
override names are examined under `KERN_SHADER_DIR`, at most 2 MiB each. Each
records its exact byte SHA256 or an explicit embedded fallback identified by the
executable hash. Missing directories and partial overrides are recorded; unrelated
files and subdirectories are never scanned. Oversized, invalid UTF-8, unreadable,
or outside-directory symlink targets remain unverified, not assumed built-in.
The configuration grouping includes these content identities, so editing a shader
in the same directory separates the runs even when the executable is unchanged.
Changed/unverified inputs and older override runs without hashes are flagged and
kept in separate groups. These two snapshots cannot detect an edit reverted during
a run or certify which live-reload generation the renderer accepted.

`play-dev-windows.ps1` also enables startup profiling and saves structured records
in `startup.json` plus its iteration manifest. The full stderr log remains the
source for records emitted after the launch helper returned.

The first async scenario run (`scenario-noon-001`) joined 120/120 post-warmup GPU
samples without drops. It actually rendered **3840×2019**, so it validates the
profiler integration but not requested 3840×2160 performance. Its clean report
keeps 119 GPU/CPU frames after excluding one diagnostic capture; the cadence
interval following the earlier warmup capture is also excluded. Ordinary slow
frames remain. The final full-size PNG/readback alone cost about 2.28 seconds;
this cost is reported separately from steady-frame timing. A cache speedup needs
matched future cold/warm runs, not comparison with unrelated startup histories.

## Accepted runtime optimization: grass visibility prepass

The near-grass transition pass now resolves alpha-tested nearest depth before
running its unchanged lighting. `KERN_GRASS_DEPTH_PREPASS=0` selects the original
path for constructor-time A/B comparison; the default is enabled. The same
compacted owner lists, source cards, photo cutoff, wind, jitter and fade are used.
No density, distance, shadow or silhouette reduction is part of this change.

The r27 four-player ABBA run used actual 3840×2160 total output with DLSS Quality,
240 frames per run and 179 clean GPU samples after warmup/capture exclusion.
GPU-covered medians were 44.529/44.579 ms off versus 35.538/35.603 ms on; the
`grass copy + nearest` interval fell from about 9.885 to 1.02 ms. That interval
includes selection, depth copy, the new prepass and lighting, so its denominator
and scope remain comparable. These are GPU intervals, not a derived gameplay FPS.
The 8.33 ms target remains unmet.

All four camera/player traces and lighting matched. Off/off versus off/on image
mean absolute RGB differences were 0.032805 versus 0.032802 on a 0–255 scale,
with p99 one byte in both: the candidate difference was comparable to repeated
DLSS runs, not byte-identical rendering. No grass-coverage difference was visible.
Evidence: `C:/dev/perf/warp-sideways-20261024/r27-runtime/grass-{off-a,on-a,on-b,off-b}`
and `grass-image-comparison.json`. The existing depth/coverage/tie and shader
dependency tests remain applicable to both paths.

## Persistent Vulkan pipeline cache

Supported Vulkan adapters now share one wgpu pipeline cache across the renderer,
views and stage workers. `KERN_PIPELINE_CACHE=0` disables it;
`KERN_PIPELINE_CACHE_DIR` chooses a local cache directory. The default Windows
directory is `%LOCALAPPDATA%/Warp/Cache/pipelines-v1`. D3D12 and other unsupported
backends keep ordinary compilation and report `supported:false`. No native
driver-cache clearing is performed, including in an application-cache OFF run.

Only this application's `PipelineCache::get_data()` blobs are accepted, with an
exact adapter/driver/backend/OS/architecture/locked-wgpu key, length limit of
128 MiB and SHA-256 integrity check. wgpu additionally validates its own format
and driver UUID. Corrupt, missing or incompatible data cold-starts; cache creation
or publication errors do not stop rendering. Atomic replacement preserves the
previous complete file. The local directory is trusted application storage, not
an import interface for downloaded native driver binaries. Old compatibility
namespaces are not automatically deleted.

Every pipeline still supplies its current shader, entry point, specialization
constants and full state to wgpu; Vulkan matches those within the cache. Shader
edits/reloads therefore cannot deliberately select an old program merely because
the application cache file exists. The cache includes our opaque, grass, glass,
wind, particles, postprocess and motion pipelines. Third-party glyphon text
pipelines and NGX's internal DLSS compilation are outside this cache API.

`renderer_startup.pipeline_cache` reports cumulative device read/verify, create,
extract and write CPU times, bytes and descriptor requests. `blob_state:"loaded"`
means a validated application blob was supplied, **not** that a particular
pipeline hit the driver cache; `driver_cache_hits` remains null. Read/create occur
before `Renderer::new`. Its explicit `pipeline_cache_persist` section includes
the constructor save; a later upscale setup or successful shader reload can save
again. There is no per-frame disk work or added GPU wait. Measure whole startup
as well as compilation sections for matched OFF / empty-directory / warm-directory
runs of the same binary, configuration and driver.

The r27 same-executable four-player 3840×2160 DLSS Quality OFF / cold / warm
comparison passed. Renderer construction took 23,549.9 / 23,362.3 / 1,360.9 ms;
the broader “Preparing graphics” phase took 26,051.2 / 25,818.6 / 3,860.35 ms.
First scene appeared after 45,047 / 45,033 / 22,630 ms, including CPU scene
construction and the scenario's 6,000-tick replay. The warm run loaded 1,603,120
bytes, saved no changed data and reported no cache errors. Steady GPU-covered
medians were 35.326 / 35.251 / 35.285 ms: this improves startup, not frame cost.
All four player/camera traces matched; OFF/warm capture mean absolute RGB
difference was 0.032739 on a 0–255 scale, p99 one byte, maximum 33, consistent
with repeated-run reconstruction noise rather than bit identity. Evidence:
`C:/dev/perf/warp-sideways-20261024/r27-runtime/pipeline-cache-comparison.json`.
The v3 visibility diagnostic fixture also passed, including stale-view rejection.

## Controlled render-category omissions

`render-category-report.py` plans and compares an instrumented baseline against
one omitted color-geometry category at a time. It never launches a renderer:

```powershell
python tools/render-category-report.py plan ../games/sideways/scenarios/fourplayer-route.toml --exe C:/path/sideways.exe --env-file C:/path/base-env.json --out C:/path/new-category-matrix
```

The default plan writes seven serial commands: baseline, trees, grass, terrain,
cars, other, then a repeated baseline. Only `KERN_DIAGNOSTIC_HIDE` changes between
their environment files (`none` enables baseline bookkeeping). UTF-8 environment
JSON with or without a BOM is accepted. Run the generated commands with exclusive
GPU access, then run `comparison_argv` from `plan.json`, or use:

```powershell
python tools/render-category-report.py compare --baseline C:/path/matrix/baseline --baseline-repeat C:/path/matrix/baseline-repeat --without trees=C:/path/matrix/trees --without grass=C:/path/matrix/grass --out C:/path/new-comparison
```

The report requires matching executable, scenario, shader content, renderer
settings, actual resolution, rendered frame IDs, simulation ticks and captured
camera/light/car state. Newly instrumented profiles also compare the actual
per-frame `render_state_hash`. All capture counters must belong to the same
render sequence, frame and viewport, with complete input-pixel accounting.
Warmup and capture readbacks are excluded. GPU samples are paired by frame ID;
missing samples are reported, never substituted with zero. The repeated baseline
shows measurement variation separately from each omission. Raw manifests,
capture metadata, image hashes, source inventories and all view counters remain
in `report.json`; `report.html` shows the paired timing differences.

This measures the **marginal effect of omitting color geometry**, not additive
category costs or an acceptable gameplay optimization. Shadows, GI, physics and
effects remain present. Removing an occluder may reveal more geometry and make a
pass slower; negative deltas are retained. No FPS is inferred from GPU pass sums.

Source inventory counts distinguish allocated slots, active instances and all
referenced LOD clusters; none is a selected-GPU count. Per-view cluster/triangle
and depth-winning pixel counters cover shared visibility/raster only. Dedicated
transition grass, far-field grass and blended glass are outside those counters,
although their color passes are included in the omission. Thus dividing a grass
timing delta by shared grass triangles would misrepresent its scope. Pass timings
cover all views; these are not independent per-view GPU timings.

CPU-only tooling checks: `python tools/test_render_categories.py`. Actual GPU
category-matrix acceptance is a separate measured run, not established by these
synthetic manifest/consistency tests.
