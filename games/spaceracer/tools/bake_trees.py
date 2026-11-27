"""Bake Space Racer's forest trees offline from Warp Vegetation species.

From the repository root run: python games/spaceracer/tools/bake_trees.py
Writes games/spaceracer/assets/trees/<name>.glb (standard) and <name>-low.glb
(phone tier). The stage loads them as shared static meshes: closed, opaque
crowns with no alpha-tested cards, so they stay solid at any distance and on
phone GPUs, and cost a few hundred triangles each.

The glTF writer and the bark/leaf swatches are Warp Theft Auto's
(games/warp-theft-auto/scripts/bake_forest.py), which bakes its stands the same way.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

GAME = Path(__file__).resolve().parents[1]
REPO = GAME.parents[1]
sys.path.insert(0, str(REPO / "games/warp-theft-auto/scripts"))
import bake_plants  # noqa: E402
from bake_forest import SWATCHES, geometry  # noqa: E402

OUT = GAME / "assets/trees"
# name: (bake species, swatch, seed, age, height m, trunk radius m, standard tris, phone tris).
# The stage scales every tree to its own height, so only proportions matter.
# Order is the stage's model index: "tree0".."tree3" in rules/stages.luau.
TREES = {
    "spruce-a": ("spruce", "fir", 553, 15, 12.2, .29, 900, 320),
    "birch-a": ("birch", "birch", 142, 14, 9.77, .22, 900, 320),
    "spruce-b": ("spruce", "fir", 911, 16, 12.8, .30, 900, 320),
    "birch-b": ("birch", "birch", 377, 13, 9.4, .21, 900, 320),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baker", type=Path, help="Use an existing bake_woodland_tree executable")
    args = parser.parse_args()
    if args.baker:
        exe = args.baker.resolve(strict=True)
    else:
        subprocess.run(["cargo", "build", "--locked", "--release", "-p", "render", "--example",
                        "bake_woodland_tree"], cwd=REPO, check=True)
        exe = REPO / "target/release/examples" / ("bake_woodland_tree" + (".exe" if os.name == "nt" else ""))
    OUT.mkdir(parents=True, exist_ok=True)
    bake_plants.OUT = OUT
    with tempfile.TemporaryDirectory() as tmp:
        for name, (species, swatch, seed, age, height, radius, regular, phone) in TREES.items():
            path = Path(tmp) / (name + ".json")
            subprocess.run([str(exe), species, str(seed), str(age), str(height), str(radius),
                            str(regular), str(phone), str(path)], check=True)
            tree = json.loads(path.read_text())
            meta = {k: tree[k] for k in ("species", "botanical_name", "source", "age", "seed", "height", "trunk_radius")}
            for tier, suffix in (("standard", ""), ("phone", "-low")):
                model = bake_plants.Model()
                model.place(model.part(geometry(tree[tier], swatch)), (0, 0, 0))
                model.save(name + suffix, (1, 1, 1), {"tier": tier, "trees": [meta]})
                tris = sum(len(tree[tier][k]["indices"]) // 3 for k in ("wood", "foliage"))
                print(f"{name}{suffix}: {tris} triangles")


if __name__ == "__main__":
    main()
