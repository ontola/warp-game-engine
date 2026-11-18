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
    [--rays 2048] [--size 1280x720] [--site LAT,LON,NORTH_DEG] [--when DAY,UTC_HOURS] [--night]
```

## Designs that say what they are made of (AtomicBuildings)

The contract is `glb-contract.md` in the AtomicBuildings project files.
A glTF material with `extras` (`pattern`, `category`, `moduleSize`,
`jointWidth`, or just `subject`) is drawn as that, per primitive, and the
position-and-name guesses above do not apply to it (per-side wall finishes
are separate primitives with their own materials). Materials without such
extras (IFC) behave as before.

- `Style` (`surfaces.rs`) is a surface laid out as the design asks:
  `planks` boards (staggered; the long side of `moduleSize` is the board's
  length, along v for a wall so vertical boards stay vertical), `slats`
  (open joints), `herringbone`, `running-bond` / `stack-bond` brick
  (module plus joint is the pitch), `random-stone` (Voronoi fieldstone),
  `ashlar` (courses of varied block length), `tiles`, `terrazzo` (chips),
  `board-formed` concrete. No `pattern`: the `category` (or, with none, the
  material name) picks wood veneer, stone, brick, concrete, plaster or
  lawn. Textile, metal, glass, ceramic, plastic, water and unknown names use
  the flat glTF colour, roughness and metalness; blended materials stay
  blended.
- Each look is drawn once per scene, neutral, at the module size (the repeat
  is a whole number of pieces, about 0.6 to 2 m, UVs in metres divided by
  the repeat), and each material tints it so the pieces (joints left out)
  average `baseColorFactor`; roughness and metalness come from the glTF.
- A primitive with a base-colour texture (a furniture model embedded in
  the GLB) keeps its material, UVs and normals; `assets::gltf` imports the
  texture, normal and metallic-roughness maps.
- `KHR_lights_punctual` point and spot lights become `House::lights`
  (node transforms and the house placement applied) and
  `House::clustered_lights()`: candela as the file gives it, colour, the
  spot cone, range 0.1 lux cut-off (30 m at most), all shadowed (the
  renderer's shadow budget decides how many keep a shadow). A scene turns
  `graphics.clustered_lights` on when `House::lights` is not empty;
  `render_house` and `house_view` do. Lamps carry direct light only: the
  room bake does not know them.
- glTF cameras become `House::cameras` (name, lens, `extras.viewTime`).
  `render_house` renders each as an extra shot named after its node. With
  no `--when`, a camera's `viewTime` sets the sun of that shot with
  `--site`; the room bake is redone for each distinct sun (minutes on CPU),
  shots are grouped by sun. `--when` or `--night` fix the sun for all.
  `house_view` lists the cameras after the room views.
- `--night`: the sun below the horizon (UTC midnight), no sun light, a dark
  sky, so the lamps carry the picture.

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

Surfaces follow the file where it names them: `ifc_to_glb.py` writes
`layers` (a build-up, outer to inner as IFC orders it; Revit's plain
material lists on walls, slabs, roofs and coverings count as one) and
`materials`. `Surface::from_material_name` reads English, Dutch and German
names (brick, plaster, concrete, aluminium, wood, tile, grass, roofing;
studs, insulation and air are skipped as unseen). A layered element's
outer finish goes on faces looking out, its inner finish inside; a face
whose front lies in a room volume (spaces carry `height`) is inside. Only
where nothing is named does the old guess (brick out, plaster in) apply.

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
`NAME.light.txt` for hosts that serve no binary files. The bake file holds the sky and sun transport, separation planes, DDGI visibility maps, probe offsets and reach (version 2, `architecture::baked::Baked`): without visibility, light leaked along every wall and ceiling edge of thin IFC walls. Without either the
tab bakes on its one thread: minutes for an IFC house (26 to 88 s on four
native cores at 2048 rays), so always ship the bake.

The plan (hosts, import paths, milestones) is in the project's
"Warp Architecture: plan" document.

Open: IFC colours are replaced by the defaults (every IFC wall turns
brick), pitched roofs get the flat-roof gravel, lamp light does not
bounce, one probe grid per
scene (coarse cells for a street). AtomicBuildings homes get a roof only
when the app has an IfcRoof element (ontola/atomic-buildings).
