#!/usr/bin/env python3
"""IFC to glTF (GLB) for Warp Architecture.

    pip install ifcopenshell
    python3 ifc_to_glb.py house.ifc house.glb

Every building element becomes one glTF node (world coordinates, IFC's
Z-up turned into glTF's Y-up: (x, y, z) -> (x, z, -y)) with the extras the
`architecture` crate reads: ifcClass, ifcGuid, name, and for spaces usage
(the space's LongName). One primitive per surface style; a style that is
more than 20 % transparent is named "glass" and blended. A space is
exported as its floor only (its downward faces turned up, 2 mm above the
slab), like the AtomicBuildings export, so rooms get a floor finish and a
camera without their volume hiding everything. Openings, the site and other
non-physical entities are left out.
"""
import json
import math
import multiprocessing
import struct
import sys

import ifcopenshell
import ifcopenshell.geom

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

    skipped = 0
    while True:
        shape = it.get()
        element = ifc.by_id(shape.id)
        cls = element.is_a()
        if cls in SKIP:
            skipped += 1
        else:
            g = shape.geometry
            v = g.verts
            pos = [(v[i], v[i + 2], -v[i + 1]) for i in range(0, len(v), 3)]
            faces = [tuple(g.faces[i:i + 3]) for i in range(0, len(g.faces), 3)]
            by_style = {}
            for f, mid in zip(faces, g.material_ids):
                tri = [pos[k] for k in f]
                if cls == "IfcSpace":
                    n = normal(tri)
                    if n[1] > -0.9:
                        continue  # only the floor
                    tri = [(p[0], p[1] + 0.002, p[2]) for p in reversed(tri)]
                    key = "floor"
                else:
                    key = material(g.materials[mid]) if mid >= 0 and mid < len(g.materials) else None
                by_style.setdefault(key, []).append(tri)
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
                if cls == "IfcSpace" and (element.LongName or element.Name):
                    extras["usage"] = element.LongName or element.Name
                nodes.append({"name": element.Name or cls, "mesh": len(meshes) - 1, "extras": extras})
        if not it.next():
            break
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
