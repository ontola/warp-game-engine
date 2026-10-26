# Meadow handoff band — r56

The shorter source-grass handoff saves about 0.66/0.77 ms of measured GPU interval, but much of the source-geometry saving becomes closer, more expensive aggregate rendering. With DLSS Performance the two route sections still measure **26.947/33.351 ms**, and total meadow remains **6.500/8.950 ms**. The planned 25-ms decision gate is not met; neither is the under-4-ms meadow budget. This experiment does not promote the shorter band or change the 8.33-ms target.

## Controlled comparison

- One immutable executable: `C:/dev/perf/warp-sideways-20261026/r56-meadow-band/sideways-r56.exe`; SHA-256 `4878f5971e4b41b385e3ae9dc337fd3be9d0eac42ee118fb89592c8cdcbe2be8`, source `9329d2fd36020c1d3e8a96f4a73317f7aaf598b9`.
- Six serial runs in A/B/A/B/A/B order, three repeats each. All completed 480 frames with exit 0. A uses the existing 60–80 m source/aggregate transition; B uses `SIDEWAYS_MEADOW_SOURCE_BAND=40,50`. Actual metadata verifies these values in all four views at every capture. Both retain the 160–180 m outer fade.
- Same 3840×2160 output and four independent moving cameras. Actual DLSS Performance inputs are 958×538 per panel, reconstructed to 1916×1076. No fallback. Same noon/clear two-section route, 120 Hz/four-substep physics, assets, saved baseline, remaining environment and all 480 camera/light/car-state hashes.
- Each 240-frame section excludes its first 60 warmup frames and explicit capture frames: 179 samples per run/section, 537 per variant/section. GPU ring errors/drops and invalid meadow IDs are zero. Pinning covers executable, harness/imported analyzer, runner/budget tool, scenario, environment, assets and baseline; pins are rechecked after runs.
- Frozen build evidence records 514 renderer tests and 185 CLI tests passed, one ignored in each suite, and a 95-second build. The analysis added no runtime edits or GPU reruns. Its four focused CPU tests passed.

The standard `budget` report selects zero samples intentionally: the runner invokes `--exclude-diagnostics`, while every hidden rendered call reports `diagnostic_sync=true`. This is a policy mismatch for this controlled experiment, not missing/invalid GPU data. All six runs contain all 480 valid GPU records. The independent analyzer explicitly accepts hidden synchronized timing, then excludes warmup and capture/readback frames; `legacy-report-policy.json` records the check. CPU synchronization wait/cadence is not CPU execution or gameplay FPS.

## Medians and repeat spread

Tables use **median of three per-run medians**, with min–max of those three medians. Group costs are formed by summing the relevant scopes inside each frame **before** computing a median. Sums of medians are not used. Total meadow includes root scopes only; aggregate children are attribution and are not added again. Interval union is independently checked against raw timestamps. Root meadow sum and union agree for the recorded samples, but neither is a whole-frame utilization metric.

| Section | Measurement | A median [range] ms | B median [range] ms |
|---|---|---:|---:|
| route-first | GPU interval | 27.604 [27.413–27.789] | 26.947 [26.799–27.058] |
| route-first | Total meadow, root only | 6.865 [6.849–6.908] | 6.500 [6.400–6.591] |
| route-first | Far aggregate parent envelope | 1.496 [1.490–1.497] | 2.611 [2.610–2.744] |
| route-first | Far aggregate child sum, attribution | 1.431 [1.429–1.433] | 2.417 [2.410–2.428] |
| route-continued | GPU interval | 34.126 [34.062–34.175] | 33.351 [33.260–33.354] |
| route-continued | Total meadow, root only | 9.475 [9.303–9.492] | 8.950 [8.915–9.034] |
| route-continued | Far aggregate parent envelope | 4.179 [4.090–4.276] | 5.566 [5.529–5.572] |
| route-continued | Far aggregate child sum, attribution | 3.954 [3.929–4.071] | 5.271 [5.142–5.345] |

GPU interval improves 2.38% / 2.27%; meadow improves 5.32% / 5.54%. These modest gains are observed in each ordered comparison, but this is three repeats under uncontrolled host/GPU background conditions, not a confidence-bound or FPS claim.

## Where the work moves

| Scope, summed across four views per frame | First A → B ms | Continued A → B ms |
|---|---:|---:|
| Source visibility geometry | 2.819 → 1.266 | 3.140 → 1.332 |
| Source shading resolve | 1.886 → 1.882 | 1.370 → 1.384 |
| Source + aggregate placement | 0.498 → 0.457 | 0.549 → 0.524 |
| Aggregate depth child | 0.466 → 0.850 | 1.402 → 1.866 |
| Aggregate material child | 0.658 → 1.226 | 2.031 → 2.761 |
| Aggregate illumination child | 0.306 → 0.333 | 0.523 → 0.608 |

Source visibility falls from 2.819/3.140 to 1.266/1.332 ms, while source resolve is almost unchanged. Aggregate depth and material work increase substantially as that representation starts closer and covers larger projected regions. Its parent envelope also includes inter-pass gaps; summing only child durations does not reproduce the full envelope.

At excluded capture 240, source triangles fall from 9,045,000 to 3,540,000 across four views; far roots rise from 70,877 to 85,761. At capture 480, source triangles fall from 9,586,400 to 3,677,200, while far roots rise from 196,952 to 212,929. This is roughly 61% fewer submitted source triangles, not 61% less total meadow time. These are diagnostic snapshots rather than route averages.

## Actual per-view measurements

View IDs come from tags captured when commands are encoded, not a total divided by four. A conditional scope absent from a frame contributes zero to the same 179-sample distribution. The following entries are medians of three run medians; exact spread for every per-view parent/child is retained in `analysis-a/passes.csv` and `summary.json`.

| View | Total meadow first A → B ms | Total meadow continued A → B ms |
|---|---:|---:|
| 0 | 1.737 → 1.600 | 2.500 → 2.383 |
| 1 | 1.683 → 1.578 | 2.289 → 2.173 |
| 2 | 1.741 → 1.639 | 2.380 → 2.225 |
| 3 | 1.649 → 1.563 | 2.252 → 2.118 |

### route-first: parent passes and child attribution

| Scope | View 0 A → B | View 1 A → B | View 2 A → B | View 3 A → B |
|---|---:|---:|---:|---:|
| Placement | 0.083 → 0.079 | 0.082 → 0.077 | 0.082 → 0.077 | 0.082 → 0.078 |
| Source visibility | 0.700 → 0.319 | 0.661 → 0.315 | 0.656 → 0.315 | 0.648 → 0.314 |
| Source resolve | 0.428 → 0.424 | 0.453 → 0.453 | 0.538 → 0.529 | 0.474 → 0.472 |
| Far placement | 0.034 → 0.036 | 0.034 → 0.035 | 0.034 → 0.035 | 0.034 → 0.035 |
| Far parent envelope | 0.378 → 0.724 | 0.369 → 0.615 | 0.375 → 0.620 | 0.366 → 0.613 |
| ↳ depth child | 0.118 → 0.217 | 0.116 → 0.210 | 0.118 → 0.213 | 0.114 → 0.210 |
| ↳ material child | 0.166 → 0.312 | 0.164 → 0.302 | 0.166 → 0.306 | 0.161 → 0.302 |
| ↳ illumination child | 0.077 → 0.083 | 0.076 → 0.082 | 0.077 → 0.084 | 0.075 → 0.084 |
| Merge | 0.015 → 0.015 | 0.015 → 0.015 | 0.015 → 0.016 | 0.015 → 0.016 |
| Composite | 0.004 → 0.004 | 0.004 → 0.004 | 0.004 → 0.004 | 0.004 → 0.004 |

### route-continued: parent passes and child attribution

| Scope | View 0 A → B | View 1 A → B | View 2 A → B | View 3 A → B |
|---|---:|---:|---:|---:|
| Placement | 0.084 → 0.078 | 0.083 → 0.078 | 0.082 → 0.078 | 0.083 → 0.078 |
| Source visibility | 0.789 → 0.333 | 0.777 → 0.335 | 0.786 → 0.333 | 0.778 → 0.334 |
| Source resolve | 0.329 → 0.329 | 0.335 → 0.330 | 0.394 → 0.408 | 0.343 → 0.342 |
| Far placement | 0.057 → 0.058 | 0.051 → 0.052 | 0.048 → 0.049 | 0.049 → 0.049 |
| Far parent envelope | 1.181 → 1.562 | 1.000 → 1.347 | 1.014 → 1.376 | 0.968 → 1.283 |
| ↳ depth child | 0.383 → 0.506 | 0.346 → 0.464 | 0.342 → 0.451 | 0.329 → 0.443 |
| ↳ material child | 0.550 → 0.749 | 0.502 → 0.691 | 0.492 → 0.670 | 0.473 → 0.655 |
| ↳ illumination child | 0.155 → 0.165 | 0.126 → 0.151 | 0.126 → 0.150 | 0.115 → 0.136 |
| Merge | 0.015 → 0.015 | 0.015 → 0.015 | 0.015 → 0.015 | 0.015 → 0.015 |
| Composite | 0.004 → 0.004 | 0.004 → 0.004 | 0.004 → 0.004 | 0.004 → 0.004 |

## Existing sharing versus a future source pool

`MeadowRenderer` already owns one terrain/ground buffer, source recipe, index buffers, pipelines, wind clock and shared aggregate atlas. The geometry/atlas is not cooked or copied four times. `ViewResources` separately owns compact root pools, indirect arguments, visibility/Hi-Z and rendering targets; each aggregate view has its own view-dependent pool and targets.

A shared world-root generation/deformation pool followed by per-view selection is a possible engine change, **not implemented by this experiment**. Frustum/Hi-Z tests, LOD selection, projection, depth, material accumulation and illumination still depend on the camera. The recorded source-plus-aggregate placement scopes total only about 0.46–0.55 ms across four views. Even eliminating those measured scopes entirely would address only that small part of the interval before replacement cost/interactions; it cannot explain a multi-millisecond meadow win. Current larger costs are source visibility and far aggregate raster/material work.

## Image review

Viewed six A/B/A-repeat contacts: frames 60, 240 and 480, views 0 and 3. The road, cars and tree silhouettes remain consistent without an obvious broad grass hole at preview scale. Mid-distance grass texture/shade changes along the verges and slopes; the aggregate representation is brought closer. Existing tall grass walls and green verge bands remain.

The images are 1600×900 JPEG previews of 4K output, sampled sparsely in time. They do not isolate an exact world-space distance mask and cannot certify full-resolution sharpness or a smooth moving handoff. Mean absolute channel difference is 1.603–1.753/255 for A/B and 1.545–1.691 for the same-A control; maximum differences reach 156 for A/B versus 45 for the control. The control is reported honestly and is not a tolerance that establishes parity. No optical/default acceptance is inferred.

## Decision gate

After the resolution and band experiments, DLSS Performance plus the shorter band still exceeds roughly 25 ms in **both** sections, including the best run medians. The [ordered plan](four-view-budget-plan-r55.md) therefore calls for discussing the four-player target or an explicit reduced quality tier before deeper rewrites. The target is not silently changed: 8.33 ms remains unmet, and this evidence does not establish 60 FPS either. The shorter band remains an opt-in tradeoff, not a solution to the structural budget gap.

## Evidence

Root: `C:/dev/perf/warp-sideways-20261026/r56-meadow-band/`.

- `exe-manifest.json`, `band-a/plan.json`, `band-a/report.json`: immutable build, source/input pins, six completed runs and clean frame IDs.
- `analysis-a/summary.json`, `summary.md`, `passes.csv`: root/child/view separation, repeated medians and spread; `placement-and-source-cost.json` adds the independently frame-summed source/placement scopes.
- `images-a/frame-000{060,240,480}-view-{0,3}-A-B-A.png`: matching preview contacts. `images-a/visual-review.json` records reviewed hashes and limitations.
- `harness-audit.json`: initial read-only audit and corrected zero-fill/pinning issues. `legacy-report-policy.json`: explains the standard report’s zero selected samples.
- `summarize-band.py`, `review-band-images.py`, `test_band_summary.py`: CPU-only analysis and four tests; no GPU reruns or changes to the frozen runner.

Resolution-only context: [r55 sweep](resolution-sweep-r55.md). Historical r54 timing is not used as the A/B baseline here.
