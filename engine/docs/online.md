# Online couches

Every Luau game plays online without netcode of its own. One couch hosts:
it runs the game. Another couch follows: its people sit in the host's
roster like local people, its pads travel to the host, and the world
comes back. The follower never runs the game's Luau; it shows what the
host sends. Design and reasons: gamenight-internal
`docs/strategy/online-multiplayer.md`.

```sh
warp run games/neon-siege --host            # prints the ticket
warp run games/neon-siege --join TICKET     # another couch, its own pads and screen
warp nettest games/neon-siege --ticks 600   # both in one process over loopback, no GPU
```

`--lan` keeps the connection to direct addresses (no relays), and
`--ticket-file F` (host) writes the ticket to a file that `--join F` reads.
`--players N` on the follower sets how many people sit on that couch.

## In the browser

The same couches over WebRTC (`engine/crates/warp/src/online_web.rs`): one
ordered data channel per follower carries the same messages, in chunks of
at most 16 KB. Offers, answers and ICE candidates go through the cloud's
signaling socket `/v1/signal/ROOM` (gamenight-internal
`crates/gamenight-cloud/src/signal.rs`; a room lives while its host is
connected, the long random code is the key).

```text
/play/?game=neon-siege&host=1          # prints the follow link in the log (?debug)
/play/?game=neon-siege&follow=ROOM     # another couch
```

That is the bare build (`online_web::from_page`). The Game Room build
(Splitscreen Club at `/play/`) needs no parameter: **every Luau game it
starts is online**, under the room's own code (signal room `club-CODE`).
The room's QR code and join link stay the one way in: on a computer that
can host (WebGPU, a mouse, no touch screen) `/join?room=CODE` opens
`/play/?follow=CODE`, which follows whatever the host plays, the next game
too; a phone joins as a controller as before, and a follow page whose room
plays nothing online goes on to `/app/?room=CODE`. One WebRTC link lasts
the page (`online::Shared`, `Up::Lobby`/`Down::Lobby` for the page's own
messages); `?online=off` keeps a page local. Code:
`lobbies/game-room/examples/browser/online.rs`.

`?signal=wss://HOST` points at another signaling server (default: the
page's origin) and `?ice=stun:...,turn:...` sets the ICE servers (default
`stun:stun.cloudflare.com:3478`; `?ice=` for none, on one machine). Built
with `engine/tools/build-web.py --warp --bare`. Tested 2026-11-25 with two
pages of one Chromium (software GPU) and a local `gamenight-cloud`: the
follower joined, got the world and its player was seated on the host; the
same again through the Game Room build.

## What travels

The code is `engine/crates/warp/src/online.rs` (wire format and diff) and
`online_game.rs` (the game side). The transport is the `net` crate (iroh,
QUIC) on native builds; the format does not depend on it.

- **Up** (follower to host): a hello with the couch's people, screen size
  and game id; its pads every tick; its screen when the window resizes; a
  ping each second.
- **Down** (host to follower), 30 frames a second (`online::SEND_HZ`):
  - entities as changes to what that follower has: new or reshaped
    entities in full, looks and wires when only they changed, poses
    (millimetres and 1/10000 radian, varints) when only they moved, and
    removed ids. A new follower starts from nothing, so its first frame is
    the whole world. Effects (`fx.burst`, `fx.ring`) are entities, so they
    come along.
  - one camera per follower person (the shared camera for `split = false`);
  - the HUD the game drew **for the follower's screen**: after each sent
    tick the host runs the game's `draw()` again with the follower's screen
    size and its people's split-screen rectangles, and every other person's
    view placed far off screen. Items off screen are dropped. The HUD is
    sent as a patch against the previous one;
  - the sounds played that tick (`sound.play`), played again on the
    follower at the same positions;
  - the sky's name when it changes.
- The follower ticks at the frame rate and shows one frame per tick, so the
  engine's usual between-tick interpolation smooths the motion and a late
  packet does not stall the picture (more than three waiting: it catches up).

On the host, `local_players()` counts only this couch's people, so its
pads never drive a follower's seat, and only this couch's views are drawn.
Start from a follower never reaches the host (the host's menu stays the
host's). Lobbies hold at most 8 people over all couches. A follower that
leaves frees its seats (`player_left`, `players_changed`).

## Measured (loopback, `warp nettest`, 2026-11-25)

| Game | Entities | kbit/s to one follower | Notes |
| --- | --- | --- | --- |
| Neon Siege | 50 to 130 | 350 to 600 | Wires and glow change every tick; HUD small |
| Dead Miles | 1600 | 140 | Mostly still scenery; humans not replicated yet |
| Stack Together | 520 | 165 | Poses only |
| Neon Trails | 330 | 90 | |
| Relay Keepers | 0 | 420 | Drawn in the HUD |

Every run mirrored the host's world exactly (to the wire's precision), and
the follower's pad moved its pilot in Neon Siege. Round trip over loopback:
about 5 ms. Not yet measured between two machines.

## Not yet

- **Warp Humans, vehicles, explosions (`fx.explosion`), plants, rooms,
  generated meshes** are not replicated: a game using them shows them on
  the host only. Generated meshes arrive as hidden entities.
- `light.set`, `post.set`, `lamps` and `sky.set_time` changes after
  `init`: the follower keeps what its own `init` set.
- Sound handles (`sound.set`, `move`, `stop`) are not forwarded; plays are.
- No prediction: a follower's own pilot moves one round trip after the
  pad. A game hook for that (`predict`) is the next step if it feels slow.
- Under a Splitscreen Club party host, seats from the party and online
  seats are not combined yet.
- Browser and native hosts cannot play together yet: native speaks iroh,
  the browser WebRTC. Native WebRTC (str0m or webrtc-rs) closes that gap.
- No TURN server yet: two couches behind strict NATs may not connect.
- Voice is not in yet (an audio track on the same peer connection).
- Rust games keep their own netcode (Growing Guns, [networking.md](networking.md)).
