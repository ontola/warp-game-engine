# Warp

A game engine focused on realistic procedural outdoor worlds, with Sideways,
a rally game for [GameNight](https://gamenight.ontola.io), as its playable testbed.
The active Rust workspace, setup and development commands are in
[`engine/`](engine/README.md).

## Current focus

- Realistic, performant procedural nature: terrain, vegetation (trees and
  grass), and sky.
- Vehicle physics, exercised in Sideways.
- AI-driven development workflows and useful engine APIs.
- Fast iteration, including compilation and the build-to-play loop.

These are development priorities, not a claim that every system is complete.
The focus can change as the engine develops.

## Missing systems

There is no human-facing editor UI or complete support for indoor scenes,
water, buildings, people, or a general animation system. Procedural plant wind
is in development; it is not a character or general-purpose animation system.

## Legacy Sideways (Godot)

The original game below is retained as historical source. Active engine work
and playtests use Warp; ongoing Godot comparisons are no longer a goal.

A gravel rally game for [GameNight](https://gamenight.ontola.io), with up to
four drivers racing the same forest stage in local split screen.

Controls: left stick steers, RT accelerates, LT brakes, A is the handbrake,
X resets and Y changes camera. Run it with Godot 4.5:

    godot --path . -- --players 2

See [AGENTS.md](AGENTS.md) for tests, screenshots and releases.

The graphics target is four players at 4K and 120 FPS; it is not yet reached.
For a repeatable native 4K benchmark, use the real renderer:

```sh
godot --path . --resolution 3840x2160 --fullscreen --disable-vsync --fixed-fps 120 -- --benchmark 1200 --benchmark-warmup 240 --views 4 --rivals 0 --seed 4242 --at 300 --weather clear --km 2
```

This drives four cars along the same route, warms up 240 rendered frames,
then measures 1200 frames using wall time. Verify output 3840x2160 and four
1916x1076 viewports at scale 1.0 in the final JSON. It reports FPS,
median/p95/p99 frame times, draw calls and the engine primitive counter
(which is not a reliable physical triangle total for MultiMesh LODs).
Keep engine, hardware and background load matched; report upscaling separately.
Use `--views 1` / `--views 2` for other layouts and `--strip trees`,
`--strip grass`, etc. only to isolate costs.

Graphics regressions cover mip data/prop LODs (`tests/graphics.gd`), actual
viewport resizing (`tests/view_resolution.gd`), tyre uniforms, stage-query
caches and tree tangents. Run them headless; CI includes all five.

See the [graphics iteration report](docs/graphics-performance.md) for measured
before/after results, visual references and remaining split-screen limits.
The [overnight iteration log](docs/graphics-night-2026-10-21.md) records the
ongoing reference-driven graphics review and accepted/rejected experiments.
