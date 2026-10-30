# Growing Guns on Warp

A port of **[Growing Guns (Godot)](https://github.com/ontola/growing-guns)**
to [Warp](../../engine/README.md). The Godot game is the reference: a fast
last-man-standing arena shooter where the loser of a round picks a card that
grows their gun. Gameplay follows Godot commit `3daf27e` (`player.gd`,
`weapon.gd`, `bullet.gd`, `blast.gd`, `grenade.gd`, `game.gd`,
`round_modifiers.gd`), checked tick for tick in `sim/src/tests.rs`.

This is the version GameNight plays on the ManCave. The status below is
kept current: see [Keeping this README current](#keeping-this-readme-current).

## Status against Godot

Done: as in Godot. Close: present, with the stated difference. Not yet:
missing.

| Area | Status | Notes |
|---|---|---|
| Movement | Done | Jump, double jump, wall jump, dash, step-ups (0.35 m), lava burn and kill, falling off the map |
| The gun | Done | Projectile rounds with drop, spread, bloom, recoil, ricochet, drill, homing, hitscan, poison, slow, lifesteal, explosive splash, impact mines, three hit ellipsoids |
| Cards | Done | All 52 cards, Luau in `mods/base/cards.luau`, line for line from `card_library.gd`; rarity, pity slot, trailing-player factor |
| Specials (RMB) | Done | Grenade, cluster, teleport, zoom, sword with backstabs, air strike, ion cannon; echo, tactical reload, refresh |
| Rounds and match | Done | Spawn cage, kill order, card picks (the dead pick mid-round with 3+ players), first to ten wins, rematch, extend |
| Round modifiers | Close | All twelve play; see the table below |
| Arenas | Done | Walled, hole, open and lava island (18 %), mirrored layouts; destructible stone chunks with wear, bullet dents and debris; stadium footings extend into lava, with cached orange lava illumination |
| Stadium | Done | Colosseum masonry, 8,363 animated spectators, crowd sound, four live TV screens |
| Bots | Done | Aim, reaction, strafing, jumps, dashes, pickups and specials; they keep to lava islands. Versus reserves eight visual seats so adding/removing bots keeps the renderer |
| Characters | Close | Knight, paladin and zombie with textures and clips, GPU skinning, 11-body ragdolls, model-derived head/body gibs. Cuts use cached model geometry; animated skin-aware dismemberment, aim and strafe layers remain incomplete |
| Effects | Done | Muzzle flash and shot light, bullet streaks, blasts with smoke, sparks, chips, scorch and blood, air strike rocket and white-out; ion cannon charges as one soft brightening column with wide bloom and bright ground illumination, with blue rising motes across the impact disc and nearly vertical rubble lift/fall |
| Sky and light | Done | Godot's sky by default; procedural time of day and weather, per round or as a cycle; visible sun and GPU lens flare with per-view occlusion; floodlights with shadows at night |
| HUD and menus | Done | Godot's layout, fonts (Anton, Barlow) and timing; mouse, keyboard and pads; see [docs/ui-parity.md](docs/ui-parity.md) |
| Standalone Quest VR | Experimental | Quest Browser runs the Rust sim locally via Wasm; tracked rifle/grenade, snap turn, bots, 8 demo cards and simplified graphics. [Build, controls and limits](../../engine/docs/quest-vr.md). Physical Quest 2 testing pending. |
| Split screen | Done | 1 to 4 local players, hot-join with X, own audio flat on your side of the screen. Join/leave reuses visual slots and clears unused screen regions |
| Online | Close | Host-authoritative over iroh; local movement prediction and buffered remote snapshots. Tested over loopback (`nettest`), not yet between two machines or under realistic network jitter; transport currently binds IPv4 only |
| Wave survival (coop) | Done | `--coop`: grunts, snipers, grenadiers, fraggers, bombers, revives |
| Music | Done | Godot's stems, following the match |
| GameNight | Done | Party settings, the next round's modifier, live knobs, each player's own stick sensitivity, the session assistant (MCP), loading and pause |
| Menu backdrop | Not yet | Godot's cinematic lava fort behind the start menu |
| Ghost mines | Not yet | Dead players planting mines |

### Round modifiers

| Modifier | Status | Notes |
|---|---|---|
| Low gravity | Done | |
| Fog | Close | Plays, but the fog itself is not drawn yet |
| Supply storm | Done | |
| Sudden death | Done | The lava leak from the start |
| Headshots only | Done | |
| Boom boom | Done | |
| Brrrrt | Done | |
| Gun swap | Done | |
| Sniper fight | Done | |
| Air strikes | Done | |
| Ion cannon | Done | |
| Blackout | Done | Sun, sky, floods and lamps out; every player carries a shadowed flashlight |

## Play

```sh
cargo run --release -p growing-guns-app -- play                   # 1 player + 3 bots
cargo run --release -p growing-guns-app -- play --players 4       # split screen, pads
cargo run --release -p growing-guns-app -- play --give explosive  # start with a card (repeatable)
cargo run --release -p growing-guns-app -- play --modifier blackout --rounds 3 --island
cargo run --release -p growing-guns-app -- play --coop --players 2
cargo run --release -p growing-guns-app -- play --host            # prints a ticket
cargo run --release -p growing-guns-app -- play --join TICKET
```

Keyboard and mouse: WASD, mouse, Space jump, Shift dash, left mouse fire,
right mouse special, R reload. Pad: sticks, A/LB jump, Y/RB dash, RT fire,
LT/B special, X reload. F12 saves a screenshot. F3 (Back + Start on a pad)
opens the debug overlay: give cards, set specials, drop pickups, pick the
sky, force a modifier, start the lava leak, next round, pause the bots.

For the standalone Quest Browser demo, run `python3 engine/tools/build-quest-demo.py`
then `python3 engine/tools/serve-quest-demo.py --public` from the repository root.
See [Quest VR](../../engine/docs/quest-vr.md) for controls and the limited mobile
presentation. It does not use the desktop `play` executable.

## Change it while it runs

- **GameNight app**: the party settings (rounds to win, modifiers (how
  often), card time, gravity, bullet drop, body shot damage, random pickup
  rate), a "Next
  round" modifier pick, the live knobs, and each player's stick sensitivity
  under My settings. Declared in `app/src/party.rs`.
- **Live knobs** (`mods/base/knobs.luau`): damage, fire rate, move speed,
  bullet speed, explosions, health, as multipliers on what the cards give.
  Console `knobs` / `knob NAME VALUE`, MCP `knobs` / `set_knob`, or the app.
- **Mods**: cards and rules are Luau in [`mods/`](mods/API.md), hot-reloaded
  within a quarter second, run inside the host's sim tick so matches still
  replay. `growing-guns mods-api` regenerates `mods/API.md`.
- **Live control (MCP)**: `play --live [PORT]` or, under GameNight, always on
  a free port for the session assistant: match state, cards, weapons, bots,
  rounds, mods, knobs, screenshots. See `app/src/live.rs`.
- **Console** (`help`): `give`, `special`, `modifier`, `round`, `arena SEED`,
  `arena island`, `lava`, `sky`, `light`, `lua`, `mods`, `knob` and more.
- **Environment**: `GG_HITCH=MS` logs slow steps (12 ms by default under
  GameNight), `GG_FLASHLIGHT`, `GG_BLACKOUT_EV`, `GG_FLOOD_SHADOWS=0`,
  `GG_BOUNCE`, `GG_CHARACTERS=off`, `GG_MODS=DIR`, `GG_SKY`.

## Test and measure

```sh
cargo test -p growing-guns -p growing-guns-app
cargo run -p growing-guns-app -- script games/growing-guns/scripts/smoke.json --out /tmp/gg
cargo run -p growing-guns-app -- shot --out s.png --views 4 --seconds 4   # no window
cargo run -p growing-guns-app -- sim --seconds 120 --trace players.0.health,round
cargo run -p growing-guns-app -- nettest                                  # host + client over loopback
cargo run --release -p growing-guns-app -- bench --views 4 --size 3840x2160
```

`shot`, `script` and `bench` open no window; `KERN_ADAPTER=software` runs them
on a CPU rasteriser. Results and their scope: [docs/measurements.md](docs/measurements.md).

## Layout

- `sim/`: the deterministic 60 Hz game (arena generator, collision, weapons,
  rounds, bots, coop). `World` implements `kern::Sim`; no rendering.
- `app/`: the game on Warp (`app::Game`): view, HUD, sound, music, sessions,
  GameNight party, live control, `shot`, `script`, `bench`.
- `mods/`: the Luau cards, rules and knobs.
- `docs/`: [UI parity](docs/ui-parity.md), [lava parity](docs/lava-parity.md),
  [measurements](docs/measurements.md), playtest notes.

Online peers must run the same protocol version (`warp/growing-guns/2`);
snapshots are positional postcard, so older builds cannot join.

## Keeping this README current

Every commit that adds, finishes or removes something a player would notice
updates the status tables above in the same commit (AGENTS.md says so too).
`cargo test -p growing-guns-app readme` fails when the README misses a round
modifier, a GameNight setting, a live knob or the card count, or stops
linking the Godot repository.
