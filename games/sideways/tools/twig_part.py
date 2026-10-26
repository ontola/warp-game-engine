#!/usr/bin/env python3
"""Bake the opaque twig part: the needle texture's alpha silhouette as
triangles, in the texture's own UV space.

Every crown frond of the three fir archetypes maps the whole of
authored_pine_albedo.png onto a bent eight-triangle card. This part is that
same twig as real geometry: tree_assets.rs places one copy per frond (136 per
tree) by mapping each part vertex from UV space onto its card, so the crown
needs no alpha test (SIDEWAYS_TREE_CROWN=opaque).

Output: assets/meshes/pine_twig_part.kmesh, one surface. Positions are
(u, v, 0), UVs are (u, v), normals +Z. Needs numpy, opencv-python, shapely>=2.1.

    python3 games/sideways/tools/twig_part.py            # default fidelity
    python3 games/sideways/tools/twig_part.py --res 512 --eps 1.2
"""
import argparse, struct
from pathlib import Path

import cv2
import numpy as np
import shapely
from shapely.geometry import Polygon
from shapely.validation import make_valid

GAME = Path(__file__).resolve().parents[1]
CUTOFF = 0.42  # raster.wgsl alpha_cut for the crown (view.extra.x)


def polygons(alpha, res, eps, cut=CUTOFF):
    """Opaque region of the alpha mask at `res`, as simplified polygons with
    holes, in pixel units of `res` (pixel i spans [i, i+1])."""
    a = cv2.resize(alpha.astype(np.float32) / 255.0, (res, res), interpolation=cv2.INTER_AREA)
    mask = (a >= cut).astype(np.uint8)
    contours, hierarchy = cv2.findContours(mask, cv2.RETR_CCOMP, cv2.CHAIN_APPROX_NONE)
    hierarchy = hierarchy[0]
    out = []
    ring = lambda c: cv2.approxPolyDP(c, eps, True)[:, 0, :].astype(np.float64) + 0.5
    for i, c in enumerate(contours):
        if hierarchy[i][3] != -1:
            continue
        shell = ring(c)
        if len(shell) < 3:
            continue
        holes, j = [], hierarchy[i][2]
        while j != -1:
            h = ring(contours[j])
            if len(h) >= 3:
                holes.append(h)
            j = hierarchy[j][0]
        geometry = make_valid(Polygon(shell, holes))
        for part in shapely.get_parts(geometry):
            pieces = [part] if part.geom_type == "Polygon" else list(getattr(part, "geoms", []))
            out += [p for p in pieces if p.geom_type == "Polygon" and p.area > 0.25]
    return mask, out


def triangulate(polys):
    tris = []
    for p in polys:
        for t in shapely.get_parts(shapely.constrained_delaunay_triangles(p)):
            tris.append(np.array(t.exterior.coords[:3]))
    return np.array(tris)


def coverage(mask, tris):
    r = np.zeros_like(mask)
    for t in tris:
        cv2.fillPoly(r, [np.round(t - 0.5).astype(np.int32)], 1)
    iou = (r & mask).sum() / max((r | mask).sum(), 1)
    return iou, r.sum() / max(mask.sum(), 1)


def write_kmesh(path, name, positions, normals, uvs, indices):
    b = bytearray(b"KMSH") + struct.pack("<I", 1)
    n = name.encode()
    b += struct.pack("<I", len(n)) + n + struct.pack("<II", len(positions), len(indices))
    b += np.asarray(positions, "<f4").tobytes() + np.asarray(normals, "<f4").tobytes()
    b += np.asarray(uvs, "<f4").tobytes() + np.asarray(indices, "<u4").tobytes()
    Path(path).write_bytes(bytes(b))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--res", type=int, default=384, help="mask resolution the silhouette is traced at")
    ap.add_argument("--eps", type=float, default=1.0, help="outline simplification, pixels at --res")
    ap.add_argument("--cut", type=float, default=0.3,
                    help="threshold on the area-averaged alpha at --res; below the card cutoff "
                         "keeps needles thinner than a --res texel")
    ap.add_argument("--out", default=str(GAME / "assets/meshes/pine_twig_part.kmesh"))
    args = ap.parse_args()
    image = cv2.imread(str(GAME / "assets/trees/authored_pine_albedo.png"), cv2.IMREAD_UNCHANGED)
    mask, polys = polygons(image[:, :, 3], args.res, args.eps, args.cut)
    tris = triangulate(polys)
    iou, ratio = coverage(mask, tris)
    # Weld identical corners so the part is indexed.
    flat = np.round(tris.reshape(-1, 2), 6)
    unique, index = np.unique(flat, axis=0, return_inverse=True)
    uv = unique / args.res
    positions = np.c_[uv, np.zeros(len(uv))]
    normals = np.tile([0.0, 0.0, 1.0], (len(uv), 1))
    indices = index.reshape(-1).astype(np.uint32)
    # Consistent winding: +Z facing in UV space (v grows downwards).
    t = uv[indices.reshape(-1, 3)]
    e1, e2 = t[:, 1] - t[:, 0], t[:, 2] - t[:, 0]
    flip = e1[:, 0] * e2[:, 1] - e1[:, 1] * e2[:, 0] < 0
    tri = indices.reshape(-1, 3)
    tri[flip] = tri[flip][:, [0, 2, 1]]
    write_kmesh(args.out, "pine_twig_part", positions, normals, uv, tri.reshape(-1))
    print(f"{args.out}: {len(tri)} triangles, {len(uv)} vertices; silhouette IoU {iou:.3f}, "
          f"area ratio {ratio:.3f} at {args.res}px, eps {args.eps}")


if __name__ == "__main__":
    main()
