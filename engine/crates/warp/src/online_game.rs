//! The game side of online couches ([`crate::online`]): a host seats a
//! follower's people and sends it the world; a follower shows it.
use super::LuaGame;
use crate::api::{self, HudItem, Player};
use crate::online::{self, Down, FollowLink, Follower, Got, HostLink, LinkEvent, OnlineHost, Pad, Remote, Up};
use app::{Controls, Game};
use script::mlua::{self, Value};

/// Where this game plays: on its own, as an online host, or following one.
#[derive(Default)]
pub enum Online {
    #[default]
    Off,
    Host(Box<OnlineHost>),
    Follow(Box<Follower>),
}

/// People on all couches together (Joep: lobbies of 8 for now).
const MAX_PEOPLE: usize = 8;

impl LuaGame {
    /// Open this game to other couches; returns the ticket they join with.
    /// `lan` keeps it to direct addresses (a local network, tests).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn host_online(&mut self, lan: bool) -> Result<String, String> {
        Ok(self.host_with(Box::new(online::iroh_link::Host::start(lan)?)))
    }

    /// Host other couches over this transport; returns its ticket.
    pub fn host_with(&mut self, link: Box<dyn HostLink>) -> String {
        let h = OnlineHost::new(link);
        let ticket = h.ticket();
        self.core().sounds.record = Some(Vec::new());
        self.online = Online::Host(Box::new(h));
        ticket
    }

    /// Follow a host: this couch's people play there and this game only shows
    /// what the host sends. The game's own Luau stops running.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn follow_online(&mut self, ticket: &str, lan: bool, screen: [u32; 2]) -> Result<(), String> {
        self.follow_with(Box::new(online::iroh_link::Client::join(ticket, lan)?), screen);
        Ok(())
    }

    /// Follow a host over this transport.
    pub fn follow_with(&mut self, link: Box<dyn FollowLink>, screen: [u32; 2]) {
        let people = self.core().roster.iter().filter(|p| !p.bot).count().max(1);
        let f = Follower::new(link, people as u8, screen, &self.game_id());
        {
            let mut c = self.core();
            c.world.clear();
            c.sounds.stop_all();
            c.roster.retain(|p| !p.bot);
            c.hud.clear();
        }
        self.assign_views();
        self.online = Online::Follow(Box::new(f));
    }

    pub fn is_following(&self) -> bool { matches!(self.online, Online::Follow(_)) }

    /// Views this screen shows: those of this couch's people.
    pub(super) fn local_views(&self) -> usize {
        let c = self.core();
        let mut v: Vec<usize> = c.roster.iter().filter(|p| p.remote.is_none()).filter_map(|p| p.view).collect();
        v.sort_unstable();
        v.dedup();
        v.len().max(1)
    }

    /// Host: take in what followers sent, then the controls for every person
    /// in roster order (this couch's pads first, then each follower's).
    pub(super) fn online_controls(&mut self, local: Vec<Controls>) -> Vec<Controls> {
        let Online::Host(h) = &mut self.online else { return local };
        let mut joined: Vec<(u64, u8, [u32; 2], String)> = Vec::new();
        let mut left = Vec::new();
        let mut lines = Vec::new();
        while let Some(ev) = h.link.poll() {
            match ev {
                LinkEvent::Joined(_) => {}
                LinkEvent::Message(client, msg) => match msg {
                    Up::Hello { people, screen, game } => {
                        if !h.remotes.contains_key(&client) && !joined.iter().any(|j| j.0 == client) { joined.push((client, people, screen, game)); }
                    }
                    Up::Pads(pads) => if let Some(r) = h.remotes.get_mut(&client) {
                        for (i, p) in pads.iter().enumerate() { if let Some(acc) = r.pressed.get_mut(i) { *acc |= p.pressed; } }
                        r.pads = pads;
                    },
                    Up::Screen(s) => if let Some(r) = h.remotes.get_mut(&client) { r.screen = s; },
                    Up::Ping(t) => if h.remotes.contains_key(&client) { h.link.send(client, Down::Pong(t)); },
                },
                LinkEvent::Left(client) => { if h.remotes.remove(&client).is_some() { left.push(client); } }
            }
        }
        for (client, people, screen, game) in joined {
            let mine = self.game_id();
            let send = |g: &mut Self, m: Down| if let Online::Host(h) = &mut g.online { h.link.send(client, m); };
            if game != mine {
                send(self, Down::Bye(format!("this host plays {mine}, not {game}")));
                lines.push(format!("online: a couch with {game} was turned away (this is {mine})"));
                continue;
            }
            let players = self.seat_remote(client, people as usize);
            if players.is_empty() {
                send(self, Down::Bye(format!("full: {MAX_PEOPLE} people at most")));
                lines.push(format!("online: a couch of {people} was turned away (full)"));
                continue;
            }
            send(self, Down::Welcome { players: players.clone(), tick_rate: self.cfg.tick_rate });
            lines.push(format!("online: a couch joined with {} people (players {players:?})", players.len()));
            let Online::Host(h) = &mut self.online else { unreachable!() };
            h.remotes.insert(client, Remote { pressed: vec![0; players.len()], pads: vec![Pad::default(); players.len()], players, screen, base: Default::default(), meter: Default::default(), sent: 0, parts: [0; 4] });
        }
        for client in left { self.unseat_remote(client); lines.push("online: a couch left".into()); }
        for l in lines { self.say(l); }
        let roster: Vec<(u32, Option<u64>)> = self.core().roster.iter().filter(|p| !p.bot).map(|p| (p.id, p.remote)).collect();
        let Online::Host(h) = &mut self.online else { return local };
        let mut local = local.into_iter();
        let out = roster.iter().map(|(id, remote)| match remote {
            None => local.next().unwrap_or_default(),
            Some(client) => h.remotes.get_mut(client).and_then(|r| {
                let i = r.players.iter().position(|p| p == id)?;
                let mut pad = r.pads.get(i).copied().unwrap_or_default();
                pad.pressed |= std::mem::take(&mut r.pressed[i]);
                Some(pad.controls())
            }).unwrap_or_default(),
        }).collect();
        out
    }

    /// Seat a follower's people before the bots; returns their player ids.
    fn seat_remote(&mut self, client: u64, people: usize) -> Vec<u32> {
        let mut added = Vec::new();
        {
            let mut c = api::lock(&self.core);
            let room = MAX_PEOPLE.saturating_sub(c.roster.iter().filter(|p| !p.bot).count());
            for i in 0..people.min(room) {
                let mut p = Player::new(self.next_id, format!("Online {}", i + 1), None, false);
                p.remote = Some(client);
                self.next_id += 1;
                let at = c.roster.iter().position(|q| q.bot).unwrap_or(c.roster.len());
                c.roster.insert(at, p.clone());
                added.push(p);
            }
            let keep = MAX_PEOPLE.max(c.roster.iter().filter(|p| !p.bot).count());
            c.roster.truncate(keep);
        }
        self.assign_views();
        let lua = self.host.lua().clone();
        for p in &added {
            if let Ok(t) = api::player_table(&lua, p) { self.hook("player_joined", || Ok(mlua::MultiValue::from_vec(vec![Value::Table(t.clone())]))); }
        }
        if !added.is_empty() { self.hook("players_changed", || Ok(mlua::MultiValue::new())); }
        added.iter().map(|p| p.id).collect()
    }

    fn unseat_remote(&mut self, client: u64) {
        let gone: Vec<Player> = {
            let mut c = self.core();
            let gone = c.roster.iter().filter(|p| p.remote == Some(client)).cloned().collect();
            c.roster.retain(|p| p.remote != Some(client));
            gone
        };
        self.assign_views();
        let lua = self.host.lua().clone();
        for p in &gone {
            if let Ok(t) = api::player_table(&lua, p) { self.hook("player_left", || Ok(mlua::MultiValue::from_vec(vec![Value::Table(t.clone())]))); }
        }
        if !gone.is_empty() { self.hook("players_changed", || Ok(mlua::MultiValue::new())); }
    }

    /// Host, after a tick: each follower gets the changes, its cameras, the
    /// HUD drawn for its screen and the sounds played.
    pub(super) fn online_send(&mut self) {
        let Online::Host(h) = &mut self.online else { return };
        if !self.core().tick.is_multiple_of(online::send_every(self.cfg.tick_rate)) { return; }
        let Online::Host(h) = &mut self.online else { return };
        let clients: Vec<(u64, [u32; 2])> = h.remotes.iter().map(|(k, r)| (*k, r.screen)).collect();
        let sounds = self.core().sounds.record.as_mut().map(std::mem::take).unwrap_or_default();
        let mut lines = Vec::new();
        for (client, screen) in clients {
            let hud = self.draw_for(client, screen);
            let c = api::lock(&self.core);
            let Online::Host(host) = &mut self.online else { return };
            let OnlineHost { link, remotes } = &mut **host;
            let Some(r) = remotes.get_mut(&client) else { continue };
            let shared = c.roster.iter().find(|p| p.view.is_some()).map(|p| p.cam);
            let cams = r.players.iter().filter_map(|id| c.roster.iter().find(|p| p.id == *id))
                .map(|p| online::cam_out(&if c.shared_view { shared.unwrap_or(p.cam) } else { p.cam })).collect();
            let mut f = r.base.diff(&c, c.tick);
            drop(c);
            f.cams = cams;
            f.hud = r.base.hud(hud.iter().map(online::hud_out).collect());
            f.sounds = sounds.clone();
            let (a, b, hb) = (online::wire_size(&f.full) + online::wire_size(&f.looks) + online::wire_size(&f.wires), online::wire_size(&f.poses), online::wire_size(&f.hud));
            let msg = Down::Frame(f);
            let n = online::wire_size(&msg);
            r.meter.add(n);
            r.sent += n as u64;
            r.parts[0] += a as u64; r.parts[1] += b as u64; r.parts[2] += hb as u64; r.parts[3] += (n - a - b - hb) as u64;
            if !link.send(client, msg) { lines.push(format!("online: couch {client} fell behind")); }
            if let Some((kbit, per)) = r.meter.report(5.0) {
                lines.push(format!("online: to couch {client}: {kbit:.0} kbit/s, {per:.0} bytes a frame, {} entities", self.core().world.entities.len()));
            }
        }
        for l in lines { self.say(l); }
    }

    /// Run the game's `draw` for a follower's screen: its people look through
    /// its split, everyone else's HUD lands off screen and is dropped.
    fn draw_for(&mut self, client: u64, screen: [u32; 2]) -> Vec<HudItem> {
        let split = self.split();
        let saved = {
            let mut c = self.core();
            let total = c.roster.iter().filter_map(|p| p.view).max().map_or(1, |v| v + 1);
            let mut mine: Vec<usize> = c.roster.iter().filter(|p| p.remote == Some(client)).filter_map(|p| p.view).collect();
            mine.sort_unstable();
            mine.dedup();
            // One shared view: the follower sees it full screen.
            if c.shared_view { mine = vec![0]; }
            let rects = render::split_rects_with(mine.len().max(1), screen[0], screen[1], split);
            let mut views = vec![online::OFFSCREEN; total];
            for (k, v) in mine.iter().enumerate() {
                let (x, y, w, h) = rects[k.min(rects.len() - 1)];
                views[*v] = [x as f32, y as f32, w as f32, h as f32];
            }
            let saved = (std::mem::replace(&mut c.views, views), std::mem::replace(&mut c.screen, screen), std::mem::take(&mut c.hud), c.phase);
            c.phase = api::Phase::Draw;
            saved
        };
        self.hook("draw", || Ok(mlua::MultiValue::new()));
        let mut c = self.core();
        let hud: Vec<HudItem> = std::mem::take(&mut c.hud).into_iter().filter(|h| online::on_screen(h, screen)).collect();
        (c.views, c.screen, c.hud, c.phase) = saved;
        hud
    }

    /// Follower tick: show what the host sent, send this couch's pads.
    pub(super) fn follow_tick(&mut self, controls: &[Controls]) {
        let mut lines = Vec::new();
        {
            let c = &mut *api::lock(&self.core);
            let Online::Follow(f) = &mut self.online else { return };
            c.world.tick_start();
            for p in &mut c.roster { p.cam_prev = p.cam; }
            let people = c.roster.iter().filter(|p| !p.bot).count();
            let mut pads: Vec<Pad> = controls.iter().map(Pad::from).collect();
            pads.resize(people.max(1), Pad::default());
            f.link.send(Up::Pads(pads));
            if f.last_ping.is_none_or(|t| t.elapsed().as_secs_f32() > 1.0) {
                if f.players.is_empty() { let hello = f.hello.clone(); f.link.send(hello); }
                let now = f.now_us();
                f.link.send(Up::Ping(now));
                f.last_ping = Some(web_time::Instant::now());
            }
            loop {
                let msg = match f.link.poll() {
                    Got::Msg(m) => m,
                    Got::Empty => break,
                    Got::Closed => { if f.closed.is_none() { f.closed = Some("the host is gone".into()); lines.push("online: the host is gone".into()); } break; }
                };
                match msg {
                    Down::Welcome { players, tick_rate } => { lines.push(format!("online: joined as players {players:?} at {tick_rate} Hz")); f.players = players; }
                    Down::Bye(why) => { lines.push(format!("online: the host said {why}")); f.closed = Some(why); }
                    Down::Pong(t) => { f.rtt_ms = Some((f.now_us().saturating_sub(t)) as f64 / 1000.0); }
                    Down::Frame(fr) => { f.meter.add(online::wire_size(&fr)); f.queue.push_back(fr); }
                }
            }
            // One frame a tick; catch up when more than a few are waiting.
            let take = if f.queue.len() > 3 { f.queue.len() - 1 } else { f.queue.len().min(1) };
            for fr in f.queue.drain(..take).collect::<Vec<_>>() {
                {
                    {
                        f.frames += 1;
                        f.host_tick = fr.tick;
                        for e in online::apply(c, &fr).into_iter().take(3) { lines.push(format!("online: {e}")); }
                        let viewers: Vec<usize> = c.roster.iter().enumerate().filter(|(_, p)| !p.bot).map(|(i, _)| i).collect();
                        for (k, cam) in fr.cams.iter().enumerate() { if let Some(&i) = viewers.get(k) { c.roster[i].cam = online::cam_in(cam); } }
                        if let Some(patch) = &fr.hud { patch.apply(&mut f.wire_hud); f.hud = f.wire_hud.iter().map(online::hud_in).collect(); }
                        for s in &fr.sounds { let tick = c.tick; let _ = c.sounds.play(&s.name, &s.params, s.at, s.db, s.pitch, tick); }
                    }
                }
            }
            if let Some((kbit, per)) = f.meter.report(5.0) {
                lines.push(format!("online: from the host: {kbit:.0} kbit/s, {per:.0} bytes a frame, round trip {}", f.rtt_ms.map_or("?".into(), |r| format!("{r:.0} ms"))));
            }
            c.tick += 1;
        }
        for l in lines { self.say(l); }
        self.ears();
    }

    /// Follower draw: the HUD the host drew for this screen.
    pub(super) fn follow_hud(&mut self, screen: [u32; 2]) {
        let Online::Follow(f) = &mut self.online else { return };
        if f.screen != screen { f.screen = screen; f.link.send(Up::Screen(screen)); }
        let hud = f.hud.clone();
        self.core().hud = hud;
    }

    /// Bytes this host sent to all couches so far.
    pub fn online_parts(&self) -> [u64; 4] { if let Online::Host(h) = &self.online { h.remotes.values().fold([0; 4], |mut t, r| { for i in 0..4 { t[i] += r.parts[i]; } t }) } else { [0; 4] } }

    pub fn online_sent(&self) -> u64 { if let Online::Host(h) = &self.online { h.remotes.values().map(|r| r.sent).sum() } else { 0 } }

    /// Entities a follower is missing or shows differently from `host`, and
    /// entities it has that the host does not.
    pub fn mirror_errors(&self, host: &LuaGame) -> (usize, usize) {
        let (a, b) = (host.core(), self.core());
        let wrong = a.world.entities.iter().filter(|(id, e)| !b.world.entities.get(id).is_some_and(|f| online::same(e, f))).count();
        let extra = b.world.entities.keys().filter(|id| !a.world.entities.contains_key(id)).count();
        (wrong, extra)
    }

    /// A line for tests and the debug overlay.
    pub fn online_state(&self) -> serde_json::Value {
        match &self.online {
            Online::Off => serde_json::Value::Null,
            Online::Host(h) => serde_json::json!({ "role": "host", "couches": h.remotes.len(), "ticket": h.ticket() }),
            Online::Follow(f) => serde_json::json!({ "role": "follower", "players": f.players, "frames": f.frames, "host_tick": f.host_tick, "rtt_ms": f.rtt_ms, "closed": f.closed }),
        }
    }
}
