# r53: disappearing roadside shadow

The user's 26 October F12 captures show a narrow black wedge on the left
road shoulder that shrinks as the car approaches. This is separate from the
undersampled ditch corrected in r52.

## Reproduction and cause

Captures `sideways-1793041119988-32728-0` and
`sideways-1793041124315-32728-2` are from the same stage and noon lighting,
about 15.8 m apart. The exact stage noise/detail seeds are
`683407368 / 2033395789`.

Serial native 1920x1080 GPU replays reproduce the wedge with meadow disabled.
It is present in the shared sun map's receiver diagnostic. The world-terrain
visibility diagnostic is clear at the affected shoulder. Forcing only
terrain/road **shadow casters** to source geometry removes it while retaining
the normal camera LOD. Setting global LOD error to zero also removes it, but
that changes both camera and shadow selection and is not an independent
camera-only test.

The CPU audit identifies an actual false road caster. At preview pixel
`945,450`, the visible source shoulder is about 20.24 mm above native terrain.
The shared shadow cut replaces it with a triangle spanning road-along
607.149–652.176 m, with reported error 0.080328 m. A sun ray from the biased
receiver intersects that floating triangle 0.164888 m away. The source road
and near shadow cut have no blocker. Across the bounded shoulder samples,
the shared cut has 69 false hits of 129 receivers in capture 0, and 106 of 189
in capture 2; source and near cuts have none. These continuous CPU rays do
not model raster slope bias or PCF, so the GPU reproduction is retained too.

## Correction

`ClusterMesh::build_with_constraints` lets callers retain selected vertices
through all LOD cuts. Constraints add to existing material policy constraints;
the positional sloppy fallback is disabled when constraints are present.
Default builders retain their original behavior. The engine has no knowledge
of Sideways road coordinates.

Sideways locks its joining shoulder (`abs(uv.x) >= 1`) when cooking the road,
matching the already protected roadside terrain corridor. Source positions,
normals, UVs, collision triangles and driving physics are unchanged. The road
interior remains eligible for simplification. This adds no shadow pass,
texture, runtime buffer, or shader branch; retaining more shoulder triangles
has a geometry cost that must be reported separately.

## Diagnostic tooling

`capture_replay` accepts repeated `--pick x,y` in its native output pixels,
up to 64. Explicit picks replace the legacy defaults, preserve order and
duplicates, and record misses. Invalid pixels fail before GPU initialization.
Per-case reports now include shadow triangle/cluster counts, near-map subsets,
submitted vertices and shared-map world texel size, with counting scope.

Evidence root: `C:/dev/perf/warp-sideways-20261026/r53-roadside-shadow/`.
The immutable r52 replay executable supplies the before images. Replays omit
cars and HUD and are not pixel-identical 4K/DLSS gameplay or FPS measurements.

## Acceptance and cost

- Six focused engine cluster tests pass, including all-cut constraint retention
  and byte-identical unconstrained behavior. The full CLI suite passes 183 tests
  with one pre-existing ignored diagnostic. The 22 replay tests pass.
- All 19,676 source shoulder triangles survive the tested camera and shadow
  cuts. At the two captured locations, false shared-map CPU hits fall from
  69/129 and 106/189 to zero, with unchanged receiver counts.
- All three matched GPU approach views (0/8/16 m) remove the wedge with grass
  enabled. The matched grass-off view also removes it. Ordinary tree shadows
  remain. This is three held viewpoints, not a continuous motion stability test.
  The exact second F12 camera also passes visual inspection.
  At native pixel 886,421, grass-off RGB changes from 7/10/10 to 147/126/100;
  the nearby control at 850,430 remains 142/130/94.
- Source road geometry remains 82,476 triangles. The whole-stage shared shadow
  cut rises from 2,522 to 39,742 triangles, before frustum culling. The camera cut
  at capture 0 rises from 21,923 to 32,456, also before culling. These are road-only
  geometry counts, not total frame submissions or a GPU timing estimate.
- A separate hidden held-camera ABBA at 4K output / DLSS Quality uses 63 post-warmup,
  non-capture GPU samples per run, 126 per variant. Near/shared shadow cull+raster
  averages 0.913→0.919 ms. Covered GPU interval averages 21.794→22.419 ms; the
  measured pass union is 21.261→21.731 ms. The two AFTER interval means differ
  (22.01/22.83 ms), so no precise total-cost attribution is claimed. CI/shared
  machine load is uncontrolled. These are synchronized diagnostic GPU times,
  not presentation cadence, a four-player result, or progress to 8.33 ms.

The scenario runner's default summary deliberately excludes hidden diagnostic
frames and therefore has zero selected gameplay samples. `cost-summary.json`
explicitly analyzes their GPU intervals instead, excluding 64 warmup frames and
the image-capture frame. Raw profiles and this distinction are retained.

Release build completed in 88 seconds. Game SHA256:
`ccfbb7c9376db5800e4470edbbada3c984095617a5545cd37eb23f37c0a7bcb6`.
Diagnostic SHA256:
`d3f3f82d4a6a70e5117c12519cfaf2a56ae90f301ac8e4a273c93029337228e4`.
Per-run manifests, CPU audits, `visual-review.json`, tests and cost profiles
are retained under the evidence root. Grass-edge scalloping and other stages/
sun angles are not certified by this focused fix.
