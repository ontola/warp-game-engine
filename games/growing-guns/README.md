# Growing Guns (vertical slice)

A slice of the Godot game Growing Guns on [Warp](../../engine/README.md):
one procedural castle arena, the movement set (jump, double jump, wall jump,
dash), the rifle with projectile bullets and headshots, last-man-standing
rounds, bots, 1 to 4 local players in split screen and host-authoritative
online play over iroh. Its main purpose is to exercise Warp as a second,
very different game and to find where the engine is in the way; see
[engine/docs/second-game-findings.md](../../engine/docs/second-game-findings.md).

- `sim/` is the deterministic 60 Hz gameplay: arena generator (a port of
  `arena_generator.gd`: walled, hole and open variants, mirrored layouts),
  collision through `kern::collide`, movement and weapon constants from
  `player.gd` and `weapon.gd`, rounds and bots. `World` implements
  `kern::Sim`. No rendering dependency.
- `app/` is the game on Warp: it implements `app::Game` (window, tick,
  input and presentation come from `engine/crates/app`), builds the scene
  from `render::primitives` with the arena in `render::slots` (new rounds
  need no renderer rebuild), exposes `state()` for `script`, and adds the
  HUD, `shot`, `bench` and online sessions.

```sh
cargo run -p growing-guns-app -- play                      # 1 player + 3 bots
cargo run -p growing-guns-app -- play --players 4          # split screen, pads
cargo run -p growing-guns-app -- play --host               # prints a ticket
cargo run -p growing-guns-app -- play --join TICKET
cargo run -p growing-guns-app -- nettest                   # host + client over loopback
cargo run -p growing-guns-app -- shot --out s.png --views 4 --seconds 4
cargo run -p growing-guns-app -- bench --views 4 --size 3840x2160
cargo run -p growing-guns-app -- sim --seconds 120 --trace players.0.health,round
cargo run -p growing-guns-app -- script games/growing-guns/scripts/smoke.json --out /tmp/gg
cargo test -p growing-guns
```

The dev profile is the iteration loop: a code change rebuilds in seconds and
plays fine. `shot` and `bench` open no window (`KERN_ADAPTER=software` runs
them on a CPU rasteriser). Controls: WASD, mouse, Space jump, Shift or right
mouse dash, left mouse fire, R reload; on a pad the sticks, A jump, B or RB
dash, RT fire, X reload. F12 saves a screenshot.

Not ported: cards and weapon growth, other weapons and specials, destruction,
ragdolls, sounds, menus, GameNight. Online play is tested over loopback (`nettest`), not yet between two
machines; clients predict only their look direction.
