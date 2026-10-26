# Warp diagnostics

Run from the repository root. Python tools require Python 3.11+.
Use a fresh output directory for each run and run GPU workloads serially.

- `run-frame-scenario.py`: runs an already built game with a declarative scene.
- `frame-budget.py`: GPU passes, CPU phases, cadence and diagnostic-frame filtering.
- `render-category-report.py`: controlled colour-geometry omission comparisons.
- `iteration-report.py`: startup, cache, build and frame evidence.

See [frame-budget.md](frame-budget.md) for contracts and limitations.
Sideways launchers and GameNight acceptance tools live in
[`games/sideways/tools`](../../games/sideways/tools/).
Scene definitions live in `games/sideways/scenarios/`.
