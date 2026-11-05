#!/usr/bin/env python3
"""One world, fixed cameras: measure every view, keep the pictures, compare two runs.

    render-suite.py run --exe target/release/examples/terrain_fly.exe --label main
    render-suite.py diff build/render-suite/main build/render-suite/candidate

See engine/docs/render-suite.md. Standard library only. Under WSL a Windows
.exe is driven with Windows paths, so the GPU numbers are native.
"""
from __future__ import annotations
import argparse
from datetime import datetime, timezone
import html
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
SUITE = Path(__file__).with_name("render-suite.json")
# A picture counts as changed past these; the noise floor between repeats is added on top.
OVER16_LIMIT_PCT = 0.1
# A pass or frame time counts as changed past this and past the spread between repeats.
MIN_DELTA_MS = 0.1


def native_path(path, exe):
    path = str(path)
    if str(exe).endswith(".exe") and "microsoft" in platform.release().lower():
        return subprocess.run(["wslpath", "-w", path], capture_output=True, text=True, check=True).stdout.strip()
    return path


def command_output(command):
    try:
        return subprocess.run(command, capture_output=True, text=True, timeout=20).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


def nvidia_smi():
    return shutil.which("nvidia-smi") or shutil.which("nvidia-smi.exe") or ("/mnt/c/Windows/System32/nvidia-smi.exe" if Path("/mnt/c/Windows/System32/nvidia-smi.exe").exists() else None)


def gpu_query(field):
    smi = nvidia_smi()
    value = command_output([smi, f"--query-gpu={field}", "--format=csv,noheader,nounits"]) if smi else ""
    try:
        return float(value.splitlines()[0])
    except (IndexError, ValueError):
        return None


def machine_state():
    """What else is running: timings from a busy machine are not comparable."""
    runners = command_output(["pgrep", "-fc", "[R]unner.Worker"]) if shutil.which("pgrep") else ""
    return {"load": os.getloadavg()[0] if hasattr(os, "getloadavg") else None, "ci_jobs": int(runners or 0), "gpu_util_pct": gpu_query("utilization.gpu"),
            "gpu_clock_mhz": gpu_query("clocks.gr"), "gpu_temp_c": gpu_query("temperature.gpu")}


class MemoryWatch:
    """Peak GPU memory over the idle level, and peak process memory on Windows."""
    def __init__(self, exe):
        self.image, self.idle, self.vram_mib, self.ram_mib = Path(exe).name, gpu_query("memory.used"), None, None
        self.done = threading.Event()
        self.thread = threading.Thread(target=self.watch, daemon=True)

    def watch(self):
        tasklist = shutil.which("tasklist.exe") or "/mnt/c/Windows/System32/tasklist.exe"
        while not self.done.wait(1.0):
            used = gpu_query("memory.used")
            if used is not None:
                self.vram_mib = max(self.vram_mib or 0, used - (self.idle or 0))
            if self.image.endswith(".exe") and Path(tasklist).exists():
                row = command_output([tasklist, "/FI", f"IMAGENAME eq {self.image}", "/FO", "CSV", "/NH"]).split('","')
                if len(row) >= 5:
                    digits = "".join(c for c in row[4] if c.isdigit())
                    if digits:
                        self.ram_mib = max(self.ram_mib or 0, int(digits) / 1024)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.done.set()
        self.thread.join()


def percentile(values, q):
    values = sorted(values)
    return values[int((len(values) - 1) * q)] if values else None


def run_view(exe, suite, view, out, stem, extra, env):
    """One process per view: no state leaks from one camera into the next."""
    frames = view.get("frames", suite["frames"])
    base = out / stem
    command = [str(exe), *suite["world"], *extra, "--no-hud", "--at", view["at"], "--fly", str(frames), "--size", suite["size"],
               "--fly-times", native_path(f"{base}.times.json", exe), "--fly-shot", native_path(f"{base}.png", exe), "--fly-stats", native_path(f"{base}.stats.json", exe)]
    command += ["--speed", str(view["speed"])] if "speed" in view else ["--stationary"]
    started = time.time()
    with MemoryWatch(exe) as memory:
        done = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True, errors="replace")
    (out / f"{stem}.log").write_text(done.stdout + done.stderr, encoding="utf-8")
    if done.returncode:
        raise SystemExit(f"{stem} failed ({done.returncode}), see {out / (stem + '.log')}\n{done.stderr[-800:]}")
    measured = suite["measure_frames"] if "speed" not in view else frames
    samples = json.loads(Path(f"{base}.times.json.gpu.json").read_text())[-measured:]
    walls = json.loads(Path(f"{base}.times.json").read_text())[-measured:]
    passes = {}
    for sample in samples:
        totals = {}
        for name, ms in sample["passes"]:
            totals[name] = totals.get(name, 0.0) + ms
        for name, ms in totals.items():
            passes.setdefault(name, []).append(ms)
    covered = [s["covered_ms"] for s in samples]
    stats = json.loads(Path(f"{base}.stats.json").read_text())
    cpu = json.loads(Path(f"{base}.times.json.cpu.json").read_text())[-measured:]
    return {"seconds": time.time() - started, "samples": len(samples), "gpu_ms": statistics.median(covered), "gpu_p95_ms": percentile(covered, 0.95), "gpu_max_ms": max(covered),
            "wall_ms": statistics.median(walls), "wall_p99_ms": percentile(walls, 0.99), "wall_max_ms": max(walls),
            "hitches": sum(w > 3 * statistics.median(walls) for w in walls),
            "terrain_cpu_ms": statistics.median(c["terrain_ms"] for c in cpu), "render_cpu_ms": statistics.median(c["draw"]["render_ms"] for c in cpu),
            "passes": {name: statistics.median(ms) * len(ms) / len(samples) for name, ms in passes.items()},
            "stats": stats, "vram_peak_mib": memory.vram_mib, "ram_peak_mib": memory.ram_mib}


def count(stats, *path):
    for key in path:
        stats = stats.get(key) if isinstance(stats, dict) else None
    return stats


def counters(stats):
    forest = stats.get("forest") or {}
    return {"camera clusters": count(stats, "camera", "clusters"), "camera triangles": count(stats, "camera", "triangles"), "camera vertices": count(stats, "camera", "submitted_vertices"),
            "shadow clusters": count(stats, "shadow", "clusters"), "shadow triangles": count(stats, "shadow", "triangles"), "shadow vertices": count(stats, "shadow", "submitted_vertices"),
            "voxel splats": sum(v["splats"] for v in stats.get("voxel_views", [])), "forest tested nodes": forest.get("tested_nodes"), "forest source slots": forest.get("source_slots"),
            "forest voxel slots": forest.get("voxel_slots"), "forest camera roots": forest.get("camera_roots"), "forest shadow-only roots": forest.get("shadow_only_roots"),
            "gpu allocated MiB": round(count(stats, "memory", "allocated_bytes") / 2**20, 1) if count(stats, "memory", "allocated_bytes") else None,
            "gpu reserved MiB": round(count(stats, "memory", "reserved_bytes") / 2**20, 1) if count(stats, "memory", "reserved_bytes") else None}


def summarize(repeats):
    """Medians over repeats, with the spread as the noise of each number."""
    def spread(values):
        values = [v for v in values if v is not None]
        return {"median": statistics.median(values), "spread": max(values) - min(values)} if values else None
    names = sorted({name for r in repeats for name in r["passes"]})
    last = repeats[-1]
    return {"gpu_ms": spread([r["gpu_ms"] for r in repeats]), "gpu_p95_ms": spread([r["gpu_p95_ms"] for r in repeats]), "wall_ms": spread([r["wall_ms"] for r in repeats]),
            "wall_p99_ms": spread([r["wall_p99_ms"] for r in repeats]), "wall_max_ms": spread([r["wall_max_ms"] for r in repeats]), "hitches": max(r["hitches"] for r in repeats),
            "terrain_cpu_ms": spread([r["terrain_cpu_ms"] for r in repeats]), "render_cpu_ms": spread([r["render_cpu_ms"] for r in repeats]),
            "passes": {n: spread([r["passes"].get(n, 0.0) for r in repeats]) for n in names},
            "counters": {k: spread([counters(r["stats"])[k] for r in repeats]) for k in counters(last["stats"])},
            "vram_peak_mib": spread([r["vram_peak_mib"] for r in repeats]), "ram_peak_mib": spread([r["ram_peak_mib"] for r in repeats]),
            "memory_top": (last["stats"].get("memory") or {}).get("top", [])[:15], "forest": last["stats"].get("forest"), "pages": last["stats"].get("pages")}


def diff_images(exe, a, b, heat):
    done = subprocess.run([str(exe), "--diff-images", native_path(a, exe), native_path(b, exe), native_path(heat, exe)], cwd=ROOT, capture_output=True, text=True, errors="replace")
    if done.returncode:
        raise SystemExit(f"image diff failed: {done.stderr[-400:]}")
    return json.loads(done.stdout.strip().splitlines()[-1])


def run(args):
    suite = json.loads(Path(args.suite).read_text())
    exe = Path(args.exe).resolve()
    out = ROOT / "build" / "render-suite" / args.label
    if out.exists() and not args.overwrite:
        raise SystemExit(f"{out} exists; pick another --label or pass --overwrite")
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    overrides = dict(kv.split("=", 1) for kv in args.env)
    env.update(overrides)
    if "microsoft" in platform.release().lower() and overrides:
        env["WSLENV"] = ":".join(filter(None, [env.get("WSLENV", ""), *overrides]))
    views = [v for v in suite["cameras"] + suite.get("flights", []) if not args.only or v["name"] in args.only.split(",")]
    before = machine_state()
    if args.require_idle and (before["ci_jobs"] or (before["gpu_util_pct"] or 0) >= 15):
        raise SystemExit(f"machine is busy: {before}")
    # The first run after a shader change compiles every pipeline for minutes.
    warm = dict(views[0], frames=6)
    print("warming the pipeline cache", flush=True)
    run_view(exe, dict(suite, measure_frames=1), warm, out, "warm", args.arg, env)
    results, states = {v["name"]: [] for v in views}, [before]
    for repeat in range(args.repeat):
        for view in views:
            stem = f"{view['name']}.r{repeat}"
            results[view["name"]].append(run_view(exe, suite, view, out, stem, args.arg, env))
            states.append(machine_state())
            r = results[view["name"]][-1]
            print(f"{stem:>14} gpu {r['gpu_ms']:6.2f} ms  wall {r['wall_ms']:6.2f} ms  tris {count(r['stats'], 'camera', 'triangles')}  {r['seconds']:.0f} s", flush=True)
    report = {"label": args.label, "at": datetime.now(timezone.utc).isoformat(timespec="seconds"), "commit": command_output(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"]),
              "dirty": bool(command_output(["git", "-C", str(ROOT), "status", "--porcelain", "--untracked-files=no"])), "env": overrides, "args": args.arg, "suite": suite, "repeats": args.repeat,
              "adapter": results[views[0]["name"]][0]["stats"].get("adapter"), "backend": results[views[0]["name"]][0]["stats"].get("backend"), "driver": results[views[0]["name"]][0]["stats"].get("driver"),
              "machine": states, "views": {}}
    for view in views:
        name = view["name"]
        summary = summarize(results[name])
        # Same build, same camera: what differs here is the noise floor of a picture.
        summary["noise"] = diff_images(exe, out / f"{name}.r0.png", out / f"{name}.r{args.repeat - 1}.png", out / f"{name}.noise.png") if args.repeat > 1 else None
        report["views"][name] = summary
    (out / "report.json").write_text(json.dumps(report, indent=1), encoding="utf-8")
    busy = sum(1 for s in states if s["ci_jobs"] or (s["load"] or 0) > 8)
    print(f"{out}: {len(views)} views x {args.repeat}" + (f"; {busy}/{len(states)} checks saw CI jobs or high load, trust GPU times over wall times" if busy else ""))


def changed(a, b):
    if not a or not b:
        return False
    return abs(b["median"] - a["median"]) > max(MIN_DELTA_MS, a["spread"], b["spread"])


def diff(args):
    base, cur = Path(args.base), Path(args.current)
    a, b = (json.loads((p / "report.json").read_text()) for p in (base, cur))
    exe = Path(args.exe).resolve()
    out = Path(args.out) if args.out else cur / f"vs-{base.name}"
    out.mkdir(parents=True, exist_ok=True)
    lines, cards, failed = [f"{b['label']} ({b['commit']}{'+' if b['dirty'] else ''}) against {a['label']} ({a['commit']}{'+' if a['dirty'] else ''}), {b['adapter']}, {b['suite']['size']}"], [], []
    if a["suite"] != b["suite"]:
        lines.append("WARNING: the suites differ, these runs are not comparable")
    lines.append(f"{'view':>8} {'gpu ms':>16} {'wall ms':>16} {'tris':>18} {'over16 %':>9} {'max':>4} {'ssim':>7}")
    for name, cv in b["views"].items():
        bv = a["views"].get(name)
        if not bv:
            continue
        image = diff_images(exe, base / f"{name}.r0.png", cur / f"{name}.r0.png", out / f"{name}.heat.png")
        limit = OVER16_LIMIT_PCT + max((bv.get("noise") or {}).get("over16_pct", 0), (cv.get("noise") or {}).get("over16_pct", 0))
        if image["over16_pct"] > limit:
            failed.append(name)
        g0, g1 = bv["gpu_ms"], cv["gpu_ms"]
        t0, t1 = (v["counters"]["camera triangles"]["median"] for v in (bv, cv))
        mark = "*" if changed(g0, g1) else " "
        lines.append(f"{name:>8} {g0['median']:7.2f}>{g1['median']:7.2f}{mark} {bv['wall_ms']['median']:7.2f}>{cv['wall_ms']['median']:7.2f}  {t0:>8.0f}>{t1:>8.0f} {image['over16_pct']:9.3f} {image['max']:4d} {image['ssim']:7.4f}" + ("  IMAGE CHANGED" if name in failed else ""))
        for label in ("terrain_cpu_ms", "render_cpu_ms"):
            if changed(bv.get(label), cv.get(label)):
                lines.append(f"{'':>10}{cv[label]['median'] - bv[label]['median']:+7.2f} ms  {label}")
        passes = sorted(((cv["passes"].get(n) or {"median": 0, "spread": 0})["median"] - (bv["passes"].get(n) or {"median": 0, "spread": 0})["median"], n)
                        for n in set(bv["passes"]) | set(cv["passes"]) if changed(bv["passes"].get(n) or {"median": 0, "spread": 0}, cv["passes"].get(n) or {"median": 0, "spread": 0}))
        passes.sort(key=lambda p: -abs(p[0]))
        for delta, n in passes[:6]:
            lines.append(f"{'':>10}{delta:+7.2f} ms  {n}")
        counts = [f"{k} {bv['counters'][k]['median']:.0f}>{cv['counters'][k]['median']:.0f}" for k in cv["counters"] if cv["counters"][k] and bv["counters"].get(k) and bv["counters"][k]["median"] != cv["counters"][k]["median"]
                  and abs(cv["counters"][k]["median"] - bv["counters"][k]["median"]) > max(0.02 * abs(bv["counters"][k]["median"]), bv["counters"][k]["spread"], cv["counters"][k]["spread"])]
        if counts:
            lines.append(f"{'':>10}{'; '.join(counts)}")
        rel = lambda p: html.escape(os.path.relpath(p, out))
        cards.append(f"<section><h2>{html.escape(name)}: {g0['median']:.2f} to {g1['median']:.2f} ms, {image['over16_pct']:.3f}% over 16 levels</h2><div class=row>"
                     f"<figure><a href='{rel(base / (name + '.r0.png'))}'><img loading=lazy src='{rel(base / (name + '.r0.png'))}'></a><figcaption>{html.escape(a['label'])}</figcaption></figure>"
                     f"<figure><a href='{rel(cur / (name + '.r0.png'))}'><img loading=lazy src='{rel(cur / (name + '.r0.png'))}'></a><figcaption>{html.escape(b['label'])}</figcaption></figure>"
                     f"<figure><a href='{name}.heat.png'><img loading=lazy src='{name}.heat.png'></a><figcaption>difference</figcaption></figure></div></section>")
    text = "\n".join(lines)
    print(text)
    (out / "summary.txt").write_text(text + "\n", encoding="utf-8")
    (out / "index.html").write_text("<!doctype html><meta charset=utf-8><title>Render suite</title><style>body{font:14px system-ui;margin:16px;background:#111;color:#ddd}pre{overflow:auto}.row{display:grid;grid-template-columns:repeat(3,1fr);gap:8px}"
                                    "figure{margin:0}img{width:100%;display:block}figcaption{padding:4px 0}</style><pre>" + html.escape(text) + "</pre>" + "".join(cards), encoding="utf-8")
    if failed:
        print(f"images changed beyond the noise floor: {', '.join(failed)}; inspect {out / 'index.html'}")
    return 1 if failed else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    r = commands.add_parser("run", help="measure every view of the suite")
    r.add_argument("--exe", default=str(ROOT / "target/release/examples/terrain_fly"))
    r.add_argument("--label", required=True, help="results go to build/render-suite/LABEL")
    r.add_argument("--suite", default=str(SUITE))
    r.add_argument("--only", help="comma-separated view names")
    r.add_argument("--repeat", type=int, default=3)
    r.add_argument("--env", action="append", default=[], metavar="KEY=VALUE", help="set for the renderer, such as a KERN_* switch")
    r.add_argument("--arg", action="append", default=[], help="extra terrain_fly argument")
    r.add_argument("--require-idle", action="store_true", help="refuse to start next to CI jobs or a busy GPU")
    r.add_argument("--overwrite", action="store_true")
    d = commands.add_parser("diff", help="compare two runs: times, counters and pictures")
    d.add_argument("base")
    d.add_argument("current")
    d.add_argument("--exe", default=str(ROOT / "target/release/examples/terrain_fly"))
    d.add_argument("--out")
    args = parser.parse_args()
    sys.exit(run(args) if args.command == "run" else diff(args))


if __name__ == "__main__":
    main()
