#!/usr/bin/env python3
"""Summarize opt-in Quest GPU samples and CPU HUD hitches; no FPS inference."""
import argparse
import ast
import json
import re
import statistics
from pathlib import Path


def summary(values):
    values = sorted(values)
    if not values:
        return None
    return {"count": len(values), "median": statistics.median(values),
            "p95": values[int((len(values) - 1) * .95)],
            "p99": values[int((len(values) - 1) * .99)], "max": values[-1]}


def analyze(path, skip):
    passes, hitches, phases, largest = {}, [], {}, []
    first = last = None
    for line in path.read_text().splitlines():
        match = re.search(r"XR GPU frame=(\d+) passes=(.*)", line)
        if match and int(match[1]) >= skip:
            first = first or line[:18]
            last = line[:18]
            for name, ms in ast.literal_eval(match[2]):
                passes.setdefault(name, []).append(ms)
        match = re.search(r"XR detail hitch frame=(\d+).*?_ms=(\[[^]]+\])", line)
        if match and int(match[1]) >= skip:
            parts = ast.literal_eval(match[2])
            hitches.append(parts)
            detail = re.search(r"upload/shape/warm/outline/atlas_ms=(\[[^]]+\])", line)
            if detail:
                for name, ms in zip(["upload", "shape", "warm", "outline", "atlas"], ast.literal_eval(detail[1])):
                    phases.setdefault(name, []).append(ms)
            largest.append((sum(parts), line))
    return {"source": str(path), "skip_first_frames": skip, "first_gpu_sample": first,
            "last_gpu_sample": last, "gpu_ms_both_eyes": {k: summary(v) for k, v in passes.items()},
            "hud_hitches_only_over_4ms": {k: summary([v[i] for v in hitches])
                for i, k in enumerate(["prepare", "encode", "submit", "overlay_acquire", "overlay_submit"])},
            "hud_prepare_phases_in_hitches": {k: summary(v) for k, v in phases.items()},
            "largest_hitches": [line for _, line in sorted(largest, reverse=True)[:10]],
            "limits": "CPU wall phases and GPU timestamp intervals differ. Instrumentation can perturb performance. Hitches exclude ordinary HUD frames."}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--skip-frames", type=int, default=240)
    args = parser.parse_args()
    print(json.dumps(analyze(args.log, args.skip_frames), indent=2))
