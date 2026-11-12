# Dynamic global illumination and reflections

Warp has no hardware ray tracing to lean on: WebGPU has none, and every pass
must fit 8 storage buffers per stage. Dynamic bounce light and reflections are
therefore software traced against representations the renderer already
keeps: the depth buffer of the frame, the terrain heightfield (with its
min/max hierarchy, [terrain-world-visibility.md](terrain-world-visibility.md)),
the canopy extinction volume, the DFAO distance field and, later, the voxel
trees. This guide describes what is live and the stages still open.

## Live: traced reflections (stage 1)

Glossy surfaces (roughness below 0.45: water, car paint coats, wet or polished
surfaces) reflect the scene around them, not only the sky.

`traced_reflection` in `shade.wgsl` marches the reflected ray through this
frame's depth buffer, 40 steps that grow with distance (20 % per step, at
least 8 cm) up to 250 m. A step where the ray passes behind the depth buffer
by less than about two steps is a candidate; six bisections find the
crossing, and it counts only where the ray meets the depth buffer's surface:
at both ends of the last bisection the gap must stay within the ray's own
depth change, the surface's depth change across the pixels between them
and the depth precision, and the surface must face the ray. A ray passing
behind a nearer object (its silhouette in front of the ray) leaves a gap of
that object's thickness and marches on.
The hit's radiance comes from **last frame's resolved image** (the TAA
history) at the hit's reprojected position, so a reflection shows whatever
the frame showed there: lit terrain, trees, voxel crowns, cars, grass, with
their shadows and their own reflections (a second bounce, one frame late).

**Stochastic reflections** (default; `sssr.rs`, `sssr.wgsl`), as
Stachowiak's SIGGRAPH 2015 method. Shade traces one ray per pixel and frame,
sampled from the GGX distribution of visible normals (Heitz 2018), and writes
the pixel's normal, roughness, specular weight (the preintegrated BRDF times
AO), environment fallback and the hit's screen position to a per-view ray
target (rgba32uint) instead of mixing a reflection itself. Three passes
follow shade:

- Resolve: each pixel reuses the rays of eight neighbours (itself and a Vogel
  disc of 6 px, turned per pixel and frame) on the same surface (within four
  pixel sizes of its plane). A neighbour's hit lights the pixel along the
  direction to it, weighted by the pixel's GGX BRDF times cosine over the pdf
  the neighbour sampled it with; the weighted mean of the hits' radiance is
  the reflection (the ratio estimator), mixed with the environment by the
  share of rays that hit. Each hit reads last frame's image at the mip whose
  texels match the solid angle the sample stands for, 1 / (8 pdf) (filtered
  importance sampling).
- Temporal: last frame's result at the reflection's virtual position (behind
  the surface at the hit distance), clamped to the current 3x3 neighbourhood,
  blended at 0.1.
- Composite: the specular weight times the result, added onto the shaded
  image (shade leaves that term out where it wrote a ray).

A lamp's emissive panel is reflected like any other surface and averages
into the lobe with the rest. A semi-rough floor (roughness 0.35, GGX alpha
0.12) spreads a reflection over about 0.25 rad, so a door 4 m away
reflects as a soft bright patch, not a sharp door. Cost at 4K on an RTX 5070
Ti (`interior_rooms`, room 1): 1.19 ms for the three passes, plus 0.37 ms
in shade. `KERN_SSSR=0` returns to the single mirror ray below.

**Single mirror ray** (`KERN_SSSR=0`; also a clear coat's layer): the
mirror ray, never jittered, with the roughness blur prefiltered: each frame
the history is copied into a mip chain (`reflection_mips.rs`, level 0 full
resolution, 0.10 ms at 4K), and the hit reads the level whose texels match
the lobe's footprint, blending the two nearest.

Both paths share the march. A ray that never hits but passed pixels showing
something much farther than itself (sky, or the field through a doorway)
reads the last of those: it escaped through the opening. The reflection
fades out between 0.75 and 1 of the roughness cut. The first mip below full
resolution is a luminance-weighted (Karis) average of the four texels above
it; the levels below are plain averages, and a level above the first is read
with a tent of four taps. Light faces (EMISSIVE with a negative scale: shade
writes alpha 0 there) leave the chain: each texel holds colour times the share
that is not a face, and readers divide it out. Their light reaches glossy
surfaces as the light's analytic specular; in the chain a coarse level
spread a bulb over everything a reflection read near it (a pendant smeared
over the table under it and into the next room's floor). The coarse hit test allows the step just taken plus 0.5 %
of the depth (at least 10 cm). Confidence
fades towards the screen edge, the reach and the roughness cut; where it is
below one the old environment term (sky above the horizon, a heuristic
ground bounce below, heightfield and canopy occlusion along the ray) fills
in. A ray that leaves the screen cannot come back (the frustum is convex),
so it keeps that environment term: the old occlusion march still decides
whether terrain hides the sky there.

Under a clear coat (car paint) only the coat traces; the base layer keeps the
environment.

Cost: only pixels below the roughness cut trace, at most 45 depth loads and
up to eight bilinear fetches from the history mips each, plus the mip
build. It needs TAA (the history) and is off under
DLSS (no history in that path yet), in the simple lighting path and on the
first frame after a resize.

Switches: `KERN_TRACED_REFLECTIONS=0` (renderer-switches.md) or
`Lighting::traced_reflections = 0` (`--light traced_reflections=0`).

Known limits of the screen trace:

- Only what the frame shows can be reflected: the underside of a car, a
  mountain behind the camera, the far side of a tree are missing and fall
  back to the environment.
- The history includes distance fog between the camera and the hit, not
  between the reflector and the hit. Distant hits are slightly too hazy.
- Hits on moving objects are reprojected with the camera only, so a
  reflected moving car lags by its own motion of one frame.
- The depth buffer has no thickness: a ray passing within a pixel's depth
  change behind a thin object (a pole, a branch) can still count as a hit.

## Reflection probes

Indoors, what the frame does not show (the wall behind the camera, the
room seen past a doorway's edge) comes from live reflection probes: the
game gives one box per room, the renderer keeps them current itself.

```rust
renderer.set_reflection_probes(&[ProbeBox { capture: Vec3::new(0.0, 1.5, -13.5),
    min: Vec3::new(-4.0, 0.0, -16.6), max: Vec3::new(4.0, 3.0, -10.5) }])?;
```

- Capture (`reflection_probes.rs`, `Renderer::render_probe_face`): each
  frame, after the players' views, one cube face (128x128, 90 degrees) of
  one probe, round robin (four probes: all fresh within 24 frames, 0.2 s at
  120 Hz). It is a view of its own: cull, raster and shade with view 0's
  lights, shadow maps, irradiance volume and live bounce settings, and no
  TAA, exposure, tone mapping, bloom, traced reflections or live bounce of
  its own. No player state changes (shadow fit, LOD, grass detail, split
  screen). Its radiance is shade's, before exposure, so a probe reads the
  same in a dark room and a lit one. Light faces stay dark in it: their
  light reaches glossy surfaces as the light's own analytic specular.
  An opened door or a switched lamp shows in the reflections a few frames
  later, with nothing baked.
- Two cube arrays (8 probes, 8 mips, Rgba16Float, 6 MB each): the raw
  captures with box-filtered mips, and their GGX prefilter per mip
  (roughness = mip / 7, 64 samples, filtered importance sampling: each
  sample reads the raw mip that matches its share of the lobe).
- Shade: in a box (within 2 cm: a wall's face lies on its room's bounds),
  the reflected ray is box-projected (where it leaves the room, seen from
  the capture point) and read at the roughness's mip, times the specular
  occlusion; it replaces the sky and irradiance-volume environment there.
  Outside every box (a doorway, the wall between rooms) the boxes within
  0.5 m that the ray heads into blend, fading to 0 at 0.5 m: no seam in a
  doorway, and a shut door's face shows its own room, not the one behind
  it. Screen-space reflections go over it where they hit.
- Cost, 4K, RTX 5070 Ti, `interior_rooms`: the capture's passes 0.02 to
  0.03 ms a frame by their timestamps (cull, raster, shade of 16k pixels,
  mips and prefilter); shade reading the probes about 0.3 ms (11.0 against
  10.7 ms, `KERN_REFLECTION_PROBES=0`).
- Open: a moving object is captured face by face at different times (the
  swinging pendant can show twice, a few frames apart, in a mirror-like
  floor). The capture has no live bounce of its own. Probes are for rooms:
  outdoors the sky and the irradiance volume stay the fallback.
- `Lighting::debug` 27 shows the probes' reflection, 28 the same as a
  mirror (magenta: no box).

## Open stages

**Stage 2: terrain surface cache.** A camera-centred 2D clipmap of the
terrain's outgoing diffuse radiance (albedo from the terrain surface
functions, sun through the shadow maps and the baked terrain sun, sky
irradiance), updated a few tiles per frame. Off-screen reflection rays that
hit the heightfield (`terrain_trace`) read it instead of the constant ground
bounce. Procedural terrain fills it by evaluating `terrain_surface.wgsl`;
mesh terrain (Sideways) by a top-down raster of its terrain clusters.

**Stage 3: world radiance probes for diffuse bounce.** A sparse probe grid
around each camera (shared between split-screen views as in
[radiance-cache.md](radiance-cache.md)), each probe tracing a few rays per
frame against the heightfield hierarchy, the canopy volume and the DFAO
field, shading hits from the stage 2 cache and the screen history, and
storing L1 irradiance with distance moments. It replaces `Gi::bounce`, the
constant green ground bounce, so a sunlit red cliff tints the ground beside
it and a dark valley gets darker.

**Stage 4: trees and objects in the trace.** Voxel tree crowns
(`voxel_forest`) and the DFAO distance field as further hit representations,
so off-screen trees occlude and reflect, and probes inside forests see the
canopy's own bounce.

**Stage 5: screen-space diffuse.** A short screen-space trace for near-field
bounce (contact colour bleeding), sharing the stage 1 march and history.
