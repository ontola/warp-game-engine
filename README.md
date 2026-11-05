# Warp

[Warp](engine/README.md) is a Rust game engine for couch games (up to four
players on one screen) and procedural outdoor worlds, built for fast
iteration with AI: everything is code and text, simulations replay bit for
bit, rendering runs headless, and a running game can be inspected and
changed over MCP.

Games on it:

- [Sideways](games/sideways/README.md): a gravel rally game for
  [GameNight](https://gamenight.ontola.io), with up to four local players.
- [Growing Guns](games/growing-guns/README.md): an arena shooter, ported from
  the Godot original, with a native Quest build.
- [SpaceRacer](games/spaceracer/README.md): a native development port with
  Luau rules.
- Spellstorm and GameNight's Game Room lobby: being ported.
- [Hello Warp](games/hello-warp/README.md), Neon Siege and Neon Trails: games
  made only of Luau scripts and sound patches, run by the
  [`warp` executable](engine/docs/warp-cli.md) without compiling anything.

One Cargo workspace and lockfile at the root. Engine crates live in
`engine/crates/`, games in `games/`; games depend on the engine, never the
reverse.

```sh
cargo run --release -p app --example terrain_fly   # fly over procedural terrain
cargo run --release -p cli --bin sideways          # Sideways
cargo run --release -p warp -- run games/hello-warp  # a Luau game
```

Documentation starts at [engine/docs/README.md](engine/docs/README.md), and
[status](engine/docs/status.md) says what works now and what is open.
Dated experiments and measurements are in the
[development log](engine/docs/log/README.md). [`website/`](website/README.md)
builds the docs into a static site. Agents: read [AGENTS.md](AGENTS.md).

Target: four players in one 4K output at 120 FPS; not yet reached.

The Godot implementation and export workflow are retired.
[Third-party asset credits](games/sideways/CREDITS.md).
