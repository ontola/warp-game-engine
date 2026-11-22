# Physics

Warp has two layers of physics, and a game uses whichever it needs:

- `kern::collide`: simple, deterministic collision for characters and
  game rules, with no dependencies beyond glam.
- the `physics` crate: rigid bodies on [Rapier](https://rapier.rs) with
  enhanced determinism, for things that tumble, fly and stack.

## Four-wheel vehicles

Sideways and Warp Theft Auto use the same `physics::vehicle::Car`. It casts
four tyres, integrates spring and damper loads, and solves combined tyre slip,
clutch, differential and gearing at four substeps per 120 Hz physics tick.
A Rust game supplies a `CarSpec`, `Ground` and `Surfaces` to the solver.
Sideways supplies its editable rally specs and stage surfaces.
`physics::vehicle::fleet::Fleet` provides a shared road world for multiple
drivers. Parked cars remain collidable and resume dynamics when occupied.

A Luau game calls `vehicles.reset("city")` after spawning its level. This
captures the solid boxes with that tag into a Rapier spatial index. Omitting
the tag captures all current solids. Reset starts a fresh fleet. The level
snapshot and vehicles live until reset or `world.clear()`. Each game can spawn
up to its configured world entity capacity in the fleet.

```lua
local car = vehicles.spawn(vector.create(10, 0, 5), 0) -- ground point and yaw
vehicles.drive(car, { throttle = 1, steer = .2 })      -- positive steers right
vehicles.step(dt)                                      -- once for all drivers
local pose = vehicles.get(car)
world.set(chassis, { pos = pose.pos, rot = pose.rot })
```

`vehicles.drive` accepts throttle, brake and handbrake in 0–1, plus steer in
−1–1. Passing nil parks the car when it comes to rest. `step` accepts simulation
seconds in 0–1 and accumulates them into 120 Hz steps. `get` returns the
axle-height origin, YXZ Euler rotation in Warp’s usual (pitch, yaw, roll) vector,
signed forward speed, gear, RPM, engine torque in Nm, ignition cut and tyre
slip. For each front-left, front-right, rear-left and rear-right wheel, its
vector contains local centre height, steering angle and spin angle in radians.

## `kern::collide`

Level geometry as boxes rotated about the vertical axis (`YawBox`), which
covers most arena and level pieces, plus rays, spheres and an upright
cylinder character.

- `YawBox::new(center, size, yaw)`, `push_out`, `ray`, `top`.
- `raycast(boxes, origin, dir, max)` and `ray_sphere` for hits.
- `Mover { radius, half_height }.slide(boxes, &mut pos, &mut vel, dt,
  max_step)`: Godot's `move_and_slide` for boxes. Sub-steps of at most
  `max_step` metres, up to four push-out passes each; velocity loses its
  component into walls, floors and ceilings.
  It returns a `Contact` with `on_floor`, `on_ceiling` and the last
  `wall_normal`.
- `slide_with_step(..., step_height, grounded)` opts into climbing low
  obstacles when ordinary movement is blocked. It requires real ground
  support, clear headroom and a supported landing; jumping and airborne
  movement retain ordinary sliding. The height budget applies to the whole
  call. Growing Guns uses 0.35 m. Existing `slide` callers are unchanged.
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
- `ragdoll::Ragdoll` builds a connected capsule tree from world-space
  `Segment` endpoints and joint anchors. Each rig has a unique collision-group
  bit: its limbs ignore one another but collide with the world and other rigs.
  After each fixed step, call `update`; `delta(part, alpha)` interpolates the
  rigid transform from that part's initial pose. Apply it to the initial world
  skin matrix, then the current instance inverse, to deform an existing skin.
  `remove` releases the bodies and joints. The game owns the skeleton mapping;
  the physics API has no dependency on a particular model or animation format.

Growing Guns uses it on the presentation side (`app/src/view/`): corpses
that tumble, ragdolls and gibs. Its deterministic gameplay stays on
`kern::collide`.

## Not there yet

- A collision layer for simple games beyond boxes: no meshes or
  heightfields in `kern::collide`.
- A general vehicle API; Sideways' car model lives in the game.
- Measured anatomical profiles, coupled swing/twist envelopes and skin-aware dismemberment for humanoid rigs.


## Passive ragdoll joints

`Ragdoll::set_angular_profile` adds three independent angular limits and
force-based damped motors. Supply parent/child local joint frames calibrated
against an authored reference skeleton, not the pose at death. An
`AngularProfile` supplies hard min/max angles, a separate neutral target,
soft-zone width, weak rest stiffness, edge stiffness, damping and a torque cap.
`Physics::step` updates the motors before each substep. It combines the neutral
spring with an edge spring whose stiffness rises quadratically over the soft
zone; the solver still enforces the hard boundary. Motor forces do not wake a
sleeping body on their own. Removing a rig also removes its passive controls.

The Growing Guns profile is game tuning, not measured anatomical data. Its
model assets contain bind transforms and animation, **no** joint ROM or muscle
rest metadata. Bind-relative skin rotations calibrate the joint frames so that
dying mid-run does not redefine neutral. Knees/elbows allow mainly flexion;
hips, shoulders, neck and spine have wider three-axis motion. Rest springs are
weak and torque-limited so gravity and contacts can win. The skin is never
snapped to a neutral pose. The primitive fallback retains its old physics.

The reference is a relaxed, slightly flexed posture, informed by the concept of
[NASA neutral body posture](https://www.nasa.gov/image-article/neutral-body-posture-specifications/),
not a literal microgravity posture forced on a body lying on the floor. Neutral
is not necessarily the arithmetic midpoint of a joint's motion range.
[Rapier joints](https://rapier.rs/docs/user_guides/templates/joints/) provide
limits and motors; the progressive response is implemented here.

Limitations: three independent angular coordinates approximate anatomy; they
are not a coupled shoulder swing cone or a musculoskeletal model. Self-collision
within one corpse is still disabled. Hand/foot articulation, active balance,
protective reflexes and injury-dependent muscle tone are not modeled. Hard
limits are numerical solver constraints and may transiently overshoot under
large impulses. Tuning and tests do not establish anatomical accuracy.


Validation (31 October 2026): 13 physics unit tests pass, including progressive
response at both edges, return to neutral after a torque impulse, limit
overshoot tolerance and exact repeatability. The character test loads all three
roster assets, preserves the initial world skin pose, and steps ten seconds
onto a floor; lifecycle cleanup also passes. A 255-tick headless gameplay
fixture captures alive/death/falling/bending/settled at 960x540 with an explicit
open arena (`games/growing-guns/scripts/character-ragdoll.json`). Static captures
verify presentation at those times, not continuous motion quality, worst-case
performance, every death mode, multiplayer or Quest hardware.


`Physics::raycast_bodies` tests a finite segment against explicitly supplied
body handles at current collider poses, including bodies created before the
next physics step. It returns the nearest body, contact and normal; stale
handles and shapes containing the segment origin are skipped. The caller must
clip the segment against scenery first. Growing Guns uses collision-checked
`BulletPath` events for this and calls `impulse(body, impulse, Some(contact))`
to wake and rotate a hit corpse. Bodies remain cosmetic and penetrable.
