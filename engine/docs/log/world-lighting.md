# World visibility and cached lighting

Status: implementation in progress, 24 October 2026. The user requested a
Lumen-inspired architecture, accepting a longer implementation. This is not a
claim that Warp implements Lumen, virtual shadow maps, or full mesh SDF tracing.

## Problem and acceptance cases

The existing sun shadow maps cover nearby geometry. Their fallback takes only
12 increasingly spaced samples, reaching48.16m. A hill beyond that range can
therefore be ignored until the receiver enters the detailed shadow map.
Camera rotation also moves coverage boundaries. That makes world lighting
change when the camera moves even though the sun and scene do not.

Two real captures remain acceptance cases:

- `build/screenshots/sideways-1792856050487-192948-1.json`: sunset light on the
  distant cliff disappears while approaching. Camera solar visibility is
  already zero in this capture; fixing camera glare does not fix surface light.
- `build/screenshots/sideways-1792855853301-192948-0.json`: distant crowns are much
  darker than nearby source trees. The24m canopy density approximation spreads
  extinction outside the actual crowns and does not preserve directional gaps.

One large4096-square static shadow map was tested and rejected: both its initial
metre-scale bias and a later reduced-bias version produced unacceptable errors.
The latter failed open-gap visibility tests. It remains gated OFF. Evidence:
`C:/dev/perf/warp-sideways-20261024/r24-forest/`.

## Architecture

1. **World visibility** answers bounded ray queries against explicit geometry.
   The first implementation is a min/max hierarchy of native heightfield
   triangles. Empty space is skipped conservatively and leaves use the actual
   triangle diagonal. Results distinguish hit, proven-clear segment and unknown.
   A finite clear ray is not automatically a proof that the sky is visible.
2. **Surface data** supplies a hit's material and outgoing radiance. Geometry
   visibility does not invent green bounce light or change albedo to conceal
   missing shadows. A terrain representation cannot represent cars, overhangs
   or thin walls; those need their own backend.
3. **World radiance cache** shares probes across cameras, tracks revisions and
   validity, and limits update work. The first reference integrates diffuse
   one-bounce material transport into SH9, with directional distance moments.
   This is a bounded reference foundation, not completed production GI.
4. **Foliage visibility** must retain source-derived directional transmittance.
   Solid spheres or arbitrary darkening cannot replace porous crowns. Existing
   voxel/coverage prototypes must be validated before connecting this backend.

For terrain, a caller may explicitly declare that the heightfield describes all
loaded terrain and sentinel holes / outside bounds are known empty. Strict
queries retain unknown-outside semantics by default. NaNs, exhausted budgets,
and unrepresented streamed geometry must not silently become sunlight.

Detailed nearby raster shadows remain useful for cars, grass and moving
branches. Their coverage must not remove a valid world-space terrain blocker.
World caches are shared by all four views; camera movement alone must not
invalidate static terrain visibility.

## Evidence required before promotion

Current evidence and sequencing are tracked in the
[forest rendering roadmap](forest-rendering-roadmap.md). On24October the
hierarchical terrain query passed CPU/GPU and real-renderer approach/orbit
fixtures. The actual sunset-cliff replay removed false distant sun while
93.416% of the initial frame remained byte-identical. This is a direct-visibility
repair, not new bounce lighting; shaded surfaces still use the existing heuristic.

`KERN_PAGED_FOLIAGE_SUN=1` adds a separate bounded source-alpha cache: fixed64m
light-space pages, 512 squared depth including guards,256 shared layers,
8updates/frame. Only foliage casts in it; terrain is handled by its hierarchy.
The page GPU fixture passed alpha-gap, caster-depth, missing-data and four-view
cache tests. This is CPU-requested static rest-pose paging, not complete VSM.
The full-scene replay exposed an inherited17-buffer grass-selection layout;
selection now uses its own six-storage layout, keeping the16-buffer contract.
The64m pages missed1850 of2106 conservative scene requests. The controlled
`KERN_FOLIAGE_SUN_PAGE_METRES=256` setting trades distant texel detail (0.508m)
for coverage within the same256MiB: actual noon151/151 resident, sunset154/154
at all four approach positions. The original problem crown approaches its
independent local-source-shadow reference. Full optical/wind/performance
acceptance remains open. The17:52UTC lighting demo enables this setting plus
world terrain visibility; radiance-cache GI remains a separate non-live prototype.

- Real GPU queries agree with triangle-exact CPU reference on long blockers,
  slopes, valleys, translated scenes, holes and bounded-work failures.
- The same world receiver keeps its terrain visibility under approach and
  camera rotation, including where the nearby shadow map ends.
- Clear sunlight remains clear: no global darkening or self-shadow acne accepted.
- Near/far forest captures compare actual crown and ground samples separately.
- Report cold setup, steady GPU time, memory, unknown-query rate and four-view
  behavior separately. No120FPS claim without a measured8.33ms frame budget.

## Iteration improvements

`capture_replay --approach-m 0,20,40,60` retains one stage and renderer while
moving the camera along its initial horizontal axis. It freezes lighting and
records each transform. It is a diagnostic sequence, not a driving benchmark.

Shader reload prepares every active material consumer from one immutable source
snapshot and commits only after all pipelines validate. A failed edit keeps the
previous generation. Geometry-deformation changes still require restart.

## Primary references

[Epic's Lumen technical documentation](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine)
separates ray visibility, cached surface lighting and final gathering. Screen
traces have a world-tracing fallback; surface updates are amortized. This is the
architectural reference, not a performance guarantee for four-player120FPS.

[Epic's mesh-distance-field documentation](https://dev.epicgames.com/documentation/en-us/unreal-engine/mesh-distance-fields-in-unreal-engine)
describes heightfields for landscape and distance fields for meshes. Warp's
first terrain hierarchy is deliberately identified as a heightfield backend.

[Wright et al., SIGGRAPH2022 Lumen](https://www.realtimerendering.com/advances/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf)
is the technical reference for cached surface radiance and sparse world probes.
Directional distance-moment interpolation in our smaller reference cache is
DDGI-inspired; it is not attributed to Lumen's implementation.

## Compact static architecture (Growing Guns)

`Scene::static_sun_instances` explicitly opts fixed architecture into the far
sun map. Growing Guns selects only stadium tiers, walls and roof; actors,
destructible blocks and crowd animation remain in the dynamic maps. The map
fits those sources (128 m minimum half extent), rather than the legacy forest
map's 1024 m minimum. Camera motion and split views share one persistent bake;
source updates, sun direction, alpha cut or LOD invalidate it. A 4096² Depth32Float
map uses 64 MiB. This opt-in does not enable the rejected forest configuration.
Coverage blends into the dynamic maps, with geometric-normal bias capped at
6 cm for the architectural map. No claim is made here of a frame-time gain;
validation uses the 4K Growing Guns stadium camera script and cache ownership,
coverage and invalidation tests. Continuous day cycles invalidate as the sun moves.
