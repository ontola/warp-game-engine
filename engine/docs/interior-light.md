# Light in rooms

Outdoors, Warp lights a surface with the sun (shadow mapped) and the sky
(occluded by terrain and canopy). Neither knows about walls: without more
data a closed room is lit by the full sky and looks almost as bright as the
field outside. This guide covers the room bake, which gives rooms their own
light the way Call of Duty's precomputed lighting does, scaled to Warp's
probe grid.

## What it does

- **Traces the real geometry.** [`room_bake::Occluders::from_scene`](../crates/render/src/room_bake.rs)
  collects every opaque, static triangle of the scene at full detail
  (foliage, grass, alpha-tested, blended and skinned geometry are left out).
  No hand-made boxes.
- **Sky and sun bounce.** Each probe stores how much sky it sees directly,
  and how much sky and sun light reaches it after several diffuse bounces
  (the sunlit patch on the floor lights the ceiling, which lights the walls).
  Values are per unit of sky and sun illuminance, so time of day, weather
  and exposure changes need no rebake; only a moving sun or changed geometry
  does.
- **No light through walls.** A probe that sees the back of surfaces along a
  quarter of its rays is inside a wall and is ignored (the DDGI rule). When
  shading, a probe only counts if it is in front of the surface and on the
  same side as its nearest wall (separation planes), so a room's corners do
  not pick up the probes outside.
- **Reflections dimmed by the baked light.** Inside the probe grid, sky
  reflections are scaled by how much of the open-air ambient reaches the
  point, so a polished floor in a dark room no longer mirrors daylight.
  Traced (screen-space) reflections still show the room itself and the
  bright doorway.
- **Dark rooms stay smooth.** Probe values are stored square-root encoded,
  so a room that sees 0.1 % of the sky is not banded.

With auto exposure ([hdr-and-exposure.md](hdr-and-exposure.md)) the camera
adapts on the way in: in the look scene below the meter reads EV 15.8 on the
field and EV 10.1 in the room, almost six stops.

## Using it (Rust)

```rust
use render::room_bake::{self, Occluders, Settings};

// Probes every 0.5 m over the building (and a cell beyond it).
let grid = render::irradiance_volume::Grid::covering(Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 3.3, 5.0), 0.5);
scene.reserve_irradiance_volume(grid)?;
let renderer = Renderer::new(&gpu, &scene, &light);

let occluders = Occluders::from_scene(&scene, |_| true);
let probes = room_bake::bake(&grid, &occluders, light.sun_dir, &Settings::default())?;
renderer.set_sky_sun_separated(&gpu, &probes, &room_bake::separation_planes(&grid, &occluders))?;
```

`Settings`: `sky_rays` (2048; a door seen from the back of a room is a few
hundredths of the sphere, fewer rays make blotches), `bounce_rays` (512),
`bounces` (3) and `reflectance` (0.5, the average albedo).

Grid: a cell of 0.5 m over the building's bounds, `Grid::covering(min,
max, cell)`, which adds a cell on every side (the volume fades out over
half a cell past its outer probes; that fade must lie outside, not on the
floor). The geometry need not respect the grid: walls of any thickness, on
or off a probe layer. The bake moves every probe that lands in geometry or
on a surface out of it, by at most 0.45 of a cell per axis
(`room_bake::probe_positions`, DDGI's probe relocation, Majercik et al.
2021); a probe still inside after that stays invalid. With visibility on,
shading uses the moved positions (`room_bake::pack_offsets`, one word per
probe). Every bake of one geometry state shares that placement:
`room_bake::Placement::new` once, then the `*_placed` bakes
(`bake_placed`, `bake_lights_placed`, `bake_visibility_placed`,
`bake_reach_placed`, `separation_planes_placed`); each plain bake places
the probes itself.

A probe moved out of an outer wall sits just outside, in daylight, in the
same cell as the wall's inner face. Two things keep it out of the room:

- The bake's bounce gather weights the probes around each hit by an exact
  ray test from the hit to the probe (`corners`), not just by facing:
  without it such a probe lit a closed dark room with daylight through the
  bounces.
- Per probe, which voxels of its neighbourhood (the cells around its grid
  point, four voxels per cell) open space joins it to without leaving the
  neighbourhood: a flood fill across voxel faces no surface blocks
  (`room_bake::bake_reach`, 16 words per probe). Shading weights a probe by
  this reach at the point (`probe_reaches`, trilinear between voxel
  centres, so it changes over 12.5 cm instead of switching), on top of the
  distance-map visibility. The distance-map test alone left about 1e-4 of
  the daylight behind a 0.3 m wall: a few lux, which a room metered at
  EV -3 showed as a quilt of faint blobs. A probe next to a doorway still
  reaches round the jamb; a closed door is baked into the reach of that
  door state. The fallback for a point no probe sees uses only probes that
  reach it.

One volume per scene, up to 65,536 probes.

Lamps: `room_bake::bake_lights` bakes each local light's bounce on its own;
add the sets of the lit lamps into the emission when a switch flips
(`set_sky_sun_emission`). Light states: bake the whole set twice, with a
door shut (its box merged into the occluders, `Occluders::merged`) and
open, and blend the two by how far the door is open while it moves, as
Modern Warfare switches light states. The blend and upload of 14,000
probes is cheap enough for every frame of a swing.
`app/examples/interior_rooms.rs` does both.

Precision: each probe axis stores sky and sun transport as three 9-bit
mantissas with a shared 4-bit exponent (2^-23 to about 2, a constant ~0.2 %
step). A dark room lit only through a doorway has transport around 1e-5,
where a fixed-point code steps by tens of percent from probe to probe and
the walls show the probe cells as blocks once the eye adapts to the dark.

Smoothness: every probe shoots its rays in its own random rotation of the
ray set, and the result is averaged with the neighbours each probe sees
(never through a wall) after every bounce and twice more at the end, the
direct sky included. Light that reaches a room along a few rays only
(through a doorway, off a sun patch next door) would otherwise show as
blotches at the probe spacing. In shading (with TAA) the probe lookup is
offset per pixel and frame by up to 0.35 of a cell in the surface's plane,
so TAA turns the trilinear cell steps into a smooth filter at no extra
fetches. The wall tests (backface, separation planes, the latter without
their 15 cm blend) stay at the surface point; where the moved point finds
no valid probe at all (a corner) the lookup is repeated unmoved, rather
than falling back to probes that ignore the walls. A move into the next
probe cell is kept only if none of that cell's probes is inside geometry;
otherwise it stays in the surface point's cell, so at a room's edges it cannot
reach the probes beyond the adjoining wall. With probe visibility (below)
that cell rule is off: it clamped every move along a wall at the cell
boundary (the next cell always holds the wall's buried probes), so pixels
just either side of a boundary averaged only one way and the wall showed a
step of the gradient times the mean offset at every cell (about 7 % on the
room 2 wall). Instead a moved lookup counts for the surface point with the share of its
trilinear weight whose probes see that point (`irradiance_coverage`); for the rest
the unmoved lookup is used, chosen per pixel and frame. Renormalising the
moved lookup alone would turn the faint Chebyshev visibility of probes
behind a corner's next wall into full daylight; this way their weight stays
faint. On an open wall the share is 1 and the blur is symmetric.

Probe visibility (DDGI): `Scene::reserve_probe_visibility` and
`Renderer::set_probe_visibility` with `room_bake::bake_visibility`,
`pack_offsets` and `bake_reach` give every probe an 8x8 octahedral map of
the mean and mean squared distance to the nearest surface, its offset and
its reach (81 words per probe in aux; 4.6 MB for 14,260 probes).
Shading then weights each of the eight probes by a Chebyshev test of the
normal-biased point's distance against that map, read bilinearly, and a
smooth backface term, instead of the separation planes. Probes behind a wall
fade out smoothly rather than per cell. Cost in `interior_rooms` at 4K: the
distance maps within run-to-run noise of the shade pass (under 0.6 ms); the
relocation offsets and reach about 1.5 ms more (6.4 to 7.6-8.2 ms and
8.7 to 10.2 ms at two views, RTX 5070 Ti; the first, eight-load reach cost
8.4 and 11.1 ms). The bake takes about
23 s instead of 8 (relocation, reach, exact visibility in the gather).
`ROOMS_NO_DDGI=1` turns it off there for comparison.

## Light shapes: cookies and profiles

A lamp is rarely uniform. Clustered lights can carry a shape
(`render::light_shapes`, `ClusteredLight::with_shape`):

- A **cookie**: a 256x256 greyscale image projected through a spot's outer
  cone (`Renderer::set_light_cookie`, index 1..=15). A flashlight's hot
  centre, reflector ring and spill.
- An **angular profile**: intensity against the angle from the light's
  axis, 0 to 180 degrees in 256 steps, an IES-like curve
  (`Renderer::set_light_profile`, index 1..=255), for spots and omnis. A
  lampshade's bright rim; a downlight's cut-off that throws scallops on the
  walls.

Both multiply the light (0..1: raise the intensity to keep the peak) and
apply in the bounded volumetric fog too
(`Renderer::set_volumetric_beams_shaped`), so a flashlight's pattern shows
in its beam. A shape can also mark a light `diffuse_only` (no specular
highlight), for a bounce light standing in for a lit patch of wall. They live
in one 256x256 R8 texture array of 16 layers in the
scene bind group (binding 30); a light's shape is packed above its shadow
face count, so the light layout is unchanged. Cost: one or two filtered
fetches per shaped light per pixel it lights; in `interior_rooms` (four
profiled lamps over a whole view) about 0.3 ms of the shade pass at 4K, on
an RTX 5070 Ti, within run-to-run noise. Not in Luau yet.

## Light faces

A lamp you can see is two things: the light (`LocalLight`) and the surface
that emits it. `render::light_face::LightFace` (a `Rect`, `Disc` or
`Sphere` of given size) ties them together, so a game declares only the
shape:

```rust
const PANEL: LightFace = LightFace::Rect { width_m: 0.5, depth_m: 0.5 };
let light = PANEL.apply(LocalLight { /* position on the panel, intensity */ .. });
let mesh = scene.add_mesh(&PANEL.primitive(scene.add_material(PANEL.material(&light))).mesh());
// place it with PANEL.transform(&light); exclude it from the light's shadow
let lamp = ClusteredLight::shadowed(light, id).with_face(PANEL);
```

- The face's radiance is the light's intensity over the face's area (a
  Lambertian emitter of luminance L and area A sends L A candela along its
  axis). A face given its own nits drifts away from its light: the
  `interior_rooms` panels were 14 times brighter than the 450 cd their
  lights send.
- The highlight is the face's own. A clustered light `with_face` a
  rectangle or disc integrates its specular over the face with linearly
  transformed cosines (`face_light_specular`, Heitz et al. 2016): the GGX
  lobe is approximated by a cosine under a 3x3 transform looked up by
  roughness and view angle, so the integral over the face polygon (a disc
  as the octagon of its area) is closed-form, clipped to the horizon. The
  tables (`src/ltc_ggx.bin`, 64 x 64) are fitted to the engine's own `ggx`
  by `examples/ltc_fit.rs`; they sit ahead of the lights in the scene
  tables. Against brute force over a 0.5 m ceiling panel (28 roughness and
  view cases, `light_face` tests) the energy-weighted error is 1.6 %; the
  sphere of equal area (Karis 2013's representative point,
  `sphere_light_specular`, still used for sphere faces and lights without
  a face) was 21.9 %, about 40 % too bright for the forward highlight at
  low roughness. The cosine has no backscatter tail, so faint highlights
  with the face behind the viewer come out low. `apply` sets the light's
  source radius to the sphere of the face's projected area, for its
  shadow penumbra.
- The face's material is marked as a light's face (EMISSIVE with a negative
  `scale`). A reflection ray that reaches such a pixel counts as a miss
  (`light_face_at` in `march_hit`): the light's own specular already
  carries that light. Before, a lamp in view was reflected twice (its
  specular plus its face in the reflection, as a long, noisy streak) and a
  lamp out of view once; rays hitting the bright face were also nearly all
  of the reflections' sparkle (18,431 isolated bright pixels on the room 3
  floor at 4K before, 19 after; 5,235 during a fast turn before, 1 after).

Cost in `interior_rooms` at 4K (four panels lighting most of the view, RTX
5070 Ti): the faces and sphere highlights took the shade pass from 7.75 to
7.93 ms; the LTC highlights another 0.65 ms (8.25 to 8.90). The light
record is 5 vec4 (the face is the fifth).

## Cost

The bake runs on the CPU's cooking threads: 3.2 s for the look scene
(5,184 probes, 2,048 + 512 rays, 3 bounces) on 4 cloud cores. Shading reads
eight probes per pixel inside the grid.

The transport (sky and sun, and the lamps' bounce) also runs on the GPU,
`room_bake_gpu::bake` and `bake_lights` with a `Placement`: the same BVH,
ray sets, per-probe rotations, gathers and smoothing as compute passes
(`shaders/room_bake.wgsl`, eight storage buffers), agreeing with the CPU
bake to 1e-7 of the brightest value (`room_bake_gpu` test). A first pass
traces, per probe and bounce ray, which of the eight probes around its hit
see it and whether the sun and each lamp do, into one word (4 bytes per
probe and ray); every bounce and lamp then traces only the ray itself.
14,000 probes, 2,048 + 512 rays, 3 bounces: 0.16 s on an RTX 5070 Ti
against 1.3 s on the CPU. `room_bake_gpu::bake_geometry` places the probes
and bakes their visibility maps, reach and separation planes the same way
(positions to 1e-7 m, maps and reach bit for bit; a plane may take the
other of two equally near surfaces). `interior_rooms` bakes both door
states this way, 3.5 s at startup with the demo window running
(`ROOMS_CPU_BAKE=1`: the CPU, 22 s).

## Look scene

```
cargo run --release -p render --example interior_light -- room out-dir --rays 2048
```

Renders a closed 8 by 8 m room with a door at noon from outside, inside and
towards the door, with auto exposure, and prints the exposure of each view.
Other variants (`none`, `vis`, `diffuse`, `sun`, `sep`) show the older
`sky_occlusion` bakes for comparison.

## Not yet

- The GPU bake compiles its kernels per call (about 0.3 s each) and bakes
  each lamp in its own passes; the lamps are most of its time.
- No Luau call yet: games written in Luau cannot ask for a room bake.
- Probes are 0.5 m apart, so the bake has no contact darkening of its own.
  Corners, creases and the floor under furniture darken through the
  horizon-based screen-space AO in `shade.wgsl` (GTAO's idea, as Call of
  Duty uses it). Like any screen-space AO it misses what the frame does not
  show (a wall behind a table gets a little less).
- Moving lights (a swinging pendant, a flashlight) cannot be baked; their
  first bounce is gathered live (below).

## Live bounce of moving lights

A clustered light marked `ClusteredLight::with_bounce()` (it needs a shadow)
gets its first diffuse bounce live, from reflective shadow maps (Dachsbacher
and Stamminger 2005; `render::rsm`, `shaders/rsm.wgsl`). Use it for lights
whose bounce no bake holds: in `interior_rooms` the pendant and the
flashlight. The ceiling lamps' bounces are baked (`bake_lights`), so they
leave it off.

```rust
// Once, before the renderer: the surfaces over min..max as voxels, which
// the bounce is tested against so it does not pass through walls. The
// engine picks the voxel size.
scene.reserve_bounce_voxels(min, max)?;
// Each frame, after moving instances: the engine voxelises what moved.
renderer.update_bounce_voxels(&gpu, &scene)?;
renderer.set_clustered_lights(&[ClusteredLight::shadowed(torch, 20).with_bounce()])?;
```

- The light's shadow tiles also store each texel's albedo and normal
  (`raster.wgsl fs_rsm`, R32Uint: RGB 565 and octahedral 8:8, 4 MiB per
  shadow layer).
- `rsm_gather` runs before shade at a quarter of the view's resolution (an
  eighth from 1600 lines: 480x270 at 4K). Per pixel and bouncing light it
  takes 8 texels a frame, half uniformly over the tile and half on a disc
  around the pixel's own projection (density 1/r, where the 1/d^2 of near
  patches is), weighted by the mixture's pdf. A texel reflects albedo x
  (the light's irradiance there at normal incidence, as shade computes it,
  cookie and profile included) x d^2 x (its solid angle); the patch lights
  the pixel as a disc, E = L A cos cos / (r^2 + A / pi). No gain is tuned.
- Visibility: an exact walk (Amanatides and Woo) through `rsm::SceneVoxels`,
  a bit grid of the scene's opaque triangles, with a brick level of 4^3
  voxels skipped when empty. The voxelisation is thin (6-separating,
  Schwarz and Seidel 2010): a voxel whose centre lies within half a voxel
  of a triangle's plane along its dominant axis, so a flat surface fills one
  layer and a step from voxel face to voxel face cannot pass it. The voxel
  size is the engine's (`rsm::voxel_size`): the finest that keeps the grid
  within 8.4 million voxels (1 MiB of bits on the GPU, 16 MiB of counts on
  the CPU), at least 2 cm; 5 cm over `interior_rooms`. The walk starts and
  ends half a voxel plus half a voxel diagonal out along both normals, clear
  of the two surfaces' own voxels; a solid run it starts or ends in, no
  longer than that, counts as the neighbouring surface in a corner. The engine keeps the grid current: per
  voxel a count of triangles, so an instance that moved or changed mesh
  (the door, the pendant) takes its old triangles out and adds its new ones.
  Walks for texels worth less than the mean so far are taken with
  probability value / mean (at least 1 in 8) and weighted by its inverse
  (Russian roulette, unbiased).
- The gather is blended with last frame's (reprojected, 0.9 kept where the
  view depth matches within 4 %), then filtered: three a-trous passes
  (5x5 B3 spline, steps 1, 2, 4; Dammertz et al. 2010) weighted by how far
  each tap lies off the centre's plane. Shade adds it to the diffuse
  ambient through a depth-aware 2x2 upsample.
- Cost, `interior_rooms`, RTX 5070 Ti, 4K, flashlight on the white block in
  room 3: gather 0.70 ms, filter 0.18 ms with 16 texels on the first 10 cm
  grid; 8 texels on the 5 cm thin grid measured the same or less relative
  to shade, under load (log 2026-11-12-thin-voxels-probe-frames). The `--pan` flicker test matches
  the bounce off (mean frame-to-frame change 0.028 against 0.029).
- Checks (`interior_rooms.suite`): `leak_r3_ceil`, room 3's ceiling by the
  wall to room 2 with the pendant on, meters EV -2.6 as with the bounce off
  (-0.1 without the visibility walk). `ceiling_wall`: the flashlight on a
  wall 2.5 m away lights the ceiling above the spot to 3.6 % of the spot
  (linear, after tone mapping).
- `leak_door` (the flashlight in room 3 on the shut 4 cm door, the camera
  in dark room 4): 3 % of the leak without visibility (0.0025 against
  0.090 mean display luminance; 30 % with the first, conservative 10 cm
  grid). What remains comes from patches within 7 cm of the door.
- `KERN_RSM=0` turns it off, `KERN_RSM=open` drops the visibility walk.
