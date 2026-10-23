#!/usr/bin/env python3
"""Plan, explicitly run, or analyze the compiled lighting_fixture example.

`plan` and `analyze` never start the engine. `run` is an explicit GPU operation
for use only when the playtest has yielded the GPU. Python analysis requires
numpy and Pillow. Fixture images are synthetic tests of the production shader,
not screenshots of a discovered route defect or measurements of gameplay FPS.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time

ENGINE_ROOT = Path(__file__).resolve().parents[1]


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def plan(args):
    out = Path(args.out).resolve()
    if out.exists():
        raise ValueError("Use a fresh plan/evidence directory")
    out.mkdir(parents=True)
    exe = str(Path(args.exe).resolve())
    jobs = []

    def add(name, group, world, camera, car="on", contact="on", reflection="on", modes="0", views=1):
        target = out / name
        command = [exe, "--out", str(target), "--obstacle", world, "--camera", camera,
                   "--car", car, "--contact", contact, "--reflection", reflection,
                   "--views", str(views), "--size", args.capture_size if group == "capture" else "3840x2160",
                   "--debug", modes, "--warmup", "1" if group == "capture" else "8",
                   "--repeat", "1" if group == "capture" else "40"]
        jobs.append({"name": name, "group": group, "out": str(target), "command": command})

    for world in ("open", "mountain"):
        add(f"side-{world}-on", "capture", world, "side", modes="0,2,3,5,7")
        add(f"side-{world}-reflection-off", "capture", world, "side", reflection="off", modes="0,2,3,5,7")
        add(f"side-{world}-no-car", "capture", world, "side", car="off", modes="2")
        add(f"contact-{world}-on", "capture", world, "contact", modes="0,2,3,4,6")
        add(f"contact-{world}-contact-off", "capture", world, "contact", contact="off", modes="0,2,3,4,6")
        add(f"contact-{world}-no-car", "capture", world, "contact", car="off", modes="0,2,3,4,6")
    for views in (1, 4):
        for index, enabled in enumerate(("on", "off", "off", "on")):
            add(f"perf-{views}-{index}-{enabled}", "perf", "mountain", "side", contact=enabled, reflection=enabled, views=views)
    value = {"schema": 1, "fixture": "indirect_visibility_v1", "out": str(out), "cwd": str(ENGINE_ROOT),
             "executable": exe, "jobs": jobs,
             "policy": "No engine is launched by planning. Run capture/perf groups explicitly after GPU handover.",
             "interpretation": ["Camera and car pose stay fixed; independent car/no-car and open/mountain variants.",
                                "Contact and reflection capture toggles isolate each feature; perf toggles both as a package.",
                                "Four-view performance repeats the same camera; this is synthetic shading cost, not route FPS."]}
    write_json(out / "plan.json", value)
    print(out / "plan.json")


def run(args):
    plan_path = Path(args.plan).resolve()
    value = json.loads(plan_path.read_text(encoding="utf-8"))
    exe = Path(value["executable"])
    binary_hash = digest(exe)
    out = Path(value["out"])
    logs = out / "logs"
    logs.mkdir(exist_ok=True)
    for job in value["jobs"]:
        if args.group != "all" and job["group"] != args.group:
            continue
        if Path(job["out"]).exists():
            raise ValueError(f"Refusing to overwrite previous evidence: {job['out']}")
        if digest(exe) != binary_hash:
            raise ValueError("Executable changed between jobs")
        started = time.time()
        print(f"RUN {job['name']}", flush=True)
        with (logs / f"{job['name']}.log").open("wb") as log:
            result = subprocess.run(job["command"], cwd=value["cwd"], stdout=log, stderr=subprocess.STDOUT,
                                    creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0), timeout=args.timeout)
        provenance = {"executable_sha256": binary_hash, "command": job["command"], "start_unix": started,
                      "duration_seconds": time.time() - started, "returncode": result.returncode}
        write_json(logs / f"{job['name']}.json", provenance)
        if result.returncode:
            raise RuntimeError(f"{job['name']} failed: see its preserved log")
    if digest(exe) != binary_hash:
        raise ValueError("Executable changed during the final job")


def signature(report):
    """Only invariants which must match between a feature/occluder pair."""
    return {k: report[k] for k in ("fixture", "size", "view_count", "camera", "car", "ground", "lighting")}


def analyze(args):
    import numpy as np
    from PIL import Image
    manifest = json.loads(Path(args.plan).read_text(encoding="utf-8"))
    out = Path(manifest["out"])
    reports = {}
    checks = []
    missing = []
    for job in manifest["jobs"]:
        path = Path(job["out"]) / "report.json"
        if path.exists():
            reports[job["name"]] = json.loads(path.read_text(encoding="utf-8"))
        elif args.group in ("all", job["group"]):
            missing.append(job["name"])
    if missing:
        write_json(out / "analysis.json", {"status": "incomplete", "missing_jobs": missing})
        raise ValueError(f"Missing {len(missing)} jobs; analysis.json records them")

    def check(name, ok, evidence):
        checks.append({"check": name, "status": "pass" if bool(ok) else "fail", "evidence": evidence})

    def image(name, mode):
        report = reports[name]
        path = out / name / f"debug-{mode}.png"
        with Image.open(path) as source:
            if list(source.size) != report["size"]:
                raise ValueError(f"Image size mismatch: {path}")
            data = np.asarray(source.convert("RGB"), dtype=np.float64) / 255.0
        x, y, w, h = report["view_rects"][0]
        return data[y:y+h, x:x+w]

    def luma(name, mode):
        rgb = image(name, mode)
        linear = np.where(rgb <= 0.04045, rgb / 12.92, ((rgb + 0.055) / 1.055) ** 2.4)
        return linear @ np.array([0.2126, 0.7152, 0.0722])

    def stats(array, mask):
        values = array[mask]
        if not values.size:
            return {"pixels": 0, "median": None, "p75": None, "p95": None}
        return {"pixels": int(values.size), "median": float(np.median(values)),
                "p75": float(np.quantile(values, .75)), "p95": float(np.quantile(values, .95))}

    def same_pose(*names):
        first = signature(reports[names[0]])
        if any(signature(reports[name]) != first for name in names[1:]):
            raise ValueError(f"Pose/render-input mismatch: {names}")

    def ground_masks(report):
        _, _, w, h = report["view_rects"][0]
        camera = report["camera"]
        position = np.array(camera["position"], dtype=float)
        forward = np.array(camera["target"], dtype=float) - position
        forward /= np.linalg.norm(forward)
        right = np.cross(forward, [0., 1., 0.]); right /= np.linalg.norm(right)
        up = np.cross(right, forward)
        tangent = math.tan(camera["fov_y_radians"] / 2)
        xx = (2 * (np.arange(w) + .5) / w - 1)[None, :, None]
        yy = (1 - 2 * (np.arange(h) + .5) / h)[:, None, None]
        rays = forward + xx * tangent * (w / h) * right + yy * tangent * up
        valid = rays[..., 1] < -1e-6
        distance = np.divide(-position[1], rays[..., 1], out=np.zeros((h, w)), where=valid)
        points = position + rays * distance[..., None]
        x, z = points[..., 0], points[..., 2]
        footprint = valid & (abs(x) <= .65) & (abs(z) <= 1.4)
        adjacent = valid & (abs(x) >= 1.3) & (abs(x) <= 2.3) & (abs(z) <= 1.4)
        return footprint, adjacent

    if args.group in ("all", "capture"):
        for world in ("open", "mountain"):
            on, off, empty = [f"contact-{world}-{v}" for v in ("on", "contact-off", "no-car")]
            same_pose(on, off, empty)
            albedo_on, albedo_off, albedo_empty = image(on, 2), image(off, 2), image(empty, 2)
            check(f"{world}_contact_toggle_keeps_geometry_and_albedo", np.max(abs(albedo_on-albedo_off)) <= 1/255,
                  {"max_srgb_difference": float(np.max(abs(albedo_on-albedo_off)))})
            footprint, adjacent = ground_masks(reports[on])
            visible_floor = np.max(abs(albedo_on-albedo_empty), axis=2) <= 1/255
            footprint &= visible_floor; adjacent &= visible_floor
            a, b, reference = luma(on, 4), luma(off, 4), luma(empty, 4)
            under_a, under_b, under_empty = stats(a, footprint), stats(b, footprint), stats(reference, footprint)
            nearby_a, nearby_b = stats(a, adjacent), stats(b, adjacent)
            enough = under_a["pixels"] >= 16 and nearby_a["pixels"] >= 16
            check(f"{world}_visible_footprint_and_control_are_sampled", enough,
                  {"under_car": under_a, "adjacent": nearby_a})
            if enough:
                drop = float(np.median((b-a)[footprint]))
                control_change = float(np.median(abs(a-b)[adjacent]))
                direct_difference = float(np.max(abs(image(on, 3)-image(off, 3))[footprint]))
                check(f"{world}_contact_reduces_indirect_light_under_car", drop > .005,
                      {"median_linear_display_drop": drop, "on": under_a, "off": under_b, "no_car": under_empty})
                check(f"{world}_contact_is_local_and_does_not_fake_sun_shadow", control_change < .03 and direct_difference <= 1/255,
                      {"adjacent_linear_display_change": control_change, "direct_srgb_max_difference": direct_difference,
                       "adjacent_on": nearby_a, "adjacent_off": nearby_b})

        opened, blocked, disabled = "side-open-on", "side-mountain-on", "side-mountain-reflection-off"
        same_pose(opened, blocked, disabled, "side-open-no-car", "side-mountain-no-car")
        car_open = np.max(abs(image(opened, 2)-image("side-open-no-car", 2)), axis=2) > 2/255
        car_blocked = np.max(abs(image(blocked, 2)-image("side-mountain-no-car", 2)), axis=2) > 2/255
        car = car_open & car_blocked
        check("reflection_fixture_has_common_visible_car_pixels", car.sum() >= 100, {"pixels": int(car.sum())})
        direct_open, direct_blocked = stats(luma(opened, 3), car), stats(luma(blocked, 3), car)
        check("mountain_actually_blocks_directional_sun", car.sum() >= 100 and direct_blocked["p75"] < direct_open["p75"]*.8,
              {"open_direct": direct_open, "mountain_direct": direct_blocked})
        visibility_open, visibility_blocked = luma(opened, 7), luma(blocked, 7)
        spec_open, spec_blocked, spec_off = luma(opened, 5), luma(blocked, 5), luma(disabled, 5)
        obstructed = car & (visibility_blocked < visibility_open-.03) & (spec_off > .005)
        visible = car & (visibility_open > .1) & (visibility_blocked >= visibility_open*.95) & (spec_open > .01)
        reduced = stats(spec_off-spec_blocked, obstructed)
        retained = stats(np.divide(spec_blocked, spec_open, out=np.zeros_like(spec_open), where=spec_open>0), visible)
        check("blocked_reflection_directions_reduce_environment_specular",
              reduced["pixels"] >= 8 and reduced["median"] is not None and reduced["median"] > .003,
              {"obstructed_pixels": reduced, "scope": "environment-only pass; displayed luminance after tonemap"})
        check("visible_sky_reflections_survive_mountain_sun_shadow",
              retained["pixels"] >= 8 and retained["median"] is not None and retained["median"] > .5,
              {"retained_visible_direction_ratio": retained})

    perf = []
    if args.group in ("all", "perf"):
        for views in (1, 4):
            names = [f"perf-{views}-{index}-{enabled}" for index, enabled in enumerate(("on", "off", "off", "on"))]
            same_pose(*names)
            group = []
            for name in names:
                report = reports[name]
                mode = next(m for m in report["modes"] if m["debug"] == 0)
                group.append({"name": name, "gpu_pass_sum": mode["gpu_pass_sum"], "wall": mode["wall"]})
            for measure in ("gpu_pass_sum", "wall"):
                if any(item[measure] is None for item in group):
                    perf.append({"views": views, "metric": measure, "available": False}); continue
                medians = [item[measure]["median_ms"] for item in group]
                enabled, disabled = statistics.mean((medians[0],medians[3])), statistics.mean((medians[1],medians[2]))
                perf.append({"views": views, "metric": measure, "available": True, "order": "on/off/off/on",
                             "run_medians_ms": medians, "on_mean_of_run_medians_ms": enabled,
                             "off_mean_of_run_medians_ms": disabled, "delta_ms": enabled-disabled,
                             "delta_percent": (enabled/disabled-1)*100 if disabled>0 else None,
                             "within_on_drift_ms": abs(medians[3]-medians[0]), "within_off_drift_ms": abs(medians[2]-medians[1])})

    hashes = {name: digest(out/name/"report.json") for name in reports}
    value = {"schema": 1, "status": "pass" if all(c["status"]=="pass" for c in checks) else "fail",
             "group": args.group, "checks": checks, "performance_abba": perf, "report_sha256": hashes,
             "acceptance_scope": ["Thresholds test direction/locality of the change, not photometric accuracy.",
                                  "Images are AgX-tonemapped; linear display luma is not scene radiance.",
                                  "A failure can mean inadequate visible fixture coverage; inspect preserved masks/captures before changing thresholds.",
                                  "Synthetic one/four-camera cost does not replace a full-route ABBA benchmark."]}
    write_json(out / "analysis.json", value)
    print(json.dumps({"status": value["status"], "checks": len(checks), "performance_abba": perf}, indent=2))
    if value["status"] != "pass":
        raise RuntimeError("Lighting regression checks failed; see analysis.json and captures")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    p = sub.add_parser("plan", help="write commands only; never starts a GPU process")
    p.add_argument("--exe", required=True, help="compiled lighting_fixture executable (may be built later)")
    p.add_argument("--out", required=True, help="fresh plan and evidence directory")
    p.add_argument("--capture-size", default="1920x1080")
    p.set_defaults(function=plan)
    for action, function in (("run", run), ("analyze", analyze)):
        p = sub.add_parser(action)
        p.add_argument("--plan", required=True)
        p.add_argument("--group", choices=("capture", "perf", "all"), default="all")
        if action == "run":
            p.add_argument("--timeout", type=float, default=300, help="per-job timeout in seconds")
        p.set_defaults(function=function)
    args = parser.parse_args()
    try:
        args.function(args)
    except Exception as error:
        print(f"lighting regression: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
