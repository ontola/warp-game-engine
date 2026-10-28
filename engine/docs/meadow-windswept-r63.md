# r63 — curved grass, tip colour and travelling wind

28 October 2026. This replaces the subtle r62 shading experiment with a new
`windswept` source profile. The implementation follows specific geometry,
colour and wind equations from SimonDev's demo. Evidence distinguishes the
failed aggregate attempt, r63b/r63c source rendering, the validated r63d
material/LOD correction and the interim r63e distance comparison.

**r63d validation is complete for the recorded cases.** Pigment and basal AO
are evaluated at the fragment's blade coordinate; six-segment geometry smoothly
morphs into the actual one-triangle cut. There are 547 passing renderer tests
(five ignored), four passing explicit GPU oracles and a completed 61-frame
approach sweep. At one-view native 3840×2160 on RTX 5070 Ti, normal/close/overview
fixed cameras measure median covered GPU intervals of 23.310/14.321/20.514 ms
and meadow pass sums of 6.511/2.620/3.527 ms, from 72/72/69 steady samples.
The interval excludes CPU and work outside its GPU timestamps, including
HUD/presentation. Evidence is in `game-r63d-visible/` under the root below.
The user subsequently reported the separate 100 m disappearance and noisy
appearance. Their r63d freeplay closed normally; no game is currently active.

**Current r63e code is an interim source-range extension.** It defaults to
250 m and adds a compact pool for homogeneous windswept profiles. The renderer
suite passes 550 tests (six ignored), 190 CLI tests pass (one ignored), and the
explicit compact-pool GPU sentinel and developer-launcher contracts pass.
The release build and final matched native-4K hillside capture completed;
the comparison below separates the range probe from the compact version.
This source-only extension does not implement the proposed far illusion.

## Selection and runtime

With `SIDEWAYS_PROCEDURAL_GRASS=field`, an unset `SIDEWAYS_MEADOW_PROFILE` now
selects `windswept`; `SIDEWAYS_MEADOW_PROFILE=classic` explicitly selects the
previous recipe. The classic cook and optional r62 `flat|rounded` shading
remain available. Windswept always uses its own curved shading. A missing
profile in historical capture metadata means classic for replay compatibility.
Without `SIDEWAYS_PROCEDURAL_GRASS=field`, the legacy photographed-grass path
still applies. These are game choices, not new Warp-wide defaults.

The game supplies a terrain field and three immutable cooked variants. GPU
placement retains deterministic world roots, habitat/road checks, visibility
selection and live growth/density controls. Budget values `120/180/360` select
**40/60/120 blades per patch** for this profile, with no compensating width
multiplier; windswept defaults to budget 120. Forty blades per 0.45 m square
give about 198 blades/m² before habitat and density edits. The saved 55% patch
setting reduces the nominal density to about 109 blades/m²; it is not a count
of visible blades in a captured frame.

The captured r63b–r63d builds used real source geometry through 100 m with an
80–100 m fade. Current r63e code defaults to **250 m with a 200–250 m fade**,
measured horizontally from camera to root before live distance edits.
`SIDEWAYS_MEADOW_WINDSWEPT_RADIUS` accepts 60–256 m and sets fade start to 80%
of that radius; inherited `SIDEWAYS_MEADOW_SOURCE_RADIUS` and
`SIDEWAYS_MEADOW_SOURCE_BAND` are ignored. The completed comparison below is
an interim source baseline, not acceptance of this as the distant representation.
The homogeneous windswept profile-2 pool uses 12 plant words plus three owner
words per candidate: **75,129,660 bytes/view (71.65 MiB)** at 250 m, versus
155,267,964 bytes (148.08 MiB) before compaction. Classic and mixed profiles
retain their 28-word plant layout. These are pool bytes, not total VRAM. The renderer
**forces the aggregate off** for windswept even when
`KERN_MEADOW_AGGREGATE=1` is inherited. `KERN_MEADOW_GROUP_CACHE=1` is rejected:
its certificates support rigid branch wind, not this nonlinear deformation.
The visibility-buffer/resolve route remains selected in the measured profile.

## User distance regression and source-range comparison

The r63d playtest exposed a separate distance problem: the user reported grass
disappearing near 100 m. F12 captures
`games/sideways/build/screenshots/sideways-1793199278964-109148-0.png` and
`sideways-1793199285612-109148-1.png` visibly show smooth, bare distant hillside
beside dense foreground grass. Both JSON sidecars report source fade 80–100 m,
live distance scale 1, size 2, density 0.55, `distant_representation="none"` and
`aggregate_enabled=false`. Size 2 enlarges blades and near LOD distances; it
does not extend the outer source radius.

Historical metadata still prints aggregate outer endpoints 160–180 m and an
`authored_source_band.scope` sentence describing a source-to-aggregate handoff.
Those statements describe the classic aggregate configuration and are inactive
for these windswept captures: **there is no replacement grass beyond the source
fade**. Read the active representation, enabled flags and source range together;
the dormant aggregate endpoints do not establish distant coverage. RGB F12
files do not contain depth, so they cannot establish an exact ground distance
for each hillside pixel.

[`windswept-grass-distance.toml`](../../games/sideways/scenarios/windswept-grass-distance.toml)
reuses the first F12's exact camera, seed 1, 3 km stage, clear/noon and one native
3840×2160 view. It contains one 96-frame section, 24 warmup frames and a capture
on frame 24. Camera position is `[-76.1271286, -4.6135941, -493.1341248]`, target
`[-80.5060654, -5.7635937, -498.2051086]`, vertical FOV 64°. Simulation and wind
time are held at zero; the user's car pose and moving wind are not reproduced.
The first image's larger exposed central hillside makes a second camera
unnecessary for this bounded regression.

The completed `distance-r63d-100/` and `distance-r63e-250/` runs under
`C:/dev/perf/warp-sideways-20261028/r63-windswept-grass/` compare source fades
80–100 m and 200–250 m. Both rendered and presented all 96 frames at native
3840×2160 with one visible view. Camera position/target/FOV, lighting, scene
seeds and saved vegetation settings match directly in capture metadata. Scenario
hashes match; the only environment difference is the explicit
`SIDEWAYS_MEADOW_WINDSWEPT_RADIUS=250` override in r63e. The frozen r63e binary
SHA256 is `707dcf7e2aa961c314ce918190abfcdabc3c13bb4b1536de6cdfa3264596283d`;
both executable hashes were checked against their run records.

GPU records were joined to presented, non-warmup, non-diagnostic gameplay frames
by frame ID. Both runs provide the same **69 steady frames, IDs 25–93**; final
GPU records 94–96 are absent from the asynchronous tail. Neither summary reports
GPU errors, drops, invalid/duplicate records or capture visibility-ID errors.

| Measure | r63d, 100 m | r63e, 250 m |
| --- | ---: | ---: |
| Median covered GPU interval | 22.813 ms | 26.745 ms |
| Median sum of all GPU passes | 22.094 ms | 25.363 ms |
| Median grass pass sum per frame | 6.041 ms | 8.317 ms |
| Median meadow placement | 0.187 ms | 0.258 ms |
| Median meadow visibility | 1.000 ms | 2.470 ms |
| Median meadow resolve | 4.807 ms | 5.509 ms |
| Median meadow composite | 0.039 ms | 0.042 ms |
| Allocated meadow pool per view | 24.27 MiB | 148.08 MiB |
| Submitted source triangles at capture | 884,920 | 4,223,800 |

The larger range adds 3.932 ms to the median GPU interval and 2.276 ms to the
grass pass sum in this pair. Pass sums are formed per frame before taking the
median; individual pass medians must not be added to reconstruct those totals.
The interval excludes CPU and work outside the instrumented endpoints. These
are one fixed-camera measurements, not moving-gameplay or four-player results.
Full per-pass values and comparison checks are saved in the new
`distance-r63d-100-vs-r63e-250-comparison.json` beside the runs.

Visual inspection of both `captures/frame-000024.jpg` files shows that 250 m
source geometry covers the previously smooth central hillside with grass.
These are 1920×1080 JPEG previews of native 4K output. Static captures establish
this coverage difference; they do **not** establish reduced noise, shimmer or
better temporal filtering. **250 m is a source-geometry comparison baseline,
not the final distant representation.** The proposed 180/400 m comparisons and
an accepted far-grass representation remain unmeasured here.

The subsequent `distance-r63e-250-compact/` run validates the compact pool at
the same 250 m source range. Its frozen `sideways-r63e-native.exe` SHA256 is
`088132bad7274098ea4f9d5565da60948e43869204ac9212aafa098045336248`; the earlier
250 m executable is retained as `sideways-r63e-range-probe.exe`. Both hashes
were checked. Camera, lighting, scene seeds, live settings, scenario and full
environment match. Both runs again provide the same 69 steady frames 25–93,
with 94–96 absent from the asynchronous GPU tail and no reported GPU errors,
drops, invalid records or invalid visibility IDs.

| Measure at 250 m | Original pool | Compact pool |
| --- | ---: | ---: |
| Median covered GPU interval | 26.745 ms | 24.227 ms |
| Median sum of all GPU passes | 25.363 ms | 23.493 ms |
| Median grass pass sum per frame | 8.317 ms | 8.060 ms |
| Median meadow placement | 0.258 ms | 0.233 ms |
| Median meadow visibility | 2.470 ms | 2.288 ms |
| Median meadow resolve | 5.509 ms | 5.485 ms |
| Median meadow composite | 0.042 ms | 0.044 ms |
| Allocated meadow pool per view | 155,267,964 bytes | 75,129,660 bytes |

The compact pool uses 71.65 MiB instead of 148.08 MiB, a 51.61% reduction in
that allocation. Both captures retain 4,223,800 source triangles, plant counts
`[159, 0, 103846]`, 1,967,184 covered visibility pixels and 37,228 visible roots.
Unique visible primitives differ by one, 287,794 versus 287,795, with zero
invalid IDs. The GPU interval decreased 2.518 ms and the per-frame grass sum
0.257 ms in this pair; a single pair does not isolate every timing change as
an effect of pool compaction.

Decoded JPEG previews differ at 61 of 2,073,600 pixels, all within the 8×8 block
`x=1160..1167, y=320..327`. Using the largest RGB-channel difference per pixel,
24 pixels differ by exactly one 8-bit level, 37 by more than one, and one by
more than four; the maximum difference is five, with none above eight.
Visual inspection finds the same covered hillside and foreground. This is
localized preview agreement, not raw native-image equality or evidence about
temporal noise. The original range report is preserved; the new
`distance-r63e-250-full-vs-compact-comparison.json` contains all pass medians,
capture checks and JPEG statistics.

Final r63e validation passed 550 render tests (six ignored), 190 CLI tests
(one ignored), and the explicit GPU
`compact_pool_gpu_serialization_preserves_legacy_and_drawlists` test. Logs are
`r63e-render-tests.log`, `r63e-cli-tests.log` and `r63e-pool-gpu.log` beside the
runs. The root agent also verified `games/sideways/tools/test-play-dev.ps1`
exited zero with identity, artifact, presentation-proof, quoting, debug-UI and
syntax checks passing; that check is recorded in tool output without a separate
log. These results validate this release and compact layout, while 250 m source
geometry remains a comparison baseline for the future distant representation.

## Pinned source and deliberate differences

Reference: [Quick_Grass at commit 6b56164272f213e353a4045d1439d071d5a968cd](https://github.com/simondevyoutube/Quick_Grass/tree/6b56164272f213e353a4045d1439d071d5a968cd).
The relevant files are the [grass component](https://github.com/simondevyoutube/Quick_Grass/blob/6b56164272f213e353a4045d1439d071d5a968cd/src/base/render/grass-component.js),
[vertex shader](https://github.com/simondevyoutube/Quick_Grass/blob/6b56164272f213e353a4045d1439d071d5a968cd/public/shaders/grass-lighting-model-vsh.glsl),
[fragment shader](https://github.com/simondevyoutube/Quick_Grass/blob/6b56164272f213e353a4045d1439d071d5a968cd/public/shaders/grass-lighting-model-fsh.glsl)
and [noise helpers](https://github.com/simondevyoutube/Quick_Grass/blob/6b56164272f213e353a4045d1439d071d5a968cd/public/shaders/noise.glsl).
The MIT notice is retained in [Quick_Grass-MIT.txt](../licenses/Quick_Grass-MIT.txt),
copyright 2023 simondevyoutube. The noise helper's upstream attribution is also
retained beside the ported shader code.

Let `t` be normalized blade height, `s` range from −0.5 to +0.5 across its
width, and each `r` denote a stable random value in [0,1). The near source is:

```text
H = 0.6 * (0.75 + 0.75*r)               # approximately 0.45–0.90 m
W(t) = 0.04 * (1 - t²)
lean = 0.1 + 0.3*r
p(t,s) = anchor + Ry(yaw) Rx(-lean*t²) (s*W(t), H*t, 0)
base = mix((.02,.075,.01), (.025,.1,.01), r)
tip  = mix((.65,.8,.25), (.8,.9,.4), r)
colour(t) = mix(base, tip, t⁴) * (.75 + .25*r) * (.25 + .75*t²)
```

Dimensions are uniformly 0.4 times the reference's 0.1 m width and 1.5 m
nominal height. Anchor XZ and unbent height are quantized to 1/1024 for exact
half-float descriptors. Existing season, world pigment and live RGB edits still
apply. In r63d, pigment and the source's density-one basal darkening are
evaluated once per fragment from interpolated `t`. Stable base/tip values are
recovered from the existing cooked basal colour and blade descriptor. This
avoids linearly interpolating the baked nonlinear colour/AO product, which made
the one-triangle cut much brighter through its middle than the detailed cut.
The old small patch-height colour gradient is skipped. Along-blade normals follow the curve;
transverse normals rotate up to ±54°. Warp retains its thin-foliage lighting
instead of the reference's heavy blend towards an upward normal.

For world-root XZ `x`, time `τ`, and cubic-smoothed value noise `N` in [−1,1]:

```text
direction offset = N(.05*x + .05*τ)     # radians
tip angle A = 1.25 * (.625 + .375*N(.25*x + τ))²
wind rotation at height t = A*t
```

Warp rotates its supplied wind direction by that offset and multiplies the
angle by its wind amplitude; the supported multiplier reaches 1.5. Current
and previous time, direction and amplitude are independent, so motion uses
the actual previous pose. A normal/area cofactor accounts for the derivative
of the height-dependent rotation. Periodic noise lattice coordinates permit
continuous time rebasing. The final source path uses authored `normal.w=t`;
arbitrary atlas/voxel samples recover `t` from `y/H=t*cos(lean*t²)` with three
Newton iterations. `anchor.w=H` marks windswept; zero retains the classic path.

This is a technique port, not a pixel-identical demo recreation. Warp keeps its
stratified roots; the pinned demo uses seeded random offsets, despite the
tutorial's jittered-grid discussion. The demo's extra ±0.1 animated lean jitter,
player interaction, camera-facing blade thickening, centre stripe and upward
normal blend are not implemented here. Instead of reproducing its separate
distance-dependent width/lean/pigment changes, r63d morphs to the actual animated
coarse surface and keeps a consistent per-fragment material. Noise hashing,
placement and lighting also differ. The new wind is a travelling value-noise
field, not Perlin noise or a substitute sine wave.

## LODs, bounds and representation limits

Cooked blades retain six, three and one-segment cuts: **11/5/1 nondegenerate
physical triangles**, with every blade and anchor present. r63b/r63c selected
them at growth-scaled 6/12 m thresholds. The r63d runtime instead selects six
segments below `6*growth` m and one beyond it; the three-segment cut remains
available in the cooked asset but is not selected by this profile.

Over `3*growth` to `6*growth` m, smoothstep blends fine vertices onto barycentric
positions in the actual animated coarse triangle. It also blends their shading
normals without normalizing until fragment evaluation, preserving the affine
normal field at the cut. Roots and tips remain fixed under this morph. Current
and previous camera positions select their respective factors; both reuse the
blade's current/previous wind. After a topology change, three previous endpoint
positions cannot exactly represent an earlier curved interior, so camera-crossing
motion validation remains necessary.

Growth includes world variation and the live size scale: saved size 2 gives
roughly a 6–12 m morph band and a 12 m cut. The reference switches six to one
around 15 m for 1.5 m nominal height, equivalent to 6 m at our 0.4 scale.
These remain authored appearance thresholds, **not a projected pixel-error
guarantee**. Conservative animated correspondence errors remain diagnostic;
the classic error-driven policy is unchanged.

All poses rotate vectors bounded by the maximum blade radius. Placement and
frustum/road envelopes therefore use fixed anchor reach plus scaled blade
radius, enlarged for terrain shear, without adding another angular-displacement
margin. This preserves conservative support while avoiding the first attempt's
enormous swept boxes. Atlas mip cooking retains a discrete representative
point/anchor/height with weighted material/normal moments; voxel samples retain
height too. Those metadata contracts do not make a rest atlas preserve coverage
under nonlinear bending, and neither atlas nor voxel rendering is the selected
windswept runtime representation.

The user prefers filtered distant coverage over individual far blades. The
proposed path is a **source-derived, directionally prefiltered thin
layer following the terrain**, not individual distant blade submissions. A
shared field would retain habitat, growth and pigment statistics, while a
small profile bank retains height, directional coverage and shading-normal
distributions from the actual source. Shared wind would animate those moments;
current sun shadows, sky, headlights and fog would light the resulting layer.
Near and far would render independently against opaque depth, blend their
premultiplied representations once and supply consistent depth/motion metadata.
This design is not implemented or performance-validated.

The legacy `FarGrassField` offers reusable terrain traversal, tile bounds and
half-resolution integration, but its photo-derived upright-card profile and
stationary motion cannot represent windswept grass unchanged. Its half-resolution
colour/distance outputs use 23.73 MiB per combined native-4K output, before field
data or animation metadata. Historical far-pass samples ranged from 0.19 to
1.19 ms ([r12/r13 evidence](../WINDOWS_VALIDATION.md)); those old-source captures
do not predict this proposed path. Matching area alone does not preserve the
source's holes or opacity. The producer must also use the runtime's coarse cut
and analytic material, rather than simply averaging baked fine-LOD vertex RGB.
Coverage, low-sun lighting, road/silhouette clipping and animated transitions
remain explicit acceptance work before replacing the source baseline.

## Performance evidence and remaining checks

The first interactive attempt produced a user report of about 240 ms/frame.
Its early native-4K GPU log contains 58 returned samples; excluding the first
two gives a median 279.25 ms covered interval, including 255.29 ms aggregate
rendering. Aggregate depth/material medians are 114.83/137.00 ms. This is a
diagnostic sample, **not a controlled before/after benchmark**. The old aggregate
expanded every patch by about 3.38 m on every side at typical saved growth 2,
then repeated atlas reconstruction and nonlinear pose work in its depth and
material passes. The log does not isolate the separate costs of overdraw and
shader arithmetic. Live evidence:
`games/sideways/build/dev-runs/20261028T135934454Z-bb860b0f/frame-profile.jsonl`.

The **r63b and r63c source-only** runs used a visible one-view 3840×2160 native
output on NVIDIA RTX 5070 Ti, seed 1, clear/noon, saved vegetation, budget 120,
visibility resolve, and three fixed cameras. Simulation/wind time was held at
zero; this is not moving-gameplay or temporal-quality evidence. Each 96-frame
section excludes 24 warmup frames; captures occur on the last warmup frame.

Final **r63c** results:

| Camera section | Valid GPU samples | Median covered GPU interval | Median grass pass sum |
| --- | ---: | ---: | ---: |
| `normal-drive-start` | 72 | 23.18 ms | 6.50 ms |
| `grass-close` | 72 | 13.98 ms | 2.33 ms |
| `grass-overview` | 69 | 20.51 ms | 3.39 ms |

Grass sums placement, visibility, resolve and composite per frame; it excludes
unrelated scene rendering. The GPU interval spans the first to last covered
timestamps; CPU work and HUD/presentation outside that interval are not included.
All 288 frames rendered and presented, all three captures completed, and there
were no reported GPU errors/drops/invalid or duplicate records. Three final
overview GPU records were unavailable because of the asynchronous tail.

Preliminary r63b measured 23.14/14.09/20.55 ms covered GPU intervals and
6.42/2.40/3.38 ms grass, with 72/72/71 samples. Its overview missed one GPU
record. The matched r63c interval differences are +0.039/−0.103/−0.041 ms;
this single pair does not establish a meaningful performance change from
using authored `t` instead of recovering it. The first bad live run is not a
matched baseline for either fixed-camera run.

Evidence root: `C:/dev/perf/warp-sideways-20261028/r63-windswept-grass/`.
`game-source-lods-visible/` contains r63b; `game-r63c-visible/` contains r63c.
Each holds `run.json`, `environment.json`, `frames.jsonl`,
`budget/summary.json` and captures. Frozen executables and SHA256:

- `sideways-r63b-native.exe`:
`fe2021ab8ec9efb2914f4a3b51a83b61a8495560f55c39bed6171cca7a4e653f`.
- `sideways-r63c-native.exe`:
`751890eeecd13f613710be01954359a1d6781786d55005c29aa9903ad7fd1a1e`.
- `sideways-r63d-native.exe` (validated in the bounded cases described above):
`b29eadbc487619d87c9de338a4ededa6776c61d8b8335b173adb5ab693b11811`.

Captures are 1920×1080 previews of the verified 3840×2160 output. Inherited
aggregate/source-band environment flags in these runs were superseded by the
windswept runtime policy; environment strings alone do not prove active passes.

## r63d mechanical validation

The renderer suite passes **547 tests, five ignored**. Four serial GPU oracle
tests were then run explicitly and passed:

- 1,828 paired cooked-source/legacy deformation cases.
- 61 position and finite-difference surface cases against an independent f64 oracle.
- 756 matched material samples across 6/3/1 cuts and three seasons, plus two
  legacy/atlas cases.
- 891 cooked-source morph vertices, including agreement with the coarse
  triangle's plane/area, affine normals and independent previous-camera factors.
  Sampled positions at normalized distances 6±0.01 m differ by less than
  0.00003 m; this is a bounded cut residual, not exact prior-topology motion.

Logs: `C:/dev/perf/warp-sideways-20261028/r63d-render-tests.log` and
`C:/dev/perf/warp-sideways-20261028/r63d-gpu-oracles.log`. The frozen r63d hash
above was checked independently. The 61-frame approach sweep and final three-camera
timing run also completed; the latter's results are reported at the top of this
document. These bounded checks do not establish all-weather appearance or broad
temporal quality. No four-player, DLSS, 120 FPS or final visual-acceptance claim
follows from these tests or captures.
The tutorial's roughly 1.4 ms grass figure covers a grass pass with unestablished
hardware/resolution here; it is neither a whole-game budget nor a directly
comparable result.

The earlier r63c freeplay used owned PID 48036,
`games/sideways/build/dev-runs/20261028T142541895Z-9535f41e/sideways.exe`,
matching the frozen r63c hash. The launcher confirmed a visible first frame at
native 3840×2160, one view, wind 0.8, aggregate off and visibility resolve on.
The user changed live vegetation size (110% observed versus the benchmark's
saved 200%); HUD timings from that session are not controlled comparison data.
That process is no longer active. The subsequent r63d freeplay also closed
normally after the user's distance/noise feedback; no game is active at handoff.
