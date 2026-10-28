# Physics

Warp has two layers of physics, and a game uses whichever it needs:

- `kern::collide`: simple, deterministic collision for characters and
  game rules, with no dependencies beyond glam.
- the `physics` crate: rigid bodies on [Rapier](https://rapier.rs) with
  enhanced determinism, for things that tumble, fly and stack.

Sideways' cars use Rapier directly in their own simulation crate
(`games/sideways/sim/src/physics.rs`): suspension, tyres, engine and
drivetrain at 120 Hz with four substeps. That vehicle model is the game's,
not an engine API yet.

## `kern::collide`

Level geometry as boxes rotated about the vertical axis (`YawBox`), which
covers most arena and level pieces, plus rays, spheres and an upright
cylinder character.

- `YawBox::new(center, size, yaw)`, `push_out`, `ray`, `top`.
- `raycast(boxes, origin, dir, max)` and `ray_sphere` for hits.
- `Mover { radius, half_height }.slide(boxes, &mut pos, &mut vel, dt,
  max_step)`: Godot's `move_and_slide` for boxes. Sub-steps of at most
  `max_step` metres, up to four push-out passes each; velocity loses its
  component into walls, floors and ceilings; small ledges are stepped up.
  It returns a `Contact` with `on_floor`, `on_ceiling` and the last
  `wall_normal`.
- The `Boxes` trait is what movers and rays query. A plain slice has no
  broadphase, which is fine for a few hundred boxes. `BoxGrid` is a uniform
  XZ grid for thousands: `remove` and `restore` boxes without a rebuild,
  with stable indices, plus `query` and `in_sphere`. Visits are in
  increasing index order, so results are the same whatever the broadphase
  (a test checks this).

Yaw convention: positive yaw turns +X towards -Z (right-handed, Y up), the
same as `glam::Quat::from_rotation_y`.

Growing Guns runs all of its gameplay collision on this: players, bullets
and a destructible castle of 6,000 to 9,400 chunks in a `BoxGrid`, with
the bots' simulation at about 0.07 ms per tick in release.

## The `physics` crate

```rust
use glam::Vec3;
use physics::{BodyDesc, Physics, Shape};

let mut p = Physics::new(Vec3::new(0.0, -9.81, 0.0), 60.0);   // gravity, tick rate
p.set_statics(&level_boxes);                                  // kern::collide::YawBox
let crate_body = p.add(&BodyDesc { vel: Vec3::X * 3.0, ..BodyDesc::new(Shape::Box(Vec3::splat(0.5)), Vec3::Y * 4.0) });
p.blast(Vec3::ZERO, 6.0, 12.0);                                // radius m, speed m/s at the centre
p.step();                                                      // once per tick
let (pos, rot) = p.pose(crate_body).unwrap();
```

- Static colliders are the same `YawBox`es the character mover uses:
  `add_static`, `remove_static`, `set_statics`.
- Dynamic bodies: `Shape::Box(half_extents)`, `Shape::Sphere(radius)`,
  `Shape::Capsule(half_height, radius)`, with density, friction,
  restitution, damping and CCD in `BodyDesc`.
- `impulse`, `blast` (a radial push that adds velocity whatever the mass,
  falling to zero at the edge, with a little lift), `ball_joint` between
  two bodies, `raycast` (which also tells whether it hit a body).
- `step()` advances one fixed tick, split into `substeps` if set.
  `pose`, `velocity`, `sleeping`, `body_count`, `clear_bodies`.
- The crate's test checks that boxes fall, settle and replay identically.

Growing Guns uses it on the presentation side (`app/src/view/`): corpses
that tumble, ragdolls and gibs. Its deterministic gameplay stays on
`kern::collide`.

## Not there yet

- A collision layer for simple games beyond boxes: no meshes or
  heightfields in `kern::collide`.
- A general vehicle API; Sideways' car model lives in the game.
- Jointed character ragdolls from skinned characters; Growing Guns'
  corpses tumble as whole models.
