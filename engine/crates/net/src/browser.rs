//! The browser build: no sockets or threaded runtime, so there is no online
//! play. Same API as the native transport; hosting and joining fail with a
//! plain reason, and tickets are never valid.
use crate::{HostEvent, Message, TryRecvError};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

const UNAVAILABLE: &str = "online play is not available in the browser";

/// Stands in for iroh's endpoint address: no browser ticket parses.
#[derive(Clone, Debug, PartialEq)]
pub struct EndpointAddr;

pub fn ticket(_: &EndpointAddr) -> String { String::new() }

pub fn parse_ticket(_: &str) -> Result<EndpointAddr, String> { Err(UNAVAILABLE.into()) }

pub struct Host<In, Out> {
    pub ticket: String,
    pub events: UnboundedReceiver<HostEvent<In, Out>>,
}

impl<In: Message, Out: Message> Host<In, Out> {
    pub fn start(_alpn: &[u8], _lan: bool, _latest: fn(&Out) -> bool) -> Result<Self, String> { Err(UNAVAILABLE.into()) }
    pub fn poll(&mut self) -> Option<HostEvent<In, Out>> { self.events.try_recv().ok() }
}

pub struct Client<In, Out> {
    pub tx: UnboundedSender<In>,
    pub rx: UnboundedReceiver<Out>,
}

impl<In: Message, Out: Message> Client<In, Out> {
    pub fn join(_ticket: &str, _alpn: &[u8], _lan: bool) -> Result<Self, String> { Err(UNAVAILABLE.into()) }
    pub fn send(&self, msg: In) -> bool { self.tx.send(msg).is_ok() }
    pub fn poll(&mut self) -> Result<Out, TryRecvError> { self.rx.try_recv() }
}
