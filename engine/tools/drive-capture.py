"""Inspect Sideways F12 source geometry or compare the last five seconds of driving."""
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

from PIL import Image, ImageChops

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
        self.visibility = meta.get("visibility") or {}
        if not self.visibility.get("views"):
            raise ValueError("no source visibility on this capture or graphics profile")
        self.views = []
        for v in self.visibility["views"]:
            with Image.open(sibling(path, v["ids"])) as image:
                if image.mode != "RGBA" or list(image.size) != v["size"]:
                    raise ValueError("visibility image format or dimensions differ from capture")
                ids = image.copy()
            self.views.append((v["rect"], ids, sibling(path, v["pairs"]).read_bytes()))

    def pick(self, x, y):
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
    import tkinter as tk
    from PIL import ImageTk
    root = tk.Tk()
    root.title("Sideways capture · source geometry")
    with Image.open(args.capture.with_suffix(".png")) as source:
        original = source.size
        preview = source.convert("RGB")
    preview.thumbnail((root.winfo_screenwidth()*3//4, root.winfo_screenheight()*2//3))
    photo = ImageTk.PhotoImage(preview)
    picture = tk.Label(root, image=photo, borderwidth=0, highlightthickness=0)
    picture.pack()
    info = tk.StringVar(value="Click a pixel to inspect its opaque source geometry.")
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
    env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target"))
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("inspect")
    p.add_argument("capture", type=Path)
    p.add_argument("--pixel", type=int, nargs=2, metavar=("X", "Y"))
    p = sub.add_parser("compare")
    p.add_argument("capture", type=Path)
    group = p.add_mutually_exclusive_group()
    group.add_argument("--baseline", default="main")
    group.add_argument("--baseline-exe", type=Path)
    p.add_argument("--candidate-exe", type=Path)
    p.add_argument("--out", type=Path)
    p.add_argument("--state-only", action="store_true")
    args = parser.parse_args()
    (inspect if args.command == "inspect" else compare)(args)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, struct.error, subprocess.CalledProcessError) as e:
        sys.exit(str(e))
