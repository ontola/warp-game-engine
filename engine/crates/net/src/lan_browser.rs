//! The browser has no UDP: no game is ever found on the local network.

pub const PORT: u16 = 47_733;

#[derive(Clone, Debug, PartialEq)]
pub struct LanHost { pub name: String, pub ticket: String }

pub struct Announcer;

impl Announcer {
    pub fn start(_game: &str, _name: &str, _ticket: &str) -> Result<Self, String> { Err("no local network in the browser".into()) }
}

pub struct Listener;

impl Listener {
    pub fn start(_game: &str) -> Result<Self, String> { Err("no local network in the browser".into()) }
    pub fn hosts(&self) -> Vec<LanHost> { Vec::new() }
}
