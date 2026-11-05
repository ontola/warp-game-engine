# Native workspace separation

24 October 2026, r29. One Cargo workspace and lockfile at the repository root.

`engine/crates/{kern,render,audio}` contains reusable engine systems. Sideways
owns its app, simulation, deterministic math, assets, tuning, replay scenarios
and launch/GameNight tools under `games/sideways/`. Package names remain stable;
`gdcompat` is active deterministic RNG/math, not a Godot dependency.

Game assets use the explicit `SIDEWAYS_GAME_ROOT`, an executable-adjacent
`assets/` and `data/` bundle, or the source game directory during development.
An explicit invalid root fails rather than silently loading checkout assets.
Audio receives caller-provided sound presets. Engine fonts, reusable IR/audio
resources and licensed renderer test fixtures stay in `engine/assets/`.

The legacy Godot runtime, scenes, tests and export pipeline were removed.
Duplicate extracted car textures were removed only after checking the embedded
GLB images byte-for-byte. Native meshes, textures, provenance and licenses remain.
Windows CI now tests/builds/packages the native workspace; its remote execution
has not been verified by this local change.

## Recovery and evidence

Before deletion, 274 files (5,723,370 bytes) were individually verified against
`C:/dev/perf/warp-sideways-20261024/r29-repo-cleanup/legacy-godot.zip`.
Archive SHA-256:
`aa7e2b941baf4d7a92e6be399339eff05c82a27c392c62d7d19eaec52a5f8f49`.
The same directory holds deletion and verification manifests. Historical builds,
screenshots and measurements were preserved. Automatic approval review blocked
removal of empty legacy directories; they have no runtime role and remain.

Local verification:

- Root workspace metadata/check passed. All-target workspace tests: 731 passed,
  zero failed, eight intentionally ignored. One stale renderer-test texture path
  was fixed with a separately licensed engine fixture.
- Diagnostics Python suite: 42 passed. Windows launcher checks passed.
- Release build succeeded; native package created by
  `games/sideways/tools/package-windows.ps1`.
- Packaged simulation from a separate working directory: 45,000 ticks,
  identical summary and `f811a948ee4d3689` simulation hash before/after relocation.
- Packaged four-view GPU replay: 240/240 exact render-state checksums match the
  pre-move baseline. Capture geometry inventories, selected work, camera poses
  and settings match; only measured sky bake time differs in capture metadata.
  Final state differs only in startup and measured CPU timing fields.

The repeated packaged route also matches all 240 render states and capture
counters. Its covered-GPU median was 36.58 ms (p95 39.00 ms); the first package
run was 39.16 ms (p95 41.64 ms), with more CPU timing variation. The earlier
category baseline was 35.52 ms. No matched interleaved before/after performance
test of the relocation was performed, so no performance parity or win is claimed.
The full comparison is `r29-repo-cleanup/migration-verification.json`.

These checks verify relocation, not a new visual-quality or performance win.
The scene remains expensive; category measurements and the next architecture
change are in [render categories](render-categories-2026-10-24.md) and the
[forest roadmap](forest-rendering-roadmap.md). GPU timings, captures and manifests
are retained under `r29-repo-cleanup/package-fourplayer/`.

Changes remain local and uncommitted, preserving the existing working tree.
