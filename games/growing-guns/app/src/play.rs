//! Growing Guns as a Warp `app::Game`: local, host or client sessions with
//! 1 to 4 local players in split screen, the rest bots. Window, tick, input
//! and presentation come from the engine's app shell.
use crate::session::Session;
use crate::{hud, view, Args};
use app::{Buttons, Controls, Draw, Frame};
use glam::Vec3;
use growing_guns::{Event, Input, World, DT, TICK_HZ};
use render::hud::HudDrawList;
use render::Lighting;

/// Hit markers, damage flash and gun bob from this tick's events.
pub fn feedback(fb: &mut [hud::Feedback], world: &World, dt: f32) {
    for (i, f) in fb.iter_mut().enumerate() {
        f.hit = (f.hit - dt).max(0.0);
        f.hurt = (f.hurt - dt).max(0.0);
        if let Some(p) = world.players.get(i) {
            let speed = (p.vel.x * p.vel.x + p.vel.z * p.vel.z).sqrt();
            f.bob += speed * dt * 0.75;
        }
    }
    for e in &world.events {
        if let Event::Hit { victim, attacker, head, .. } = *e {
            if let Some(f) = fb.get_mut(attacker) { f.hit = 0.25; f.headshot = head; }
            if let Some(f) = fb.get_mut(victim) { f.hurt = 0.35; }
        }
    }
}

/// Engine controls to this game's input. Look x is right-positive in the
/// shell and left-positive (yaw) in the simulation.
fn input(c: &Controls) -> Input {
    Input {
        mv: c.mv,
        look: [-c.look[0], c.look[1]],
        jump: c.pressed.has(Buttons::SOUTH),
        dash: c.pressed.has(Buttons::EAST | Buttons::RIGHT_SHOULDER | Buttons::LEFT_TRIGGER),
        reload: c.pressed.has(Buttons::WEST),
        fire: c.held.has(Buttons::RIGHT_TRIGGER),
    }
}

pub struct Gg {
    pub world: World,
    prev: Vec<Vec3>,
    pub vis: view::Visuals,
    light: Lighting,
    fb: Vec<hud::Feedback>,
    session: Session,
    /// World player index per local view.
    slots: Vec<usize>,
    since_snap: f32,
    rebuild: bool,
}

impl Gg {
    pub fn new(world: World, session: Session, slots: Vec<usize>) -> Self {
        let n = world.players.len();
        let mut vis = view::Visuals::build(&world);
        vis.hide_own_bodies(&slots);
        Gg { prev: world.players.iter().map(|p| p.pos).collect(), vis, light: view::lighting(),
            fb: vec![hud::Feedback::default(); n], world, session, slots, since_snap: 0.0, rebuild: true }
    }

    pub fn light_mut(&mut self) -> &mut Lighting { &mut self.light }

    /// A new round: refill the arena slots; the renderer is kept.
    fn new_arena(&mut self) {
        self.vis.set_arena(&self.world);
        self.prev = self.world.players.iter().map(|p| p.pos).collect();
    }
}

impl app::Game for Gg {
    fn local_players(&self) -> usize { self.slots.len() }

    fn tick(&mut self, controls: &[Controls]) {
        let local: Vec<Input> = controls.iter().map(input).collect();
        if self.session.is_client() { self.session.send(local); return; }
        let mut inputs = vec![Input::default(); self.world.players.len()];
        for (k, &s) in self.slots.iter().enumerate() { if let Some(i) = local.get(k) { inputs[s] = *i; } }
        self.session.poll(&mut self.world);
        self.session.remote_inputs(&mut inputs);
        self.prev = self.world.players.iter().map(|p| p.pos).collect();
        self.world.step(&inputs);
        if self.world.events.contains(&Event::NewArena) { self.new_arena(); }
        self.vis.absorb(&self.world);
        feedback(&mut self.fb, &self.world, DT);
        self.session.publish(&self.world);
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let mut alpha = frame.alpha;
        if self.session.is_client() {
            let before: Vec<Vec3> = self.world.players.iter().map(|p| p.pos).collect();
            let (vis, fb) = (&mut self.vis, &mut self.fb);
            let (snaps, arena) = self.session.receive(&mut self.world, &self.slots, |w| { vis.absorb(w); feedback(fb, w, DT); });
            if snaps > 0 { self.prev = before; self.since_snap = 0.0; } else { self.since_snap += frame.dt; }
            if arena { self.new_arena(); }
            alpha = (self.since_snap / DT).clamp(0.0, 1.0);
        }
        self.vis.pose(&self.world, &self.prev, alpha, frame.dt);
        if std::mem::take(&mut self.rebuild) || frame.renderer().is_none() { frame.set_scene(&self.vis.scene, &self.light); }
        let r = frame.renderer().expect("scene set above");
        self.vis.flush(frame.gpu, r)?;
        r.update_instances(frame.gpu, self.vis.first_dynamic, self.vis.dynamic())?;
        r.set_local_lights(&self.vis.local_lights())?;
        let rects = frame.rects(self.slots.len());
        let cameras = self.slots.iter().map(|&s| view::camera(&self.world, s, &self.prev, alpha)).collect();
        let mut list = HudDrawList::new(frame.size);
        for (rect, &s) in rects.iter().zip(&self.slots) { hud::view(&mut list, &self.world, s, *rect, &self.fb[s]); }
        let status = self.session.status(&self.slots);
        hud::global(&mut list, &self.world, status.as_deref());
        Ok(Draw { cameras, light: self.light.clone(), hud: list })
    }

    fn title(&self) -> Option<String> { Some(format!("round {}", self.world.round)) }

    fn state(&self) -> Option<String> {
        let w = &self.world;
        let players: Vec<_> = w.players.iter().enumerate().map(|(i, p)| serde_json::json!({
            "index": i, "bot": p.bot, "local_view": self.slots.iter().position(|&s| s == i),
            "alive": p.alive, "health": p.health, "score": p.score, "ammo": p.ammo, "reloading": p.reloading(),
            "pos": p.pos.to_array(), "vel": p.vel.to_array(), "yaw": p.yaw, "pitch": p.pitch,
            "on_floor": p.on_floor, "on_wall": p.wall_normal.is_some(), "dash_charges": p.dash_charges,
        })).collect();
        Some(serde_json::json!({
            "tick": w.tick, "round": w.round, "intermission": w.intermission,
            "arena": { "seed": w.arena.seed, "size": w.arena.size, "variant": format!("{:?}", w.arena.variant), "blocks": w.arena.blocks.len() },
            "players": players, "bullets": w.bullets.len(), "events": w.events,
        }).to_string())
    }
}

pub fn run(args: &Args) -> Result<(), String> {
    let total = args.num("--total", 4usize).clamp(1, 8);
    let humans = args.num("--players", 1usize).clamp(1, 4).min(total);
    let seed = args.num("--seed", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs()));
    let lan = args.flag("--lan");
    let (session, world, slots) = if let Some(ticket) = args.value("--join") {
        let (session, world, slots) = Session::join(ticket, humans as u8, lan)?;
        println!("joined: playing as {:?}", slots.iter().map(|s| s + 1).collect::<Vec<_>>());
        (session, world, slots)
    } else {
        let bots: Vec<bool> = (0..total).map(|i| i >= humans).collect();
        let session = if args.flag("--host") { Session::host(lan)? } else { Session::Local };
        if let Some(t) = session.ticket() {
            println!("hosting. Others join with:\n  growing-guns play --join {t}{}", if lan { " --lan" } else { "" });
            let _ = std::fs::write("growing-guns-ticket.txt", t);
        }
        (session, World::new(seed, &bots), (0..humans).collect())
    };
    println!("seed {seed}, {} local, {} players", slots.len(), world.players.len());
    let mut game = Gg::new(world, session, slots);
    let cfg = app::Config {
        title: "Growing Guns | Warp".into(),
        tick_hz: TICK_HZ as f32,
        fullscreen: args.flag("--fullscreen"),
        keyboard: !args.flag("--no-kbm"),
        frames: args.num("--frames", 0u64),
        ..Default::default()
    };
    app::run(cfg, &mut game)
}
