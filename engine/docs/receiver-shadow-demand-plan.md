# Current-pose directional shadow demand: prototype and remaining design

26 October 2026. The original design below now has a default-off implementation.
See [r49 evidence and rejected candidates](lod-shadow-demand-r49.md): sparse
native fixtures preserve exact images/requested depth, but the full scene
still requests all tiles. No full-scene raster saving is demonstrated.
Keep the existing shadow projections, resolution, alpha test, current poses,
caster LOD and lighting. Remove a caster only when its conservative light-space
footprint cannot affect any receiver queried this frame. This is not a paged
shadow cache, a rest-pose substitute, or a camera-visible caster list.

The first implementation avoids the proposed frame reorder: it requests demand
from current instance spheres intersecting each current camera frustum, plus
conservative meadow and particle providers. Terrain bounds are refined using
all resident clusters, including every LOD/material. Unique tile occupancy
counts stop redundant gathering as soon as all relevant maps are full; caster
queries also retain full maps immediately. Unknown providers force full
demand. These conservative bounds still over-request. The selected-cluster
preparation below remains a possible refinement, not an implemented frame split.

The Sun-map selector uses `view.extra.z = -(map_index + 1)` and is accepted only
for orthographic Sun views in `[-5, -1]`. Positive values are reserved for the
existing raster alpha mip scale. The initial `receiver-a` fixture exposed this
ABI collision: positive near-map selectors changed alpha coverage even when
no near-map clusters were rejected. That failed evidence is retained. The
repair changes the selector namespace, leaves raster alpha testing unchanged,
and adds a regression evaluating the actual raster shader expression. Corrected
B/D/E GPU fixtures prove requested-texel depth and final-image parity in their
bounded cases; that is not acceptance for all game scenes or useful speedup.

The motivation is actual submitted work. In the r47
[hidden-four on-a sunrise capture](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/hidden-four-a/on-a/captures/frame-000384.json),
the shared map selects about 47.84 million triangles and the four near maps
about 11.71 million. The corresponding clean
[budget](C:/dev/perf/warp-sideways-20261026/r47-meadow-work/hidden-four-a/on-a/budget.json)
reports 7.427 ms shared sun raster and 3.161 ms near raster, with 127 samples.
These are individual-run pass medians, not additive predicted savings. The
source bounds may produce nearly full demand at sunrise; occupancy must be
measured before claiming this design reduces that work.

## Minimum safe mechanism

Use one mask for the existing 4096² shared map and one for each existing 2048²
near map. Start with 64×64 shadow-texel tiles, a u32 occupancy value per tile,
and a 2×2 OR hierarchy. All five maps need 32 KiB for base masks and 43,684 bytes
including their OR levels, plus small descriptors/counters. There is no new
depth atlas or page allocator. Near maps retain their current 16 m half-width;
shared extent/depth fitting remains unchanged.

1. Run wind, part poses, animation and existing spatial-bound refit as today.
   Prepare all camera targets and run each camera's existing opaque visibility
   work once, including its two-phase tree Hi-Z path. Retain those buffers for
   subsequent shading; do not duplicate geometry submission. Current code
   renders shared shadows before camera visibility and near shadows inside the
   per-view loop, so this requires an explicit preparation/shading split in
   `frame.rs`, not an extra visibility pass.
2. Clear current-frame demand and gather receivers from **selected source
   cluster bounds**, transformed by current instances plus the existing wind
   margin. This is deliberately conservative. Opaque shading reconstructs a
   source-triangle point in `shade.wgsl`, rather than simply unprojecting depth;
   selected geometry bounds cover that point without assuming identical
   reconstruction arithmetic. A later exact-pixel mask must share that
   reconstruction code and prove its bounds. Never use camera fog/Hi-Z to
   reject a shadow **caster**.
3. Union the additional receiver providers listed below. Shared demand is the
   union of all views; each near map receives its own view's demand. Initially
   request both maps wherever either might be sampled; do not optimize the
   near/shared blend or discard back-facing receivers. Thin foliage and the
   current indirect response can use shadow visibility on either side.
4. Expand projected demand for the existing sampling footprint. The plane
   filter reads a 4×4 neighbourhood; the legacy path additionally shifts by
   up to 1.5 world texels along the normal and uses filtered 3×3 PCF. A proposed
   four-texel guard covers those existing footprints at the tested world scale,
   subject to an explicit footprint test. Derive additional rounding allowance
   from the light projection and coordinate magnitudes; unbounded precision
   uncertainty forces full demand. This is a **request guard**, not a new
   shadow bias. Convert to inclusive tile bounds only after expansion. Invalid
   bounds or an unregistered consumer request full-map demand.
5. In the existing Sun cull path, reject a spatial group, owner or selected
   cluster only if its projected conservative sphere misses every demanded
   tile. Keep existing scene-wide light-depth bounds: an offscreen tree or hill
   anywhere upstream can still block a requested receiver. Do not depth-cull
   against receiver height in the first version. An OR level chosen to cover
   the projected rectangle permits at most four conservative reads; a positive
   coarse result retains work rather than guessing emptiness.

Clear the entire ordinary depth map before drawing its retained casters. Its
unrequested texels may differ from the reference and must not be sampled.
Disable sparse-demand reuse/static splitting initially, or force full demand
when those modes are active. Current `ShadowContent` validity does not contain
receiver demand: newly requested tiles must never expose depth from an older
mask. Rebuild masks every rendered frame; wind, cuts, viewport changes and tree
Apply must not use stale receiver lists. No CPU readback is needed to render.

## Receiver/consumer audit

| Current consumer | Required coverage or fallback |
| --- | --- |
| Opaque terrain, trees, cars and cutout foliage | Current selected `(owner, cluster)` bounds from `main.visible`, including actual wind bounds. Alpha holes may over-request; they must not remove background receivers. |
| Forward glass (`transparency.wgsl::transparent_color`) | The current sphere of every relevant blended owner. Its triangles are deliberately absent from opaque culling. Build a compact owner list from `Scene.blended_materials`; do not filter it with caster `NO_SHADOW` policy. |
| Meadow source **and** directional aggregate | Reuse actual generated root lists and shared source support/growth/shear/wind bounds. Prepare source and far root generation early once, then reuse it for drawing. Project complete support, including fade/partial-coverage regions; source visibility IDs alone omit aggregate receivers. The 112-byte root record has no stored radius, so factor the existing radius equation or expose a bounds iterator instead of inventing a height. |
| Legacy transition/far grass | Source-owner bounds cover cards only. The separate far-field shading support needs its own provider; until registered, enable full demand for that renderer. |
| Part foliage volumes / real meadow bricks | Current `GpuTree.bounds_min/max` already include mip/filter support and are updated with part poses; brick `GpuLevel` bounds are explicit. Project the complete traversed volume, not an opaque hit or centroid. Unknown/stale generations use full demand. Initially these optional paths can conservatively force full demand. |
| Wheel dust/debris | **Additional real map consumer:** `wheel_particles.wgsl::particle_sun(center)` samples shared and near maps at the current analytic centre. Export the live particle positions/support using existing `wheel_particles.rs::position/radius` and the same clock; include all active centres conservatively. Until that accessor is connected, active particles force full demand. Never infer absence from opaque depth. |
| Analytic atmosphere fog | `shade.wgsl::fog_scattering_source` currently uses SH plus the camera terrain-horizon scalar. It does not sample these sun maps along the ray. No extra request is needed for this implementation. A future shadowed volumetric integrator must register its complete sample range or force full demand. |
| Headlights / headlight scattering | Headlight depth maps are separate and unchanged by this proposal. Current surface/vegetation sampling uses those maps. There is no current general headlight-fog march to cover with the directional mask; do not claim one exists. |
| GI / reflections / trace cache | The current bounce approximation uses the shadow result at the already covered receiver. `sky_visibility`, `reflected_visibility` and contact traces query canopy/terrain/solid proxies, not remote current sun-map points. The standalone radiance-cache consumer reads cached SH/moments and is not a live sun-map producer. Any future relighting of remote hits/probes must submit their receiver bounds or force full demand. |
| Exhaust / distortion | Current exhaust uses ambient/emission and existing HDR, not current directional maps. Keeping all original opaque receivers covers the pixels it may sample. |

This audit is a compatibility contract, not a permanent allowlist. Unknown
render extensions must default to full demand. Do not quietly disable grass,
particles, volumes, glass, fog or GI to obtain a favourable measurement.

## Implementation boundaries

Five bounded work units, after the current headlight experiment:

1. New `receiver_shadow_demand.rs`: map descriptors, one shared GPU buffer,
   validity/full-demand reasons, receiver-owner inventory and diagnostics.
   Register as an opt-in engine mechanism, default off.
2. New `shaders/receiver_shadow_demand.wgsl`: source-bound requests, provider
   sphere/AABB requests, filter guard, OR reduction and conservative rectangle
   query. CPU tests cover projection, touching edges, nonfinite fallback and
   four-view unions. No lighting or material changes.
3. `frame.rs`: split existing camera visibility preparation from shading;
   retain each view's immutable resources, union demands, then render unchanged
   shared/near shadows and shade. Add separate request/reduction timestamps and
   explicit capture of mask plus depth. Preserve TAA/DLSS history order.
4. `shaders/cull.wgsl`: one guarded Sun-only footprint test, after existing LOD
   eligibility and before appending clusters, plus optional conservative group
   rejection. Current cull layout has five scene storage buffers and ten view
   storage buffers; one demand buffer reaches the portable limit of **16**.
   Pack descriptors/masks/counters together and test the composed entrypoint;
   do not raise the device requirement. Far rest-pose pages, lamps and camera
   selection remain on their current policies.
5. Receiver adapter functions, principally `MeadowRenderer` preparation and
   `WheelParticles` current-support export. Reuse generated source data and the
   same clock; no second root generation, CPU forest BVH or GPU readback. The
   optional volume/far-field providers may initially select explicit full
   demand. These narrow adapters are required before a gameplay performance
   claim, even if the controlled first fixture uses fewer providers.

Capture requested/total tiles per map, full-demand reason, receiver counts,
group/owner/cluster rejections, selected triangles/vertices, shared bytes and
per-view work. A saturated mask is a valid **no-saving result**. Large terrain
clusters, overlapping forest bounds and a 180 m meadow can make the first mask
too conservative; refine only after measuring this, not by loosening coverage.

## First controlled proof

Use a small real-renderer native fixture with low sun, alpha-cutout moving tree
casters, an off-camera blocker, terrain, glass, source/fading aggregate grass,
a particle puff and a bounded fractional volume. Include an opaque surface
behind alpha holes, near/shared blend boundaries, a caster touching the filter
guard, four separated views, camera cuts, wind changes, night-to-dawn and a
provider-invalid/full-demand case. Also include many irrelevant upstream
casters so the test must show some real omitted triangles.

For identical poses/projections, compare full reference maps with candidate
maps **bitwise at every texel of every requested tile**, separately for shared
and near maps. Unrequested texels are expected to differ; whole-map hashes are
not a valid parity criterion. Verify masks match the declared filter support,
no sampled receiver lies outside demand, and requested-depth differences are
zero. Retain exact lit/directed-shadow images as a separate end-to-end check,
including grass, glass and particles. Candidate caster lists are expected to
be smaller, so the existing whole-list equality gate must be replaced with
the requested-depth oracle, not silently waived.

Only then use the existing r47 four-view sunrise/noon scenario with the same
executable and one flag toggled, off/on/on/off. Keep source recipes, lighting,
resolution and all effects fixed; retain capture/warmup exclusions and report
request/reduction overhead, cull/raster work and the complete GPU interval.
No performance number or default promotion is implied by this plan.
