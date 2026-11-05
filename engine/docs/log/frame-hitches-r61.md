# r61: visible foliage-page hitch investigation

Measured 2026-10-27. Six alternating visible runs reproduce a CPU wait on the
same two foliage-page update frames. Enabling asynchronous counter validation
removes those repeated 56–61 ms entry intervals while retaining similar GPU
work. Other occasional long intervals remain. The usual frame time barely
changes; this is evidence for a specific hitch fix, not a general FPS gain.

Evidence directory: `C:/dev/perf/warp-sideways-20261027/r61-hitch-investigation`.
`comparison.json` contains per-run and per-context statistics, all cadence
outliers, exact page-update events and all GPU-pass medians. `verification.json`
independently checks capture provenance and completeness; `visual-check.json`
compares all twelve JPEG captures, including repeated A/A and B/B controls.
Each numbered run
retains `frames.jsonl`, `hitches.json`, `run.json`, final state and JPEG captures.

All six runs use the same `Sideways-r61.exe`, SHA-256
`3ae3e74087829185873f13862a1d526aa3ec90d24ab0cecf52061bacb31f8a3c`.
The only recorded renderer-environment difference is
`KERN_ASYNC_SHADOW_VALIDATION=0` (A) versus `1` (B). The scenario hash is
`1d1fe84e3744696513912491dafd2550866c99e0c9b6890a85cd2e507ad272fd`.
Shader provenance is stable. All 3,600 rendered frame IDs and recorded
camera/lighting/car-state hashes match across all runs. Hash equality does not
establish pixel equality or identical unrecorded internal state.

The actual configuration is **one view, 3840×2160 output, DLSS Performance
from 1920×1080**, evaluated without fallback. The first 600 frames hold the
forest camera; the next 3,000 use the moving chase camera. Physics and the
driving simulation continue during the held-camera section: it is not a frozen
scene. Physics remains 120 Hz with four substeps and one scenario tick per
rendered frame. This experiment does not validate four-player performance or
the 8.33 ms target.

The first 120 frames of each section and capture frames 600/3600 are excluded.
Section transitions and intervals immediately after diagnostics are excluded;
both endpoints must be adjacent visible, focused, presented gameplay calls.
Every run has 3,358 selected frames and 3,356 cadence intervals: 478 held-camera
and 2,878 chase-camera intervals. All 3,600 rendered frames have matching GPU
samples. CPU gaps/errors, GPU profiler drops/errors, writer drops and final
profiler pending samples are zero.

Cadence is CPU frame-entry to frame-entry, not display scanout. The interval
ending at frame N contains the preceding CPU frame N−1. Delayed GPU samples
join by real `frame_id`, never their arrival call. CPU wall time includes waits;
GPU covered time spans the instrumented timestamp endpoints. These times are
not added, and GPU pass sums are not converted to FPS.

| Run | Cadence p50 ms | p99 ms | Maximum ms | Count >33.33 ms |
|---|---:|---:|---:|---:|
| 01-A | 16.203 | 24.438 | 64.818 | 5 |
| 02-B | 16.109 | 24.366 | 52.903 | 1 |
| 03-A | 16.053 | 23.384 | 58.748 | 2 |
| 04-B | 16.131 | 23.107 | 33.490 | 2 |
| 05-A | 16.058 | 22.549 | 59.234 | 5 |
| 06-B | 16.272 | 22.721 | 32.448 | 0 |

The following are medians of three run statistics, not pooled percentiles.
Counts are summed across the three runs in each variant.

| Selection | A/B p50 ms | A/B p99 ms | A/B median maximum ms | A/B count >33.33 ms |
|---|---:|---:|---:|---:|
| All selected intervals | 16.058 / 16.131 | 23.384 / 23.107 | 59.234 / 33.490 | 12 / 3 |
| Held camera | 17.150 / 17.236 | 21.298 / 19.749 | 25.432 / 23.702 | 0 / 0 |
| Chase camera | 15.785 / 15.815 | 23.479 / 23.274 | 59.234 / 33.490 | 12 / 3 |

There are 10,068 selected intervals per variant. Counts above twice each run's
overall median are A=13 and B=7; context-specific thresholds differ slightly.
The held-camera section has no steady foliage-page updates. The only selected
page-update frames are **1200 and 1234**, in every run. They correspond to the
cadence intervals ending at frames **1201 and 1235**.

| Updated frame | A/B following cadence ms | A/B CPU encode+submit wall ms | A/B GPU covered ms | A/B foliage cull ms | A/B foliage raster ms |
|---|---:|---:|---:|---:|---:|
| 1200 | 59.234 / 15.489 | 57.459 / 3.507 | 14.891 / 14.677 | 0.224192 / 0.225664 | 0.078112 / 0.078016 |
| 1234 | 56.918 / 14.386 | 55.314 / 3.628 | 14.241 / 14.288 | 0.221440 / 0.221952 | 0.008448 / 0.009024 |

These are medians of the same frame across three runs. All six A page-update
intervals exceed 33.33 ms; none of the six B intervals do. In
[`frame.rs`](../../crates/render/src/frame.rs), the original no-readback fast path
required an empty foliage-page update plan. Any page update therefore entered
the counter-readback path, submitted work, mapped counters and called
`gpu.wait()`. That synchronous wait is inside `render_encode_submit`, so it
appears in the next entry interval. The repeatable frame-aligned A/B change,
with similar page and total GPU timings, isolates this wait as the cause of
these two recurring stalls.

The enabled path copies page counters into
[`shadow_validation::State`](../../crates/render/src/shadow_validation.rs), with at
most four pending batches and eight pages per batch. It maps and polls without
waiting; saturation defers new page updates. Queue ordering renders pages
before their sampling, and submission permits provisional cache reuse.
Overflow, mapping, polling or callback errors remain persistent render errors.
Explicit metrics/captures keep synchronous validation and drain earlier
asynchronous batches after their existing wait. The legacy far-sun wait is
unchanged. The A/B switch was opt-in for these measurements.

Representative GPU medians below are medians of each run's selected-frame
median. Missing conditional passes count as zero only within a valid GPU
sample. Sparse foliage updates consequently have zero whole-run medians; the
event table above compares their actual work.

| GPU measurement | A ms | B ms |
|---|---:|---:|
| Covered interval | 15.748880 | 15.828800 |
| Shade | 3.554064 | 3.561328 |
| DLSS | 2.159872 | 2.160320 |
| Shared shadow raster | 0.824608 | 0.824800 |
| Tree remaining raster | 1.604864 | 1.603600 |
| Meadow aggregate render | 3.782736 | 3.872832 |
| Meadow resolve | 0.987648 | 1.024864 |

The median of the three run medians for CPU encode+submit is 3.411600 ms in A
and 3.411300 ms in B. Comparable GPU costs support comparable rendering work;
they do not prove raw image equivalence. Three serial runs per variant also
do not establish a precise population-wide hitch rate.

Unassigned intervals above 33.33 ms remain after preceding frames 1299, 1264
and 2647 in 01-A; 2783, 2864 and 2918 in 05-A; 944 in 02-B; and 777/779 in
04-B. They have no foliage-page work on the preceding frame. The longest B
interval is 52.903 ms after frame 944. These events require separate evidence
before attributing them to scheduling, presentation, GPU variation or another
cause. 06-B has no intervals above 33.33 ms.

Visual review inspected 01-A/02-B at frame 600 and frame 3600, and 05-A/06-B at
frame 3600. No obvious missing or shifting shadows were observed. Captures are
1600-pixel-wide quality-85 JPEG previews; these sampled checks do not establish
raw equivalence or inspect the exact update frames. Across the three A/B pairs,
mean absolute channel difference is 1.300–1.306 on the 0–255 scale at frame 600
and 1.471–1.473 at frame 3600. Same-mode comparisons span 1.303–1.310 and
1.464–1.479 respectively. A/B differences are comparable to repeat-run
variation in these previews. They do not establish motion or all-weather
parity.

A teardown limitation remains: the asynchronous validation state has no
dedicated shutdown flush. Renderer destruction can drop pending validation
before a later drain reports an error. These runs end with a diagnostic
capture, whose existing wait drains earlier batches, but the profiler's
zero pending count concerns its timestamp ring, not the separate shadow
validation queue. The captures therefore do not prove arbitrary early-exit
validation is complete.

Separately, [`gamenight.rs`](../../../games/sideways/app/src/gamenight.rs) now reports
disconnect operation, structural WebSocket/OS cause or close code, connection
age, sent/received counts and time since last received frame. It avoids peer
payloads, close-reason text, URLs and tokens, and ignores ordinary read
timeouts. These diagnostics help investigate the reported disconnect; the
standalone renderer A/B does not reproduce or establish its cause.

Reproduce reports from the repository root, with Python available:

```powershell
$evidence = 'C:/dev/perf/warp-sideways-20261027/r61-hitch-investigation'
python engine/tools/frame-hitches.py "$evidence/03-A/frames.jsonl" --out "$evidence/03-A/hitches.json" --force
python "$evidence/compare-runs.py"
python -m unittest discover -s engine/tools -p 'test_frame*.py'
```

`compare-runs.py` processes one capture at a time, with explicit file/record
limits, and writes `comparison.json`. All 27 report fixture tests passed.
The selector treats `diagnostic_sync` as authoritative: the producer emits a
tiny `diagnostic_wait` checkpoint even on ordinary visible frames, which must
not exclude them. Actual capture/readback work and its following interval
remain excluded.

For a fresh serial replay, retain the saved executable, scenario and environment
and choose a new output directory for every invocation:

```powershell
python engine/tools/run-frame-scenario.py "$evidence/route.toml" --exe "$evidence/Sideways-r61.exe" --env-file "$evidence/env-A.json" --out "$evidence/replay-01-A" --visible
```

Alternate A/B three times using `env-A.json`/`env-B.json`; keep GPU workloads
serial. The earlier r59 hidden synchronized capture contains zero eligible
visible gameplay intervals and is not evidence for gameplay cadence.
