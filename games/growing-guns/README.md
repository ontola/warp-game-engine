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
cargo run -p growing-guns-app -- play --give explosive     # start with a card (repeatable)
cargo run -p growing-guns-app -- play --players 4          # split screen, pads
cargo run -p growing-guns-app -- play --host               # prints a ticket
cargo run -p growing-guns-app -- play --join TICKET
cargo run -p growing-guns-app -- nettest                   # host + client over loopback
cargo run -p growing-guns-app -- shot --out s.png --views 4 --seconds 4
cargo run -p growing-guns-app -- bench --views 4 --size 3840x2160
cargo run --release -p growing-guns-app -- bench --stress bullets --csv bullets.csv
cargo run --release -p growing-guns-app -- bench --stress explosions --csv explosions.csv --shot explosions.png
cargo run -p growing-guns-app -- sim --seconds 120 --trace players.0.health,round
cargo run -p growing-guns-app -- script games/growing-guns/scripts/smoke.json --out /tmp/gg
cargo test -p growing-guns
```

The dev profile is the iteration loop: a code change rebuilds in seconds and
plays fine. `shot` and `bench` open no window (`KERN_ADAPTER=software` runs
them on a CPU rasteriser). Controls: WASD, mouse, Space jump, Shift or right
mouse dash, left mouse fire, R reload; on a pad the sticks, A jump, B or RB
dash, RT fire, X reload. F12 saves a screenshot.

Gameplay is at parity with Godot 3daf27e (specs from `player.gd`,
`weapon.gd`, `bullet.gd`, `blast.gd`, `grenade.gd`, `game.gd` and
`round_modifiers.gd`; checked in `sim/src/tests.rs` against Godot's tick
counts):

- all 52 cards (`mods/base/cards.luau`, line-for-line from
  `card_library.gd`, rarity-weighted offers with the pity slot and the
  trailing-player factor) on a `Weapon` with Godot's fields and getters;
- the rifle: muzzle-to-crosshair pellets in a centre-biased cone, movement
  spread, bloom and recoil that wear off, self push, the projectile budget,
  165 m/s bullets with drop, rays every other tick, three hit ellipsoids that
  scale with the body cards, ricochet, drill, body bounce, homing, hitscan,
  grow, last bullet, poison, slow, lifesteal, scavenger, explosive splash
  (`Blast`, with sound-speed delays), impact mines;
- the RMB specials: grenade (default), cluster, teleport, zoom, sword with
  backstabs, air strike, ion cannon, with echo, tactical reload and refresh
  (several strikes may be under way at once; Godot's single-strike gate was
  a bug);
- dash cards (dodge, ninja slash, backblast bomb), acrobat jumps, phoenix
  ascent and sky drop, lava pool and burn band, falling off the map;
- rounds: the 2.75 s spawn cage, kills in order (trades go to whoever died
  second), offers 0.35 s after a win, the dead pick mid-round with 3+
  players, bot picks after 1.2 s, no pick deadline for a solo human, match
  end at ten wins (`rematch`, `extend`), the twelve round modifiers, pickups
  and the lava leak at 30 s.

Debug commands (`cmd` steps, `help`): `give`, `special`, `modifier`,
`round`, `cage`, `rematch`, `extend`, `lua`, `mods` and more.
`play --modifier ID`, `--rounds N`.

Mods: the cards and game rules are Luau in [`mods/`](mods/API.md), run by
`engine/crates/script` (sandboxed, a step budget per call, a load error
keeps the previous version). Edit a file while the game runs and it reloads
within a quarter second; `state` survives the reload. Hooks (`hit`, `kill`,
`tick`, `round_start`, a card's `pick`) run inline in the host's sim tick, so
matches still replay exactly. The debug console runs Luau against the live
match: `lua g:give(1, 'explosive')`, `mods` lists modules and errors.
`growing-guns mods-api` regenerates `mods/API.md` and `mods/types.d.luau`
from the Rust side; `GG_MODS=DIR` points at another mods folder.

The original knight, paladin and classic zombie now replace the primitive
players, including their original textures and idle/run/jump clips. Weighted
skinning runs once per frame on the GPU, shared across split-screen views and
shadow maps. Weapons follow the animated right-hand socket. The physics capsule
and gameplay rules are unchanged. See [asset provenance and regeneration](assets/characters/README.md).
`GG_CHARACTERS=off` restores the explicit primitive fallback; a directory value
loads an alternate roster. Packaged builds should include `assets/characters`
beside the executable; development builds also find the source assets.

Character limitations: corpses currently tumble as whole models in their last
pose; skin-aware dismemberment and jointed character ragdolls are not ported.
Aim/spine and strafe animation layers are not yet equivalent to the original.
Animated pixels reject temporal history until deformation motion vectors are
implemented. The older benchmark results below used primitive characters and
are not measurements of the new roster.

Not ported yet: wave survival (coop), Godot's bot AI (bots still use the
slice's simple brain), ghosts planting mines (unreachable in Godot at this
commit), the island lava layout, music, GameNight. Online play is tested
over loopback (`nettest`), not yet between two machines; clients predict
only their look direction.

The opt-in stress fixtures run 180 warmup ticks and 900 measured ticks at the
simulation's 60 Hz, with one offscreen render and GPU completion wait per tick.
They sustain 210 rifle shots or 40 gameplay explosions per simulated second;
these rates are not wall-clock presentation FPS. Explosion targets are restored
every three simulated seconds, or sooner when depleted, and those costs remain
in the measurements. Players are pinned and kept alive. The stress path flushes
arena changes, debris and effects just as play does; audio and HUD are excluded.
It checks actual event counts, writes raw CPU/GPU timings and reports effect
pool limits. Screenshot readback happens after measurement. The ordinary bot
benchmark is a different fixture; these results are not interchangeable with
its old results or with Godot's differently staged stress scenes.

Recorded measurements from 2026-10-28:

- [Sustained bullet/explosion results](tools/stress-results-20261028.json):
  three alternating pairs per layout, retained executable identity and workload
  counts. These measurements precede the rebase onto the newer renderer; do not
  present them as measurements of the latest commit.
- [Godot probe and evidence](tools/godot-stress/README.txt): existing Godot stress
  scenes, with resolution, event counts and timing scope recorded separately.
- [Renderer configuration sweep](tools/perf-results-20261028.json): measured
  commit `74ea8f0`, ten successful Vulkan configurations and two failed DX12
  attempts. DX12 fails compiling the `cull` pipeline in FXC (X3663), before any
  timing data. This sweep is one fixed-order repetition, not evidence for a
  default change. At 4K/four views the default GPU median is 4.15 ms and serialized
  CPU-plus-GPU-wait median/p95 is 7.57/12.85 ms; this is an offscreen fixture, not
  a sustained 120 FPS gameplay claim. Each measured run retains all 600 requested
  GPU profiles. The sweep includes complete scene/effect uploads, unlike older
  ordinary-benchmark results.
