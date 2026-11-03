//! Local, host or client play around the same `World`. The host maps
//! remote players onto bot slots and streams snapshots; a client renders
//! them, predicting only its own look direction (acknowledged input
//! sequence numbers reconcile it with the host).
use crate::net::{Client, Host, HostEvent, ToClient, ToHost};
use crate::view::Pose;
use glam::Vec3;
use growing_guns::{Breakage, Event, Input, Player, World, DT};
use std::collections::VecDeque;
use net::HostSender;
use std::sync::Arc;

/// Inputs beyond this many queued ticks are folded together, so a stalled
/// connection catches up instead of adding latency for the rest of the match.
const MAX_QUEUED_INPUTS: usize = 4;

pub struct Remote {
    client: u64,
    slots: Vec<usize>,
    tx: HostSender<ToClient>,
    inputs: InputQueue,
}

/// A remote client's inputs on the host. One entry per client tick, applied
/// one per host tick, so the host moves the player exactly as far as the
/// client predicted (bunched packets merged into one tick rubber-banded).
struct InputQueue {
    queue: VecDeque<(u32, Vec<Input>)>,
    /// The last applied input with its edges cleared: held move, fire and
    /// special carry on while the queue is empty.
    held: Vec<Input>,
    /// The newest sequence received, and the last one applied (the ack).
    received: u32,
    seq: u32,
}

impl InputQueue {
    fn new(players: usize) -> Self { Self { queue: VecDeque::new(), held: vec![Input::default(); players], received: 0, seq: 0 } }

    fn push(&mut self, seq: u32, inputs: Vec<Input>) {
        if seq <= self.received { return; }
        self.received = seq;
        self.queue.push_back((seq, inputs));
    }

    /// This host tick's inputs, in slot order.
    fn next(&mut self) -> Vec<Input> {
        while self.queue.len() > MAX_QUEUED_INPUTS {
            let (_, mut folded) = self.queue.pop_front().unwrap();
            let next = &mut self.queue.front_mut().unwrap().1;
            for (f, n) in folded.iter_mut().zip(next.iter()) { merge(f, n); }
            *next = folded;
        }
        if let Some((seq, tick)) = self.queue.pop_front() {
            for (h, i) in self.held.iter_mut().zip(tick) { *h = i; }
            self.seq = seq;
        }
        let now = self.held.clone();
        for p in &mut self.held {
            p.look = [0.0; 2];
            p.jump = false;
            p.dash = false;
            p.reload = false;
            p.pick = None;
        }
        now
    }
}

#[derive(Clone, Debug)]
pub struct LocalPred {
    pub player: Player,
    pub prev_pose: Pose,
    pub curr_pose: Pose,
    pub error_pos: Vec3,
    pub initialized: bool,
}

impl Default for LocalPred {
    fn default() -> Self {
        Self {
            player: Player::new(false),
            prev_pose: Pose::default(),
            curr_pose: Pose::default(),
            error_pos: Vec3::ZERO,
            initialized: false,
        }
    }
}

pub enum Session {
    Local,
    /// `joining`: connected clients that have not said hello yet.
    /// `arena`: send the arena with the next snapshot (replaced between ticks).
    Host { host: Host, remotes: Vec<Remote>, joining: Vec<(u64, HostSender<ToClient>)>, arena: bool },
    Client {
        client: Client,
        ack: u32,
        seq: u32,
        sent: VecDeque<(u32, Vec<Input>)>,
        inbox: VecDeque<ToClient>,
        local_pred: Vec<LocalPred>,
        primed: bool,
        closed: bool,
        broken: Breakage,
    },
}

/// Fold one more input into a pending one: look adds up, edges stick.
fn merge(into: &mut Input, i: &Input) {
    into.mv = i.mv;
    into.fire = i.fire;
    // A press must survive folding (the sim finds the edge against the last tick).
    into.special |= i.special;
    into.look = [into.look[0] + i.look[0], into.look[1] + i.look[1]];
    into.jump |= i.jump;
    into.dash |= i.dash;
    into.reload |= i.reload;
    into.pick = i.pick.or(into.pick);
}

impl Session {
    pub fn host(lan: bool) -> Result<Session, String> {
        Ok(Session::Host { host: crate::net::host(lan)?, remotes: Vec::new(), joining: Vec::new(), arena: false })
    }

    /// Connect, then wait for the host's welcome: returns the session, the
    /// world shell and the player slots this client controls.
    pub fn join(ticket: &str, players: u8, lan: bool) -> Result<(Session, World, Vec<usize>), String> {
        let mut client = crate::net::join(ticket, players, lan)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            match client.rx.try_recv() {
                Ok(ToClient::Welcome { slots, players, arena, coop }) => {
                    let mut world = World::remote(arena, players);
                    world.coop = coop;
                    let local_pred = vec![LocalPred::default(); slots.len()];
                    return Ok((
                        Session::Client {
                            client,
                            ack: 0,
                            seq: 0,
                            sent: VecDeque::new(),
                            inbox: VecDeque::new(),
                            local_pred,
                            primed: false,
                            closed: false,
                            broken: Breakage::default(),
                        },
                        world,
                        slots,
                    ));
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
        let Session::Host { host, remotes, joining, .. } = self else { return };
        while let Some(ev) = host.poll() {
            match ev {
                HostEvent::Joined { client, tx } => joining.push((client, tx)),
                HostEvent::Message { client, msg: ToHost::Hello { players } } => {
                    let Some(k) = joining.iter().position(|j| j.0 == client) else { continue };
                    let tx = joining.remove(k).1;
                    let taken: Vec<usize> = remotes.iter().flat_map(|r| r.slots.iter().copied()).collect();
                    // Coop: only the match's seats, never a wave's enemies.
                    let seats = if world.coop { world.waves.seats.min(world.players.len()) } else { world.players.len() };
                    let mut candidates: Vec<usize> = (1..seats).filter(|i| world.players[*i].bot && !taken.contains(i)).collect();
                    while !world.coop && candidates.len() < players as usize && world.players.len() < 8 {
                        candidates.push(world.add_bot());
                    }
                    // Prefer alive bots so a joining player takes over an active body first;
                    // if only dead/ghost seats remain, `join_human` spawns them into the round.
                    candidates.sort_by_key(|&i| (!world.players[i].alive, i));
                    let free: Vec<usize> = candidates.into_iter().take(players as usize).collect();
                    if free.len() < players as usize || players == 0 { let _ = tx.send(ToClient::Full); continue; }
                    for &s in &free { world.join_human(s); }
                    let _ = tx.send(ToClient::Welcome { slots: free.clone(), players: world.players.len(), arena: world.arena.clone(), coop: world.coop });
                    println!("client {client} joined as players {:?}", free.iter().map(|s| s + 1).collect::<Vec<_>>());
                    remotes.push(Remote { client, inputs: InputQueue::new(free.len()), slots: free, tx });
                }
                HostEvent::Message { client, msg: ToHost::Input { seq, inputs } } => {
                    if let Some(r) = remotes.iter_mut().find(|r| r.client == client) { r.inputs.push(seq, inputs); }
                }
                HostEvent::Left { client } => {
                    joining.retain(|j| j.0 != client);
                    if let Some(k) = remotes.iter().position(|r| r.client == client) {
                        let r = remotes.remove(k);
                        for s in r.slots { world.set_bot(s, true); }
                        println!("client {client} left");
                    }
                }
            }
        }
    }

    /// Host: put remote inputs into the tick's input list, one client tick
    /// per host tick. Without a new input, held state (move, fire, special)
    /// persists; look and edges are consumed.
    pub fn remote_inputs(&mut self, inputs: &mut [Input]) {
        let Session::Host { remotes, .. } = self else { return };
        for r in remotes.iter_mut() {
            for (&s, i) in r.slots.iter().zip(r.inputs.next()) {
                if let Some(slot) = inputs.get_mut(s) { *slot = i; }
            }
        }
    }

    /// Host: after a step, stream the result.
    pub fn publish(&mut self, world: &World) {
        let Session::Host { remotes, arena: replaced, .. } = self else { return };
        if remotes.is_empty() { *replaced = false; return; }
        let arena = (std::mem::take(replaced) || world.events.contains(&Event::NewArena)).then(|| world.arena.clone());
        let snap = Arc::new(world.snapshot());
        for r in remotes.iter() {
            if !world.breakage.is_empty() { let _ = r.tx.send(ToClient::Broken(world.breakage.clone())); }
            if let Some(a) = &arena { let _ = r.tx.send(ToClient::Arena(a.clone())); }
            let _ = r.tx.send(ToClient::Snap { ack: r.inputs.seq, snap: snap.clone() });
        }
    }

    /// Host: the arena was replaced outside a tick (a debug command or live
    /// tool); the tick's events are cleared before publishing, so say so.
    pub fn mark_arena(&mut self) { if let Session::Host { arena, .. } = self { *arena = true; } }

    /// Host: the world slots remote players hold.
    pub fn remote_slots(&self) -> Vec<usize> {
        match self { Session::Host { remotes, .. } => remotes.iter().flat_map(|r| r.slots.iter().copied()).collect(), _ => Vec::new() }
    }

    /// Host: players playing from other machines.
    pub fn remote_players(&self) -> usize {
        match self { Session::Host { remotes, .. } => remotes.iter().map(|r| r.slots.len()).sum(), _ => 0 }
    }

    /// Client: step local player prediction for this tick and send inputs to the host.
    pub fn send(&mut self, world: &mut World, slots: &[usize], local: Vec<Input>) {
        let Session::Client { client, seq, sent, local_pred, .. } = self else { return };
        *seq += 1;
        let _ = client.tx.send(ToHost::Input { seq: *seq, inputs: local.clone() });
        sent.push_back((*seq, local.clone()));
        while sent.len() > 600 { sent.pop_front(); }
        if local_pred.len() < slots.len() {
            local_pred.resize_with(slots.len(), LocalPred::default);
        }
        for (k, &s) in slots.iter().enumerate() {
            let Some(inp) = local.get(k).copied() else { continue };
            let pred = &mut local_pred[k];
            if !pred.initialized {
                if let Some(p) = world.players.get(s) {
                    pred.player = p.clone();
                    pred.prev_pose = Pose::of(p);
                    pred.curr_pose = pred.prev_pose;
                    pred.initialized = true;
                } else {
                    continue;
                }
            }
            if let Some(p) = world.players.get_mut(s) {
                *p = pred.player.clone();
            }
            pred.prev_pose = Pose::of(&world.players[s]);
            world.predict_player(s, inp);
            pred.curr_pose = Pose::of(&world.players[s]);
            pred.player = world.players[s].clone();
        }
    }

    /// Client: buffer incoming packets and advance `since_snap` smoothly through
    /// queued snapshots (absorbs Wi-Fi/relay jitter without 2x jumps or freezes),
    /// reconciling local predicted players against acknowledged inputs.
    pub fn receive(
        &mut self,
        world: &mut World,
        prev: &mut Vec<Pose>,
        since_snap: &mut f32,
        dt: f32,
        slots: &[usize],
        mut on_snap: impl FnMut(&World),
    ) -> (usize, bool) {
        let Session::Client { client, ack, sent, inbox, local_pred, primed, closed, broken, .. } = self else { return (0, false) };
        loop {
            match client.rx.try_recv() {
                Ok(msg) => inbox.push_back(msg),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => { *closed = true; break; }
                Err(_) => break,
            }
        }
        if local_pred.len() < slots.len() {
            local_pred.resize_with(slots.len(), LocalPred::default);
        }
        let mut snaps = 0usize;
        let mut arena = false;

        let mut pop_next_snap = |world: &mut World,
                                 prev: &mut Vec<Pose>,
                                 ack: &mut u32,
                                 sent: &mut VecDeque<(u32, Vec<Input>)>,
                                 local_pred: &mut [LocalPred],
                                 broken: &mut Breakage,
                                 arena: &mut bool,
                                 inbox: &mut VecDeque<ToClient>|
         -> bool {
            while let Some(msg) = inbox.pop_front() {
                match msg {
                    ToClient::Arena(a) => {
                        world.arena = a;
                        *arena = true;
                        *broken = Breakage::default();
                        for pred in local_pred.iter_mut() {
                            pred.initialized = false;
                            pred.error_pos = Vec3::ZERO;
                        }
                    }
                    ToClient::Broken(b) => {
                        world.arena.apply_breakage(&b);
                        broken.extend(b);
                    }
                    ToClient::Snap { ack: a, snap } => {
                        *prev = Pose::all(world);
                        world.apply(Arc::unwrap_or_clone(snap));
                        world.breakage = std::mem::take(broken);
                        *ack = a;
                        while sent.front().is_some_and(|(s, _)| *s <= a) {
                            sent.pop_front();
                        }
                        reconcile_locals(world, slots, sent, local_pred);
                        on_snap(world);
                        return true;
                    }
                    _ => {}
                }
            }
            false
        };

        // Apply any leading Arena/Broken messages even if no Snap followed yet.
        while inbox.front().is_some_and(|m| matches!(m, ToClient::Arena(_) | ToClient::Broken(_))) {
            match inbox.pop_front() {
                Some(ToClient::Arena(a)) => {
                    world.arena = a;
                    arena = true;
                    *broken = Breakage::default();
                    for pred in local_pred.iter_mut() {
                        pred.initialized = false;
                        pred.error_pos = Vec3::ZERO;
                    }
                }
                Some(ToClient::Broken(b)) => {
                    world.arena.apply_breakage(&b);
                    broken.extend(b);
                }
                _ => {}
            }
        }

        let count_snaps = |q: &VecDeque<ToClient>| q.iter().filter(|m| matches!(m, ToClient::Snap { .. })).count();
        let mut queued = count_snaps(inbox);

        if !*primed && queued > 0 {
            if pop_next_snap(world, prev, ack, sent, local_pred, broken, &mut arena, inbox) {
                *prev = Pose::all(world);
                *since_snap = 0.0;
                *primed = true;
                snaps += 1;
                queued = count_snaps(inbox);
            }
        }

        if *primed {
            // Keep ~1 snapshot in reserve to absorb network jitter cleanly:
            // slightly slow down when the cushion empties, gently speed up when 2+ queue up.
            let rate = match queued {
                0 => 0.88,
                1 => 1.00,
                2 => 1.08,
                3 => 1.20,
                _ => 1.35,
            };
            *since_snap += dt * rate;
            while *since_snap >= DT && count_snaps(inbox) > 0 {
                if pop_next_snap(world, prev, ack, sent, local_pred, broken, &mut arena, inbox) {
                    *since_snap -= DT;
                    snaps += 1;
                } else {
                    break;
                }
            }
            // If a major network stall dumped a backlog (>3 snapshots), catch up so latency stays low.
            while count_snaps(inbox) > 3 {
                if pop_next_snap(world, prev, ack, sent, local_pred, broken, &mut arena, inbox) {
                    snaps += 1;
                    *since_snap = (*since_snap).min(DT * 0.5);
                } else {
                    break;
                }
            }
            if count_snaps(inbox) == 0 && *since_snap > DT {
                *since_snap = DT;
            }
        }
        (snaps, arena)
    }

    /// Client: apply sub-tick `frame_alpha` interpolation and smooth reconciliation
    /// error decay to local player slots so local camera and movement run at full
    /// display refresh rate without snapshot jitter.
    pub fn pose_locals(&mut self, world: &mut World, prev: &mut [Pose], slots: &[usize], frame_alpha: f32, dt: f32) {
        let Session::Client { local_pred, .. } = self else { return };
        let decay = (-18.0 * dt.clamp(0.0, 0.1)).exp();
        for (k, &s) in slots.iter().enumerate() {
            let Some(pred) = local_pred.get_mut(k).filter(|p| p.initialized) else { continue };
            pred.error_pos *= decay;
            let turn = (pred.curr_pose.yaw - pred.prev_pose.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            let drawn = Pose {
                pos: pred.prev_pose.pos.lerp(pred.curr_pose.pos, frame_alpha) + pred.error_pos,
                yaw: pred.prev_pose.yaw + turn * frame_alpha,
                pitch: pred.prev_pose.pitch + (pred.curr_pose.pitch - pred.prev_pose.pitch) * frame_alpha,
            };
            if let Some(q) = prev.get_mut(s) {
                *q = drawn;
            }
            if let Some(p) = world.players.get_mut(s) {
                p.pos = drawn.pos;
                p.yaw = drawn.yaw;
                p.pitch = (drawn.pitch - p.recoil_pitch).clamp(-growing_guns::PITCH_LIMIT, growing_guns::PITCH_LIMIT);
            }
        }
    }

    /// A network error for Godot's red status panel (`_on_network_status_changed`).
    pub fn error(&self) -> Option<String> {
        match self { Session::Client { closed: true, .. } => Some("Lost the connection to the host.".into()), _ => None }
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

fn reconcile_locals(
    world: &mut World,
    slots: &[usize],
    sent: &VecDeque<(u32, Vec<Input>)>,
    local_pred: &mut [LocalPred],
) {
    for (k, &s) in slots.iter().enumerate() {
        let Some(auth) = world.players.get(s).cloned() else { continue };
        for (_, inputs) in sent.iter() {
            if let Some(&inp) = inputs.get(k) {
                world.predict_player(s, inp);
            }
        }
        // Keep authoritative weapon/firing timers from the snapshot so HUD ammo and recoil match.
        if let Some(p) = world.players.get_mut(s) {
            p.ammo = auth.ammo;
            p.reloading = auth.reloading;
            p.rifle_cd = auth.rifle_cd;
            p.special_cd = auth.special_cd;
            p.since_shot = auth.since_shot;
            p.recoil_pitch = auth.recoil_pitch;
            p.bloom = auth.bloom;
            p.health = auth.health;
            p.score = auth.score;
            p.cards = auth.cards;
            p.weapon = auth.weapon;
            p.offer = auth.offer;
            p.offer_age = auth.offer_age;
        }
        let reconciled = Pose::of(&world.players[s]);
        let pred = &mut local_pred[k];
        let snap_reset = !pred.initialized
            || world.players[s].alive != pred.player.alive
            || world.players[s].ghost != pred.player.ghost
            || world.players[s].frozen != pred.player.frozen
            || world.players[s].phoenix.is_some()
            || world.players[s].launching.is_some()
            || (reconciled.pos - pred.curr_pose.pos).length() > 4.0;
        if snap_reset {
            pred.player = world.players[s].clone();
            pred.curr_pose = reconciled;
            pred.prev_pose = reconciled;
            pred.error_pos = Vec3::ZERO;
            pred.initialized = true;
        } else {
            let shift = reconciled.pos - pred.curr_pose.pos;
            pred.error_pos = (pred.error_pos - shift).clamp_length_max(1.5);
            pred.prev_pose.pos += shift;
            pred.curr_pose = reconciled;
            pred.player = world.players[s].clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bunched_remote_inputs_apply_one_per_tick_and_keep_the_special_press() {
        let mut q = InputQueue::new(1);
        let walk = Input { mv: [0.0, 1.0], look: [0.05, 0.0], ..Default::default() };
        // Three client ticks in one packet burst: a press and release of RMB among them.
        q.push(1, vec![walk]);
        q.push(2, vec![Input { special: true, ..walk }]);
        q.push(3, vec![walk]);
        q.push(2, vec![Input { jump: true, ..walk }]); // a duplicate or late sequence is ignored
        let ticks: Vec<Input> = (0..3).map(|_| q.next()[0]).collect();
        assert_eq!(q.seq, 3);
        assert_eq!(ticks.iter().map(|i| i.special).collect::<Vec<_>>(), [false, true, false]);
        assert!(ticks.iter().all(|i| i.mv == walk.mv && i.look == walk.look && !i.jump));
        // Starved: held movement continues, look and edges do not repeat.
        let idle = q.next()[0];
        assert_eq!((idle.mv, idle.look, idle.special), (walk.mv, [0.0; 2], false));
        assert_eq!(q.seq, 3);
    }

    #[test]
    fn a_remote_input_backlog_folds_so_latency_stays_bounded() {
        let mut q = InputQueue::new(1);
        for seq in 1..=10 { q.push(seq, vec![Input { look: [0.01, 0.0], special: seq == 2, jump: seq == 3, ..Default::default() }]); }
        let first = q.next()[0];
        assert_eq!(q.queue.len(), MAX_QUEUED_INPUTS - 1);
        assert!(first.special && first.jump, "folded presses survive");
        assert!((first.look[0] - 0.07).abs() < 1e-6, "folded look adds up");
        assert_eq!(q.seq, 7);
    }

    #[test]
    fn host_moves_a_remote_player_as_far_as_the_client_predicted_despite_bunching() {
        let mut host = World::new(7, &[false, false]);
        host.release_cage();
        let mut client = World::remote(host.arena.clone(), 2);
        client.apply(host.snapshot());
        let mut q = InputQueue::new(1);
        let inp = Input { mv: [0.3, 1.0], look: [0.02, 0.0], ..Default::default() };
        // The client predicts 12 ticks; its packets reach the host in bursts of 3.
        for tick in 1..=12u32 {
            client.predict_player(1, inp);
            q.push(tick, vec![inp]);
            if tick % 3 != 0 { continue; }
            for _ in 0..3 { let i = q.next(); host.step(&[Input::default(), i[0]]); }
        }
        assert_eq!(q.seq, 12);
        assert!((host.players[1].pos - client.players[1].pos).length() < 1e-3,
            "host {:?} vs predicted {:?}", host.players[1].pos, client.players[1].pos);
    }

    #[test]
    fn client_prediction_and_reconciliation_keep_movement_and_look_continuous() {
        let mut host = World::new(7, &[false, false]);
        host.release_cage();
        let mut client = World::remote(host.arena.clone(), 2);
        let slots = [1usize];
        let mut sent: VecDeque<(u32, Vec<Input>)> = VecDeque::new();
        let mut local_pred = vec![LocalPred::default(); 1];

        // Prime from initial snapshot.
        client.apply(host.snapshot());
        reconcile_locals(&mut client, &slots, &sent, &mut local_pred);
        assert!(local_pred[0].initialized);

        // Step 5 ticks on client with movement + mouse turn before any snapshot returns.
        let inp = Input { mv: [0.0, 1.0], look: [0.08, -0.02], ..Default::default() };
        for seq in 1..=5 {
            sent.push_back((seq, vec![inp]));
            local_pred[0].prev_pose = Pose::of(&client.players[1]);
            client.predict_player(1, inp);
            local_pred[0].curr_pose = Pose::of(&client.players[1]);
            local_pred[0].player = client.players[1].clone();
        }
        let predicted_before_snap = local_pred[0].curr_pose;

        // Host steps first 3 ticks and sends snapshot acknowledging seq 3.
        for _ in 1..=3 {
            host.step(&[Input::default(), inp]);
        }
        client.apply(host.snapshot());
        while sent.front().is_some_and(|(s, _)| *s <= 3) {
            sent.pop_front();
        }
        reconcile_locals(&mut client, &slots, &sent, &mut local_pred);

        // Reconciled state after replaying unacked seq 4..=5 matches predicted pose with zero pop!
        let drawn_after = local_pred[0].curr_pose.pos + local_pred[0].error_pos;
        assert!((drawn_after - predicted_before_snap.pos).length() < 1e-4);
        assert!((local_pred[0].curr_pose.yaw - predicted_before_snap.yaw).abs() < 1e-5);
    }
}
