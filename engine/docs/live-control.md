# Live control over MCP

A running Warp game can serve the
[Model Context Protocol](https://modelcontextprotocol.io) (MCP), so you
can change it while you play, together with Claude (Claude Desktop, Claude
Code), ChatGPT, or Gamepad Party's session assistant (see below). Ask "make the
shotgun spread wider and let me hear the new shot", and the assistant
reads the match, edits a Weapon field or a Luau card, renders the sound
patch, reloads a shader and takes a screenshot to check the result.

The engine side is the `live` crate (`engine/crates/live`). Growing Guns is
the first game wired to it (`games/growing-guns/app/src/live.rs`); every
Luau game run by the [`warp` executable](warp-cli.md) has it too
(`warp run DIR --live`, `warp serve DIR`).

## Quick start (Growing Guns)

```sh
cargo run --release -p growing-guns-app -- play --live
```

The game prints its endpoint and a fresh token:

```text
live: MCP on http://127.0.0.1:7878/mcp (bearer token 3f9c...e1)
live: token file /tmp/warp-live/growing-guns.json (the warp-mcp bridge reads it)
```

Then connect a client (below) and ask it to call `describe` first.

Flags (any game that uses `live::Config::from_args`):

| Flag | Meaning |
| --- | --- |
| `--live [PORT]` | Serve MCP. No port: 7878. `0`: any free port (read it from the token file). |
| `--live-token TOKEN` | A fixed token instead of a random one per run. `WARP_LIVE_TOKEN` does the same. |
| `--live-token-file PATH` | Where to write `{game, url, token, pid}`. Default: `WARP_LIVE_DIR`, else `<temp>/warp-live/<game>.json`. |
| `--live-remote` | Accept any `Host` and `Origin` header (tunnels, LAN). The token is still required. |
| `--live-bind ADDR` | Listen on another address than 127.0.0.1. A non-loopback address also needs `--live-remote`. |

`WARP_LIVE_CONSOLE=0` stops the server from copying stdout and stderr into
the `logs` tool. The copy works on Unix only. On Windows, `logs` holds what
the game and the server log themselves.

### Headless, without a window

`growing-guns serve --live [PORT] [--seconds T] [--total N] [--size 960x540]`
runs the same game in real time with bots in every seat and no window. It
renders a frame only for a screenshot or a shader reload. The game's sound
runs into a mixer that is thrown away, so `play_sound` and the game's own
triggers work, silently. It is used for tests and works on machines with
no GPU (`KERN_ADAPTER=software`):

```sh
KERN_ADAPTER=software growing-guns serve --live 0 --seconds 600
```

## Connecting Claude

### Claude Code

Over HTTP, with the token from the game's startup line:

```sh
claude mcp add --transport http growing-guns http://127.0.0.1:7878/mcp \
  --header "Authorization: Bearer <token>"
```

The token changes every run unless you fix it. To keep one entry working
across restarts, start the game with `--live-token` (or `WARP_LIVE_TOKEN`),
or use the stdio bridge, which reads the token file itself:

```sh
cargo build --release -p live --bin warp-mcp
claude mcp add --transport stdio warp -- /path/to/warp/target/release/warp-mcp --game growing-guns
```

The same as a project `.mcp.json`:

```json
{
  "mcpServers": {
    "warp": {
      "command": "/path/to/warp/target/release/warp-mcp",
      "args": ["--game", "growing-guns"]
    }
  }
}
```

### Claude Desktop

Claude Desktop starts local servers as commands, so it uses the bridge.
Add it to `claude_desktop_config.json` (macOS:
`~/Library/Application Support/Claude/`, Windows: `%APPDATA%\Claude\`)
and restart Claude Desktop:

```json
{
  "mcpServers": {
    "warp": {
      "command": "/path/to/warp/target/release/warp-mcp",
      "args": ["--game", "growing-guns"]
    }
  }
}
```

On Windows the command is `C:\\path\\to\\warp\\target\\release\\warp-mcp.exe`.

### The warp-mcp bridge

`warp-mcp [--game NAME] [--url http://127.0.0.1:7878/mcp --token TOKEN]`
is a stdio MCP server that forwards every message to the game's HTTP
endpoint.

- Without `--url`, it reads `<temp>/warp-live/<game>.json` (or the newest
  token file when `--game` is left out), and reads it again when the game
  restarts with a new token.
- It can start before the game. Until a game is running, it offers one
  tool, `warp_status`, that explains how to start one.
- It sends `notifications/tools/list_changed` when a game appears or goes
  away, so the client lists the tools again.
- Both must use the same `WARP_LIVE_DIR`, if you set one.

## The Gamepad Party session assistant

A Luau game that Gamepad Party launches (`warp run`, `GAMENIGHT=1`) turns live
control on by itself, on a free local port (`WARP_LIVE=0` turns that off;
`--live` still picks the port). It tells the party host the path of its
token file in `declare_commands` (`mcp`, see Gamepad Party's `docs/protocol.md`).
When an allow-listed player then asks the assistant in the Gamepad Party app
"make the enemies slower" or "give my ship a red trail", the host reads the
token file, lists these same tools, and carries out the request with them,
one model step at a time from the Gamepad Party cloud. The token stays on the
host, the cloud only sees tool results, and the player hears back what was
changed. Nothing here is GameNight-specific: it is the same MCP server and
the same tools that Claude Code uses.

## Connecting ChatGPT

ChatGPT connects to MCP servers from OpenAI's servers, not from your
machine. It needs a public HTTPS URL, and the local endpoint is not one.
Warp sets up no tunnel. To try it, you expose the endpoint yourself and
understand the risks below first.

1. Start the game with a token you choose and accept tunnel host names:

   ```sh
   growing-guns play --live 7878 --live-remote --live-token "$(openssl rand -hex 24)"
   ```

2. Expose port 7878 over HTTPS with a tunnel you trust. For example
   `cloudflared tunnel --url http://127.0.0.1:7878`, `ngrok http 7878` or
   `tailscale funnel 7878`.
3. In ChatGPT, add a custom connector (in developer mode, under the
   connector or app settings), with the URL
   `https://<tunnel-host>/mcp/<token>`. ChatGPT connectors cannot send
   a custom bearer header, so the server also accepts the token as the
   last part of the path. Choose no authentication in ChatGPT: the token
   in the URL is the authentication.
4. When you stop playing, stop the tunnel. The token stops working when
   the game exits.

The ChatGPT settings and their names change often. Check OpenAI's current
documentation for custom MCP connectors.

### Risks of remote access

- Anyone with the URL controls the game. They can change the match, write
  files in the moddable folders, run Luau, and see screenshots of your game.
  Treat the URL like a password. Don't paste it into chats, issues or
  screenshots.
- A token in the URL path ends up in the tunnel provider's logs, and maybe
  in proxy logs. Use a new token for each session, and never reuse
  `--live-token` values.
- `--live-remote` turns off the `Host` and `Origin` checks that stop web
  pages from reaching the local server through DNS rebinding. Use it only
  while the tunnel runs.
- `--live-bind 0.0.0.0` (with `--live-remote`) opens the endpoint, over
  plain HTTP, to your whole network. Use it only on a network you trust.
- What a client can reach stays limited to these tools. There is no file
  access outside the moddable folders, no shell, and no process control.
  Luau runs in the script crate's sandbox, with its step budget and memory
  cap. Shaders are compiled by wgpu, and a broken shader keeps the old one.
  But a mod can still change the game in any way the mod API allows.

## Security model

- **Localhost only by default.** The server binds `127.0.0.1`. Binding to
  any other address needs both `--live-bind` and `--live-remote`.
- **A bearer token on every request.** The token is random (24 bytes from
  the OS) for each run, unless you set one. It is printed at startup and
  written to a token file that only you can read (mode 0600 on Unix). The
  file is removed when the game exits normally. Tokens are compared in constant
  time. A wrong or missing token gets `401`.
- **DNS rebinding.** Unless `--live-remote` is set, a request whose `Host`
  is not `localhost`, `127.0.0.1` or `[::1]` is refused, and so is a
  request whose `Origin` is not loopback.
- **Sandboxed files.** `read_file`, `find_text`, `write_file`, `edit_file` and `list_files` see only
  the folders the game names (Growing Guns: `mods`, `sounds`, `shaders`).
  Paths are relative to a folder: absolute paths and `..` are refused, and
  symlinks are resolved and checked to stay inside the folder. Each folder
  can limit file types (`.luau`, `.wgsl`). Reads are capped at 512 KiB and
  writes at 1 MiB. Writes go to a temporary file first and are then
  renamed, so the game's file watcher never reads half a file.
- **The game never waits on the network.** Requests are queued. The game
  thread runs at most 16 of them per tick, between ticks. A request the
  game does not answer within 10 s (30 s for frames) fails with "nothing
  was changed", and the game skips it when it gets there.

## Tools

Generic tools (from the `live` crate). Each one is listed only if the game
supports it:

| Tool | What it does |
| --- | --- |
| `describe` | What the game is, how to tweak it, its tools and folders. Call it first. |
| `state` | The game's full state as JSON (`Game::state`). |
| `query` | Parts of the state by JSON pointer (`/players/0/health`, or `players.0.health`). |
| `screenshot` | A PNG of the current frame, with the HUD. `max_width` downscales it (default 1024). |
| `run_luau` | One Luau chunk in the game's script console, against the live game. |
| `list_files`, `read_file`, `write_file` | The moddable folders. A write triggers that folder's reload, and the reply says whether it loaded. `read_file` with `offset`/`limit` returns those lines, numbered, so a long file is read in parts. |
| `find_text`, `edit_file` | Find the lines that hold some text (`path:line: text`), and replace the one place an exact text occurs (it must occur once). An edit reloads like a write; for a long file it beats rewriting it whole. |
| `reload_shaders` | Recompile the WGSL shading chain on the next frame (`Renderer::reload_shaders`). A failure keeps the old shaders and returns the compiler error. |
| `logs` | Recent console output, errors and tool calls (`lines`, `filter`). |
| `play_sound` | Without a name: list the sounds with their parameters. With a name: play it once with `params`, at the listener or `at` a world position. |
| `set_param` | Change a sound parameter's default for every later play, including the game's own triggers. Values the game passes for a trigger still win. |
| `render_sound` | Render a sound offline and return what `warp-audio render` prints: length, peak, RMS, loudest 50 ms, spectral centroid and a text spectrogram. Lets a model judge an edit without hearing it. The render runs off the game thread. |

Growing Guns adds:

| Tool | What it does |
| --- | --- |
| `match_state` | Players (bot, alive, health, score, ammo, cards, offer, special, changed Weapon fields), round, phase, modifier, session. With `player`: that player's whole Weapon. With `fields: true`: every Weapon field and its kind. |
| `list_cards` | Every card on offer: id, name, description, rarity. |
| `give_card` | Give a player a card now, as if they picked it (its stats, its pick hook, and the gun grows). |
| `set_weapon` | Set Weapon fields (`weapon::FIELDS`: numbers, booleans, `[x, y, z]`, `special` by name) for one player or all. The change lasts until the round ends, because weapons are rebuilt from cards each round. Make it permanent with a card. |
| `spawn_bots` | Add bots, up to 8 players. |
| `next_round`, `restart_match` | Skip to the next round (cards are kept), or start the match over. |
| `load_mod` | Write `mods/<name>.luau` and load it now. Returns its status or the load error. A broken module keeps its previous version. See `mods/API.md`. |
| `command` | One debug console line: `teleport`, `health`, `ammo`, `special`, `modifier`, `bots pause`, `boom`, `arena`, and more. `help` lists them. |

Folders: `mods` (Luau cards and rules, `.luau` and `.md`), `sounds` (synth
patches, see [audio](audio.md)), and `shaders` (WGSL). `shaders` exists
only when the game runs with
`KERN_SHADER_DIR=engine/crates/render/src/shaders`, because the shaders
are otherwise compiled into the binary.

### Gameplay changes and online play

Growing Guns online is host-authoritative. Only the host runs the
simulation, and clients apply its snapshots. The live tools change the
world on the game thread, between ticks, exactly like the debug console.
On a host, the next snapshot carries the change to every client. Weapons
and cards travel with the players. A new arena between ticks is flagged
with `Session::mark_arena`, so clients get it too.
`growing-guns nettest` checks that a round skip and a given card made on
the host between ticks reach the client.

So the rules are:

- Gameplay tools work offline and on a host. On a client they are refused
  with a message that says to make the change on the host. `run_luau`
  still runs on a client, but the next snapshot overwrites its changes.
- `spawn_bots` works offline, or on a host before anyone has joined. The
  joined clients' views were built for the old player count.
- Mods run on the host only, inside the sim tick (World hooks). Clients
  need the same card files, for the names in their menus.
- Replays: a match with the same seed and mods replays exactly, and desktop F12 feedback includes five seconds of host/local input
  (tick, arena seed and inputs in seat order) beside the snapshot replay.
  The JSON records recent gameplay tool calls and Luau console evaluations with
  their tick and arguments. A mod file reload records its path and result.
  The journal keeps up to 128 edits and marks omitted arguments over 8 KiB.
  The snapshot player shows the recorded outcome without executing the journal.

## Adding live control to a game

1. Add `live = { path = "../../../engine/crates/live" }` to the game.
2. At startup, `live::Config::from_args("my-game", &args)?` and, if it
   returns a config, `live::start(cfg, manifest)?`. The `Manifest` holds
   the game's own tools (`ToolSpec` with a JSON Schema), its folders
   (`Root::new(name, path, Reload::Game | Reload::Shaders | Reload::None, about)`),
   and the generic features it supports.
3. Implement `LiveGame` on a short-lived context over your game: `state`,
   `call` for your tools, optionally `luau`, `file_written` (reload a
   written file and say what happened), and `sound` (a `SoundHook`).
4. In the tick, call `live.serve(&mut ctx)`. It never blocks.
5. For screenshots, return `live.wants_capture()` from `Game::wants_capture`
   and pass `Game::captured` on to `live.captured`. The app shell (window
   or headless) renders the frame offscreen as well.
6. For shaders, when `live.wants_shader_reload()` is true, call
   `frame.renderer_mut()` and its `reload_shaders(gpu)` in `Game::frame`,
   then pass the result to `live.shaders_reloaded`.

`app::run_headless(cfg, game, gpu, seconds)` runs any `app::Game` in real
time without a window, and renders frames only when the game asks for one.

## Tests

- `cargo test -p live` starts the server on a random port against a fake
  game. It checks the token, `Host` and `Origin` checks, the MCP
  initialize handshake, `tools/list`, and calls to each generic tool. It
  also checks that `render_sound` work runs off the game thread, and that
  the stdio bridge forwards (and answers alone when no game runs).
- `KERN_ADAPTER=software cargo test -p growing-guns-app --test live -- --ignored`
  runs the real game headless (`serve --live 0`, software rasteriser, no
  window), on copies of the mods, sounds and shaders. A small client then
  drives every Growing Guns tool, the mod load errors, the sound tools, a
  patch edit, a broken and a fixed shader reload, and screenshots.
- `growing-guns nettest` checks that host tweaks reach a client.
