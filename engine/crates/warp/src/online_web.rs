//! Online couches in a browser: WebRTC data channels, signalled through the
//! cloud's `/v1/signal/ROOM` socket (gamenight-internal signal.rs). One
//! ordered, reliable channel per follower carries the same postcard messages
//! as the native transport, in chunks of at most 16 KB (browsers cap a data
//! channel message at about 256 KB, some lower).
use crate::online::{Down, FollowLink, Got, HostLink, LinkEvent, Up};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use web_sys::{MessageEvent, RtcConfiguration, RtcDataChannel, RtcDataChannelEvent, RtcDataChannelInit, RtcDataChannelType, RtcIceCandidateInit, RtcIceServer, RtcPeerConnection, RtcPeerConnectionIceEvent, RtcSdpType, RtcSessionDescriptionInit, WebSocket};

const CHUNK: usize = 16 * 1024;

fn log(s: &str) { app::web::log(s); }

thread_local! { static USED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

/// The page's online couch, from its address: `?follow=ROOM` follows that
/// room, `?host=1` (or a code of 8 or more characters) hosts and logs the
/// follow link. Only the page's own game (`?game=`, or the first one loaded)
/// and only once a page, so the next game in a queue plays locally.
pub fn from_page(name: &str, cfg: &mut app::Config, game: &mut crate::LuaGame) -> Result<(), String> {
    if crate::web::param("game").is_some_and(|g| g != name) || USED.with(|u| u.replace(true)) { return Ok(()); }
    if let Some(room) = crate::web::param("follow") {
        game.follow_with(Box::new(Client::join(&room)?), cfg.size);
        cfg.tick_hz = game.cfg.tick_rate / crate::online::send_every(game.cfg.tick_rate) as f32;
        log(&format!("online: following room {room}"));
    } else if let Some(room) = crate::web::param("host") {
        let room = if room.len() >= 8 { room } else { room_code() };
        game.host_with(Box::new(Host::start(&room)?));
        let path = web_sys::window().and_then(|w| w.location().pathname().ok()).unwrap_or_default();
        app::web::log_pinned(&format!("online: other couches join with {}{path}?game={name}&follow={room}", app::web::origin()));
    } else {
        USED.with(|u| u.set(false));
    }
    Ok(())
}

/// The signaling socket's address for a room: `?signal=URL` or this page's origin.
pub fn signal_url(room: &str, role: &str) -> String {
    let base = app::web::query_param("signal").unwrap_or_else(|| app::web::origin().replacen("http", "ws", 1));
    format!("{}/v1/signal/{room}?role={role}", base.trim_end_matches('/'))
}

/// STUN (and TURN) servers: `?ice=stun:host:port,turn:...` or a public STUN server.
fn rtc() -> Result<RtcPeerConnection, String> {
    let urls = app::web::query_param("ice").unwrap_or_else(|| "stun:stun.cloudflare.com:3478".into());
    let servers = js_sys::Array::new();
    for u in urls.split(',').filter(|u| !u.is_empty()) {
        let s = RtcIceServer::new();
        s.set_urls(&JsValue::from_str(u));
        servers.push(&s);
    }
    let cfg = RtcConfiguration::new();
    cfg.set_ice_servers(&servers);
    RtcPeerConnection::new_with_configuration(&cfg).map_err(|e| format!("WebRTC: {e:?}"))
}

/// A long random room code (the code is the key to the room).
pub fn room_code() -> String {
    let mut b = [0u8; 9];
    getrandom(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn getrandom(buf: &mut [u8]) {
    if let Some(c) = web_sys::window().and_then(|w| w.crypto().ok()) { let _ = c.get_random_values_with_u8_array(buf); }
}

/// Split a message into data channel chunks: a leading 1 means more follow.
fn send_chunks(ch: &RtcDataChannel, bytes: &[u8]) -> bool {
    let mut parts = bytes.chunks(CHUNK).peekable();
    if bytes.is_empty() { return ch.send_with_u8_array(&[0]).is_ok(); }
    while let Some(p) = parts.next() {
        let mut v = Vec::with_capacity(p.len() + 1);
        v.push(u8::from(parts.peek().is_some()));
        v.extend_from_slice(p);
        if ch.send_with_u8_array(&v).is_err() { return false; }
    }
    true
}

/// Gather chunks into whole messages.
#[derive(Default)]
struct Joiner(Vec<u8>);

impl Joiner {
    fn push(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let (&more, rest) = data.split_first()?;
        self.0.extend_from_slice(rest);
        (more == 0).then(|| std::mem::take(&mut self.0))
    }
}

fn bytes_of(e: &MessageEvent) -> Vec<u8> {
    e.data().dyn_into::<js_sys::ArrayBuffer>().map(|b| js_sys::Uint8Array::new(&b).to_vec()).unwrap_or_default()
}

fn send_json(ws: &WebSocket, v: serde_json::Value) { let _ = ws.send_with_str(&v.to_string()); }

/// Hand our ICE candidates to the other side through the signaling socket.
fn trickle(pc: &RtcPeerConnection, ws: &WebSocket, to: u64) {
    let ws = ws.clone();
    let f = Closure::<dyn FnMut(RtcPeerConnectionIceEvent)>::new(move |e: RtcPeerConnectionIceEvent| {
        if let Some(c) = e.candidate() {
            send_json(&ws, serde_json::json!({ "to": to, "data": { "candidate": c.candidate(), "mid": c.sdp_mid(), "line": c.sdp_m_line_index() } }));
        }
    });
    pc.set_onicecandidate(Some(f.as_ref().unchecked_ref()));
    f.forget();
}

fn add_candidate(pc: &RtcPeerConnection, d: &serde_json::Value) {
    let Some(c) = d["candidate"].as_str() else { return };
    let init = RtcIceCandidateInit::new(c);
    init.set_sdp_mid(d["mid"].as_str());
    init.set_sdp_m_line_index(d["line"].as_u64().map(|l| l as u16));
    let _ = pc.add_ice_candidate_with_opt_rtc_ice_candidate_init(Some(&init));
}

async fn wait(p: js_sys::Promise) -> Result<JsValue, String> { wasm_bindgen_futures::JsFuture::from(p).await.map_err(|e| format!("{e:?}")) }

fn sdp_of(v: &JsValue) -> String { js_sys::Reflect::get(v, &"sdp".into()).ok().and_then(|s| s.as_string()).unwrap_or_default() }

fn on_message(ws: &WebSocket, f: impl FnMut(serde_json::Value) + 'static) {
    let mut f = f;
    let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        if let Some(t) = e.data().as_string() && let Ok(v) = serde_json::from_str(&t) { f(v); }
    });
    ws.set_onmessage(Some(cb.as_ref().unchecked_ref()));
    cb.forget();
}

// ---- host ----

#[derive(Default)]
struct HostState {
    events: VecDeque<LinkEvent>,
    peers: BTreeMap<u64, (RtcPeerConnection, Option<RtcDataChannel>)>,
    closed: Option<String>,
}

pub struct Host { room: String, state: Rc<RefCell<HostState>>, _ws: WebSocket }

impl Host {
    /// Open room `room` for other couches.
    pub fn start(room: &str) -> Result<Self, String> {
        let ws = WebSocket::new(&signal_url(room, "host")).map_err(|e| format!("signal: {e:?}"))?;
        let state = Rc::new(RefCell::new(HostState::default()));
        let (st, wsc) = (state.clone(), ws.clone());
        on_message(&ws, move |v| {
            if let Some(id) = v["left"].as_u64() {
                if let Some((pc, _)) = st.borrow_mut().peers.remove(&id) { pc.close(); st.borrow_mut().events.push_back(LinkEvent::Left(id)); }
                return;
            }
            let (Some(from), d) = (v["from"].as_u64(), &v["data"]) else { return };
            if d["type"] == "offer" {
                let Ok(pc) = rtc() else { return };
                trickle(&pc, &wsc, from);
                // The follower made the channel; take it when it arrives.
                let st2 = st.clone();
                let on_dc = Closure::<dyn FnMut(RtcDataChannelEvent)>::new(move |e: RtcDataChannelEvent| {
                    let ch = e.channel();
                    ch.set_binary_type(RtcDataChannelType::Arraybuffer);
                    let (a, b, c) = (st2.clone(), st2.clone(), st2.clone());
                    let open = Closure::<dyn FnMut()>::new(move || a.borrow_mut().events.push_back(LinkEvent::Joined(from)));
                    let mut joiner = Joiner::default();
                    let msg = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
                        if let Some(m) = joiner.push(&bytes_of(&e)) && let Ok(up) = postcard::from_bytes::<Up>(&m) { b.borrow_mut().events.push_back(LinkEvent::Message(from, up)); }
                    });
                    let close = Closure::<dyn FnMut()>::new(move || { if c.borrow_mut().peers.remove(&from).is_some() { c.borrow_mut().events.push_back(LinkEvent::Left(from)); } });
                    ch.set_onopen(Some(open.as_ref().unchecked_ref()));
                    ch.set_onmessage(Some(msg.as_ref().unchecked_ref()));
                    ch.set_onclose(Some(close.as_ref().unchecked_ref()));
                    open.forget(); msg.forget(); close.forget();
                    if let Some(p) = st2.borrow_mut().peers.get_mut(&from) { p.1 = Some(ch); }
                });
                pc.set_ondatachannel(Some(on_dc.as_ref().unchecked_ref()));
                on_dc.forget();
                st.borrow_mut().peers.insert(from, (pc.clone(), None));
                let (sdp, ws) = (d["sdp"].as_str().unwrap_or_default().to_string(), wsc.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    let offer = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
                    offer.set_sdp(&sdp);
                    let r: Result<(), String> = async {
                        wait(pc.set_remote_description(&offer)).await?;
                        let answer = wait(pc.create_answer()).await?;
                        let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
                        init.set_sdp(&sdp_of(&answer));
                        wait(pc.set_local_description(&init)).await?;
                        send_json(&ws, serde_json::json!({ "to": from, "data": { "type": "answer", "sdp": sdp_of(&answer) } }));
                        Ok(())
                    }.await;
                    if let Err(e) = r { log(&format!("online: answering couch {from}: {e}")); }
                });
            } else if let Some((pc, _)) = st.borrow().peers.get(&from) {
                add_candidate(pc, d);
            }
        });
        let st = state.clone();
        let closed = Closure::<dyn FnMut()>::new(move || { st.borrow_mut().closed = Some("signaling closed".into()); });
        ws.set_onclose(Some(closed.as_ref().unchecked_ref()));
        closed.forget();
        Ok(Host { room: room.into(), state, _ws: ws })
    }
}

impl HostLink for Host {
    fn poll(&mut self) -> Option<LinkEvent> { self.state.borrow_mut().events.pop_front() }
    fn send(&mut self, client: u64, msg: Down) -> bool {
        let st = self.state.borrow();
        let Some((_, Some(ch))) = st.peers.get(&client) else { return false };
        match postcard::to_allocvec(&msg) { Ok(b) => send_chunks(ch, &b), Err(_) => false }
    }
    fn ticket(&self) -> String { self.room.clone() }
}

// ---- follower ----

#[derive(Default)]
struct FollowState { inbox: VecDeque<Down>, outbox: Vec<Vec<u8>>, channel: Option<RtcDataChannel>, closed: bool }

pub struct Client { state: Rc<RefCell<FollowState>>, _ws: WebSocket, _pc: RtcPeerConnection }

impl Client {
    /// Follow the host of room `room`.
    pub fn join(room: &str) -> Result<Self, String> {
        let ws = WebSocket::new(&signal_url(room, "follow")).map_err(|e| format!("signal: {e:?}"))?;
        let pc = rtc()?;
        let state = Rc::new(RefCell::new(FollowState::default()));
        let init = RtcDataChannelInit::new();
        init.set_ordered(true);
        let ch = pc.create_data_channel_with_data_channel_dict("couch", &init);
        ch.set_binary_type(RtcDataChannelType::Arraybuffer);
        let (a, b, c) = (state.clone(), state.clone(), state.clone());
        let chc = ch.clone();
        let open = Closure::<dyn FnMut()>::new(move || {
            let mut s = a.borrow_mut();
            for m in std::mem::take(&mut s.outbox) { send_chunks(&chc, &m); }
            s.channel = Some(chc.clone());
        });
        let mut joiner = Joiner::default();
        let msg = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            if let Some(m) = joiner.push(&bytes_of(&e)) && let Ok(d) = postcard::from_bytes::<Down>(&m) { b.borrow_mut().inbox.push_back(d); }
        });
        let close = Closure::<dyn FnMut()>::new(move || c.borrow_mut().closed = true);
        ch.set_onopen(Some(open.as_ref().unchecked_ref()));
        ch.set_onmessage(Some(msg.as_ref().unchecked_ref()));
        ch.set_onclose(Some(close.as_ref().unchecked_ref()));
        open.forget(); msg.forget(); close.forget();
        let (pcc, wsc) = (pc.clone(), ws.clone());
        on_message(&ws, move |v| {
            if v["you"].is_u64() {
                trickle(&pcc, &wsc, 0);
                let (pc, ws) = (pcc.clone(), wsc.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    let r: Result<(), String> = async {
                        let offer = wait(pc.create_offer()).await?;
                        let init = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
                        init.set_sdp(&sdp_of(&offer));
                        wait(pc.set_local_description(&init)).await?;
                        send_json(&ws, serde_json::json!({ "to": 0, "data": { "type": "offer", "sdp": sdp_of(&offer) } }));
                        Ok(())
                    }.await;
                    if let Err(e) = r { log(&format!("online: offer: {e}")); }
                });
                return;
            }
            let d = &v["data"];
            if d["type"] == "answer" {
                let (pc, sdp) = (pcc.clone(), d["sdp"].as_str().unwrap_or_default().to_string());
                wasm_bindgen_futures::spawn_local(async move {
                    let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
                    init.set_sdp(&sdp);
                    if let Err(e) = wait(pc.set_remote_description(&init)).await { log(&format!("online: answer: {e}")); }
                });
            } else {
                add_candidate(&pcc, d);
            }
        });
        let st = state.clone();
        let closed = Closure::<dyn FnMut()>::new(move || { if st.borrow().channel.is_none() { st.borrow_mut().closed = true; } });
        ws.set_onclose(Some(closed.as_ref().unchecked_ref()));
        closed.forget();
        Ok(Client { state, _ws: ws, _pc: pc })
    }
}

impl FollowLink for Client {
    fn send(&mut self, msg: Up) {
        let Ok(b) = postcard::to_allocvec(&msg) else { return };
        let mut s = self.state.borrow_mut();
        match &s.channel {
            Some(ch) => { send_chunks(ch, &b); }
            // Before the channel opens only the latest hello matters.
            None => if matches!(msg, Up::Hello { .. }) { s.outbox = vec![b]; },
        }
    }
    fn poll(&mut self) -> Got {
        let mut s = self.state.borrow_mut();
        match s.inbox.pop_front() { Some(m) => Got::Msg(m), None if s.closed => Got::Closed, None => Got::Empty }
    }
}
