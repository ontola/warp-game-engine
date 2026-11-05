# DLSS resolution sweep — r55

Completed no-code resolution experiment, 26 October 2026. No renderer setting or source change was promoted. The measured four-view GPU interval remains above 25 ms even in Performance mode. The subsequent [r56 grass experiment](meadow-band-r56.md) also remains above the decision gate; deeper renderer rewrites await the target/quality-tier discussion.

## Reproducible scope

- Immutable executable: `C:/dev/perf/warp-sideways-20261026/r54-pr-trials/sideways-trials.exe`. SHA-256: `028dd855a9fca56159ba4506c034916f23fd5d61f18858a23a06e37c3a4b01f7`.
- One 3840×2160 output, four independent moving cameras. Each reconstructed panel is 1916×1076. Actual DLSS inputs: Quality 1277×717, Balanced 1111×624, Performance 958×538. All modes evaluated; no fallback.
- Nine serial hidden runs in order Q/B/P/B/P/Q/P/Q/B. Each completed 480 frames, exit 0, with two 240-frame noon/clear sections of the same route. Physics stays 120 Hz, four substeps; one simulation tick per rendered frame.
- Exclude the first 60 frames of each section and capture frames. Each section has 179 clean samples per run, 537 per mode. No missing/dropped/error GPU samples or writer records.
- All 480 camera/light/car-state hashes and capture physical identities match across modes/repeats. Executable, runtime assets, scenario, renderer environment and saved baseline are pinned; only requested DLSS mode differs.
- Experimental opaque crowns, software raster, static sun cache, cache guard, wind freeze and cluster traversal remain off. Card crowns/fullness 1.25, the existing benchmark's visibility/hidden-ground paths and saved vegetation edits stay fixed. Meadow uses 120 blades, source radius 80 m and the existing 60–80 m handoff; aggregate outer range is unchanged. Full flags are in `environment.json`.

Tables report the median of three per-run medians. Named pass costs are summed **within each frame** before calculating a distribution. The GPU interval is the first-to-last measured timestamp span; pass medians do not add to it. The executable aggregates equal pass names across views, so these are four-view totals, not individually measured camera costs. CPU frame wall includes synchronization waits and must not be read as CPU execution or FPS.

## GPU interval and repeat spread

| Route section | Mode | Median ms | Range of three run medians ms |
|---|---|---:|---:|
| route-first | quality | 32.157 | 31.830–32.455 |
| route-first | balanced | 29.893 | 29.773–30.179 |
| route-first | performance | 27.669 | 27.445–27.770 |
| route-continued | quality | 40.346 | 40.310–40.386 |
| route-continued | balanced | 36.418 | 36.331–36.547 |
| route-continued | performance | 33.933 | 33.690–34.441 |

Balanced saves 7.0% / 9.7% relative to Quality in the two sections; Performance saves 14.0% / 15.9%. These are GPU interval reductions within this interleaved sweep, not frame-rate guarantees.

## Cost split

Each group below is also summed per frame before its median. **Total meadow uses the per-frame sum of its constituent pass samples**, not the sum of the near/far medians.

| Scope | First Q / B / P ms | Continued Q / B / P ms |
|---|---:|---:|
| total_meadow | 8.655 / 8.032 / 6.875 | 12.905 / 11.035 / 9.509 |
| near_meadow | 6.130 / 5.724 / 4.990 | 5.935 / 5.350 / 4.969 |
| far_meadow_and_composite | 2.452 / 2.114 / 1.722 | 6.974 / 5.668 / 4.498 |
| camera_geometry | 11.105 / 10.994 / 10.632 | 14.416 / 14.191 / 13.793 |
| sun_shadows | 1.266 / 1.262 / 1.280 | 1.739 / 1.740 / 1.755 |
| shade | 6.474 / 5.212 / 4.010 | 6.540 / 4.875 / 3.844 |
| dlss | 2.177 / 2.160 / 2.358 | 2.190 / 2.167 / 2.367 |

The pixel-sensitive work improves substantially: main shade falls from 6.474/6.540 to 4.010/3.844 ms. Tree-heavy camera geometry barely changes, from 11.105/14.416 to 10.632/13.793 ms. DLSS itself is slightly more expensive in Performance mode (2.358/2.367 ms versus Quality 2.177/2.190 ms). Lower input resolution does not remove the geometry bottleneck.

At capture 480, selected tree triangles total 87.08M / 88.08M / 87.34M for Q/B/P; source grass totals 9.82M / 9.71M / 9.59M. Pixel-error LOD and Hi-Z selections depend on input resolution, so this is a complete DLSS-mode comparison, not an isolated pixel-shader experiment. These counts are excluded diagnostic snapshots, not frame averages.

Total meadow repeat ranges: first Q 8.491–8.777, B 7.912–8.081, P 6.798–6.875 ms; continued Q 12.815–12.962, B 11.030–11.116, P 9.363–9.806 ms. It has not reached the planned under-4-ms budget.

## Individual pass medians

All values are four-view totals except genuinely shared passes. The JSON/CSV retains full precision and the min/max and three individual medians for every row. A zero median for an intermittent cached pass does not mean it never ran; the original report retains its full distribution.

| Pass | First Q / B / P ms | Continued Q / B / P ms |
|---|---:|---:|
| tree remaining raster | 8.044 / 7.930 / 7.633 | 10.475 / 10.231 / 9.994 |
| meadow aggregate render | 2.180 / 1.834 / 1.480 | 6.618 / 5.349 / 4.218 |
| shade | 6.474 / 5.212 / 4.010 | 6.540 / 4.875 / 3.844 |
| meadow visibility | 2.877 / 2.872 / 2.804 | 3.361 / 3.168 / 3.151 |
| tree remaining cull | 2.125 / 2.129 / 2.176 | 2.849 / 2.852 / 2.818 |
| dlss | 2.177 / 2.160 / 2.358 | 2.190 / 2.167 / 2.367 |
| meadow resolve | 2.743 / 2.524 / 1.886 | 2.073 / 1.652 / 1.360 |
| shadow raster | 0.844 / 0.839 / 0.854 | 0.868 / 0.861 / 0.879 |
| transparency | 0.555 / 0.470 / 0.391 | 0.657 / 0.545 / 0.471 |
| near shadow raster | 0.183 / 0.183 / 0.182 | 0.626 / 0.627 / 0.623 |
| meadow placement | 0.366 / 0.356 / 0.325 | 0.372 / 0.361 / 0.330 |
| tree occlusion pyramid | 0.307 / 0.298 / 0.264 | 0.307 / 0.298 / 0.264 |
| tree occluder raster | 0.191 / 0.175 / 0.166 | 0.287 / 0.262 / 0.243 |
| instance group select | 0.263 / 0.262 / 0.259 | 0.285 / 0.284 / 0.280 |
| meadow aggregate placement | 0.136 / 0.136 / 0.135 | 0.201 / 0.207 / 0.201 |
| bloom | 0.150 / 0.149 / 0.148 | 0.149 / 0.150 / 0.148 |
| motion vectors | 0.119 / 0.092 / 0.071 | 0.124 / 0.099 / 0.077 |
| tree occluder cull | 0.103 / 0.103 / 0.102 | 0.117 / 0.116 / 0.114 |
| exhaust | 0.115 / 0.114 / 0.105 | 0.115 / 0.114 / 0.105 |
| near shadow cull | 0.110 / 0.110 / 0.110 | 0.109 / 0.109 / 0.109 |
| meadow merge | 0.103 / 0.079 / 0.062 | 0.103 / 0.081 / 0.061 |
| tonemap | 0.086 / 0.086 / 0.086 | 0.086 / 0.086 / 0.086 |
| sun instance group select | 0.089 / 0.090 / 0.091 | 0.085 / 0.086 / 0.086 |
| shadow cull | 0.044 / 0.044 / 0.044 | 0.044 / 0.043 / 0.044 |
| plant wind | 0.036 / 0.035 / 0.038 | 0.036 / 0.036 / 0.036 |
| wheel particles | 0.035 / 0.034 / 0.034 | 0.031 / 0.032 / 0.032 |
| instance group refit | 0.021 / 0.021 / 0.021 | 0.021 / 0.021 / 0.021 |
| meadow composite | 0.021 / 0.018 / 0.015 | 0.021 / 0.018 / 0.016 |
| canopy volume | 0.000 / 0.000 / 0.000 | 0.000 / 0.000 / 0.000 |
| filter canopy | 0.000 / 0.000 / 0.000 | 0.000 / 0.000 / 0.000 |
| foliage page cull | 0.000 / 0.000 / 0.000 | 0.000 / 0.000 / 0.000 |
| foliage page raster | 0.000 / 0.000 / 0.000 | 0.000 / 0.000 / 0.000 |

## Historical baseline identity audit

The previous r54 `integrated-route-a` Quality run measured 43.097/54.158 ms; new `01-Q` measures 31.830/40.386 ms with the **same executable, scenario bytes, exact renderer environment, actual dimensions, all 480 render-state hashes and captured geometry/work counts**. All five capture metadata differences are limited to file path and CPU cook times. Final-state differences are file path and startup timing fields.

Renderer preparation dropped from 31.872 s to 1.793 s. Warmer pipeline/driver state is plausible, but no historical GPU clock/background-load telemetry establishes the cause of the steady GPU difference. The decrease spans tree raster (~28%), shade (27–28%), aggregate (25–28%) and DLSS (~23%). Do not attribute that between-session drop to an engine improvement; use the current interleaved repeats. The full recursive audit is in `baseline-identity-check.json`.

## Image review and remaining acceptance

Viewed Q/B/P contacts at frames 240 and 480 for views 0 and 3, plus both same-Quality repeat control contacts. Road/car/tree silhouettes and broad lighting remain consistent; no obvious missing geometry or gross new defect is visible at preview scale. Fine grass, foliage and road detail differ, with Performance appearing somewhat softer. Existing grass-wall/green-band artifacts remain in all modes.

These are 1600×900 JPEG previews downsampled from 4K, not native-resolution images or a motion review. They cannot establish final reconstruction quality, temporal stability or exact pixel parity. Quality-repeat mean absolute channel differences are 0.477–0.492/255; Q/B is 3.877–3.904 and Q/P 5.312–5.346. Same-mode noise is reported as a control, not used to waive differences. No quality-default promotion follows from these previews alone.

The separately controlled [r56 shorter source-grass handoff experiment](meadow-band-r56.md) preserves distant coverage. Its result triggers the [four-view budget plan](four-view-budget-plan-r55.md) decision gate: Performance plus the grass changes still exceeds roughly 25 ms, so discuss the target/quality tier before deeper rewrites. The 8.33-ms objective remains unmet.

## Evidence

Local root: `C:/dev/perf/warp-sideways-20261026/r55-resolution-grass/`.

- `plan.json`, `environment.json`, `Q.toml`, `B.toml`, `P.toml`: immutable input pins and order.
- `sweep-a/report.json`: all nine validated runs, clean frame IDs, actual dimensions, identities, hashes and full pass distributions.
- `sweep-analysis-b/summary.json`, `summary.md`, `passes.csv`: compact repeated medians and spread, including framewise total meadow.
- `sweep-analysis-b/visual-review.json`: reviewed contact hashes and limited visual conclusion.
- `sweep-analysis-b/frame-000240-view-{0,3}-Q-B-P.png`, `frame-000480-view-{0,3}-Q-B-P.png`: preview contact sheets.
- `sweep-analysis-b/control-frame-000240-view-0.png`, `control-frame-000480-view-3.png`: same-Quality repeat contacts.
- `baseline-identity-check.json`: r54 versus first r55 Quality audit.
- `run-resolution-sweep.py` runs the serial experiment; `summarize-resolution-sweep.py` is read-only analysis into a fresh output directory. Four harness CPU tests passed; this documentation added no runtime changes or GPU runs.
