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

## IFC (BIM)

`tools/ifc_to_glb.py` (in the crate; `pip install ifcopenshell`) turns an
IFC file into a GLB with the same extras: one node per element (class,
GUID), one primitive per surface style (more than 20 % transparent becomes
`glass`), each space as its floor only, with its LongName as `usage`.
Openings, the site and annotations are left out. Tested on the KIT
FZK-Haus (Archicad, IFC4), the Revit Duplex (IFC2x3) and the Xbim
SampleHouse (IFC4); converted copies are in the project files
(`architecture/samples/`), the IFC files are not in the repository.

## Several buildings

`House::load_at` places a house by the middle of its footprint and turns
it; heights are kept (files put the ground floor at 0). A scene has one
probe grid of at most 65536 probes, so `architecture::probe_grid` over all
houses grows its cell from 0.5 m until it fits (1 m for four houses on a
66 m street; the bake took 45 s on CPU at 2048 rays).

```sh
cargo run --release -p architecture --example street -- out/ house.glb fzk-haus.glb duplex.glb:90 [--rooms 2]
```

Surfaces are chosen per patch (triangles sharing edges with normals within
30 degrees), so a wall face cut around a window stays one surface; curved
patches (round reveals) get smooth normals and triplanar textures. Meshes
are welded and built with `SimplificationPolicy::TexturedGeometry`: loose
triangles made the LOD simplifier fall back to sloppy clustering, which
shattered a round window frame a few metres away. The ground sits at
`House::grade`, the underside of the ground floor's slab, never in a
floor's plane. `ifc_to_glb.py` cuts slab holes (stairwells) out of room
floors (needs shapely).

`house_view --cam px,py,pz,tx,ty,tz --shot out.png` renders a fixed spot
for regression stills.

Room shots skip spaces where a standing eye has no ceiling within 6 m
(roof terraces, the eye above an attic's roof).

## Walking through, natively and in a browser

`house_view` shows one house: Left/Right jump between four outside views
and one per room, WASD and the mouse move, Up/Down change height.

```sh
cargo run --release -p architecture --example house_view -- house.glb [--shot out.png] [--view N]
# bake the light once, on a machine with cores, and ship it with the model
cargo run --release -p architecture --example house_view -- house.glb --bake-out house.light.bin
python3 tools/build-web.py --package architecture --example house_view --glb house.glb
```

The page loads `?house=NAME` or `#NAME` from `assets/NAME.json` (glTF with
an embedded buffer) or `NAME.glb`, and its light from `NAME.light.bin`
(`architecture::baked`, checked against the grid and sun), else base64
`NAME.light.txt` for hosts that serve no binary files. Without either the
tab bakes on its one thread: minutes for an IFC house (26 to 88 s on four
native cores at 2048 rays), so always ship the bake.

The plan (hosts, import paths, milestones) is in the project's
"Warp Architecture: plan" document.

Open: per-element material choice from the app (only defaults now), IFC
colours are replaced by the defaults (every IFC wall turns brick), pitched
roofs get the flat-roof gravel, lamps, furniture assets, one probe grid per
scene (coarse cells for a street). AtomicBuildings homes get a roof only
when the app has an IfcRoof element (ontola/atomic-buildings).
