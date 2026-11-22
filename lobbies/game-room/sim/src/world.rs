//! Platformer simulation in metres, stepped at a fixed 60 Hz. Pure data, so
//! tests run it headless and the view can be rebuilt freely. A line-by-line
//! port of the Godot lobby's world.gd: same constants, same order of
//! updates, same random draws.

use crate::layout::{self, PLATFORMS};
use gdcompat::Pcg;

pub const DT: f64 = 1.0 / 60.0;
pub const WALK: f64 = 7.2;
pub const ACCEL_GROUND: f64 = 70.0;
pub const ACCEL_AIR: f64 = 42.0;
pub const FRICTION: f64 = 60.0;
pub const JUMP: f64 = 14.6;
pub const GRAVITY: f64 = 40.0;
/// Extra gravity multiplier after releasing jump.
pub const FAST_FALL: f64 = 2.0;
pub const TERMINAL: f64 = 26.0;
pub const WALL_SLIDE: f64 = 2.6;
pub const COYOTE: f64 = 0.09;
pub const BUFFER: f64 = 0.12;
pub const HALF_W: f64 = 0.42;
pub const HEIGHT: f64 = 1.7;

pub const BTN_A: u32 = 1;
pub const BTN_B: u32 = 2;
pub const BTN_X: u32 = 4;
pub const BTN_Y: u32 = 8;
pub const BTN_LB: u32 = 16;
pub const BTN_RB: u32 = 32;
pub const BTN_BACK: u32 = 64;
pub const BTN_START: u32 = 128;
/// Right stick click (R3), as the host's wire sends it.
pub const BTN_RS: u32 = 1 << 9;
pub const BTN_UP: u32 = 1 << 10;
pub const BTN_DOWN: u32 = 1 << 11;
pub const BTN_LEFT: u32 = 1 << 12;
pub const BTN_RIGHT: u32 = 1 << 13;
/// Right trigger, folded into the button mask (the wire sends it as an axis).
pub const BTN_RT: u32 = 1 << 20;

pub const DROP_EVERY: f64 = 9.0;
pub const MAX_ITEMS: usize = 6;
pub const ITEM_LIFE: f64 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bomb,
    Blaster,
    Popcorn,
    Minigun,
    Bazooka,
    Laser,
    Saber,
    Ball,
}

pub const ITEM_KINDS: [Kind; 10] = [
    Kind::Bomb,
    Kind::Blaster,
    Kind::Popcorn,
    Kind::Minigun,
    Kind::Bazooka,
    Kind::Laser,
    Kind::Saber,
    Kind::Bomb,
    Kind::Blaster,
    Kind::Ball,
];

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Bomb => "bomb",
            Kind::Blaster => "blaster",
            Kind::Popcorn => "popcorn",
            Kind::Minigun => "minigun",
            Kind::Bazooka => "bazooka",
            Kind::Laser => "laser",
            Kind::Saber => "saber",
            Kind::Ball => "ball",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        ITEM_KINDS.into_iter().find(|k| k.name() == s)
    }

    pub fn weapon(self) -> Option<&'static Weapon> {
        WEAPONS.iter().find(|(k, _)| *k == self).map(|(_, w)| w)
    }
}

/// Shots: speed, life, knock (sideways), lift, stun, gravity (0 flies
/// straight) and blast (explodes where it lands).
#[derive(Debug)]
pub struct Weapon {
    pub ammo: i32,
    pub cooldown: f64,
    pub recoil: f64,
    pub speed: f64,
    pub life: f64,
    pub knock: f64,
    pub lift: f64,
    pub stun: f64,
    pub spread: f64,
    pub gravity: f64,
    pub handful: Option<u32>,
    pub blast: bool,
}

const W: Weapon = Weapon {
    ammo: 0,
    cooldown: 0.0,
    recoil: 0.0,
    speed: 0.0,
    life: 0.0,
    knock: 0.0,
    lift: 0.0,
    stun: 0.0,
    spread: 0.0,
    gravity: 0.0,
    handful: None,
    blast: false,
};

pub const WEAPONS: [(Kind, Weapon); 6] = [
    (Kind::Blaster, Weapon { ammo: 8, cooldown: 0.22, recoil: 1.5, speed: 20.0, life: 0.7, knock: 6.5, lift: 4.5, stun: 0.3, ..W }),
    (Kind::Minigun, Weapon { ammo: 60, cooldown: 0.06, recoil: 0.7, speed: 24.0, life: 0.6, knock: 3.4, lift: 2.2, stun: 0.12, spread: 0.12, ..W }),
    (Kind::Bazooka, Weapon { ammo: 2, cooldown: 0.9, recoil: 7.0, speed: 12.0, life: 1.8, blast: true, ..W }),
    (Kind::Popcorn, Weapon { ammo: 10, cooldown: 0.3, speed: 7.0, life: 1.4, knock: 1.8, lift: 1.6, stun: 0.06, gravity: 0.6, handful: Some(6), ..W }),
    (Kind::Laser, Weapon { ammo: 4, cooldown: 0.55, recoil: 2.5, knock: 10.0, lift: 5.0, stun: 0.45, ..W }),
    (Kind::Saber, Weapon { ammo: 14, cooldown: 0.32, knock: 11.0, lift: 7.0, stun: 0.45, ..W }),
];

#[derive(Clone, Debug)]
pub struct Player {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub facing: f64,
    pub grounded: bool,
    pub wall: i32,
    pub held: u32,
    pub pressed: u32,
    /// x right, y down (SDL convention), -1..1.
    pub stick: [f32; 2],
    pub coyote: f64,
    pub buffer: f64,
    pub drop: f64,
    pub item: Option<u32>,
    pub hat: String,
    pub punch: f64,
    pub punch_cd: f64,
    pub stun: f64,
    pub fire_cd: f64,
    pub last_input: f64,
    pub sleeping: bool,
    pub walk_phase: f64,
    pub land: f64,
    pub spawn: f64,
    pub cheer: f64,
    pub carrying: Option<String>,
    pub carried_by: Option<String>,
    pub flung: f64,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: u32,
    pub kind: Kind,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub holder: Option<String>,
    pub fuse: f64,
    pub ammo: i32,
    pub age: f64,
    pub spin: f64,
    /// Angular velocity (rad/s, counter-clockwise); spin is its integral.
    pub av: f64,
    pub thrower: Option<String>,
    pub dead: bool,
}

#[derive(Clone, Debug)]
pub struct Shot {
    pub kind: Kind,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub owner: String,
    pub life: f64,
    pub hue: f64,
    pub knock: f64,
    pub lift: f64,
    pub stun: f64,
    pub gravity: f64,
    pub blast: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ev {
    Spawn,
    Jump,
    WallJump,
    Land,
    Grab,
    Throw,
    Punch,
    Hit,
    Fire,
    Laser,
    Slash,
    Kick,
    Drop,
    Bounce,
    Explode,
    Ejected,
}

/// Something the view shows once: dust, a flash, a sound.
#[derive(Clone, Debug)]
pub struct Event {
    pub ev: Ev,
    pub x: f64,
    pub y: f64,
    pub player: Option<String>,
    pub kind: Option<Kind>,
    /// Land: fall speed. Laser: the beam's far end x. Slash: facing.
    pub value: f64,
}

fn ev(ev: Ev, x: f64, y: f64, player: Option<&str>) -> Event {
    Event { ev, x, y, player: player.map(str::to_owned), kind: None, value: 0.0 }
}

fn move_toward(from: f64, to: f64, delta: f64) -> f64 {
    if (to - from).abs() <= delta { to } else { from + (to - from).signum() * delta }
}

/// Godot's signf: 0 stays 0.
fn signf(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// Godot's Rect2 (single floats) as x, y, w, h.
#[derive(Clone, Copy)]
struct Rect(f32, f32, f32, f32);

impl Rect {
    fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect(x as f32, y as f32, w as f32, h as f32)
    }
    fn intersects(self, b: Rect) -> bool {
        // Godot compares the far edges after adding in single floats.
        !(self.0 >= b.0 + b.2 || self.0 + self.2 <= b.0 || self.1 >= b.1 + b.3 || self.1 + self.3 <= b.1)
    }
    fn has_point(self, x: f64, y: f64) -> bool {
        let (x, y) = (x as f32, y as f32);
        x >= self.0 && y >= self.1 && x < self.0 + self.2 && y < self.1 + self.3
    }
}

/// Godot's Vector2(x, y).length() in single floats.
/// Godot's Vector2.normalized() in single floats.
fn normalized(x: f32, y: f32) -> (f32, f32) {
    let l = x * x + y * y;
    if l == 0.0 {
        return (x, y);
    }
    let l = l.sqrt();
    (x / l, y / l)
}

fn length32(x: f64, y: f64) -> f32 {
    let (x, y) = (x as f32, y as f32);
    (x * x + y * y).sqrt()
}

pub struct World {
    /// Move free items as the Godot lobby did (no rolling or tumbling), for
    /// the line-for-line parity test against the original.
    pub classic_items: bool,
    /// In join order, like the Godot dictionary.
    pub players: Vec<Player>,
    pub items: Vec<Item>,
    pub shots: Vec<Shot>,
    /// Consumed by the view each frame.
    pub events: Vec<Event>,
    /// Players thrown out the exit, consumed by the lobby.
    pub ejected: Vec<String>,
    pub time: f64,
    pub ticks: u64,
    drop_at: f64,
    next_item: u32,
    pub rng: Pcg,
}

impl Default for World {
    fn default() -> Self {
        World::new(7)
    }
}

impl World {
    pub fn new(seed: u64) -> World {
        World {
            classic_items: false,
            players: Vec::new(),
            items: Vec::new(),
            shots: Vec::new(),
            events: Vec::new(),
            ejected: Vec::new(),
            time: 0.0,
            ticks: 0,
            drop_at: 4.0,
            next_item: 1,
            rng: Pcg::new(seed),
        }
    }

    pub fn index(&self, id: &str) -> Option<usize> {
        self.players.iter().position(|p| p.id == id)
    }

    pub fn player(&self, id: &str) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn player_mut(&mut self, id: &str) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    fn index_of(&self, id: &Option<String>) -> Option<usize> {
        id.as_deref().and_then(|id| self.index(id))
    }

    pub fn ensure_player(&mut self, id: &str, slot: usize) -> &mut Player {
        if let Some(i) = self.index(id) {
            return &mut self.players[i];
        }
        let x = layout::SPAWN_X[slot % layout::SPAWN_X.len()];
        let p = Player {
            id: id.to_owned(),
            x,
            y: 3.0 + slot as f64 * 0.4,
            vx: 0.0,
            vy: 0.0,
            facing: if x < 0.0 { 1.0 } else { -1.0 },
            grounded: false,
            wall: 0,
            held: 0,
            pressed: 0,
            stick: [0.0f32; 2],
            coyote: 0.0,
            buffer: 0.0,
            drop: 0.0,
            item: None,
            hat: "none".into(),
            punch: 0.0,
            punch_cd: 0.0,
            stun: 0.0,
            fire_cd: 0.0,
            last_input: self.time,
            sleeping: false,
            walk_phase: 0.0,
            land: 0.0,
            spawn: 0.6,
            cheer: 0.0,
            carrying: None,
            carried_by: None,
            flung: 0.0,
        };
        self.events.push(ev(Ev::Spawn, p.x, p.y, Some(id)));
        self.players.push(p);
        self.players.last_mut().unwrap()
    }

    pub fn remove_player(&mut self, id: &str) {
        let Some(i) = self.index(id) else { return };
        self.drop_item(i, false);
        self.release(i);
        self.players.remove(i);
    }

    /// held: button bitmask, stick: x right, y down (SDL convention), -1..1.
    pub fn set_input(&mut self, id: &str, held: u32, stick: [f32; 2]) {
        let Some(i) = self.index(id) else { return };
        let time = self.time;
        let p = &mut self.players[i];
        p.pressed |= held & !p.held;
        let moved = held != 0 || (stick[0] * stick[0] + stick[1] * stick[1]).sqrt() as f64 > 0.3;
        if moved {
            p.last_input = time;
        }
        // Carrying is for players who are away: any input wriggles free.
        if moved && p.carried_by.is_some() {
            self.release(i);
        }
        let p = &mut self.players[i];
        p.held = held;
        p.stick = stick;
    }

    pub fn step(&mut self) {
        self.time += DT;
        self.ticks += 1;
        for i in 0..self.players.len() {
            self.step_player(i);
        }
        self.step_items();
        self.step_shots();
        self.maybe_drop();
        for p in &mut self.players {
            p.pressed = 0;
        }
    }

    fn axis(p: &Player) -> f64 {
        let mut x = p.stick[0] as f64;
        if x.abs() < 0.25 {
            x = 0.0;
        }
        if p.held & BTN_LEFT != 0 {
            x = -1.0;
        }
        if p.held & BTN_RIGHT != 0 {
            x = 1.0;
        }
        x.clamp(-1.0, 1.0)
    }

    fn down(p: &Player) -> bool {
        p.stick[1] as f64 > 0.6 || p.held & BTN_DOWN != 0
    }

    fn step_player(&mut self, i: usize) {
        if self.players[i].carried_by.is_some() {
            self.follow_carrier(i);
            return;
        }
        let p = &mut self.players[i];
        p.flung = (p.flung - DT).max(0.0);
        p.spawn = (p.spawn - DT).max(0.0);
        p.land = (p.land - DT).max(0.0);
        p.cheer = (p.cheer - DT).max(0.0);
        p.punch = (p.punch - DT).max(0.0);
        p.punch_cd = (p.punch_cd - DT).max(0.0);
        p.fire_cd = (p.fire_cd - DT).max(0.0);
        p.drop = (p.drop - DT).max(0.0);
        let stunned = p.stun > 0.0;
        p.stun = (p.stun - DT).max(0.0);
        let axis = if stunned || p.sleeping { 0.0 } else { Self::axis(p) };
        if axis != 0.0 {
            p.facing = signf(axis);
        }
        // Horizontal movement with snappy acceleration and friction.
        let target = axis * WALK;
        let mut accel = if p.grounded { ACCEL_GROUND } else { ACCEL_AIR };
        if stunned {
            accel = 4.0;
        }
        if axis == 0.0 && p.grounded && !stunned {
            p.vx = move_toward(p.vx, 0.0, FRICTION * DT);
        } else {
            p.vx = move_toward(p.vx, target, accel * DT);
        }
        // Jumping: coyote time, buffered presses, variable height, wall jumps.
        p.coyote = if p.grounded { COYOTE } else { (p.coyote - DT).max(0.0) };
        if p.pressed & BTN_A != 0 && !p.sleeping {
            p.buffer = BUFFER;
        } else {
            p.buffer = (p.buffer - DT).max(0.0);
        }
        if p.buffer > 0.0 && !stunned {
            if Self::down(p) && p.grounded && p.y > 0.01 {
                p.drop = 0.25;
                p.grounded = false;
                p.y -= 0.05;
                p.buffer = 0.0;
            } else if p.coyote > 0.0 {
                p.vy = JUMP;
                p.grounded = false;
                p.coyote = 0.0;
                p.buffer = 0.0;
                let e = ev(Ev::Jump, p.x, p.y, Some(&p.id));
                self.events.push(e);
            } else if p.wall != 0 {
                p.vy = JUMP * 0.92;
                p.vx = -(p.wall as f64) * WALK * 1.15;
                p.facing = -(p.wall as f64);
                p.buffer = 0.0;
                let e = ev(Ev::WallJump, p.x, p.y, Some(&p.id));
                self.events.push(e);
            }
        }
        let p = &mut self.players[i];
        let mut gravity = GRAVITY;
        if p.vy > 0.0 && p.held & BTN_A == 0 {
            gravity *= 1.0 + FAST_FALL;
        }
        p.vy = (p.vy - gravity * DT).max(-TERMINAL);
        if p.wall != 0 && p.vy < -WALL_SLIDE && axis == p.wall as f64 {
            p.vy = -WALL_SLIDE;
        }
        self.move_body(i);
        let p = &mut self.players[i];
        if p.grounded && p.vx.abs() > 0.2 {
            p.walk_phase += p.vx.abs() * DT * 1.6;
        }
        if p.flung > 0.0 && (p.x - layout::EXIT_DOOR).abs() <= 0.7 && p.y < 2.6 {
            p.flung = 0.0;
            self.ejected.push(p.id.clone());
            let e = ev(Ev::Ejected, p.x, p.y + 0.8, Some(&p.id));
            self.events.push(e);
        } else if p.grounded {
            p.flung = 0.0;
        }
        self.actions(i, stunned);
    }

    fn move_body(&mut self, i: usize) {
        let p = &mut self.players[i];
        let was_grounded = p.grounded;
        let old_y = p.y;
        let fall = p.vy;
        p.x += p.vx * DT;
        p.y += p.vy * DT;
        p.wall = 0;
        if p.x - HALF_W < layout::LEFT {
            p.x = layout::LEFT + HALF_W;
            p.vx = p.vx.max(0.0);
            if !p.grounded {
                p.wall = -1;
            }
        }
        if p.x + HALF_W > layout::RIGHT {
            p.x = layout::RIGHT - HALF_W;
            p.vx = p.vx.min(0.0);
            if !p.grounded {
                p.wall = 1;
            }
        }
        if p.y + HEIGHT > layout::CEILING {
            p.y = layout::CEILING - HEIGHT;
            p.vy = p.vy.min(0.0);
        }
        p.grounded = false;
        let top = landing(p.x, HALF_W, old_y, p.y, p.drop > 0.0);
        if p.vy <= 0.0 && top > f64::NEG_INFINITY {
            p.y = top;
            p.vy = 0.0;
            p.grounded = true;
            if !was_grounded && fall < -8.0 {
                p.land = 0.12;
                let mut e = ev(Ev::Land, p.x, p.y, Some(&p.id));
                e.value = -fall;
                self.events.push(e);
            }
        }
    }

    fn actions(&mut self, i: usize, stunned: bool) {
        if stunned || self.players[i].sleeping {
            return;
        }
        // Captured before B, like the Godot dictionary: a dropped item can
        // still be used by RT in the same tick.
        let item = self.item_index(self.players[i].item);
        let pressed = self.players[i].pressed;
        if self.players[i].carrying.is_some() {
            if pressed & BTN_B != 0 {
                self.throw_player(i);
            }
            return;
        }
        if pressed & BTN_B != 0 {
            if item.is_some() {
                self.drop_item(i, true);
            } else if self.pickup_target(i).is_some() {
                self.grab(i);
            } else {
                self.grab_player(i);
            }
        }
        // RT uses the held item, or punches empty-handed. X is only for the
        // room's buttons, so it does nothing here. The minigun fires while RT
        // is held, everything else once per pull.
        let Some(item) = item else {
            if pressed & BTN_RT != 0 {
                self.punch(i);
            }
            return;
        };
        let kind = self.items[item].kind;
        if kind == Kind::Minigun && self.players[i].held & BTN_RT != 0 {
            self.fire(i, item);
        } else if pressed & BTN_RT != 0 {
            match kind {
                Kind::Bomb => self.drop_item(i, true),
                Kind::Laser => self.laser(i, item),
                Kind::Saber => self.slash(i, item),
                k if k.weapon().is_some() => self.fire(i, item),
                _ => {}
            }
        }
    }

    /// The exact item B will use, shared with the floating pickup hint.
    pub fn pickup_target(&self, i: usize) -> Option<usize> {
        let p = &self.players[i];
        if p.sleeping || p.stun > 0.0 || self.item_index(p.item).is_some() {
            return None;
        }
        self.items.iter().position(|item| {
            item.holder.is_none()
                && !item.dead
                && (item.x - p.x).abs() < 0.8
                && item.y < p.y + HEIGHT
                && item.y + 0.6 > p.y
        })
    }

    fn grab(&mut self, i: usize) {
        let Some(it) = self.pickup_target(i) else { return };
        if self.items[it].kind == Kind::Ball {
            self.kick(i, it, 1.6);
            return;
        }
        let id = self.players[i].id.clone();
        let item = &mut self.items[it];
        item.holder = Some(id.clone());
        self.players[i].item = Some(item.id);
        if item.kind == Kind::Bomb && item.fuse < 0.0 {
            item.fuse = 3.2;
        }
        let e = ev(Ev::Grab, item.x, item.y, Some(&id));
        self.events.push(e);
    }

    /// The player B lifts when no item is in reach: the nearest one close
    /// by (up to a metre above or below, so a player on the couch or the TV
    /// console counts) who is not already carrying or being carried.
    pub fn carry_target(&self, i: usize) -> Option<usize> {
        let p = &self.players[i];
        if p.sleeping
            || p.stun > 0.0
            || p.carrying.is_some()
            || p.carried_by.is_some()
            || self.item_index(p.item).is_some()
        {
            return None;
        }
        self.players
            .iter()
            .enumerate()
            .filter(|&(j, o)| {
                j != i
                    && o.carrying.is_none()
                    && o.carried_by.is_none()
                    && o.spawn <= 0.0
                    && (o.x - p.x).abs() < 1.1
                    && (o.y - p.y).abs() < 1.2
            })
            .min_by(|a, b| (a.1.x - p.x).abs().total_cmp(&(b.1.x - p.x).abs()))
            .map(|(j, _)| j)
    }

    fn grab_player(&mut self, i: usize) {
        let Some(o) = self.carry_target(i) else { return };
        self.drop_item(o, false);
        let pid = self.players[i].id.clone();
        self.players[i].carrying = Some(self.players[o].id.clone());
        self.players[o].carried_by = Some(pid.clone());
        self.follow_carrier(o);
        let e = ev(Ev::Grab, self.players[o].x, self.players[o].y, Some(&pid));
        self.events.push(e);
    }

    /// A carried player lies across the carrier's raised arms.
    fn follow_carrier(&mut self, i: usize) {
        let carrier = self.index_of(&self.players[i].carried_by);
        let Some(c) = carrier.filter(|&c| self.players[c].carrying.as_deref() == Some(&self.players[i].id))
        else {
            self.players[i].carried_by = None;
            return;
        };
        let (x, y, vx, vy) = {
            let c = &self.players[c];
            (c.x, c.y, c.vx, c.vy)
        };
        let p = &mut self.players[i];
        p.x = x;
        p.y = y + HEIGHT * 0.85;
        p.vx = vx;
        p.vy = vy;
        p.grounded = false;
    }

    /// Lets go of whoever this player carries or is carried by.
    fn release(&mut self, i: usize) {
        if let Some(c) = self.index_of(&self.players[i].carrying) {
            self.players[c].carried_by = None;
            self.players[c].vy = 3.0;
        }
        if let Some(c) = self.index_of(&self.players[i].carried_by) {
            self.players[c].carrying = None;
            self.players[i].vy = 3.0;
        }
        self.players[i].carrying = None;
        self.players[i].carried_by = None;
    }

    /// Throws the carried player; landing in the exit door takes them out.
    fn throw_player(&mut self, i: usize) {
        let other = self.index_of(&self.players[i].carrying);
        self.players[i].carrying = None;
        let Some(o) = other else { return };
        let (x, facing, vx, pid) = {
            let p = &self.players[i];
            (p.x, p.facing, p.vx, p.id.clone())
        };
        let other = &mut self.players[o];
        other.carried_by = None;
        other.x = x + facing * 0.5;
        other.vx = facing * 12.0 + vx * 0.5;
        other.vy = 7.0;
        other.grounded = false;
        other.stun = 0.5;
        other.flung = 1.4;
        let e = ev(Ev::Throw, other.x, other.y, Some(&pid));
        self.events.push(e);
    }

    fn drop_item(&mut self, i: usize, thrown: bool) {
        let item = self.item_index(self.players[i].item);
        self.players[i].item = None;
        let Some(it) = item else { return };
        let p = &self.players[i];
        let item = &mut self.items[it];
        item.holder = None;
        item.x = p.x + p.facing * 0.5;
        item.y = p.y + 0.7;
        if thrown {
            item.vx = p.facing * 11.0 + p.vx * 0.5;
            item.vy = 6.5;
            // Released with the hand rolling over it: forward spin (a box tumbles).
            let body = Body::of(item.kind);
            item.av = -p.facing * if body.rolling.is_some() { 6.0 / body.radius * 0.25 } else { 11.0 };
            item.thrower = Some(p.id.clone());
            let e = ev(Ev::Throw, item.x, item.y, Some(&p.id));
            self.events.push(e);
        } else {
            item.vx = p.vx;
            item.vy = 2.0;
            item.av = 0.0;
        }
    }

    fn punch(&mut self, i: usize) {
        if self.players[i].punch_cd > 0.0 {
            return;
        }
        let p = &mut self.players[i];
        p.punch = 0.24;
        p.punch_cd = 0.36;
        let (px, py, facing) = (p.x, p.y, p.facing);
        let e = ev(Ev::Punch, p.x, p.y, Some(&p.id));
        self.events.push(e);
        let reach = Rect::new(px + if facing > 0.0 { 0.0 } else { -1.25 }, py + 0.25, 1.25, 0.8);
        self.hit_players(i, reach, facing * 7.5, 5.0, 0.28);
        for it in 0..self.items.len() {
            let item = &self.items[it];
            if item.kind == Kind::Ball && reach.has_point(item.x, item.y + 0.4) {
                self.kick(i, it, 1.8);
            }
        }
    }

    fn hit_players(&mut self, i: usize, reach: Rect, vx: f64, vy: f64, stun: f64) {
        for j in 0..self.players.len() {
            if j == i {
                continue;
            }
            let o = &self.players[j];
            if reach.intersects(Rect::new(o.x - HALF_W, o.y, HALF_W * 2.0, HEIGHT)) {
                self.knock(j, vx, vy, stun);
                let o = &self.players[j];
                let e = ev(Ev::Hit, o.x, o.y + 0.8, Some(&o.id));
                self.events.push(e);
            }
        }
    }

    fn fire(&mut self, i: usize, it: usize) {
        if self.players[i].fire_cd > 0.0 {
            return;
        }
        let kind = self.items[it].kind;
        let w = kind.weapon().expect("a weapon");
        self.players[i].fire_cd = w.cooldown;
        let (px, py, facing, pid) = {
            let p = &self.players[i];
            (p.x, p.y, p.facing, p.id.clone())
        };
        for _ in 0..w.handful.unwrap_or(1) {
            let y = py + 0.72 + self.randf_range(-w.spread, w.spread);
            let hue = self.rng.randf() as f64;
            let mut shot = Shot {
                kind,
                x: px + facing * 0.7,
                y,
                vx: facing * w.speed,
                vy: 0.0,
                owner: pid.clone(),
                life: w.life,
                hue,
                knock: w.knock,
                lift: w.lift,
                stun: w.stun,
                gravity: w.gravity,
                blast: w.blast,
            };
            if w.handful.is_some() {
                shot.vx = facing * self.randf_range(w.speed * 0.6, w.speed * 1.3);
                shot.vy = self.randf_range(3.0, 7.5);
            }
            self.shots.push(shot);
        }
        let mut e = ev(Ev::Fire, px + facing * 0.8, py + 0.72, Some(&pid));
        e.kind = Some(kind);
        self.events.push(e);
        self.players[i].vx -= facing * w.recoil;
        self.use_ammo(i, it);
    }

    /// A beam across the room: everyone in its line flies, nothing stops it.
    fn laser(&mut self, i: usize, it: usize) {
        if self.players[i].fire_cd > 0.0 {
            return;
        }
        let w = Kind::Laser.weapon().unwrap();
        self.players[i].fire_cd = w.cooldown;
        let (px, py, facing, pid) = {
            let p = &self.players[i];
            (p.x, p.y, p.facing, p.id.clone())
        };
        let y = py + 0.72;
        let end = if facing > 0.0 { layout::RIGHT } else { layout::LEFT };
        for j in 0..self.players.len() {
            let o = &self.players[j];
            if j == i || y < o.y || y > o.y + HEIGHT {
                continue;
            }
            if signf(o.x - px) != signf(facing) {
                continue;
            }
            self.knock(j, facing * w.knock, w.lift, w.stun);
            let e = ev(Ev::Hit, self.players[j].x, y, Some(&self.players[j].id));
            self.events.push(e);
        }
        let mut e = ev(Ev::Laser, px + facing * 0.8, y, Some(&pid));
        e.value = end;
        self.events.push(e);
        self.players[i].vx -= facing * w.recoil;
        self.use_ammo(i, it);
    }

    /// A wide, hard swing. Wears out after a number of swings.
    fn slash(&mut self, i: usize, it: usize) {
        if self.players[i].fire_cd > 0.0 {
            return;
        }
        let w = Kind::Saber.weapon().unwrap();
        let p = &mut self.players[i];
        p.fire_cd = w.cooldown;
        p.punch = 0.24;
        let (px, py, facing, pid) = (p.x, p.y, p.facing, p.id.clone());
        let reach = Rect::new(px + if facing > 0.0 { -0.3 } else { -1.9 }, py, 2.2, HEIGHT + 0.3);
        self.hit_players(i, reach, facing * w.knock, w.lift, w.stun);
        for o in 0..self.items.len() {
            let other = &self.items[o];
            if other.kind == Kind::Ball && reach.has_point(other.x, other.y + 0.4) {
                self.kick(i, o, 2.4);
            }
        }
        let mut e = ev(Ev::Slash, px + facing * 0.9, py + 0.9, Some(&pid));
        e.value = facing;
        self.events.push(e);
        self.use_ammo(i, it);
    }

    fn use_ammo(&mut self, i: usize, it: usize) {
        let item = &mut self.items[it];
        item.ammo -= 1;
        if item.ammo <= 0 {
            item.dead = true;
            self.players[i].item = None;
        }
    }

    fn knock(&mut self, i: usize, vx: f64, vy: f64, stun: f64) {
        self.drop_item(i, false);
        if self.players[i].carrying.is_some() {
            self.release(i);
        }
        let p = &mut self.players[i];
        p.vx = vx;
        p.vy = p.vy.max(vy);
        p.grounded = false;
        p.stun = p.stun.max(stun);
    }

    fn kick(&mut self, i: usize, it: usize, power: f64) {
        let p = &self.players[i];
        let item = &mut self.items[it];
        item.vx = p.facing * 5.5 * power + p.vx * 0.6;
        item.vy = 5.0 * power;
        item.av = -item.vx / Body::of(item.kind).radius * 0.5;
        let e = ev(Ev::Kick, item.x, item.y, Some(&p.id));
        self.events.push(e);
    }

    /// The live item with this id.
    pub fn item_index(&self, id: Option<u32>) -> Option<usize> {
        let id = id?;
        self.items.iter().position(|item| item.id == id && !item.dead)
    }

    pub fn spawn_item(&mut self, kind: Kind, x: f64, y: f64) -> usize {
        let vx = self.randf_range(-1.0, 1.0);
        self.items.push(Item {
            id: self.next_item,
            kind,
            x,
            y,
            vx,
            vy: 0.0,
            holder: None,
            fuse: -1.0,
            ammo: kind.weapon().map_or(0, |w| w.ammo),
            age: 0.0,
            spin: 0.0,
            av: 0.0,
            thrower: None,
            dead: false,
        });
        self.next_item += 1;
        self.items.len() - 1
    }

    fn maybe_drop(&mut self) {
        if self.players.is_empty() || self.time < self.drop_at {
            return;
        }
        self.drop_at = self.time + DROP_EVERY;
        let free = self.items.iter().filter(|i| i.holder.is_none()).count();
        if free >= MAX_ITEMS {
            return;
        }
        let mut kind = ITEM_KINDS[self.rng.next_u32() as usize % ITEM_KINDS.len()];
        if kind == Kind::Ball && self.items.iter().any(|i| i.kind == Kind::Ball) {
            kind = Kind::Popcorn;
        }
        let x = self.randf_range(layout::LEFT + 1.5, layout::RIGHT - 1.5);
        let it = self.spawn_item(kind, x, layout::CEILING - 0.6);
        let e = ev(Ev::Drop, self.items[it].x, self.items[it].y, None);
        self.events.push(e);
    }

    /// A free item as the Godot lobby moved it (the parity test).
    fn classic_item(&mut self, it: usize) {
        let item = &mut self.items[it];
        let ball = item.kind == Kind::Ball;
        item.vy = (item.vy - GRAVITY * if ball { 0.55 } else { 1.0 } * DT).max(-TERMINAL);
        let old_y = item.y;
        item.x += item.vx * DT;
        item.y += item.vy * DT;
        item.spin += item.vx * DT * 2.0;
        let half = if ball { 0.45 } else { 0.25 };
        if item.x - half < layout::LEFT {
            item.x = layout::LEFT + half;
            item.vx = item.vx.abs() * 0.7;
        }
        if item.x + half > layout::RIGHT {
            item.x = layout::RIGHT - half;
            item.vx = -item.vx.abs() * 0.7;
        }
        if item.y + half * 2.0 > layout::CEILING {
            item.y = layout::CEILING - half * 2.0;
            item.vy = -item.vy.abs() * 0.5;
        }
        let top = landing(item.x, half, old_y, item.y, false);
        if item.vy <= 0.0 && top > f64::NEG_INFINITY {
            item.y = top;
            if item.vy.abs() > 3.0 {
                item.vy = -item.vy * if ball { 0.72 } else { 0.3 };
                let mut e = ev(Ev::Bounce, item.x, item.y, None);
                e.kind = Some(item.kind);
                self.events.push(e);
            } else {
                item.vy = 0.0;
            }
            item.vx *= if ball { 0.97 } else { 0.8 };
        }
    }

    /// A free item: thrown, kicked or dropped, under gravity, rolling or
    /// sliding (see [`Body`]).
    fn free_item(&mut self, it: usize) {
        let item = &mut self.items[it];
        let body = Body::of(item.kind);
        let half = body.radius;
        item.vy = (item.vy - GRAVITY * body.gravity * DT).max(-TERMINAL);
        item.vx *= 1.0 - AIR_DRAG * DT;
        let old_y = item.y;
        item.x += item.vx * DT;
        item.y += item.vy * DT;
        item.spin += item.av * DT;
        if item.x - half < layout::LEFT {
            item.x = layout::LEFT + half;
            item.vx = item.vx.abs() * body.restitution;
        }
        if item.x + half > layout::RIGHT {
            item.x = layout::RIGHT - half;
            item.vx = -item.vx.abs() * body.restitution;
        }
        if item.y + half * 2.0 > layout::CEILING {
            item.y = layout::CEILING - half * 2.0;
            item.vy = -item.vy.abs() * 0.5;
        }
        let top = landing(item.x, half, old_y, item.y, false);
        if item.vy <= 0.0 && top > f64::NEG_INFINITY {
            item.y = top;
            // The floor pushes back with this much impulse (per mass):
            // the impact, or just holding it up against gravity.
            let impact = -item.vy;
            let normal = if impact > 1.2 { impact * (1.0 + body.restitution) } else { GRAVITY * body.gravity * DT };
            if impact > 1.2 {
                item.vy = impact * body.restitution;
                if impact > 3.0 {
                    let mut e = ev(Ev::Bounce, item.x, item.y, None);
                    e.kind = Some(item.kind);
                    self.events.push(e);
                }
            } else {
                item.vy = 0.0;
            }
            let item = &mut self.items[it];
            body.contact(item, normal);
        }
    }

    fn step_items(&mut self) {
        for it in 0..self.items.len() {
            if self.items[it].dead {
                continue;
            }
            self.items[it].age += DT;
            if let Some(holder) = self.items[it].holder.clone() {
                match self.index(&holder) {
                    None => self.items[it].holder = None,
                    Some(pi) => {
                        let p = &self.players[pi];
                        let item = &mut self.items[it];
                        item.x = p.x + p.facing * 0.62;
                        item.y = p.y + 0.7;
                        item.vx = p.vx;
                        item.vy = p.vy;
                        item.age = 0.0;
                        item.spin = 0.0;
                        item.av = 0.0;
                    }
                }
            } else {
                let ball = self.items[it].kind == Kind::Ball;
                let half = if self.classic_items { 0.45 } else { Body::of(Kind::Ball).radius };
                if self.classic_items { self.classic_item(it); } else { self.free_item(it); }
                if ball {
                    for p in &self.players {
                        let item = &mut self.items[it];
                        if (p.x - item.x).abs() < HALF_W + half && item.y < p.y + HEIGHT && item.y + half * 2.0 > p.y {
                            if (item.vx - p.vx).abs() < 3.0 || signf(item.x - p.x) != signf(item.vx - p.vx) {
                                item.vx = signf(item.x - p.x) * (p.vx.abs() * 1.3).max(4.0);
                                item.vy = item.vy.max(3.5);
                            }
                        }
                    }
                }
            }
            let item = &mut self.items[it];
            if item.kind == Kind::Bomb && item.fuse >= 0.0 {
                item.fuse -= DT;
                if item.fuse <= 0.0 {
                    self.explode(it);
                }
            }
            let item = &mut self.items[it];
            if item.holder.is_none() && item.age > ITEM_LIFE && item.kind != Kind::Ball {
                item.dead = true;
            }
        }
        for item in &self.items {
            if item.dead {
                for p in &mut self.players {
                    if p.item == Some(item.id) {
                        p.item = None;
                    }
                }
            }
        }
        self.items.retain(|i| !i.dead);
    }

    fn explode(&mut self, it: usize) {
        self.items[it].dead = true;
        let (x, y) = (self.items[it].x, self.items[it].y + 0.25);
        self.blast(x, y, Some(it));
    }

    /// Bombs and bazooka rockets: everyone and everything nearby flies.
    fn blast(&mut self, x: f64, y: f64, source: Option<usize>) {
        self.events.push(ev(Ev::Explode, x, y, None));
        for i in 0..self.players.len() {
            let p = &self.players[i];
            let (ox, oy) = ((p.x - x) as f32, (p.y + 0.85 - y) as f32);
            let distance = length32(ox as f64, oy as f64) as f64;
            if distance < 2.6 {
                let push = (1.0 - distance / 2.6) * 16.0 + 4.0;
                let (dx, dy) = if distance > 0.05 { normalized(ox, oy) } else { (0.0, -1.0) };
                self.knock(i, dx as f64 * push, (dy as f64).abs() * push * 0.6 + 6.0, 0.6);
            }
        }
        for o in 0..self.items.len() {
            let other = &mut self.items[o];
            if Some(o) == source || other.holder.is_some() {
                continue;
            }
            let (ox, oy) = ((other.x - x) as f32, other.y + 0.25 - y);
            if (length32(ox as f64, oy) as f64) < 2.6 {
                other.vx += signf(ox as f64) * 9.0;
                other.vy += 7.0;
                if other.kind == Kind::Bomb && other.fuse < 0.0 {
                    other.fuse = 0.25;
                }
            }
        }
    }

    fn step_shots(&mut self) {
        for s in 0..self.shots.len() {
            let shot = &mut self.shots[s];
            if shot.gravity > 0.0 {
                shot.vy -= GRAVITY * shot.gravity * DT;
                shot.y += shot.vy * DT;
            }
            shot.x += shot.vx * DT;
            shot.life -= DT;
            if shot.x < layout::LEFT || shot.x > layout::RIGHT || shot.y < 0.0 {
                shot.life = 0.0;
                if shot.blast {
                    let (x, y) = (shot.x.clamp(layout::LEFT, layout::RIGHT), shot.y.max(0.3));
                    self.blast(x, y, None);
                }
            } else if shot.life <= 0.0 && shot.blast {
                let (x, y) = (shot.x, shot.y);
                self.blast(x, y, None);
            }
            for j in 0..self.players.len() {
                let shot = &self.shots[s];
                let p = &self.players[j];
                if p.id == shot.owner || shot.life <= 0.0 {
                    continue;
                }
                if (p.x - shot.x).abs() < HALF_W + 0.1 && shot.y > p.y && shot.y < p.y + HEIGHT {
                    let shot = &mut self.shots[s];
                    shot.life = 0.0;
                    let shot = shot.clone();
                    if shot.blast {
                        self.blast(shot.x, shot.y, None);
                    } else {
                        self.knock(j, signf(shot.vx) * shot.knock, shot.lift, shot.stun);
                        let mut e = ev(Ev::Hit, shot.x, shot.y, Some(&self.players[j].id));
                        e.kind = Some(shot.kind);
                        self.events.push(e);
                    }
                }
            }
            for item in &mut self.items {
                let shot = &mut self.shots[s];
                let (dx, dy) = (item.x - shot.x, item.y + 0.45 - shot.y);
                if item.kind == Kind::Ball && (length32(dx, dy) as f64) < 0.55 {
                    item.vx += signf(shot.vx) * 4.0;
                    item.vy += 3.0;
                    shot.life = 0.0;
                }
            }
        }
        self.shots.retain(|s| s.life > 0.0);
    }

    /// Whether this player stands still on the floor zone centred at x.
    pub fn grounded_near(&self, id: &str, x: f64, half: f64, floor_y: f64) -> bool {
        self.player(id)
            .is_some_and(|p| p.grounded && (p.y - floor_y).abs() < 0.1 && (p.x - x).abs() <= half)
    }

    /// `RandomNumberGenerator.randf_range`, which works in single floats.
    fn randf_range(&mut self, from: f64, to: f64) -> f64 {
        self.rng.randf_range_f32(from as f32, to as f32) as f64
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

/// Highest surface crossed while falling from old_y to new_y, or -inf.
/// Air slows everything a little (per second).
const AIR_DRAG: f64 = 0.08;

/// How a free item moves: a rolling sphere (bomb, ball) or a tumbling box
/// (the weapons and the rest), with Coulomb friction at the contact point.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub radius: f64,
    pub gravity: f64,
    pub restitution: f64,
    pub friction: f64,
    /// Moment of inertia over m·r² (2/5 a solid sphere, 2/3 a hollow one).
    pub inertia: f64,
    /// Rolling resistance (m/s²); boxes do not roll.
    pub rolling: Option<f64>,
}

impl Body {
    pub fn of(kind: Kind) -> Body {
        match kind {
            Kind::Ball => Body { radius: 0.45, gravity: 0.55, restitution: 0.72, friction: 0.5, inertia: 2.0 / 3.0, rolling: Some(0.35) },
            Kind::Bomb => Body { radius: 0.25, gravity: 1.0, restitution: 0.42, friction: 0.6, inertia: 0.4, rolling: Some(0.9) },
            _ => Body { radius: 0.25, gravity: 1.0, restitution: 0.25, friction: 0.55, inertia: 1.0 / 6.0, rolling: None },
        }
    }

    /// Friction from one contact with the floor carrying `normal` impulse
    /// per mass: it brings the contact point's slip (vx + av·r) towards
    /// zero, trading speed for spin, and a sphere then rolls.
    fn contact(&self, item: &mut Item, normal: f64) {
        let r = self.radius;
        match self.rolling {
            Some(resistance) => {
                let slip = item.vx + item.av * r;
                // Impulse that stops the slip, at most mu·N.
                let stop = -slip / (1.0 + 1.0 / self.inertia);
                let j = stop.clamp(-self.friction * normal, self.friction * normal);
                item.vx += j;
                item.av += j / (self.inertia * r);
                // Rolling resistance slows a rolling ball to a stop.
                if (item.vx + item.av * r).abs() < 0.05 {
                    let v = move_toward(item.vx, 0.0, resistance * DT);
                    item.vx = v;
                    item.av = -v / r;
                }
            }
            None => {
                // A box slides to a stop and topples flat: its spin dies on
                // the floor and its angle settles to the nearest side.
                item.vx = move_toward(item.vx, 0.0, self.friction * normal);
                item.av *= if normal > GRAVITY * DT * 1.5 { 0.35 } else { 0.85 };
                let flat = (item.spin / std::f64::consts::PI).round() * std::f64::consts::PI;
                item.spin += (flat - item.spin) * 0.25;
            }
        }
    }
}

pub fn landing(x: f64, half: f64, old_y: f64, new_y: f64, dropping: bool) -> f64 {
    if new_y <= 0.0 {
        return 0.0;
    }
    let mut best = f64::NEG_INFINITY;
    if dropping {
        return best;
    }
    for plat in PLATFORMS {
        if x + half * 0.6 < plat.x0 || x - half * 0.6 > plat.x1 {
            continue;
        }
        if old_y >= plat.top - 0.001 && new_y <= plat.top && plat.top > best {
            best = plat.top;
        }
    }
    best
}
