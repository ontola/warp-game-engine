# r64 — distant grass shading and basal edge contact

28 October 2026. Two explicit experiments address the user's grainy distant
grass and weak contact at its ground edge. Both operate on the current
[r63 windswept source](meadow-windswept-r63.md). They do not implement the
proposed terrain-following far representation, remove far source triangles,
or establish final visual acceptance.

## Controls and preserved source

| Control | Effect | Default |
| --- | --- | --- |
| `KERN_MEADOW_SPATIAL_FILTER=1` | Native, depth-aware filtering of the rendered grass RGB before composite | Off |
| `KERN_MEADOW_EDGE_SHADOW=1` | Source-accepted basal contact field for terrain/road sky and sun light | Off |

The filter requires homogeneous authored-height blades with no aggregate,
TAA, DLSS or meadow motion output. Orthographic pixels pass through unchanged.
The edge contact model is separate from the older `KERN_MEADOW_CONTACT_AO`
experiment; the comparisons keep that older model off. Requested flags and
actual constructed/per-view settings are recorded separately.

Placement, density, curved blades, per-fragment pigment/AO, source LOD morph,
250 m source range and compact pool remain as in r63e. Current travelling wind
and per-frame lighting still render the source before filtering. No frozen
grass photograph, generic green ground layer or temporal history is introduced.

The live comparison uses grass size **1.6**, density **1.0** and RGB multipliers
**1.3/0.9/1.3**. The saved baseline remains size 2.0/density 0.55. The new
`-DebugUiScript` developer-launcher argument stages and hashes a bounded JSON
event script; `games/sideways/scenarios/windswept-grass-edge-settings.json`
reproduces the session values without editing the saved baseline. A preview
must use that script and verify the actual captured values, not assume that
the stored baseline matches the user's current view.

## Native spatial RGB filter

The final source composite conditionally reads a 3×3 binomial neighbourhood.
It averages premultiplied RGB with actual grass coverage as the weight, then
restores the centre pixel's original alpha exactly. Empty pixels, near pixels
and rejected depth discontinuities keep their original values. Source and
opaque depth prevent invalid foreground taps from contributing. Filter strength
never exceeds 0.65. There is no extra image target or history buffer; its GPU
cost is included in the existing `meadow composite` interval.

Preserving alpha is deliberate. Averaging coverage only where the centre has
grass would erode thin blades; filling empty centres would spread coverage
onto roads and silhouettes. This experiment instead reduces shading variance
while retaining the original silhouette and valid opaque-shading skip mask.
It cannot recover blades missed by single-sample rasterization, fix silhouette
aliasing, or promise temporal stability from static captures.

The threshold follows actual cooked basal width `W` times live growth. The
far source is one triangle with width `W(1−t)`, whose average over height is
`W/2`. If `f` is focal length in pixels and `z` the view-space depth, the
world footprint is `z/f`. Filtering starts when that footprint exceeds
`W/2` and reaches its maximum at `W`. The initial probe used basal width
`W`/`2W`; it began too far away and is retained only as comparison evidence.
The metadata field `spatial_filter_blade_width_m` reports basal width, not
the halved onset. `spatial_filter_enabled` describes the encoded composite,
not a measured count of modified pixels.

## Accepted-root edge contact

A GPU producer reruns the actual meadow habitat, live density, growth and
per-blade road-anchor decisions on a 151×151 world-cell grid around each view.
It retains off-screen eligible roots and ignores source frustum/Hi-Z rejection.
Each accepted record holds a terrain-conforming basal centroid and bounded
mass derived from source area. Empty habitat, rejected road anchors and zero
density produce no contact. Four independent view blocks occupy **1,459,520
bytes total**, including their headers.

Terrain and road receivers use one shared 5×5 scan, **25 records for both**
sky and sun queries. The final radius is the smaller of anchor radius +0.18 m,
0.55 m, and the certified neighbour-support bound; the measured current source
is 0.4941 m. The sun query follows the current sun direction, with a displacement
capped at 0.35 m. Contact fades over 24–32 m and respects the source's outer
fade. Receiver normal/height checks prevent raised decks from borrowing ground
contact; view-block bounds and camera checks reject stale or unrelated data.

Maximum sky and sun strengths are 0.24 and 0.48 respectively. Only diffuse
skylight and incident sun are attenuated; headlights, bounce, environment
reflection, emission and fog keep their existing paths. This is a local basal
approximation, not traced blade silhouettes or a blade shadow map. Basal roots
are stationary under wind, so it does not add full-height moving grass shadows.
The source's conservative road clearance also limits how far this contact can
reach toward asphalt.

The earlier probe used a smaller radius and two nine-record queries. Its
approximately 0.022 ms producer timing is not a measurement of the final
receiver kernel. Receiver cost is inside opaque `shade`, separately from the
`meadow edge occupancy` producer interval.

## Matched preliminary measurements

Evidence root:
`C:/dev/perf/warp-sideways-20261028/r64-grass-edge-grain/`.

The scenario `games/sideways/scenarios/windswept-grass-edge.toml` renders one
native 3840×2160 view on the RTX 5070 Ti with one raster sample, TAA/DLSS off,
fixed wind strength 0.8/time 128.0692108 s and no simulation advance. It uses
the exact user F12 edge camera at noon and morning, plus the earlier hillside
camera. Each section has 24 warmup frames and 72 intended measured frames.
Final PNGs are lossless; delayed tail samples leave 72/72/69 matched timing
samples. The reported covered GPU interval excludes CPU, HUD/presentation and
work outside its instrumented endpoints.

The first r64 executable with both flags off reproduces all three r63e PNGs
pixel-exactly: `baseline-vs-probe-off/comparison.json`. This proves those
captured frames, not all cameras, environments or temporal behaviour.

| Native scene | OFF GPU ms | W/2 filter-only GPU ms | OFF composite ms | Filter composite ms |
| --- | ---: | ---: | ---: | ---: |
| User edge, noon | 22.928 | 23.020 | 0.042 | 0.077 |
| Far hillside, noon | 25.965 | 26.060 | 0.046 | 0.105 |
| User edge, morning | 25.079 | 25.161 | 0.044 | 0.078 |

These are medians from `off-vs-filter-only-v2/comparison.json`, one matched
pair, not repeated-run significance or a final combined cost. OFF uses frozen
`sideways-r64-probe.exe` SHA256
`03f8287e4366cee721c5050d3b88e25570e7021e8784d263c6cb69ed47bc74e0`;
filter-only uses `sideways-r64-filter.exe` SHA256
`eec5463b6b9141138d0d740ff31ab22cef97eae0811800a20b491d17659feffc`.

The near-edge and road-control crops remain pixel-exact with the filter alone.
The initially selected right-hillside crop changes negligibly. A farther
interior crop, added after inspecting the candidate, measures high-pass
display-luma variance **1911.524 → 451.691** (−76.4%), while mean display luma
increases **112.930 → 123.054** (+9.0%). Its rectangle is
`[2130,630,2390,690]` in frame 120. This is an explicitly exploratory ROI,
not a preselected acceptance target. It shows reduced spatial contrast and a
brightness shift; it does not prove unbiased radiance, correct coverage,
improved motion or an accepted appearance. Do not describe the statistic as
a 76% reduction in temporal noise.

The initial combined probe's smaller contact kernel was subtle: near-edge
mean display luma changed 91.918→91.698 at noon and 99.374→99.299 in the
morning; road controls were unchanged. See `probe-off-vs-combined/comparison.json`.
Its producer measured 0.0222/0.0226/0.0221 ms. This motivated the bounded
anchor-extent radius and shared 25-record receiver scan now in code.

## Verification

- Workspace check passed. The first renderer suite passed 555 tests with
  seven ignored; after the transition fixture, 556 passed with eight ignored.
- CLI passed 190 tests with one ignored. Developer-launcher identity,
  artifact, quoting, debug-UI forwarding and syntax contracts passed.
- Full production filter WGSL and the contact shader pass Naga validation.
  CPU tests cover source width, coarse-triangle mean, live growth, near
  identity, constant HDR colour, unchanged alpha and depth discontinuities.
- The explicit filter GPU oracle passed **2312 pixels / eight fixtures**,
  with exact alpha and maximum RGB error **1.1920929e−7** against the f64
  oracle. Fixtures cover the W/2 transition, live size 1.6, foreground and
  empty pixels. Depth values use R32Float fixture textures; production
  depth-texture binding types are checked by full shader validation.
- The accepted-contact producer GPU oracle passed empty habitat, roads,
  zero density, off-screen acceptance and view isolation. Updated CPU bound
  tests and the final explicit GPU rerun after the radius/receiver-kernel
  change passed; see `contact-tests-final.log` and `contact-gpu-final.log`.
  The producer oracle does not establish receiver visual quality.

GPU fixtures run serially without a live game. No four-player/120 FPS,
moving-camera, weather-wide or physical-controller acceptance follows from
these tests. The 250 m source geometry cost remains and the distant
terrain-following representation is still unimplemented.

## Final combined capture

The frozen final `sideways-r64-native.exe` combines W/2 filtering with the
enlarged, bounded 25-record contact receiver. SHA256:
`afc89e31a0e16af50dd2a62450aa7371ff914264310a63f272b653b082a607d0`.
`final-combined-static` completed 288/288 frames and three native 4K PNGs.
Selected camera, size/density/colour, source geometry and measured visibility
coverage match OFF. Actual metadata confirms both experiments enabled, basal
width 0.064 m, contact radius 0.4941 m and the full 1,459,520-byte contact field.

| Native scene | Final GPU ms | All meadow pass sum ms | Contact producer ms | Composite ms |
| --- | ---: | ---: | ---: | ---: |
| User edge, noon | 23.311 | 7.279 | 0.0221 | 0.0766 |
| Far hillside, noon | 26.326 | 10.033 | 0.0224 | 0.1051 |
| User edge, morning | 25.410 | 7.853 | 0.0221 | 0.0805 |

These medians use the same 72/72/69 matched samples as OFF. Total covered GPU
differences are +0.384/+0.361/+0.331 ms in this pair. The all-meadow sum is
computed per frame before taking the median; it includes occupancy but not
the receiver work inside opaque `shade`. Pass medians are not additive.
See `off-vs-final-combined/comparison.json` and
`filter-only-vs-final-combined/comparison.json`. This is not a repeated-run
confidence estimate or evidence of a speedup.

The enlarged contact remains visually subtle. Near-edge mean display luma
falls by about 0.628/255 at noon and 0.185/255 in the morning; road-control
crops remain exact. The exploratory distant crop is identical to filter-only,
retaining both its contrast reduction and brightness shift. The final result
therefore does not establish that the user considers the grass edge strong
enough or the filtered distant appearance acceptable.

`final-wind-poses` completed 72/72 frames with six native 4K PNGs at simulation
wind times 0.3667, 0.7667, 1.1667, 1.5667, 1.9667 and 2.3667 seconds. The crops
show changing blade bend; all recorded numeric metadata is finite and
visibility invalid IDs remain zero. Read `final-wind-poses/wind-validation.json`.
Six selected poses are not a dense animation, moving-camera or temporal
stability proof.

## Visible playtest

The final contact GPU rerun passed. The final preview reached a confirmed
visible first frame with **PID 23356**, launched at 16:56 UTC from
`games/sideways/build/dev-runs/20261028T165617934Z-5eab5a82/iteration.json`
with the frozen final executable, both flags enabled, one native 4K view,
wind strength 0.8 and advancing wind time. Its copied/hashed debug script
requests the same size 1.6/density 1.0 settings verified in the static scenarios.
No live F12 was captured, so those controls were not independently verified
in the interactive session. Its executable path is inside the matching run
directory and its hash matches the final AFC89E… artifact. The owner stopped
this playtest at the user's request on 28 October; PID 23356 is historical.
Comparison images are `final-far-comparison.png` and
`final-edge-comparison.png` in the evidence root. Both features remain opt-in; user acceptance,
four-view cost and the proposed far representation remain open.
