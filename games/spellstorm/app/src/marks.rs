//! What the wizards carry and what holds the way: the light from each staff's
//! gem (Wizard._staff_light), the sword's arc on the ground while it swings
//! (slash.gdshader), and the walls of mist that shut the way on until the
//! fight there is done (ward.gdshader).
use crate::fx::{uv, Cell, IMAGE, LIGHT, LIT, SLICE, SLICES};
use crate::look::linear3;
use glam::Vec3;
use render::headlights::{LocalLight, SpotLight};
use render::sprites::Sprite;
use spellstorm::fx::{el_c, C};
use spellstorm::wizard::{self, swing_reach, Wizard, NOVA_HOLD, SWING_COOLDOWN};
use spellstorm::enemy::Attack;
use spellstorm::World;
use std::collections::HashMap;

/// Wizard.HERO_LIGHT and GEM_WARM.
const HERO_LIGHT: f32 = 3.2;
const GEM_WARM: [f32; 3] = [1.0, 0.8, 0.55];
/// ward.gdshader's colour.
const MIST: [f32; 3] = [0.85, 0.8, 0.65];

#[derive(Default)]
pub struct Marks {
    /// Each wizard's gem brightness, eased (Wizard._shine), by seat.
    shine: HashMap<usize, f32>,
    pub sprites: Vec<Sprite>,
    /// Each staff's gem light as a whole sphere: what lights the haze.
    pub lights: Vec<LocalLight>,
    /// The same light on surfaces: a shadowed spot straight down from the
    /// gem (Godot's shadowed omni; the engine shadows spots only, up to
    /// 1.3 rad) and an unshadowed rest for everything outside that cone.
    pub spots: Vec<SpotLight>,
    pub spill: Vec<LocalLight>,
    /// Each wizard's gem on the posed staff (by wizard index), from the view.
    pub gems: Vec<Option<Vec3>>,
}

/// The shadowed cone under each gem: full inside INNER, gone past OUTER,
/// where the unshadowed spill takes over with the mirrored edge.
const SPOT_INNER: f32 = 0.9;
const SPOT_OUTER: f32 = 1.3;
/// Godot's omni_attenuation of the staff light.
pub const STAFF_FALLOFF: f32 = 1.1;

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A little value noise, steady in `x` and `t`.
fn hash(n: f32) -> f32 {
    ((n * 12.9898).sin() * 43758.547).fract().abs()
}

fn noise(x: f32) -> f32 {
    let (i, f) = (x.floor(), x.fract());
    let k = f * f * (3.0 - 2.0 * f);
    hash(i) + (hash(i + 1.0) - hash(i)) * k
}

impl Marks {
    pub fn update(&mut self, w: &World, time: f32, dt: f32) {
        self.sprites.clear();
        self.lights.clear();
        self.spots.clear();
        self.spill.clear();
        for (i, &id) in w.wizards.iter().enumerate() {
            let Some(a) = w.actor(id) else { continue };
            let wz = w.wiz(id);
            let tip = self.gems.get(i).copied().flatten().unwrap_or_else(|| wizard::staff_tip(a.pos, &wz));
            if a.alive {
                self.staff(&wz, a.hp as f32 / a.max_hp.max(1.0) as f32, tip, time, dt);
                self.cast_mark(&wz, time);
                if wz.swing_t > 0.0 {
                    self.swing(a.pos, &wz);
                }
            } else {
                self.shine.remove(&wz.seat);
            }
        }
        for wd in &w.run.wards {
            self.ward(wd.z, wd.lo, wd.hi, wd.open, time);
        }
        for &id in w.wizards.iter().chain(&w.enemies) {
            if let Some(a) = w.actor(id) {
                if a.alive {
                    self.status(&a, id.id() as f32, time);
                }
            }
        }
        // A shot winding up: a ball of its element swells in front of the
        // monster, the way it will go (Enemy._warning).
        for &id in &w.enemies {
            let Ok(e) = w.ecs.get::<&spellstorm::enemy::Enemy>(id) else { continue };
            let Some(a) = w.actor(id) else { continue };
            if !a.alive || e.wind <= 0.0 || !matches!(e.cfg().attack, Attack::Ranged | Attack::Lob) {
                continue;
            }
            let k = (1.0 - e.wind / e.wind_time()) as f32;
            let at = a.pos + e.lock * (a.radius + 0.35) + Vec3::Y;
            let c = el_c(e.attack_el());
            let lin = linear3([c[0], c[1], c[2]]);
            let size = 0.5 * (0.4 + 1.1 * k);
            let inner = linear3([c[0] + (1.0 - c[0]) * 0.6, c[1] + (1.0 - c[1]) * 0.6, c[2] + (1.0 - c[2]) * 0.6]) * 0.8;
            self.sprites.push(glow(at, size, Cell::HaloOut, lin * 0.9 * 1.4));
            self.sprites.push(glow(at, size, Cell::HaloIn, inner * 0.9 * 1.4));
        }
    }

    /// What ails a body (Actor.setup_status_fx and its overlay): flames
    /// licking up while it burns, sickly bubbles while poisoned, and one
    /// tint over it, the strongest first: the white of a fresh hit, then
    /// frozen, chilled, wet, mired, poisoned.
    fn status(&mut self, a: &spellstorm::actor::Actor, seed: f32, time: f32) {
        let r = a.radius;
        if a.burn > 0.0 {
            let c = linear3(spellstorm::el::El::Fire.color()) * 1.4;
            for j in 0..24 {
                let h = seed * 3.1 + j as f32 * 1.73;
                let ph = (time / 0.6 + hash(h)).fract();
                let t = ph * 0.6;
                let ang = hash(h + 0.2) * std::f32::consts::TAU;
                let o = Vec3::new(ang.cos(), (hash(h + 0.4) - 0.5) * 1.2, ang.sin()) * r * hash(h + 0.6).sqrt();
                let p = a.pos + Vec3::Y * 0.8 + o + Vec3::Y * ((0.5 + hash(h + 0.8)) * t + t * t);
                let k = smooth(0.0, 0.15, ph) * (1.0 - smooth(0.4, 1.0, ph));
                let size = 0.35 * (1.0 - 0.5 * ph);
                let mut s = Sprite::billboard(p, [size, size * 1.3], IMAGE, uv(Cell::Flame), c * k);
                s.additive = true;
                s.alpha_cut = 0.0;
                self.sprites.push(s);
            }
        }
        if a.poison > 0.0 {
            let c = linear3(spellstorm::el::El::Poison.color()) * 1.4;
            for j in 0..10 {
                let h = seed * 5.7 + j as f32 * 2.31;
                let ph = (time / 0.9 + hash(h)).fract();
                let t = ph * 0.9;
                let ang = hash(h + 0.3) * std::f32::consts::TAU;
                let o = Vec3::new(ang.cos(), 0.0, ang.sin()) * r * hash(h + 0.5).sqrt();
                let p = a.pos + Vec3::Y * 0.9 + o + Vec3::Y * ((0.2 + 0.4 * hash(h + 0.7)) * t + 0.4 * t * t);
                let k = smooth(0.0, 0.2, ph) * (1.0 - smooth(0.6, 1.0, ph));
                let mut s = Sprite::billboard(p, [0.12, 0.12], IMAGE, uv(Cell::Drop), c * k);
                s.additive = true;
                s.alpha_cut = 0.0;
                self.sprites.push(s);
            }
        }
        // No tinted haze over the body for any status: each shows as a few
        // small particles of its own (Joep). A hit flashes the body itself
        // white (View: set_hit_flash).
        if a.frozen > 0.0 || a.chill > 0.0 {
            // Frost flakes glinting as they drift down off it.
            let n = if a.frozen > 0.0 { 10 } else { 5 };
            self.drips(a.pos, r, seed * 2.3, time, n, 1.4, Cell::Flake, linear3([0.75, 0.92, 1.0]) * 1.3, 0.09, 0.6, true);
        }
        if a.wet > 0.0 {
            // Water dripping off it: small pale drops falling to the floor.
            self.drips(a.pos, r, seed * 4.1, time, 6, 0.7, Cell::Drop, linear3([0.75, 0.88, 1.0]) * 0.9, 0.06, 4.0, true);
        }
        if a.slow > 0.0 {
            // Mud: dark heavy drops, not glowing.
            self.drips(a.pos, r * 0.8, seed * 6.7, time, 5, 0.8, Cell::Drop, linear3([0.3, 0.22, 0.12]) * LIT * 2.0, 0.08, 4.5, false);
        }
    }

    /// `n` little things leaving a body every `period` seconds, each from a
    /// random spot on it, falling from rest with `fall` m/s² and fading.
    #[allow(clippy::too_many_arguments)]
    fn drips(&mut self, pos: Vec3, r: f32, seed: f32, time: f32, n: usize, period: f32, cell: Cell, c: Vec3, size: f32, fall: f32, glow: bool) {
        for j in 0..n {
            let h = seed + j as f32 * 1.97;
            let ph = (time / period + hash(h)).fract();
            let t = ph * period;
            let ang = hash(h + 0.3) * std::f32::consts::TAU;
            let o = Vec3::new(ang.cos(), 0.0, ang.sin()) * r * (0.5 + 0.5 * hash(h + 0.5));
            let y = 0.5 + 0.9 * hash(h + 0.7) - 0.5 * fall * t * t;
            if y < 0.03 {
                continue;
            }
            let k = smooth(0.0, 0.1, ph) * (1.0 - smooth(0.75, 1.0, ph));
            let mut s = Sprite::billboard(pos + o + Vec3::Y * y, [size, size], IMAGE, uv(cell), c * k);
            s.additive = glow;
            s.alpha_cut = 0.0;
            if !glow {
                s.opacity = k;
            }
            self.sprites.push(s);
        }
    }

    /// Embers rising off the torches near the view (Arena._torch): 14
    /// sparks each, half a second up and gone.
    pub fn torches(&mut self, torches: &[Vec3], near: Vec3, time: f32) {
        let c = linear3([1.0, 0.5, 0.15]) * 1.6;
        for (i, &t) in torches.iter().enumerate() {
            if t.distance_squared(near) > 900.0 {
                continue;
            }
            for j in 0..14 {
                let h = i as f32 * 7.3 + j as f32 * 1.91;
                let ph = (time / 0.5 + hash(h)).fract();
                let s = ph * 0.5;
                let ang = hash(h + 0.2) * std::f32::consts::TAU;
                let v = 0.2 + 0.4 * hash(h + 0.4);
                let side = Vec3::new(ang.cos(), 0.0, ang.sin()) * v * 0.35 * s;
                let p = t + Vec3::Y * (0.15 + v * s + s * s) + side;
                let k = smooth(0.0, 0.15, ph) * (1.0 - smooth(0.4, 1.0, ph));
                let size = 0.24 * (1.0 - 0.5 * ph);
                let mut f = Sprite::billboard(p, [size, size * 1.3], IMAGE, uv(Cell::Flame), c * k);
                f.additive = true;
                f.alpha_cut = 0.0;
                self.sprites.push(f);
            }
        }
    }

    /// The spot a charging spell is aimed at (CastMark): runes turning on
    /// the floor and the element welling up there, faint at first and
    /// stronger as the spell fills.
    fn cast_mark(&mut self, w: &Wizard, time: f32) {
        let (Some(s), Some(at)) = (w.charging.as_ref(), w.mark) else { return };
        let k = (w.charge_t / w.charge_need.max(0.01)).clamp(0.0, 1.0) as f32;
        if s.shape == spellstorm::Shape::Overload && s.primary == Some(spellstorm::el::El::Water) {
            // A spell that rolls forward marks its lane, not a spot
            // (cast_mark.gdshader mode 1): the strip the wave will sweep.
            use spellstorm::spells::{WAVE_RUN, WAVE_START, WAVE_WIDTH};
            let dir = Vec3::new(w.aim_dir.x, 0.0, w.aim_dir.z).normalize_or(Vec3::NEG_Z);
            let side = Vec3::new(-dir.z, 0.0, dir.x);
            let len = WAVE_RUN + 2.0;
            let centre = Vec3::new(at.x, 0.05, at.z) + dir * (1.0 - WAVE_START * 0.5);
            let c = el_c(spellstorm::el::El::Water);
            let mut m = Sprite::fixed(centre, [WAVE_WIDTH, len], side, dir, IMAGE, uv(Cell::Patch), linear3([c[0], c[1], c[2]]) * 1.6);
            m.additive = true;
            m.alpha_cut = 0.0;
            m.depth_bias = 0.2;
            m.effect = crate::effects::LANE;
            m.params = [0.25 + 0.75 * k, time, len, 0.0];
            self.sprites.push(m);
            return;
        }
        let r = match s.shape {
            spellstorm::Shape::Overload => spellstorm::spells::overload_reach(s.primary),
            spellstorm::Shape::Projectile => (s.n.splash as f32 * 0.8).max(0.9),
            spellstorm::Shape::Strike => s.n.radius as f32,
            _ => 0.9,
        };
        let c = s.primary.map(el_c).unwrap_or([1.0, 0.85, 0.5, 1.0]);
        let lin = linear3([c[0], c[1], c[2]]);
        let fill = 0.25 + 0.75 * k;
        let centre = Vec3::new(at.x, 0.05, at.z);
        let (sn, cs) = (time * 0.6).sin_cos();
        let (right, up) = (Vec3::new(cs, 0.0, -sn), Vec3::new(sn, 0.0, cs));
        let edge = linear3(Vec3::from([c[0], c[1], c[2]]).lerp(Vec3::ONE, 0.2).to_array());
        let mut m = Sprite::fixed(centre, [r * 2.0, r * 2.0], right, up, IMAGE, uv(Cell::Runes), edge * fill * 0.65 * 1.4);
        m.additive = true;
        m.alpha_cut = 0.0;
        m.depth_bias = 0.2;
        self.sprites.push(m);
        // The glow welling up in the middle.
        let well = (0.35 + 0.15 * (time * 4.0).sin()) * k;
        let mut g = Sprite::fixed(centre, [r * 1.8, r * 1.8], Vec3::X, Vec3::Z, IMAGE, uv(Cell::HaloOut), lin * well * 1.4);
        g.additive = true;
        g.alpha_cut = 0.0;
        g.depth_bias = 0.2;
        self.sprites.push(g);
        // The element seeping up out of the ground in a ring.
        let speed = 0.5 + 0.7 * k;
        for j in 0..24 {
            let h = w.seat as f32 * 9.1 + j as f32 * 1.37;
            let ph = (time * speed / 0.9 + hash(h)).fract();
            let t = ph * 0.9;
            let a = hash(h + 0.3) * std::f32::consts::TAU;
            let rr = r * (0.3 + 0.55 * hash(h + 0.5).sqrt());
            let p = Vec3::new(at.x + a.cos() * rr, 0.05 + (0.4 + 0.7 * hash(h + 0.7)) * t + 0.3 * t * t, at.z + a.sin() * rr);
            let fade = smooth(0.0, 0.15, ph) * (1.0 - smooth(0.5, 1.0, ph));
            self.sprites.push(glow(p, 0.18, Cell::Dot, lin * fade * 1.4));
        }
    }

    /// The gem's warm light that each wizard carries: tinted by the queued
    /// elements, swelling while a spell winds up, dimmer after a swing,
    /// guttering when hurt badly, flashing with each shot.
    fn staff(&mut self, w: &Wizard, hp_k: f32, tip: Vec3, time: f32, dt: f32) {
        let slots = &w.queue.slots;
        let mut tint = Vec3::from(GEM_WARM);
        if !slots.is_empty() {
            let mut mix = Vec3::ZERO;
            for &e in slots {
                let c = el_c(e);
                mix += Vec3::new(c[0], c[1], c[2]);
            }
            mix /= slots.len() as f32;
            tint = tint.lerp(mix, 0.35 + 0.1 * slots.len() as f32);
        }
        let mut want = 1.0 + 0.12 * slots.len() as f32;
        let nova_k = if w.rb_t > 0.0 && w.form_open(spellstorm::grammar::Form::Nova) { (w.rb_t / NOVA_HOLD).min(1.0) as f32 } else { 0.0 };
        if w.charging.is_some() || nova_k > 0.0 {
            want += 0.45 * nova_k.max(0.6) + 0.08 * (time * 18.0).sin();
        }
        if w.swing_cd > 0.0 {
            want *= 0.65;
        }
        let low = (1.0 - hp_k / 0.35).clamp(0.0, 1.0);
        if low > 0.0 {
            let gutter = 0.5 + 0.5 * (time * 11.0 + (time * 3.7).sin() * 4.0).sin();
            want *= 1.0 - low * (0.25 + 0.3 * gutter);
            tint = tint.lerp(Vec3::new(1.0, 0.45, 0.3), low * 0.5);
        }
        let s = self.shine.entry(w.seat).or_insert(want);
        *s += (want - *s) * (8.0 * dt).min(1.0);
        // A shot's flare on the gem, as bright as the flash_light knob says.
        let shine = *s + 0.7 * w.bloom * spellstorm::knobs::flash_light() as f32 * 2.0;
        let flash = Vec3::new(w.flash_c[0], w.flash_c[1], w.flash_c[2]);
        let color = tint.lerp(flash, w.bloom * 0.7);
        let lin = linear3(color.to_array());
        let gem = LocalLight {
            position: tip + Vec3::Y * 0.15,
            direction: Vec3::NEG_Y,
            intensity_cd: lin * HERO_LIGHT * shine * LIGHT,
            range_m: 12.0,
            inner_cos: -0.999,
            outer_cos: -1.0,
            source_radius_m: 0.05,
            falloff: STAFF_FALLOFF,
        };
        self.lights.push(gem);
        self.spots.push(SpotLight {
            position: gem.position,
            direction: Vec3::NEG_Y,
            up: Vec3::NEG_Z,
            intensity_cd: gem.intensity_cd,
            range_m: gem.range_m,
            inner_angle_radians: SPOT_INNER,
            outer_angle_radians: SPOT_OUTER,
        });
        // Pointing up with the cone's edge mirrored: 1 minus the spot's.
        self.spill.push(LocalLight { direction: Vec3::Y, inner_cos: -SPOT_OUTER.cos(), outer_cos: -SPOT_INNER.cos(), ..gem });
        // The soft bloom round the gem (Fx.halo, strength 0.55 * shine).
        let size = 0.6 * (0.45 + 0.25 * shine) * 2.0;
        let k = 0.55 * shine * 1.4;
        let inner = lin.lerp(Vec3::ONE, 0.6) * 0.8;
        self.sprites.push(glow(tip, size, Cell::HaloOut, lin * k));
        self.sprites.push(glow(tip, size, Cell::HaloIn, inner * k));
    }

    /// The arc the sword sweeps, opening on the floor the way the blade
    /// goes and fading as the swing ends: it shows how far the sword reaches.
    fn swing(&mut self, pos: Vec3, w: &Wizard) {
        let sweep = 1.0 - (w.swing_t / SWING_COOLDOWN) as f32;
        // The lead runs ahead with the blade; the tail follows it the same
        // way, so the arc fades out in the direction of the cut.
        let lead = 1.0 - (1.0 - (sweep * 1.6).min(1.0)).powi(3);
        let tail = smooth(0.3, 1.0, sweep) * 1.3 - 0.15;
        let enchanted = w.enchant_time > 0.0;
        let strength = if enchanted { 0.45 } else { 1.0 };
        let sc: C = w.bolt_el.map(el_c).unwrap_or([1.0, 0.85, 0.5, 1.0]);
        let c = Vec3::new(sc[0], sc[1], sc[2]).lerp(Vec3::new(1.0, 0.9, 0.6), 0.5);
        let r1 = if enchanted { w.enchant_reach() + 0.3 } else { swing_reach() + 0.3 };
        let dir = Vec3::new(w.swing_dir.x, 0.0, w.swing_dir.z).normalize_or(Vec3::Z);
        let yaw = dir.x.atan2(dir.z);
        let half = spellstorm::wizard::SWING_ARC;
        let at = Vec3::new(pos.x, 0.06, pos.z);
        // Slice centres from one end of the arc to the other, both ends on it.
        for i in 0..=SLICES {
            // u runs the way the blade goes: it cuts from right to left.
            let u = 1.0 - i as f32 / SLICES as f32;
            let shown = smooth(0.0, 0.18, lead - u) * smooth(0.0, 0.3, u - tail);
            if shown <= 0.0 {
                continue;
            }
            let edge = (-(u - lead).abs() * 10.0).exp() * (1.0 - smooth(0.9, 1.0, lead));
            let ang = yaw + half - i as f32 * SLICE;
            let up = Vec3::new(ang.sin(), 0.0, ang.cos());
            let right = Vec3::new(up.z, 0.0, -up.x);
            let k = (1.0 + edge * 1.2) * shown * strength;
            let col = linear3(c.lerp(Vec3::ONE, 0.25 + edge * 0.5).to_array()) * 0.8 * k;
            let mut s = Sprite::fixed(at, [r1 * 2.0, r1 * 2.0], right, up, IMAGE, uv(Cell::Slice), col);
            s.additive = true;
            s.alpha_cut = 0.0;
            s.depth_bias = 0.2;
            self.sprites.push(s);
        }
    }

    /// A curtain of pale haze across the ground at `z`, thickest at the
    /// floor, curling slowly upward, with runes flickering along its foot;
    /// it thins away as the ward opens.
    fn ward(&mut self, z: f32, lo: f32, hi: f32, open: f32, time: f32) {
        if open <= 0.0 {
            return;
        }
        // One curtain across the way, its haze and runes drawn by fx.wgsl
        // `ward` (Run._ward_wall: 2 m wider than the way, 3.2 m high).
        let width = hi - lo + 2.0;
        let height = 3.2;
        let at = Vec3::new((lo + hi) * 0.5, height * 0.5, z);
        let mut s = Sprite::fixed(at, [width, height], Vec3::X, Vec3::Y, IMAGE, uv(Cell::Wisp), linear3(MIST) * LIT * 3.4);
        s.alpha_cut = 0.0;
        s.depth_bias = 0.3;
        s.effect = crate::effects::WARD;
        s.params = [width, height, open, time];
        self.sprites.push(s);
    }
}

fn glow(at: Vec3, size: f32, cell: Cell, radiance: Vec3) -> Sprite {
    let mut s = Sprite::billboard(at, [size, size], IMAGE, uv(cell), radiance);
    s.additive = true;
    s.alpha_cut = 0.0;
    s.depth_bias = 0.3;
    s
}
