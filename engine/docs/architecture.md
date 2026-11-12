# Buildings (Warp Architecture)

`engine/crates/architecture` shows buildings inside and out: homes from
AtomicBuildings, and later IFC from architects. It reads glTF 2.0 only;
every source is turned into glTF first (AtomicBuildings' `exportHomeGlb`,
IfcOpenShell's `IfcConvert` for IFC).

Node `extras` it understands: `ifcClass` (`IfcWall`, `IfcWindow`,
`IfcSpace`, ...) and `reviewState` (`proposed`, `confirmed`). From them
`House` derives:

- the probe grid for the room bake ([interior-light.md](interior-light.md)),
  half-metre cells over the building, a quarter cell off whole-metre walls;
- a ground under the lowest floor;
- four outside 3/4 views and one standing view per `IfcSpace`.

```sh
cargo run --release -p architecture --example render_house -- house.glb out/ [--rays 2048] [--size 1280x720]
```

The plan (hosts, import paths, milestones) is in the project's
"Warp Architecture: plan" document.

Open: elements are one merged mesh (no per-element swap or styling yet),
proposed elements look like confirmed ones, no material library, sun not
from a real address and date, AtomicBuildings' export has no roof class.
