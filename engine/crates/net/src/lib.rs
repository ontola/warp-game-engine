//! Online play over iroh (QUIC with hole punching and relays): a host and
//! clients exchange typed messages on one reliable stream per client
//! (length-prefixed postcard). iroh runs on its own tokio runtime; the
//! game loop polls channels and never blocks. A ticket (hex of the host's
//! endpoint address) is what a player shares to join.
//!
//! The protocol is the game's: `In` goes client to host, `Out` host to
//! client. A host-authoritative game sends inputs up and snapshots down;
//! `latest` marks messages where only the newest matters (snapshots), so a
//! slow client gets the newest instead of a backlog.
#[cfg(not(target_arch = "wasm32"))]
pub mod lan;
#[cfg(target_arch = "wasm32")]
#[path = "lan_browser.rs"]
pub mod lan;
use serde::{de::DeserializeOwned, Serialize};
mod pending;
pub use pending::HostSender;

pub use tokio::sync::mpsc::error::TryRecvError;

pub trait Message: Serialize + DeserializeOwned + Send + Sync + 'static {}
impl<T: Serialize + DeserializeOwned + Send + Sync + 'static> Message for T {}

/// What the host's game loop sees.
pub enum HostEvent<In, Out> {
    /// A client connected; send to it through `tx`.
    Joined { client: u64, tx: HostSender<Out> },
    Message { client: u64, msg: In },
    Left { client: u64 },
}

// iroh needs sockets and a threaded tokio runtime; a browser build gets the
// same API where hosting and joining report that online play is unavailable.
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
pub use browser::*;
