# Warp and Sideways

One Rust workspace, with Cargo.toml and Cargo.lock at the repository root.

- `engine/`: reusable rendering, audio, simulation interfaces and diagnostics.
- `games/sideways/`: game app, simulation, assets, tuning and GameNight integration.
- Package names remain kern, render, audio, sideways, cli and gdcompat.
  The cli package produces the sideways executable. gdcompat is stable game
  math/RNG code, not a Godot runtime dependency.

Keep engine APIs independent of Sideways. The game supplies assets/settings.
Preserve attribution, licenses and unrelated work. Never reset or stash another
task's changes. Check absolute paths and dependencies before moves/deletions.

Physics stays at 120 Hz with four substeps. Reorganization must preserve
simulation output and retained asset bytes. The Godot implementation, export
pipeline and comparison workflow are retired.

## Validation and iteration

Run focused checks appropriate to the change:

```powershell
cargo check --locked --workspace
cargo test --locked -p render --lib
cargo test --locked -p cli --bin sideways
games/sideways/tools/test-play-dev.ps1
games/sideways/tools/play-dev-windows.ps1 -Restart
```

Run GPU fixtures serially without a simultaneous game. Stop only an owned
process, verified by executable path, PID and creation time. Keep a playtest
running during compilation where possible; a build is not proof of presentation.

Use engine/tools/run-frame-scenario.py with games/sideways/scenarios for replay.
JPEG previews are default; full PNG is for pixel tests and F12. Exclude warmup,
startup and readback from steady-frame results. CPU wall time and GPU time
are separate. Geometry omission deltas are non-additive.

Target: one 4K output, four players, 120 FPS (8.33 ms); not reached. Report actual
resolution, reconstruction, sample count and scene. No further Godot comparison.

See both READMEs, engine/HANDOFF.md and engine/docs/forest-rendering-roadmap.md.
