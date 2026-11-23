"""Bake the Toyama's authored panels into standard named glTF nodes.

Reorder only the body's existing index streams, once for all paint variants.
The wreck descriptors reuse those geometry buffers and carry only new materials.
Run with --check to verify the shipped files against Sideways' CC BY source.
"""
import argparse
import hashlib
import json
import struct
from collections import defaultdict
from pathlib import Path

GAME = Path(__file__).resolve().parents[1]
CARS = GAME / "assets/cars"
SOURCE = GAME.parent / "sideways/assets/cars/toyama_xr.glb"
NAMES = ["frame", "bonnet", "door-lf", "door-rf", "door-lr", "door-rr", "boot"]

# Connected islands in the source paint's indexed topology, in face order.
# UV/normal splits preserve the artist's folds and the returns inside each seam.
PAINT = {
    1: {16, 192, 194, 195, 196, 197, 200, 205},
    2: {8, 10, 35, 38, 39, 40, 41, 44, 45, *range(87, 96), *range(209, 217)},
    3: {110, 112, 130, 133, 134, 135, 136, 137, 138, *range(173, 183), *range(217, 225)},
    4: {5, 9, 32, 33, 42, *range(96, 104)},
    5: {107, 111, 127, 128, *range(183, 192)},
    6: {*range(19, 28), *range(117, 124), 193, 198, 199, 201, 202, 203, 204, 206, 207},
}


def islands(triangles, keys):
    parent = list(range(len(triangles)))

    def root(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    seen = {}
    for i, triangle in enumerate(triangles):
        for vertex in triangle:
            other = seen.setdefault(keys[vertex], i)
            parent[root(i)] = root(other)
    groups = defaultdict(list)
    for i in range(len(triangles)):
        groups[root(i)].append(i)
    return sorted(groups.values(), key=lambda group: group[0])


def assign(material, component, center):
    x, y, z = center
    if material == 0:
        for part, components in PAINT.items():
            if component in components:
                return part
    if material == 1:
        return {3: 2, 4: 4, 6: 3, 7: 5}.get(component, 0)
    if material == 4 and component == 5 or material == 7 and component in (9, 10):
        return 1  # scoop insert and washer nozzles
    if material == 6 and component in (0, 1):
        return 2 if x < 0 else 3  # mirror glass
    if material == 5 and component in (10, 11):
        # The two rubber window surrounds share an island at the B-pillar.
        return (2 if x < 0 else 3) + (2 if z > .28 else 0)
    return 0


def unpack(doc, data, index, fmt):
    a = doc["accessors"][index]
    view = doc["bufferViews"][a["bufferView"]]
    start = view.get("byteOffset", 0) + a.get("byteOffset", 0)
    return list(struct.iter_unpack(fmt, data[start:start+a["count"]*struct.calcsize(fmt)]))


def add_inserts(meshes, views, accessors):
    """Add the surfaces visible behind open panels using the existing trim materials."""
    from math import sqrt
    data = bytearray()
    batches = defaultdict(list)

    def face(part, material, a, b, c, d):
        ab = [j-i for i, j in zip(a, b)]
        ac = [j-i for i, j in zip(a, c)]
        normal = [ab[1]*ac[2]-ab[2]*ac[1], ab[2]*ac[0]-ab[0]*ac[2], ab[0]*ac[1]-ab[1]*ac[0]]
        length = sqrt(sum(n*n for n in normal))
        batches[part, material].append(([a,b,c,d], [n/length for n in normal]))

    def box(part, material, lo, hi):
        x,y,z=lo
        X,Y,Z=hi
        bottom=[(x,y,z),(X,y,z),(X,y,Z),(x,y,Z)]
        top=[(x,Y,z),(X,Y,z),(X,Y,Z),(x,Y,Z)]
        face(part,material,*bottom)
        face(part,material,*reversed(top))
        for i in range(4):
            j=(i+1)%4
            face(part,material,bottom[j],bottom[i],top[i],top[j])

    # An inset engine bay and a shallow boot well. Keep the chassis rails at
    # the edges of the opening so the remaining shell has a readable rim.
    box(0, 7, (-.64,.04,-1.98), (.63,.12,-.98))
    box(0, 5, (-.32,.12,-1.72), (.25,.35,-1.16))
    for z in (-1.63,-1.49,-1.35):
        box(0, 5, (-.29,.35,z), (.22,.38,z+.046))
    box(0, 7, (.31,.12,-1.68), (.53,.31,-1.36))
    box(0, 5, (-.64,.12,-1.96), (-.59,.32,-.98))
    box(0, 5, (.58,.12,-1.96), (.63,.32,-.98))
    box(0, 7, (-.60,.12,1.50), (.59,.18,2.02))
    box(0, 5, (-.60,.18,1.50), (-.55,.53,2.02))
    box(0, 5, (.54,.18,1.50), (.59,.53,2.02))
    box(0, 5, (-.60,.18,1.50), (.59,.53,1.55))
    # Upholstery tapers into the original headrests. The source exterior has
    # only the portions visible above the beltline of the closed body.
    def pad(x, bottom_y, top_y, z, bottom_size, top_size, recline):
        def ring(y, center_z, width, depth):
            w,d=width*.5,depth*.5
            bevel=min(.055,w*.22,d*.26)
            return [(x+u,y,center_z+k) for u,k in (
                (-w+bevel,-d),(w-bevel,-d),(w,-d+bevel),(w,d-bevel),
                (w-bevel,d),(-w+bevel,d),(-w,d-bevel),(-w,-d+bevel))]
        bottom=ring(bottom_y,z,*bottom_size)
        top=ring(top_y,z+recline,*top_size)
        for i in range(8):
            j=(i+1)%8
            face(0,8,bottom[j],bottom[i],top[i],top[j])
        # Cover the octagonal crown with a centre strip and two side strips.
        for a,b,c,d in ((0,1,4,5),(1,2,3,4),(5,6,7,0)):
            face(0,8,top[d],top[c],top[b],top[a])

    box(0,8,(-.71,-.055,-.80),(.70,-.005,1.26))
    box(0,8,(-.105,-.005,-.73),(.10,.21,.28))
    box(0,8,(-.65,.35,-.81),(.64,.53,-.55))
    for x in (-.40,.40):
        pad(x,.07,.265,-.055,(.41,.51),(.48,.54),-.01)
        pad(x,.215,.68,.205,(.47,.16),(.42,.145),.11)
    pad(0,.075,.30,.78,(1.18,.42),(1.25,.50),-.015)
    pad(0,.24,.625,1.035,(1.26,.15),(1.17,.13),.08)
    # A dark underside for each lid and an inward-facing door card. These
    # move with the detached skin; mirrors, handles and windows are original.
    face(1,7,(-.58,.42,-1.87),(.57,.42,-1.87),(.62,.56,-1.02),(-.63,.56,-1.02))
    face(6,7,(-.54,.64,1.65),(.53,.64,1.65),(.51,.60,2.0),(-.52,.60,2.0))
    for part in range(2,6):
        side = -1 if part in (2,4) else 1
        front = part in (2,3)
        a, b = (-.74,.20) if front else (.27,1.07)
        # The rear lower edge follows the wheel arch.
        edge = b if front else .87
        corners = [(side*.765,.03,a+.06),(side*.765,.03,edge),(side*.735,.57,b),(side*.735,.56,a)]
        if side < 0:
            corners.reverse()
        face(part,5,*corners)
        x0,x1 = sorted((side*.72,side*.76))
        box(part,5,(x0,.30,a+.22),(x1,.355,a+.63))

    def attribute(values, fmt, kind, width, component, target, bounds=False):
        while len(data)%4: data.append(0)
        start=len(data)
        for row in values:
            data.extend(struct.pack(fmt,*row))
        view=len(views)
        views.append({"buffer":1,"byteOffset":start,"byteLength":len(data)-start,"target":target})
        a={"bufferView":view,"componentType":component,"count":len(values),"type":kind}
        if bounds:
            a["min"],a["max"]=[[func(row[i] for row in values) for i in range(width)] for func in (min,max)]
        accessors.append(a)
        return len(accessors)-1

    for (part, material), faces in batches.items():
        positions,normals,indices=[],[],[]
        for corners, n in faces:
            k=len(positions)
            positions.extend(corners)
            normals.extend([n]*4)
            indices.extend((k+i,) for i in (0,1,2,0,2,3))
        pos=attribute(positions,"<3f","VEC3",3,5126,34962,True)
        nrm=attribute(normals,"<3f","VEC3",3,5126,34962)
        idx=attribute(indices,"<H","SCALAR",1,5123,34963)
        prim={"attributes":{"POSITION":pos,"NORMAL":nrm},"indices":idx,"material":material}
        if part:
            # A loose panel has an interior face. Its simplified liner must
            # never poke through the continuously deformed exterior skin.
            prim["extras"]={"warpDetachedOnly":True}
        meshes[part]["primitives"].append(prim)
    return data


def wreck_descriptor(model):
    # The same shell, crushed geometry and detachable parts can all share one
    # set of burn materials, irrespective of the paint the vehicle started with.
    doc = {k: v for k, v in model.items() if k not in ("images", "textures", "samplers")}
    materials = []
    for original in model["materials"]:
        name = original["name"]
        material = {"name": name, "doubleSided": original.get("doubleSided", False)}
        if name == "Body_Glass":
            material["alphaMode"] = "BLEND"
            color, rough, metal = [.024, .030, .030, .54], .56, .02
        else:
            if name == "Body_Paint":
                color, rough, metal = [.047, .039, .033, 1], .90, .09
            elif name in ("Body_Chrome", "Body_Alloy"):
                color, rough, metal = [.077, .071, .064, 1], .78, .32
            else:
                color, rough, metal = [.021, .020, .019, 1], .95, .02
            material["extras"] = {**original.get("extras", {}), "warpSurface": 220}
        material["pbrMetallicRoughness"] = {
            "baseColorFactor": color, "roughnessFactor": rough, "metallicFactor": metal,
        }
        materials.append(material)
    doc["materials"] = materials
    return (json.dumps(doc, separators=(",", ":")) + "\n").encode()


def bake():
    glb = SOURCE.read_bytes()
    size = struct.unpack_from("<I", glb, 12)[0]
    original = json.loads(glb[20:20+size])
    source = glb[28+size:]
    model = json.loads((CARS / "sedan.gltf").read_bytes())
    packed = bytearray((CARS / "toyama.bin").read_bytes())
    meshes = [{"primitives": []} for _ in NAMES]
    accessors = model["accessors"][:32]
    for m, prim in enumerate(original["meshes"][0]["primitives"]):
        positions = unpack(model, packed, 4*m, "<3f")
        indices = [i for (i,) in unpack(original, source, prim["indices"], "<H")]
        triangles = list(zip(indices[::3], indices[1::3], indices[2::3]))
        keys = range(len(positions)) if m == 0 else [tuple(round(v, 5) for v in p) for p in positions]
        groups = islands(triangles, keys)
        expected = (225, 10, 2, 4, 15, 21, 5, 16)
        if len(groups) != expected[m]:
            raise ValueError(f"Re-author panel assignment for updated material {m}")
        ids = [0] * len(triangles)
        for component, group in enumerate(groups):
            for t in group:
                center = [sum(positions[i][axis] for i in triangles[t])/3 for axis in range(3)]
                ids[t] = assign(m, component, center)
        view = accessors[4*m+3]["bufferView"]
        start = model["bufferViews"][view].get("byteOffset", 0)
        count = 0
        for part, mesh in enumerate(meshes):
            chunk = [i for tri, owner in zip(triangles, ids) if owner == part for i in tri]
            if not chunk:
                continue
            mesh["primitives"].append({"attributes": {"POSITION": 4*m, "NORMAL": 4*m+1, "TEXCOORD_0": 4*m+2},
                                       "indices": len(accessors), "material": m})
            accessors.append({"bufferView": view, "byteOffset": count*2, "componentType": 5123,
                              "count": len(chunk), "type": "SCALAR"})
            struct.pack_into("<"+"H"*len(chunk), packed, start+count*2, *chunk)
            count += len(chunk)
        assert count == len(indices)
    yield CARS / "toyama.bin", packed
    views = model["bufferViews"][:32]
    inserts = add_inserts(meshes, views, accessors)
    yield CARS / "toyama-inserts.bin", inserts
    for path in sorted(CARS.glob("sedan*.gltf")):
        doc = json.loads(path.read_bytes())
        doc["scenes"] = [{"nodes": list(range(len(NAMES)))}]
        doc["nodes"] = [{"name": name, "mesh": i, "extras": {"warpPart": i}} for i, name in enumerate(NAMES)]
        doc["materials"] = doc["materials"][:8] + [{"name":"Cabin_Charcoal","pbrMetallicRoughness":{
            "baseColorFactor":[.075,.083,.088,1],"roughnessFactor":.94,"metallicFactor":0}}]
        doc["meshes"], doc["accessors"], doc["bufferViews"] = meshes, accessors, views
        doc["buffers"] = [doc["buffers"][0], {"uri":"toyama-inserts.bin","byteLength":len(inserts)}]
        yield path, (json.dumps(doc, separators=(",", ":"))+"\n").encode()
        if path.name == "sedan.gltf":
            yield CARS / "wreck.gltf", wreck_descriptor(doc)
    for side in ("left", "right"):
        wheel = json.loads((CARS / f"wheel-{side}.gltf").read_bytes())
        yield CARS / f"wreck-wheel-{side}.gltf", wreck_descriptor(wheel)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    for path, content in bake():
        if args.check:
            if path.read_bytes() != content:
                raise SystemExit(f"{path.name} needs rebaking")
        else:
            path.write_bytes(content)
        print(path.name, len(content), hashlib.sha256(content).hexdigest()[:12])
