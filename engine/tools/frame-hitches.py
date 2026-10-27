#!/usr/bin/env python3
"""Report visible gameplay cadence tails and associated CPU/GPU work as JSON.

Python 3.10+, standard library only. Example:
  python engine/tools/frame-hitches.py frames.jsonl --out hitches.json
The interval ending at call N contains CPU call N-1. GPU joins use frame_id,
never arrival order; an association does not establish the cause of a stall.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import importlib.util
import json
import math
from pathlib import Path
import statistics
import sys

# Keep JSON validation, percentile interpolation, GPU pass validation and
# cumulative ring-counter handling identical to the broader budget report.
_spec = importlib.util.spec_from_file_location("frame_budget", Path(__file__).with_name("frame-budget.py"))
_budget = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_budget)
read_jsonl, number, duration = _budget.read_jsonl, _budget.number, _budget.duration
percentile, pass_map, ring_history = _budget.percentile, _budget.pass_map, _budget.ring_history

CPU_PARTS = ("render_encode_submit", "present_acquire", "physics")
GPU_PASSES = ("foliage page cull", "foliage page raster")
CAMERA_KEYS = ("camera_id", "camera_mode", "camera_epoch")


def summary(values):
    values = list(values)
    return {"n": len(values), "p50_ms": percentile(values, .5),
            "p95_ms": percentile(values, .95), "p99_ms": percentile(values, .99),
            "max_ms": max(values) if values else None,
            "mean_ms": statistics.fmean(values) if values else None}


def tails(values):
    result = summary(values)
    median = result["p50_ms"]
    result.update({"twice_median_ms": 2 * median if median is not None else None,
                   "over_2x_median_n": sum(v > 2 * median for v in values) if values else 0,
                   "over_33_33_ms_n": sum(v > 33.33 for v in values)})
    return result


def pearson(pairs):
    if len(pairs) < 3:
        return {"paired_n": len(pairs), "pearson_r": None}
    xs, ys = zip(*pairs)
    xmean, ymean = statistics.fmean(xs), statistics.fmean(ys)
    xx = sum((x - xmean) ** 2 for x in xs)
    yy = sum((y - ymean) ** 2 for y in ys)
    r = sum((x - xmean) * (y - ymean) for x, y in pairs) / math.sqrt(xx * yy) if xx and yy else None
    return {"paired_n": len(pairs), "pearson_r": max(-1, min(1, r)) if r is not None else None}


def diagnostic(row):
    # play.rs opens diagnostic_wait on every frame, even when the conditional
    # hidden-frame wait does not run. Its tiny nonzero checkpoint overhead is
    # not evidence of synchronous diagnostics. diagnostic_sync is authoritative;
    # diagnostic_readback is additionally emitted only for actual capture work.
    parts = row.get("sections_ms")
    return bool(row.get("diagnostic_sync") or row.get("screenshot") or
                row.get("screenshot_diagnostics") or
                (isinstance(parts, dict) and any(duration(v) and v > 0 and
                 (k == "diagnostic_readback" or k.startswith("screenshot")) for k, v in parts.items())))


def frame_info(row, gpu):
    """Missing timings stay null; absence in a complete valid GPU sample is zero."""
    parts = row.get("sections_ms")
    parts = parts if isinstance(parts, dict) else {}
    item = gpu.get(row["frame_id"])
    result = {key: row.get(key) for key in ("call_id", "frame_id", "section", "section_frame", "race_frame", "start_ms")}
    result["cpu_wall_ms"] = row.get("cpu_frame_ms") if duration(row.get("cpu_frame_ms")) else None
    result["cpu_sections_ms"] = {name: parts.get(name) if duration(parts.get(name)) else None for name in CPU_PARTS}
    result["gpu"] = None
    if item:
        sample, passes = item
        intervals = []
        raw_intervals = sample.get("gpu_pass_intervals")
        for interval in raw_intervals if isinstance(raw_intervals, list) else []:
            if not isinstance(interval, dict) or interval.get("name") not in GPU_PASSES:
                continue
            start, end = interval.get("start_ms"), interval.get("end_ms")
            if duration(start) and duration(end) and end >= start:
                intervals.append({"name": interval["name"], "view_index": interval.get("view_index"),
                                  "start_ms": start, "end_ms": end, "duration_ms": end - start})
        covered = sample.get("covered_interval_ms", sample.get("gpu_covered_ms"))
        result["gpu"] = {"frame_id": sample["frame_id"], "received_call_id": sample.get("received_call_id"),
                         "covered_interval_ms": covered if duration(covered) else None,
                         "foliage_pass_ms": [[name, passes.get(name, 0.0)] for name in GPU_PASSES],
                         "foliage_pass_intervals": intervals}
    return result


def metric_values(frame):
    values = {"cpu_wall_ms": frame["cpu_wall_ms"], **frame["cpu_sections_ms"]}
    sample = frame["gpu"]
    values["gpu_covered_interval_ms"] = sample["covered_interval_ms"] if sample else None
    for name in GPU_PASSES:
        values[name] = dict(sample["foliage_pass_ms"])[name] if sample else None
    return values


def associations(intervals, outlier_calls):
    result = []
    for side in ("preceding_frame", "following_frame"):
        for metric in ("cpu_wall_ms", *CPU_PARTS, "gpu_covered_interval_ms", *GPU_PASSES):
            pairs, outliers, others = [], [], []
            for interval in intervals:
                value = metric_values(interval[side])[metric]
                if value is None:
                    continue
                pairs.append((interval["cadence_ms"], value))
                (outliers if interval["following_frame"]["call_id"] in outlier_calls else others).append(value)
            result.append({"alignment": side, "metric": metric, **pearson(pairs),
                           "at_cadence_outliers": summary(outliers), "at_other_intervals": summary(others)})
    return result


def interval_report(intervals, top):
    cadence = tails([item["cadence_ms"] for item in intervals])
    outliers = [item for item in intervals if item["cadence_ms"] > cadence["twice_median_ms"] or item["cadence_ms"] > 33.33]
    calls = {item["following_frame"]["call_id"] for item in outliers}
    return {"cadence": cadence, "outlier_n": len(outliers),
            "outliers": sorted(outliers, key=lambda item: -item["cadence_ms"])[:top],
            "outliers_omitted_n": max(0, len(outliers) - top),
            "associations": associations(intervals, calls) if intervals else []}


def analyze(records, skip_frames=0, top=20, camera_transition_calls=()):
    if skip_frames < 0 or top < 0:
        raise ValueError("skip-frames and top must be nonnegative")
    cpu = [row for row in records if row.get("kind") == "cpu_frame"]
    if not cpu:
        raise ValueError("no cpu_frame records; legacy synchronous GPU profiles contain no gameplay cadence evidence")
    headers = [row for row in records if row.get("kind") in ("frame_profile_header", "header")]
    if len(headers) > 1:
        raise ValueError("multiple profile headers; analyze separate runs separately")
    header = headers[0] if headers else {}
    footer = next((row for row in reversed(records) if row.get("kind") == "frame_profile_footer"), {})
    ids, calls = set(), set()
    previous = None
    for row in cpu:
        call, frame = row.get("call_id"), row.get("frame_id")
        if not isinstance(call, int) or isinstance(call, bool) or (previous is not None and call <= previous):
            raise ValueError("CPU call IDs must be unique increasing integers; analyze separate runs separately")
        if frame is not None:
            if not isinstance(frame, int) or isinstance(frame, bool) or frame in ids:
                raise ValueError("rendered frame IDs must be unique integers")
            ids.add(frame)
        calls.add(call)
        previous = call
    explicit_camera = set(camera_transition_calls)
    if explicit_camera - calls:
        raise ValueError("camera-transition-call must identify a recorded CPU call")
    gpu, gpu_counts = {}, Counter({key: 0 for key in ("records", "invalid_records", "duplicate_records")})
    for row in records:
        if row.get("kind") != "gpu_frame":
            continue
        gpu_counts["records"] += 1
        frame, passes = row.get("frame_id"), pass_map(row)
        if not isinstance(frame, int) or isinstance(frame, bool) or passes is None:
            gpu_counts["invalid_records"] += 1
        elif frame in gpu:
            gpu_counts["duplicate_records"] += 1
        else:
            gpu[frame] = row, passes

    events, excluded = Counter(), Counter()
    selected, intervals, groups = [], [], defaultdict(list)
    previous = None
    previous_reasons = {"first_call"}
    skipped_in_segment = 0
    camera_markers = False
    for row in cpu:
        reasons = set()
        phase = str(row.get("phase", "unknown"))
        events["phase:" + phase] += 1
        if phase != "gameplay":
            reasons.add("non_gameplay")
        for key in ("paused", "warmup", "error"):
            if row.get(key):
                reasons.add(key)
                events[key] += 1
        if row.get("hidden", header.get("hidden_smoke")) is not False:
            reasons.add("hidden_or_visibility_unknown")
            events["hidden_or_visibility_unknown"] += 1
        if row.get("presented") is not True or row.get("frame_id") is None:
            reasons.add("not_presented")
        if diagnostic(row):
            reasons.add("diagnostic")
            events["diagnostic"] += 1
        if row.get("focused") is False:
            reasons.add("unfocused")
            events["unfocused"] += 1
        camera_markers |= any(key in row for key in (*CAMERA_KEYS, "camera_transition"))
        camera_changed = bool(row.get("camera_transition") or row["call_id"] in explicit_camera)
        section_changed = previous is not None and any(previous.get(key) != row.get(key) for key in ("section", "round"))
        config_changed = previous is not None and any(previous.get(key) != row.get(key) for key in ("views", "output_size"))
        if previous:
            camera_changed |= any(key in previous and key in row and previous[key] != row[key] for key in CAMERA_KEYS)
        for name, changed in (("camera_transition", camera_changed), ("section_transition", section_changed), ("configuration_transition", config_changed)):
            if changed:
                reasons.add(name)
                events[name] += 1
        if previous is None or section_changed or camera_changed or config_changed:
            skipped_in_segment = 0
        if not reasons and skipped_in_segment < skip_frames:
            reasons.add("skip_frames")
            skipped_in_segment += 1
        if not reasons:
            selected.append(row)
        excluded.update(reasons)

        interval_reasons = set(reasons)
        if previous is None:
            interval_reasons.add("first_call")
        else:
            if previous_reasons:
                interval_reasons.add("preceding_frame_excluded")
            if diagnostic(previous):
                interval_reasons.add("immediately_after_diagnostic")
                events["immediately_after_diagnostic"] += 1
            if row["call_id"] != previous["call_id"] + 1:
                interval_reasons.add("call_gap")
                events["call_gap"] += 1
        if not interval_reasons:
            start, prior = row.get("start_ms"), previous.get("start_ms")
            interval = start - prior if number(start) and number(prior) else row.get("cadence_ms")
            if not duration(interval) or interval <= 0:
                interval_reasons.add("invalid_cadence")
            else:
                item = {"cadence_ms": interval, "preceding_frame": frame_info(previous, gpu),
                        "following_frame": frame_info(row, gpu)}
                intervals.append(item)
                key = json.dumps({key: row.get(key) for key in ("section", "round", "views", "output_size", *CAMERA_KEYS)}, sort_keys=True)
                groups[key].append(item)
        events.update("cadence_excluded:" + name for name in interval_reasons)
        previous, previous_reasons = row, reasons

    selected_ids = {row["frame_id"] for row in selected}
    statuses = [row for row in records if row.get("kind") == "gpu_status"]
    epochs = ring_history(statuses)
    warnings = [
        "Cadence is adjacent visible presented gameplay entry-to-entry wall time, not display scanout. Both endpoint calls must be eligible.",
        "The preceding CPU frame runs inside the interval. The following CPU frame starts at its end; following-frame associations are context only.",
        "GPU samples join by rendered frame_id regardless of received_call_id. GPU work is asynchronous and may overlap other frames; correlation does not establish causality.",
        "CPU wall sections include waits. CPU and GPU timings are never added; pass durations are not FPS.",
        "Transition calls and their following intervals are excluded. The interval immediately after synchronous diagnostics/readback is excluded.",
        "Missing GPU samples stay missing. An absent foliage pass is zero only within a valid complete GPU sample. Percentiles use type-7 interpolation.",
    ]
    if not camera_markers and not explicit_camera:
        warnings.append("No camera transition markers are recorded. Scenario section boundaries are excluded; unmarked interactive camera cuts cannot be identified. Supply --camera-transition-call for known cuts.")
    if not footer:
        warnings.append("No footer: capture may be active/truncated; final writer drops are unknown.")
    if not intervals:
        warnings.append("No gameplay-cadence evidence: no eligible adjacent visible presented gameplay calls survived exclusions.")
    elif len(intervals) < 100:
        warnings.append("Fewer than 100 eligible intervals: tail percentiles and correlations have limited sample support.")
    if len(groups) > 1:
        warnings.append("Overall statistics pool multiple contexts. Use contexts for scene/configuration-specific medians, outliers and associations.")
    gpu_counts.update({"matched_cpu_frames": sum(frame in ids for frame in gpu),
                       "orphan_records": sum(frame not in ids for frame in gpu),
                       "matched_selected_frames": len(selected_ids & gpu.keys()),
                       "missing_selected_frames": len(selected_ids - gpu.keys()),
                       "missing_all_rendered_frames": len(ids - gpu.keys()),
                       "observed_dropped_samples": sum(epoch["dropped"] for epoch in epochs),
                       "observed_errors": sum(epoch["errors"] for epoch in epochs)})
    return {"schema": 1, "gameplay_cadence_evidence": bool(intervals),
            "selection": {"skip_eligible_frames_per_segment": skip_frames, "top_outliers": top,
                          "explicit_camera_transition_calls": sorted(explicit_camera),
                          "outlier_rule": "cadence > 2x selected median OR > 33.33 ms; strict comparisons"},
            "counts": {"records": len(records), "cpu_calls": len(cpu), "selected_frames": len(selected),
                       "cpu_missing_calls_between_records": sum(b["call_id"] - a["call_id"] - 1 for a, b in zip(cpu, cpu[1:])),
                       "excluded_frames_by_reason_nonexclusive": dict(excluded),
                       "events_nonexclusive": dict(events), "gpu": dict(gpu_counts),
                       "writer_dropped_records": footer.get("writer_dropped_records"),
                       "gpu_pending_last": statuses[-1].get("pending") if statuses else None},
            "capture": {key: header.get(key) for key in ("seed", "output_size", "views", "upscale", "hidden_smoke", "scenario")},
            "gpu_status_epochs": epochs, **interval_report(intervals, top),
            "contexts": [{"context": json.loads(key), **interval_report(items, top)} for key, items in groups.items()],
            "warnings": warnings}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("input", type=Path)
    parser.add_argument("--out", type=Path, help="JSON file; default writes JSON to stdout")
    parser.add_argument("--top", type=int, default=20, help="maximum outlier details per context (default: 20)")
    parser.add_argument("--skip-frames", type=int, default=0, help="additional warmup exclusion per section/camera/configuration segment")
    parser.add_argument("--camera-transition-call", type=int, action="append", default=[], help="exclude a known camera cut call and the following interval; repeatable")
    parser.add_argument("--force", action="store_true", help="replace an existing output JSON file")
    args = parser.parse_args(argv)
    try:
        records, digest = read_jsonl(args.input)
        report = analyze(records, args.skip_frames, args.top, args.camera_transition_call)
        report["input"] = {"path": str(args.input.resolve()), "sha256": digest}
        encoded = json.dumps(report, indent=2, ensure_ascii=False, allow_nan=False) + "\n"
        if args.out:
            if args.out.resolve() == args.input.resolve():
                raise ValueError("refusing to overwrite input profile")
            if args.out.exists() and not args.force:
                raise ValueError("output exists; choose a new file or pass --force")
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(encoded, encoding="utf-8")
            print(f"Wrote {args.out}; eligible intervals: {report['cadence']['n']}; outliers: {report['outlier_n']}")
        else:
            print(encoded, end="")
        return 0
    except (OSError, UnicodeError, ValueError) as error:
        print(f"frame-hitches: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
