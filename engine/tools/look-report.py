#!/usr/bin/env python3
"""How a render-suite run of tools/look-suite.json looks, as numbers and a report.

    render-suite.py run --suite engine/tools/look-suite.json --label after --arg=--voxel-raster-profile
    look-report.py after --base before --out build/look-reports/after

Prints per view: mean linear crown colour (tree series, with the ratio to the
nearest view), the strength of the grass grid (autocorrelation of dark pixels at
the spacing in the view's "grid_px"), straight lines in views with "lines" and the
largest voxel cell. With --out it
writes report.json and a 960 px JPEG per view (and per base view) for a web page.
Standard library only; JPEGs need Pillow, ffmpeg, ImageMagick or, from WSL,
Windows PowerShell.
"""
import argparse
import json
import re
import shutil
import statistics
import struct
import subprocess
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GROUPS = [
    ("selection", ("GPU forest selection residency", "GPU forest materialize", "GPU forest shared poses", "foliage seed")),
    ("voxel walk", ("voxel splats",)),
    ("splat raster", ("voxel splat raster", "voxel splat resolve", "shade voxels")),
    ("occluder", ("occluder cull", "occluder raster", "occlusion pyramid")),
    ("remaining", ("remaining cull", "remaining raster")),
    ("sun", ("voxel sun", "voxel near sun", "shadow", "near shadow", "far sun")),
    ("terrain", ("shade terrain", "terrain coarse")),
    ("shade", ("shade",)),
]


def group_of(name):
    for group, starts in GROUPS:
        if any(name == s or name.startswith(s + " ") for s in starts):
            return group
    return "other"


def groups(passes):
    totals = {}
    for name, value in passes.items():
        ms = value["median"] if isinstance(value, dict) else value
        totals[group_of(name)] = round(totals.get(group_of(name), 0.0) + ms, 3)
    return totals


def read_png(path):
    """8 bit RGB or RGBA PNG to (width, height, channels, rows of bytes)."""
    data = Path(path).read_bytes()
    pos, idat, width = 8, [], 0
    while pos < len(data):
        size, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + size]
        if kind == b"IHDR":
            width, height, depth, colour = struct.unpack(">IIBB", body[:10])
            if depth != 8 or colour not in (2, 6):
                raise SystemExit(f"{path}: not 8 bit RGB(A)")
            bpp = 3 if colour == 2 else 4
        elif kind == b"IDAT":
            idat.append(body)
        pos += size + 12
    raw = zlib.decompress(b"".join(idat))
    stride = width * bpp
    rows, prior = [], bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        kind, row = raw[start], bytearray(raw[start + 1:start + 1 + stride])
        if kind == 1:
            for i in range(bpp, stride):
                row[i] = (row[i] + row[i - bpp]) & 255
        elif kind == 2:
            row = bytearray((a + b) & 255 for a, b in zip(row, prior))
        elif kind == 3:
            for i in range(stride):
                row[i] = (row[i] + (((row[i - bpp] if i >= bpp else 0) + prior[i]) >> 1)) & 255
        elif kind == 4:
            for i in range(stride):
                a, b, c = (row[i - bpp] if i >= bpp else 0), prior[i], (prior[i - bpp] if i >= bpp else 0)
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                row[i] = (row[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(row)
        prior = row
    return width, height, bpp, rows


LINEAR = [(i / 255) ** 2.2 for i in range(256)]


def crown_rgb(image, box):
    """Mean linear RGB of the leaf pixels in a box: sky (blue over green) left out."""
    _, _, bpp, rows = image
    x0, y0, x1, y1 = box
    total, count = [0.0, 0.0, 0.0], 0
    for row in rows[y0:y1]:
        for x in range(x0, x1):
            r, g, b = row[x * bpp:x * bpp + 3]
            if b >= g:
                continue
            total[0] += LINEAR[r]
            total[1] += LINEAR[g]
            total[2] += LINEAR[b]
            count += 1
    return [round(t / count, 4) for t in total] if count else None


def line_strength(image):
    """Straight lines in the ground: peak autocorrelation of the row and column means, high-passed."""
    width, height, bpp, rows = image
    top = height // 4  # above the ground in most views
    lum = [[0.3 * row[x * bpp] + 0.6 * row[x * bpp + 1] + 0.1 * row[x * bpp + 2] for x in range(0, width, 2)] for row in rows[top::2]]
    across = [sum(line) / len(line) for line in lum]
    down = [sum(line[x] for line in lum) / len(lum) for x in range(len(lum[0]))]

    def peak(means):
        n = len(means)
        smooth = [sum(means[max(0, i - 10):i + 11]) / len(means[max(0, i - 10):i + 11]) for i in range(n)]
        high = [a - b for a, b in zip(means, smooth)]
        variance = sum(v * v for v in high) / n
        if variance <= 0:
            return {"c": 0.0, "px": 0}
        best = max((sum(high[i] * high[i + lag] for i in range(n - lag)) / (n - lag) / variance, lag) for lag in range(2, min(60, n // 2)))
        return {"c": round(best[0], 2), "px": best[1] * 2}
    return {"rows": peak(across), "columns": peak(down)}


def grid_strength(image, spacing):
    """Peak autocorrelation of the dark pixels around one grid step, as big-integer masks."""
    width, height, bpp, rows = image
    step = 4
    w, h = width // step, height // step
    top = h // 4  # above the ground in most views
    lum = [[sum(row[(x * step) * bpp + c] * k for c, k in enumerate((0.3, 0.6, 0.1))) for x in range(w)] for row in rows[top * step::step]]
    flat = sorted(v for line in lum for v in line)
    limit = flat[len(flat) // 2] * 0.55
    pad = w + 80
    mask = 0
    ones = 0
    for y, line in enumerate(lum):
        for x, v in enumerate(line):
            if v < limit:
                mask |= 1 << (y * pad + x)
                ones += 1
    total = len(lum) * w
    mean = ones / max(total, 1)
    if mean in (0.0, 1.0):
        return None
    best = (-1.0, 0, 0)
    lo, hi = spacing * 0.75 / step, spacing * 1.25 / step
    reach = int(hi) + 1
    for dy in range(0, reach + 1):
        for dx in range(-reach, reach + 1):
            if not lo * lo <= dx * dx + dy * dy <= hi * hi:
                continue
            shift = dy * pad + dx
            both = (mask & (mask >> shift)).bit_count() if shift >= 0 else 0
            pairs = (len(lum) - dy) * (w - abs(dx))
            value = (both / pairs - mean * mean) / (mean * (1 - mean))
            if value > best[0]:
                best = (value, dx * step, dy * step)
    return {"c": round(best[0], 3), "dx": best[1], "dy": best[2], "dark": round(mean, 3)}


def largest_cell(log):
    if not log.exists():
        return None
    for line in log.read_text(errors="replace").splitlines():
        if line.startswith("Voxel raster profile:"):
            views = json.loads(line.split(":", 1)[1])
            cells = views[0].get("largest_submitted_cells") or []
            if cells:
                owner = cells[0].get("cut_owner") or {}
                return {"px": round(cells[0]["diameter_px"], 2), "recipe": owner.get("root_recipe"), "kind": owner.get("kind"), "missing": views[0].get("missing_records")}
    return None


def jpeg(source, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    try:
        from PIL import Image
        image = Image.open(source).convert("RGB")
        image.thumbnail((960, 960))
        image.save(target, quality=85)
        return
    except ImportError:
        pass
    for tool, command in (("ffmpeg", ["ffmpeg", "-y", "-loglevel", "error", "-i", str(source), "-vf", "scale=960:-2", "-q:v", "4", str(target)]),
                          ("magick", ["magick", str(source), "-resize", "960x", "-quality", "85", str(target)])):
        if shutil.which(tool):
            subprocess.run(command, check=True)
            return
    shell = shutil.which("powershell.exe") or "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"
    native = [subprocess.run(["wslpath", "-w", str(p)], capture_output=True, text=True, check=True).stdout.strip() for p in (source, target)]
    script = ("Add-Type -AssemblyName System.Drawing;"
              f"$i=[Drawing.Image]::FromFile('{native[0]}');$h=[int]($i.Height*960/$i.Width);$o=New-Object Drawing.Bitmap 960,$h;"
              "$g=[Drawing.Graphics]::FromImage($o);$g.InterpolationMode='HighQualityBicubic';$g.DrawImage($i,0,0,960,$h);"
              "$c=[Drawing.Imaging.ImageCodecInfo]::GetImageEncoders()|?{$_.MimeType -eq 'image/jpeg'};"
              "$p=New-Object Drawing.Imaging.EncoderParameters 1;$p.Param[0]=New-Object Drawing.Imaging.EncoderParameter ([Drawing.Imaging.Encoder]::Quality,[long]85);"
              f"$o.Save('{native[1]}',$c,$p);$g.Dispose();$o.Dispose();$i.Dispose()")
    subprocess.run([shell, "-NoProfile", "-Command", script], check=True)


def measure(label):
    run = ROOT / "build" / "render-suite" / label
    report = json.loads((run / "report.json").read_text())
    suite = {v["name"]: v for v in report["suite"]["cameras"] + report["suite"].get("flights", [])}
    views = {}
    for name, summary in report["views"].items():
        view, png = suite[name], run / f"{name}.r0.png"
        image = read_png(png) if "crown" in view or "grid_px" in view or view.get("lines") else None
        scenario, _, angle = name.partition(".")
        counters = {k: (v["median"] if v else None) for k, v in summary["counters"].items()}
        views[name] = {"scenario": scenario, "angle": angle, "about": view.get("about"), "gpu_ms": round(summary["gpu_ms"]["median"], 2),
                       "groups": groups(summary["passes"]), "passes": {k: round(v["median"], 3) for k, v in summary["passes"].items() if v["median"] >= 0.05},
                       "counts": counters, "selected": (summary.get("forest") or {}).get("selected_records"),
                       "crown_rgb": crown_rgb(image, view["crown"]) if "crown" in view else None,
                       "grid": grid_strength(image, view["grid_px"]) if "grid_px" in view else None,
                       "largest_cell": largest_cell(run / f"{name}.r0.log")}
    near = next((v["crown_rgb"] for v in views.values() if v["crown_rgb"]), None)
    for view in views.values():
        if view["crown_rgb"] and near:
            view["crown_ratio"] = [round(a / max(b, 1e-6), 2) for a, b in zip(view["crown_rgb"], near)]
    return run, report, views


def show(views):
    for name, v in views.items():
        extra = []
        if v["crown_rgb"]:
            extra.append(f"crown {v['crown_rgb']} x{v.get('crown_ratio')}")
        if v["grid"]:
            extra.append(f"grid c={v['grid']['c']} at ({v['grid']['dx']},{v['grid']['dy']})")
        if v.get("lines"):
            extra.append(f"lines rows {v['lines']['rows']['c']} at {v['lines']['rows']['px']} px, columns {v['lines']['columns']['c']} at {v['lines']['columns']['px']} px")
        if v["largest_cell"]:
            extra.append(f"largest {v['largest_cell']['px']} px recipe {v['largest_cell']['recipe']}")
        print(f"{name:>16} {v['gpu_ms']:6.2f} ms  {'  '.join(extra)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("label", help="render-suite run under build/render-suite")
    parser.add_argument("--base", help="an earlier run of the same suite, to pair before and after")
    parser.add_argument("--out", help="write report.json and JPEGs here")
    parser.add_argument("--commit", help="commit of the run, when the tool ran from another checkout")
    parser.add_argument("--base-commit")
    args = parser.parse_args()
    run, report, views = measure(args.label)
    show(views)
    base_run, base_report, base_views = (None, None, None)
    if args.base:
        print(f"--- base {args.base}")
        base_run, base_report, base_views = measure(args.base)
        show(base_views)
    if not args.out:
        return
    out = Path(args.out)
    for name, view in views.items():
        jpeg(run / f"{name}.r0.png", out / "images" / f"{name}.jpg")
        view["image"] = f"images/{name}.jpg"
        if base_views and name in base_views:
            jpeg(base_run / f"{name}.r0.png", out / "images" / f"{name}.base.jpg")
            view["base"] = dict(base_views[name], image=f"images/{name}.base.jpg")
    clips = {}
    for name, folder, key in ((args.label, run, "after"), (args.base, base_run, "before")):
        for clip in sorted((folder / "clips").glob("*.mp4")) if folder else []:
            (out / "clips").mkdir(parents=True, exist_ok=True)
            target = out / "clips" / f"{clip.stem}.{key}.mp4"
            shutil.copyfile(clip, target)
            clips.setdefault(clip.stem, {})[key] = f"clips/{target.name}"
    summary = {"label": args.label, "commit": args.commit or report["commit"], "at": report["at"], "adapter": report.get("adapter"), "size": report["suite"]["size"],
               "base": {"label": args.base, "commit": args.base_commit or base_report["commit"]} if base_report else None, "clips": clips, "views": views}
    (out / "report.json").write_text(json.dumps(summary, indent=1), encoding="utf-8")
    print(f"{out}: {len(views)} views, {sum(p.stat().st_size for p in out.rglob('*') if p.is_file()) / 2**20:.1f} MiB")


if __name__ == "__main__":
    main()
