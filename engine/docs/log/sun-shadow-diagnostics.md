# Low-sun shadow diagnostics — 2026-10-24

These are independently opt-in experiments, not performance claims. Root
owns matched GPU validation and playtest promotion.

## Playtest decision

**Receiver-plane and adaptive-cliff candidates are rejected for this playtest.**
The selected launch configuration is `KERN_ROCK_VARIATION=1`,
`KERN_SUN_SCENE_DEPTH=1`, `KERN_SUN_RECEIVER_PLANE=0`,
`KERN_SUN_SURFACE_SOURCE=0`, `KERN_SUN_TERRAIN_ERROR=0`, and
`SIDEWAYS_ADAPTIVE_CLIFF=0`. No global default switches were made here.

Actual `r23-lighting/rock-clipped/lit.png` was inspected. Domain clipping
removes the broad triangle wire pattern, and source surface casters remove
the long black road polygons. However, four audited wall slits remain black
and additional black stipple appears on small steep facets. Raw near/far maps
also report occlusion there. This is unacceptable despite passing isolated
mathematical and GPU fixtures. The low-sun near/far lighting defect is **not
declared solved** by this launch. The selected scene-depth correction handles
the independently demonstrated missing distant casters; it does not fix bias,
CLOD mismatch, or subtexel self-shadowing.

Future investigation must use the exact retained wall pixel/triangle cases,
source versus selected caster geometry, and actual per-texel raster depth and
coverage. In particular, a shadow texel may not sample a small receiving
triangle at all. A nearest-depth fallback then aliases even though its sample
distance is bounded. Raising precision epsilon is not an evidence-based cure
for missing coverage or different surface geometry. Do not re-enable this
filter on the strength of the flat-plane or V-crease fixture alone.

## Source evidence

The F12 sunrise tree-root case uses sun direction
`[0.9653374, 0.0348995, -0.2586614]` (2 degrees elevation). Two pixels on the
same trunk triangle receive radically different sun visibility. The CPU audit
at `C:/dev/perf/warp-sideways-20261024/r23-tree-root-audit/clod-sun-rays.json`
intersects the actual selected terrain triangles, not a substitute heightfield.
Near-map selection and source both contain 35,374 terrain triangles. For the
bright receiver, blockers at 1.366, 1.692 and 4.020 m receive raster slope-depth
offsets of 2.170, 5.516 and 5.639 m. A dark adjacent receiver's blocker at
7.925 m receives only 1.329 m. All are inside the map. Far CLOD's nearest
bright-receiver hit at 4.716 m receives a 48.228 m slope offset.

This demonstrates a real unbounded bias mechanism for those centre rays.
It does not, alone, prove every reported camera-distance lighting change has
the same cause. Caster LOD and map coverage remain independent concerns.

## `KERN_SUN_RECEIVER_PLANE=1`

- Creates a separate sun depth pipeline with slope bias zero, retaining only
  the existing two depth-unit constant bias. Headlights retain their original
  depth pipeline, matrices and receiver policy.
- The opaque shader uses the actual rasterized triangle normal, not the
  interpolated lighting normal or texture normal. At each texel, it compares
  the depth of that plane. It does not move the surface along its normal.
- Sixteen explicit depth loads reproduce the exact 4 by 4 footprint and
  separable weights of the old nine bilinear comparison samples. There is a
  small f32 projection-roundoff allowance, not a slope-scaled world offset.
- Follow-up: the initial infinite-plane kernel introduced dark triangle edges.
  The current candidate clips taps to the actual receiving triangle and
  renormalizes retained weights. A sunlit concave V-crease analytically
  reproduced the defect even with identical source geometry and no roundoff:
  original visibility drops to 0.70 near the seam; domain clipping keeps 1.0.
  When the triangle contains no map sample centre, a single nearest sample
  compares the actual receiver depth, with at most half a texel quantization
  per axis. It neither extrapolates beyond the triangle nor forces visibility
  to one. Subtexel geometry can still alias; moving GPU validation is required.
- Near-parallel, absent or invalid planes use a finite centred-depth fallback.
  Volume, grass-composite and transparent consumers do not invent a geometric
  plane from their shading normal. Their smoothing quality needs GPU review.
- Existing near/far blend weights and canopy fallback remain unchanged. Wheel
  particles have their separate lightweight comparison shader, also unchanged.
- The spare `LightUniform.near_shadow.w` transports the flag without a layout
  change. Default zero keeps the original shader branch and raster settings.

Sixteen explicit loads are not assumed cheaper than nine hardware-filtered
comparisons. Measure the opaque shade cost and full four-view workload.

## `KERN_SUN_SCENE_DEPTH=1`

The old projection covers 399 m toward the sun and 500 m away from its target.
A 600 m sunward caster can therefore be absent even when its receiver is well
inside the shadow map. This flag replaces only sun-map Z with a conservative
absolute scene interval, identical for far and all near maps. X/Y snapping,
map dimensions, selected caster error thresholds and the headlight projection
are unchanged.

The build-time world AABB includes actual casting instance spheres, maximum
allowed affine tree/grass wind, articulated branch path lengths, and the full
baked animation clip envelope. CPU instance updates can expand it; it never
shrinks during a scene. Projecting its corners into the current sun direction
and rounding outward includes all these casters. Current receiver positions
are included too. This is O(instances + rig parts + baked parts times clip frames) once at scene build,
O(updated instances) for CPU updates, and eight corners per frame. It does
not walk all instances every frame or add another shadow map.

Tradeoff: an entire long stage can give a much wider Z interval than the old
900 m. Depth precision and extra eligible sun casters must be measured. This
does not repair insufficient X/Y coverage, terrain outside the loaded scene,
or CLOD silhouettes; it is not a terrain horizon implementation.

## Acceptance

### `KERN_SUN_TERRAIN_ERROR=1` isolation

An additional default-off diagnostic caps only opaque `TERRAIN` sun casters
at 0.015625 m world error in both maps. A caller's stricter threshold (including
zero/source-only) remains stricter. Culling converts that same cap to each
map's pixel scale and applies it identically to parent and own cluster error,
preserving a complete CLOD cut. Other materials, colour-camera geometry,
headlights, map sizes and filter footprints do not change. The existing
material binding and a reserved `ViewUniform.fog_cull.z` transport this without
another buffer or pipeline. F12 metadata and the development launch manifest
record the flag.

The focused CPU test and all four composed shader tests pass. The far map's
physical PCF footprint is still three times wider than the near map's; a
remaining filtered edge is not by itself evidence of incorrect geometry.
Measure added caster triangles/cost before promoting any such policy.

### Source geometry isolation and raw diagnostics

Root's matched all-source (`lod_pixels=0`) GPU run removed the new long black
road polygons but retained thin wall seams. This separates surface-CLOD
mismatch from the independently reproduced triangle-domain filter defect.
The 1.5625 cm terrain cap did not change the far tree-root sample.

`KERN_SUN_SURFACE_SOURCE=1` is therefore another default-off diagnostic:
opaque `TERRAIN|ROAD` casters use zero error in both sun maps. It does not force
trees, colour-camera geometry or headlights to source LOD. Visible CLOD can
still differ from source casters; this is an isolation, not a claimed general
solution. Its extra caster cost must be measured.

Debug 15 exposes the raw near filter (requires near enabled); 16 the raw far
filter; 17 the near-map blend weight. These avoid conflating an individual
shadow map with the canopy fallback inside debug 9.

After these follow-up edits, four receiver-plane CPU tests and all four
composed shader checks pass. Root repeated the actual GPU fixture:
`r23-lighting/sun-fixture-clipped/report.json` passes all four 600 m blocker
cases (visibility 0), unblocked plane (229), and both concave V-crease faces
(229 each). This proves the isolated correction but does **not** override the
failed actual wall acceptance described above.

Added CPU regressions compare plane-depth math against independent ray/plane
intersections at multiple sun angles and map sizes; compare the 16 weights
against the nine bilinear samples; exercise 600 m caster coverage with exact
X/Y equality and shared near/far depth; and cover maximum wind, dynamic bounds,
baked clip interpolation and articulated branches. Full composed WGSL parsing
must pass before GPU tests. Root ran the full render suite: 181 passed, one
ignored; CLI suite: 117 passed, one ignored. Logs are in the r23-lighting
directory below.

Root's actual native 640 by 384 GPU fixture with both flags passed:
`C:/dev/perf/warp-sideways-20261024/r23-lighting/sun-fixture-after/report.json`.
The 600 m blocker gives zero visibility in the near, far, opposite-far and
near-disabled cases; the unblocked receiver is 229/255. This proves that
caster depth inclusion and the tested plane comparison execute on the GPU.
It does not establish gameplay FPS, moving stability, or general foliage
quality. Root also reports the actual near tree-root case changes from
shadow-debug 218 to 2; its far-only case still leaks (171), so the full
near/far continuity issue remains under investigation.

Run four matched flag combinations on the actual F12 tree/rock case, plus
`render/examples/sun_visibility_gpu.rs`: 600 m blocker, near/far receivers,
focus shifts, and unblocked-plane acne checks. Then inspect representative
normal sun angles, foliage, close car shadows, headlights and multiple views.
Do not promote a diagnostic solely because one dark patch becomes darker.

Pre-edit copies of frame.rs, stability.rs and shade.wgsl are saved under
`C:/dev/perf/warp-sideways-20261024/r23-lighting/source-before/`.
