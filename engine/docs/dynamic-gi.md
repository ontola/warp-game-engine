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
by less than about two steps is a hit; five bisections find the crossing.
The hit's radiance comes from **last frame's resolved image** (the TAA
history) at the hit's reprojected position, so a reflection shows whatever
the frame showed there: lit terrain, trees, voxel crowns, cars, grass, with
their shadows and their own reflections (a second bounce, one frame late).

Every pixel traces its mirror ray, never jittered: one jittered ray per
pixel hits and misses at random from pixel to pixel, a bright lamp most of
all, which shows as grain TAA cannot average away. The roughness blur
is prefiltered instead, as cone-traced reflections do: each frame the
history is copied into a mip chain (`reflection_mips.rs`, level 0 full
resolution, 0.10 ms at 4K on an RTX 5070 Ti), and the hit reads the level
whose texels match the lobe's footprint (roughness times the ray length,
in pixels at the hit), blending the two nearest. Each level is a
luminance-weighted (Karis) average of the four texels above it, so one very
bright pixel (a bulb) does not fill a coarse texel, and a level above the
first is read with a tent of four taps, so its texels do not show as
squares. The hit test allows the step just taken plus 0.5 % of the depth
(at least 10 cm): a looser one took rays passing behind a thin frame for
hits. Confidence
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
- The depth buffer has no thickness: a ray passing just behind a thin
  object (a pole, a branch) can count as a hit.

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
