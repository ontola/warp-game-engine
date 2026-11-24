# Vibe studio

Makes a Warp couch game from a prompt while people play: the service the
Splitscreen Club cloud calls when someone at the couch types "a game where
...". It starts from the starter or a template (`fps`, `platformer`, picked
from the prompt), has Claude Code write the game with the [game kit](../../docs/kit.md),
plays it headless with three bots (the [vibe-eval](../vibe-eval/vibe-eval.py)
score: loads, runs, lives, finishes) and sends the problems back to the
agent, up to two fix rounds. A game passes at 4 of 4. Then the cloud fetches
its files (all text, 10 to 30 KB) and the room loads them like a shelf game.

```sh
cargo build --release --locked -p warp
python3 engine/tools/vibe-studio/vibe-studio.py serve --port 8795 --data build/vibe-studio
python3 engine/tools/vibe-studio/vibe-studio.py make "penguins bump each other off a melting ice floe"
```

The API is in the script's header. In short:

| Call | Does |
| --- | --- |
| `POST /v1/games {"prompt", "room"}` | starts a game; 202 with the job |
| `GET /v1/games/ID` | the job: `status` (queued, working, ready, failed, refused), `stage` for the TV, `title`, `score`, `error` |
| `GET /v1/games/ID/files` | `{"title", "version", "files": {path: text}}` of the last playable version |
| `POST /v1/games/ID/revise {"prompt"}` | "higher jumps": the next version; the last one stays playable meanwhile |

Measured 24 November 2026 in the cloud container (Sonnet, lavapipe): "a game
where everyone is a penguin on a shrinking ice floe and you bump each other
into the sea" was ready in under 4 minutes on the first try, 4 of 4, 280
lines, as Floe Fight (fish power-ups, belly slide, a melting floe).

## Running it

- Needs `warp` (`WARP_BIN`, else `target/release/warp`) and `claude` signed
  in. The agent may only read and edit the game folder and run `warp check`,
  `warp script` and `warp templates` (`--allowedTools`); the game itself is
  Luau, which runs sandboxed in the room.
- `VIBE_STUDIO_TOKEN`: every call then needs `Authorization: Bearer TOKEN`
  (the cloud calls it server to server).
- `VIBE_STUDIO_ROOM_LIMIT` (6): new games and revisions per room per hour.
  `VIBE_STUDIO_AGENT_TIMEOUT` (900 s) per agent run. `--workers` (2) games
  at once.
- The brief has the agent refuse (REFUSED.md) what is not fit for a family
  couch or copies a named existing game; the job then says `refused`.
- `VIBE_STUDIO_AGENT` swaps the agent command (`{prompt}`, `{dir}`,
  `{tools}`). A fake agent tests the plumbing without a model: one that
  writes a broken main.luau first and a working one when PROMPT.md says
  "failed its playtest" must end `ready` after 2 attempts.

## Open

- The room cannot take a game from the cloud yet: the poll reply needs a
  "new game" entry, and the page then fetches `/files`, puts them in the
  `vfs` and loads them with `warp::web::load` (browser.md).
- No screenshot judge: a game can pass and still be too dark or have its
  camera in a wall. The agent renders a frame itself, which catches most.
- Revising a game while it is played: the room would reload it with
  `reload()` keeping `state`, as `warp run` does natively.
