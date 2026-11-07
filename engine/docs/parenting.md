# Parenting and attachment points

## Luau entities

A parent gives a child a moving coordinate frame. The optional third argument
of `world.spawn(shape, options, parent)` is a live entity ID. The corresponding native runtime calls are `world.spawn(Spawn { parent, ..Default::default() })`
and `World::set_parent(id, parent)` in the `warp::world` module. Store IDs for
attachment points you want to use again. A socket may be an invisible,
non-solid entity, and counts towards `max_entities`.

```lua
local v = vector.create
local chassis = world.spawn("cube", {size = v(2, 1, 4)})
local lamp = world.spawn("sphere", {pos = v(0.8, 0.3, -1.8),
    size = 0.15, emissive = 1, solid = false}, chassis)
world.place(chassis, v(10, 0, 5), math.pi / 2)
local lightPosition = world.get(lamp).world_pos
```

- A root uses world coordinates. A child's `pos` and `rot` in `world.get`,
  `world.set` and `world.place` use its parent's frame. Angles are in radians.
  Rotation and translation propagate through all descendants immediately.
  Each entity owns its size in metres, visibility, solidity, colour and tag.
- `world.get(id).parent` is the parent ID or nil. `world_pos` is the resolved
  centre in world space. Raycasts, overlaps and `world.move` use world space.
  Collision uses the composed yaw and the existing upright-box approximation.
  A mover ignores its own ID. Use a query's `ignore` for other solid family members.
- `world.set_parent(id, parent)` keeps the numeric local position and
  rotation. The entity can jump when its frame changes. Pass nil to detach.
  Missing IDs and cycles raise an error before changing the hierarchy.
- Removing an entity removes its descendants, including invisible ones.
  Reparent a child first if it should survive. Changing visibility alone
  leaves descendants alone. Roots keep the existing entity API.
- Simulation queries see the current tick. Drawing uses the usual render-slot
  interpolation between the two resolved world poses. A child with a long
  offset follows the chord between tick positions when its parent turns.
- Spawn, place, reparent and remove from simulation hooks. Code reload keeps
  existing entities and links. Restart creates a fresh world.

See the [Neon Trails bike](../../games/neon-trails/main.luau) for a shell that
follows one moving head and disappears with it when the rider crashes.

## Rust model sockets

Construct `kern::parenting::ModelHierarchy::new(parents, sockets)` once per
model. Each entry in the parent vector is a node's parent index or `None`.
Multiple roots and forward references are supported. Construction reports
out-of-range indices and cycles. Multiple socket names may point at one node.

```rust
let tree = ModelHierarchy::new(vec![None, Some(0), Some(1)],
    [("gem".to_owned(), 2)].into())?;
if let Some(gem) = tree.node("gem") {
    let model = tree.model_transform(gem, |i| pose.get(&i).copied().unwrap_or(rest[i]));
    let light_position = (actor_world * model).transform_point3(Vec3::ZERO);
}
```

Use valid indices from the imported model or `node(name)`. `ancestors(i)`
includes the node and ends at its root. `model_transform` uses the local
matrix at each visited node, including root scale and any animation
overrides. Resolving one socket costs one walk up its chain and allocates
nothing. The game supplies the actor's world pose and may override a final
piece with a physics pose. Spellstorm uses the drawn gem position for a
fallen wizard so its light follows the ragdoll.

For an already allocated contiguous group of meshes, call
`Scene::update_children(first, parent_world, children)` with pairs of mesh ID
and local `Affine3A` in slot order. This writes `parent_world * local` for
each child, preserves slot flags, and returns the number written. Reserve
enough slots and hide any unused tail. Call again when the pose changes.
Sideways uses this for the suspended wheels and body. Growing Guns supplies
the same rig under distinct first- and third-person poses.

For a Sideways F12 capture, a pixel hit on a car returns its car index and
placed part name; rigid vegetation hits can carry a pose-to-root chain.
[Inspect a player-marked pixel](warp-cli.md#sideways-feedback).
