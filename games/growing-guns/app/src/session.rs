//! Local, host or client play around the same `World`. The host maps
//! remote players onto bot slots and streams snapshots; a client renders
//! them, predicting only its own look direction (acknowledged input
//! sequence numbers reconcile it with the host).
use crate::net::{Client, Host, HostEvent, ToClient, ToHost};
use growing_guns::{Event, Input, World};
use std::collections::VecDeque;
use tokio::sync::mpsc::UnboundedSender;

pub struct Remote {
    client: u64,
    slots: Vec<usize>,
    tx: UnboundedSender<ToClient>,
    pending: Vec<Input>,
    seq: u32,
}

pub enum Session {
    Local,
    Host { host: Host, remotes: Vec<Remote> },
    Client { client: Client, ack: u32, seq: u32, sent: VecDeque<(u32, Vec<Input>)>, base_look: Vec<(f32, f32)>, closed: bool },
}

/// Fold one more input into a pending one: look adds up, edges stick.
fn merge(into: &mut Input, i: &Input) {
    into.mv = i.mv;
    into.fire = i.fire;
    into.look = [into.look[0] + i.look[0], into.look[1] + i.look[1]];
    into.jump |= i.jump;
    into.dash |= i.dash;
    into.reload |= i.reload;
    into.pick = i.pick.or(into.pick);
}

impl Session {
    pub fn host(lan: bool) -> Result<Session, String> {
        Ok(Session::Host { host: Host::start(lan)?, remotes: Vec::new() })
    }

    /// Connect, then wait for the host's welcome: returns the session, the
    /// world shell and the player slots this client controls.
    pub fn join(ticket: &str, players: u8, lan: bool) -> Result<(Session, World, Vec<usize>), String> {
        let mut client = Client::join(ticket, players, lan)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            match client.rx.try_recv() {
                Ok(ToClient::Welcome { slots, players, arena }) => {
                    let world = World::remote(arena, players);
                    let base = vec![(0.0, 0.0); slots.len()];
                    return Ok((Session::Client { client, ack: 0, seq: 0, sent: VecDeque::new(), base_look: base, closed: false }, world, slots));
                }
                Ok(ToClient::Full) => return Err("the game is full".into()),
                Ok(_) => {}
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return Err("host closed the connection".into()),
                Err(_) => {
                    if std::time::Instant::now() > deadline { return Err("no welcome from the host".into()); }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
    }

    pub fn ticket(&self) -> Option<&str> {
        match self { Session::Host { host, .. } => Some(&host.ticket), _ => None }
    }

    pub fn is_client(&self) -> bool { matches!(self, Session::Client { .. }) }

    /// Host: handle joins, leaves and remote input.
    pub fn poll(&mut self, world: &mut World) {
        let Session::Host { host, remotes } = self else { return };
        while let Ok(ev) = host.events.try_recv() {
            match ev {
                HostEvent::Joined { client, players, tx } => {
                    let taken: Vec<usize> = remotes.iter().flat_map(|r| r.slots.iter().copied()).collect();
                    let free: Vec<usize> = (1..world.players.len()).filter(|i| world.players[*i].bot && !taken.contains(i)).take(players as usize).collect();
                    if free.len() < players as usize || players == 0 { let _ = tx.send(ToClient::Full); continue; }
                    for &s in &free { world.set_bot(s, false); }
                    let _ = tx.send(ToClient::Welcome { slots: free.clone(), players: world.players.len(), arena: world.arena.clone() });
                    println!("client {client} joined as players {:?}", free.iter().map(|s| s + 1).collect::<Vec<_>>());
                    remotes.push(Remote { client, pending: vec![Input::default(); free.len()], slots: free, tx, seq: 0 });
                }
                HostEvent::Input { client, seq, inputs } => {
                    if let Some(r) = remotes.iter_mut().find(|r| r.client == client) {
                        for (p, i) in r.pending.iter_mut().zip(&inputs) { merge(p, i); }
                        r.seq = r.seq.max(seq);
                    }
                }
                HostEvent::Left { client } => {
                    if let Some(k) = remotes.iter().position(|r| r.client == client) {
                        let r = remotes.remove(k);
                        for s in r.slots { world.set_bot(s, true); }
                        println!("client {client} left");
                    }
                }
            }
        }
    }

    /// Host: put remote inputs into the tick's input list. Held state (move,
    /// fire) persists until the next packet; look and edges are consumed.
    pub fn remote_inputs(&mut self, inputs: &mut [Input]) {
        let Session::Host { remotes, .. } = self else { return };
        for r in remotes.iter_mut() {
            for (k, &s) in r.slots.iter().enumerate() {
                let p = &mut r.pending[k];
                if let Some(slot) = inputs.get_mut(s) { *slot = *p; }
                p.look = [0.0; 2];
                p.jump = false;
                p.dash = false;
                p.reload = false;
                p.pick = None;
            }
        }
    }

    /// Host: after a step, stream the result.
    pub fn publish(&mut self, world: &World) {
        let Session::Host { remotes, .. } = self else { return };
        let arena = world.events.contains(&Event::NewArena).then(|| world.arena.clone());
        let snap = world.snapshot();
        for r in remotes.iter() {
            if let Some(a) = &arena { let _ = r.tx.send(ToClient::Arena(a.clone())); }
            let _ = r.tx.send(ToClient::Snap { ack: r.seq, snap: snap.clone() });
        }
    }

    /// Client: send this tick's local inputs.
    pub fn send(&mut self, local: Vec<Input>) {
        let Session::Client { client, seq, sent, .. } = self else { return };
        *seq += 1;
        let _ = client.tx.send(ToHost::Input { seq: *seq, inputs: local.clone() });
        sent.push_back((*seq, local));
        while sent.len() > 600 { sent.pop_front(); }
    }

    /// Client: apply everything the host sent. Returns (snapshots applied,
    /// arena replaced). Each applied snapshot's events stay in `world.events`
    /// only until the next one, so `on_snap` sees every snapshot.
    pub fn receive(&mut self, world: &mut World, slots: &[usize], mut on_snap: impl FnMut(&World)) -> (usize, bool) {
        let Session::Client { client, ack, sent, base_look, closed, .. } = self else { return (0, false) };
        let (mut snaps, mut arena) = (0, false);
        loop {
            match client.rx.try_recv() {
                Ok(ToClient::Arena(a)) => { world.arena = a; arena = true; }
                Ok(ToClient::Snap { ack: a, snap }) => {
                    world.apply(snap);
                    *ack = a;
                    while sent.front().is_some_and(|(s, _)| *s <= a) { sent.pop_front(); }
                    for (k, &s) in slots.iter().enumerate() { base_look[k] = (world.players[s].yaw, world.players[s].pitch); }
                    snaps += 1;
                    on_snap(world);
                }
                Ok(_) => {}
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => { *closed = true; break; }
                Err(_) => break,
            }
        }
        // Predicted look: the host's view plus inputs it has not applied yet.
        for (k, &s) in slots.iter().enumerate() {
            let (mut y, mut p) = base_look[k];
            for (_, inputs) in sent.iter() { if let Some(i) = inputs.get(k) { y += i.look[0]; p += i.look[1]; } }
            world.players[s].yaw = y;
            world.players[s].pitch = p.clamp(-1.5, 1.5);
        }
        (snaps, arena)
    }

    pub fn status(&self, slots: &[usize]) -> Option<String> {
        match self {
            Session::Local => None,
            Session::Host { remotes, .. } => Some(format!("hosting, {} remote players", remotes.iter().map(|r| r.slots.len()).sum::<usize>())),
            Session::Client { closed, sent, .. } => Some(if *closed { "host disconnected".into() } else {
                format!("online as {}, {} inputs in flight", slots.iter().map(|s| format!("P{}", s + 1)).collect::<Vec<_>>().join(" "), sent.len())
            }),
        }
    }
}
