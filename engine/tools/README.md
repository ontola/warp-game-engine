# Warp diagnostics

Run from the repository root. Python tools require Python 3.11+.
Use a fresh output directory for each run and run GPU workloads serially.

- `run-frame-scenario.py`: runs an already built game with a declarative scene.
- `frame-budget.py`: GPU passes, CPU phases, cadence and diagnostic-frame filtering.
- `frame-hitches.py`: visible gameplay cadence tails, preceding CPU work and
  frame-ID-matched GPU foliage-page timings; JSON output.
- `render-category-report.py`: controlled colour-geometry omission comparisons.
- `iteration-report.py`: startup, cache, build and frame evidence.

See [frame-budget.md](frame-budget.md) for contracts and limitations.
Sideways launchers and GameNight acceptance tools live in
[`games/sideways/tools`](../../games/sideways/tools/).
Scene definitions live in `games/sideways/scenarios/`.

```powershell
python engine/tools/frame-hitches.py path/to/frames.jsonl --out path/to/hitches.json
```

The hitch report excludes loading, pause, warmup, hidden/unpresented frames,
synchronous diagnostics and section/camera transitions. Both cadence endpoints
must qualify, so the interval immediately after capture/readback is excluded.
Use `--skip-frames N` for extra warmup per segment, `--top N` to bound outlier
details, and repeat `--camera-transition-call CALL_ID` for known cuts absent
from the trace metadata. Without `--out`, JSON goes to stdout; `--force` permits
replacing a report. See [the r61 investigation](../docs/frame-hitches-r61.md) for
measured examples and the CPU/GPU alignment contract.
