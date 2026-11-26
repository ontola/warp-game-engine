"""Bake the game trees offline from Warp Vegetation species.

From the repository root run: python games/warp-theft-auto/scripts/bake_forest.py
The browser and native game load the resulting shared glTF meshes.
"""
import argparse
import json
import math
import os
import random
import re
import struct
import subprocess
import tempfile
from pathlib import Path

from bake_plants import Geometry, Model, TONES, between, blade, shoot, soil

GAME = Path(__file__).resolve().parents[1]
REPO = GAME.parents[1]
SWATCHES = {
    "pine": (("pine bark", (.19, .128, .072)), ("pine shadow", (.044, .100, .049)),
             ("pine leaf", (.062, .139, .073)), ("pine tips", (.098, .184, .091))),
    "fir": (("spruce bark", (.193, .139, .083)), ("spruce shadow", (.046, .102, .052)),
            ("spruce leaf", (.070, .145, .066)), ("spruce tips", (.100, .183, .083))),
    "birch": (("birch bark", (.50, .46, .34)), ("birch shadow", (.088, .152, .052)),
              ("birch leaf", (.151, .242, .075)), ("birch tips", (.182, .273, .100))),
    "street": (("street bark", (.22, .153, .093)), ("street shadow", (.075, .145, .045)),
               ("street leaf", (.117, .206, .058)), ("street tips", (.155, .248, .075))),
}
# Species, seed, development age and per-tree triangle caps (standard, phone).
# The existing root transforms in forest.module.luau vary size and heading.
RECIPES = {
    "pine": ("pine", 18, 14, 850, 115),
    "fir": ("spruce", 553, 15, 715, 115),
    "birch": ("birch", 142, 14, 680, 112),
    "street": ("broadleaf", 35, 14, 1500, 190),
}


def read_planting():
    source = (GAME / "forest.module.luau").read_text()
    number = r"(-?[\d.]+)"
    roots = re.findall(r"\{" + ",".join([number] * 4) + r',"([a-z]+)"\}', source)
    if not roots or any(row[-1] not in RECIPES for row in roots):
        raise ValueError("Unknown or empty forest planting")

    def dimensions(key):
        table = re.search(r"local " + key + r" = \{([^}]+)\}", source).group(1)
        return {kind: float(value) for kind, value in re.findall(r"([a-z]+)=" + number, table)}

    return roots, dimensions("radius"), dimensions("height")


def cook_trees(executable, radius, height):
    trees = {}
    with tempfile.TemporaryDirectory() as directory:
        for kind, (species, seed, age, regular, phone) in RECIPES.items():
            output = Path(directory) / (kind + ".json")
            subprocess.run([str(executable), species, str(seed), str(age), str(height[kind]),
                            str(radius[kind]), str(regular), str(phone), str(output)], check=True)
            tree = json.loads(output.read_text())
            metadata = {key: tree[key] for key in ("species", "botanical_name", "source", "age", "seed",
                                                   "height", "trunk_radius", "source_leaf_samples")}
            for tier, budget in (("standard", regular), ("phone", phone)):
                parts = tree[tier]
                count = sum(len(parts[i]["indices"]) // 3 for i in ("wood", "foliage"))
                vertices = [p for i in ("wood", "foliage") for p in parts[i]["positions"]]
                ymin, ymax = min(p[1] for p in vertices), max(p[1] for p in vertices)
                if (not 0 < count <= budget or abs(ymin) > 0.06 or
                        abs(ymax - height[kind]) > 0.04 * height[kind]):
                    raise ValueError(f"{kind}/{tier}: {count}/{budget} tris, Y range {ymin}..{ymax}")
            trees[kind] = (tree, metadata)
    return trees


def geometry(parts, kind):
    """Use the engine's indexed trunk, boughs and foliage; keep opaque surfaces single-sided."""
    mesh = Geometry()
    palette = SWATCHES[kind]
    TONES.update(palette)
    for key in ("wood", "foliage"):
        src = parts[key]
        points, normals, indices = src["positions"], src["normals"], src["indices"]
        if not points or len(normals) != len(points) or len(indices) % 3:
            raise ValueError(f"Invalid geometry on {kind} {key}")
        groups = {}
        for i in range(0, len(indices), 3):
            tone = 0 if key == "wood" else 1 + src["tones"][i // 3]
            groups.setdefault(palette[tone][0], []).append(indices[i:i+3])
        for tone, faces in groups.items():
            ids = dict.fromkeys(idx for face in faces for idx in face)
            remap = {idx: local for local, idx in enumerate(ids)}
            mesh.patch(tone, [points[idx] for idx in ids], [normals[idx] for idx in ids],
                       [tuple(remap[idx] for idx in face) for face in faces])
    return mesh


def understory():
    rng = random.Random(865)
    g = Geometry()
    for i in range(4):
        a = i*2.4
        x,z = math.cos(a),math.sin(a)
        run = rng.uniform(.68,1.08)
        rise = rng.uniform(.40,.79)
        root = (x*.04,.03,z*.04)
        bend = (x*.30*run,rise*.84,z*.30*run)
        end = (x*run,rise*.52,z*run)
        shoot(g,[root,bend,end],[.018,.012,.006],3,"sage")
        for j in range(4):
            t = (j+.45)/4
            p = between(bend,end,t)
            for hand in [-1,1]:
                direction = a+hand*1.02
                blade(g,p,(math.cos(direction),.20-t*.59,math.sin(direction)),.115*(1-.52*t),.29*(1-.56*t),"sage")
    return g


def trail_copse(source_name="forest-stand.glb"):
    # Reuse the indexed positions, normals and materials already downloaded with
    # forest-stand. The second glTF describes only a narrow arrangement of them.
    source = (GAME / "assets/plants" / source_name).read_bytes()
    json_bytes = struct.unpack_from("<I", source, 12)[0]
    doc = json.loads(source[20:20 + json_bytes])
    nodes = doc["nodes"]
    config = (GAME / "forest.module.luau").read_text().split("local trailRoots = {", 1)[1].split("\n}", 1)[0]
    roots = re.findall(r"\{(\d+),([-\d.]+),([-\d.]+)\}", config)
    assert len(roots) == 4
    doc["nodes"] = [dict(nodes[int(i)-1], translation=[float(x),0,float(z)]) for i,x,z in roots]
    doc["scenes"] = [{"nodes": list(range(len(roots)))}]
    doc["buffers"] = [{"uri": source_name, "byteLength": len(source)}]
    for view in doc["bufferViews"]:
        view["byteOffset"] += 28 + json_bytes
    return json.dumps(doc, separators=(",", ":")).encode()


def write_models(trees, roots, tier):
    low = tier == "phone"
    suffix = "-low" if low else ""
    output = GAME / "assets/plants"
    output.mkdir(parents=True, exist_ok=True)
    models = {kind: geometry(tree[tier], kind) for kind, (tree, _) in trees.items()}
    stand = Model()
    parts = {kind: stand.part(models[kind]) for kind in ("pine", "birch", "fir")}
    for x, z, size, heading, kind in roots:
        stand.place(parts[kind], (float(x), 0, float(z)), math.radians(float(heading)),
                    size=(float(size),) * 3)
    if not low:
        fern = stand.part(understory())
        rng = random.Random(3174)
        for _ in range(15):
            x, z = rng.uniform(-10.7, 10.7), rng.uniform(-10.7, 10.7)
            size = rng.uniform(.7, 1.30)
            stand.place(fern, (x, 0, z), rng.random() * math.tau, size=(size,) * 3)
    stand_name = "forest-stand" + suffix
    stand.save(stand_name, (1, 1, 1), {"tier": tier, "trees": [trees[k][1] for k in parts]})
    (output / ("trail-copse" + suffix + ".gltf")).write_bytes(trail_copse(stand_name + ".glb"))

    street = Model()
    street.place(street.part(models["street"]), (0, 0, 0))
    bed = Geometry()
    soil(bed, 1.12, 1.12)
    street.place(street.part(bed), (0, 0, 0), size=(.90, .875, .90))
    street.save("street-tree" + suffix, (1, 1, 1),
                {"tier": tier, "trees": [trees["street"][1]]})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baker", type=Path, help="Use an existing bake_woodland_tree executable")
    args = parser.parse_args()
    if args.baker:
        executable = args.baker.resolve(strict=True)
    else:
        subprocess.run(["cargo", "build", "--locked", "--release", "-p", "render", "--example",
                        "bake_woodland_tree"], cwd=REPO, check=True)
        executable = REPO / "target/release/examples" / ("bake_woodland_tree" + (".exe" if os.name == "nt" else ""))
    roots, radius, height = read_planting()
    trees = cook_trees(executable, radius, height)
    for tier in ("standard", "phone"):
        write_models(trees, roots, tier)
