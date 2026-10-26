#!/usr/bin/env python3
"""Analyze Sideways frame-profile JSONL without turning pass sums into FPS.

Python 3.10+, standard library only. GPU records are joined by frame_id, not
arrival order. See frame-budget.md for the measurement and denominator contract.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import hashlib
import html
import json
import math
from pathlib import Path
import statistics
import sys


def number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def duration(value):
    return number(value) and value >= 0


def percentile(values, fraction):
    """Linear interpolation between ordered samples (Hyndman/Fan type 7)."""
    ordered = sorted(values)
    if not ordered:
        return None
    position = (len(ordered) - 1) * fraction
    lo = math.floor(position)
    hi = math.ceil(position)
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (position - lo)


def stats(values, budget):
    values = list(values)
    if not values:
        return {"n": 0, "mean_ms": None, "median_ms": None, "p95_ms": None,
                "p99_ms": None, "max_ms": None, "over_budget_n": 0,
                "over_budget_fraction": None}
    over = sum(value > budget for value in values)
    return {"n": len(values), "mean_ms": statistics.fmean(values),
            "median_ms": percentile(values, .5), "p95_ms": percentile(values, .95),
            "p99_ms": percentile(values, .99), "max_ms": max(values),
            "over_budget_n": over, "over_budget_fraction": over / len(values)}


def read_jsonl(path):
    records = []
    raw = path.read_bytes()
    text = raw.decode("utf-8-sig")
    for line_no, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        try:
            item = json.loads(line, parse_constant=lambda value: (_ for _ in ()).throw(
                ValueError(f"non-finite JSON constant {value}")))
        except (ValueError, json.JSONDecodeError) as error:
            raise ValueError(f"{path}:{line_no}: {error}") from error
        if not isinstance(item, dict):
            raise ValueError(f"{path}:{line_no}: record must be an object")
        records.append(item)
    if not records:
        raise ValueError("profile contains no JSON objects")
    return records, hashlib.sha256(raw).hexdigest()


def pass_map(record):
    pairs = record.get("gpu_pass_ms")
    if not isinstance(pairs, list):
        return None
    result = defaultdict(float)
    for pair in pairs:
        if (not isinstance(pair, list) or len(pair) != 2 or
                not isinstance(pair[0], str) or not duration(pair[1])):
            return None
        result[pair[0]] += pair[1]
    return dict(result) if result else None


def contributors(samples, budget):
    """Absent conditional work is zero only in otherwise valid complete samples."""
    names = sorted({name for sample in samples for name in sample})
    result = []
    for name in names:
        measured = [sample.get(name, 0.0) for sample in samples]
        result.append({"name": name, "observed_n": sum(name in s for s in samples),
                       "denominator_n": len(samples), **stats(measured, budget)})
    return sorted(result, key=lambda row: (-row["mean_ms"], row["name"]))


def ring_history(rows):
    """Cumulative counters reset with each renderer; never add each poll."""
    result = []
    current = None
    for row in rows:
        reset = current is not None and any(
            duration(row.get(key)) and row[key] < current["last"].get(key, 0)
            for key in ("dropped", "errors"))
        if current is None or current["round"] != row.get("round") or reset:
            current = {"round": row.get("round"), "last": row, "dropped": 0, "errors": 0}
            result.append(current)
        current["last"] = row
        for key in ("dropped", "errors"):
            if duration(row.get(key)):
                current[key] = max(current[key], row[key])
    return result


def analyze(records, target_fps=120.0, skip_frames=0, include_paused=False,
            exclude_diagnostics=False, section=None, _group=True):
    if not number(target_fps) or target_fps <= 0:
        raise ValueError("target FPS must be finite and positive")
    if skip_frames < 0:
        raise ValueError("skip-frames must be nonnegative")
    budget = 1000.0 / target_fps
    warnings = []
    headers = [r for r in records if r.get("kind") in ("frame_profile_header", "header")]
    footers = [r for r in records if r.get("kind") == "frame_profile_footer"]
    status_rows = [r for r in records if r.get("kind") == "gpu_status"]
    cpu = [r for r in records if r.get("kind") == "cpu_frame"]
    legacy = not cpu and any(r.get("kind") == "frame" and "gpu_pass_ms" in r for r in records)
    gpu_records = [r for r in records if r.get("kind") == "gpu_frame"]
    unknown_kinds = Counter(str(r.get("kind")) for r in records if r.get("kind") not in {
        "frame_profile_header", "frame_profile_footer", "header", "cpu_frame",
        "gpu_frame", "gpu_status", "frame", "stage_load"})
    if unknown_kinds:
        warnings.append("Unrecognized record kinds retained in counts: " + str(dict(unknown_kinds)))
    if legacy:
        warnings.append("Legacy --gpu-profile: synchronous diagnostic measurement. CPU work and real cadence are unavailable; frame_time_ms is clamped/synthetic and intentionally ignored.")
        old = [r for r in records if r.get("kind") == "frame" and "gpu_pass_ms" in r]
        cpu = [{"call_id": i, "frame_id": r.get("rendered_frame", i),
                "phase": "paused" if r.get("paused") else "gameplay", "presented": False,
                "views": len(r["views"]) if isinstance(r.get("views"), list) else r.get("views"),
                "output_size": r.get("output_size"), "legacy": True} for i, r in enumerate(old)]
        gpu_records = [{**r, "kind": "gpu_frame", "frame_id": r.get("rendered_frame", i)}
                       for i, r in enumerate(old) if r.get("timings_available", True)]
    if not cpu:
        raise ValueError("no cpu_frame or legacy GPU frame records found")

    # Strict call ordering prevents hidden resets/gaps from inventing cadence.
    seen_calls = set()
    seen_frames = set()
    previous_call = None
    for row in cpu:
        call_id = row.get("call_id")
        if not isinstance(call_id, int) or isinstance(call_id, bool):
            raise ValueError("cpu_frame requires an integer call_id")
        if call_id in seen_calls or (previous_call is not None and call_id <= previous_call):
            raise ValueError("CPU call IDs must be unique and increasing; analyze separate runs separately")
        seen_calls.add(call_id)
        previous_call = call_id
        frame_id = row.get("frame_id")
        if frame_id is not None:
            if not isinstance(frame_id, int) or isinstance(frame_id, bool) or frame_id in seen_frames:
                raise ValueError("rendered frame IDs must be unique integers")
            seen_frames.add(frame_id)

    phases = {}
    phase_rows = defaultdict(list)
    for row in cpu:
        phase_rows[str(row.get("phase", "unknown"))].append(row)
    for phase, rows in phase_rows.items():
        phases[phase] = {"calls": len(rows), "rendered": sum(r.get("frame_id") is not None for r in rows),
                         "presented": sum(r.get("presented") is True for r in rows),
                         "cpu_work": stats((r["cpu_frame_ms"] for r in rows if duration(r.get("cpu_frame_ms"))), budget)}

    selected = []
    skipped = 0
    skipped_by_section = Counter()
    section_names = list(dict.fromkeys(row.get("section") for row in cpu if isinstance(row.get("section"), str)))
    if section is not None and section not in section_names:
        raise ValueError(f"section {section!r} not present; available: {section_names}")
    for row in cpu:
        eligible = row.get("phase") in (("gameplay", "paused") if include_paused else ("gameplay",))
        eligible = eligible and not row.get("warmup", False) and (section is None or row.get("section") == section)
        if eligible and row.get("frame_id") is not None and not (exclude_diagnostics and row.get("diagnostic_sync")):
            section_key = row.get("section")
            if skipped_by_section[section_key] < skip_frames:
                skipped += 1
                skipped_by_section[section_key] += 1
            else:
                selected.append(row)
    selected_ids = {row["frame_id"] for row in selected}
    selected_calls = {row["call_id"] for row in selected}
    gpu = {}
    invalid_gpu = 0
    duplicate_gpu = 0
    for row in gpu_records:
        frame_id = row.get("frame_id")
        passes = pass_map(row)
        if not isinstance(frame_id, int) or isinstance(frame_id, bool) or passes is None:
            invalid_gpu += 1
            continue
        if frame_id in gpu:
            duplicate_gpu += 1
            continue
        gpu[frame_id] = (row, passes)
    if duplicate_gpu:
        warnings.append(f"Ignored {duplicate_gpu} duplicate GPU records; first result per frame ID used.")
    if invalid_gpu:
        warnings.append(f"Rejected {invalid_gpu} invalid/empty GPU records; they are not zero-duration samples.")
    matched = [gpu[row["frame_id"]] for row in selected if row["frame_id"] in gpu]
    gpu_covered = []
    for row, _ in matched:
        value = row.get("covered_interval_ms", row.get("gpu_covered_ms"))
        if duration(value):
            gpu_covered.append(value)
    gpu_sums = [sum(passes.values()) for _, passes in matched]
    if matched and not gpu_covered:
        warnings.append("GPU covered interval unavailable. Pass-duration sum is diagnostic work, not GPU critical-path time or FPS.")
    elif len(gpu_covered) < len(matched):
        warnings.append(f"{len(matched) - len(gpu_covered)} valid GPU records lack a covered interval; covered-interval statistics use their own smaller denominator.")
    for row, passes in matched:
        declared = row.get("instrumented_gpu_ms")
        if duration(declared) and abs(declared - sum(passes.values())) > max(.01, declared * .001):
            warnings.append(f"GPU pass sum differs from instrumented_gpu_ms for frame {row['frame_id']}.")

    cpu_work = []
    cpu_execution = []
    cpu_parts = []
    invalid_partitions = 0
    cadence_after_diagnostic = 0
    cadence = []
    previous = None
    timeline = []
    for row in cpu:
        if row["call_id"] in selected_calls:
            execution = row.get("cpu_execution_ms")
            if duration(execution):
                cpu_execution.append(execution)
            work = row.get("cpu_frame_ms")
            if duration(work):
                cpu_work.append(work)
                sections = row.get("sections_ms")
                if (isinstance(sections, dict) and sections and
                        all(isinstance(k, str) and duration(v) for k, v in sections.items()) and
                        abs(sum(sections.values()) - work) <= max(.05, work * .005)):
                    cpu_parts.append(sections)
                else:
                    invalid_partitions += 1
            # Cadence is entry-to-entry, not scanout or a sum of measured work.
            interval = None
            contiguous = (previous is not None and previous["call_id"] in selected_calls and
                          previous["call_id"] + 1 == row["call_id"] and
                          previous.get("presented") is True and row.get("presented") is True and
                          previous.get("section") == row.get("section") and
                          previous.get("phase") == row.get("phase"))
            if (exclude_diagnostics and previous is not None and previous.get("diagnostic_sync") and
                    previous["call_id"] + 1 == row["call_id"]):
                cadence_after_diagnostic += 1
            if contiguous:
                start, prior = row.get("start_ms"), previous.get("start_ms")
                if number(start) and number(prior) and start > prior:
                    interval = start - prior
                    declared = row.get("cadence_ms")
                    if duration(declared) and abs(declared - interval) > max(.05, interval * .001):
                        warnings.append(f"Cadence field differs from monotonic starts at call {row['call_id']}; starts used.")
                elif number(start) and number(prior):
                    warnings.append(f"Non-increasing monotonic start at call {row['call_id']}; cadence interval excluded.")
                elif duration(row.get("cadence_ms")) and row["cadence_ms"] > 0:
                    interval = row["cadence_ms"]
            if interval is not None:
                cadence.append(interval)
            item = gpu.get(row["frame_id"])
            covered = item[0].get("covered_interval_ms", item[0].get("gpu_covered_ms")) if item else None
            timeline.append({"call_id": row["call_id"], "frame_id": row["frame_id"],
                             "section": row.get("section"),
                             "start_ms": row.get("start_ms"),
                             "cpu_ms": work if duration(work) else None,
                             "cpu_execution_ms": execution if duration(execution) else None,
                             "cadence_ms": interval,
                             "gpu_covered_ms": covered if duration(covered) else None,
                             "gpu_pass_sum_ms": sum(item[1].values()) if item else None})
        previous = row
    if invalid_partitions:
        warnings.append(f"{invalid_partitions} CPU records lack a valid non-overlapping section partition; excluded from contributor denominators, not from CPU-total statistics.")
    statuses = status_rows[-1] if status_rows else {}
    status_history = ring_history(status_rows)
    footer = footers[-1] if footers else {}
    if not legacy and not footer:
        warnings.append("No footer: run may still be active or truncated. Final pending GPU and writer-drop counts are not confirmed.")
    if footer.get("writer_dropped_records", 0):
        warnings.append("Background writer dropped records; missing CPU calls/GPU records cannot be reconstructed from surviving records.")
    configurations = Counter((row.get("views"), tuple(row.get("output_size") or [])) for row in selected)
    gpu_configurations = Counter((row.get("views"), tuple(row.get("output_size") or []))
                                 for row, _ in matched if isinstance(row.get("output_size"), list))
    resolution_mismatches = sum(bool(row.get("output_size")) and bool(gpu[row["frame_id"]][0].get("output_size")) and
                               row["output_size"] != gpu[row["frame_id"]][0]["output_size"]
                               for row in selected if row["frame_id"] in gpu)
    if resolution_mismatches:
        warnings.append(f"{resolution_mismatches} CPU-declared output sizes differ from actual GPU records. GPU configurations are authoritative; do not claim the requested resolution was measured.")
    if len(configurations) > 1:
        warnings.append("Selected frames contain multiple view counts/output sizes. Combined statistics describe this mixed run, not one fixed rendering configuration.")
    if any(row.get("diagnostic_sync") for row in selected):
        warnings.append("Selected frames include synchronous diagnostics. Use --exclude-diagnostics for a separate report, keeping the full report as evidence of real stalls.")
    warnings.extend(["No CPU+GPU sum is used: CPU and GPU work may overlap.",
                     "Cadence measures observed frame-entry intervals for adjacent presented calls, not display scanout. Loading, phase changes and record gaps do not enter gameplay cadence.",
                     "GPU covered interval spans first-to-last timestamps, including gaps or uninstrumented work between them; it excludes work before/after those endpoints. Inspect header scope. Async instrumentation still has timestamp/copy/poll/logging cost."])
    result = {
        "schema": 1, "source_format": "legacy_gpu_profile" if legacy else "frame_profile_v1",
        "target": {"fps": target_fps, "frame_budget_ms": budget},
        "selection": {"skip_eligible_rendered_frames": skip_frames, "actually_skipped": skipped,
                      "include_paused": include_paused, "exclude_diagnostics": exclude_diagnostics,
                      "section": section, "warmup_excluded": True, "skip_scope": "per_section",
                      "gpu_join": "frame_id"},
        "counts": {"records": len(records), "cpu_calls": len(cpu), "selected_rendered_frames": len(selected),
                   "cpu_call_gaps": sum(b["call_id"] - a["call_id"] - 1 for a, b in zip(cpu, cpu[1:])),
                   "warmup_calls": sum(bool(row.get("warmup")) for row in cpu
                                       if section is None or row.get("section") == section),
                   "warmup_rendered_frames": sum(bool(row.get("warmup")) and row.get("frame_id") is not None for row in cpu
                                       if section is None or row.get("section") == section),
                   "excluded_diagnostic_rendered_frames": sum(row.get("diagnostic_sync") is True and not row.get("warmup", False) and
                                       row.get("frame_id") is not None and row.get("phase") in (("gameplay", "paused") if include_paused else ("gameplay",))
                                       for row in cpu if exclude_diagnostics and (section is None or row.get("section") == section)),
                   "cadence_excluded_after_diagnostic": cadence_after_diagnostic,
                   "selected_presented_frames": sum(r.get("presented") is True for r in selected),
                   "selected_diagnostic_sync_frames": sum(r.get("diagnostic_sync") is True for r in selected),
                   "selected_error_frames": sum(bool(r.get("error")) for r in selected),
                   "gpu_records": len(gpu_records), "gpu_matched_selected": len(matched),
                   "gpu_missing_selected": len(selected) - len(matched),
                   "gpu_covered_samples": len(gpu_covered),
                   "gpu_covered_missing_selected": len(selected) - len(gpu_covered),
                   "gpu_orphan_records": sum(key not in seen_frames for key in gpu),
                   "gpu_unselected_records": sum(key in seen_frames and key not in selected_ids for key in gpu),
                   "gpu_invalid_records": invalid_gpu, "gpu_duplicate_records": duplicate_gpu,
                   "cpu_invalid_section_partitions": invalid_partitions,
                   "cpu_gpu_resolution_mismatch_frames": resolution_mismatches,
                   "unknown_record_kinds": dict(unknown_kinds)},
        "configurations": [{"views": views, "output_size": list(size), "frames": count}
                           for (views, size), count in configurations.items()],
        "actual_gpu_configurations": [{"views": views, "output_size": list(size), "frames": count}
                           for (views, size), count in gpu_configurations.items()],
        "phases": phases, "cpu_work": stats(cpu_work, budget),
        "cpu_execution": stats(cpu_execution, budget),
        "cpu_execution_missing_selected": len(selected_calls) - len(cpu_execution),
        "cpu_execution_scope": "Main-thread OS user+kernel time only, excluding waits and worker threads. Coarse counters: prefer multi-frame means. Missing execution samples are never replaced by wall time or zero.",
        "cpu_contributors": contributors(cpu_parts, budget),
        "gpu_covered_interval": stats(gpu_covered, budget),
        "gpu_pass_duration_sum": {key: value for key, value in stats(gpu_sums, budget).items()
                                  if not key.startswith("over_budget")},
        "gpu_contributors": contributors([passes for _, passes in matched], budget),
        "cadence": {**stats(cadence, budget), "intervals_total_ms": sum(cadence),
                    "observed_entry_rate_fps": 1000 * len(cadence) / sum(cadence) if cadence else None,
                    "definition": "count / total duration of adjacent eligible presented entry intervals"},
        "gpu_status_last": statuses, "gpu_status_epochs": status_history,
        "gpu_observed_drops": sum(row["dropped"] for row in status_history),
        "gpu_observed_errors": sum(row["errors"] for row in status_history),
        "footer": footer, "headers": headers,
        "warnings": list(dict.fromkeys(warnings)), "timeline": timeline,
    }
    if _group and section is None and section_names:
        result["sections"] = [{"name": name, **analyze(records, target_fps, skip_frames,
                                include_paused, exclude_diagnostics, name, _group=False)}
                              for name in section_names]
        if len(section_names) > 1:
            result["grouped_only"] = True
            result["warnings"].append("Multiple scenario sections: aggregate budget metrics intentionally omitted. Compare each section independently; warmup is excluded within every section.")
            for key in ("cpu_work", "cpu_execution", "gpu_covered_interval", "gpu_pass_duration_sum"):
                result[key] = stats([], budget)
            result["cpu_contributors"], result["gpu_contributors"] = [], []
            result["cadence"] = {**stats([], budget), "intervals_total_ms": None,
                                 "observed_entry_rate_fps": None, "definition": "per section only"}
    return result


def fmt(value, digits=2):
    return "n/a" if value is None else f"{value:,.{digits}f}"


def render_html(report):
    esc = lambda value: html.escape(str(value), quote=True)
    budget = report["target"]["frame_budget_ms"]
    cards = []
    for title, key, note in [
        ("CPU main-thread execution", "cpu_execution", "OS user+kernel time; excludes waiting and worker threads. Mean shown because OS counters are coarse. Missing stays unavailable."),
        ("CPU frame body", "cpu_work", "Wall time of instrumented frame body; includes measured waits."),
        ("GPU covered interval", "gpu_covered_interval", "First-to-last timestamps, including intervening gaps; excludes work outside these endpoints."),
        ("Observed cadence", "cadence", "Adjacent presented calls; entry-to-entry, not scanout."),
        ("GPU pass-duration sum", "gpu_pass_duration_sum", "Diagnostic only; overlap can double-count. Never converted to FPS.")]:
        value = report[key]
        center = "mean" if key == "cpu_execution" else "median"
        cards.append(f'<article><h2>{title}</h2><b>{fmt(value[center+"_ms"])} <small>ms {center}</small></b>'
                     f'<p>p95 {fmt(value["p95_ms"])} · p99 {fmt(value["p99_ms"])} · n={value["n"]}</p><p class="muted">{note}</p></article>')
    def table(rows, first="Contributor"):
        if not rows:
            return '<p class="muted">No valid samples.</p>'
        body = []
        for row in rows:
            width = min(100, row["mean_ms"] / budget * 100)
            body.append(f'<tr><td>{esc(row["name"])}</td><td><div class="barbox"><div class="bar" style="width:{width:.3f}%"></div></div>'
                        f'{fmt(row["mean_ms"])} ms</td><td>{fmt(row["median_ms"])}</td><td>{fmt(row["p95_ms"])}</td><td>{fmt(row["p99_ms"])}</td>'
                        f'<td>{row["observed_n"]}/{row["denominator_n"]}</td></tr>')
        return f'<table><thead><tr><th>{first}</th><th>Mean / {budget:.2f} ms budget</th><th>Median ms</th><th>p95 ms</th><th>p99 ms</th><th>Observed / denominator</th></tr></thead><tbody>{"".join(body)}</tbody></table>'
    phase_rows = ''.join(f'<tr><td>{esc(name)}</td><td>{r["calls"]}</td><td>{r["rendered"]}</td><td>{r["presented"]}</td><td>{fmt(r["cpu_work"]["p95_ms"])}</td></tr>' for name, r in report["phases"].items())
    counts = report["counts"]
    sample_note = (f'{counts["gpu_matched_selected"]} / {counts["selected_rendered_frames"]} selected rendered frames have GPU samples; '
                   f'{counts["gpu_missing_selected"]} missing, {counts["gpu_orphan_records"]} unmatched IDs. '
                   f'{counts["selected_diagnostic_sync_frames"]} selected frames include synchronous diagnostics.')
    # Static SVG: max-pool buckets retain spikes instead of thinning them away.
    timeline = report["timeline"]
    series = []
    max_y = budget * 2
    for field, color in [("cpu_ms", "#4f8ef7"), ("gpu_covered_ms", "#d180ed"), ("cadence_ms", "#e8ad44")]:
        bins = []
        stride = max(1, math.ceil(len(timeline) / 900))
        for start in range(0, len(timeline), stride):
            values = [p[field] for p in timeline[start:start + stride] if p[field] is not None]
            bins.append(max(values) if values else None)
        if bins:
            max_y = max(max_y, max((x for x in bins if x is not None), default=0))
            series.append((bins, color))
    graph = []
    for bins, color in series:
        for index, value in enumerate(bins):
            if value is not None:
                x = 45 + index * 900 / max(1, len(bins) - 1)
                y = 220 - 185 * value / max_y
                graph.append(f'<circle cx="{x:.2f}" cy="{y:.2f}" r="1.6" fill="{color}"/>')
    budget_y = 220 - 185 * budget / max_y
    chart = f'<svg viewBox="0 0 1000 250" role="img" aria-label="Frame duration timeline"><line x1="45" x2="955" y1="{budget_y:.2f}" y2="{budget_y:.2f}" stroke="#da5766" stroke-dasharray="6 4"/><text x="48" y="20" fill="currentColor">0–{max_y:.1f} ms; red line {budget:.2f} ms</text>{"".join(graph)}<text x="45" y="244" fill="currentColor">Selected frame order → (bucket maxima if more than 900 frames)</text></svg>'
    source = report.get("input", {})
    sections_html = ''
    for section in report.get("sections", []):
        rows = ''.join(f'<tr><td>{label}</td><td>{section[key]["n"]}</td><td>{fmt(section[key]["mean_ms" if key == "cpu_execution" else "median_ms"])}</td><td>{fmt(section[key]["p95_ms"])}</td><td>{fmt(section[key]["p99_ms"])}</td></tr>'
                       for label, key in [("CPU execution (mean; others median)", "cpu_execution"), ("CPU body wall", "cpu_work"), ("GPU covered", "gpu_covered_interval"), ("Cadence", "cadence"), ("GPU pass sum (diagnostic)", "gpu_pass_duration_sum")])
        sections_html += f'<section><h2>Section: {esc(section["name"])}</h2><p>{section["counts"]["selected_rendered_frames"]} selected frames; {section["counts"]["warmup_rendered_frames"]} rendered warmup frames excluded; {section["counts"]["gpu_missing_selected"]} missing GPU samples.</p><table><tr><th>Metric</th><th>n</th><th>Mean / median ms</th><th>p95 ms</th><th>p99 ms</th></tr>{rows}</table><h3>CPU contributors</h3>{table(section["cpu_contributors"])}<h3>GPU contributors</h3>{table(section["gpu_contributors"])}<p>Observed entry rate: {fmt(section["cadence"]["observed_entry_rate_fps"])} FPS; no inference from pass sum.</p></section>'
    if report.get("grouped_only"):
        cards = ['<p>Different scenario sections are reported separately below; there is no blended frame-budget verdict.</p>']
    return f'''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Sideways frame budget</title>
<style>body{{font:15px/1.5 system-ui,sans-serif;background:#10141c;color:#e5eaf2;max-width:1250px;margin:30px auto;padding:0 22px}}h1{{margin-bottom:4px}}h2{{font-size:18px}}small,.muted{{color:#aeb9ca}}.cards{{display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:14px}}article,section{{background:#19212d;border:1px solid #303c4d;border-radius:8px;padding:18px;margin:18px 0}}article b{{font-size:29px}}article p{{margin-bottom:0}}article small{{font-size:13px}}table{{border-collapse:collapse;width:100%;font-variant-numeric:tabular-nums}}th,td{{padding:9px 10px;border-bottom:1px solid #344153;text-align:right}}th:first-child,td:first-child{{text-align:left}}.barbox{{height:7px;background:#303c4d;min-width:140px}}.bar{{height:7px;background:#4f8ef7}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;font-size:12px}}a{{color:#86b9ff}}svg{{width:100%}}li{{margin:7px 0}}.tablewrap{{overflow-x:auto}}</style>
<h1>Sideways frame budget</h1><p class="muted">Target {report["target"]["fps"]:g} FPS · {budget:.3f} ms per frame · {esc(report["source_format"])}</p>
<p>{esc(source.get("path", ""))}<br><small>SHA-256 {esc(source.get("sha256", ""))}</small></p>
<div class="cards">{"".join(cards)}</div><p>{esc(sample_note)}</p>{sections_html}
<section><h2>Frame timeline</h2>{chart}<p class="muted">Blue: CPU body. Purple: GPU covered interval. Gold: observed cadence. These series are not added.</p><p>Observed entry rate: {fmt(report["cadence"]["observed_entry_rate_fps"])} FPS over {report["cadence"]["n"]} eligible intervals. This is measured cadence, not a pass-sum estimate or proof of 120 FPS.</p></section>
<section><h2>CPU contributors</h2><p class="muted">Non-overlapping section contract validated per call. Conditional sections absent from a valid partition count as zero. GPU/CPU percentages are not combined.</p><div class="tablewrap">{table(report["cpu_contributors"])}</div></section>
<section><h2>GPU pass contributors</h2><p class="muted">Each row is its own duration statistic. Passes may overlap. Missing GPU frames are excluded, never replaced with zeros; absent passes in a completed valid frame count as zero.</p><div class="tablewrap">{table(report["gpu_contributors"])}</div></section>
<section><h2>Phase coverage</h2><div class="tablewrap"><table><tr><th>Phase</th><th>Calls</th><th>Rendered</th><th>Presented</th><th>CPU p95 ms</th></tr>{phase_rows}</table></div></section>
<section><h2>Limits, missing samples and overhead</h2><ul>{''.join('<li>'+esc(w)+'</li>' for w in report["warnings"])}</ul><details open><summary>Counts / GPU ring / writer</summary><pre>{esc(json.dumps({"counts":counts,"gpu_status_last":report["gpu_status_last"],"gpu_status_epochs":report["gpu_status_epochs"],"gpu_observed_drops":report["gpu_observed_drops"],"gpu_observed_errors":report["gpu_observed_errors"],"footer":report["footer"]},indent=2))}</pre></details></section>
<section><h2>Capture contract</h2><details><summary>Headers, selection and configurations</summary><pre>{esc(json.dumps({"headers":report["headers"],"selection":report["selection"],"cpu_configurations":report["configurations"],"actual_gpu_configurations":report["actual_gpu_configurations"]},indent=2))}</pre></details><p class="muted">Percentiles use linear interpolation (type 7). Small sample counts and missing samples limit p95/p99 interpretation. Full numeric data: summary.json.</p></section></html>'''


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path, help="one frame-profile or legacy gpu-profile JSONL")
    parser.add_argument("--out", required=True, type=Path, help="new output directory")
    parser.add_argument("--target-fps", type=float, default=120.0)
    parser.add_argument("--skip-frames", type=int, default=0, help="exclude first N eligible rendered gameplay frames")
    parser.add_argument("--include-paused", action="store_true", help="include paused rendered frames; cadence still splits at phase changes")
    parser.add_argument("--exclude-diagnostics", action="store_true", help="exclude synchronous diagnostic frames from gameplay summaries; retain phase counts")
    parser.add_argument("--section", help="report only this named scenario section; default shows every section separately")
    parser.add_argument("--force", action="store_true", help="replace summary.json/report.html in an existing output directory")
    args = parser.parse_args(argv)
    try:
        records, digest = read_jsonl(args.input)
        report = analyze(records, args.target_fps, args.skip_frames, args.include_paused, args.exclude_diagnostics, args.section)
        report["input"] = {"path": str(args.input.resolve()), "sha256": digest}
        paths = [args.out / "summary.json", args.out / "report.html"]
        if not args.force and any(p.exists() for p in paths):
            raise ValueError("output exists; choose a new --out directory or explicitly pass --force")
        if args.input.resolve() in [p.resolve() for p in paths]:
            raise ValueError("refusing to overwrite input profile")
        args.out.mkdir(parents=True, exist_ok=True)
        paths[0].write_text(json.dumps(report, indent=2, ensure_ascii=False, allow_nan=False) + "\n", encoding="utf-8")
        paths[1].write_text(render_html(report), encoding="utf-8")
        print(f"Wrote {paths[0]} and {paths[1]}")
        for item in report.get("sections", [report]):
            print(f"{item.get('name', 'Selected frames')}: GPU samples {item['counts']['gpu_matched_selected']}/{item['counts']['selected_rendered_frames']}; "
                  f"CPU p95 {fmt(item['cpu_work']['p95_ms'])} ms; GPU covered p95 {fmt(item['gpu_covered_interval']['p95_ms'])} ms; "
                  f"cadence p95 {fmt(item['cadence']['p95_ms'])} ms")
        return 0
    except (OSError, UnicodeError, ValueError) as error:
        print(f"frame-budget: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
