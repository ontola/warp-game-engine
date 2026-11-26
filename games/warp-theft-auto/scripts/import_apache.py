"""Convert Geibu3D's Attack Chopper OBJ from its CC BY 4.0 source ZIP."""
import hashlib
import io
import json
from pathlib import Path
import struct
import sys
from zipfile import ZipFile
from PIL import Image

OUT = Path(__file__).resolve().parents[1] / "assets" / "apache"
CREDIT = "Attack Chopper by Geibu3D (CC BY 4.0), adapted for Warp"
PIVOTS = {"body": (0., 0., 0.), "rotor": (0., 1.25, -.46), "tail-rotor": (-.22, 1.21, 4.25)}


def bake(archive):
    if hashlib.sha256(archive.read_bytes()).hexdigest() != "e4241f6d3d2f452be546bb55e91112abf040f9cc0ad70c3419f9335ffe6d173f":
        raise ValueError("Expected the Attack Chopper archive linked in CREDITS.md")
    OUT.mkdir(parents=True, exist_ok=True)
    verts, uvs, normals = [], [], []
    faces = {key: [] for key in PIVOTS}
    with ZipFile(archive) as z:
        tex = Image.open(io.BytesIO(z.read("Textures/Apache_Texture_Green.png"))).convert("RGB")
        tex.resize((512, 512), Image.Resampling.LANCZOS).save(OUT / "green.jpg", quality=85, optimize=True)
        part = "body"
        for line in z.read("OBJ/Attack_Helicopter.obj").decode().splitlines():
            row = line.split()
            if not row:
                continue
            if row[0] == "o":
                part = "tail-rotor" if row[1].startswith("Back_Propeller") else "rotor" if row[1].startswith("Propellers") else "body"
            elif row[0] == "v":
                x, y, zc = map(float, row[1:4])
                verts.append((-x, y-.96, -zc))
            elif row[0] == "vt":
                u, v = map(float, row[1:3])
                uvs.append((u, 1-v))
            elif row[0] == "vn":
                x, y, zc = map(float, row[1:4])
                normals.append((-x, y, -zc))
            elif row[0] == "f":
                poly = [tuple(int(i)-1 for i in item.split("/")) for item in row[1:]]
                for i in range(1, len(poly)-1):
                    faces[part].extend([poly[0], poly[i], poly[i+1]])
    for part, corners in faces.items():
        data = bytearray()
        views, attrs = [], []
        def add(values, fmt, shape, component, target, bounds=False):
            data.extend(bytes((-len(data)) % 4))
            offset = len(data)
            for value in values:
                data.extend(struct.pack("<" + fmt, *value))
            views.append({"buffer": 0, "byteOffset": offset, "byteLength": len(data)-offset, "target": target})
            access = {"bufferView": len(views)-1, "componentType": component, "count": len(values), "type": shape}
            if bounds:
                access.update(min=list(map(min,zip(*values))), max=list(map(max,zip(*values))))
            attrs.append(access)
            return len(attrs)-1
        lookup = {key: n for n,key in enumerate(dict.fromkeys(corners))}
        pivot = PIVOTS[part]
        pos = [tuple(verts[i][a] - pivot[a] for a in range(3)) for i,_,_ in lookup]
        primitive = {"attributes": {
            "POSITION": add(pos,"3f","VEC3",5126,34962,True),
            "NORMAL": add([normals[n] for _,_,n in lookup],"3f","VEC3",5126,34962),
            "TEXCOORD_0": add([uvs[t] for _,t,_ in lookup],"2f","VEC2",5126,34962)},
            "indices": add([(lookup[k],) for k in corners],"H","SCALAR",5123,34963), "material": 0}
        uri = part+".bin"
        doc = {"asset":{"version":"2.0","copyright":CREDIT}, "scene":0, "scenes":[{"nodes":[0]}],
               "nodes":[{"name":part,"mesh":0}], "meshes":[{"primitives":[primitive]}],
               "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"roughnessFactor":.78,"metallicFactor":.06}}],
               "images":[{"uri":"green.jpg"}], "textures":[{"source":0}], "accessors":attrs,
               "bufferViews":views,"buffers":[{"uri":uri,"byteLength":len(data)}]}
        (OUT / uri).write_bytes(data)
        (OUT / (part+".gltf")).write_text(json.dumps(doc,separators=(",",":"))+"\n",encoding="utf-8")
        print(part, len(corners)//3, "triangles,", len(data), "bytes")

if __name__ == "__main__":
    bake(Path(sys.argv[1]))
