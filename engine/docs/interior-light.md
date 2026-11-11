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

// Probes every 0.5 m over the building, starting a quarter cell off the
// walls and reaching below the floor (outside the grid the outdoor light
// fades back in over half a cell).
let grid = render::irradiance_volume::Grid {
    origin: Vec3::new(-5.75, -0.25, -5.75), cell: 0.5, dims: [24, 9, 24] };
scene.reserve_irradiance_volume(grid)?;
let renderer = Renderer::new(&gpu, &scene, &light);

let occluders = Occluders::from_scene(&scene, |_| true);
let probes = room_bake::bake(&grid, &occluders, light.sun_dir, &Settings::default())?;
renderer.set_sky_sun_separated(&gpu, &probes, &room_bake::separation_planes(&grid, &occluders))?;
```

`Settings`: `sky_rays` (2048; a door seen from the back of a room is a few
hundredths of the sphere, fewer rays make blotches), `bounce_rays` (512),
`bounces` (3) and `reflectance` (0.5, the average albedo).

Grid: a cell of 0.5 m against walls of 0.2 to 0.3 m works. The grid must
cover the room, its walls and a cell outside, and reach below the floor.
One volume per scene, up to 65,536 probes.

## Cost

The bake runs on the CPU's cooking threads: 3.2 s for the look scene
(5,184 probes, 2,048 + 512 rays, 3 bounces) on 4 cloud cores. Shading reads
eight probes per pixel inside the grid.

## Look scene

```
cargo run --release -p render --example interior_light -- room out-dir --rays 2048
```

Renders a closed 8 by 8 m room with a door at noon from outside, inside and
towards the door, with auto exposure, and prints the exposure of each view.
Other variants (`none`, `vis`, `diffuse`, `sun`, `sep`) show the older
`sky_occlusion` bakes for comparison.

## Not yet

- The bake is on the CPU and takes seconds; a GPU bake would make it
  interactive and allow larger buildings.
- No Luau call yet: games written in Luau cannot ask for a room bake.
- Probes are 0.5 m apart: light changes smoothly, so contact darkening where
  furniture meets the floor and in corners has to come from a screen-space
  pass (GTAO in Call of Duty), which Warp does not have in full yet.
- Local lights (lamps) are not part of this bake; `irradiance_volume::BoxRoom`
  still bakes those for box rooms.
