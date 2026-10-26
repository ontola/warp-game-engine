# r52: repetitive roadside wedges

User report, 26 October 2026: repeated triangular rises and dips roughly every
two metres beside the track. F12 evidence:

- `games/sideways/build/screenshots/sideways-1793039797575-117876-0.png`
- `games/sideways/build/screenshots/sideways-1793039836887-117876-1.png`

Both captures are night scenes, lit by headlights. The actual generated stage
has noise/detail seeds `370671355 / -305494282`. Source grass uses the saved
size 2, population 0.55 and LOD error 4 settings. The issue is close to the
camera, before the 60-80 m source/aggregate transition.

## Cause and change

The terrain height function cut a 55 cm deep Gaussian ditch beside the road.
Its approximately 1.9 m full width was undersampled by the shared 2 m native
terrain grid. As an oblique road crossed the grid and its triangle diagonals,
the interpolated ditch acquired an artificial repeating longitudinal wave.
The road shoulder conformed to those same triangles, exposing their shape.

A CPU audit over 84 flat-road orientations/grid phases measured up to
0.344786 m of artificial peak-to-trough height. At the captured location,
the grid differed from the continuous intended surface by up to 0.477 m.
These are geometry measurements, not estimates from screenshot brightness.

The fix replaces this unresolved decorative groove with the existing
8 cm road foundation inset, smoothly returning to the surrounding surface
over 4 m outside the shoulder. It retains the terrain grid and the drivable
road ribbon. CPU construction, collision, grass roots and the GPU heightfield
derive from the same height function. No noise overlay or extra grass is used
to hide the shape.

## Geometry validation

Evidence root: `C:/dev/perf/warp-sideways-20261026/r52-roadside-terrain/`.
`geometry-before.json`, `geometry-after.json` and `geometry-validation.json`
retain the original measurements and final tests.

- Maximum artificial flat-road wave: 344.786 mm before, 8.533 mm after.
- At the captured ditch centre, maximum grid interpolation error drops from
  473.26/475.37 mm on the two sides to 24.09/11.65 mm.
- The outer 3.2 m offset also includes the existing hill transition. Its
  maximum error is 90.95 mm; it is not included in the 3 cm inner-verge claim.
- 38 simulation tests pass, two diagnostic tests are ignored by the normal
  suite. The explicit roadside audit also passes. Added regressions constrain
  direction/grid-phase waves, the continuous foundation join and the reported
  location. Existing exact road-ribbon identity and seam/collision tests pass.
- The older wider off-road foundation hash changes intentionally. The actual
  drivable ribbon vertices and triangle identity remain unchanged.

The native grid, collision topology and per-frame sampling algorithm are
unchanged. Slope changes can alter stage decoration eligibility and RNG
consumption, so before/after tree placements are not guaranteed identical.
No FPS improvement is claimed from this geometry change.

## Matched visual checks and replay tooling

The enhanced F12 replay restores applied tree settings before renderer
construction, and restores captured grass/debug/bloom/local-light settings.
It validates source recipe, range and crown fullness against the capture.
Missing historical fields use explicit historical neutral/off values, never
the current saved game preset. Malformed fields fail. `--no-meadow` records
its explicit override after restoring the capture. Nineteen focused replay
tests pass. Report schema3 records captured and effective values.

The frozen BEFORE executable is `capture-replay-before.exe`, SHA256
`e7b974249cf0949c565f4f351544257fd77785975fa5818a25c2c668a63fc825`.
Serial `before-grass` and `before-no-grass` captures completed and were
inspected. The same prominent triangular black wedges remain when grass is
disabled, confirming that the reported failure does not depend on meadow
geometry or clipping. The replay reproduces the captured stage/camera and
headlamps at native 1920x1080, omitting cars/HUD. These are diagnostic views,
not pixel-identical replays of the original 4K DLSS screenshot or FPS tests.

AFTER executable SHA256:
`aeddda752437a54ea1348d357b7ca7c36e3bb1cb486baf3eee59e38dd2e51cc0`.
Serial `after-grass` and `after-no-grass` renders completed and were inspected.
Both remove the prominent repeated black wedges at the captured roadside.
The grass-off result is a continuous, softly lit verge; the grass-on result
retains small irregularities from grass clearance, without the large terrain
pits. Captured controls, lights, stage seeds and camera match before/after;
source geometry and potentially slope-dependent decorations intentionally
differ. No light/shadow settings were weakened to remove the pattern.

The 182 CLI tests also pass (one ignored). The release game and diagnostic
built successfully together in 88 seconds. Game executable SHA256:
`82d4d08a2973df43a411a25f221bb6b0bbeb26c9b51ea7b3de05373f36466921`.
Build/test logs and per-run manifests are retained in the evidence root.
The updated 4K-output / DLSS Quality interactive playtest reached its first
visible frame (PID 32728). Launch manifest:
`games/sideways/build/dev-runs/20261026T185740242Z-424ec09c/iteration.json`.

## Separate grass-edge concerns

Grass does not cast shadows in these captures. Its clipping boundary can
nevertheless accentuate the verge: terrain road-distance attributes currently
use nearest road points, and source patches at terrain triangle boundaries
use conservative whole-patch road clearance. A grass-off matched replay is
needed to distinguish that shape from actual terrain/headlight shadowing.
These mechanisms must not be declared fixed solely by changing the ditch.
The existing ledge-specific overrides can also retain an 8 cm uphill or 2 cm
downhill step at the protected shoulder boundary; their design is unchanged.
