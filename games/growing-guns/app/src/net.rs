//! Growing Guns' online protocol on the engine's `net` crate: clients send
//! a hello and their inputs, the host streams snapshots (only the newest
//! matters) and new arenas. The host runs the only simulation, as the
//! Godot version does over godot_iroh.
use growing_guns::{Arena, Breakage, Input, Snapshot};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// Version 2 adds the cage countdown to positional postcard snapshots, 3 the
// players' recoil lift, 4 headset poses, 5 per-player match kills. Reject
// older peers during connection setup, before decoding any state.
pub const ALPN: &[u8] = b"warp/growing-guns/5";

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
    Snap { ack: u32, snap: Arc<Snapshot> },
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

#[cfg(test)]
mod tests {
    use super::*;
    // Keep the pre-sharing wire schema as a compatibility reference. Arc is
    // an in-process ownership choice, never a protocol revision.
    #[derive(Serialize, Deserialize)]
    enum OwnedClient {
        Welcome { slots: Vec<usize>, players: usize, arena: Arena, coop: bool },
        Arena(Arena), Broken(Breakage), Snap { ack: u32, snap: Snapshot }, Full,
    }
    #[test]
    fn a_headset_input_survives_the_wire() {
        let aim = growing_guns::TrackedAim { offset: glam::Vec3::new(0.1, 0.2, 0.3), direction: glam::Vec3::NEG_Z };
        let input = Input { head: Some([0.5, -0.25]), tracked: Some(aim), fire: true, ..Default::default() };
        let bytes = postcard::to_allocvec(&ToHost::Input { seq: 7, inputs: vec![input] }).unwrap();
        let ToHost::Input { seq, inputs } = postcard::from_bytes(&bytes).unwrap() else { panic!("wrong packet kind") };
        assert_eq!((seq, inputs), (7, vec![input]));
    }

    #[test]
    fn shared_snapshot_retains_exact_wire_format_and_shares_across_peers() {
        let world = growing_guns::World::new(3, &[true; 4]);
        let snap = Arc::new(world.snapshot());
        let a = ToClient::Snap { ack: 19, snap: snap.clone() };
        let b = ToClient::Snap { ack: 27, snap: snap.clone() };
        assert_eq!(Arc::strong_count(&snap), 3);
        let bytes = postcard::to_allocvec(&a).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&OwnedClient::Snap { ack: 19, snap: (*snap).clone() }).unwrap());
        let decoded: ToClient = postcard::from_bytes(&bytes).unwrap();
        let ToClient::Snap { ack, snap: received } = decoded else { panic!("wrong packet kind") };
        assert_eq!(ack, 19);
        assert_eq!(Arc::strong_count(&received), 1);
        let owned = Arc::try_unwrap(received).expect("client owns decoded snapshot without a deep clone");
        assert_eq!(owned.tick, world.tick);
        drop(b);
        assert_eq!(Arc::strong_count(&snap), 2);
    }
}
