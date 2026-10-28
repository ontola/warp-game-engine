#!/usr/bin/env python3
"""Read-only analysis of compare-cpu-spikes.ps1 artifacts; Python 3.10+.

Usage: python analyze-cpu-spikes.py --manifest C:/bench/fresh-ab/manifest.json
       python analyze-cpu-spikes.py --manifest C:/bench/fresh-ab/manifest.json --json --require-complete
Also accepts --input DIRECTORY for existing run-ab.ps1 evidence. Prints only
to stdout (text, or JSON with --json). Never
launches a benchmark, writes a report/cache, changes raw files or saves images.
The contaminated pilot is excluded unconditionally, including --diagnostic.
Optional --diagnostic STEM reads an explicitly selected --cpu-thread run into a
separate section, never into A/B timings. --require-complete fails until all
the manifest's alternating repeats and paired workload checks pass (default
runner configuration: three repeats, 24 runs).
Pillow is optional and used only to read existing first-repeat screenshots.
"""
import argparse
import csv
import hashlib
import json
import math
import os
import re
import statistics
import sys
from collections import Counter, defaultdict
from pathlib import Path

BASE = Path(__file__).resolve().parent
FRAMES = list(range(180, 1080))
CASES = [(s, v, "1920x1080" if v == 1 else "3840x2160")
         for v in (1, 4) for s in ("bullets", "explosions")]
CPU = ["reset_ms", "sim_ms", "scene_ms", "fx_flush_ms", "upload_ms", "render_submit_ms", "cpu_total_ms", "cpu_gpu_wait_ms"]
GPU = ["gpu_union_ms", "gpu_interval_ms", "gpu_instrumented_ms"]
WALL_EXTRA = ["absorb_ms", "pose_ms"]
THREAD = ["thread_" + n + "_ms" for n in ("reset", "sim", "absorb", "pose", "fx_flush", "upload", "render_submit", "total")]
COUNTERS = ["requested", "shots", "explosions", "impacts", "kills", "chunks_removed", "chunks_worn", "arena_resets", "fallback_targets", "debris_spawned"]
OCCUPANCY = ["live_bullets", "rendered_tracers", "alive_players", "alive_chunks", "local_lights", "cpu_particles", "live_gpu_debris", "effect_puffs"]
REQUIRED = ["frame", "measured_tick", "simulation_s"] + CPU + COUNTERS + OCCUPANCY + GPU
HEADER = re.compile(r"Warp stress (\w+) \| seed (\d+) \| (\d+)x(\d+) native output \| (\d+) views \| (.*?) \| light (\w+)")
COVERAGE = re.compile(r"GPU timestamp coverage: (\d+)/(\d+) measured frames, enabled (\w+), dropped (\d+), errors (\d+)")
STRATA = ("all", "reset", "nonreset")


def text(path):
    return path.read_text(encoding="utf-8-sig", errors="replace")


def basename(value):
    return str(value).replace("\\", "/").rsplit("/", 1)[-1]


def external_path(value):
    value = str(value).replace("\\", "/")
    if os.name != "nt" and re.match(r"^[A-Za-z]:/", value):
        value = "/mnt/" + value[0].lower() + value[2:]
    return Path(value)


def no_pilot(path):
    if "pilot" in basename(path).lower():
        raise ValueError("Contaminated Sideways pilot is excluded unconditionally")


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def stats(values):
    a = sorted(float(v) for v in values if v is not None and math.isfinite(v))
    if not a:
        return {"count": 0}
    # Rust f64::round for nonnegative indices, not Python's ties-to-even round.
    q = lambda p: a[math.floor((len(a) - 1) * p + 0.5)]
    return dict(count=len(a), sum=sum(a), mean=statistics.fmean(a), median=q(.5),
                p95=q(.95), p99=q(.99), min=a[0], max=a[-1])


def numeric_rows(path):
    with path.open(encoding="utf-8-sig", newline="") as f:
        reader = csv.DictReader(f)
        if reader.fieldnames is None or len(reader.fieldnames) != len(set(reader.fieldnames)):
            raise ValueError("Missing/duplicate CSV header")
        missing = set(REQUIRED) - set(reader.fieldnames)
        if missing:
            raise ValueError("Missing CSV columns: " + ", ".join(sorted(missing)))
        out = []
        for row in reader:
            if None in row or any(v is None for v in row.values()):
                raise ValueError("CSV row width does not match header")
            values = {k: float(v) if v else None for k, v in row.items()}
            if any(v is not None and (not math.isfinite(v) or v < 0) for v in values.values()):
                raise ValueError("CSV contains nonfinite or negative values")
            out.append(values)
        return out


def select(rows, stratum):
    return rows if stratum == "all" else [r for r in rows if bool(r["arena_resets"]) == (stratum == "reset")]


def summarize(rows, passes, metrics):
    out = {}
    for name in STRATA:
        group = select(rows, name)
        ids = {int(r["frame"]) for r in group}
        perpass = defaultdict(list)
        for frame in ids:
            for p, duration in passes.get(frame, {}).items():
                perpass[p].append(duration)
        out[name] = {"frames": len(group), "metrics_ms": {k: stats(r.get(k) for r in group) for k in metrics},
                     "gpu_passes_ms": {k: stats(v) for k, v in sorted(perpass.items())}}
    return out


def analyze_run(stem, record=None, diagnostic=False):
    no_pilot(stem)
    log = text(Path(str(stem) + ".log"))
    err = text(Path(str(stem) + ".err.log"))
    rows = numeric_rows(Path(str(stem) + ".csv"))
    if not rows or any(r[k] is None for r in rows for k in REQUIRED if k not in GPU):
        raise ValueError("No rows or missing required non-GPU values")
    head = HEADER.search(log)
    if not head:
        raise ValueError("Missing stress metadata in log")
    scenario, seed, w, h, views, adapter, light = head.groups()
    if scenario not in ("bullets", "explosions"):
        raise ValueError("Unknown stress scenario")
    cov = COVERAGE.search(log)
    coverage = None if not cov else dict(samples=int(cov[1]), denominator=int(cov[2]), enabled=cov[3] == "true", dropped=int(cov[4]), errors=int(cov[5]))
    passes, duplicate_passes = defaultdict(dict), []
    with Path(str(stem) + ".csv.gpu.csv").open(encoding="utf-8-sig", newline="") as f:
        for r in csv.DictReader(f):
            frame, name, duration = int(r["frame"]), r["pass"], float(r["gpu_ms"])
            if name in passes[frame]:
                duplicate_passes.append([frame, name])
            passes[frame][name] = duration
    totals = {k: sum(r[k] for r in rows) for k in COUNTERS}
    ids = [r["frame"] for r in rows]
    runtime_issues = [line for line in (log + "\n" + err).splitlines()
                      if re.search(r"(?:^|\b)(?:panic|error:|warning:|primitive fallback)", line, re.I)]
    active = any(r.get(k) is not None for r in rows for k in THREAD)
    thread_enabled = "thread CPU profiling enabled" in log or bool(record and "--cpu-thread" in record.get("arguments", ""))
    complete_gpu = all(r[k] is not None for r in rows for k in GPU)
    expected = 3150 if scenario == "bullets" else 600
    checks = {
        "frame_ids_exact": ids == FRAMES,
        "measured_ticks_exact": [r["measured_tick"] for r in rows] == list(range(900)),
        "simulation_timestamps": all(abs(r["simulation_s"] - (r["frame"] + 1) / 60) <= .000001 for r in rows),
        "finite_nonnegative": all(v is None or math.isfinite(v) and v >= 0 for r in rows for v in r.values()),
        "integer_counters": all(r[k].is_integer() for r in rows for k in COUNTERS + OCCUPANCY),
        "workload_total": totals["requested"] == expected and totals["shots" if scenario == "bullets" else "explosions"] == expected,
        "workload_schedule": all(r["requested"] == (4 if int(r["frame"]) % 2 == 0 else 3) if scenario == "bullets"
                                 else r["requested"] == int(int(r["frame"]) % 3 != 2) for r in rows),
        "processed_per_frame": all(r["requested"] == r["shots" if scenario == "bullets" else "explosions"] for r in rows),
        "other_event_kind_zero": totals["explosions" if scenario == "bullets" else "shots"] == 0,
        "four_alive_no_kills_no_fallback": all(r["alive_players"] == 4 for r in rows) and totals["kills"] == totals["fallback_targets"] == 0,
        "reset_flags_boolean": all(r["arena_resets"] in (0, 1) for r in rows),
        "bullets_no_reset": scenario != "bullets" or totals["arena_resets"] == 0,
        "original_characters": "characters: original knight, paladin, ch10 with idle/run/jump" in log and "primitive fallback" not in log + err,
        "seed_full_light": seed == "721077" and light == "full",
        "full_gpu_log_coverage": coverage == dict(samples=900, denominator=900, enabled=True, dropped=0, errors=0),
        "gpu_ids_match": set(passes) == set(ids),
        "gpu_no_duplicate_passes": not duplicate_passes,
        "gpu_complete_finite": complete_gpu and all(math.isfinite(v) and v >= 0 for p in passes.values() for v in p.values()),
        "gpu_scope_consistent": complete_gpu and all(0 < r["gpu_union_ms"] <= r["gpu_interval_ms"] + .000002
                                                     and r["gpu_union_ms"] <= r["gpu_instrumented_ms"] + .000002 for r in rows),
        "gpu_pass_sum_matches": complete_gpu and all(abs(sum(passes.get(int(r["frame"]), {}).values()) - r["gpu_instrumented_ms"]) <= .00005 for r in rows),
        "wait_includes_submission": all(r["cpu_gpu_wait_ms"] + .000001 >= r["cpu_total_ms"] for r in rows),
        "runtime_clean": not runtime_issues,
    }
    if record is not None:
        checks.update(exit_zero_no_timeout=record.get("exit_code") == 0 and not record.get("timeout", False),
                      metadata_matches=scenario == record["scenario"] and int(views) == record["views"] and f"{w}x{h}" == record["size"])
    if not diagnostic:
        checks["unprofiled_primary_run"] = not active and not thread_enabled
    else:
        checks["thread_profiling_requested"] = thread_enabled
    report = dict(source=str(stem), accepted=all(checks.values()), checks=checks,
                  failed_checks=[k for k, v in checks.items() if not v], event_totals=totals,
                  render=dict(scenario=scenario, seed=int(seed), size=f"{w}x{h}", views=int(views), adapter=adapter, light=light),
                  gpu_coverage=coverage, reset_frames=[int(r["frame"]) for r in rows if r["arena_resets"]],
                  strata=summarize(rows, passes, CPU + GPU + WALL_EXTRA), runtime_issues=runtime_issues,
                  thread_diagnostic={"enabled": thread_enabled, "available": active, "strata": summarize(rows, {}, THREAD),
                                     "note": "Calling-thread user+kernel only; quantized samples; means/sums more useful than frame percentiles. Not included in A/B timings."},
                  context={k: record.get(k) for k in ("cpu_before", "cpu_after", "gpu_before", "gpu_after", "start_utc", "end_utc")} if record else {},
                  exit_verified=record is not None)
    return report, rows


def paired_stat(pairs, stratum, family, metric):
    samples = []
    for repeat, before, after in pairs:
        a = before["strata"][stratum][family].get(metric, {})
        b = after["strata"][stratum][family].get(metric, {})
        if not a.get("count") or not b.get("count"):
            continue
        sample = {"repeat": repeat, "baseline": a, "candidate": b}
        sample["difference_ms"] = {q: b[q] - a[q] for q in ("mean", "median", "p95", "p99", "max")}
        sample["change_percent"] = {q: (b[q] / a[q] - 1) * 100 if a[q] else None for q in ("mean", "median", "p95", "p99", "max")}
        samples.append(sample)
    return {"pairs": len(samples), "per_run": samples,
            "paired_center": {q: {"baseline": statistics.median(s["baseline"][q] for s in samples),
                                   "candidate": statistics.median(s["candidate"][q] for s in samples),
                                   "difference_ms": statistics.median(s["difference_ms"][q] for s in samples),
                                   "change_percent": statistics.median(p) if (p := [s["change_percent"][q] for s in samples if s["change_percent"][q] is not None]) else None}
                              for q in ("mean", "median", "p95", "p99", "max")} if samples else {}}


def image_difference(a, b):
    if not a.exists() or not b.exists():
        return {"available": False, "missing": [str(p) for p in (a, b) if not p.exists()]}
    result = dict(available=True, baseline=str(a), candidate=str(b), file_sha256_equal=digest(a) == digest(b))
    try:
        from PIL import Image, ImageChops, ImageStat
    except ImportError:
        return dict(result, pixels_available=False, reason="Pillow unavailable; byte equality only")
    with Image.open(a) as aa, Image.open(b) as bb:
        result.update(baseline_size=list(aa.size), candidate_size=list(bb.size))
        if aa.size != bb.size:
            return dict(result, pixels_available=False, reason="Image dimensions differ")
        diff = ImageChops.difference(aa.convert("RGBA"), bb.convert("RGBA"))
        channels = diff.split()
        maximum = channels[0]
        for channel in channels[1:]:
            maximum = ImageChops.lighter(maximum, channel)
        pixels = aa.width * aa.height
        changed = pixels - maximum.histogram()[0]
        return dict(result, pixels_available=True, exact_pixels=changed == 0, total_pixels=pixels,
                    changed_pixels=changed, changed_percent=100 * changed / pixels,
                    max_channel_difference=max(hi for lo, hi in diff.getextrema()),
                    mean_absolute_channel_difference=statistics.fmean(ImageStat.Stat(diff).mean),
                    changed_bbox=maximum.getbbox(), comparison="RGBA8 decoded pixels; no tolerance or generated images")


def guard_checks(record, manifest):
    before_hash = record.get("executable_sha256_before", "").lower()
    return {
        "known_game_guards_clear": all(isinstance(record.get(k), dict)
            and bool(record[k].get("checked_utc")) and record[k].get("games") == []
            for k in ("concurrency_before", "concurrency_after")),
        "completed_status": record.get("status") == "completed",
        "immutable_run_executable": bool(before_hash
            and before_hash == record.get("executable_sha256_after", "").lower()
            and before_hash == manifest.get(record["variant"] + "_sha256", "").lower()),
    }


def analyze(directory, diagnostic_stems, manifest_path=None):
    report = {"schema": 1, "source": str(directory), "status": "pending", "complete": False,
              "excluded": [{"pattern": "pilot*", "reason": "Baseline pilot contaminated by unrelated Sideways; excluded from every timing/parity result"}],
              "notes": ["CPU columns are elapsed submission wall time, not exclusive CPU execution.",
                        "CPU+GPU wait serializes completion; GPU timestamps are instrumented scopes, not presented FPS.",
                        "Reset strata retain whole-frame costs; no subtraction of reset_ms.",
                        "Pair by repeat and case; report median of paired run statistics, never pool frames across runs.",
                        "Before/after utilization snapshots do not prove causation of a frame spike.",
                        "Screenshot changes are descriptive, not an automatic visual-quality judgment."],
              "runs": [], "cases": {}, "diagnostics": []}
    for stem in diagnostic_stems:
        try:
            r, _ = analyze_run(external_path(stem), diagnostic=True)
            report["diagnostics"].append(r)
        except (OSError, ValueError, KeyError, TypeError) as e:
            report["diagnostics"].append({"source": str(stem), "accepted": False, "error": str(e)})
    manifest_path = manifest_path or directory / "manifest.json"
    if not manifest_path.exists():
        report["reason"] = "A/B manifest absent; no run has been inferred from loose files"
        return report
    manifest = json.loads(text(manifest_path))
    repeats = manifest.get("repeats", 3)
    if not isinstance(repeats, int) or isinstance(repeats, bool) or not 1 <= repeats <= 10:
        raise ValueError("manifest.repeats must be an integer from 1 to 10")
    repeat_numbers = range(1, repeats + 1)
    strict_guards = manifest.get("schema", 1) >= 2
    report["known_game_guard_required"] = strict_guards
    if not strict_guards:
        report["notes"].append("Legacy manifest: before/after known-game guards were not recorded and cannot be verified retrospectively.")
    report["status"] = "analyzed"
    records = manifest.get("runs", [])
    expected = {(repeat, variant, s, v) for repeat in repeat_numbers for s, v, _ in CASES for variant in ("baseline", "candidate")}
    observed, loaded, hash_checks = Counter(), {}, {}
    for variant in ("baseline", "candidate"):
        p = external_path(manifest.get(variant, ""))
        expected_hash = manifest.get(variant + "_sha256", "").lower()
        actual = digest(p) if p.is_file() else None
        hash_checks[variant] = dict(path=str(p), expected=expected_hash, actual=actual, matches=bool(actual and expected_hash == actual))
    report["executable_hashes"] = hash_checks
    order = []
    for record in records:
        try:
            key = (int(record["repeat"]), record["variant"], record["scenario"], int(record["views"]))
            observed[key] += 1
            order.append(key)
            expected_name = f"{key[0]}-{key[1]}-{key[2]}-{key[3]}v"
            no_pilot(record["stem"])
            if key not in expected or basename(record["stem"]) != expected_name:
                raise ValueError("Unexpected run identity/stem; not a comparison-runner primary run")
            r, rows = analyze_run(directory / expected_name, record)
            if strict_guards:
                r["checks"].update(guard_checks(record, manifest))
                r["accepted"] = all(r["checks"].values())
                r["failed_checks"] = [k for k, v in r["checks"].items() if not v]
            r["identity"] = list(key)
            if observed[key] == 1:
                loaded[key] = (r, rows)
            report["runs"].append(r)
        except (OSError, ValueError, KeyError, TypeError) as e:
            report["runs"].append({"source": record.get("stem"), "accepted": False, "error": str(e)})
    pair_validations = []
    for scenario, views, size in CASES:
        pairs = []
        for repeat in repeat_numbers:
            ka, kb = (repeat, "baseline", scenario, views), (repeat, "candidate", scenario, views)
            if ka not in loaded or kb not in loaded:
                continue
            (a, ar), (b, br) = loaded[ka], loaded[kb]
            diffs = [{"frame": ra["frame"], "field": k, "baseline": ra[k], "candidate": rb[k]}
                     for ra, rb in zip(ar, br) for k in ["frame"] + COUNTERS + OCCUPANCY if ra[k] != rb[k]]
            before, after = (ka, kb) if repeat % 2 else (kb, ka)
            alternating = order.index(after) == order.index(before) + 1
            okay = (observed[ka] == observed[kb] == 1 and a["accepted"] and b["accepted"] and len(ar) == len(br)
                    and not diffs and alternating and a["render"] == b["render"])
            pair_validations.append(dict(repeat=repeat, scenario=scenario, views=views, accepted=okay, alternating_adjacent=alternating,
                                         differing_counter_cells=len(diffs), first_differences=diffs[:20]))
            if okay:
                pairs.append((repeat, a, b))
        case = {"accepted_pairs": len(pairs), "expected_pairs": repeats, "strata": {}}
        for stratum in STRATA:
            pass_names = sorted({n for _, a, b in pairs for r in (a, b) for n in r["strata"][stratum]["gpu_passes_ms"]})
            case["strata"][stratum] = {"metrics_ms": {k: paired_stat(pairs, stratum, "metrics_ms", k) for k in CPU + GPU + WALL_EXTRA},
                                       "gpu_passes_ms": {k: paired_stat(pairs, stratum, "gpu_passes_ms", k) for k in pass_names}}
        try:
            case["screenshots"] = image_difference(directory / f"1-baseline-{scenario}-{views}v.png", directory / f"1-candidate-{scenario}-{views}v.png")
        except (OSError, ValueError) as e:
            case["screenshots"] = {"available": False, "error": str(e)}
        report["cases"][f"{scenario}-{size}-{views}v"] = case
    report["pairs"] = pair_validations
    report["matrix"] = dict(expected_runs=len(expected), manifest_runs=len(records), missing=[list(k) for k in sorted(expected - set(observed))],
                            duplicate_or_unexpected=[list(k) for k, n in observed.items() if n != 1 or k not in expected],
                            runner_finished=bool(manifest.get("end_utc")), declared_repeats=manifest.get("repeats"))
    report["complete"] = (set(observed) == expected and all(n == 1 for n in observed.values()) and len(records) == len(expected)
                          and bool(manifest.get("end_utc")) and not manifest.get("failure")
                          and all(r["accepted"] for r in report["runs"]) and len(pair_validations) == len(CASES) * repeats
                          and all(p["accepted"] for p in pair_validations) and all(v["matches"] for v in hash_checks.values()))
    return report


def print_summary(report):
    print(f"A/B analysis: {report['status']}; complete and accepted={report['complete']}")
    print("Contaminated Sideways pilot EXCLUDED. No pooled-frame statistics or presented-FPS claim.")
    if "reason" in report:
        print(report["reason"])
    for case_name, case in report["cases"].items():
        print(f"\n{case_name}: {case['accepted_pairs']}/{case['expected_pairs']} accepted alternating pairs")
        for stratum, families in case["strata"].items():
            print(f"  {stratum}: median of per-run median/p95/p99 (baseline -> candidate; paired median change)")
            for family in ("metrics_ms", "gpu_passes_ms"):
                for name, metric in families[family].items():
                    if not metric["pairs"]:
                        continue
                    centers = metric["paired_center"]
                    aa = "/".join(f"{centers[q]['baseline']:.3f}" for q in ("median", "p95", "p99"))
                    bb = "/".join(f"{centers[q]['candidate']:.3f}" for q in ("median", "p95", "p99"))
                    pct = centers["median"]["change_percent"]
                    change = "n/a" if pct is None else f"{pct:+.1f}%"
                    print(f"    {name}: {aa} -> {bb} ms; {change} ({metric['pairs']} pairs)")
        print("  screenshots:", json.dumps(case["screenshots"], ensure_ascii=False))
    for r in report["runs"]:
        if not r["accepted"]:
            print("REJECTED", r.get("source"), r.get("failed_checks", r.get("error")))
    for p in report.get("pairs", []):
        if not p["accepted"]:
            print("PAIR REJECTED", json.dumps(p))
    for d in report["diagnostics"]:
        print("\nSeparate active-thread diagnostic:", d.get("source"), "accepted=", d["accepted"])
        if "thread_diagnostic" in d:
            for stratum, data in d["thread_diagnostic"]["strata"].items():
                for name, values in data["metrics_ms"].items():
                    if values.get("count"):
                        print(f"  {stratum} {name}: n={values['count']} sum={values['sum']:.3f} mean={values['mean']:.3f} ms")
        else:
            print(d.get("error"))
    for note in report["notes"]:
        print("Note:", note)


def self_test():
    assert stats([0, 1])["median"] == 1
    assert stats([None, 0, 3])["count"] == 2
    assert stats([]) == {"count": 0}
    rows = [{"arena_resets": 1}, {"arena_resets": 0}, {"arena_resets": 1}]
    assert len(select(rows, "reset")) == 2 and len(select(rows, "nonreset")) == 1
    try:
        no_pilot("pilot-baseline")
    except ValueError:
        pass
    else:
        raise AssertionError("pilot exclusion failed")
    mk = lambda n: {"strata": {"all": {"metrics_ms": {"m": stats([n] * 900)}}}}
    p = paired_stat([(1, mk(10), mk(5)), (2, mk(20), mk(10)), (3, mk(30), mk(15))], "all", "metrics_ms", "m")
    assert p["paired_center"]["median"]["change_percent"] == -50
    assert p["paired_center"]["median"]["difference_ms"] == -10
    guarded = dict(variant="baseline", status="completed", executable_sha256_before="ABC", executable_sha256_after="abc",
                   concurrency_before=dict(checked_utc="t1", games=[]), concurrency_after=dict(checked_utc="t2", games=[]))
    assert all(guard_checks(guarded, {"baseline_sha256": "abc"}).values())
    assert not guard_checks({**guarded, "concurrency_after": None}, {"baseline_sha256": "abc"})["known_game_guards_clear"]
    assert not guard_checks({**guarded, "concurrency_after": dict(checked_utc="t2", games=[{"Name": "sideways.exe"}])},
                            {"baseline_sha256": "abc"})["known_game_guards_clear"]
    assert not guard_checks({**guarded, "executable_sha256_after": "xyz"}, {"baseline_sha256": "abc"})["immutable_run_executable"]
    print("Self-tests passed: Rust percentile convention, missing/zero, strata, pilot exclusion, paired run statistics, guards/hashes")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--manifest", type=Path, help="explicit manifest JSON (existing run-ab.ps1 manifests also supported)")
    source.add_argument("--input", type=Path, help="evidence directory containing manifest.json (legacy CLI)")
    parser.add_argument("--json", action="store_true", help="print full structured report to stdout")
    parser.add_argument("--diagnostic", action="append", default=[], metavar="STEM", help="separate opt-in thread CPU run stem (without .csv)")
    parser.add_argument("--require-complete", action="store_true")
    parser.add_argument("--self-test", action="store_true", help="pure in-memory checks; no run artifacts needed")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.manifest is None and args.input is None:
        parser.error("provide --manifest FILE or --input DIRECTORY")
    directory = args.manifest.parent if args.manifest is not None else args.input
    try:
        report = analyze(directory, args.diagnostic, args.manifest)
    except (OSError, ValueError, KeyError, TypeError) as e:
        report = dict(schema=1, status="invalid", complete=False, error=str(e))
        print(json.dumps(report) if args.json else f"Analysis failed: {e}")
        return 2
    if args.json:
        print(json.dumps(report, indent=2, allow_nan=False))
    else:
        print_summary(report)
    return 2 if args.require_complete and not report["complete"] else 0


if __name__ == "__main__":
    sys.exit(main())
