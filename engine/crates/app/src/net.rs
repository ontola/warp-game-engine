//! Couches playing one game over the network, for games that run the whole
//! simulation on every couch (rollback netcode, kern::rollback). The page
//! hands such a game a [`Wire`] ([`crate::Game::go_online`]); what travels
//! on it is the game's own business.

/// Messages between this couch and the others in its online room. The host
/// couch relays: what a follower sends reaches the host and every other
/// follower.
pub trait Wire {
    /// This couch hosts the room.
    fn hosting(&self) -> bool;
    /// Send to every other couch.
    fn send(&mut self, data: Vec<u8>);
    /// What arrived, by the couch that sent it (0: the host).
    fn poll(&mut self) -> Vec<(u64, Vec<u8>)>;
    /// The other couches connected now (host side; a follower sees none).
    fn couches(&self) -> Vec<u64>;
    /// The host is gone (follower side).
    fn closed(&self) -> bool { false }
}
