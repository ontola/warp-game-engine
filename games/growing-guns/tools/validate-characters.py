#!/usr/bin/env python3
"""Validate the standalone character files; optionally render a CPU contact sheet.

Requires NumPy, and Pillow when --preview is used. No Godot, window, GPU, source
FBX, or renderer runtime is required. Bounds checks use every weighted vertex
in every clip frame, independently of the exporter.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import numpy as np

VERTEX = np.dtype([
    ("position", "<f4", (3,)), ("normal", "<f4", (3,)),
    ("uv", "<f4", (2,)), ("joints", "<u4", (8,)), ("weights", "<f4", (8,)),
])


def load(path: Path):
    doc = json.loads(path.read_text(encoding="utf-8-sig"))
    raw = (path.parent / doc["mesh_file"]).read_bytes()
    count = doc["vertex_count"]
    vertices = np.frombuffer(raw, VERTEX, count=count)
    indices = np.frombuffer(raw, "<u4", count=doc["index_count"], offset=count * 96)
    atlas = (path.parent / doc["animation_file"]).read_bytes()
    return doc, vertices, indices, atlas


def matrices(doc, atlas, clip):
    return np.frombuffer(atlas, "<f4", offset=clip["matrix_offset_bytes"],
                         count=clip["frames"] * doc["joint_count"] * 12).reshape(
                             clip["frames"], doc["joint_count"], 3, 4)


def skin(vertices, pose):
    position = np.zeros((len(vertices), 3), dtype=np.float64)
    normal = np.zeros_like(position)
    for slot in range(8):
        m = pose[vertices["joints"][:, slot]]
        weight = vertices["weights"][:, slot, None]
        position += (np.einsum("vij,vj->vi", m[:, :, :3], vertices["position"]) + m[:, :, 3]) * weight
        normal += np.einsum("vij,vj->vi", m[:, :, :3], vertices["normal"]) * weight
    normal /= np.maximum(np.linalg.norm(normal, axis=1, keepdims=True), 1e-20)
    return position, normal


def check(path: Path):
    doc, vertices, indices, atlas = load(path)
    assert doc["version"] == 1 and VERTEX.itemsize == doc["vertex_stride_bytes"] == 96
    assert doc["matrix_stride_bytes"] == 48 and doc["joint_count"] <= 256
    assert (path.parent / doc["mesh_file"]).stat().st_size == len(vertices) * 96 + len(indices) * 4
    assert doc["index_offset_bytes"] == len(vertices) * 96
    assert len(indices) % 3 == 0 and indices.max() < len(vertices)
    for field in ("position", "normal", "uv", "weights"):
        assert np.isfinite(vertices[field]).all(), field
    assert np.all(vertices["joints"] < doc["joint_count"])
    assert np.all(vertices["weights"] >= 0)
    assert np.max(np.abs(vertices["weights"].sum(axis=1) - 1)) < 2e-6
    assert np.max(np.abs(np.linalg.norm(vertices["normal"], axis=1) - 1)) < 2e-5
    first_vertex = first_index = 0
    for surface in doc["surfaces"]:
        assert surface["first_vertex"] == first_vertex and surface["first_index"] == first_index
        surface_indices = indices[first_index:first_index + surface["index_count"]]
        assert surface_indices.min() >= first_vertex
        assert surface_indices.max() < first_vertex + surface["vertex_count"]
        first_vertex += surface["vertex_count"]
        first_index += surface["index_count"]
        assert (path.parent / surface["material"]["albedo_texture"]).is_file()
        if surface["material"].get("normal_texture"):
            assert (path.parent / surface["material"]["normal_texture"]).is_file()
    assert first_vertex == len(vertices) and first_index == len(indices)
    tri = indices.reshape(-1, 3)
    p, n = vertices["position"][tri], vertices["normal"][tri].sum(axis=1)
    signs = np.einsum("ij,ij->i", np.cross(p[:, 1] - p[:, 0], p[:, 2] - p[:, 0]), n)
    assert np.count_nonzero(signs > 1e-9) > 20 * np.count_nonzero(signs < -1e-9)
    end = 0
    for clip in doc["clips"]:
        assert clip["matrix_offset_bytes"] == end
        frames = matrices(doc, atlas, clip)
        assert np.isfinite(frames).all()
        assert clip["frames"] == round(clip["duration"] * 30) + (not clip["looping"])
        lower, upper = np.full(3, np.inf), np.full(3, -np.inf)
        for pose in frames:
            points, normals = skin(vertices, pose)
            assert np.isfinite(points).all() and np.isfinite(normals).all()
            lower = np.minimum(lower, points.min(axis=0))
            upper = np.maximum(upper, points.max(axis=0))
        assert np.max(np.abs(lower - clip["bounds_min"])) < 2e-5, (clip["name"], lower)
        assert np.max(np.abs(upper - clip["bounds_max"])) < 2e-5, (clip["name"], upper)
        assert np.all(lower >= np.array(doc["bounds_min"]) - 2e-5)
        assert np.all(upper <= np.array(doc["bounds_max"]) + 2e-5)
        assert len(clip["sockets"]) == clip["frames"]
        # Every clip must contain actual motion, not repeated rest-pose frames.
        assert np.max(np.abs(frames[-2] - frames[0])) > 1e-3
        end += frames.nbytes
    assert len(atlas) == end
    print(f'{doc["name"]}: {len(vertices):,} vertices, {len(indices)//3:,} triangles, '
          f'{doc["joint_count"]} bindings, all clip bounds/weights/winding valid')
    return doc, vertices, indices, atlas


def render_tile(root, character, clip_index, size=(400, 540)):
    """Small z-buffered orthographic CPU rasterizer, original albedo UVs."""
    from PIL import Image
    doc, vertices, indices, atlas = character
    clip = doc["clips"][clip_index]
    frames = matrices(doc, atlas, clip)
    pose = frames[0 if clip_index == 0 else len(frames) // 3]
    points, normals = skin(vertices, pose)
    eye = np.array([2.2, 1.3, 4.5])
    forward = np.array([0, 1.05, 0]) - eye
    forward /= np.linalg.norm(forward)
    right = np.cross(forward, [0, 1, 0]); right /= np.linalg.norm(right)
    up = np.cross(right, forward)
    rel = points - [0, 1.15, 0]
    scale = 195
    projected = np.column_stack((rel @ right * scale + size[0] / 2,
                                 -rel @ up * scale + size[1] / 2,
                                 rel @ forward))
    pixels = np.full((size[1], size[0], 3), [38, 44, 54], dtype=np.uint8)
    depth = np.full((size[1], size[0]), np.inf)
    light = np.array([-0.35, 0.8, 0.6]); light /= np.linalg.norm(light)
    for surface in doc["surfaces"]:
        texture = np.asarray(Image.open(root / surface["material"]["albedo_texture"]).convert("RGB"))
        tint = np.array(surface["material"]["albedo"][:3])
        surf = indices[surface["first_index"]:surface["first_index"] + surface["index_count"]]
        for triangle in surf.reshape(-1, 3):
            p = projected[triangle]
            a, b, c = p[:, :2]
            denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1])
            if abs(denominator) < 1e-8:
                continue
            xmin = max(0, int(np.floor(p[:, 0].min())))
            xmax = min(size[0] - 1, int(np.ceil(p[:, 0].max())))
            ymin = max(0, int(np.floor(p[:, 1].min())))
            ymax = min(size[1] - 1, int(np.ceil(p[:, 1].max())))
            if xmax < xmin or ymax < ymin:
                continue
            yy, xx = np.mgrid[ymin:ymax + 1, xmin:xmax + 1]
            xx = xx + .5; yy = yy + .5
            wa = ((b[1] - c[1]) * (xx - c[0]) + (c[0] - b[0]) * (yy - c[1])) / denominator
            wb = ((c[1] - a[1]) * (xx - c[0]) + (a[0] - c[0]) * (yy - c[1])) / denominator
            wc = 1 - wa - wb
            weights = np.stack((wa, wb, wc), axis=-1)
            z = weights @ p[:, 2]
            zslice = depth[ymin:ymax + 1, xmin:xmax + 1]
            visible = (weights.min(axis=-1) >= -1e-6) & (z < zslice)
            if not visible.any():
                continue
            bary = weights[visible]
            uv = bary @ vertices["uv"][triangle]
            tx = np.floor((uv[:, 0] % 1) * texture.shape[1]).astype(int)
            ty = np.floor((uv[:, 1] % 1) * texture.shape[0]).astype(int)
            ns = bary @ normals[triangle]
            ns /= np.maximum(np.linalg.norm(ns, axis=1, keepdims=True), 1e-20)
            lighting = .45 + .55 * np.maximum(0, ns @ light)
            color = texture[ty, tx] * tint * lighting[:, None]
            pixels[ymin:ymax + 1, xmin:xmax + 1][visible] = np.clip(color, 0, 255).astype(np.uint8)
            zslice[visible] = z[visible]
    return Image.fromarray(pixels)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, default=Path(__file__).resolve().parents[1] / "assets/characters")
    parser.add_argument("--preview", type=Path)
    args = parser.parse_args()
    characters = [check(args.assets / f"{name}.json") for name in ("knight", "paladin", "ch10")]
    provenance_path = args.assets / "provenance.json"
    if provenance_path.is_file():
        provenance = json.loads(provenance_path.read_text(encoding="utf-8-sig"))
        for entry in provenance["generated_files"]:
            assert hashlib.sha256((args.assets / entry["path"]).read_bytes()).hexdigest() == entry["sha256"], entry["path"]
        print("All generated file SHA-256 hashes match provenance.json")
    if args.preview:
        from PIL import Image, ImageDraw
        sheet = Image.new("RGB", (1200, 1650), (38, 44, 54))
        draw = ImageDraw.Draw(sheet)
        for column, character in enumerate(characters):
            for row, clip in enumerate(character[0]["clips"]):
                sheet.paste(render_tile(args.assets, character, row), (column * 400, row * 550 + 10))
                draw.text((column * 400 + 14, row * 550 + 12), character[0]["name"] + " / " + clip["name"], fill=(235, 239, 244))
        args.preview.parent.mkdir(parents=True, exist_ok=True)
        sheet.save(args.preview)
        print(f"CPU-rendered preview: {args.preview}")


if __name__ == "__main__":
    main()
