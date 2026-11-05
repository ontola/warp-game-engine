# Distant tree sunlight

User regression: F12 `sideways-1792855853301-192948-0` has pale, exposed
nearby crowns but an almost black band of crowns on the distant hills.
This capture has zero distance fog. Changing overall exposure or tree tint
would not correct the distance-dependent cause.

## Diagnosis

The same source meshes and leaf materials are used at both distances. A
component replay shows the band in direct light; albedo is consistent.
Outside the detailed sun maps, the previous implementation estimated direct
sun visibility through a coarse, additive forest-density field. At its 24 m
resolution, that field places density outside the visible crowns and cannot
distinguish gaps, neighbouring trees or the receiving tree itself.

The controlled 96-tree comparison holds camera, source geometry, atmosphere,
indirect lighting and canopy anchors fixed. Changing only real sun maps to
the coarse approximation produces 51.41 byte crown sun RMSE. Changing source
geometry to ordinary CLOD with real shadows gives 2.85 byte RMSE instead.
These are tonemapped diagnostic values, not linear optical measurements.

Of 1,050 reference-lit crown samples, average visibility falls from 150.9 to
114.0; 882 become more than 20 bytes darker. All 263 reference-dark samples
become more than 20 bytes lighter. The approximation loses both bright and
dark structure, so merely brightening distant foliage is not a repair.

The distant instance scale distribution also differed from the native one.
`tree_assets::scale_from_units` now shares the native coupled width/height
policy; native scales, random-number consumption and colliders are unchanged.
This reduces excess distant crown coverage but does not alone remove the band.

## Candidate implementation

**Historical candidate below is rejected and OFF.** Its stage-wide texels
introduced ground self-shadowing and failed the open-gap control. The current
replacement is foliage-only page caching (`KERN_PAGED_FOLIAGE_SUN=1`) plus
separate terrain visibility. Its small GPU fixture passes. A256m-page actual
forest replay has full request coverage and restores direct light to the
reported crown close to a separate source-shadow reference. This setting is
in the17:52UTC demo; whole-forest optical parity, wind and cost remain open.
See the [current roadmap](forest-rendering-roadmap.md) for evidence and status.

`KERN_FAR_SUN_SHADOW=1` enables a shared, cached sun-depth map built from
source-derived tree CLOD and its actual alpha cutouts, plus static terrain
and objects. Near and medium sun maps retain their existing detail. The
shader blends visibility between maps rather than multiplying their shadows.
The density field continues to supply indirect canopy occlusion and the
fallback outside source-map coverage.

The distant map is shared across split-screen views. Its static extent is
fitted to the loaded forest in sun space; its depth includes static terrain
casters. Camera rotation must not move it. Cars and live grass do not
invalidate it. Static caster edits, the sun and alpha/LOD changes must.
Far foliage shadows deliberately use the source rest pose: near shadows
still animate, while the distant map does not redraw for every wind frame.

This is a raster shadow cache, not completed voxel foliage or Nanite.
It must be assessed for crown contrast, terrain occlusion, transition
continuity, cache invalidation and GPU cost before acceptance.

## Evidence

`C:/dev/perf/warp-sideways-20261024/r24-forest/` contains:

- `crowns-components`: exact captured camera, albedo/direct/indirect isolation.
- `crowns-without-gi`: diagnostic only; disables both coarse sun fallback and
  indirect occlusion and is not a shipping setting.
- `crowns-scale-only`: shared scale policy, still-visible dark band.
- `dense-coarse-before` and `dense-coarse-review.json`: matched 96-tree test.
- `crowns-compact`: rejected compact-density experiment. It barely changes
  the band while increasing shading cost materially. It remains disabled.

Replays are native diagnostic reconstructions without cars/HUD, not the
original DLSS gameplay frames. Single-frame GPU timings are not route FPS
or evidence that the four-player 4K/120 FPS target has been achieved.

Candidate visual and performance acceptance is pending.
