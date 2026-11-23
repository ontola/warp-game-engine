"""Cook the small Growing Guns blood meshes used by the Luau city (requires Pillow)."""
import io
import json
import math
import struct
from pathlib import Path

from PIL import Image

GAME = Path(__file__).resolve().parents[1]
OUTPUT = GAME / "assets" / "effects"
SOURCE = GAME.parent / "growing-guns" / "assets" / "blood-splats.png"


def unit(point):
    length = math.hypot(*point)
    return tuple(x / length for x in point) if length else (0, 1, 0)


def mesh(name, points, normals, indices, color, roughness, texture=None):
    blob = bytearray()
    views, accessors = [], []

    def view(data):
        blob.extend(bytes(-len(blob) % 4))
        views.append(dict(buffer=0, byteOffset=len(blob), byteLength=len(data)))
        blob.extend(data)
        return len(views) - 1

    def accessor(rows, kind, fmt, field, **kwargs):
        raw = b"".join(struct.pack("<" + fmt, *row) for row in rows)
        accessors.append(dict(bufferView=view(raw), count=len(rows), componentType=kind, type=field, **kwargs))
        return len(accessors) - 1

    attr = {"POSITION": accessor(points, 5126, "3f", "VEC3", min=list(map(min, zip(*points))), max=list(map(max, zip(*points)))),
            "NORMAL": accessor(normals, 5126, "3f", "VEC3")}
    pbr = dict(baseColorFactor=[*(x**2.2 for x in color), 1], metallicFactor=0, roughnessFactor=roughness)
    material = {"pbrMetallicRoughness": pbr}
    doc = dict(asset=dict(version="2.0", generator="Warp bake_combat_fx.py"), scene=0, scenes=[{"nodes": [0]}],
               nodes=[{"mesh": 0}], materials=[material], bufferViews=views, accessors=accessors)
    if texture:
        image, uv = texture
        attr["TEXCOORD_0"] = accessor(uv, 5126, "2f", "VEC2")
        png = io.BytesIO()
        image.save(png, format="PNG", optimize=True)
        doc.update(images=[{"bufferView": view(png.getvalue()), "mimeType": "image/png"}], textures=[{"source": 0}])
        pbr["baseColorTexture"] = {"index": 0}
        material.update(alphaMode="MASK", alphaCutoff=0.5)
    idx = accessor([(i,) for i in indices], 5123, "H", "SCALAR")
    doc["meshes"] = [{"primitives": [{"attributes": attr, "indices": idx, "material": 0}]}]
    doc["buffers"] = [{"byteLength": len(blob)}]
    blob.extend(bytes(-len(blob) % 4))
    head = json.dumps(doc, separators=(",", ":")).encode()
    head += b" " * (-len(head) % 4)
    result = struct.pack("<4sII", b"glTF", 2, 28 + len(head) + len(blob))
    result += struct.pack("<I4s", len(head), b"JSON") + head + struct.pack("<I4s", len(blob), b"BIN\0") + blob
    (OUTPUT / (name + ".glb")).write_bytes(result)


def drop():
    # Growing Guns app/src/blood.rs::drop_mesh, four rings and seven meridians.
    points, normals, indices = [], [], []
    for j in range(5):
        lat = math.pi * j / 4
        for i in range(7):
            lon = math.tau * i / 7
            q = (math.cos(lon) * math.sin(lat), math.cos(lat), math.sin(lon) * math.sin(lat))
            taper = 0.66 - 0.46 * q[2]
            scale = (0.64 * taper, 0.73 * taper, 1.36)
            points.append(tuple(a * b for a, b in zip(q, scale)))
            normals.append(unit(tuple(a / b for a, b in zip(q, scale))))
            if j < 4:
                a, b = j * 7 + i, j * 7 + (i + 1) % 7
                if j > 0:
                    indices.extend([a, b, a + 7])
                if j < 3:
                    indices.extend([b, b + 7, a + 7])
    mesh("blood-drop", points, normals, indices, (0.62, 0.09, 0.11), 0.4)


def shard():
    # The sheared/tapered outline of stadium/gibs.rs::unit_shard, with a faceted cut face.
    points, normals, indices = [], [], []
    rings = [(-.5, .32), (-.24, .51), (.22, .44), (.48, .27)]
    for y, radius in rings:
        taper = .80 - .52 * y
        for i in range(6):
            angle = math.tau * i / 6
            x, z = math.cos(angle), math.sin(angle)
            r = radius * (1 + .16 * math.sin(i * 19 + y * 6))
            points.append((r*x*taper-.24*y, y+.06*math.sin(i*4), r*z*taper))
            normals.append(unit((x, y*2.3, z)))
    for j in range(3):
        for i in range(6):
            a, b = j * 6 + i, j * 6 + (i + 1) % 6
            indices.extend((a, a+6, b, b, a+6, b+6))
    for i in range(1, 5):
        indices.extend((0, i, i+1, 18, 18+i+1, 18+i))
    mesh("blood-shard", points, normals, indices, (0.55, 0.09, 0.07), 0.83)


def splat():
    # An actual cutout from the source game's cooked directional footprint atlas.
    atlas = Image.open(SOURCE)
    tile = atlas.width // 4
    image = atlas.crop((0, 0, tile, tile)).resize((128, 128), Image.Resampling.LANCZOS)
    points = [(-.5,0,-.5), (-.5,0,.5), (.5,0,.5), (.5,0,-.5)]
    mesh("blood-splat", points, [(0,1,0)]*4, [0,1,2,0,2,3], (1,1,1), 0.36,
         (image, [(0,0),(0,1),(1,1),(1,0)]))


if __name__ == "__main__":
    OUTPUT.mkdir(parents=True, exist_ok=True)
    drop()
    shard()
    splat()
