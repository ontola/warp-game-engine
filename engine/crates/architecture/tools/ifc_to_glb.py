#!/usr/bin/env python3
"""IFC to glTF (GLB) for Warp Architecture.

    pip install ifcopenshell shapely
    python3 ifc_to_glb.py house.ifc house.glb

Every building element becomes one glTF node (world coordinates, IFC's
Z-up turned into glTF's Y-up: (x, y, z) -> (x, z, -y)) with the extras the
`architecture` crate reads: ifcClass, ifcGuid, name, layers (a layered
build-up's material names) or materials (other material names), and for
spaces usage (the space's LongName) and height (of its volume). One primitive per surface style; a style that is
more than 20 % transparent is named "glass" and blended. A space is
exported as its floor only (its downward faces turned up, 2 mm above the
slab), like the AtomicBuildings export, so rooms get a floor finish and a
camera without their volume hiding everything; holes in the slab under it
(stairwells) are cut out of that floor. Openings, the site and other
non-physical entities are left out.
"""
import json
import math
import multiprocessing
import struct
import sys

import ifcopenshell
import ifcopenshell.geom
import ifcopenshell.util.element
import shapely
from shapely.geometry import Polygon

SKIP = {"IfcOpeningElement", "IfcSite", "IfcAnnotation", "IfcVirtualElement", "IfcGrid", "IfcSpatialZone"}


def convert(src, dst):
    ifc = ifcopenshell.open(src)
    settings = ifcopenshell.geom.settings()
    settings.set("use-world-coords", True)
    it = ifcopenshell.geom.iterator(settings, ifc, multiprocessing.cpu_count())
    if not it.initialize():
        raise SystemExit(f"{src}: no geometry")
    nodes, meshes, materials, accessors, views = [], [], [], [], []
    material_index = {}
    blob = bytearray()

    def add_view(data, target):
        while len(blob) % 4:
            blob.append(0)
        views.append({"buffer": 0, "byteOffset": len(blob), "byteLength": len(data), "target": target})
        blob.extend(data)
        return len(views) - 1

    def material(style):
        transparency = style.transparency if not math.isnan(style.transparency) else 0.0
        rgb = list(style.diffuse.components) if style.diffuse else [0.7, 0.7, 0.7]
        glass = transparency > 0.2
        key = (style.name, tuple(round(c, 3) for c in rgb), glass)
        if key not in material_index:
            m = {"name": "glass" if glass else style.name,
                 "pbrMetallicRoughness": {"baseColorFactor": rgb + [1.0 - transparency if glass else 1.0],
                                          "metallicFactor": 0.0, "roughnessFactor": 0.1 if glass else 0.8}}
            if glass:
                m["alphaMode"] = "BLEND"
                m["doubleSided"] = True
            material_index[key] = len(materials)
            materials.append(m)
        return material_index[key]

    def emit(element, cls, by_style, more=None):
        primitives = []
        for key, tris in by_style.items():
            flat = [c for t in tris for p in t for c in p]
            nrm = [c for t in tris for c in normal(t) * 3]
            count = len(flat) // 3
            lo = [min(flat[i::3]) for i in range(3)]
            hi = [max(flat[i::3]) for i in range(3)]
            accessors.append({"bufferView": add_view(struct.pack(f"<{len(flat)}f", *flat), 34962), "componentType": 5126,
                              "count": count, "type": "VEC3", "min": lo, "max": hi})
            p_acc = len(accessors) - 1
            accessors.append({"bufferView": add_view(struct.pack(f"<{len(nrm)}f", *nrm), 34962), "componentType": 5126,
                              "count": count, "type": "VEC3"})
            prim = {"attributes": {"POSITION": p_acc, "NORMAL": len(accessors) - 1}}
            if key == "floor":
                if "floor" not in material_index:
                    material_index["floor"] = len(materials)
                    materials.append({"name": "floor", "pbrMetallicRoughness": {"baseColorFactor": [0.6, 0.45, 0.3, 1], "metallicFactor": 0, "roughnessFactor": 0.6}})
                prim["material"] = material_index["floor"]
            elif key is not None:
                prim["material"] = key
            primitives.append(prim)
        if primitives:
            meshes.append({"primitives": primitives})
            extras = {"ifcClass": cls, "ifcGuid": element.GlobalId}
            layers, names = material_names(element)
            if layers:
                extras["layers"] = layers
            elif names:
                extras["materials"] = names
            extras.update(more or {})
            if cls == "IfcSpace" and (element.LongName or element.Name):
                extras["usage"] = element.LongName or element.Name
            nodes.append({"name": element.Name or cls, "mesh": len(meshes) - 1, "extras": extras})

    skipped = 0
    # Holes in slabs (stairwells) as plan outlines with their height range,
    # and spaces' floors: a floor is the space's footprint minus the holes
    # of the slab it stands on, else it covers the stairwell.
    holes, spaces = [], []
    while True:
        shape = it.get()
        element = ifc.by_id(shape.id)
        cls = element.is_a()
        g = shape.geometry
        v = g.verts
        pos = [(v[i], v[i + 2], -v[i + 1]) for i in range(0, len(v), 3)]
        faces = [tuple(g.faces[i:i + 3]) for i in range(0, len(g.faces), 3)]
        if cls == "IfcOpeningElement" and any(r.RelatingBuildingElement.is_a("IfcSlab") for r in element.VoidsElements):
            outline = plan_outline([[pos[k] for k in f] for f in faces])
            if outline is not None:
                ys = [p[1] for p in pos]
                holes.append((outline, min(ys), max(ys)))
        if cls in SKIP:
            skipped += 1
        elif cls == "IfcSpace":
            floor = [[pos[k] for k in f] for f in faces]
            ys = [p[1] for p in pos]
            floor = [t for t in floor if normal(t)[1] < -0.9]
            if floor:
                spaces.append((element, floor, max(ys) - min(ys)))
        else:
            by_style = {}
            for f, mid in zip(faces, g.material_ids):
                tri = [pos[k] for k in f]
                key = material(g.materials[mid]) if mid >= 0 and mid < len(g.materials) else None
                by_style.setdefault(key, []).append(tri)
            emit(element, cls, by_style)
        if not it.next():
            break
    for element, floor, height in spaces:
        y = sum(p[1] for t in floor for p in t) / (3 * len(floor))
        outline = plan_outline(floor)
        for hole, lo, hi in holes:
            if lo - 0.3 <= y <= hi + 0.3:
                outline = outline.difference(hole)
        tris = []
        for t in shapely.constrained_delaunay_triangles(outline).geoms if not outline.is_empty else []:
            a, b, c = [(x, y + 0.002, z) for x, z in list(t.exterior.coords)[:3]]
            tris.append([a, b, c] if normal([a, b, c])[1] > 0 else [a, c, b])
        if tris:
            emit(element, "IfcSpace", {"floor": tris}, {"height": round(height, 4)})
    doc = {"asset": {"version": "2.0", "generator": "warp architecture ifc_to_glb.py",
                     "extras": {"source": src.split("/")[-1], "schema": ifc.schema}},
           "scene": 0, "scenes": [{"nodes": list(range(len(nodes)))}], "nodes": nodes, "meshes": meshes,
           "materials": materials, "accessors": accessors, "bufferViews": views, "buffers": [{"byteLength": len(blob)}]}
    while len(blob) % 4:
        blob.append(0)
    doc["buffers"][0]["byteLength"] = len(blob)
    js = json.dumps(doc, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    out = struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(js) + 8 + len(blob)) + struct.pack("<II", len(js), 0x4E4F534A) + js \
        + struct.pack("<II", len(blob), 0x004E4942) + bytes(blob)
    open(dst, "wb").write(out)
    print(f"{dst}: {len(nodes)} elements, {len(materials)} materials, {skipped} skipped, {len(out) // 1024} KiB")


def material_names(element):
    """(layers, materials): a layered build-up's material names in IFC order
    (from the layer set's reference side), else the element's other
    material names (a list, constituents or a single material)."""
    m = ifcopenshell.util.element.get_material(element)
    if m is None:
        return [], []
    if m.is_a("IfcMaterialLayerSetUsage"):
        m = m.ForLayerSet
    if m.is_a("IfcMaterialLayerSet"):
        return [l.Material.Name for l in m.MaterialLayers if l.Material and l.Material.Name], []
    if m.is_a("IfcMaterial"):
        return [], [m.Name] if m.Name else []
    if m.is_a("IfcMaterialList"):
        names = [x.Name for x in m.Materials if x.Name]
        # Revit writes a wall's or slab's build-up as a plain list.
        if element.is_a("IfcWall") or element.is_a("IfcSlab") or element.is_a("IfcRoof") or element.is_a("IfcCovering"):
            return names, []
        return [], names
    if m.is_a("IfcMaterialConstituentSet"):
        return [], [c.Material.Name for c in m.MaterialConstituents or [] if c.Material and c.Material.Name]
    if m.is_a("IfcMaterialProfileSet"):
        return [], [p.Material.Name for p in m.MaterialProfiles if p.Material and p.Material.Name]
    return [], []


def plan_outline(tris):
    """The union of triangles seen from above, as a shapely (multi)polygon in (x, z)."""
    parts = [Polygon([(p[0], p[2]) for p in t]) for t in tris]
    parts = [q for q in parts if q.is_valid and q.area > 1e-8]
    return shapely.unary_union(parts).buffer(0) if parts else None


def normal(t):
    a, b, c = t
    u = [b[i] - a[i] for i in range(3)]
    v = [c[i] - a[i] for i in range(3)]
    n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
    length = math.sqrt(sum(x * x for x in n)) or 1.0
    return [x / length for x in n]


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    convert(sys.argv[1], sys.argv[2])
