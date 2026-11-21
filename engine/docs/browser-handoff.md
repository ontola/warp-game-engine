# Handoff: Splitscreen Club in a browser

Written 21 November 2026 to hand the browser host to the next agent. How
the build works now is in [browser.md](browser.md) ("Luau games:
Splitscreen Club in a browser"); this file lists what is done, how to test
it, and what is left, in order.

## Goal

A host that runs entirely in a browser tab, with no install: one engine
download (the Warp runtime as wasm with WebGPU), then the Game Room lobby,
then Luau games that load in under a second each. Only Warp + Luau games:
Godot, Unity, LÖVE and Rust games stay on the native host. The plan is for
this zero-install link to be the real public launch (around February 2027).

## Done (warp-game-engine main, a8ef848)

- Luau (mlua, C++) compiled to wasm32 against wasi-libc and libc++; no C++
  exceptions (`engine/crates/script/src/web.rs`).
- `vfs` crate: game files come from fetch in a browser, `std::fs` natively.
- Gamepads through the Gamepad API (`engine/crates/app/src/web_pads.rs`),
  sound through cpal's WebAudio host (`engine/crates/app/src/sound.rs`).
- `app::web` (`engine/crates/app/src/web.rs`): `switch` and
  `switch_keeping` swap games on the same GPU device without a reload;
  `on_exit` catches Back (Select / Tab) and a game's Quit.
- `warp::web` (`engine/crates/warp/src/web.rs`): fetch a game folder, load
  it, the shared WebAudio mixer, the query string.
- `lobbies/game-room/examples/browser.rs`: the Game Room first, with a small
  host inside the page. The room's commands come out of `Lobby::outbox`;
  Play loads the game, Back returns to the room with the game paused, the TV
  offers Resume. The queue holds every game and never runs dry.
- `game-room bake-light` bakes the room's light ahead (`room-light.bin`);
  the page builds the room in 2 s instead of 44 s.
- Glass is left out on devices with 16 sampled textures per stage (it needs
  17); the page logs `no glass`.

Measured in the cloud on SwiftShader (no GPU; frame times there mean
nothing): wasm 27.3 MB (about 11 MB gzipped), a game loads in 0.2 to 0.5 s,
first frame after Play about 1 s.

## Build and test

```sh
sudo apt install wasi-libc libc++-dev-wasm32 libclang-rt-dev-wasm32
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <the one in Cargo.lock> --locked   # the script says which
python3 engine/tools/build-web.py --warp --serve     # http://localhost:8790/
```

`?players=3` sets local players (default 2), `?game=neon-siege` starts in a
game. `--bare` builds the games without the room
(`engine/crates/warp/examples/warp_web.rs`).

Without a GPU (cloud containers): headless Chromium loses the WebGPU device
on present. Run Chromium headful under xvfb with `--enable-unsafe-webgpu
--use-angle=swiftshader --enable-features=Vulkan --use-vulkan=swiftshader`
(`xvfb-run -a -s "-screen 0 1280x800x24" node test.js` with Playwright).
A frame takes about 10 s there, so hold keys for 20 s or the lobby never
sees them. Never present such screenshots as evidence of quality or speed.

## What is left, in order

1. **Test on a real GPU.** Only SwiftShader has run it. Joep tests the zip
   in Chrome on a normal PC; fix what that shows first.
2. **Phones and profiles: connect to the cloud room.** The page host now
   fakes a party (`Lobby::new` demo mode, fixed players, no QR code, no
   profile pickup, no phone controllers). Replace the fake party with the
   real Splitscreen Club room: the party snapshot, seats, playlist, player
   links and phone controller frames. Read first:
   - `gamenight-internal` (the cloud server; rooms, phones, `/api/player-links`)
     and its `docs/strategy` (Joep expects these read before designing).
   - `ontola/gamenight` `contract/protocol.schema.json` (the wire protocol)
     and `ontola/gamepad-party` `gamenight-core` (pure serde, compiles to wasm).
   - `engine/crates/splitscreen-club` (the native lobby client the room
     uses: `lobby.rs`). It uses tungstenite and threads; a browser needs
     `web_sys::WebSocket` and `fetch` instead.
   The browser host must do what the native host does for the room: answer
   `next`, `queue_next`, `queue_game`, `remove_playlist_entry`,
   `close_overlay` (now in `Room::command` in `examples/browser.rs`) and
   send the party snapshot to the room with `Lobby::set_party`.
3. **Players come and go.** Local players are fixed by `?players=`. A pad
   that wakes up should join as a player (in the room and the next game);
   one that leaves should drop out after the room's usual timeout.
4. **Put it online.** Serve it from splitscreen.club: Brotli or gzip, long
   cache with hashed file names, WebGPU feature check with a clear message
   in other browsers. Build in CI (the apt packages above; note that
   warp-game-engine's GitHub Actions currently fails before any step, a
   billing issue, so check with Joep).
5. **Glass within 16 textures.** `render/src/transparency.rs` + `frame.rs`
   `transparency_sampled_textures`: the glass pass sees the scene group's
   textures plus six of its own. Fit it in 16 (share or drop one binding)
   so glass shows in normal browsers.
6. **Lua errors.** A Lua error now ends the game (`__cxa_throw` panics).
   Real exceptions need libc++abi built with `-fwasm-exceptions`.
7. **Saves.** `warp::storage` is in memory on wasm; move it to IndexedDB so
   saves and settings survive a reload.
8. **Sound off the main thread.** WebAudio runs on the page thread and
   stutters on a long frame; an AudioWorklet fixes that.

## Rules of this repository and project

- Read `AGENTS.md` first. Commit straight to `main`, no pull requests.
- Docs describe the current state: update `browser.md` in place with each
  change (this handoff file may be deleted once the list is done).
- Dependencies point one way: games and lobbies depend on the engine, never
  the reverse. The browser host lives in `lobbies/game-room/examples/`.
- The Game Room's code (`lobbies/game-room/src`) has its own owner; keep
  changes there small and say what you changed.
- Engine rule: every render pass fits 8 storage buffers per shader stage.
