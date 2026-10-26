#!/usr/bin/env python3
"""Plan/review controlled color-geometry omissions; never launch the game."""
import argparse
import hashlib
import html
import importlib.util
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import sys


def tool(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


budget = tool("frame-budget")
runner = tool("run-frame-scenario")
CATEGORIES = ("other", "terrain", "trees", "grass", "cars")
FLAG = "KERN_DIAGNOSTIC_HIDE"
SCOPE = ("Marginal effect of omitting one category's color geometry from this exact scene. "
         "Shadows, lighting, physics and effects remain controls. Deltas are not additive category costs, "
         "not per-view GPU timings, and not an acceptable gameplay performance improvement.")
COUNTER_SCOPE = ("Source counts distinguish allocated slots, active instances and all referenced LOD clusters; none is submitted GPU work. "
                 "Per-view retained cluster/pixel counters cover the shared main visibility/raster path only, not transition grass, "
                 "far-field grass or blended glass. Omissions also affect dedicated color passes. Do not divide their marginal time "
                 "by the narrower shared-path counts or call that total category cost.")


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8-sig"), parse_constant=lambda _: (_ for _ in ()).throw(ValueError("nonfinite JSON")))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def summary(values):
    values = list(values)
    return {"n": len(values), "mean_ms": statistics.fmean(values) if values else None,
            "median_ms": budget.percentile(values, .5), "p95_ms": budget.percentile(values, .95),
            "min_ms": min(values) if values else None, "max_ms": max(values) if values else None}


def load_run(directory, category):
    directory = directory.resolve()
    manifest = read_json(directory / "run.json")
    require(manifest.get("exit_code") == 0 and all(manifest.get(k) is True for k in
        ("completed_expected_frames", "completed_expected_captures", "resolution_verified")), f"{directory}: incomplete/unverified scenario")
    expected = "none" if category is None else category
    environment = manifest.get("renderer_environment", {})
    require(environment.get(FLAG) == expected, f"{directory}: expected {FLAG}={expected}")
    require(manifest.get("visible") is True, f"{directory}: hidden synchronous diagnostics cannot enter this timing comparison")
    require(manifest.get("shader_provenance_stable") is True, f"{directory}: shader inputs not verified stable")
    provenance = manifest.get("shader_provenance", {})
    require(provenance.get("verified") is True and provenance.get("content_sha256"), f"{directory}: missing verified shader identity")
    require(manifest.get("shader_provenance_after", {}).get("content_sha256") == provenance["content_sha256"], f"{directory}: changed shader identity")
    records, profile_sha = budget.read_jsonl(directory / "frames.jsonl")
    analyzed = budget.analyze(records, exclude_diagnostics=True)
    require(analyzed["source_format"] == "frame_profile_v1", "legacy synchronous profiles are unsupported")
    require(analyzed["footer"] and not analyzed["footer"].get("writer_dropped_records"), f"{directory}: incomplete/dropped profile")
    require(not analyzed["counts"]["gpu_duplicate_records"] and not analyzed["counts"]["gpu_invalid_records"], f"{directory}: ambiguous GPU records")
    cpu = {r["frame_id"]: r for r in records if r.get("kind") == "cpu_frame" and r.get("frame_id") is not None}
    gpu = {r["frame_id"]: r for r in records if r.get("kind") == "gpu_frame"}
    captures = []
    for capture in manifest.get("captures", []):
        frame = capture["frame_id"]
        path = directory / "captures" / f"frame-{frame:06}.json"
        data = read_json(path)
        require(data.get("frame_id") == frame and frame in cpu, f"{path}: capture frame identity missing/mismatched")
        diagnostic = data.get("render_diagnostic", {})
        mask = 0 if category is None else 1 << CATEGORIES.index(category)
        require(diagnostic.get("enabled") is True and diagnostic.get("mask") == mask and
                diagnostic.get("categories") == ([] if category is None else [category]), f"{path}: category omission not confirmed by renderer")
        require(diagnostic.get("scope") == "color_geometry_only", f"{path}: unsupported diagnostic scope")
        require(diagnostic.get("retained") == ["shadows", "GI", "physics", "effects"], f"{path}: retained controls not confirmed")
        require(isinstance(diagnostic.get("dedicated_passes"), dict), f"{path}: dedicated pass behavior not recorded")
        require(isinstance(diagnostic.get("source_counts"), list) and diagnostic["source_counts"], f"{path}: missing raw source counts")
        require(isinstance(diagnostic.get("views"), list) and len(diagnostic["views"]) == len(data.get("views", [])) > 0,
                f"{path}: missing per-view diagnostic evidence")
        require([row.get("category") for row in diagnostic["source_counts"]] == list(CATEGORIES), f"{path}: incomplete source categories")
        require(diagnostic["dedicated_passes"].get("grass_enabled") is (category != "grass"), f"{path}: dedicated grass omission mismatched")
        sequences = set()
        for index, view in enumerate(diagnostic["views"]):
            require(view.get("view_index") == index and view.get("frame_id") == frame, f"{path}: stale/duplicate diagnostic view")
            require(view.get("viewport") == data["views"][index].get("rect"), f"{path}: diagnostic viewport mismatch")
            require(view.get("output_size") == data.get("source_output_size"), f"{path}: diagnostic output resolution differs from captured source")
            require(isinstance(view.get("render_sequence"), int), f"{path}: render sequence missing")
            sequences.add(view["render_sequence"])
            require([row.get("category") for row in view.get("categories", [])] == list(CATEGORIES), f"{path}: incomplete per-view categories")
            for row in view["categories"]:
                require(all(isinstance(row.get(k), int) and row[k] >= 0 for k in ("instances", "clusters", "triangles", "submitted_vertices", "visibility_pixels")), f"{path}: invalid GPU counters")
                if row["category"] == category:
                    require(all(row[k] == 0 for k in ("instances", "clusters", "triangles", "submitted_vertices", "visibility_pixels")),
                            f"{path}: excluded {category} still has shared geometry/pixels in view {index}")
            require(len(view.get("input_size", [])) == 2 and all(isinstance(n, int) and n > 0 for n in view["input_size"]), f"{path}: invalid diagnostic input resolution")
            require(isinstance(view.get("uncovered_pixels"), int) and view["uncovered_pixels"] >= 0 and
                    sum(row["visibility_pixels"] for row in view["categories"]) + view["uncovered_pixels"] == math.prod(view["input_size"]), f"{path}: visibility pixel accounting does not cover the view")
        require(len(sequences) == 1, f"{path}: diagnostic views came from different renders")
        extension = manifest["capture_config"]["format"]
        require(extension in ("jpg", "png"), "unsupported recorded capture format")
        image_path = directory / "captures" / f"frame-{frame:06}.{extension}"
        require(runner.image_size(image_path) == data.get("capture_size"), f"{image_path}: capture dimensions not verified")
        captures.append({"frame_id": frame, "path": str(path), "sha256": digest(path),
                         "image": str(image_path), "image_sha256": digest(image_path), "data": data})
    require(captures, f"{directory}: at least one actual four-view/camera capture is required")
    require(len({c["frame_id"] for c in captures}) == len(captures), f"{directory}: duplicate captures")
    return {"directory": directory, "manifest": manifest, "records": records, "analysis": analyzed,
            "cpu": cpu, "gpu": gpu, "captures": captures, "profile_sha256": profile_sha}


def compare_identity(base, variant):
    a, b = base["manifest"], variant["manifest"]
    for key in ("executable_sha256", "scenario_sha256", "expected_frames", "actual_output_sizes", "capture_config", "visible", "fullscreen"):
        require(a.get(key) is not None and a[key] == b.get(key), f"comparison mismatch: {key}")
    env = lambda m: {k: v for k, v in m["renderer_environment"].items() if k != FLAG}
    require(env(a) == env(b), f"renderer settings differ outside {FLAG}")
    require(a["shader_provenance"]["content_sha256"] == b["shader_provenance"]["content_sha256"], "shader contents/executable differ")
    require(set(base["cpu"]) == set(variant["cpu"]), "rendered frame IDs differ")
    for frame in base["cpu"]:
        for key in ("race_frame", "target_tick", "round", "section", "section_frame", "warmup", "output_size", "views"):
            require(base["cpu"][frame].get(key) == variant["cpu"][frame].get(key), f"frame {frame}: {key} differs")
        if "render_state_hash" in base["cpu"][frame] or "render_state_hash" in variant["cpu"][frame]:
            require(isinstance(base["cpu"][frame].get("render_state_hash"), str) and base["cpu"][frame]["render_state_hash"] == variant["cpu"][frame].get("render_state_hash"),
                    f"frame {frame}: actual render state hash differs/missing")
    require([c["frame_id"] for c in base["captures"]] == [c["frame_id"] for c in variant["captures"]], "capture frame IDs differ")
    for a, b in zip(base["captures"], variant["captures"]):
        for key in ("frame_id", "race_frame", "round", "seed", "stage_noise_seed", "stage_detail_seed", "section", "section_frame",
                    "source_output_size", "views", "player_positions", "sun_direction", "sun_lux", "sky_lux", "ev100"):
            require(key in a["data"] and a["data"][key] == b["data"].get(key), f"capture {a['frame_id']}: {key} differs/missing")
        require(a["data"]["render_diagnostic"]["scope"] == b["data"]["render_diagnostic"]["scope"], "diagnostic scope differs")
        require(a["data"]["render_diagnostic"]["source_counts"] == b["data"]["render_diagnostic"]["source_counts"], "source scene inventory changed")
        for av, bv in zip(a["data"]["render_diagnostic"]["views"], b["data"]["render_diagnostic"]["views"]):
            for key in ("view_index", "viewport", "input_size", "output_size"):
                require(av.get(key) == bv.get(key), f"capture {a['frame_id']} view {av['view_index']}: diagnostic {key} differs")


def sections(run):
    analysis = run["analysis"]
    return {s.get("name", "all"): s for s in analysis.get("sections", [analysis])}


def delta(base, variant):
    return {"baseline": summary(base), "excluded": summary(variant),
            "baseline_minus_excluded": summary(a - b for a, b in zip(base, variant))}


def compare_runs(base, variant, category):
    compare_identity(base, variant)
    aa, bb = sections(base), sections(variant)
    require(set(aa) == set(bb), "scenario sections differ")
    result = []
    for name, a in aa.items():
        b = bb[name]
        eligible_a = {r["frame_id"] for r in a["timeline"]}
        eligible_b = {r["frame_id"] for r in b["timeline"]}
        eligible = eligible_a & eligible_b
        matched = sorted(f for f in eligible if f in base["gpu"] and f in variant["gpu"] and
                         budget.duration(base["gpu"][f].get("covered_interval_ms")) and budget.duration(variant["gpu"][f].get("covered_interval_ms")))
        require(matched, f"{name}: no matched eligible GPU frames")
        for frame in matched:
            for key in ("output_size", "views"):
                require(base["gpu"][frame].get(key) == variant["gpu"][frame].get(key), f"GPU frame {frame}: {key} differs")
        pa = [budget.pass_map(base["gpu"][f]) for f in matched]
        pb = [budget.pass_map(variant["gpu"][f]) for f in matched]
        names = sorted({p for m in pa + pb for p in m})
        passes = [{"name": p, **delta([m.get(p, 0.) for m in pa], [m.get(p, 0.) for m in pb])} for p in names]
        result.append({"name": name, "matched_frame_ids": matched, "matched_n": len(matched),
            "eligible_baseline_n": len(eligible_a), "eligible_excluded_n": len(eligible_b),
            "missing_or_invalid_gpu_pairs": len(eligible) - len(matched),
            "gpu_covered_interval": delta([base["gpu"][f]["covered_interval_ms"] for f in matched], [variant["gpu"][f]["covered_interval_ms"] for f in matched]),
            "gpu_passes": passes, "baseline_selection": a["selection"], "excluded_selection": b["selection"],
            "baseline_counts": a["counts"], "excluded_counts": b["counts"]})
    hashed = sum(isinstance(r.get("render_state_hash"), str) for r in base["cpu"].values())
    return {"category": category, "scope": SCOPE, "counter_scope": COUNTER_SCOPE, "sections": result,
            "baseline": evidence(base), "excluded": evidence(variant),
            "render_state_hash_frames": hashed, "rendered_frames": len(base["cpu"]),
            "camera_evidence": f"Exact captured cameras/player positions checked; {hashed}/{len(base['cpu'])} rendered frames also have equal camera/lighting/pose hashes. Physics ticks and section/frame IDs match for every rendered frame; unhashed intervening cameras are not independently verified."}


def evidence(run):
    return {"directory": str(run["directory"]), "run_sha256": digest(run["directory"] / "run.json"),
            "profile_sha256": run["profile_sha256"], "manifest": run["manifest"],
            "captures": run["captures"], "warnings": run["analysis"]["warnings"],
            "gpu_ring_drops": run["analysis"]["gpu_observed_drops"], "gpu_ring_errors": run["analysis"]["gpu_observed_errors"]}


def render_html(report):
    esc = lambda x: html.escape(str(x))
    fmt = lambda x: "n/a" if x is None else f"{x:.3f}"
    blocks = []
    entries = [("Without " + c["category"], c) for c in report["comparisons"]]
    if report.get("baseline_repeat"):
        entries.insert(0, ("Baseline repeat (measurement variability)", report["baseline_repeat"]))
    for title, comparison in entries:
        rows = []
        for section in comparison["sections"]:
            for label, metric in [("GPU covered interval", section["gpu_covered_interval"])] + [(p["name"], p) for p in section["gpu_passes"]]:
                change = metric["baseline_minus_excluded"]
                rows.append(f'<tr><td>{esc(section["name"])}</td><td>{esc(label)}</td><td>{section["matched_n"]}</td><td>{fmt(metric["baseline"]["median_ms"])}</td><td>{fmt(metric["excluded"]["median_ms"])}</td><td>{fmt(change["median_ms"])}</td><td>{fmt(change["p95_ms"])}</td></tr>')
        blocks.append(f'<section><h2>{esc(title)}</h2><table><tr><th>Section</th><th>Interval</th><th>Pairs</th><th>Baseline ms</th><th>Compared run ms</th><th>Median paired delta ms</th><th>p95 paired delta ms</th></tr>{"".join(rows)}</table><p>{esc(comparison["camera_evidence"])}</p><p>{esc(COUNTER_SCOPE)}</p><details><summary>Raw source inventory and per-view evidence</summary><pre>{esc(json.dumps({side:comparison[side]["captures"] for side in ("baseline","excluded")},indent=2))}</pre></details></section>')
    return '<!doctype html><meta charset="utf-8"><title>Render category diagnostics</title><style>body{font:15px system-ui;max-width:1300px;margin:40px auto;padding:20px}table{border-collapse:collapse;width:100%}th,td{padding:7px;border-bottom:1px solid #ccc;text-align:right}td:first-child,td:nth-child(2){text-align:left}pre{white-space:pre-wrap}section{margin:35px 0}</style><h1>Render category omissions</h1><p>'+esc(SCOPE)+'</p><p>Positive delta means baseline took longer; negative results are retained. Shared passes are not split into additive category bars. Full manifests, denominator counts and provenance are in report.json.</p>'+''.join(blocks)


def plan(args):
    categories = args.categories.split(",")
    require(categories and len(set(categories)) == len(categories) and all(c in CATEGORIES for c in categories), "invalid/duplicate category")
    require(args.scenario.is_file() and args.exe.is_file(), "scenario and executable must exist")
    environment = {k: v for k, v in os.environ.items() if runner.renderer_key(k)}
    overrides = read_json(args.env_file) if args.env_file else {}
    require(isinstance(overrides, dict), "environment must be an object")
    for key, value in overrides.items():
        require(runner.renderer_key(key) and (value is None or isinstance(value, (str, int, float, bool))), "unsupported environment setting")
        require(not isinstance(value, float) or math.isfinite(value), "nonfinite environment value")
        environment[key] = value
    args.out.mkdir(parents=True, exist_ok=False)
    scenario = args.out / "scenario.toml"
    scenario.write_bytes(args.scenario.read_bytes())
    commands = []
    for label, category in [("baseline", None)] + [(c, c) for c in categories] + [("baseline-repeat", None)]:
        env_path = args.out / f"environment-{label}.json"
        env_path.write_text(json.dumps({**environment, FLAG: category or "none"}, indent=2) + "\n", encoding="utf-8")
        commands.append({"category": category, "argv": [sys.executable, str(Path(__file__).with_name("run-frame-scenario.py").resolve()),
            str(scenario.resolve()), "--exe", str(args.exe.resolve()), "--env-file", str(env_path.resolve()), "--out", str((args.out / label).resolve())]})
    compare = [sys.executable, str(Path(__file__).resolve()), "compare", "--baseline", str((args.out / "baseline").resolve()), "--baseline-repeat", str((args.out / "baseline-repeat").resolve()), "--out", str((args.out / "comparison").resolve())]
    for category in categories:
        compare += ["--without", f"{category}={(args.out/category).resolve()}"]
    result = {"schema": 1, "scope": SCOPE, "commands": commands, "comparison_argv": compare,
              "execution": "Plan only. Run commands serially with exclusive GPU access. Each scenario runner preserves exact manifests; comparison rejects differences outside the omission flag."}
    (args.out / "plan.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    for command in [c["argv"] for c in commands] + [compare]:
        print(subprocess.list2cmdline(command))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("plan", help="write commands/env files only; no process is launched")
    p.add_argument("scenario", type=Path); p.add_argument("--exe", required=True, type=Path)
    p.add_argument("--env-file", type=Path); p.add_argument("--categories", default="trees,grass,terrain,cars,other")
    p.add_argument("--out", required=True, type=Path)
    c = commands.add_parser("compare", help="compare completed controlled scenario runs")
    c.add_argument("--baseline", required=True, type=Path); c.add_argument("--without", action="append", required=True, help="category=run-directory")
    c.add_argument("--baseline-repeat", type=Path, help="optional repeated diagnostic baseline with the same settings")
    c.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "plan":
            plan(args)
        else:
            baseline = load_run(args.baseline, None)
            comparisons = []
            seen = set()
            for value in args.without:
                category, separator, path = value.partition("=")
                require(separator and category in CATEGORIES and category not in seen, "--without requires unique category=directory")
                seen.add(category)
                comparisons.append(compare_runs(baseline, load_run(Path(path), category), category))
            args.out.mkdir(parents=True, exist_ok=False)
            repeat = compare_runs(baseline, load_run(args.baseline_repeat, None), "baseline_repeat") if args.baseline_repeat else None
            report = {"schema": 1, "scope": SCOPE, "comparisons": comparisons, "baseline_repeat": repeat}
            (args.out / "report.json").write_text(json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf-8")
            (args.out / "report.html").write_text(render_html(report), encoding="utf-8")
            print(args.out / "report.html")
        return 0
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"render-category-report: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
