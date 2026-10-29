//! What the wizards carry and what holds the way: the light from each staff's
//! gem (Wizard._staff_light), the sword's arc on the ground while it swings
//! (slash.gdshader), and the walls of mist that shut the way on until the
//! fight there is done (ward.gdshader).
use crate::fx::{uv, Cell, IMAGE, LIGHT, LIT, SLICE, SLICES};
use crate::look::linear3;
use glam::Vec3;
use render::headlights::LocalLight;
use render::sprites::Sprite;
use spellstorm::fx::{el_c, C};
use spellstorm::wizard::{self, Wizard, NOVA_HOLD, SWING_COOLDOWN, SWING_REACH};
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
    pub lights: Vec<LocalLight>,
}

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
        for &id in &w.wizards {
            let Some(a) = w.actor(id) else { continue };
            let wz = w.wiz(id);
            let tip = wizard::staff_tip(a.pos, &wz);
            if a.alive {
                self.staff(&wz, a.hp as f32 / a.max_hp.max(1.0) as f32, tip, time, dt);
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
        let shine = *s + 0.7 * w.bloom;
        let flash = Vec3::new(w.flash_c[0], w.flash_c[1], w.flash_c[2]);
        let color = tint.lerp(flash, w.bloom * 0.7);
        let lin = linear3(color.to_array());
        self.lights.push(LocalLight {
            position: tip + Vec3::Y * 0.15,
            direction: Vec3::NEG_Y,
            intensity_cd: lin * HERO_LIGHT * shine * LIGHT,
            range_m: 12.0,
            inner_cos: -0.999,
            outer_cos: -1.0,
            source_radius_m: 0.05,
            falloff: 1.1,
        });
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
        let r1 = if enchanted { w.enchant_reach() + 0.3 } else { SWING_REACH + 0.3 };
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
        let width = hi - lo + 2.0;
        let x0 = lo - 1.0;
        let step = 0.7;
        let n = (width / step).ceil() as usize + 1;
        let mist = Vec3::from(MIST);
        for i in 0..n {
            let x = x0 + i as f32 * step;
            let edge = smooth(0.0, 1.2, x - x0) * smooth(0.0, 1.2, x0 + width - x);
            if edge <= 0.0 {
                continue;
            }
            for row in 0..4 {
                let seed = i as f32 * 3.17 + row as f32 * 11.3;
                // Each wisp rises and fades over its own little cycle.
                let cycle = 3.0 + hash(seed) * 2.0;
                let ph = ((time / cycle) + hash(seed + 1.0)).fract();
                let h = row as f32 * 0.6 + ph * 1.2;
                let hk = (h / 3.2).min(1.0);
                let thick = 0.55 + 0.45 * smooth(0.25, 0.75, noise(x * 0.35 + time * 0.12 + row as f32 * 5.0));
                let fade = smooth(0.0, 0.2, ph) * (1.0 - smooth(0.6, 1.0, ph));
                // Opening thins it from the top down.
                let gone = smooth(open - 0.15, open + 0.15, hk * 0.6 + thick * 0.4);
                let a = edge * thick * fade * (1.0 - hk) * (1.0 - gone) * open * 0.8;
                if a < 0.01 {
                    continue;
                }
                let drift = (time * 0.4 + seed).sin() * 0.25;
                let at = Vec3::new(x + drift, 0.3 + h, z + (hash(seed + 2.0) - 0.5) * 0.4);
                let size = 1.6 + 0.8 * hash(seed + 3.0) + 0.6 * ph;
                let col = linear3(mist.lerp(mist * 0.6, hk).to_array()) * LIT * 3.4;
                let mut s = Sprite::billboard(at, [size, size], IMAGE, uv(Cell::Wisp), col);
                s.alpha_cut = 0.0;
                s.opacity = a;
                // Not cut by the floor it rises from.
                s.depth_bias = 0.8;
                self.sprites.push(s);
            }
        }
        // The runes: short bright marks that come and go along the foot.
        let mut x = x0 + 0.35;
        while x < x0 + width {
            let cell = (x / 0.7).floor();
            let lit = hash(cell * 7.1 + (time * 0.8 + cell * 0.37).floor()) > 0.55;
            let edge = smooth(0.0, 1.2, x - x0) * smooth(0.0, 1.2, x0 + width - x);
            if lit && edge > 0.0 {
                let col = linear3(MIST) * 3.0 * edge * open;
                let mut s = Sprite::fixed(Vec3::new(x, 0.12, z), [0.12, 0.34], Vec3::X, Vec3::Y, IMAGE, uv(Cell::Spark), col);
                s.additive = true;
                s.alpha_cut = 0.0;
                s.depth_bias = 0.1;
                self.sprites.push(s);
                self.sprites.push(glow(Vec3::new(x, 0.15, z), 0.7, Cell::HaloOut, col * 0.4));
            }
            x += 0.7;
        }
    }
}

fn glow(at: Vec3, size: f32, cell: Cell, radiance: Vec3) -> Sprite {
    let mut s = Sprite::billboard(at, [size, size], IMAGE, uv(cell), radiance);
    s.additive = true;
    s.alpha_cut = 0.0;
    s.depth_bias = 0.3;
    s
}
