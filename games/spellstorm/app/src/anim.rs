//! How the bodies move: Wizard._visuals and _limbs, Enemy._animate,
//! _windup_pose and _flinch. Only looks: the sim keeps everything that
//! plays (where the staff points, which way the body faces).
use crate::kits::Kit;
use glam::{EulerRot, Mat3, Mat4, Quat, Vec3};
use spellstorm::actor::{Actor, HURT_TIME};
use spellstorm::enemy::{Attack, Enemy};
use spellstorm::fx::Death;
use spellstorm::wizard::{swing_reach, Wizard, ROLL_TIME, SWING_COOLDOWN};
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

const SWORD_HAND: f32 = 0.62;

/// Godot's Node3D rotation (Euler YXZ) as a basis.
pub fn euler(r: Vec3) -> Mat3 {
    Mat3::from_euler(EulerRot::YXZ, r.y, r.x, r.z)
}

/// A node's local transform as Godot builds it from position, rotation, scale.
pub fn trs(pos: Vec3, rot: Mat3, scale: Vec3) -> Mat4 {
    Mat4::from_cols((rot.x_axis * scale.x).extend(0.0), (rot.y_axis * scale.y).extend(0.0), (rot.z_axis * scale.z).extend(0.0), pos.extend(1.0))
}

/// Per-node overrides of a kit's rest transforms.
pub type Pose = HashMap<usize, Mat4>;

/// Node `i`'s transform in the kit's space under `pose`.
pub fn posed(kit: &Kit, pose: &Pose, i: usize) -> Mat4 {
    let local = |j: usize| pose.get(&j).copied().unwrap_or(kit.nodes[j].xf);
    kit.tree.model_transform(i, local)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_angle(from: f32, to: f32, w: f32) -> f32 {
    let d = (to - from).rem_euclid(TAU);
    let d = (2.0 * d).rem_euclid(TAU) - d;
    from + d * w
}

/// The bits of a wizard's look that build up over frames.
#[derive(Clone, Debug)]
pub struct WizardAnim {
    walk: f32,
    stride: f32,
    sword_pos: Vec3,
    sword_rot: Vec3,
    body_scale: Vec3,
    body_yaw: f32,
}

impl Default for WizardAnim {
    fn default() -> Self {
        WizardAnim { walk: 0.0, stride: 0.0, sword_pos: Vec3::new(-0.36, 0.78, 0.24), sword_rot: Vec3::ZERO, body_scale: Vec3::ONE, body_yaw: 0.0 }
    }
}

impl WizardAnim {
    /// The wizard's pose this frame (Wizard._visuals and _limbs).
    pub fn pose(&mut self, kit: &Kit, a: &Actor, w: &Wizard, dt: f32) -> Pose {
        let node = |k: &str| kit.tree.node(k);
        let mut pose = Pose::new();
        let moving = a.vel.length() > 0.5;
        self.walk += dt * if moving { 9.0 } else { 0.0 };
        let target_yaw = w.aim_dir.x.atan2(w.aim_dir.z);
        // The sim turns the body (it aims the staff); between ticks it is
        // smoothed the same way here.
        self.body_yaw = lerp_angle(self.body_yaw, w.body_yaw, (30.0 * dt).min(1.0));
        let mut body_pos = Vec3::new(0.0, if moving { self.walk.sin().abs() * 0.07 } else { 0.0 }, 0.0);
        let mut body_rot = Vec3::new(0.0, self.body_yaw, if moving { self.walk.sin() * 0.05 } else { 0.0 });
        // Staff: the sim's pitch and reach.
        if let Some(s) = node("staff") {
            let rest = kit.nodes[s].xf.w_axis.truncate();
            // Shaken (R3): it rattles side to side as well.
            let k = (w.shake_t / spellstorm::wizard::SHAKE_TIME) as f32;
            let roll = if k > 0.0 { ((1.0 - k) * 9.0 * std::f32::consts::TAU).sin() * 0.45 * k } else { 0.0 };
            pose.insert(s, trs(Vec3::new(rest.x, rest.y, w.staff_z), euler(Vec3::new(w.staff_rx, 0.0, roll)), Vec3::ONE));
        }
        let k12 = 1.0 - (-12.0 * dt).exp();
        let sweep = if w.swing_t > 0.0 { 1.0 - (w.swing_t / SWING_COOLDOWN) as f32 } else { -1.0 };
        if sweep >= 0.0 {
            let e = 1.0 - (1.0 - (sweep * 1.6).min(1.0)).powi(3);
            self.sword_pos = self.sword_pos.lerp(Vec3::new(-0.15, 0.95, 0.5), 1.0 - (-30.0 * dt).exp());
            self.sword_rot = Vec3::new(PI * 0.5, lerp(-1.9, 1.9, e), 0.0);
            body_rot.y = target_yaw + lerp(-0.6, 0.6, e);
            self.body_yaw = body_rot.y;
        } else {
            let rest = Vec3::new(-0.4, 0.74, 0.12 - self.walk.sin() * self.stride * 0.08);
            let length = w.enchant_reach().max(swing_reach()) - SWORD_HAND + 0.05;
            let dip = ((rest.y - 0.12) / length).clamp(0.0, 1.0).asin() * 0.9;
            self.sword_pos = self.sword_pos.lerp(rest, k12);
            self.sword_rot = self.sword_rot.lerp(Vec3::new(PI * 0.5 + dip, 0.0, 0.32 + self.walk.sin() * self.stride * 0.05), k12);
        }
        let sword = node("sword");
        if let Some(s) = sword {
            pose.insert(s, trs(self.sword_pos, euler(self.sword_rot), Vec3::ONE));
        }
        // The roll: a tucked somersault round the middle of the body.
        if w.roll_t > 0.0 {
            let k = 1.0 - (w.roll_t / ROLL_TIME) as f32;
            let ang = TAU * (1.0 - (1.0 - k).powi(2));
            let mid = 0.55;
            let tuck = (k * PI).sin();
            body_rot = Vec3::new(ang, w.roll_dir.x.atan2(w.roll_dir.z), 0.0);
            let off = Vec3::new(0.0, mid * (1.0 - ang.cos()), -mid * ang.sin());
            body_pos = Quat::from_rotation_y(body_rot.y) * off + Vec3::Y * 0.5 * (ang * 0.5).sin();
            self.body_scale = Vec3::new(1.0 + 0.08 * tuck, 1.0 - 0.35 * tuck, 1.0 + 0.08 * tuck);
        } else {
            self.body_scale = self.body_scale.lerp(Vec3::ONE, (14.0 * dt).min(1.0));
        }
        if let Some(b) = node("body") {
            pose.insert(b, trs(body_pos, euler(body_rot), self.body_scale));
        }
        // Legs step, arms reach for what their hands hold.
        self.stride = lerp(self.stride, if moving && w.roll_t <= 0.0 { 1.0 } else { 0.0 }, 1.0 - (-10.0 * dt).exp());
        let step = self.walk.sin() * self.stride;
        let tuck = if w.roll_t > 0.0 { ((1.0 - (w.roll_t / ROLL_TIME) as f32) * PI).sin() } else { 0.0 };
        for (leg, sign) in [("leg_l", 1.0), ("leg_r", -1.0)] {
            if let Some(l) = node(leg) {
                let rest = kit.nodes[l].xf.w_axis.truncate();
                pose.insert(l, trs(rest, euler(Vec3::new(sign * step * 0.6 - tuck * 1.2, 0.0, 0.0)), Vec3::ONE));
            }
        }
        for (arm, held) in [("arm_r", "staff"), ("arm_l", "sword")] {
            let (Some(a), Some(h)) = (node(arm), node(held)) else { continue };
            let arm_at = kit.nodes[a].xf.w_axis.truncate();
            let held_at = pose.get(&h).map(|m| m.w_axis.truncate()).unwrap_or(kit.nodes[h].xf.w_axis.truncate());
            let to_hand = held_at - arm_at;
            if to_hand.length() > 0.01 {
                let q = Quat::from_rotation_arc(Vec3::NEG_Y, to_hand.normalize());
                pose.insert(a, trs(arm_at, Mat3::from_quat(q), Vec3::ONE));
            }
        }
        pose
    }
}

/// A monster's frame-to-frame look.
#[derive(Clone, Debug, Default)]
pub struct EnemyAnim {
    walk: f32,
    rot_x: f32,
    scale: f32,
    spin: f32,
    /// Seconds since it died.
    pub dead_t: Option<f32>,
    /// Thrown apart into pieces (the view's debris has them now).
    pub scattered: bool,
    /// Gone limp: its slot among the view's fallen monsters (ragdoll.rs).
    pub doll: Option<usize>,
    /// Its fall was decided (a ragdoll, or its own way when none was free).
    pub fell: bool,
    /// How far a burrower's body is sunk into the floor (0 up, 1 under).
    sunk: f32,
    /// Until the next puff of dust over a burrower.
    pub dust: f32,
    /// Its motion-captured body (rig.rs), and the hurt_t it last saw.
    pub body: body::Gait,
    pub hurt_seen: f64,
}

/// How a monster's body goes (Enemy._death), by its model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fall {
    /// Falls apart into pieces flung `power` sideways and `up` upward,
    /// lying `life` seconds before they sink.
    Scatter { power: f32, up: f32, life: f32 },
    /// Swells for its fuse, then bursts into pieces.
    BlowUp,
    /// Slumps flat into the floor (burnt robes, an emptied necromancer).
    Slump { time: f32 },
    /// Unravels upward into mist.
    Unravel,
    /// Topples like a felled tree, lies, then sinks.
    Topple,
}

pub fn fall(model: &str, big: f32) -> Fall {
    match model {
        "skeleton" => Fall::Scatter { power: 1.8 * big, up: 3.0, life: 5.0 },
        "ghoul" => Fall::Scatter { power: 2.6, up: 4.0, life: 3.0 },
        "frost_brute" => Fall::Scatter { power: 3.5 * big, up: 5.0, life: 3.0 },
        "lurker" => Fall::Scatter { power: 2.4, up: 3.0, life: 3.0 },
        // A bat drops out of the air; crystal shells crack apart; a
        // toadstool goes to pulp in its own spore cloud.
        "bat" => Fall::Scatter { power: 1.2, up: 1.0, life: 2.0 },
        "crawler" => Fall::Scatter { power: 2.8 * big, up: 3.5, life: 3.0 },
        "shambler" => Fall::Scatter { power: 2.0, up: 2.5, life: 2.0 },
        "drowned" | "toad" => Fall::Scatter { power: 1.5 * big, up: 2.0, life: 0.6 },
        "bile" => Fall::Scatter { power: 5.0 * big, up: 5.0, life: 2.0 },
        "imp" | "sprite" | "watcher" | "demon" => Fall::BlowUp,
        "necromancer" => Fall::Slump { time: 0.7 },
        "cultist" | "hellhound" => Fall::Slump { time: 0.9 },
        "wraith" => Fall::Unravel,
        _ => Fall::Topple,
    }
}

/// The blow-up's pieces (Enemy._blow_up's scatter).
pub const BLOWN: Fall = Fall::Scatter { power: 6.0, up: 6.0, life: 1.5 };

fn ease_in(k: f32) -> f32 {
    let k = k.clamp(0.0, 1.0);
    k * k
}

impl EnemyAnim {
    /// The model's transform under the monster's root (Enemy._animate).
    /// `rigged`: its clips move the body, so the whole-body bob, wind-up lean
    /// and toppling are left to them.
    pub fn model(&mut self, base: f32, a: &Actor, e: &Enemy, time: f32, seed: u32, dt: f32, rigged: bool) -> Mat4 {
        let cfg = e.cfg();
        if self.scale == 0.0 {
            self.scale = base;
        }
        let moving = a.vel.length() > 0.4;
        self.walk += dt * if moving { 8.0 } else { 0.0 } * a.speed as f32 / 3.0;
        if !a.alive {
            *self.dead_t.get_or_insert(0.0) += dt;
        }
        let bob = if rigged { 0.0 } else { 1.0 };
        let mut pos = Vec3::new(0.0, (self.walk.sin()).abs() * 0.06 * bob, 0.0);
        let mut rot = Vec3::new(self.rot_x, 0.0, self.walk.sin() * 0.06 * bob);
        let mut scale = Vec3::splat(self.scale);
        let winding = (e.wind > 0.0 || e.boss_wind > 0.0) && !rigged;
        if winding {
            let k = if e.wind > 0.0 { 1.0 - (e.wind / e.wind_time()) as f32 } else { 1.0 - (e.boss_wind / e.boss_wind_time.max(1e-3)) as f32 };
            let ee = k * k * (3.0 - 2.0 * k);
            let shake = (time * 1000.0 / 22.0).sin() * 0.04 * smoothstep(0.6, 1.0, k);
            let attack = if cfg.boss && e.wind <= 0.0 { Attack::Slam } else { cfg.attack };
            match attack {
                Attack::Slam => {
                    rot.x = -0.35 * ee;
                    pos.y = 0.35 * ee;
                    scale = Vec3::splat(base) * Vec3::new(1.0 + 0.08 * ee, 1.0 + 0.18 * ee, 1.0 + 0.08 * ee);
                }
                Attack::Dash => {
                    rot.x = 0.35 * ee;
                    scale = Vec3::splat(base) * Vec3::new(1.0 + 0.12 * ee, 1.0 - 0.25 * ee, 1.0 + 0.12 * ee);
                }
                Attack::Ranged | Attack::Lob => rot.x = -0.25 * ee,
                _ => {
                    rot.x = -0.6 * ee;
                    scale = Vec3::splat(base) * Vec3::new(1.0, 1.0 + 0.1 * ee, 1.0);
                }
            }
            rot.z += shake;
            self.rot_x = rot.x;
        } else {
            self.rot_x = lerp(self.rot_x, 0.0, (10.0 * dt).min(1.0));
            rot.x = self.rot_x;
            scale = scale.lerp(Vec3::splat(base), (12.0 * dt).min(1.0));
        }
        self.scale = scale.x;
        let model = cfg.model.unwrap_or(cfg.key);
        let jitter = (seed % 97) as f32;
        if cfg.key == "sprite" {
            pos.y = ((time + (seed % 53) as f32) * 3.3).sin() * 0.15;
            self.spin += dt * 6.0;
            rot.y = self.spin;
        } else if cfg.key == "wraith" || model == "wraith" {
            pos.y = 0.3 + (time * 1000.0 / 400.0).sin() * 0.1;
        } else if cfg.key == "bat" {
            // Flutters: bobs and rocks as it hangs in the air.
            let tt = time * 9.0 + jitter;
            pos.y = 0.1 + tt.sin().abs() * 0.12;
            rot.z = (tt * 0.5).sin() * 0.12;
        } else if cfg.key == "watcher" {
            let tt = time + jitter;
            pos.y = 0.25 + (tt * 2.1).sin() * 0.15;
            rot.z = (tt * 1.3).sin() * 0.08;
        }
        // A hit snaps the body away from where it came.
        if a.hurt_t > 0.0 && a.alive {
            let u = 1.0 - (a.hurt_t / HURT_TIME) as f32;
            let k = ((u * 1.6).min(1.0) * PI * 0.5).sin() * (1.0 - u) - 0.15 * (u * PI).sin() * u;
            let mut local = Quat::from_rotation_y(-a.yaw) * a.hurt_dir;
            local.y = 0.0;
            let k = k * a.hurt_k as f32;
            pos += local * 0.18 * k;
            rot.x += local.z * 0.45 * k;
            rot.z += -local.x * 0.45 * k;
        }
        // Dying by an element goes that element's way (remains.gd, Joep: by
        // the element): lightning jolts it before it drops, void lifts it,
        // stretches it thin and swallows it, wind throws it tumbling away,
        // frost holds it still until it shatters (the view's debris), and a
        // huge blow bursts it (the same). Fire and plain blows keep the
        // model's own fall.
        if let (Some(t), Some(how)) = (self.dead_t, e.how) {
            let b = Vec3::splat(base);
            let mut away = Quat::from_rotation_y(-a.yaw) * a.last_dir;
            away.y = 0.0;
            let away = away.normalize_or(Vec3::Z);
            let sink = -1.95 * ((t - 2.6) / 1.4).clamp(0.0, 1.0);
            match how {
                Death::Shock => {
                    let j = (1.0 - t / 0.8).max(0.0);
                    pos = Vec3::new((t * 97.0).sin() * 0.07 * j, 0.0, (t * 83.0).cos() * 0.07 * j);
                    rot = Vec3::new(0.0, (t * 61.0).sin() * 0.2 * j, (t * 71.0).sin() * 0.35 * j);
                    let k = ease_in((t - 0.8) / 0.45);
                    rot.x = -PI / 2.0 * k;
                    pos.y += 0.15 * k + sink;
                    return trs(pos, euler(rot), b);
                }
                Death::Void => {
                    let k1 = smoothstep(0.0, 0.35, t);
                    let k2 = ease_in((t - 0.35) / 0.4);
                    let s = Vec3::ONE.lerp(Vec3::new(0.45, 1.5, 0.45), k1) * (1.0 - 0.98 * k2);
                    let s = if t >= 0.8 { Vec3::ZERO } else { b * s };
                    return trs(Vec3::Y * (0.5 * k1 + 1.2 * k2), euler(Vec3::new(0.0, 9.0 * k2, 0.0)), s);
                }
                Death::Blown => {
                    let f = t.min(0.5);
                    let up = (5.0 * f - 10.0 * f * f).max(0.0);
                    let slide = 7.0 * f + 1.5 * (1.0 - (-(t - 0.5).max(0.0) * 4.0).exp());
                    // Tumbles one and a quarter turns, landing on its back.
                    let turn = -(PI / 2.0 + TAU) * (t / 0.5).min(1.0);
                    let pos = away * slide + Vec3::Y * (up + 0.15 * (t / 0.5).min(1.0) + sink);
                    let tilt = Quat::from_axis_angle(Vec3::Y.cross(away).normalize_or(Vec3::X), -turn);
                    return Mat4::from_translation(pos) * Mat4::from_quat(tilt) * Mat4::from_scale(b);
                }
                Death::Freeze => return trs(Vec3::ZERO, euler(Vec3::ZERO), b),
                Death::Gore => return trs(Vec3::ZERO, euler(Vec3::ZERO), b),
                _ => {}
            }
        }
        if let Some(t) = self.dead_t {
            let big = if cfg.model.is_some() { cfg.size as f32 } else { 1.0 };
            let b = Vec3::splat(base);
            match fall(model, big) {
                Fall::Scatter { .. } => {}
                Fall::BlowUp => scale = b * (1.0 + 0.45 * ease_in(t / e.fuse.max(0.05) as f32)),
                Fall::Slump { .. } if rigged => pos.y = -0.4 * ((t - 3.0) / 1.0).clamp(0.0, 1.0),
                Fall::Slump { time } => {
                    scale = b.lerp(b * Vec3::new(1.25, 0.08, 1.25), ease_in(t / time));
                    pos.y = -0.4 * ((t - time - 1.5) / 1.0).clamp(0.0, 1.0);
                }
                Fall::Unravel => {
                    let k = ease_in(t / 0.6);
                    scale = b.lerp(b * Vec3::new(0.1, 2.2, 0.1), k);
                    pos.y = 1.5 * k;
                    if t >= 0.6 {
                        scale = Vec3::ZERO;
                    }
                }
                Fall::Topple if rigged => {
                    pos.y = -1.95 * ((t - 2.6) / 1.4).clamp(0.0, 1.0);
                    scale = b;
                }
                Fall::Topple => {
                    let k = ease_in(t / 0.5);
                    rot = Vec3::new(-PI / 2.0 * k, rot.y, 0.0);
                    pos.y = 0.15 * k - 1.95 * ((t - 2.0) / 1.4).clamp(0.0, 1.0);
                    scale = b;
                }
            }
        }
        // A burrower under the floor: only the ridge of its back breaks the
        // ground (Godot's dirt mound), heaving up as it winds up to burst
        // out; it bursts up from deep down (Enemy._erupt).
        if a.alive {
            if e.under {
                self.sunk = (self.sunk + dt / 0.35).min(1.0);
                let ridge = -0.74 + 0.3 * e.heave + ((time * 1000.0 / 90.0).sin()).abs() * 0.04;
                pos.y = lerp(pos.y, ridge, self.sunk * self.sunk);
                rot.x = 0.0;
                rot.z = (time * 37.0).sin() * 0.05 * e.heave;
            } else if e.rise_t > 0.0 {
                self.sunk = 0.0;
                let k = 1.0 - (e.rise_t / 0.22).clamp(0.0, 1.0) as f32;
                // TRANS_BACK, EASE_OUT: overshoots a little at the top.
                let c = 1.70158;
                let back = 1.0 + (c + 1.0) * (k - 1.0).powi(3) + c * (k - 1.0).powi(2);
                pos.y += e.rise_from * (1.0 - back);
            } else {
                self.sunk = 0.0;
            }
        }
        trs(pos, euler(rot), Vec3::new(scale.x, scale.y, scale.z))
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
