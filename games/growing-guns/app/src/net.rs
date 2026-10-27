//! Online play over iroh: the host runs the only simulation, clients send
//! input and render host snapshots, as the Godot version does over
//! godot_iroh. One reliable QUIC stream per client carries length-prefixed
//! postcard messages. iroh runs on its own tokio runtime; the game loop
//! talks to it through unbounded channels and never blocks.
use growing_guns::{Arena, Input, Snapshot};
use iroh::endpoint::{presets, Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, RelayMode};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub const ALPN: &[u8] = b"warp/growing-guns/0";

#[derive(Debug, Serialize, Deserialize)]
pub enum ToHost {
    Hello { players: u8 },
    /// Inputs for the client's local players, in slot order.
    Input { seq: u32, inputs: Vec<Input> },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ToClient {
    Welcome { slots: Vec<usize>, players: usize, arena: Arena },
    Arena(Arena),
    /// `ack`: the last input sequence the host has applied for this client.
    Snap { ack: u32, snap: Snapshot },
    Full,
}

async fn write<T: Serialize>(s: &mut SendStream, msg: &T) -> Result<(), String> {
    let bytes = postcard::to_allocvec(msg).map_err(|e| e.to_string())?;
    s.write_all(&(bytes.len() as u32).to_le_bytes()).await.map_err(|e| e.to_string())?;
    s.write_all(&bytes).await.map_err(|e| e.to_string())
}

async fn read<T: for<'de> Deserialize<'de>>(r: &mut RecvStream) -> Result<T, String> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await.map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes(len) as usize;
    if len > 1 << 22 { return Err("message too large".into()); }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
    postcard::from_bytes(&buf).map_err(|e| e.to_string())
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())
}

/// `lan`: no relay and no address publishing, direct addresses only.
async fn endpoint(lan: bool, alpns: Vec<Vec<u8>>) -> Result<Endpoint, String> {
    let b = if lan { Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled) } else { Endpoint::builder(presets::N0) };
    b.alpns(alpns).bind().await.map_err(|e| e.to_string())
}

/// A shareable ticket: the endpoint address as hex. Over the internet the
/// endpoint id inside is enough (relay and address lookup find the rest).
pub fn ticket(addr: &EndpointAddr) -> String {
    postcard::to_allocvec(addr).unwrap_or_default().iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_ticket(t: &str) -> Result<EndpointAddr, String> {
    let t = t.trim();
    if t.len() % 2 != 0 { return Err("bad ticket".into()); }
    let bytes: Result<Vec<u8>, _> = (0..t.len()).step_by(2).map(|i| u8::from_str_radix(&t[i..i + 2], 16)).collect();
    postcard::from_bytes(&bytes.map_err(|_| "bad ticket")?).map_err(|e| format!("bad ticket: {e}"))
}

pub enum HostEvent {
    Joined { client: u64, players: u8, tx: UnboundedSender<ToClient> },
    Input { client: u64, seq: u32, inputs: Vec<Input> },
    Left { client: u64 },
}

pub struct Host {
    pub ticket: String,
    pub events: UnboundedReceiver<HostEvent>,
    _rt: tokio::runtime::Runtime,
}

impl Host {
    pub fn start(lan: bool) -> Result<Host, String> {
        let rt = runtime()?;
        let (etx, events) = unbounded_channel();
        let ep = rt.block_on(endpoint(lan, vec![ALPN.to_vec()]))?;
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
                    if let Err(e) = serve(conn, client, etx.clone()).await { eprintln!("client {client}: {e}"); }
                    let _ = etx.send(HostEvent::Left { client });
                });
            }
        });
        Ok(Host { ticket, events, _rt: rt })
    }
}

async fn serve(conn: Connection, client: u64, etx: UnboundedSender<HostEvent>) -> Result<(), String> {
    let (mut send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
    let ToHost::Hello { players } = read(&mut recv).await? else { return Err("expected hello".into()) };
    let (tx, mut rx) = unbounded_channel::<ToClient>();
    etx.send(HostEvent::Joined { client, players, tx }).map_err(|_| "host gone")?;
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            // Coalesce a backlog of snapshots: only the newest matters.
            let mut msg = msg;
            while let Ok(next) = rx.try_recv() {
                if matches!(msg, ToClient::Snap { .. }) && matches!(next, ToClient::Snap { .. }) { msg = next; continue; }
                if write(&mut send, &msg).await.is_err() { return; }
                msg = next;
            }
            if write(&mut send, &msg).await.is_err() { return; }
        }
    });
    let result = loop {
        match read::<ToHost>(&mut recv).await {
            Ok(ToHost::Input { seq, inputs }) => { if etx.send(HostEvent::Input { client, seq, inputs }).is_err() { break Ok(()); } }
            Ok(ToHost::Hello { .. }) => {}
            Err(e) => break Err(e),
        }
    };
    writer.abort();
    result
}

pub struct Client {
    pub tx: UnboundedSender<ToHost>,
    pub rx: UnboundedReceiver<ToClient>,
    _rt: tokio::runtime::Runtime,
}

impl Client {
    /// Connect and say hello; blocks until the connection is up.
    pub fn join(ticket: &str, players: u8, lan: bool) -> Result<Client, String> {
        let addr = parse_ticket(ticket)?;
        let rt = runtime()?;
        let (tx, mut out) = unbounded_channel::<ToHost>();
        let (itx, rx) = unbounded_channel::<ToClient>();
        let (mut send, mut recv) = rt.block_on(async {
            let ep = endpoint(lan, vec![]).await?;
            let conn = tokio::time::timeout(std::time::Duration::from_secs(20), ep.connect(addr, ALPN)).await
                .map_err(|_| "timed out connecting".to_string())?.map_err(|e| e.to_string())?;
            let (mut send, recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
            write(&mut send, &ToHost::Hello { players }).await?;
            // Keep the endpoint alive as long as the runtime.
            std::mem::forget(ep);
            Ok::<_, String>((send, recv))
        })?;
        rt.spawn(async move {
            while let Some(msg) = out.recv().await { if write(&mut send, &msg).await.is_err() { break; } }
        });
        rt.spawn(async move {
            while let Ok(msg) = read::<ToClient>(&mut recv).await { if itx.send(msg).is_err() { break; } }
        });
        Ok(Client { tx, rx, _rt: rt })
    }
}
