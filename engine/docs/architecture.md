# Buildings (Warp Architecture)

`engine/crates/architecture` shows buildings inside and out: homes from
AtomicBuildings, and later IFC from architects. It reads glTF 2.0 only;
every source is turned into glTF first (AtomicBuildings' `exportHomeGlb`,
IfcOpenShell's `IfcConvert` for IFC).

Node `extras` it understands: `ifcClass` (`IfcWall`, `IfcWindow`,
`IfcSpace`, ...), `reviewState` (`proposed`, `confirmed`, `rejected`) and
`usage` (a space's use). Every node becomes its own mesh and instance
(`Element::instance`), so it can be hidden or restyled. From the tags
`House` derives:

- surfaces drawn in code (`surfaces.rs`, no image files): faces that look
  out of the building (four rays all leave it) get brick, a gravel roof on
  top; inside faces plaster; floors oak, or 0.6 m tiles in halls, kitchens
  and wet rooms (matched in `usage` or the space name, English and Dutch);
  frames anthracite. UVs are metres along each face, so sizes are real at
  any wall angle;
- proposed elements as a blue ghost (blended, so they do not block light in
  the bake), rejected ones left out;
- the sun for a place and time (`sun::Site`: latitude, longitude and the
  compass bearing of the plan's up; NOAA equations);
- the probe grid for the room bake ([interior-light.md](interior-light.md)),
  half-metre cells over the building, a quarter cell off whole-metre walls;
- a ground under the lowest floor;
- four outside 3/4 views and one standing view per `IfcSpace`, all with a
  level camera so verticals stay vertical.

```sh
cargo run --release -p architecture --example render_house -- house.glb out/ \
    [--rays 2048] [--size 1280x720] [--site LAT,LON,NORTH_DEG] [--when DAY,UTC_HOURS]
```

The plan (hosts, import paths, milestones) is in the project's
"Warp Architecture: plan" document.

Open: per-element material choice from the app (only defaults now), lamps,
furniture assets. AtomicBuildings' export has no roof class, slabs follow
the wall axes (dark seam between storeys) and walls end at their axes (a
notch at outside corners shows plaster end faces); fix those in the
exporter, not here.
