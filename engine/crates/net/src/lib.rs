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
use iroh::endpoint::{presets, Connection, PortmapperConfig, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, RelayMode};
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
mod pending;
pub use pending::HostSender;

pub use tokio::sync::mpsc::error::TryRecvError;

pub trait Message: Serialize + DeserializeOwned + Send + Sync + 'static {}
impl<T: Serialize + DeserializeOwned + Send + Sync + 'static> Message for T {}

async fn write<T: Serialize>(s: &mut SendStream, msg: &T, bytes: &mut Vec<u8>) -> Result<(), String> {
    bytes.clear();
    bytes.extend_from_slice(&[0u8; 4]);
    *bytes = postcard::to_extend(msg, std::mem::take(bytes)).map_err(|e| e.to_string())?;
    let payload_len = bytes.len() - 4;
    if payload_len > 1 << 24 { return Err("message too large".into()); }
    bytes[..4].copy_from_slice(&(payload_len as u32).to_le_bytes());
    s.write_all(bytes).await.map_err(|e| e.to_string())
}

async fn read<T: DeserializeOwned>(r: &mut RecvStream) -> Result<T, String> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await.map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes(len) as usize;
    if len > 1 << 24 { return Err("message too large".into()); }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
    postcard::from_bytes(&buf).map_err(|e| e.to_string())
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())
}

/// `lan`: no relays or address publishing; direct addresses only (fast,
/// local network or tests). Otherwise iroh's default relays and lookup.
/// Binds IPv4 only (`0.0.0.0:0`) and disables portmapping so macOS `utun0`
/// Continuity (`200::/7`) and router UPnP are never touched.
async fn endpoint(lan: bool, alpns: Vec<Vec<u8>>) -> Result<Endpoint, String> {
    let b = if lan { Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled) } else { Endpoint::builder(presets::N0) };
    let b = b
        .clear_ip_transports()
        .bind_addr("0.0.0.0:0")
        .map_err(|e| e.to_string())?
        .portmapper_config(PortmapperConfig::Disabled);
    b.alpns(alpns).bind().await.map_err(|e| e.to_string())
}

pub fn ticket(addr: &EndpointAddr) -> String {
    postcard::to_allocvec(addr).unwrap_or_default().iter().map(|b| format!("{b:02x}")).collect()
}

pub fn parse_ticket(t: &str) -> Result<EndpointAddr, String> {
    let t = t.trim();
    if !t.is_ascii() || t.len() % 2 != 0 || t.is_empty() { return Err("bad ticket".into()); }
    let bytes: Result<Vec<u8>, _> = (0..t.len()).step_by(2).map(|i| u8::from_str_radix(&t[i..i + 2], 16)).collect();
    let addr: EndpointAddr = postcard::from_bytes(&bytes.map_err(|_| "bad ticket")?).map_err(|e| format!("bad ticket: {e}"))?;
    // A peer can be on a routed LAN/VPN or use a relay. Its addresses must
    // never be filtered using this machine's interfaces or subnet.
    Ok(addr)
}

/// What the host's game loop sees.
pub enum HostEvent<In, Out> {
    /// A client connected; send to it through `tx`.
    Joined { client: u64, tx: HostSender<Out> },
    Message { client: u64, msg: In },
    Left { client: u64 },
}

pub struct Host<In, Out> {
    pub ticket: String,
    pub events: UnboundedReceiver<HostEvent<In, Out>>,
    _rt: tokio::runtime::Runtime,
}

impl<In: Message, Out: Message> Host<In, Out> {
    /// Listen for clients speaking `alpn`.
    pub fn start(alpn: &[u8], lan: bool, latest: fn(&Out) -> bool) -> Result<Self, String> {
        let rt = runtime()?;
        let (etx, events) = unbounded_channel();
        let ep = rt.block_on(endpoint(lan, vec![alpn.to_vec()]))?;
        let addr = rt.block_on(async {
            if !lan { let _ = tokio::time::timeout(std::time::Duration::from_secs(5), ep.online()).await; }
            ep.addr()
        });
        let ticket = ticket(&addr);
        rt.spawn(async move {
            let mut next = 1u64;
            while let Some(incoming) = ep.accept().await {
                let client = next;
                next += 1;
                let etx = etx.clone();
                tokio::spawn(async move {
                    let Ok(accepting) = incoming.accept() else { return };
                    let Ok(conn) = accepting.await else { return };
                    let _ = serve::<In, Out>(conn, client, etx.clone(), latest).await;
                    let _ = etx.send(HostEvent::Left { client });
                });
            }
        });
        Ok(Host { ticket, events, _rt: rt })
    }

    pub fn poll(&mut self) -> Option<HostEvent<In, Out>> { self.events.try_recv().ok() }
}

async fn serve<In: Message, Out: Message>(conn: Connection, client: u64, etx: UnboundedSender<HostEvent<In, Out>>, latest: fn(&Out) -> bool) -> Result<(), String> {
    let (mut send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
    let (tx, mut rx) = pending::channel::<Out>(latest);
    etx.send(HostEvent::Joined { client, tx }).map_err(|_| "host gone")?;
    let writer_conn = conn.clone();
    let writer = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let abort = rx.abort_signal();
        // A saturated peer may be flow-controlled inside write_all. Observe
        // overflow independently so it also cancels that in-flight write.
        let failed = tokio::select! {
            _ = abort.wait() => true,
            result = async {
                while let Some(msg) = rx.recv().await {
                    if write(&mut send, &msg, &mut bytes).await.is_err() { return true; }
                }
                rx.aborted()
            } => result,
        };
        if failed { writer_conn.close(0u32.into(), b"host output overflow or failure"); }
        else { let _ = send.finish(); }
    });
    let result = loop {
        match read::<In>(&mut recv).await {
            Ok(msg) => if etx.send(HostEvent::Message { client, msg }).is_err() { break Ok(()) },
            Err(e) => break Err(e),
        }
    };
    writer.abort();
    result
}

pub struct Client<In, Out> {
    pub tx: UnboundedSender<In>,
    pub rx: UnboundedReceiver<Out>,
    conn: Connection,
    ep: Option<Endpoint>,
    rt: Option<tokio::runtime::Runtime>,
}

impl<In, Out> Drop for Client<In, Out> {
    /// Close at once, so the host sees the client leave without a timeout.
    fn drop(&mut self) {
        self.conn.close(0u32.into(), b"bye");
        if let (Some(rt), Some(ep)) = (self.rt.take(), self.ep.take()) {
            rt.block_on(async { let _ = tokio::time::timeout(std::time::Duration::from_secs(2), ep.close()).await; });
            rt.shutdown_background();
        }
    }
}

impl<In: Message, Out: Message> Client<In, Out> {
    /// Connect to a ticket; blocks until the stream is open (20 s limit).
    pub fn join(ticket: &str, alpn: &[u8], lan: bool) -> Result<Self, String> {
        let addr = parse_ticket(ticket)?;
        let rt = runtime()?;
        let (tx, mut out) = unbounded_channel::<In>();
        let (itx, rx) = unbounded_channel::<Out>();
        let alpn = alpn.to_vec();
        let (mut send, mut recv, ep, conn) = rt.block_on(async {
            let ep = endpoint(lan, vec![]).await?;
            let conn = tokio::time::timeout(std::time::Duration::from_secs(20), ep.connect(addr, &alpn)).await
                .map_err(|_| "timed out connecting".to_string())?.map_err(|e| connect_error(&e.to_string()))?;
            let (send, recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
            Ok::<_, String>((send, recv, ep, conn))
        })?;
        rt.spawn(async move {
            // The first write opens the stream for the host's accept_bi.
            let mut bytes = Vec::new();
            while let Some(msg) = out.recv().await { if write(&mut send, &msg, &mut bytes).await.is_err() { break; } }
        });
        rt.spawn(async move {
            while let Ok(msg) = read::<Out>(&mut recv).await { if itx.send(msg).is_err() { break; } }
        });
        Ok(Client { tx, rx, conn, ep: Some(ep), rt: Some(rt) })
    }

    pub fn send(&self, msg: In) -> bool { self.tx.send(msg).is_ok() }
    pub fn poll(&mut self) -> Result<Out, TryRecvError> { self.rx.try_recv() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn tickets_preserve_routed_peer_addresses() {
        let id = iroh::PublicKey::from_bytes(&[0; 32]).unwrap();
        let mut addr = EndpointAddr::new(id)
            .with_relay_url("https://relay.example.com".parse().unwrap());
        // .0/.255 can be hosts outside a /24; private peers can be on a VPN
        // or a routed subnet, and IPv6 tickets must remain lossless too.
        for ip in ["10.40.1.0:1234", "172.20.1.255:1234", "192.168.250.7:1234", "[2001:db8::1]:1234"] {
            addr = addr.with_ip_addr(ip.parse().unwrap());
        }
        assert_eq!(parse_ticket(&ticket(&addr)).unwrap(), addr);
    }

    #[test]
    fn malformed_tickets_return_errors_instead_of_panicking() {
        for t in ["", "f", "zz", "0€", "€0", "😀", "00"] {
            assert!(parse_ticket(t).is_err(), "accepted {t:?}");
        }
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    enum Up { Hello(String), Ping(u32) }
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    enum Down { Welcome(u64), Pong(u32), State(Vec<u32>) }

    fn wait<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            if let Some(v) = f() { return v; }
            assert!(std::time::Instant::now() < end, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn host_and_client_talk_over_loopback() {
        let mut host = Host::<Up, Down>::start(b"warp/test/0", true, |m| matches!(m, Down::State(_))).unwrap();
        let mut client = Client::<Up, Down>::join(&host.ticket, b"warp/test/0", true).unwrap();
        assert!(client.send(Up::Hello("p1".into())));
        let (id, tx) = wait(|| match host.poll() { Some(HostEvent::Joined { client, tx }) => Some((client, tx)), _ => None });
        let hello = wait(|| match host.poll() { Some(HostEvent::Message { client, msg }) => Some((client, msg)), _ => None });
        assert_eq!(hello, (id, Up::Hello("p1".into())));
        tx.send(Down::Welcome(id)).unwrap();
        assert_eq!(wait(|| client.poll().ok()), Down::Welcome(id));
        // Round trips keep their order.
        for k in 0..20 { client.send(Up::Ping(k)); }
        for k in 0..20 {
            let msg = wait(|| match host.poll() { Some(HostEvent::Message { msg, .. }) => Some(msg), _ => None });
            assert_eq!(msg, Up::Ping(k));
            tx.send(Down::Pong(k)).unwrap();
        }
        for k in 0..20 { assert_eq!(wait(|| client.poll().ok()), Down::Pong(k)); }
        drop(client);
        wait(|| match host.poll() { Some(HostEvent::Left { client }) => Some(client), _ => None });
        assert!(parse_ticket("zz").is_err());
    }
}

/// A TLS ALPN mismatch means host and client run different game versions;
/// say that instead of "error 120: peer doesn't support any known protocol".
fn connect_error(e: &str) -> String {
    if e.contains("error 120") || e.contains("known protocol") || e.contains("no_application_protocol") {
        "the host runs a different version of this game; both players need the same (latest) build".into()
    } else { e.to_string() }
}

#[cfg(test)]
mod connect_error_tests {
    #[test]
    fn version_mismatch_reads_as_one() {
        let e = super::connect_error("aborted by peer: the cryptographic handshake failed: error 120: peer doesn't support any known protocol");
        assert!(e.contains("different version"));
        assert_eq!(super::connect_error("timed out"), "timed out");
    }
}
