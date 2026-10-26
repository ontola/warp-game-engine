#!/usr/bin/env python3
"""Read explicitly selected developer/scenario runs; report each without averaging workflows."""
from __future__ import annotations
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import hashlib
import html
import json
import math
from pathlib import Path
import sys


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8-sig"), parse_constant=lambda value: (_ for _ in ()).throw(ValueError(f"nonfinite JSON: {value}")))


def finite(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value >= 0


def ms(value, scale=1):
    return value * scale if finite(value) else None


def startup_from_log(path):
    found = []
    if path.is_file():
        with path.open(encoding="utf-8", errors="replace") as lines:
            for line in lines:
                try:
                    row = json.loads(line)
                    if isinstance(row, dict) and row.get("kind") in ("cli_startup", "renderer_startup"):
                        found.append(row)
                except ValueError:
                    pass
    return found


def startup_summary(records):
    # CLI events are cumulative snapshots; retain the latest startup partition,
    # not the sum of first-loading/ready/first-scene/run-end elapsed timestamps.
    unique = {json.dumps(row, sort_keys=True): row for row in records if isinstance(row, dict)}
    records = list(unique.values())
    cli = [r for r in records if r.get("kind") == "cli_startup"]
    complete = [r for r in cli if r.get("startup_complete")]
    latest = (complete or cli)[-1] if cli else None
    renderers = [r for r in records if r.get("kind") == "renderer_startup"]
    return {"cli_partition": latest,
            "cli_startup_ms": ms(latest.get("partition_end_ms")) if latest and latest.get("startup_complete") else None,
            "renderer_constructors": renderers,
            "first_renderer_cpu_ms": ms(renderers[0].get("total_cpu_ms")) if renderers else None,
            "scope": "CLI snapshots are not additive. Renderer constructor/worker phases nest or overlap the startup timeline. No sum is presented."}


def cache_fields(value, prefix=""):
    """Keep measured cache state/counters, not a guessed warm/cold classification."""
    found = {}
    if isinstance(value, dict):
        for key, child in value.items():
            path = f"{prefix}.{key}" if prefix else key
            if "cache" in key.lower() or key in ("foliage_sun", "ground_cover_streaming", "canopy_updated_levels"):
                found[path] = child
            elif isinstance(child, (dict, list)):
                found.update(cache_fields(child, path))
    elif isinstance(value, list):
        for i, child in enumerate(value):
            found.update(cache_fields(child, f"{prefix}[{i}]"))
    return found


def stable_arguments(arguments):
    result = []
    paths = {"--state-out", "--state-trace", "--frame-profile", "--gpu-profile", "--hud-profile", "--capture", "--scenario-output"}
    i = 0
    while i < len(arguments):
        item = arguments[i]
        result.append(item)
        if item in paths and i + 1 < len(arguments):
            result.append("<output>")
            i += 1
        i += 1
    return result


def profile_evidence(directory, warnings):
    path = next((directory / name for name in ("frames.jsonl", "frame-profile.jsonl") if (directory / name).is_file()), None)
    if path is None:
        return {"path": None, "capture_readback_ms": None, "cache_records": 0, "cache_last": {}, "actual_gpu_configurations": []}
    if path.stat().st_size > 64 * 1024 * 1024:
        warnings.append(f"Profile exceeds 64 MiB; keep existing budget report and inspect separately: {path}")
        return {"path": str(path), "not_scanned": True}
    captures = []
    layouts = Counter()
    cache_count, last_cache = 0, {}
    malformed = 0
    with path.open(encoding="utf-8-sig") as lines:
        for line in lines:
            try:
                row = json.loads(line)
            except ValueError:
                malformed += 1
                continue
            if not isinstance(row, dict):
                malformed += 1
                continue
            if row.get("kind") == "cpu_frame":
                cost = row.get("sections_ms", {}).get("diagnostic_readback")
                if finite(cost) and cost > 0:
                    captures.append({"frame_id": row.get("frame_id"), "section": row.get("section"), "wall_ms": cost})
            if row.get("kind") == "gpu_frame" and isinstance(row.get("output_size"), list):
                layouts[(row.get("views"), tuple(row["output_size"]))] += 1
            cache = cache_fields(row)
            if cache:
                cache_count += 1
                last_cache.update(cache)
    if malformed:
        warnings.append(f"{path}: {malformed} invalid records; partial evidence only")
    return {"path": str(path), "capture_readback_ms": sum(r["wall_ms"] for r in captures),
            "capture_readback_events": captures, "cache_records": cache_count, "cache_last": last_cache,
            "actual_gpu_configurations": [{"views": views, "output_size": list(size), "samples": n} for (views, size), n in layouts.items()]}


def load_run(path, workflow):
    manifest = read_json(path)
    directory = path.parent
    warnings = []
    if workflow == "scenario" and "scenario_sha256" not in manifest:
        return None
    startup = manifest.get("startup_profiles") or []
    saved = directory / "startup.json"
    if saved.is_file():
        value = read_json(saved)
        startup += value if isinstance(value, list) else value.get("records", [])
    startup += startup_from_log(directory / ("stderr.log" if workflow == "scenario" else "play.stderr.log"))
    start = startup_summary(startup)
    final_path = directory / "final-state.json"
    final = read_json(final_path) if final_path.is_file() else {}
    startup_report = manifest.get("startup_report") or final.get("startup_report") or {}
    profile = profile_evidence(directory, warnings)
    budget_path = directory / "budget" / "summary.json"
    budget = read_json(budget_path) if budget_path.is_file() else None
    clean_path = directory / "budget-clean" / "summary.json"
    clean = read_json(clean_path) if clean_path.is_file() else None
    shader_before = manifest.get("shader_provenance")
    shader_after = manifest.get("shader_provenance_after")
    shader_identity = None
    if shader_before:
        shader_identity = {"before": shader_before.get("content_sha256"),
                           "after": shader_after.get("content_sha256") if shader_after else None,
                           "stable": manifest.get("shader_provenance_stable")}
        if manifest.get("shader_provenance_stable") is not True:
            warnings.append("Shader inputs changed, were unreadable, or lack a final snapshot; this is not verified stable-shader evidence")
            # Do not silently group two unverified or mixed-generation runs.
            shader_identity["unverified_run"] = str(path)
    else:
        environment = manifest.get("renderer_environment", manifest.get("graphics_environment", {}))
        if "KERN_SHADER_DIR" in environment:
            warnings.append("Shader override content was not recorded; identical executable/directory does not establish identical shaders")
            shader_identity = {"unverified_run": str(path)}
    config = {"workflow": workflow, "renderer_environment": manifest.get("renderer_environment", manifest.get("graphics_environment", {})),
              "profile": manifest.get("profile"), "cargo_features": manifest.get("cargo_features"),
              "scenario_sha256": manifest.get("scenario_sha256"), "visible": manifest.get("visible"),
              "arguments": stable_arguments(manifest.get("arguments", [])),
              "shader_provenance": shader_identity,
              "actual_gpu_configurations": [{k: v for k, v in layout.items() if k != "samples"}
                                            for layout in profile.get("actual_gpu_configurations", [])]}
    config_id = hashlib.sha256(json.dumps(config, sort_keys=True).encode()).hexdigest()[:12]
    if workflow == "scenario":
        status = "completed" if manifest.get("exit_code") == 0 and manifest.get("completed_expected_frames") and manifest.get("completed_expected_captures") else "incomplete_or_failed"
        if manifest.get("timed_out"):
            status = "timeout"
    else:
        status = manifest.get("status", "unknown")
    requested = manifest.get("requested_output_size")
    if requested and profile.get("actual_gpu_configurations") and any(layout["output_size"] != requested for layout in profile["actual_gpu_configurations"]):
        warnings.append("Actual GPU size differs from requested output; not exact-resolution acceptance")
    cache = cache_fields(final)
    cache.update(cache_fields(startup))
    return {"id": directory.name, "manifest": str(path), "workflow": workflow, "status": status,
            "started_utc": manifest.get("started_utc"), "configuration_id": config_id, "configuration": config,
            "executable_sha256": manifest.get("executable_sha256", manifest.get("sha256")),
            "source_revision": manifest.get("source_revision", manifest.get("git", {}).get("head")),
            "source_dirty": manifest.get("source_dirty", manifest.get("git", {}).get("dirty")),
            "timing_ms": {"build": ms(manifest.get("build_ms_external")) if workflow == "scenario" else ms(manifest.get("build_seconds"), 1000),
                          "startup_to_visible_poll": ms(manifest.get("startup_seconds"), 1000),
                          "cli_startup_partition": start["cli_startup_ms"], "first_renderer_constructor": start["first_renderer_cpu_ms"],
                          "graphics_ready_elapsed": ms(startup_report.get("elapsed_ms")),
                          "process_elapsed": ms(manifest.get("process_elapsed_ms")),
                          "runner_elapsed": ms(manifest.get("runner_elapsed_ms")),
                          "report_analysis": ms(manifest.get("analysis_elapsed_ms")),
                          "diagnostic_capture_readback": profile.get("capture_readback_ms")},
            "startup": start, "startup_report": startup_report, "profile": profile,
            "shader_provenance": {"before": shader_before, "after": shader_after},
            "budget": budget, "budget_clean": clean,
            "cache_snapshot": cache, "cache_scope": "Snapshots/cumulative counters; not added across records. Missing is unknown, not zero; held last_update_ms is not a fresh per-frame sample.",
            "warnings": warnings}


def collect(dev_roots, scenario_roots):
    rows, errors, visited = [], [], set()
    for workflow, roots, filename in [("dev", dev_roots, "iteration.json"), ("scenario", scenario_roots, "run.json")]:
        for root in roots:
            root = root.resolve()
            if not root.is_dir():
                raise ValueError(f"explicit root does not exist: {root}")
            for path in sorted(root.rglob(filename)):
                path = path.resolve()
                if path in visited:
                    continue
                visited.add(path)
                try:
                    row = load_run(path, workflow)
                    if row:
                        rows.append(row)
                except (OSError, ValueError, TypeError, KeyError) as error:
                    errors.append({"path": str(path), "error": str(error)})
    rows.sort(key=lambda r: (r["started_utc"] or "", r["id"]))
    groups = defaultdict(list)
    for row in rows:
        groups[row["configuration_id"]].append(row["id"])
    return {"schema": 1, "generated_utc": datetime.now(timezone.utc).isoformat(),
            "roots": {"dev": [str(p.resolve()) for p in dev_roots], "scenario": [str(p.resolve()) for p in scenario_roots]},
            "scope": "Read-only snapshots of explicitly selected workflows. No cross-run averages or inferred cache speedups. Same configuration does not certify equal driver/cache/system state.",
            "runs": rows, "configuration_groups": dict(groups), "errors": errors}


def render_html(report):
    esc = lambda value: html.escape(str(value), quote=True)
    fmt = lambda value: "n/a" if value is None else f"{value:,.1f}"
    cards = []
    for run in report["runs"]:
        timing = ''.join(f'<tr><td>{esc(name)}</td><td>{fmt(value)}</td></tr>' for name, value in run["timing_ms"].items())
        renderer = run["startup"]["renderer_constructors"]
        phases = []
        for i, constructor in enumerate(renderer):
            total = constructor.get("total_cpu_ms") or 1
            parts = sorted(constructor.get("sections", []), key=lambda r: r.get("cpu_ms", 0), reverse=True)
            phases.append(f'<h3>Renderer constructor {i + 1}</h3><p>Host CPU wall time, excluding earlier decode and GPU completion.</p>')
            phases.extend(f'<div class="phase"><span>{esc(p["name"])}: {fmt(p.get("cpu_ms"))} ms</span><i style="width:{min(100, 100*p.get("cpu_ms",0)/total):.2f}%"></i></div>' for p in parts)
        cards.append(f'<section><h2>{esc(run["id"])}</h2><p>{esc(run["workflow"])} · {esc(run["status"])} · config {run["configuration_id"]}</p><p class="muted">{esc(run["manifest"])}<br>Executable {esc(run["executable_sha256"])}</p><table><tr><th>Timing scope (never add nested rows)</th><th>ms</th></tr>{timing}</table>{"".join(phases)}<details><summary>Startup timeline / worker scope</summary><pre>{esc(json.dumps({"startup":run["startup"],"worker":run["startup_report"]},indent=2))}</pre></details><details><summary>Cache evidence</summary><pre>{esc(json.dumps({"snapshot":run["cache_snapshot"],"profile":run["profile"],"scope":run["cache_scope"]},indent=2))}</pre></details><details><summary>Exact configuration and warnings</summary><pre>{esc(json.dumps({"config":run["configuration"],"warnings":run["warnings"]},indent=2))}</pre></details></section>')
    return f'<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Warp iteration measurements</title><style>body{{font:15px/1.5 system-ui;max-width:1150px;margin:30px auto;padding:20px;background:#111722;color:#e7edf7}}section{{padding:22px;margin:24px 0;border:1px solid #39485c;border-radius:8px;background:#1a2331}}table{{width:100%;border-collapse:collapse}}td,th{{padding:5px;text-align:left;border-bottom:1px solid #39485c}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;font-size:12px}}.muted{{color:#aebbd0}}.phase{{margin:5px 0}}.phase i{{display:block;height:6px;background:#6aa7e5}}details{{margin-top:16px}}</style><h1>Warp iteration measurements</h1><p>{esc(report["scope"])}</p><p>{len(report["runs"])} runs, {len(report["configuration_groups"])} distinct configurations; generated {esc(report["generated_utc"])}. Missing measurements are n/a. No workflow averages.</p>{"".join(cards)}<h2>Read errors</h2><pre>{esc(json.dumps(report["errors"],indent=2))}</pre></html>'


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dev-root", action="append", type=Path, default=[])
    parser.add_argument("--scenario-root", action="append", type=Path, default=[])
    parser.add_argument("--out", required=True, type=Path, help="new report directory")
    args = parser.parse_args(argv)
    try:
        if not args.dev_root and not args.scenario_root:
            raise ValueError("provide at least one explicit --dev-root or --scenario-root")
        if args.out.exists():
            raise ValueError("output directory exists; choose a new report path")
        report = collect(args.dev_root, args.scenario_root)
        args.out.mkdir(parents=True, exist_ok=False)
        (args.out / "summary.json").write_text(json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf-8")
        (args.out / "report.html").write_text(render_html(report), encoding="utf-8")
        print(f"{len(report['runs'])} runs / {len(report['configuration_groups'])} configurations; report {args.out / 'report.html'}")
        return 0 if not report["errors"] else 1
    except (OSError, ValueError) as error:
        print(f"iteration-report: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
