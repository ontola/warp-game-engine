//! Blast effects: fire and smoke billows, flame tongues, ember streaks, a
//! refracting heat shell and the flash and fireball lights, ported from
//! Growing Guns' `violence.gd` (`spawn_bullet_blast`). Drawn in HDR after
//! temporal reconstruction (no ghosting, never in TAA history), clipped and
//! softened against the opaque depth. All motion is evaluated on the GPU
//! from one clock: the game spawns, then only sends the time.
//!
//! ```ignore
//! let mut fx = Effects::default();
//! fx.explosion(time, pos, 6.5, Vec3::new(1.0, 0.7, 0.2));   // on the event
//! fx.flush(&gpu, &renderer, time)?;                          // per frame
//! lights.extend(fx.lights(time));                            // local lights
//! ```
use crate::headlights::LocalLight;
#[path="fx_budget.rs"] mod budget;
pub use budget::{ExplosionBudget,ExplosionStats};
use crate::frame::resource_cache::{BindGroups, CacheStats, TextureViews};
use crate::{Camera, Gpu};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::sync::Mutex;

/// Local lights accept at most 1e8 (lux at 1 m); near a blast the fp16 HDR
/// target saturates anyway.
const MAX_CD: f32 = 100_000_000.0;
const SPHERE_VERTICES: u32 = 16 * 9 * 6;
const BILLOW_VERTICES: u32 = 48 * 24 * 6;
const COLUMN_VERTICES: u32 = 6; // Bounded ray-integrated volume, including camera-inside views.
const STREAK_VERTICES: u32 = 6 * 12;
const MOTE_VERTICES: u32 = 6;
const MAX_PUFFS: usize = 16384;
// A larger history lets the renderer reduce resolution under load instead of
// abruptly replacing every old plume. Retirement has separate bounded room.
const MAX_ACTIVE_SMOKE_VOLUMES: usize = 64;
const MAX_SMOKE_VOLUMES: usize = 96;
const SMOKE_RETIRE_SECONDS: f32 = 0.30;
const SMOKE_SIZE: u32 = 48;
const SMOKE_FRAMES: u32 = 32;
// Playback duration in fx.wgsl; the source bake contains 2.6 seconds of motion.
const SMOKE_LIFETIME: f32 = 4.6;
const SMOKE_BYTES: &[u8] = include_bytes!("../../../assets/fx/explosion-smoke.rg8");
/// Heat shells only for blasts at least this big (Godot BLAST_DISTORTION_MIN_RADIUS).
pub const DISTORTION_MIN_RADIUS: f32 = 5.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Puff {
    pub center_kind: [f32; 4],
    pub ax: [f32; 4],
    pub ay: [f32; 4],
    pub az: [f32; 4],
    pub body: [f32; 4],
    pub timing: [f32; 4],
    pub extra: [f32; 4],
}

const SMOKE: f32 = 0.0;
const FIRE: f32 = 1.0;
const EMBER: f32 = 2.0;
const HEAT: f32 = 3.0;
const COLUMN: f32 = 4.0;
const STREAK: f32 = 5.0;
const MOTE: f32 = 6.0;
const BAKED_SMOKE: f32 = 7.0;

/// An emissive volume animated entirely from its birth time. Values are
/// metres, seconds and linear RGB radiance in nits. `position` is the bottom
/// centre. The GPU integrates a soft radial density along the visible ray,
/// clipped to opaque geometry; cameras may enter the volume. Tapered columns
/// use eight density samples, straight cylinders an exact integral.
#[derive(Clone, Copy, Debug)]
pub struct AdditiveColumn {
    pub position: Vec3,
    pub birth: f32,
    pub life: f32,
    pub height: f32,
    pub radius: [f32; 2],
    pub radius_power: f32,
    pub color: [Vec3; 2],
    /// Normalized ages over which color changes, using smoothstep.
    pub color_transition: [f32; 2],
    pub opacity: [f32; 2],
    pub opacity_power: f32,
    pub radiance: [f32; 2],
    pub radiance_power: f32,
    pub pulse_radians_per_second: f32,
    /// Added sinusoidal radiance, weighted by the radiance ramp.
    pub pulse_radiance: f32,
    pub bottom_radius_scale: f32,
}

impl Default for AdditiveColumn {
    fn default() -> Self {
        Self { position: Vec3::ZERO, birth: 0.0, life: 1.0, height: 1.0,
            radius: [0.1; 2], radius_power: 1.0, color: [Vec3::ONE; 2],
            color_transition: [0.0, 1.0], opacity: [1.0, 0.0], opacity_power: 1.0,
            radiance: [1000.0; 2], radiance_power: 1.0,
            pulse_radians_per_second: 0.0, pulse_radiance: 0.0, bottom_radius_scale: 1.0 }
    }
}

/// A compact six-sided additive streak (72 vertices with caps), or a small mote when
/// length and diameter are similar. It travels along `travel` over its life;
/// motion_power 1 is linear and 2 eases out. Fade durations are fractions of
/// its lifetime. No scene instances, CPU simulation or material uploads are
/// needed after spawning.
#[derive(Clone, Copy, Debug)]
pub struct AdditiveStreak {
    pub position: Vec3,
    pub travel: Vec3,
    pub birth: f32,
    pub life: f32,
    pub radius: [f32; 2],
    pub length: [f32; 2],
    pub color: Vec3,
    pub radiance: f32,
    pub opacity: f32,
    pub fade_in: f32,
    pub fade_out: f32,
    pub motion_power: f32,
}

impl Default for AdditiveStreak {
    fn default() -> Self {
        Self { position: Vec3::ZERO, travel: Vec3::Y, birth: 0.0, life: 1.0,
            radius: [0.05; 2], length: [0.1; 2], color: Vec3::ONE,
            radiance: 1000.0, opacity: 1.0, fade_in: 0.1, fade_out: 0.5, motion_power: 1.0 }
    }
}

/// A soft round, camera-facing light point. GPU motion follows age^motion_power:
/// 1 is linear, values above 1 start at rest and accelerate. Fade durations
/// are fractions of life. Radius is in metres; no CPU simulation is needed.
#[derive(Clone, Copy, Debug)]
pub struct AdditiveMote {
    pub position: Vec3,
    pub travel: Vec3,
    pub birth: f32,
    pub life: f32,
    pub radius: f32,
    pub color: Vec3,
    pub radiance: f32,
    pub opacity: f32,
    pub fade_in: f32,
    pub fade_out: f32,
    pub motion_power: f32,
    /// Seconds to remain stationary after birth, independently of fading in.
    pub motion_delay: f32,
}

impl Default for AdditiveMote {
    fn default() -> Self {
        Self { position: Vec3::ZERO, travel: Vec3::Y, birth: 0.0, life: 2.0,
            radius: 0.025, color: Vec3::ONE, radiance: 1000.0, opacity: 1.0,
            fade_in: 0.3, fade_out: 0.3, motion_power: 3.0, motion_delay: 0.0 }
    }
}

impl AdditiveMote {
    /// Sample the same trajectory as fx_mote_vs for a validated recipe.
    pub fn position_at(&self, time: f32) -> Vec3 {
        let age = ((time - self.birth - self.motion_delay) / (self.life - self.motion_delay).max(1e-6)).clamp(0.0, 1.0);
        self.position + self.travel * age.powf(self.motion_power)
    }

    fn puff(self) -> Result<Puff, String> {
        if !self.motion_delay.is_finite() || self.motion_delay < 0.0 || self.motion_delay >= self.life {
            return Err("additive mote motion delay must be finite and in [0, life)".into());
        }
        // Reuse the common additive particle validation and packed layout.
        let mut puff = AdditiveStreak { position: self.position, travel: self.travel,
            birth: self.birth, life: self.life, radius: [self.radius; 2], length: [0.0; 2],
            color: self.color, radiance: self.radiance, opacity: self.opacity,
            fade_in: self.fade_in, fade_out: self.fade_out, motion_power: self.motion_power }.puff()?;
        puff.center_kind[3] = MOTE;
        puff.extra[0] = self.motion_delay;
        Ok(puff)
    }
}

fn effect_lifetime(birth: f32, life: f32) -> bool {
    birth.is_finite() && life.is_finite() && life > 0.0 && life <= 120.0
        && (birth + life).is_finite()
}
fn nonnegative(values: &[f32]) -> bool { values.iter().all(|v| v.is_finite() && *v >= 0.0) }
fn ramp_power(value: f32) -> bool { value.is_finite() && (0.01..=32.0).contains(&value) }

impl AdditiveColumn {
    fn puff(self) -> Result<Puff, String> {
        if !self.position.is_finite() || !effect_lifetime(self.birth, self.life)
            || !self.height.is_finite() || self.height <= 0.0
            || !(self.position.y + self.height).is_finite()
            || !nonnegative(&self.radius) || !ramp_power(self.radius_power)
            || self.color.iter().any(|c| !c.is_finite() || c.min_element() < 0.0)
            || !nonnegative(&self.color_transition) || self.color_transition[0] >= self.color_transition[1] || self.color_transition[1] > 1.0
            || !nonnegative(&self.opacity) || self.opacity.iter().any(|v| *v > 1.0) || !ramp_power(self.opacity_power)
            || !nonnegative(&self.radiance) || !ramp_power(self.radiance_power)
            || !nonnegative(&[self.pulse_radians_per_second, self.pulse_radiance, self.bottom_radius_scale])
            || self.bottom_radius_scale <= 0.0 || self.radius.iter().any(|v| !(v * self.bottom_radius_scale).is_finite())
            || self.radiance.iter().any(|v| !(*v + self.pulse_radiance).is_finite())
            || !(self.pulse_radians_per_second * self.life).is_finite()
            || self.color.iter().any(|c| !(c.max_element() * (self.radiance[0].max(self.radiance[1]) + self.pulse_radiance)).is_finite())
        { return Err("additive column requires finite pose, dimensions and nonnegative radiance; life (0,120], opacity [0,1], ordered color transition [0,1] and powers [0.01,32]".into()); }
        Ok(Puff { center_kind: self.position.extend(COLUMN).to_array(),
            ax: [self.radius[0], self.radius[1], self.height, self.radius_power],
            ay: self.color[0].extend(self.opacity[0]).to_array(),
            az: self.color[1].extend(self.opacity[1]).to_array(),
            body: [self.radiance[0], self.radiance[1], self.opacity_power, self.radiance_power],
            timing: [self.birth, self.life, self.color_transition[0], self.color_transition[1]],
            extra: [self.pulse_radians_per_second, self.pulse_radiance, self.bottom_radius_scale, 0.0] })
    }
}

impl AdditiveStreak {
    fn puff(self) -> Result<Puff, String> {
        if !self.position.is_finite() || !self.travel.is_finite() || !(self.position + self.travel).is_finite()
            || !self.travel.length_squared().is_finite() || !effect_lifetime(self.birth, self.life)
            || !nonnegative(&self.radius) || !nonnegative(&self.length)
            || !self.color.is_finite() || self.color.min_element() < 0.0
            || !nonnegative(&[self.radiance, self.opacity, self.fade_in, self.fade_out])
            || self.opacity > 1.0 || self.fade_in + self.fade_out > 1.0 || !ramp_power(self.motion_power)
            || !(self.color.max_element() * self.radiance).is_finite()
        { return Err("additive streak requires finite pose, dimensions and nonnegative radiance; life (0,120], opacity [0,1], fade fractions totaling <=1 and motion power [0.01,32]".into()); }
        Ok(Puff { center_kind: self.position.extend(STREAK).to_array(),
            ax: self.travel.extend(self.motion_power).to_array(),
            ay: [self.radius[0], self.radius[1], self.length[0], self.length[1]],
            az: self.color.extend(self.radiance).to_array(),
            body: [self.opacity, self.fade_in, self.fade_out, 0.0],
            timing: [self.birth, self.life, 0.0, 0.0], extra: [0.0; 4] })
    }
}

impl Puff {
    fn dust(&self) -> bool { self.kind() == 0 && self.az[3] >= 3.0 }
    fn drift(&self, time: f32) -> Vec3 {
        if !self.dust() { return Vec3::ZERO; }
        let age=((time-self.timing[0])/self.timing[1]).clamp(0.0,1.0);
        Vec3::from_slice(&self.extra[..3]) * (1.0-(1.0-age).powi(4))
    }
    fn kind(&self) -> u32 { self.center_kind[3] as u32 }
    fn end(&self) -> f32 {
        let natural = self.timing[0] + self.timing[1];
        if self.kind() == BAKED_SMOKE as u32 && self.extra[1] > 0.0 {
            natural.min(self.extra[0] + self.extra[1])
        } else { natural }
    }
}

/// A point light that flashes and fades (Godot's blast lights).
#[derive(Clone, Copy, Debug)]
pub struct LightPop { pub pos: Vec3, pub birth: f32, pub kind: u8, pub radius: f32, pub color: Vec3 }

/// Godot TRANS_EXPO EASE_OUT.
fn expo_out(t: f32) -> f32 { if t >= 1.0 { 1.0 } else { 1.0 - 2f32.powf(-10.0 * t) } }

/// Live effects owned by the game.
pub struct Effects {
    budget:Option<budget::BudgetState>,
    puffs: Vec<Puff>,
    pops: Vec<LightPop>,
    rng: u64,
    changed: bool,
    baked_smoke: bool,
    /// Candela per unit of Godot light energy: Godot lights a surface at
    /// distance d with energy / d^attenuation times its sun, so one unit at
    /// 1 m is about a sun's worth (37,000 lux horizontal at midday).
    pub light_scale: f32,
    /// Continuous soot shading instead of the original banded arena palette.
    pub smooth_billows: bool,
}

impl Default for Effects {
    fn default() -> Self { Effects { budget:None,puffs: Vec::new(), pops: Vec::new(), rng: 0x2545_F491_4F6C_DD1D, changed: false, baked_smoke: false, light_scale: 37_000.0, smooth_billows: false } }
}

impl Effects {
    fn f(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() }

    pub fn len(&self) -> usize { self.puffs.len() }
    pub fn is_empty(&self) -> bool { self.puffs.is_empty() && self.budget.as_ref().is_none_or(|b|b.is_empty()) }
    pub fn clear(&mut self) { if let Some(b)=self.budget.take(){self.budget=Some(budget::BudgetState::reset(b));} self.puffs.clear(); self.pops.clear(); self.changed = true; }

    /// Use the baked volume animation for subsequent explosions. Other smoke
    /// (blood mist, gib mist) and previously spawned effects are unchanged.
    /// The animation is visual only, shared by every view and driven by the
    /// same interpolated clock as the blast's fire, lights and embers.
    pub fn set_baked_smoke(&mut self, enabled: bool) { self.baked_smoke = enabled; }

    /// Spawn one GPU-animated column. Oldest effects are recycled at the
    /// shared pool limit, including when many future births are scheduled.
    pub fn column(&mut self, spec: AdditiveColumn) -> Result<(), String> {
        let puff = spec.puff()?;
        self.additive(puff);
        Ok(())
    }

    pub fn streak(&mut self, spec: AdditiveStreak) -> Result<(), String> {
        let puff = spec.puff()?;
        self.additive(puff);
        Ok(())
    }

    pub fn mote(&mut self, spec: AdditiveMote) -> Result<(), String> {
        let puff = spec.puff()?;
        self.additive(puff);
        Ok(())
    }

    fn additive(&mut self, puff: Puff) {
        if self.puffs.len() >= MAX_PUFFS { self.puffs.drain(..self.puffs.len() + 1 - MAX_PUFFS); }
        self.puffs.push(puff);
        self.changed = true;
    }

    /// Preserve ordinary tail motion while easing the oldest over-budget
    /// plumes out. Kind 7 packs retirement start/duration in extra.xy; a zero
    /// duration means the natural animation lifetime. Only a pathological
    /// burst exceeding the hard total bound can force an earlier removal.
    fn make_smoke_room(&mut self, time: f32) {
        self.puffs.retain(|p| p.kind() != BAKED_SMOKE as u32 || p.end() > time);
        if self.puffs.iter().filter(|p| p.kind() == BAKED_SMOKE as u32).count() >= MAX_SMOKE_VOLUMES {
            // Source frame zero is empty. Prefer removing a plume that has not
            // been visible yet; otherwise reclaim the retirement nearest its
            // end, preserving established smoke through large same-tick bursts.
            let unseen = self.puffs.iter().position(|p| p.kind() == BAKED_SMOKE as u32 && p.timing[0] >= time);
            let retiring = self.puffs.iter().enumerate()
                .filter(|(_,p)| p.kind() == BAKED_SMOKE as u32 && p.extra[1] > 0.0)
                .min_by(|(_,a),(_,b)| a.end().total_cmp(&b.end())).map(|(i,_)| i);
            let oldest = self.puffs.iter().enumerate().filter(|(_,p)| p.kind() == BAKED_SMOKE as u32)
                .min_by(|(_,a),(_,b)| a.timing[0].total_cmp(&b.timing[0])).map(|(i,_)| i);
            if let Some(index) = unseen.or(retiring).or(oldest) { self.puffs.remove(index); }
        }
        if self.puffs.iter().filter(|p| p.kind() == BAKED_SMOKE as u32 && p.extra[1] <= 0.0).count() >= MAX_ACTIVE_SMOKE_VOLUMES {
            let oldest = self.puffs.iter().enumerate()
                .filter(|(_,p)| p.kind() == BAKED_SMOKE as u32 && p.extra[1] <= 0.0)
                .min_by(|(_,a),(_,b)| a.timing[0].total_cmp(&b.timing[0])).map(|(i,_)| i);
            if let Some(index) = oldest {
                let puff = &mut self.puffs[index];
                puff.extra[0] = time.max(puff.timing[0]);
                puff.extra[1] = SMOKE_RETIRE_SECONDS;
            }
        }
    }

    /// Remaining slots for live ground dust.
    pub fn dust_capacity(&self, time: f32) -> usize {
        384usize.saturating_sub(self.puffs.iter().filter(|p|p.dust() && p.end()>time).count())
    }

    /// Low ground dust, using the existing lit billow shader. Travel eases out
    /// on the GPU; overload skips new dust rather than erasing existing trails.
    pub fn ground_dust(&mut self,time:f32,pos:Vec3,travel:Vec3,radius:f32) {
        if self.dust_capacity(time)==0 {return;}
        let seed=self.f();let life=self.range(0.85,1.25);
        self.additive(Puff {
            center_kind:pos.extend(SMOKE).to_array(),
            ax:[radius*2.0,0.0,0.0,seed], ay:[0.0,radius*1.3,0.0,0.0],
            az:[0.0,0.0,radius*2.0,3.0],body:[0.9,0.83,0.72,1.2],
            timing:[time,life,0.12,0.0],extra:travel.extend(0.55).to_array(),
        });
    }

    pub fn exhaust_smoke(&mut self, time: f32, pos: Vec3, radius: f32) {
        let seed=self.f();
        let r=radius.max(0.1);
        self.additive(Puff {
            center_kind:pos.extend(SMOKE).to_array(),
            ax:[r,0.0,0.0,seed], ay:[0.0,r,0.0,0.0], az:[0.0,0.0,r,2.25],
            body:[0.22,0.23,0.24,0.65], timing:[time-0.30,3.0,2.8,0.35],
            extra:[1.0,1.0,1.0,0.38],
        });
    }

    /// A full blast at `time` (seconds on the clock passed to `flush`):
    /// smoke, fireball, flame tongues, embers, heat shell and lights.
    /// `color` tints embers (Godot passes the bullet colour).
    fn full_explosion(&mut self, time: f32, pos: Vec3, radius: f32, color: Vec3) {
        let r = radius.max(0.5);
        // Consume the same recipe RNG in either mode: enabling volume smoke
        // must not change the subsequent fire, embers or their trajectories.
        let smoke_start = self.puffs.len();
        let n = ((r * 1.1) as i32).clamp(8, 24) as usize;
        self.billows(time, pos, r, n, false, 0.5 + r * 0.13, 0.5, 1.3, r * 0.7, 1.0, 0.0, Vec3::ONE, 1.0);
        if self.baked_smoke {
            self.puffs.truncate(smoke_start);
            self.make_smoke_room(time);
            let yaw = (self.rng >> 40) as f32 / (1u64 << 24) as f32 * std::f32::consts::TAU;
            let (sin, cos) = yaw.sin_cos();
            let side = r * 2.4;
            self.puffs.push(Puff {
                center_kind: (pos - Vec3::Y * r * 0.12).extend(BAKED_SMOKE).to_array(),
                ax: [cos * side, 0.0, -sin * side, 0.0],
                ay: [0.0, side, 0.0, 0.0],
                az: [sin * side, 0.0, cos * side, 0.0],
                body: [1.0, 0.97, 0.92, 14.0],
                timing: [time, SMOKE_LIFETIME, 0.0, 0.0],
                ..Default::default()
            });
        }
        // Fireball (spawn_blast_fire_clouds).
        let n = ((r * 0.8) as i32).clamp(6, 18) as usize;
        self.billows(time, pos, r, n, true, 0.35 + r * 0.055, 0.3, 0.7, r * 0.3, 1.0, 0.0, Vec3::ONE, 1.0);
        // Flame tongues: stretched fire puffs, a beat later (spawn_blast_flame_shards).
        let n = ((r * 0.7) as i32).clamp(5, 12) as usize;
        self.billows(time, pos, r, n, true, 0.34 + r * 0.05, 0.28, 0.6, r * 0.28, 2.6, 0.14, Vec3::ONE, 1.0);
        self.embers(time, pos, r, color);
        if r >= DISTORTION_MIN_RADIUS {
            let duration = (0.42 + r * 0.018).clamp(0.42, 0.95);
            let strength = (r * 0.026).clamp(0.11, 0.26);
            self.puffs.push(Puff { center_kind: pos.extend(HEAT).to_array(), ax: [r * 2.2, 0.0, 0.0, 0.0], timing: [time, duration, 0.0, 0.0],
                extra: [strength, 0.0, 0.0, 0.0], body: [0.0, 0.0, 0.0, 1.0], ..Default::default() });
        }
        self.pops.push(LightPop { pos, birth: time, kind: 0, radius: r, color });
        self.pops.push(LightPop { pos, birth: time, kind: 1, radius: r, color });
        self.changed = true;
    }

    /// A sparse attached emitter; callers provide positions at their own fixed
    /// emission interval. Uses the same soft fire/smoke volumes as explosions.
    pub fn burning_fragment(&mut self, time:f32, pos:Vec3, radius:f32) {
        let r=radius.clamp(0.1,3.0);
        self.billows(time,pos,r,1,false,0.8,0.55,1.4,r*0.3,1.1,0.0,Vec3::ONE,0.7);
        self.billows(time,pos,r,1,true,0.28,0.45,0.9,r*0.15,1.7,0.0,Vec3::ONE,1.0);
        self.changed=true;
    }

    /// Narrow upright flames around a sinking body. Keep the hot geometry
    /// away from its eye position instead of enclosing the camera in a puff.
    pub fn burning_body(&mut self, time: f32, pos: Vec3) {
        let seed = self.f();
        let width = self.range(0.25, 0.42);
        let height = self.range(2.4, 3.4);
        self.additive(Puff {
            center_kind: (pos + Vec3::Y * 0.12).extend(FIRE).to_array(),
            ax: [width, 0.0, 0.0, seed], ay: [0.0, height, 0.0, 0.0],
            az: [0.0, 0.0, width, 2.55], body: [1.0, 1.0, 1.0, 0.95],
            timing: [time, 0.55, 0.08, 0.0], extra: [1.0, 1.0, 1.0, 1.2],
            ..Default::default()
        });
    }

    /// A cloud of smoke or vapour welling up from `pos` (its bottom centre)
    /// and drifting apart over a few seconds, about `radius` metres across
    /// at its widest: the baked volumetric plume when that is on, else soft
    /// billows. `tint` is its linear albedo (0.15 soot .. 1.0 white steam),
    /// `density` 0.3 a thin wisp .. 1.5 thick.
    pub fn smoke_cloud(&mut self, time: f32, pos: Vec3, radius: f32, tint: Vec3, density: f32) {
        if !time.is_finite() || !pos.is_finite() || !radius.is_finite() || !tint.is_finite() || !density.is_finite() {
            return;
        }
        let r = radius.max(0.2);
        let density = density.clamp(0.05, 3.0);
        if self.baked_smoke {
            self.make_smoke_room(time);
            let yaw = self.f() * std::f32::consts::TAU;
            let (sin, cos) = yaw.sin_cos();
            let side = r * 2.0;
            self.puffs.push(Puff {
                center_kind: pos.extend(BAKED_SMOKE).to_array(),
                ax: [cos * side, 0.0, -sin * side, 0.0],
                ay: [0.0, side, 0.0, 0.0],
                az: [sin * side, 0.0, cos * side, 0.0],
                body: tint.clamp(Vec3::ZERO, Vec3::ONE).extend(14.0 * density).to_array(),
                timing: [time, SMOKE_LIFETIME, 0.0, 0.0],
                ..Default::default()
            });
        } else {
            let n = ((r * 2.5) as usize).clamp(4, 14);
            // The billows' own greys average about 0.17.
            self.billows(time, pos, r * 0.8, n, false, 1.0 + r * 0.15, 0.45, 1.05, r * 0.5, 1.0, 0.0, tint / 0.17, (0.55 * density).min(1.0));
        }
        self.changed = true;
    }

    /// A hanging red haze where spectators died (Godot's blood mist: the
    /// smoke billow with a red tint, translucent, slower to rise).
    pub fn blood_mist(&mut self, time: f32, pos: Vec3, radius: f32, kills: usize) {
        let r = (radius * 2.0).max(2.0);
        let n = ((4.0 + kills as f32 * 0.7).round() as usize).clamp(4, 10);
        self.billows(time, pos, r, n, false, (0.5 + 0.13 * r) * 1.6, 0.45, 1.05, r * 0.3, 1.0, 0.0, Vec3::new(2.2, 0.22, 0.18), 0.6);
        self.changed = true;
    }

    /// A short burst of red spray where a body came apart (Godot
    /// `spawn_gib_mist`): small dark puffs that swell and fade in half a
    /// second. `intensity` 0.35..2.8.
    pub fn gib_mist(&mut self, time: f32, pos: Vec3, intensity: f32) {
        let r = 0.35 + intensity.clamp(0.35, 2.8) * 0.12;
        let n = ((8.0 + intensity * 2.5).round() as usize).clamp(8, 14);
        self.billows(time, pos, r, n, false, 0.55, 0.45, 0.9, r * 0.25, 1.0, 0.0, Vec3::new(1.15, 0.07, 0.05), 0.55);
        self.changed = true;
    }

    #[allow(clippy::too_many_arguments)]
    fn billows(&mut self, time: f32, pos: Vec3, r: f32, count: usize, fire: bool, life: f32, scale_lo: f32, scale_hi: f32, rise: f32, stretch: f32, delay_bias: f32, tint: Vec3, opacity: f32) {
        let spread = r * if fire { 0.45 } else { 0.65 };
        let base = pos + Vec3::Y * r * 0.1;
        for _ in 0..count {
            let ang = self.f() * std::f32::consts::TAU;
            let rad = self.range(0.0, spread);
            let mut off = Vec3::new(ang.cos() * rad, self.range(-0.2, 0.6) * r * 0.45, ang.sin() * rad);
            let s = r * self.range(scale_lo, scale_hi);
            let (mut ax, mut ay, mut az) = (Vec3::X * s, Vec3::Y * s, Vec3::Z * s);
            if stretch > 1.01 {
                // Tongues fire off radially, long axis along their direction.
                let azim = self.f() * std::f32::consts::TAU;
                let elev = self.range(-0.25, 0.85);
                let dir = Vec3::new(azim.cos() * elev.cos(), elev.sin(), azim.sin() * elev.cos());
                off = dir * (rad + r * 0.15);
                let up = if dir.y.abs() < 0.95 { Vec3::Y } else { Vec3::X };
                let ex = up.cross(dir).normalize();
                let ez = dir.cross(ex);
                (ax, ay, az) = (ex * s, dir * s * stretch, ez * s);
            }
            let seed = self.f();
            let delay = delay_bias + self.range(0.0, if fire { 0.22 } else { 0.12 });
            let (heat, body, opk) = if fire {
                ((1.0 - (rad / spread.max(0.001)).clamp(0.0, 1.0)).powf(1.4), Vec3::ONE, self.range(0.75, 0.95))
            } else {
                let h = self.range(0.2, 0.55);
                let g = self.range(0.07, 0.26);
                (h, Vec3::new(g * 1.08, g, g * 0.9), self.range(0.6, 0.85))
            };
            let speed = self.range(0.0, 0.8);
            let tail = if fire { 2.2 } else { 0.38 };
            self.puffs.push(Puff {
                center_kind: (base + off).extend(if fire { FIRE } else { SMOKE }).to_array(),
                ax: ax.extend(seed).to_array(), ay: ay.extend(delay).to_array(), az: az.extend(heat + if self.smooth_billows { 2.0 } else { 0.0 }).to_array(),
                body: body.extend(opk * opacity).to_array(),
                timing: [time, life, rise, speed],
                extra: tint.extend(tail).to_array(),
            });
        }
    }

    fn embers(&mut self, time: f32, pos: Vec3, r: f32, color: Vec3) {
        let count = ((r * 5.0) as i32).clamp(30, 110);
        let hot = color.lerp(Vec3::new(1.0, 0.86, 0.5), 0.4);
        let life = 0.3 + r * 0.03;
        for _ in 0..count {
            let theta = self.f() * std::f32::consts::TAU;
            let elev = self.range(-0.2, 1.35);
            let dir = Vec3::new(theta.cos() * elev.cos(), elev.sin(), theta.sin() * elev.cos()).normalize();
            let dist = r * self.range(0.8, 2.0);
            let drop = r * self.range(0.2, 0.7);
            let travel = dir * dist + Vec3::NEG_Y * drop;
            let sz = self.range(0.4, 1.15);
            let streak = 1.0 + (dist / r).clamp(0.0, 2.0) * self.range(1.6, 3.6);
            let delay = self.range(0.0, 0.15);
            let bright = self.range(0.6, 1.0);
            self.puffs.push(Puff {
                center_kind: pos.extend(EMBER).to_array(),
                ax: travel.extend(delay).to_array(),
                body: hot.extend(bright).to_array(),
                timing: [time, life, 0.0, 0.0],
                extra: [sz, ((streak - 1.0) / 7.0).clamp(0.0, 1.0), 1.0, 5.0],
                ..Default::default()
            });
        }
    }

    /// The blast lights at `time`: a hot white flash and the longer orange
    /// fireball glow (Godot `_spawn_blast_flash_light`,
    /// `_spawn_blast_fireball_light`).
    pub fn lights(&self, time: f32) -> Vec<LocalLight> {
        let hot = Vec3::new(1.0, 0.995, 0.98);
        let warm = Vec3::new(1.0, 0.54, 0.12);
        let mut out = Vec::new();
        for p in self.pops.iter().rev() {
            let t = time - p.birth;
            if t < 0.0 { continue; }
            let r = p.radius;
            // Godot falloff: flash d^-1 (default attenuation), fireball d^-0.42.
            let falloff = if p.kind == 0 { 1.0 } else { 0.42 };
            let (energy, color, range) = if p.kind == 0 {
                let peak = 6.0 + r * 6.5;
                let hold = (0.018 + r * 0.0035).clamp(0.018, 0.13);
                let decay = (0.05 + r * 0.008).clamp(0.05, 0.32);
                if t > hold + decay { continue; }
                let e = if t < hold { peak } else { peak * (1.0 - expo_out((t - hold) / decay)) };
                (e, hot.lerp(warm, (t / (hold + decay)).clamp(0.0, 1.0)), (r * 6.5).clamp(45.0, 190.0))
            } else {
                // Keep the footprint stable: an expanding range reads as a ring
                // sweeping across the arena, detached from the actual fireball.
                let life = (0.25 + r * 0.02).clamp(0.30, 0.75);
                if t >= life { continue; }
                let phase = (t / life).clamp(0.0, 1.0);
                let fade = 1.0 - phase * phase * (3.0 - 2.0 * phase);
                let peak = 178.0 * (r / 30.0).clamp(0.0, 1.0).powf(0.92);
                let e = peak * fade;
                let c = p.color.lerp(hot, 0.45).lerp(p.color.lerp(warm, 0.65), phase);
                let range = (r * 0.58 * 2.6).clamp(14.0, 64.0);
                (e, c, range)
            };
            if energy <= 0.0 { continue; }
            out.push(LocalLight { position: p.pos, direction: Vec3::NEG_Y, intensity_cd: (color * energy * self.light_scale).min(Vec3::splat(MAX_CD)), range_m: range.min(200.0),
                inner_cos: -0.99, outer_cos: -1.0, source_radius_m: (r * 0.3).clamp(0.2, 1.0), falloff });
        }
        out
    }

    /// Drop finished effects and send the live ones (only when changed).
    pub fn flush(&mut self, gpu: &Gpu, renderer: &crate::Renderer, time: f32) -> Result<(), String> {
        self.emit_pending(time);
        let before = self.puffs.len();
        self.puffs.retain(|p| p.end() > time);
        self.pops.retain(|p| time - p.birth < 2.0);
        // Oldest go first when over budget.
        if self.puffs.len() > MAX_PUFFS { let extra = self.puffs.len() - MAX_PUFFS; self.puffs.drain(..extra); }
        let changed = std::mem::take(&mut self.changed) || self.puffs.len() != before;
        renderer.set_effects(gpu, time, changed.then_some(&self.puffs[..]))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform { view_proj: [[f32; 4]; 4], inverse_view_proj: [[f32; 4]; 4], depth_vp: [[f32; 4]; 4], view: [[f32; 4]; 4], camera_near: [f32; 4], size: [f32; 4], clock: [f32; 4], smoke: [f32; 4], depth_from_output: [[f32;4];4], ambient: [f32;4], sunlight: [f32;4], fog: [f32;4], horizon: [f32;4], sun_vp: [[f32;4];4], fog_scattering: [f32;4], sun_direction: [f32;4], sky_mean: [f32;4] }

// The budget counts full-resolution pixel layers, so approaching a plume or
// overlapping more plumes lowers raymarch resolution before work is submitted.
const SMOKE_PIXEL_BUDGET: f32 = 450_000.0;

fn projected_smoke_work(source: &[Puff], time: f32, vp: Mat4, near: f32, size: (u32,u32)) -> (f32,usize) {
    source.iter().filter(|p| p.kind() == BAKED_SMOKE as u32 && p.timing[0] <= time && p.end() > time).map(|p| {
        let base=Vec3::from_slice(&p.center_kind[..3]);
        let ax=Vec3::from_slice(&p.ax[..3]);
        let ay=Vec3::from_slice(&p.ay[..3]);
        let az=Vec3::from_slice(&p.az[..3]);
        let corners:[glam::Vec4;8]=std::array::from_fn(|i| {
            let q=Vec3::new((i&1) as f32-0.5,((i>>1)&1) as f32,((i>>2)&1) as f32-0.5);
            vp*(base+ax*q.x+ay*q.y+az*q.z).extend(1.0)
        });
        if corners.iter().all(|c|c.x < -c.w) || corners.iter().all(|c|c.x > c.w)
            || corners.iter().all(|c|c.y < -c.w) || corners.iter().all(|c|c.y > c.w)
            || corners.iter().all(|c|c.w <= near) {return 0.0;}
        let pixels=size.0 as f32*size.1 as f32;
        if corners.iter().any(|c|c.w <= near) {return pixels;}
        let mut lo=glam::Vec2::ONE;
        let mut hi=glam::Vec2::NEG_ONE;
        for c in corners {
            let q=glam::Vec2::new(c.x,c.y)/c.w;
            lo=lo.min(q); hi=hi.max(q);
        }
        let span=(hi.min(glam::Vec2::ONE)-lo.max(glam::Vec2::NEG_ONE)).max(glam::Vec2::ZERO);
        span.x*span.y*0.25*pixels
    }).fold((0.0,0),|(pixels,count),area|(pixels+area,count+usize::from(area>0.0)))
}

// Farthest possible camera-forward depth of the complete soft bucket. When
// both scene guides lie beyond this bound, their discontinuity cannot occlude
// any soft effect and ordinary bilinear reconstruction is safe.
fn soft_depth_limit(source: &[Puff], time: f32, view: Mat4) -> f32 {
    source.iter().filter(|p| matches!(p.kind(),0|1|7) && p.timing[0]<=time && p.end()>time).map(|p| {
        let center=-view.transform_point3(Vec3::from_slice(&p.center_kind[..3])+p.drift(time)).z;
        let axes=Vec3::new(-view.transform_vector3(Vec3::from_slice(&p.ax[..3])).z,
            -view.transform_vector3(Vec3::from_slice(&p.ay[..3])).z,
            -view.transform_vector3(Vec3::from_slice(&p.az[..3])).z);
        let (extent,rise)=if p.kind()==BAKED_SMOKE as u32 {
            // Texture local x/z are -0.5..0.5; local y is 0..1.
            (0.5*(axes.x.abs()+axes.z.abs())+axes.y.max(0.0),0.0)
        } else {
            // fx_vs keeps local position parallel to a unit sphere normal.
            // Its largest radial displacement is 0.5+0.32=0.82 (continuous
            // billows are smaller), and growth never exceeds one. Include
            // both ends of the full rise, including negative rise amounts.
            (0.82*axes.length(),(-view.transform_vector3(Vec3::Y*p.timing[2]).z).max(0.0))
        };
        let guard=0.05+0.001*(center.abs()+extent+rise);
        center+extent+rise+guard
    }).fold(0.0,f32::max)
}

/// Actual per-view adaptive decision from the last FX submission. Pixel layers
/// use conservative projected volume bounds, not an estimate of visible density.
#[derive(Clone,Debug,Default,serde::Serialize)]
pub struct SmokeQualityStats {
    pub view_index: usize,
    pub divisor: u32,
    pub ray_steps: u32,
    pub projected_pixel_layers: f32,
    pub visible_volumes: usize,
    pub soft_size: [u32;2],
}

#[derive(Default)]
struct SmokeQuality {
    divisor: u32,
    last_time: Option<f32>,
    recover_since: Option<f32>,
}
impl SmokeQuality {
    fn desired(pixels: f32) -> u32 {
        [1,2,4,8].into_iter().find(|d|pixels/((*d * *d) as f32) <= SMOKE_PIXEL_BUDGET).unwrap_or(8)
    }
    fn update(&mut self, pixels: f32, time: f32) -> u32 {
        let desired=Self::desired(pixels);
        let reset=self.divisor==0 || self.last_time.is_some_and(|last|time<last || time-last>1.0);
        self.last_time=Some(time);
        if reset || pixels<=0.0 || desired>self.divisor {
            self.divisor=desired;
            self.recover_since=None;
        } else if desired<self.divisor && pixels/((self.divisor/2).pow(2) as f32) < SMOKE_PIXEL_BUDGET*0.65 {
            let since=*self.recover_since.get_or_insert(time);
            if time-since>=0.25 {self.divisor=desired; self.recover_since=None;}
        } else {self.recover_since=None;}
        self.divisor
    }
}

#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct CompositeUniform {
    depth_from_output: [[f32;4];4],
    // Output width, height, near plane, farthest possible soft-effect depth.
    full: [f32;4],
    // Reduced width, height and reciprocals.
    low: [f32;4],
}
struct SoftTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32,u32),
    binds: BindGroups<2>,
}

struct Target {
    size: (u32, u32),
    output: wgpu::Texture,
    output_view: wgpu::TextureView,
    uniform: wgpu::Buffer,
    order: wgpu::Buffer,
    // Each view can alternate TAA histories. Cache actual input/depth identity;
    // a same-size replacement must not sample the preceding frame's textures.
    backgrounds: TextureViews<2>,
    binds: BindGroups<2>,
    composite_uniform: wgpu::Buffer,
    soft: [Option<SoftTarget>;3],
    quality: SmokeQuality,
    quality_stats: SmokeQualityStats,
}

/// Shared CPU scratch: views encode serially and queue writes copy their data.
/// Each view still owns its GPU order buffer until the common submission.
#[derive(Default)]
struct DrawOrder {
    minimum_selection:Option<(f32,Vec3,Vec3,Vec<u32>)>,
    heat: Vec<u32>,
    billows: Vec<(u32, f32)>,
    embers: Vec<u32>,
    columns: Vec<u32>,
    streaks: Vec<u32>,
    motes: Vec<u32>,
    ids: Vec<u32>,
}

impl DrawOrder {
    fn prepare(&mut self, source: &[Puff], time: f32, camera: Vec3) -> [u32; 6] {
        self.heat.clear();
        self.billows.clear();
        self.embers.clear();
        self.columns.clear();
        self.streaks.clear();
        self.motes.clear();
        self.ids.clear();
        for (i, p) in source.iter().enumerate() {
            if p.end() <= time || p.timing[0] > time { continue; }
            match p.kind() {
                0 | 1 | 7 => {
                    let mut center = Vec3::from_slice(&p.center_kind[..3])+p.drift(time);
                    if p.kind() == BAKED_SMOKE as u32 { center += Vec3::from_slice(&p.ay[..3]) * 0.5; }
                    self.billows.push((i as u32, center.distance_squared(camera)));
                },
                2 => self.embers.push(i as u32),
                3 => self.heat.push(i as u32),
                4 => self.columns.push(i as u32),
                5 => self.streaks.push(i as u32),
                6 => self.motes.push(i as u32),
                _ => {},
            }
        }
        // Source indices are unique, so the tie-break gives the same total
        // order as stable sort without that algorithm's temporary allocation.
        self.billows.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        self.finish()
    }
    fn limit_minimum(&mut self,source:&[Puff],time:f32,camera:Camera,vp:Mat4)->[u32;6] {
        let forward=(camera.target-camera.pos).normalize_or_zero();
        let selected=if let Some((old_time,old_pos,old_forward,ids))=&self.minimum_selection
            && *old_time==time && old_pos.distance_squared(camera.pos)<0.04 && old_forward.dot(forward)>0.995 {ids.clone()}
        else {
            let rows:[glam::Vec4;4]=std::array::from_fn(|i|vp.row(i));
            let planes=[rows[3]+rows[0],rows[3]-rows[0],rows[3]+rows[1],rows[3]-rows[1],rows[3]-rows[2]];
            let mut candidates=Vec::new();
            for &(id,_) in &self.billows {let p=&source[id as usize];if p.kind()==BAKED_SMOKE as u32 {continue;}let anim=((time-p.timing[0])/p.timing[1]).clamp(0.,1.);
                let age=((anim-p.ay[3])*(1.+p.timing[3])/(1.-p.ay[3]).max(0.001)).clamp(0.,1.);
                if anim<=p.ay[3]||age>=1. {continue;}
                let centre=Vec3::from_slice(&p.center_kind[..3])+p.drift(time)+Vec3::Y*p.timing[2]*anim;
                let radius=[p.ax,p.ay,p.az].map(|a|Vec3::from_slice(&a[..3]).length()).into_iter().fold(0.,f32::max)*0.5*(0.35+0.65*age);
                if planes.iter().any(|p|p.dot(centre.extend(1.)) < -radius*p.truncate().length()){continue;}
                let z=(vp*centre.extend(1.)).w;
                let area=(std::f32::consts::PI*radius*radius*rows[0].truncate().length()*rows[1].truncate().length()/(4.*(z-radius).max(camera.near).powi(2))).min(1.);
                candidates.push((p.kind()!=1,centre.distance_squared(camera.pos),id,area));
            }
            // Fire communicates the event; smoke spends only leftover coverage.
            candidates.sort_by(|a,b|a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
            let mut area=0.;let mut ids=Vec::new();
            for (_,_,id,cost) in candidates {if ids.is_empty()||area+cost<=0.65 {ids.push(id);area+=cost;}}
            self.minimum_selection=Some((time,camera.pos,forward,ids.clone()));ids
        };
        self.billows.retain(|p|source[p.0 as usize].kind()==BAKED_SMOKE as u32||selected.contains(&p.0));
        self.finish()
    }
    fn finish(&mut self)->[u32;6] {
        self.ids.clear();
        self.ids.extend(self.heat.iter().copied().chain(self.billows.iter().map(|b| b.0))
            .chain(self.embers.iter().copied()).chain(self.columns.iter().copied()).chain(self.streaks.iter().copied()).chain(self.motes.iter().copied()));
        [self.heat.len() as u32, self.billows.len() as u32, self.embers.len() as u32,
            self.columns.len() as u32, self.streaks.len() as u32, self.motes.len() as u32]
    }
}

/// Renderer side, created on first use.
pub(crate) struct FxRenderer {
    layout: wgpu::BindGroupLayout,
    billow: wgpu::RenderPipeline,
    dust: wgpu::RenderPipeline,
    ember: wgpu::RenderPipeline,
    heat: wgpu::RenderPipeline,
    column: wgpu::RenderPipeline,
    streak: wgpu::RenderPipeline,
    mote: wgpu::RenderPipeline,
    smoke: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite: wgpu::RenderPipeline,
    adaptive_smoke: bool,
    smoke_view: wgpu::TextureView,
    smoke_loaded: bool,
    ldr: Option<(wgpu::TextureFormat,[wgpu::RenderPipeline;6])>,
    sampler: wgpu::Sampler,
    puffs: wgpu::Buffer,
    source: Vec<Puff>,
    time: f32,
    draw_order: DrawOrder,
    targets: Vec<Option<Target>>,
}

pub fn shader_source() -> &'static str { concat!(include_str!("../../../assets/fx/explosion-smoke-bounds.wgsl"), include_str!("shaders/fx.wgsl")) }
fn composite_shader_source() -> &'static str { include_str!("shaders/fx_composite.wgsl") }

impl FxRenderer {
    fn new(gpu: &Gpu) -> Self {
        let entries: Vec<_> = (0..8).map(|binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, count: None,
            ty: match binding {
                0 => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                1 | 2 => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                3 => wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                4 | 7 => wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                6 => wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                _ => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            } }).collect();
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fx"), entries: &entries });
        let pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fx"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("fx"), source: wgpu::ShaderSource::Wgsl(shader_source().into()) });
        let additive = wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add } };
        let pipe = |vertex: &str, entry: &str, blend: wgpu::BlendState| gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor { label: Some(entry), layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some(vertex), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() }, depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(entry), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: gpu.pipeline_cache() });
        let dust = pipe("fx_min_billow_vs", "fx_min_billow", wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let billow = pipe("fx_vs", "fx_billow", wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let ember = pipe("fx_vs", "fx_ember", additive);
        let heat = pipe("fx_vs", "fx_heat", wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let column = pipe("fx_column_vs", "fx_column", additive);
        let streak = pipe("fx_streak_vs", "fx_streak", additive);
        let mote = pipe("fx_mote_vs", "fx_mote", additive);
        let smoke = pipe("fx_smoke_vs", "fx_smoke", wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        // A tiny empty binding keeps games without baked smoke from allocating
        // or uploading the 6.75 MiB animation. Replace it on the first spawn.
        let smoke_view = Self::smoke_texture(gpu, false);
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("fx background"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, address_mode_w: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        let puffs = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("fx puffs"), size: (MAX_PUFFS * std::mem::size_of::<Puff>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let composite_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:Some("soft FX depth-aware composite"),entries:&[
                wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},count:None},
                wgpu::BindGroupLayoutEntry{binding:1,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
                wgpu::BindGroupLayoutEntry{binding:2,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Depth,view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
            ]});
        let composite_pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some("soft FX composite"),bind_group_layouts:&[Some(&composite_layout)],immediate_size:0});
        let composite_shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("soft FX composite"),source:wgpu::ShaderSource::Wgsl(composite_shader_source().into())});
        let composite=gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some("soft FX depth-aware upsample"),layout:Some(&composite_pl),
            vertex:wgpu::VertexState{module:&composite_shader,entry_point:Some("fullscreen"),compilation_options:Default::default(),buffers:&[]},
            primitive:Default::default(),depth_stencil:None,multisample:Default::default(),
            fragment:Some(wgpu::FragmentState{module:&composite_shader,entry_point:Some("composite"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format:wgpu::TextureFormat::Rgba16Float,blend:Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),
            multiview_mask:None,cache:gpu.pipeline_cache()});
        let adaptive_smoke=std::env::var("WARP_SMOKE_QUALITY").as_deref()!=Ok("full");
        FxRenderer { layout, billow, dust, ember, heat, column, streak, mote, smoke, composite_layout, composite, adaptive_smoke, smoke_view, smoke_loaded: false, ldr:None, sampler, puffs, source: Vec::new(), time: 0.0,
            draw_order: DrawOrder::default(), targets: Vec::new() }
    }

    fn ldr_pipelines(&self,gpu:&Gpu,format:wgpu::TextureFormat)->[wgpu::RenderPipeline;6] {
        let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("minimum gameplay FX"),source:wgpu::ShaderSource::Wgsl(shader_source().into())});
        let layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:None,bind_group_layouts:&[Some(&self.layout)],immediate_size:0});
        let additive=wgpu::BlendState{color:wgpu::BlendComponent{src_factor:wgpu::BlendFactor::One,dst_factor:wgpu::BlendFactor::One,operation:wgpu::BlendOperation::Add},alpha:wgpu::BlendComponent{src_factor:wgpu::BlendFactor::Zero,dst_factor:wgpu::BlendFactor::One,operation:wgpu::BlendOperation::Add}};
        [("fx_min_billow_vs","fx_min_billow",wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),("fx_vs","fx_ember",additive),("fx_column_vs","fx_column",additive),("fx_streak_vs","fx_streak",additive),("fx_mote_vs","fx_mote",additive),("fx_smoke_vs","fx_smoke",wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)].map(|(vertex,fragment,blend)|
            gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some(fragment),layout:Some(&layout),
                vertex:wgpu::VertexState{module:&shader,entry_point:Some(vertex),compilation_options:Default::default(),buffers:&[]},
                primitive:wgpu::PrimitiveState{cull_mode:None,..Default::default()},depth_stencil:None,multisample:Default::default(),
                fragment:Some(wgpu::FragmentState{module:&shader,entry_point:Some(fragment),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format,blend:Some(blend),write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache:gpu.pipeline_cache()}))
    }

    fn smoke_texture(gpu: &Gpu, animation: bool) -> wgpu::TextureView {
        let side = if animation { SMOKE_SIZE } else { 1 };
        let depth = if animation { SMOKE_SIZE * SMOKE_FRAMES } else { 1 };
        let size = wgpu::Extent3d { width: side, height: side, depth_or_array_layers: depth };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("baked explosion density and light"), size, mip_level_count: 1,
            sample_count: 1, dimension: wgpu::TextureDimension::D3, format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let bytes = if animation { SMOKE_BYTES } else { &[0, 255] };
        gpu.queue.write_texture(texture.as_image_copy(), bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side * 2), rows_per_image: Some(side) }, size);
        texture.create_view(&Default::default())
    }

    fn set(&mut self, gpu: &Gpu, time: f32, puffs: Option<&[Puff]>) {
        self.time = time;
        if let Some(p) = puffs {
            let p = &p[..p.len().min(MAX_PUFFS)];
            if !self.smoke_loaded && p.iter().any(|p| p.kind() == BAKED_SMOKE as u32) {
                self.smoke_view = Self::smoke_texture(gpu, true);
                self.smoke_loaded = true;
            }
            self.draw_order.minimum_selection=None;
            self.source.clear();
            self.source.extend_from_slice(p);

            if !p.is_empty() { gpu.queue.write_buffer(&self.puffs, 0, bytemuck::cast_slice(p)); }
        }
    }

    pub(crate) fn quality_stats(&self) -> Vec<SmokeQualityStats> {
        self.targets.iter().flatten().map(|target|target.quality_stats.clone()).collect()
    }

    pub(crate) fn resource_cache_stats(&self) -> CacheStats {
        let mut stats = CacheStats::default();
        for target in self.targets.iter().flatten() {
            stats.add(target.backgrounds.stats());
            stats.add(target.binds.stats());
            for soft in target.soft.iter().flatten() {stats.add(soft.binds.stats());}
        }
        stats
    }

    /// Draw over a copy of `input`; returns the new HDR output, or None when
    /// nothing is live.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, index: usize, input: &wgpu::Texture, depth: &wgpu::TextureView,
        camera: Camera, view: Mat4, output_vp: Mat4, depth_vp: Mat4, white: f32, light: &crate::Lighting, ambient: Vec3, horizon: Vec3, sun_visibility: f32, shadow: Option<(&wgpu::TextureView, Mat4)>) -> Result<Option<wgpu::Texture>, String> {
        self.render_output(gpu,enc,index,input,depth,camera,view,output_vp,depth_vp,white,0.0,light,ambient,horizon,sun_visibility,shadow)
    }

    /// Minimum profiles draw gameplay effects in place, without scene copies,
    /// heat refraction or an HDR intermediate. Exposure zero retains the HDR path.
    #[allow(clippy::too_many_arguments)]
    pub fn render_output(&mut self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,index:usize,input:&wgpu::Texture,depth:&wgpu::TextureView,
        camera:Camera,view:Mat4,output_vp:Mat4,depth_vp:Mat4,white:f32,exposure:f32,light:&crate::Lighting,ambient:Vec3,horizon:Vec3,sun_visibility:f32,shadow:Option<(&wgpu::TextureView,Mat4)>)->Result<Option<wgpu::Texture>,String> {
        let ldr=exposure>0.0;
        // Heat first (it refracts the plain scene), then billows back to front, then embers.
        let mut counts=self.draw_order.prepare(&self.source,self.time,camera.pos);
        if ldr {counts=self.draw_order.limit_minimum(&self.source,self.time,camera,output_vp);}
        let [nh,nb,ne,nc,ns,nm]=counts;
        if self.draw_order.ids.is_empty() {
            if let Some(Some(target))=self.targets.get_mut(index) {
                target.quality.update(0.0,self.time);
                target.quality_stats=SmokeQualityStats{view_index:index,divisor:1,ray_steps:if ldr{24}else{48},soft_size:[input.width(),input.height()],..Default::default()};
            }
            return Ok(None);
        }
        if if ldr {!matches!(input.format(),wgpu::TextureFormat::Rgba8Unorm|wgpu::TextureFormat::Rgba8UnormSrgb)||!input.usage().contains(wgpu::TextureUsages::RENDER_ATTACHMENT)}else{input.format()!=wgpu::TextureFormat::Rgba16Float||!input.usage().contains(wgpu::TextureUsages::COPY_SRC|wgpu::TextureUsages::TEXTURE_BINDING)} {
            return Err("fx requires copyable sampleable HDR input".into());
        }
        if ldr&&self.ldr.as_ref().is_none_or(|(format,_)|*format!=input.format()) {self.ldr=Some((input.format(),self.ldr_pipelines(gpu,input.format())));}
        let size = (input.width(), input.height());
        if self.targets.len() <= index { self.targets.resize_with(index + 1, || None); }
        if self.targets[index].as_ref().is_none_or(|t| t.size != size||t.output.format()!=input.format()) {
            // Retain bounded cache bookkeeping across resize. New uniform and
            // order buffer identities force a miss even if an input is reused.
            let (backgrounds, binds) = self.targets[index].take()
                .map(|previous| (previous.backgrounds, previous.binds)).unwrap_or_default();
            let output = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("fx HDR output (not temporal history)"), size: if ldr {wgpu::Extent3d{width:1,height:1,depth_or_array_layers:1}}else{input.size()}, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format: input.format(),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] });
            let buffer = |label, size, usage| gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
            let output_view = output.create_view(&Default::default());
            self.targets[index] = Some(Target { size, output, output_view,
                uniform: buffer("fx view", std::mem::size_of::<Uniform>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
                order: buffer("fx order", (MAX_PUFFS * 4) as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST),
                backgrounds, binds,
                composite_uniform: buffer("soft FX composite view",std::mem::size_of::<CompositeUniform>() as u64,wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST),
                soft: std::array::from_fn(|_|None), quality: SmokeQuality::default(), quality_stats: SmokeQualityStats::default() });
        }
        let (pixels,visible_volumes)=projected_smoke_work(&self.source,self.time,output_vp,camera.near,size);
        let t = self.targets[index].as_mut().unwrap();
        let requested=t.quality.update(if self.adaptive_smoke && !ldr {pixels}else{0.0},self.time);
        // Bright, sharp fire silhouettes reveal the coarse smoke grid. Keep them
        // at least half resolution; smoke-only aftermath can still use 1/4-1/8.
        let fire_visible=self.source.iter().any(|p|p.kind()==FIRE as u32 && p.timing[0]<=self.time && p.end()>self.time);
        let divisor=if fire_visible {requested.min(2)} else {requested};
        let samples=if ldr {24.0}else{match divisor {1=>48.0,2=>40.0,4=>32.0,_=>24.0}};
        t.quality_stats=SmokeQualityStats{view_index:index,divisor,ray_steps:samples as u32,projected_pixel_layers:pixels,visible_volumes,
            soft_size:[size.0.div_ceil(divisor),size.1.div_ceil(divisor)]};
        let soft_index=match divisor {2=>Some(0),4=>Some(1),8=>Some(2),_=>None};
        if let Some(slot)=soft_index {
            if t.soft[slot].is_none() {
                let small=(size.0.div_ceil(divisor),size.1.div_ceil(divisor));
                let texture=gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("adaptive soft FX layer"),size:wgpu::Extent3d{width:small.0,height:small.1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,
                    format:wgpu::TextureFormat::Rgba16Float,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
                let view=texture.create_view(&Default::default());
                t.soft[slot]=Some(SoftTarget{_texture:texture,view,size:small,binds:Default::default()});
            }
        }
        let depth_from_output=(depth_vp*output_vp.inverse()).to_cols_array_2d();
        let soft_size=t.quality_stats.soft_size;
        let uniform = Uniform { view_proj: output_vp.to_cols_array_2d(), inverse_view_proj: output_vp.inverse().to_cols_array_2d(), depth_vp: depth_vp.to_cols_array_2d(), view: view.to_cols_array_2d(),
            camera_near: camera.pos.extend(camera.near).to_array(), size: [size.0 as f32, size.1 as f32, 1.0 / size.0 as f32, 1.0 / size.1 as f32],
            clock: [self.time, white, exposure, if input.format().is_srgb(){1.0}else{0.0}],
            smoke: [samples,divisor as f32,1.0/soft_size[0] as f32,1.0/soft_size[1] as f32], depth_from_output,
            ambient: ambient.max(Vec3::splat(white * 0.025)).extend(0.0).to_array(),
            sunlight: (light.sun_color * light.sun_lux / std::f32::consts::PI).extend(if shadow.is_some() {light.shadow_strength} else {0.0}).to_array(),
            fog: [light.fog_start,light.fog_end,light.fog_density,light.fog_height_m],
            horizon: horizon.extend(0.0).to_array(),
            sun_vp: shadow.map_or(Mat4::IDENTITY,|s|s.1).to_cols_array_2d(),
            fog_scattering: light.fog_scattering_albedo.extend(light.fog_phase_g).to_array(),
            sun_direction: light.sun_dir.extend(sun_visibility.clamp(0.0,1.0)).to_array(),
            sky_mean: ambient.max(Vec3::ZERO).extend(0.0).to_array() };
        gpu.queue.write_buffer(&t.uniform, 0, bytemuck::bytes_of(&uniform));
        gpu.queue.write_buffer(&t.order, 0, bytemuck::cast_slice(&self.draw_order.ids));
        if !ldr {enc.copy_texture_to_texture(input.as_image_copy(), t.output.as_image_copy(), input.size());}
        let background = t.backgrounds.get(if ldr{&t.output}else{input});
        let bind = t.binds.get(gpu, &self.layout, "cached fx view", &[
            (0, t.uniform.as_entire_binding()), (1, self.puffs.as_entire_binding()), (2, t.order.as_entire_binding()),
            (3, wgpu::BindingResource::TextureView(&background)), (4, wgpu::BindingResource::TextureView(depth)),
            (5, wgpu::BindingResource::Sampler(&self.sampler)), (6, wgpu::BindingResource::TextureView(&self.smoke_view)), (7, wgpu::BindingResource::TextureView(shadow.map_or(depth,|s|s.0)))]);
        let direct_view=ldr.then(||input.create_view(&Default::default()));
        let pipelines=self.ldr.as_ref().map(|(_,p)|p);
        let output_view=direct_view.as_ref().unwrap_or(&t.output_view);
        // Reducing the entire sorted soft bucket retains fire/smoke ordering.
        // Heat and all sharp additive effects still draw at output resolution.
        let draw_billows=|pass:&mut wgpu::RenderPass<'_>| {
            let mut start=nh;
            while start<nh+nb {
                let volume=self.source[self.draw_order.ids[start as usize] as usize].kind()==BAKED_SMOKE as u32;
                let dust=self.source[self.draw_order.ids[start as usize] as usize].dust();
                let mut end=start+1;
                while end<nh+nb && (self.source[self.draw_order.ids[end as usize] as usize].kind()==BAKED_SMOKE as u32)==volume && self.source[self.draw_order.ids[end as usize] as usize].dust()==dust {end+=1;}
                let pipeline=if volume {if ldr{&pipelines.unwrap()[5]}else{&self.smoke}}
                    else if ldr {&pipelines.unwrap()[0]} else if dust {&self.dust} else {&self.billow};
                pass.set_pipeline(pipeline);
                pass.draw(0..if volume||ldr||dust{6}else{BILLOW_VERTICES},start..end);
                start=end;
            }
        };
        if let Some(slot)=soft_index {
            if nh>0 {
                let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("full-resolution heat"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:output_view,depth_slice:None,resolve_target:None,
                    ops:wgpu::Operations{load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
                pass.set_bind_group(0,&bind,&[]); pass.set_pipeline(&self.heat); pass.draw(0..SPHERE_VERTICES,0..nh);
            }
            let soft=t.soft[slot].as_mut().unwrap();
            {
                let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("adaptive smoke and fire"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:&soft.view,depth_slice:None,resolve_target:None,
                    ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
                pass.set_bind_group(0,&bind,&[]); draw_billows(&mut pass);
            }
            let composite_uniform=CompositeUniform{depth_from_output,
                full:[size.0 as f32,size.1 as f32,camera.near,soft_depth_limit(&self.source,self.time,view)],low:[soft.size.0 as f32,soft.size.1 as f32,1.0/soft.size.0 as f32,1.0/soft.size.1 as f32]};
            gpu.queue.write_buffer(&t.composite_uniform,0,bytemuck::bytes_of(&composite_uniform));
            let composite_bind=soft.binds.get(gpu,&self.composite_layout,"cached soft FX composite",&[
                (0,t.composite_uniform.as_entire_binding()),(1,wgpu::BindingResource::TextureView(&soft.view)),(2,wgpu::BindingResource::TextureView(depth))]);
            let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("soft FX depth-aware composite"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:output_view,depth_slice:None,resolve_target:None,
                ops:wgpu::Operations{load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
            pass.set_bind_group(0,&composite_bind,&[]); pass.set_pipeline(&self.composite); pass.draw(0..3,0..1);
        }
        let mut pass=enc.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("full-resolution FX"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:output_view,depth_slice:None,resolve_target:None,
            ops:wgpu::Operations{load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
        pass.set_bind_group(0,&bind,&[]);
        if soft_index.is_none() {
            if !ldr && nh>0 {pass.set_pipeline(&self.heat); pass.draw(0..SPHERE_VERTICES,0..nh);}
            draw_billows(&mut pass);
        }
        if ne>0 {pass.set_pipeline(if ldr{&pipelines.unwrap()[1]}else{&self.ember});pass.draw(0..SPHERE_VERTICES,nh+nb..nh+nb+ne);}
        if nc>0 {pass.set_pipeline(if ldr{&pipelines.unwrap()[2]}else{&self.column});pass.draw(0..COLUMN_VERTICES,nh+nb+ne..nh+nb+ne+nc);}
        if ns>0 {pass.set_pipeline(if ldr{&pipelines.unwrap()[3]}else{&self.streak});pass.draw(0..STREAK_VERTICES,nh+nb+ne+nc..nh+nb+ne+nc+ns);}
        if nm>0 {pass.set_pipeline(if ldr{&pipelines.unwrap()[4]}else{&self.mote});pass.draw(0..MOTE_VERTICES,nh+nb+ne+nc+ns..nh+nb+ne+nc+ns+nm);}
        drop(pass);
        Ok((!ldr).then(||t.output.clone()))
    }
}

/// Lazily created effects renderer shared by all views.
#[derive(Default)]
pub(crate) struct FxSlot(pub Mutex<Option<FxRenderer>>);

impl FxSlot {
    pub fn prepare(&self, gpu: &Gpu) {
        self.0.lock().unwrap().get_or_insert_with(|| FxRenderer::new(gpu));
    }
    pub fn set(&self, gpu: &Gpu, time: f32, puffs: Option<&[Puff]>) {
        let mut g = self.0.lock().unwrap();
        if g.is_none() && puffs.is_none_or(|p| p.is_empty()) { return; }
        g.get_or_insert_with(|| FxRenderer::new(gpu)).set(gpu, time, puffs);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ground_dust_decelerates_without_replacing_live_puffs() {
        let mut fx=super::Effects::default();
        for _ in 0..400 {fx.ground_dust(1.0,glam::Vec3::ZERO,glam::Vec3::X*8.0,0.8);}
        assert_eq!(fx.len(),384);
        assert_eq!(fx.dust_capacity(1.1),0);
        let p=fx.puffs[0];
        let start=p.drift(1.0);let middle=p.drift(1.0+p.timing[1]*0.5);let end=p.drift(p.end());
        assert_eq!(start,glam::Vec3::ZERO);
        assert!((end.x-8.0).abs()<1e-4);
        assert!(middle.x>7.0 && end.x-middle.x<1.0);
        assert_eq!(middle.y,0.0);
        assert_eq!(fx.dust_capacity(3.0),384);
    }

    use super::*;

    #[test]
    fn minimum_overlap_keeps_one_fire_and_all_gameplay_warnings_in_both_eyes() {
        let mut fx=Effects::with_explosion_budget(ExplosionBudget::MINIMUM);
        for i in 0..8 {fx.explosion(0.,Vec3::new(i as f32*0.01,0.,-0.5),8.,Vec3::ONE);}
        fx.column(AdditiveColumn{life:10.,..Default::default()}).unwrap();fx.emit_pending(0.2);
        let cam=Camera{pos:Vec3::ZERO,target:Vec3::NEG_Z,near:0.05,fov_y:1.};
        let projection=Mat4::perspective_infinite_reverse_rh(1.,1.,cam.near);
        let mut order=DrawOrder::default();order.prepare(&fx.puffs,0.2,cam.pos);
        let count=order.limit_minimum(&fx.puffs,0.2,cam,projection);
        assert_eq!(count[1],1);assert_eq!(count[3],1);
        assert_eq!(fx.puffs[order.billows[0].0 as usize].kind(),1);
        let first=order.ids.clone();let right=Camera{pos:Vec3::X*0.064,target:Vec3::X*0.064+Vec3::NEG_Z,..cam};
        order.prepare(&fx.puffs,0.2,right.pos);order.limit_minimum(&fx.puffs,0.2,right,projection*Mat4::look_at_rh(right.pos,right.target,Vec3::Y));
        assert_eq!(order.ids,first,"stereo eyes must share coverage selection");
    }

    #[test]
    fn fireball_light_has_stable_reach_and_a_soft_short_tail() {
        for radius in [0.5, 6.5, 24.0, 50.0] {
            let mut fx = Effects::default();
            fx.pops.push(LightPop { pos: Vec3::ZERO, birth: 0.0, kind: 1, radius, color: Vec3::ONE });
            let life = (0.25_f32 + radius * 0.02).clamp(0.30, 0.75);
            let first = fx.lights(0.0)[0];
            let mut previous = first.intensity_cd.length();
            for i in 1..100 {
                let light = fx.lights(life * i as f32 / 100.0)[0];
                assert_eq!(light.range_m, first.range_m);
                let energy = light.intensity_cd.length();
                assert!(energy <= previous);
                previous = energy;
            }
            assert!(previous < first.intensity_cd.length() * 0.001);
            assert!(fx.lights(life).is_empty());
        }
    }

    #[test]
    fn view_order_preserves_layers_lifetimes_and_equal_depth_source_order() {
        let puff = |kind, z, birth, life| Puff { center_kind: [0.0, 0.0, z, kind],
            timing: [birth, life, 0.0, 0.0], ..Default::default() };
        let source = [
            puff(FIRE, -1.0, 0.0, 1.0), puff(SMOKE, -3.0, 0.0, 1.0), puff(FIRE, 3.0, 0.0, 1.0),
            puff(EMBER, 0.0, 0.0, 1.0), puff(HEAT, 0.0, 0.0, 1.0),
            puff(COLUMN, 0.0, 0.0, 1.0), puff(STREAK, 0.0, 0.0, 1.0),
            puff(FIRE, -5.0, -1.0, 1.0), puff(HEAT, 0.0, 1.0, 1.0), puff(MOTE, 0.0, 0.0, 1.0),
        ];
        let mut order = DrawOrder::default();
        assert_eq!(order.prepare(&source, 0.0, Vec3::ZERO), [1, 3, 1, 1, 1, 1]);
        assert_eq!(order.ids, [4, 1, 2, 0, 3, 5, 6, 9]);
        // Another view must recompute distance, retaining the same layer order.
        assert_eq!(order.prepare(&source, 0.0, Vec3::Z * 4.0), [1, 3, 1, 1, 1, 1]);
        assert_eq!(order.ids, [4, 1, 0, 2, 3, 5, 6, 9]);
        assert_eq!(order.prepare(&source, 1.0, Vec3::ZERO), [1, 0, 0, 0, 0, 0]);
        assert_eq!(order.ids, [8]);
        assert_eq!(order.prepare(&source, 2.0, Vec3::ZERO), [0; 6]);
        assert!(order.ids.is_empty());
    }

    #[test]
    fn repeated_views_reuse_order_storage_and_match_stable_sort() {
        // Deliberately repeat positions across smoke and fire to exercise many
        // equal-depth ties while the camera moves between independent views.
        let source: Vec<_> = (0..256).map(|i| Puff {
            center_kind: [(i % 5) as f32, 0.0, (i % 7) as f32, (i % 7) as f32],
            timing: [0.0, 1.0, 0.0, 0.0], ..Default::default()
        }).collect();
        let capacities = |order: &DrawOrder| [order.heat.capacity(), order.billows.capacity(),
            order.embers.capacity(), order.columns.capacity(), order.streaks.capacity(), order.motes.capacity(), order.ids.capacity()];
        let mut order = DrawOrder::default();
        order.prepare(&source, 0.0, Vec3::ZERO);
        let allocated = capacities(&order);
        for camera in [Vec3::ZERO, Vec3::X * 4.0, Vec3::Z * -3.0, Vec3::ZERO] {
            let mut stable: Vec<_> = source.iter().enumerate().filter(|(_, p)| p.kind() <= 1)
                .map(|(i, p)| (i as u32, Vec3::from_slice(&p.center_kind[..3]).distance_squared(camera))).collect();
            stable.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            order.prepare(&source, 0.0, camera);
            assert_eq!(order.billows, stable);
            assert_eq!(capacities(&order), allocated);
            order.prepare(&[], 0.0, camera);
            assert!(order.ids.is_empty());
            assert_eq!(capacities(&order), allocated);
        }
    }

    #[test]
    fn fx_wgsl_validates_and_layouts_match() {
        let module = naga::front::wgsl::parse_str(shader_source()).unwrap_or_else(|e| panic!("{}", e.emit_to_string(shader_source())));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        assert_eq!(std::mem::size_of::<Puff>(), 112);
        assert_eq!(std::mem::size_of::<Uniform>(), 560);
        assert_eq!(std::mem::size_of::<CompositeUniform>(),96);
        let source=composite_shader_source();
        let module=naga::front::wgsl::parse_str(source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap();
    }

    #[test]
    fn an_explosion_follows_the_godot_recipe() {
        let mut fx = Effects::default();
        fx.explosion(1.0, Vec3::ZERO, 9.0, Vec3::new(1.0, 0.7, 0.2));
        let count = |k: f32| fx.puffs.iter().filter(|p| p.center_kind[3] == k).count();
        assert_eq!(count(SMOKE), 9);
        assert_eq!(count(FIRE), 7 + 6);
        assert_eq!(count(EMBER), 45);
        assert_eq!(count(HEAT), 1);
        // Smoke lingers longest: 0.5 + 9 * 0.13 s.
        let last = fx.puffs.iter().map(Puff::end).fold(0.0, f32::max);
        assert!((last - (1.0 + 0.5 + 9.0 * 0.13)).abs() < 1e-4);
        // Lights: both at birth, only the fireball glow later, none at the end.
        assert_eq!(fx.lights(1.0).len(), 2);
        assert_eq!(fx.lights(1.2).len(), 1);
        assert!(fx.lights(2.2).is_empty());
        // Small blasts get no heat shell.
        let mut small = Effects::default();
        small.explosion(0.0, Vec3::ZERO, 4.0, Vec3::ONE);
        assert!(small.puffs.iter().all(|p| p.center_kind[3] != HEAT));
    }

    #[test]
    fn baked_smoke_changes_only_explosion_smoke_and_keeps_random_sequence() {
        let mut plain = Effects::default();
        let mut baked = Effects::default();
        baked.set_baked_smoke(true);
        for time in [0.0, 0.5] {
            plain.explosion(time, Vec3::new(1.0, 2.0, -3.0), 6.5, Vec3::ONE);
            baked.explosion(time, Vec3::new(1.0, 2.0, -3.0), 6.5, Vec3::ONE);
        }
        let ordinary = |fx: &Effects| fx.puffs.iter().copied()
            .filter(|p| p.kind() != SMOKE as u32 && p.kind() != BAKED_SMOKE as u32).collect::<Vec<_>>();
        assert_eq!(ordinary(&plain), ordinary(&baked));
        assert_eq!(plain.rng, baked.rng);
        assert_eq!(baked.puffs.iter().filter(|p| p.kind() == BAKED_SMOKE as u32).count(), 2);
        assert!(!baked.puffs.iter().any(|p| p.kind() == SMOKE as u32));
        baked.blood_mist(0.8, Vec3::ZERO, 3.0, 1);
        assert!(baked.puffs.iter().any(|p| p.kind() == SMOKE as u32));
        baked.clear();
        baked.explosion(1.0, Vec3::ZERO, 3.0, Vec3::ONE);
        assert_eq!(baked.puffs.iter().filter(|p| p.kind() == BAKED_SMOKE as u32).count(), 1);
        baked.set_baked_smoke(false);
        baked.clear();
        baked.explosion(1.0, Vec3::ZERO, 3.0, Vec3::ONE);
        assert!(!baked.puffs.iter().any(|p| p.kind() == BAKED_SMOKE as u32));
    }

    #[test]
    fn baked_smoke_retires_over_budget_without_retiming_its_motion() {
        let mut fx = Effects::default();
        fx.set_baked_smoke(true);
        for i in 0..MAX_ACTIVE_SMOKE_VOLUMES + 4 {
            fx.explosion(i as f32 * 0.01, Vec3::ZERO, 2.0, Vec3::ONE);
        }
        let volumes: Vec<_> = fx.puffs.iter().filter(|p| p.kind() == BAKED_SMOKE as u32).collect();
        assert_eq!(volumes.len(), MAX_ACTIVE_SMOKE_VOLUMES + 4);
        assert_eq!(volumes.iter().filter(|p| p.extra[1] == 0.0).count(), MAX_ACTIVE_SMOKE_VOLUMES);
        assert_eq!(volumes[0].timing[0], 0.0, "oldest plume stays present during retirement");
        assert!((volumes[0].extra[0] - 0.64).abs() < 1e-6);
        assert_eq!(volumes[0].extra[1], SMOKE_RETIRE_SECONDS);
        assert!((volumes[0].end() - 0.94).abs() < 1e-6);
        for p in &volumes {
            assert!((Vec3::from_slice(&p.ax[..3]).length() - 4.8).abs() < 1e-5);
            assert!(Vec3::from_slice(&p.ax[..3]).dot(Vec3::from_slice(&p.az[..3])).abs() < 1e-4);
            assert_eq!(p.timing[1], SMOKE_LIFETIME, "retirement must not speed the baked motion up");
        }
        fx.explosion(6.0, Vec3::ZERO, 2.0, Vec3::ONE);
        assert_eq!(fx.puffs.iter().filter(|p| p.kind() == BAKED_SMOKE as u32).count(), 1);
    }

    #[test]
    fn baked_smoke_hard_bound_preserves_established_plumes_during_new_bursts() {
        let mut fx = Effects::default();
        fx.set_baked_smoke(true);
        for _ in 0..MAX_ACTIVE_SMOKE_VOLUMES { fx.explosion(0.0, Vec3::ZERO, 2.0, Vec3::ONE); }
        for _ in 0..MAX_SMOKE_VOLUMES * 2 { fx.explosion(1.0, Vec3::ZERO, 2.0, Vec3::ONE); }
        let volumes: Vec<_> = fx.puffs.iter().filter(|p| p.kind() == BAKED_SMOKE as u32).collect();
        assert_eq!(volumes.len(), MAX_SMOKE_VOLUMES);
        assert_eq!(volumes.iter().filter(|p| p.timing[0] == 0.0).count(), MAX_ACTIVE_SMOKE_VOLUMES,
            "hard overflow should discard unseen new plumes before established ones");
        assert_eq!(volumes.iter().filter(|p| p.extra[1] == 0.0).count(), MAX_ACTIVE_SMOKE_VOLUMES);
        assert!(volumes.iter().filter(|p| p.extra[1] > 0.0).all(|p| (p.end()-1.3).abs()<1e-6));
        fx.explosion(1.31, Vec3::ZERO, 2.0, Vec3::ONE);
        assert!(fx.puffs.iter().filter(|p| p.kind() == BAKED_SMOKE as u32).all(|p| p.end()>1.31));
        assert!(fx.puffs.iter().filter(|p| p.kind() == BAKED_SMOKE as u32).count()<=MAX_SMOKE_VOLUMES);
    }

    #[test]
    fn smoke_retirement_expiry_does_not_change_other_effect_kinds() {
        let retiring = Puff { center_kind:[0.0,0.0,0.0,BAKED_SMOKE], timing:[1.0,4.6,0.0,0.0],
            extra:[2.0,0.3,0.0,0.0], ..Default::default() };
        assert_eq!(retiring.end(), 2.3);
        let natural_end = retiring.timing[0]+retiring.timing[1];
        for kind in [SMOKE,FIRE,EMBER,HEAT,COLUMN,STREAK,MOTE] {
            let p=Puff {center_kind:[0.0,0.0,0.0,kind],..retiring};
            assert_eq!(p.end(), natural_end);
        }
        let natural = Puff { extra:[0.0;4],..retiring };
        assert_eq!(natural.end(), natural_end);
    }

    #[test]
    fn volume_and_fire_share_view_order_and_lifetime_clipping() {
        let source = [
            Puff { center_kind: [0.0, 0.0, -4.0, FIRE], timing: [0.0, 1.0, 0.0, 0.0], ..Default::default() },
            Puff { center_kind: [0.0, -1.0, -6.0, BAKED_SMOKE], ay: [0.0, 2.0, 0.0, 0.0],
                timing: [0.0, 2.6, 0.0, 0.0], ..Default::default() },
        ];
        let mut order = DrawOrder::default();
        assert_eq!(order.prepare(&source, 0.5, Vec3::ZERO), [0, 2, 0, 0, 0, 0]);
        assert_eq!(order.ids, [1, 0]);
        order.prepare(&source, 0.5, Vec3::NEG_Z * 10.0);
        assert_eq!(order.ids, [0, 1]);
        order.prepare(&source, 1.0, Vec3::ZERO);
        assert_eq!(order.ids, [1]);
        order.prepare(&source, 2.6, Vec3::ZERO);
        assert!(order.ids.is_empty());
    }

    #[test]
    fn baked_smoke_asset_has_empty_endpoints_and_changes_between_frames() {
        let frame_bytes = (SMOKE_SIZE.pow(3) * 2) as usize;
        assert_eq!(SMOKE_BYTES.len(), frame_bytes * SMOKE_FRAMES as usize);
        assert!(SMOKE_BYTES[..frame_bytes].iter().step_by(2).all(|v| *v == 0));
        assert!(SMOKE_BYTES[SMOKE_BYTES.len() - frame_bytes..].iter().step_by(2).all(|v| *v == 0));
        let a = &SMOKE_BYTES[frame_bytes * 8..frame_bytes * 9];
        let b = &SMOKE_BYTES[frame_bytes * 16..frame_bytes * 17];
        assert!(a.iter().step_by(2).any(|v| *v > 128));
        assert!(a.iter().zip(b).step_by(2).filter(|(a, b)| a != b).count() > 1000);
        assert!(a.iter().skip(1).step_by(2).any(|v| *v < 200), "baked self-shadow must carry shape");
    }

    #[test]
    fn baked_smoke_bounds_cover_quantized_density_and_filter_support() {
        let metadata = include_str!("../../../assets/fx/explosion-smoke-bounds.wgsl");
        let rows: Vec<[f32; 4]> = metadata.lines().filter_map(|line| line.trim().strip_prefix("vec4<f32>("))
            .map(|line| {
                let fields: Vec<f32> = line.strip_suffix("),").expect("generated vec4 row terminator")
                    .split(',').map(|field| field.trim().parse().expect("generated finite float")).collect();
                fields.try_into().expect("generated bounds need four components")
            }).collect();
        let frames = SMOKE_FRAMES as usize;
        let side = SMOKE_SIZE as usize;
        let frame_bytes = side.pow(3) * 2;
        const EPS: f32 = 1e-6;
        assert_eq!(rows.len(), frames * 2, "expected MIN rows followed by MAX rows for every frame");
        assert!(metadata.find("const SMOKE_BOUNDS_MIN:").unwrap() < metadata.find("const SMOKE_BOUNDS_MAX:").unwrap());
        assert_eq!(SMOKE_BYTES.len(), frames * frame_bytes);
        for (frame, bytes) in SMOKE_BYTES.chunks_exact(frame_bytes).enumerate() {
            let (lo, hi) = (rows[frame], rows[frames + frame]);
            assert!(lo.iter().chain(hi.iter()).all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                "invalid normalized bounds in frame {frame}");
            let mut peak = 0u8;
            for (voxel, channels) in bytes.chunks_exact(2).enumerate() {
                let density = channels[0];
                peak = peak.max(density);
                if density == 0 { continue; }
                let xyz = [voxel % side, (voxel / side) % side, voxel / (side * side)];
                for axis in 0..3 {
                    // smoke_frame maps normalized uvw onto voxel centres at
                    // i/(SIZE-1). Trilinear support extends one voxel each way.
                    let support_min = xyz[axis].saturating_sub(1) as f32 / (side - 1) as f32;
                    let support_max = (xyz[axis] + 1).min(side - 1) as f32 / (side - 1) as f32;
                    assert!(lo[axis] <= support_min + EPS && hi[axis] + EPS >= support_max,
                        "frame {frame}, voxel {xyz:?}, axis {axis}: [{}, {}] clips filter support [{support_min}, {support_max}]",
                        lo[axis], hi[axis]);
                }
            }
            if peak == 0 {
                assert!(hi[3].abs() <= EPS, "empty frame {frame} must have MAX.w = 0");
            } else {
                assert!(hi[3] + EPS >= peak as f32 / 255.0,
                    "frame {frame} peak {} underestimates quantized density {}", hi[3], peak as f32 / 255.0);
            }
        }
    }

    #[test]
    fn additive_effects_preserve_ramps_and_use_compact_geometry() {
        let mut fx = Effects::default();
        let column = AdditiveColumn { position: Vec3::new(2.0, -4.0, 6.0), birth: 7.0, life: 3.0,
            height: 100.0, radius: [8.0, 0.5], radius_power: 2.0,
            color: [Vec3::new(0.2, 0.7, 1.0), Vec3::ONE], color_transition: [0.4, 0.9],
            opacity: [0.01, 0.8], opacity_power: 4.2, radiance: [100.0, 50_000.0],
            radiance_power: 2.35, pulse_radians_per_second: 9.0, pulse_radiance: 200.0,
            bottom_radius_scale: 1.03 };
        fx.column(column).unwrap();
        let c = fx.puffs[0];
        assert_eq!(c.kind(), 4);
        assert_eq!(c.center_kind, [2.0, -4.0, 6.0, COLUMN]);
        assert_eq!(c.ax, [8.0, 0.5, 100.0, 2.0]);
        assert_eq!(c.body, [100.0, 50_000.0, 4.2, 2.35]);
        assert_eq!(c.end(), 10.0);

        let streak = AdditiveStreak { position: Vec3::X, travel: Vec3::Y * 30.0,
            birth: 10.0, life: 0.4, radius: [0.1, 0.2], length: [16.0, 24.0],
            color: Vec3::new(0.8, 0.95, 1.0), radiance: 40_000.0, opacity: 0.7,
            fade_in: 0.0, fade_out: 0.85, motion_power: 1.0 };
        fx.streak(streak).unwrap();
        let s = fx.puffs[1];
        assert_eq!(s.kind(), 5);
        assert_eq!(s.ax, [0.0, 30.0, 0.0, 1.0]);
        assert_eq!(s.ay, [0.1, 0.2, 16.0, 24.0]);
        assert_eq!(s.body, [0.7, 0.0, 0.85, 0.0]);
        assert!(fx.changed);
        assert_eq!(COLUMN_VERTICES, 6);
        assert_eq!(STREAK_VERTICES, 72);
        assert!(STREAK_VERTICES < SPHERE_VERTICES / 8);
    }

    #[test]
    fn invalid_additive_effects_do_not_enter_gpu_pool() {
        let mut fx = Effects::default();
        let c = AdditiveColumn::default();
        for bad in [
            AdditiveColumn { birth: f32::NAN, ..c },
            AdditiveColumn { life: 0.0, ..c },
            AdditiveColumn { life: 121.0, ..c },
            AdditiveColumn { radius: [-1.0, 1.0], ..c },
            AdditiveColumn { color_transition: [0.5, 0.5], ..c },
            AdditiveColumn { radiance: [f32::INFINITY, 1.0], ..c },
            AdditiveColumn { opacity: [0.1, 1.1], ..c },
            AdditiveColumn { radius_power: 0.0, ..c },
            AdditiveColumn { bottom_radius_scale: 0.0, ..c },
        ] { assert!(fx.column(bad).is_err()); }
        let s = AdditiveStreak::default();
        for bad in [
            AdditiveStreak { travel: Vec3::splat(f32::NAN), ..s },
            AdditiveStreak { life: -1.0, ..s },
            AdditiveStreak { radius: [-1.0, 0.0], ..s },
            AdditiveStreak { length: [0.1, f32::INFINITY], ..s },
            AdditiveStreak { color: Vec3::NEG_Y, ..s },
            AdditiveStreak { radiance: -1.0, ..s },
            AdditiveStreak { fade_in: 0.5, fade_out: 0.6, ..s },
            AdditiveStreak { motion_power: 33.0, ..s },
        ] { assert!(fx.streak(bad).is_err()); }
        let m = AdditiveMote::default();
        for bad in [AdditiveMote { radius: f32::NAN, ..m },
            AdditiveMote { motion_power: 0.0, ..m },
            AdditiveMote { motion_delay: -1.0, ..m },
            AdditiveMote { motion_delay: f32::NAN, ..m },
            AdditiveMote { motion_delay: m.life, ..m },
            AdditiveMote { fade_in: 0.8, fade_out: 0.8, ..m }] {
            assert!(fx.mote(bad).is_err());
        }
        assert!(fx.is_empty());
        assert!(!fx.changed);
    }

    #[test]
    fn scheduled_additive_effects_are_bounded_before_upload() {
        let mut fx = Effects::default();
        for i in 0..MAX_PUFFS + 3 {
            fx.streak(AdditiveStreak { birth: i as f32, ..Default::default() }).unwrap();
        }
        assert_eq!(fx.len(), MAX_PUFFS);
        assert_eq!(fx.puffs[0].timing[0], 3.0);
    }
    #[test]
    fn soft_depth_bound_contains_cube_and_billow_growth_with_signed_rise() {
        let view=Mat4::look_at_rh(Vec3::new(4.0,8.0,12.0),Vec3::new(-1.0,2.0,0.0),Vec3::Y);
        let base=Vec3::new(1.0,2.0,-3.0);
        let ax=Vec3::new(3.0,1.0,0.0);
        let ay=Vec3::new(0.0,4.0,0.0);
        let az=Vec3::new(0.0,1.0,5.0);
        let puff=Puff{center_kind:base.extend(BAKED_SMOKE).to_array(),ax:ax.extend(0.0).to_array(),ay:ay.extend(0.0).to_array(),az:az.extend(0.0).to_array(),timing:[0.0,4.6,0.0,0.0],..Default::default()};
        let cube_limit=soft_depth_limit(&[puff],0.85,view);
        for x in [-0.5,0.5] {for y in [0.0,1.0] {for z in [-0.5,0.5] {
            assert!(-view.transform_point3(base+ax*x+ay*y+az*z).z<cube_limit);
        }}}
        for kind in [SMOKE,FIRE] {for rise in [-6.0,6.0] {
            let billow=Puff{center_kind:base.extend(kind).to_array(),timing:[0.0,4.6,rise,0.0],..puff};
            let limit=soft_depth_limit(&[billow],0.85,view);
            // Exercise the outer geometric envelope independently of the
            // procedural lump phases; every animated shape fits inside it.
            for x in -3..=3 {for y in -3..=3 {for z in -3..=3 {
                let direction=Vec3::new(x as f32,y as f32,z as f32).normalize_or_zero();
                let local=direction*0.82;
                for rise_fraction in [0.0,1.0] {
                    let world=base+ax*local.x+ay*local.y+az*local.z+Vec3::Y*rise*rise_fraction;
                    assert!(-view.transform_point3(world).z<limit,"kind {kind}, rise {rise}");
                }
            }}}
        }}
        let distant=Puff{center_kind:[0.0,0.0,-100_000.0,BAKED_SMOKE],..puff};
        assert!(soft_depth_limit(&[distant],0.85,view)>60_000.0,"a clamped depth guide cannot bypass a farther volume");
        let future=Puff{timing:[1.0,4.6,0.0,0.0],..distant};
        let expired=Puff{timing:[-5.0,4.6,0.0,0.0],..distant};
        let sharp=Puff{center_kind:[0.0,0.0,-100_000.0,EMBER],..puff};
        assert_eq!(soft_depth_limit(&[puff,future,expired,sharp],0.85,view),cube_limit);
    }

    #[test]
    fn adaptive_smoke_quality_changes_with_work_and_recovers_without_thrashing() {
        let mut quality=SmokeQuality::default();
        assert_eq!(quality.update(1_000_000.0,0.0),2);
        assert_eq!(quality.update(400_000.0,0.05),2,"hold near a tier boundary");
        assert_eq!(quality.update(100_000.0,0.10),2);
        assert_eq!(quality.update(100_000.0,0.34),2);
        assert_eq!(quality.update(100_000.0,0.36),1);
        assert_eq!(quality.update(100_000_000.0,0.4),8,"coarsen immediately under load");
        assert_eq!(quality.update(100_000.0,0.2),1,"rewinding resets prior quality");
        assert_eq!(quality.update(100_000_000.0,0.4),8);
        assert_eq!(quality.update(100_000.0,2.0),1,"a long inactive gap resets quality");
        assert_eq!(quality.update(100_000_000.0,2.1),8);
        assert_eq!(quality.update(0.0,2.2),1,"no visible baked smoke uses the original path");
    }

    #[test]
    fn adaptive_smoke_projection_counts_only_live_visible_bounds() {
        let mut puff=Puff{center_kind:[0.0,0.0,0.0,BAKED_SMOKE],ax:[2.0,0.0,0.0,0.0],ay:[0.0,2.0,0.0,0.0],az:[0.0,0.0,2.0,0.0],timing:[0.0,4.6,0.0,0.0],..Default::default()};
        let projection=Mat4::perspective_infinite_reverse_rh(std::f32::consts::FRAC_PI_2,1.0,0.05);
        let vp=projection*Mat4::look_at_rh(Vec3::new(0.0,1.0,10.0),Vec3::Y,Vec3::Y);
        let (far,count)=projected_smoke_work(&[puff],0.85,vp,0.05,(1000,1000));
        assert_eq!(count,1); assert!(far>1_000.0 && far<100_000.0);
        let inside=projection*Mat4::look_at_rh(Vec3::Y,Vec3::Y+Vec3::NEG_Z,Vec3::Y);
        assert_eq!(projected_smoke_work(&[puff],0.85,inside,0.05,(1000,1000)),(1_000_000.0,1));
        puff.center_kind[0]=1000.0;
        assert_eq!(projected_smoke_work(&[puff],0.85,vp,0.05,(1000,1000)),(0.0,0));
        puff.center_kind[0]=0.0; puff.center_kind[2]=20.0;
        assert_eq!(projected_smoke_work(&[puff],0.85,vp,0.05,(1000,1000)),(0.0,0));
        puff.center_kind[2]=0.0;
        assert_eq!(projected_smoke_work(&[puff],-0.1,vp,0.05,(1000,1000)),(0.0,0));
        assert_eq!(projected_smoke_work(&[puff],4.6,vp,0.05,(1000,1000)),(0.0,0));
    }

}
