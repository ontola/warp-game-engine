# Procedural meadow field — r34

The new Sideways playtest mode is `SIDEWAYS_PROCEDURAL_GRASS=field`.
It replaces the photographed grass across supported ground, rather than
replacing only the 3.6 × 1.8 metre r33 demonstration patch. It is an opt-in
development mode while visual and cost validation continues.

## Engine and game responsibilities

Sideways supplies the native terrain height field, signed road clearance,
habitat settings, growth, season and seed. These are uploaded at scene creation.
Warp cooks three shared MeadowGrass variants from the same assembly recipe
used by the unified vegetation experiment. All source LODs retain 36 blades
and four branch groups per tuft. Geometry, actual normals and material colours
come from that source; no photograph is substituted in a transition band.

Each camera dispatches a bounded GPU candidate grid. The GPU samples terrain,
rejects roads and unsupported slopes, derives stable world-cell variation,
culls against the camera, selects source geometry by projected error, and
builds indirect draws. Camera movement does not require a CPU root list or
plant-transform upload. Four views currently evaluate their candidate grids
independently; sharing a resident tile cache across views is future work.

After opaque rendering, a current-frame minimum reversed-depth pyramid lets
the GPU reject entirely hidden wind-expanded plant bounds. Its minimum
reduction preserves sky holes; odd last rows and columns fold into the last
parent. The query covers every relevant texel and retains near-plane crossings.
It does not use previous-frame visibility. `KERN_MEADOW_OCCLUSION=0` disables
the rejection for image parity tests (the pyramid still builds). Its cost is
included in the `meadow placement` GPU interval. This reusable depth pyramid
is currently consumed by meadow only, not the main tree/mesh culler.

Four branch wind poses are evaluated once per visible root on the GPU.
Depth, colour, actual blade normals and previous-frame motion all use those
same poses. The gust field follows the tree wind harmonics. The CPU updates
camera, lighting and wind uniforms, not individual grass blades.

The field is bounded to 100 metres, with true opacity fading from 80 metres.
The selected source LOD preserves all blades; it does not randomly thin grass.
The compact root/owner buffers have a mathematical candidate bound and an
explicit overflow check during diagnostic readback.

## Current limits

- This scalable field renderer uses source mesh LODs. Source-derived voxel
  aggregation for distant field tiles is still required; the bounded r33
  per-branch volume tracer is not silently multiplied across the landscape.
- Grass receives shared scene lighting and shadows. It does not yet cast
  its own shadow map or contribute a new grass occlusion field.
- The outer fade uses the nearest grass surface; it is not multilayer
  transparency. No claim of complete source/voxel optical parity is made.
- Native and DLSS use the same geometry. DLSS motion receives actual current
  and previous source positions. A wind direction/configuration discontinuity
  resets history; normal clock advancement does not.
- Shared lighting shader reload is explicitly rejected while this renderer
  is active, pending transactional meadow pipeline reload support.
- Root/owner memory is per view and source triangles are drawn twice for
  the depth prepass and colour pass. Instancing is not a promise of low GPU cost.

## Measurement and evidence

Evidence: `C:/dev/perf/warp-sideways-20261025/r34-meadow-field`.
CPU tests after Hi-Z: renderer 304 passed, one ignored; game 144 passed, one
ignored before the renderer-only Hi-Z change. Focused checks include source
equivalence, invariant blade counts, deterministic variation, valid geometry
bounds, conservative odd-sized depth reduction, sky holes and WGSL validation.
Terrain and road input tests are in the game suite.

The actual GPU motion pass passes all 157 synthetic affine-oracle cases,
including 61 new meadow cases, independent views, camera cuts, moving grass,
hidden-layer rejection and partial layers over grass. This validates motion
layer consumption; it is not an independent oracle for source vertex production
or a proof of DLSS image quality.

F12 and scenario captures report separate meadow indirect plant/triangle
counts for each LOD and view. Main `FrameStats.triangles` does not include
these hardware meadow draws. Capture metadata reports both source triangle
counts and depth-plus-colour submissions. Normal gameplay never reads them
back synchronously.

The user reported concurrent CI CPU load. All measurements in this iteration
must retain that caveat. CPU frame time, GPU timestamp intervals, startup and
capture readback are different quantities. No clean speedup or 120 FPS claim
can be inferred from these runs.

Before Hi-Z, the first full-field 4K-output DLSS Quality capture selected
25,021 plants / 17,909,784 source triangles. Depth plus colour submit those
triangles twice. This includes hidden terrain behind the foreground cliff,
which motivated conservative occlusion rather than removing visible coverage.

### Actual GPU results

- `hiz-parity-a/parity-report.json`: native1280x720 off/on/off, same executable,
  wind0.8 at fixed time1.9, first/settled frames and camera cut. All four
  candidate captures and repeated-reference captures are decoded RGBA-identical.
  Cliff source submissions fall14,448,888 ->118,008 at this resolution.
- `stage-hiz-one`: 3840x2160 output,2560x1440 internal, DLSS Quality;120 rendered
  and presented frames without errors. First close-cliff source count118,008.
  GPU covered p95 across four held views16.14/21.09/22.21/28.43ms,9 steady
  samples each. This is a small instrumented scenario, not typical gameplay FPS.
- `stage-hiz-four`: one4K output with four different cameras, then their camera
  cuts;120 rendered/presented frames without errors. GPU coveredp95
  34.01/34.22ms; CPU p9536.88/38.85ms,38 steady samples/section. Still well
  above8.33ms. Native per-panel output and reconstructed input sizes are in the
  run's final-state.json. No four-times-4K rendering.
- Synthetic motion oracle157/157 passes; it is distinct from the real DLSS
  scenario and does not prove temporal image quality in every route/weather.

Playtest first visible frame confirmed after18.15seconds, PID29644 at launch:
`games/sideways/build/dev-runs/20261025T085637578Z-r34-play/iteration.json`.
The game uses one4K output/DLSS Quality, full-field grass and live wind0.6.
Executable SHA256`0d64ca137d7fad3742d2cdf518e27a15661901be0c745cf57ae5e1e7f57b135b`.
Check current process identity before acting; this is a launch record.
