"""Inspect Sideways F12 geometry, isolate lighting effects, or compare recorded driving."""
import argparse
from collections import defaultdict
from contextlib import contextmanager
import json
import os
from pathlib import Path
import statistics
import struct
import subprocess
import sys
import tempfile

from PIL import Image, ImageChops, ImageDraw

ROOT = Path(__file__).resolve().parents[2]


def read(path):
    return json.loads(path.read_bytes())


def sibling(capture, name):
    if not isinstance(name, str) or Path(name).name != name or name in ("", ".", ".."):
        raise ValueError("invalid capture attachment name")
    return capture.parent / name


class Inspector:
    def __init__(self, path):
        self.path = path
        meta = read(path)
        self.point = meta.get("player_point")
        size = meta.get("capture", {}).get("size")
        if self.point is not None and not (
            isinstance(self.point, list) and len(self.point) == 2 and
            isinstance(size, list) and len(size) == 2 and
            all(type(p) is int and type(n) is int and 0 <= p < n for p, n in zip(self.point, size))
        ):
            raise ValueError("invalid player_point in capture JSON")
        self.visibility = meta.get("visibility") or {}
        if not self.visibility.get("views") and self.point is None:
            raise ValueError("no source visibility on this capture or graphics profile")
        self.views = []
        for v in self.visibility.get("views", []):
            with Image.open(sibling(path, v["ids"])) as image:
                if image.mode != "RGBA" or list(image.size) != v["size"]:
                    raise ValueError("visibility image format or dimensions differ from capture")
                ids = image.copy()
            self.views.append((v["rect"], ids, sibling(path, v["pairs"]).read_bytes()))

    def pick(self, x, y):
        if not self.views:
            return {"source": "no source visibility on this capture or graphics profile"}
        for (left, top, w, h), ids, pairs in self.views:
            if left <= x < left + w and top <= y < top + h:
                pixel = ids.getpixel(((x-left)*ids.width//w, (y-top)*ids.height//h))
                word = int.from_bytes(bytes(pixel), "little")
                if word == 0xffffffff:
                    return {"source": "no opaque source triangle"}
                if word >> 31:
                    return {"source": "voxel visibility", "packed_id": word}
                owner, cluster = struct.unpack_from("<II", pairs, 8*(word >> 7))
                return {"instance": owner, "cluster": cluster, "triangle": word & 127,
                        **self.visibility["owners"].get(str(owner), {}),
                        **self.visibility["clusters"].get(str(cluster), {})}
        return {"source": "outside captured viewports"}


def inspect(args):
    viewer = Inspector(args.capture)
    if args.pixel is not None:
        print(json.dumps(viewer.pick(*args.pixel), indent=2))
        return
    if viewer.point is not None and not args.preview:
        print(json.dumps({"player_point": viewer.point, **viewer.pick(*viewer.point)}, indent=2))
        return
    import tkinter as tk
    from PIL import ImageTk
    root = tk.Tk()
    root.title("Sideways capture · source geometry")
    with Image.open(args.capture.with_suffix(".png")) as source:
        original = source.size
        preview = source.convert("RGB")
    preview.thumbnail((root.winfo_screenwidth()*3//4, root.winfo_screenheight()*2//3))
    if viewer.point is not None:
        x, y = [(p + 0.5) * n / original[i] for i, (p, n) in enumerate(zip(viewer.point, preview.size))]
        pen = ImageDraw.Draw(preview)
        for radius, color in [(9, "black"), (8, "white")]:
            pen.ellipse((x-radius, y-radius, x+radius, y+radius), outline=color)
    photo = ImageTk.PhotoImage(preview)
    picture = tk.Label(root, image=photo, borderwidth=0, highlightthickness=0)
    picture.pack()
    initial = "Click a pixel to inspect its opaque source geometry."
    if viewer.point is not None:
        initial = f"Player point {tuple(viewer.point)}  " + json.dumps(viewer.pick(*viewer.point), ensure_ascii=False)
    info = tk.StringVar(value=initial)
    tk.Label(root, textvariable=info, justify="left", wraplength=preview.width).pack()

    def clicked(event):
        x, y = event.x*original[0]//preview.width, event.y*original[1]//preview.height
        info.set(f"({x}, {y})  " + json.dumps(viewer.pick(x, y), ensure_ascii=False))

    picture.bind("<Button-1>", clicked)
    root.mainloop()


@contextmanager
def checkout(ref):
    commit = subprocess.check_output(["git", "rev-parse", "--verify", "--end-of-options",
                                      ref+"^{commit}"], cwd=ROOT, text=True).strip()
    with tempfile.TemporaryDirectory(prefix="warp-drive-") as tmp:
        path = Path(tmp) / "source"
        subprocess.run(["git", "worktree", "add", "--quiet", "--detach", path, commit], cwd=ROOT, check=True)
        try:
            yield path, commit
        finally:
            subprocess.run(["git", "worktree", "remove", path], cwd=ROOT, check=True)


def replay(root, exe, capture, dest, state_only):
    env = dict(os.environ)
    if not exe:
        env["SIDEWAYS_GAME_ROOT"] = str(root / "games/sideways")
    target = Path(env.get("CARGO_TARGET_DIR", "target"))
    if not target.is_absolute():
        target = ROOT / target
    # A baseline must never replace an explicitly selected candidate executable.
    env["CARGO_TARGET_DIR"] = str(target if root == ROOT else target / "warp-compare-baseline")
    cmd = [exe] if exe else ["cargo", "run", "--locked", "-p", "cli", "--bin", "sideways", "--"]
    subprocess.run([*cmd, "replay-drive", capture, dest, *(["--state-only"] if state_only else [])],
                   cwd=root, env=env, check=True)
    return read(dest / "drive.json")


def animation(folder, frames):
    images = []
    for frame in frames:
        with Image.open(folder / frame["image"]) as img:
            images.append(img.convert("RGB"))
    images[0].save(folder / "drive.webp", save_all=True, append_images=images[1:],
                   duration=[round((i+1)*1000/15)-round(i*1000/15) for i in range(len(images))],
                   loop=0, lossless=True)


def pass_medians(frames):
    samples = defaultdict(list)
    for frame in frames[1:]:
        totals = defaultdict(float)
        for name, ms in frame["gpu_pass_ms"]:
            totals[name] += ms
        for name, total in totals.items():
            samples[name].append(total)
    return {name: statistics.median(values) for name, values in samples.items()}


def compare(args):
    capture = args.capture.resolve(strict=True)
    if (read(capture).get("drive") or {}).get("version") != 1:
        raise ValueError("requires a Sideways F12 capture with version 1 input history")
    out = args.out.resolve() if args.out else Path(tempfile.mkdtemp(prefix="warp-compare-"))
    if args.out:
        out.mkdir(parents=True)
    print(out, flush=True)
    base = out / "baseline"
    candidate = out / "candidate"
    if args.baseline_exe:
        ref = str(args.baseline_exe.resolve(strict=True))
        a = replay(ROOT, ref, capture, base, args.state_only)
    else:
        with checkout(args.baseline) as (src, ref):
            a = replay(src, None, capture, base, args.state_only)
    b = replay(ROOT, args.candidate_exe.resolve(strict=True) if args.candidate_exe else None,
               capture, candidate, args.state_only)
    diff = None
    for left, right in zip(a["samples"], b["samples"], strict=True):
        if left["tick"] != right["tick"]:
            raise ValueError("tick mismatch between replay traces")
        if diff is None and left != right:
            diff = {"tick": left["tick"], "baseline": left, "candidate": right}
    summary = {"baseline": ref, "baseline_matches_recording": a["matches_capture"],
               "candidate_matches_recording": b["matches_capture"], "first_difference_in_window": diff,
               "tick_range": [a["samples"][0]["tick"], a["end"]]}
    if a["frames"]:
        if [f["tick"] for f in a["frames"]] != [f["tick"] for f in b["frames"]]:
            raise ValueError("rendered ticks differ")
        for side, report in [(base, a), (candidate, b)]:
            animation(side, report["frames"])
        with Image.open(base / a["frames"][-1]["image"]) as left, Image.open(candidate / b["frames"][-1]["image"]) as right:
            ImageChops.difference(left.convert("RGB"), right.convert("RGB")).save(out / "difference.png")
        # Exclude the first render (lazy GPU pipeline initialization).
        def median_ms(frames):
            return statistics.median(f["render_readback_ms"] for f in frames[1:])
        summary["adapters"] = [a["adapter"], b["adapter"]]
        if len(a["frames"]) > 1 and a["adapter"] == b["adapter"]:
            before, after = pass_medians(a["frames"]), pass_medians(b["frames"])
            summary["gpu_pass_median_delta_ms"] = {name: after[name] - before[name]
                for name in sorted(before.keys() & after.keys())}
            old, new = median_ms(a["frames"]), median_ms(b["frames"])
            summary["diagnostic_render_including_readback_ms"] = {"baseline_median": old,
                "candidate_median": new, "delta": new-old}
    (out / "comparison.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))


LIGHT_CONTROLS = ("contact_occlusion", "shadow_strength", "near_shadows", "gi", "reflection_occlusion")


def luminance(rgb):
    # Relative luminance of the displayed sRGB pixel after tone mapping.
    values = [channel / 255 for channel in rgb]
    linear = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in values]
    return sum(value * weight for value, weight in zip(linear, (0.2126, 0.7152, 0.0722)))


def viewport(views, point):
    x, y = point
    for i, view in enumerate(views):
        left, top, width, height = view["rect"]
        if left <= x < left + width and top <= y < top + height:
            return i, view["rect"]
    raise ValueError(f"pixel {point} lies outside the captured visibility viewports")


def crop_sample(path, size, crop_box, points):
    with Image.open(path) as img:
        if img.size != tuple(size):
            raise ValueError(f"unexpected dimensions for {path}")
        crop = img.crop(crop_box).convert("RGB")
        samples = []
        for x, y in points:
            rgb = img.crop((x, y, x+1, y+1)).convert("RGB").getpixel((0, 0))
            samples.append({"rgb": rgb, "luminance": luminance(rgb)})
    return crop, samples


def lighting_report(capture, meta, out, view_rect, points, radius, inspector):
    report = read(out / "native" / "report.json")
    left, top, width, height = view_rect
    local = [(x-left, y-top) for x, y in points]
    x, y = local[0]
    box = (max(x-radius, 0), max(y-radius, 0), min(x+radius+1, width), min(y+radius+1, height))
    source_box = (box[0]+left, box[1]+top, box[2]+left, box[3]+top)
    source_crop, samples = crop_sample(capture.with_suffix(".png"), meta["capture"]["size"], source_box, points)
    source_hits = [inspector.pick(*p) for p in points]
    cases = {"capture": {"samples": samples, "picks": source_hits}}
    crops = {"capture": source_crop}
    for row in report["rows"]:
        name = row["name"]
        crop, samples = crop_sample(out / "native" / (name+".png"), (width, height), box, local)
        crops[name] = crop
        if [p["pixel"] for p in row["picks"]] != [list(p) for p in local]:
            raise ValueError("renderer returned different pick coordinates")
        cases[name] = {"samples": samples, "picks": row["picks"]}
    baseline = cases["lit"]
    for name, case in cases.items():
        for i, sample in enumerate(case["samples"]):
            sample["delta_from_lit"] = sample["luminance"] - baseline["samples"][i]["luminance"]
        if len(points) == 2:
            case["reference_minus_point"] = case["samples"][1]["luminance"] - case["samples"][0]["luminance"]
        crops[name].save(out / (name+"-crop.png"))
    panels = [name for name in crops if name != "repeat"]
    w, h = source_crop.size
    cell_w = max(w, 170)
    sheet = Image.new("RGB", (2*cell_w, ((len(panels)+1)//2)*(h+24)), "#202020")
    draw = ImageDraw.Draw(sheet)
    for i, name in enumerate(panels):
        ox, oy = (i % 2)*cell_w, (i // 2)*(h+24)
        label = {"lit": "Baseline", "capture": "Original F12"}.get(name, name+" off")
        draw.text((ox+5, oy+4), label, fill="white")
        sheet.paste(crops[name], (ox, oy+24))
        for (px, py), color in zip(local, ("#ffbc58", "#59cefb")):
            if box[0]+4 <= px < box[2]-4 and box[1]+4 <= py < box[3]-4:
                cx, cy = ox+px-box[0], oy+24+py-box[1]
                draw.ellipse((cx-4, cy-4, cx+4, cy+4), outline=color)
    sheet.save(out / "comparison.png")
    def same_triangle(source, replay, owner_key):
        keys = (owner_key, "cluster", "triangle", "mesh", "material")
        return (all(source.get(k) is not None for k in keys)
                and all(source[k] == replay.get("owner" if k == owner_key else k) for k in keys))
    summary = {"schema": 1, "capture": str(capture), "view": report["view_index"], "points": points,
               "crop_box_in_original": source_box, "baseline_controls": report["baseline_controls"],
               "source_triangle_matches_baseline": [same_triangle(a, b, "instance") for a, b in zip(source_hits, baseline["picks"])],
               "replay_triangle_stable": {name: [same_triangle(a, b, "owner") for a, b in zip(baseline["picks"], case["picks"])]
                                          for name, case in cases.items() if name not in ("capture", "lit")},
               "repeat_crop_identical": ImageChops.difference(crops["lit"], crops["repeat"]).getbbox() is None,
               "units": "display-referred relative luminance, linearized sRGB, range 0..1",
               "cases": cases, "limitations": report["limitations"]}
    (out / "lighting.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")


def relight(args):
    capture = args.capture.resolve()
    meta = read(capture)
    inspector = Inspector(capture)
    point = args.pixel or inspector.point
    if point is None:
        raise ValueError("capture has no player_point; pass --pixel X Y")
    views = inspector.visibility.get("views") or []
    if not views:
        raise ValueError("relight needs the source visibility attachments for viewport and surface checks")
    view, rect = viewport(views, point)
    points = [point]
    if args.reference:
        if viewport(views, args.reference)[0] != view:
            raise ValueError("reference must be in the same viewport as the selected pixel")
        points.append(args.reference)
    if not 1 <= args.radius <= 1024:
        raise ValueError("--radius must be in 1..1024")
    if len(set(args.without)) != len(args.without):
        raise ValueError("each --without control must be unique")
    if args.out:
        out = args.out.resolve()
        out.mkdir(parents=True, exist_ok=False)
    else:
        out = Path(tempfile.mkdtemp(prefix="warp-relight-"))
    cmd = ["cargo", "run", "--locked", "-p", "cli", "--example", "capture_replay", "--",
           capture, out / "native", "--view", str(view)]
    for x, y in points:
        cmd.extend(["--pick", f"{x-rect[0]},{y-rect[1]}"])
    for effect in args.without:
        cmd.extend(["--without", effect])
    log = out / "render.log"
    with log.open("wb") as stream:
        result = subprocess.run(cmd, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT)
    if result.returncode:
        tail = log.read_text(encoding="utf-8", errors="replace").splitlines()[-3:]
        raise ValueError(f"lighting replay failed; see {log}\n" + "\n".join(tail))
    lighting_report(capture, meta, out, rect, points, args.radius, inspector)
    print(json.dumps({"report": str(out / "lighting.json"), "image": str(out / "comparison.png")}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("inspect")
    p.add_argument("capture", type=Path)
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--preview", action="store_true", help="open the image with the player point circled")
    mode.add_argument("--pixel", type=int, nargs=2, metavar=("X", "Y"))
    p = sub.add_parser("relight", help="rerender a marked static surface with one lighting control disabled per image")
    p.add_argument("capture", type=Path)
    p.add_argument("--without", choices=LIGHT_CONTROLS, action="append", required=True)
    p.add_argument("--pixel", type=int, nargs=2, metavar=("X", "Y"), help="override the saved player_point")
    p.add_argument("--reference", type=int, nargs=2, metavar=("X", "Y"), help="also measure a nearby reference pixel")
    p.add_argument("--radius", type=int, default=128, help="crop radius around the selected pixel")
    p.add_argument("--out", type=Path, help="fresh output directory")
    p = sub.add_parser("compare")
    p.add_argument("capture", type=Path)
    group = p.add_mutually_exclusive_group()
    group.add_argument("--baseline", default="main")
    group.add_argument("--baseline-exe", type=Path)
    p.add_argument("--candidate-exe", type=Path)
    p.add_argument("--out", type=Path)
    p.add_argument("--state-only", action="store_true")
    args = parser.parse_args()
    {"inspect": inspect, "compare": compare, "relight": relight}[args.command](args)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, struct.error, subprocess.CalledProcessError) as e:
        sys.exit(str(e))
