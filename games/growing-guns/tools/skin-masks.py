"""Write the skin and hair masks of the Growing Guns characters.

A mask is a greyscale PNG beside the character (NAME_skin.png) at the size of
its albedo texture: 0 is hair, 128 skin, 255 any other material, and values
between blend linearly. Warp copies it into the albedo alpha of surfaces
listed in NAME.json's "skin_hair" entry (assets::skinned).

Texels count when a triangle that the head joint moves most covers them and
their colour reads as skin or hair; everything else stays 255. The rules per
character are below. Needs NumPy and Pillow:

    python games/growing-guns/tools/skin-masks.py [--preview DIR]
"""
import argparse
import json
import pathlib

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

ROOT = pathlib.Path(__file__).resolve().parents[1] / "assets" / "characters"
# Per character: albedo texture, the joints whose triangles may be skin or
# hair, and the UV box (u0, v0, u1, v1) that holds the centres of the face,
# scalp and ear islands (the head joint also moves the helmet pieces).
RULES = {
    "paladin": ("paladin_0.png", ("mixamorig_Head", "mixamorig_Neck"), (0.0, 0.60, 0.34, 0.83)),
}


def load_mesh(name):
    manifest = json.loads((ROOT / f"{name}.json").read_text())
    raw = (ROOT / manifest["mesh_file"]).read_bytes()
    n = manifest["vertex_count"]
    stride = manifest["vertex_stride_bytes"]
    vertices = np.frombuffer(raw[: n * stride], dtype="<f4").reshape(n, stride // 4)
    joints = vertices[:, 8:16].view("<u4")
    weights = vertices[:, 16:24]
    uv = vertices[:, 6:8]
    off = manifest["index_offset_bytes"]
    indices = np.frombuffer(raw[off: off + manifest["index_count"] * 4], dtype="<u4").reshape(-1, 3)
    names = [j["name"] for j in manifest["joints"]]
    return manifest, uv, joints, weights, indices, names


def head_triangles(uv, joints, weights, indices, names, allowed, box):
    allowed_ids = {i for i, n in enumerate(names) if n in allowed}
    keep = []
    for tri in indices:
        score = {}
        for v in tri:
            for j, w in zip(joints[v], weights[v]):
                score[int(j)] = score.get(int(j), 0.0) + float(w)
        best = max(score.items(), key=lambda kv: (kv[1], -kv[0]))[0]
        if best in allowed_ids:
            keep.append(tri)
    keep = np.array(keep)
    # UV islands: triangles joined through shared vertices.
    parent = {}
    def find(a):
        while parent.setdefault(a, a) != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a
    for tri in keep:
        for v in tri[1:]:
            parent[find(int(v))] = find(int(tri[0]))
    islands = {}
    for tri in keep:
        islands.setdefault(find(int(tri[0])), []).append(tri)
    out = []
    for tris in islands.values():
        centre = uv[np.array(tris).ravel()].mean(axis=0) % 1.0
        if box[0] <= centre[0] <= box[2] and box[1] <= centre[1] <= box[3]:
            out.extend(tris)
    return np.array(out)


def smoothstep(a, b, x):
    t = np.clip((x - a) / (b - a), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def classify(rgb):
    """Per texel: 1 for skin, 0 for hair, from colour (head islands only).

    The paladin's buzz cut is skin showing through stubble except where the
    hair is dense: dark and grey. Dark but saturated texels (the neck under
    the helmet) stay skin."""
    peak = rgb.max(axis=-1)
    low = rgb.min(axis=-1)
    saturation = (peak - low) / np.maximum(peak, 1e-3)
    hair = (1.0 - smoothstep(0.24, 0.34, peak)) * (1.0 - smoothstep(0.26, 0.36, saturation))
    return 1.0 - hair


def build(name, texture, joints_allowed, box, preview):
    manifest, uv, joints, weights, indices, names = load_mesh(name)
    image = Image.open(ROOT / texture).convert("RGB")
    w, h = image.size
    tris = head_triangles(uv, joints, weights, indices, names, set(joints_allowed), box)
    cover = Image.new("L", (w, h), 0)
    draw = ImageDraw.Draw(cover)
    for tri in tris:
        pts = [(float(uv[v, 0] % 1.0) * w, float(uv[v, 1] % 1.0) * h) for v in tri]
        draw.polygon(pts, fill=255)
    # Grow by a few texels so filtering at island edges keeps the mask.
    cover = cover.filter(ImageFilter.MaxFilter(7))
    covered = np.asarray(cover, dtype=np.float32) / 255.0
    rgb = np.asarray(image.filter(ImageFilter.GaussianBlur(2)), dtype=np.float32) / 255.0
    skin = classify(rgb)
    # 0 hair, 0.5 skin, 1 other.
    value = covered * (0.5 * skin) + (1.0 - covered)
    mask = Image.fromarray(np.clip(value * 255.0 + 0.5, 0, 255).astype(np.uint8), "L")
    mask.save(ROOT / f"{name}_skin.png", optimize=True)
    print(f"{name}: {len(tris)} head triangles, {int((covered > 0.5).sum())} masked texels")
    if preview:
        tint = np.asarray(image, dtype=np.float32) / 255.0
        m = np.asarray(mask, dtype=np.float32)[..., None] / 255.0
        hair = np.clip(1.0 - 2.0 * m, 0, 1)
        sk = 1.0 - np.abs(2.0 * m - 1.0)
        out = tint * 0.5 + hair * np.array([0.0, 0.5, 1.0]) * 0.5 + sk * np.array([1.0, 0.3, 0.0]) * 0.5
        Image.fromarray((np.clip(out, 0, 1) * 255).astype(np.uint8)).save(pathlib.Path(preview) / f"{name}_skin_preview.png")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--preview", help="write colour previews into this directory")
    args = parser.parse_args()
    for name, (texture, joints_allowed, box) in RULES.items():
        build(name, texture, joints_allowed, box, args.preview)


if __name__ == "__main__":
    main()
