# Four-view render budget: ordered experiments

Target: one 3840x2160 output, four players, DLSS, 120 FPS (8.33 ms).
The corrected r54 baseline measured a 48.80 ms mean covered GPU interval in
one ordered trial. That is a historical starting point, not a repeated median.

All candidates remain opt-in until visual and performance acceptance. Each
comparison uses one immutable executable, matching route/input/settings except
the named variable, at least three runs per mode in alternating order, per-pass
medians and image checks. Exclude warmup, capture/readback and failed frames.
Report run spread and actual internal/output dimensions. Host CI may compete.

## Ordered work

0. Park PR 4 as draft: opaque crowns regress about 46 ms in the held single-view
   case and must not be combined with paged shadows. Keep source and findings.
   PR 6 remains opt-in: raw visibility/depth localization and repeated route
   measurements are required before promotion. Land corrected PR 5 opt-in only;
   its small gain includes a 57% increase in shared shadow texel size.
1. No-code DLSS Quality/Balanced/Performance sweep, three runs per mode using the
   r54 immutable executable. Determine pixel-sensitive versus geometry/shared
   work empirically. Existing JSON sums equal pass names across views: do not
   present a divided total as an individually measured camera cost.
2. Meadow: measure individual views and placement, visibility, aggregate and
   resolve. Share genuinely world-space work where safe; aggregate projection,
   depth and lighting remain view-dependent. Trial a 40-50 m source handoff
   against the current 60-80 m band, preserving the distant range. Target under
   4 ms meadow total across four views. Inspect moving transition bands.

**Decision gate after 1-2:** if Performance DLSS plus grass changes remain above
approximately 25 ms, discuss a 60 FPS/four-player target or reduced split-screen
quality tier before deeper renderer rewrites. Do not silently change the target.

3. Shade: per-tile cost breakdown first, then classify cheaper distant material
   paths, reduce distant shadow taps or share lower-resolution indirect/sky
   visibility if evidence supports it. Target under 4 ms.
4. Trees: tight alpha-hull card polygons and an additional conservative tree
   occlusion phase, avoiding the PR 4 needle geometry explosion. Target under
   5 ms. Coverage and appearance must be checked rather than assumed identical.
5. Audit every four-times pass (cull, pyramid, meadow, DLSS), move only truly
   view-independent work to once per frame.

## Evidence

Prior results: [r54 PR trials](pr-foliage-trials-2026-10-26.md).
New local evidence: `C:/dev/perf/warp-sideways-20261026/r55-resolution-grass/`.
Implementation and measured status are appended as each gate completes.

Bookkeeping completed: PR 4 is draft. Corrected PR 5 merged separately to main
as `8c62e47ec5367c419587d5fe0adf5891fb113352`, after 509 renderer and 183 CLI tests
passed (one ignored in each). Its remote head was `34a2c73`; neither PR 4 nor
PR 6 was merged. PR 6 remains opt-in on the trial branch, with raw visibility
and depth parity explicitly unaccepted. The resolution sweep keeps all three
experimental modes off and uses the pre-existing r54 executable unchanged.

## Decision gate reached: steps 1-2 measured

The [r55 resolution sweep](resolution-sweep-r55.md) completed nine runs, three
per mode. Quality measured 32.157 / 40.346 ms in the two route sections,
Balanced 29.893 / 36.418 ms, and Performance 27.669 / 33.933 ms. The historical
48.80 ms baseline cannot be treated as a same-session speedup: identical-input
Quality measurements shifted between sessions without GPU-clock/load evidence.

The [r56 grass experiment](meadow-band-r56.md) completed six Performance runs
with one new immutable executable: A/B/A/B/A/B. The 40-50 m band changes total
GPU interval from 27.604 / 34.126 to 26.947 / 33.351 ms. Root-only grass totals
change from 6.865 / 9.475 to 6.500 / 8.950 ms. Neither the approximately 25 ms
decision gate nor the under-4-ms grass budget is met. Defaults remain unchanged.

The engine now reports true per-view GPU scopes and aggregate depth, material
and illumination children without double counting them. Atlas/terrain/source
resources are already shared; projection, depth, coverage and lighting are
view-dependent. Moving remaining world-space placement into a shared pool has
not been implemented: its measured aggregate-placement cost is about 0.2 ms,
not the multi-millisecond raster bottleneck. This is an explicit deferred item.

Preview image checks found no gross missing coverage, but differences remain
and moving transition/native-resolution acceptance is still pending. There is
no basis for promoting the shorter band or Performance DLSS as defaults.

Steps 3-4 and the remaining pass-sharing implementation in step 5 have **not**
started. Discuss a reduced four-player quality tier while retaining the 120 FPS
goal, or a 60 FPS milestone (16.67 ms, also not yet reached), before deeper
renderer work. The current 120 FPS target has not been changed.
