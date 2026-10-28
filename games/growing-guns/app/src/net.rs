//! Growing Guns' online protocol on the engine's `net` crate: clients send
//! a hello and their inputs, the host streams snapshots (only the newest
//! matters) and new arenas. The host runs the only simulation, as the
//! Godot version does over godot_iroh.
use growing_guns::{Arena, Breakage, Input, Snapshot};
use serde::{Deserialize, Serialize};

pub const ALPN: &[u8] = b"warp/growing-guns/1";

#[derive(Debug, Serialize, Deserialize)]
pub enum ToHost {
    Hello { players: u8 },
    /// Inputs for the client's local players, in slot order.
    Input { seq: u32, inputs: Vec<Input> },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ToClient {
    /// `coop`: wave survival (the view makes room for the waves).
    Welcome { slots: Vec<usize>, players: usize, arena: Arena, #[serde(default)] coop: bool },
    Arena(Arena),
    /// What a tick did to the arena; reliable and in order, never collapsed.
    Broken(Breakage),
    /// `ack`: the last input sequence the host has applied for this client.
    Snap { ack: u32, snap: Snapshot },
    Full,
}

pub type Host = net::Host<ToHost, ToClient>;
pub type Client = net::Client<ToHost, ToClient>;
pub type HostEvent = net::HostEvent<ToHost, ToClient>;

pub fn host(lan: bool) -> Result<Host, String> { Host::start(ALPN, lan, |m| matches!(m, ToClient::Snap { .. })) }

pub fn join(ticket: &str, players: u8, lan: bool) -> Result<Client, String> {
    let c = Client::join(ticket, ALPN, lan)?;
    c.send(ToHost::Hello { players });
    Ok(c)
}
