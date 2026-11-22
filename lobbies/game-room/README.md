# Game Room in Warp

The Splitscreen Club Game Room lobby (gamenight `lobbies/game-room`), rebuilt on
Warp: the same room, props, doors, shelf and lobby logic, talking to the
Splitscreen Club host over the lobby protocol.

```
cargo build --release -p game-room
game-room play                 # demo party with bots, in a window
game-room lobby [--windowed]   # the real lobby: joins the host at GAMENIGHT_ADDR
game-room live-shot --host --out room.png   # headless, against a running host
```

Started by the host with `GAMENIGHT_LOBBY_API=1` and no arguments, it runs
as the lobby, so it registers like any replacement lobby. Add it to
`local-games.json` in the Splitscreen Club data folder (next to `shelf.json`) with
absolute paths:

```json
[{"id":"warp-game-room","title":"Game Room (Warp)","launch":{
  "command":"C:\\dev\\warp-playtest\\target\\release\\game-room.exe",
  "env":{"GAMENIGHT_LOBBY_API":"1"}
}}]
```

Then pick it under **Switch lobby** in the Start menu, with
`gamenight-launcher --choose-lobby`, or at `http://127.0.0.1:7913/host/lobby`.

In a browser, with the Luau games on its shelf and a small host in the
page (`examples/browser/main.rs`): `python3 engine/tools/build-web.py --warp
--serve`, see engine/docs/browser.md. `game-room bake-light --out FILE`
writes the room's light bake that build ships, so the page need not bake.
