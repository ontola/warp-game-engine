# Water optics

Terrain water (ditches, streams, rivers and ponds from `terrain_field`) can
be drawn with refraction: the bed seen through absorbing water, Fresnel and
caustics. It is **opt-in** until it has been checked on real hardware
(Windows RTX 5070 Ti and the M5 Pro Mac). Without it, water is a dark,
smooth surface. Either way it reflects the scene through the
[traced reflections](dynamic-gi.md) every glossy surface gets.

## Turning it on

- Luau and Rust: `Lighting::water = true`, or the light setting `water=1`
  (`terrain_fly --light water=1`).
- Any game: `KERN_WATER=1` makes it the default for every `Lighting`.
- F10 debug menu, Light: **Water optics** toggles it live.

Reflections need temporal AA (they read last frame's TAA result). Without
TAA, or on the first frame after a resize, water reflects only the sky.
Headless: `--shot` renders a single frame, so use `--dump 5 DIR` to see
reflections (`terrain_fly --style lowland --at -975,-20,3,2.82,-0.12` is a
river between trees, `--style polder --at -130,6,4,-1.571,-0.15` a ditch).

## What it does

The water surface is the terrain itself, raised to the water level
(`Sample::pos` includes the water depth), so there is no separate water
mesh and the bed is never rasterized. The two halves of the look come from
two places:

**Through the water** (`terrain_surface.wgsl` `water_body`). The ground
colour under the water is treated as the bed, `depth` metres down (the
carved depth, or the depth a far film stands for). The view ray refracts at
the rippled surface (Snell, n = 1.333) and the sun refracts down too; along
both paths the water absorbs (lowland water with algae and silt: `sigma =
(0.95, 0.5, 0.62)` per metre, red first) and scatters an olive green back.
Deep beds settle to silt rather than grass. What the surface reflects
(Schlick Fresnel) it does not let through, so at grazing angles a river is
mostly reflection and seen from above it shows its bed. Shallow, sunlit
beds get moving caustics at the point the refracted ray lands, so they
shift with the view like a real bed does. They fade once a pixel covers
more than about 2 cm of ground, so they show only close up.

**Off the water** (`shade.wgsl` `traced_reflection`, shared with every
glossy surface; see [dynamic GI](dynamic-gi.md)). Water's roughness is
0.04, so its ray is the mirror direction. The trace tests the opaque depth
and, where rasterized voxel trees keep their own depth (`VOXEL_OCCLUSION`),
their depth too, so far forests show in rivers. Where the trace finds
nothing the environment (sky, or ground where a hill is in the way) shows.

The switch travels in the light uniform (`fog_sampling.w`); no binding is
added.

## Limits

- Only terrain water. A game's own water meshes (Sideways) do not use it.
- Objects standing in water are not seen below the surface: the terrain at
  water level hides them. Refraction only bends and darkens the bed.
- Reflections are screen space (see the traced reflections' own limits).
- Caustics brighten the bed's albedo, so they also show in the shade of a
  tree; they are faint and fade with depth and distance.
- One water colour for every style; it is not yet a style or region
  setting.
- Not yet timed on a real GPU.
