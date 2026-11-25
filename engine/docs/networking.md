# Networking

Luau games play online through the engine, without netcode of their own:
see [online couches](online.md). This page is the transport and the
Rust games' own netcode.

The `net` crate carries a game's own message types between a host and its
clients over [iroh](https://iroh.computer): QUIC with hole punching and
relays, which are meant to let players connect without opening ports. The crate is
small and has no idea what a game is. The protocol, the
authority model and the prediction are the game's.

## Model

- One host, any number of clients.
- One reliable, ordered stream per client. Messages are length-prefixed
  [postcard](https://docs.rs/postcard); a message may be at most 16 MiB.
- `In` messages go client to host, `Out` messages host to client. Any type
  that is `Serialize + DeserializeOwned + Send + Sync + 'static` works.
- iroh runs on its own small tokio runtime. The game loop polls channels
  and never waits for network I/O (except `Client::join`, which waits up
  to 20 s for the connection). Host output uses a short queue mutex.
- A ticket, the hex of the host's endpoint address, is what a player shares
  to join.
- `lan = true` disables relays and address publishing: direct addresses
  only, for a local network and tests. Otherwise iroh's default relays and
  lookup are used.
- The current endpoint binds IPv4 only and disables automatic router port
  mapping. Direct IPv6-only connections are not supported by this binding.
  Ticket encoding and decoding preserve peer addresses without local-subnet
  filtering; remote private addresses may be reachable through routed LANs/VPNs.

## API

```rust
use net::{Client, Host, HostEvent};

#[derive(serde::Serialize, serde::Deserialize)]
enum Up { Hello(String), Input(u32, [f32; 2]) }
#[derive(serde::Serialize, serde::Deserialize)]
enum Down { Welcome(u64), Snapshot(Vec<u8>) }

// Host: `latest` marks messages where only the newest matters.
let mut host = Host::<Up, Down>::start(b"my-game/1", false, |m| matches!(m, Down::Snapshot(_)))?;
println!("join with {}", host.ticket);
while let Some(ev) = host.poll() {
    match ev {
        HostEvent::Joined { client, tx } => { tx.send(Down::Welcome(client)).ok(); }
        HostEvent::Message { client, msg } => { /* apply input */ }
        HostEvent::Left { client } => { /* free the slot */ }
    }
}

// Client
let mut c = Client::<Up, Down>::join(&ticket, b"my-game/1", false)?;
c.send(Up::Hello("p1".into()));
while let Ok(msg) = c.poll() { /* apply snapshot */ }
```

`HostEvent::Joined` hands the game a cloneable `HostSender<Out>` for that
client. Its mailbox holds at most 64 messages. Consecutive `latest`
messages (snapshots) replace the pending tail at enqueue time, so a slow
client cannot build an unbounded snapshot backlog. Other messages form
ordered barriers and are never silently replaced. If those barriers fill
the mailbox, `send` fails and the connection closes, including when the
writer is blocked by network flow control. This bound does not change
the client input channel or the host event channel.

Each writer reuses its serialization buffer. Dropping the final host
sender drains its pending messages and finishes the stream normally.

Dropping a `Client` closes the connection at once, so the host sees it
leave without waiting for a timeout.

## Worked example: Growing Guns

Growing Guns plays host-authoritative, like its Godot version did over
godot_iroh. See `games/growing-guns/app/src/net.rs` (the protocol) and
`session.rs` (local, host and client play around the same `World`).

- Clients send a hello and their inputs with sequence numbers.
- The host maps remote players onto bot seats, runs the only simulation,
  and streams snapshots (marked `latest`) and arena changes (reliable,
  never collapsed, so breakage reaches every client).
- Each published snapshot is built once and shared across remote peers
  with `Arc`; decoded snapshots move into the client's world. The postcard
  wire format is unchanged. Each peer still serializes its own packet
  because its acknowledged input sequence differs.
- A client buffers remote snapshots and predicts its own movement and look;
  acknowledged input sequences reconcile that prediction with the host.
  These paths have local tests; realistic latency, jitter and two-machine
  presentation still need validation.

```sh
cargo run -p growing-guns-app -- play --host            # prints a ticket
cargo run -p growing-guns-app -- play --join TICKET
cargo run -p growing-guns-app -- nettest                # host + client over loopback
```

Online play is tested over loopback (`nettest` and the crate's own test),
not yet between two machines.

## Why determinism helps

Because the simulation is deterministic and mod hooks run inline in the
host's tick, a match can be replayed from its inputs. Lockstep or rollback
networking would build on that, but neither exists yet: today's model is
host-authoritative snapshots.
