//! The rifle's stats, a port of Godot's `weapon.gd` (`class_name Weapon`):
//! the single source of truth that every card mutates. Field names match
//! the Godot ones (mods address them by name); gameplay reads the getters.
use glam::Vec3;
use serde::{Deserialize, Serialize};

pub const BASE_DAMAGE: f32 = 22.0;
pub const MIN_DAMAGE: f32 = 1.0;
pub const BASE_FIRE_INTERVAL: f32 = 0.22;
pub const BASE_MAG_SIZE: i32 = 5;
pub const BASE_RELOAD_TIME: f32 = 1.2;
pub const BASE_HEADSHOT_MULT: f32 = 2.0;
/// Radians, about 0.46 degrees.
pub const BASE_SPREAD: f32 = 0.008;
pub const BASE_BULLET_COLOR: [f32; 3] = [1.0, 0.9, 0.3];
pub const MAX_SHOTS_PER_TRIGGER: i32 = 16;
pub const MAX_PROJECTILES_PER_SECOND: f32 = 240.0;
pub const BASE_RECOIL: f32 = 0.018;
pub const KNOCKBACK_PER_DAMAGE: f32 = 0.12;
pub const BLAST_KNOCKBACK_MULT: f32 = 1.5;
pub const REFERENCE_KNOCKBACK: f32 = 25.0 * KNOCKBACK_PER_DAMAGE;
/// Matches the player's gravity: bullets fall like everything else.
pub const BASE_BULLET_DROP: f32 = 30.0;
pub const BASE_BULLET_SPEED: f32 = 165.0;

/// The RMB ability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Special { #[default] Grenade, Teleport, Zoom, Sword, AirStrike, IonCannon, ClusterGrenade }

pub const SPECIALS: [(Special, &str); 7] = [
    (Special::Grenade, "grenade"), (Special::Teleport, "teleport"), (Special::Zoom, "zoom"), (Special::Sword, "sword"),
    (Special::AirStrike, "air_strike"), (Special::IonCannon, "ion_cannon"), (Special::ClusterGrenade, "cluster_grenade"),
];

impl Special {
    pub fn named(n: &str) -> Option<Special> { SPECIALS.iter().find(|s| s.1 == n).map(|s| s.0) }
    pub fn name(self) -> &'static str { SPECIALS.iter().find(|s| s.0 == self).map_or("grenade", |s| s.1) }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Weapon {
    // Multiplicative (cards multiply).
    pub damage_mult: f32,
    /// > 1 is faster.
    pub fire_rate_mult: f32,
    /// > 1 is faster.
    pub reload_mult: f32,
    /// On top of BASE_HEADSHOT_MULT.
    pub headshot_mult: f32,
    // Additive.
    pub mag_size_bonus: i32,
    /// Bullets per trigger beyond the first.
    pub extra_projectiles: i32,
    /// Wall bounces.
    pub ricochet_count: i32,
    /// Enemy bullets that bounce off your body.
    pub body_ricochet_count: i32,
    /// Multiplier on the final round in the magazine.
    pub last_shot_damage_mult: f32,
    pub silencer_stacks: i32,
    // Behaviour.
    /// Radians; random yaw and pitch offset per shot.
    pub spread: f32,
    pub recoil_per_shot: f32,
    /// Fraction of damage dealt returned as health.
    pub lifesteal: f32,
    /// Per-bullet splash radius (m) and damage at the centre.
    pub explosive_radius: f32,
    pub explosive_damage: f32,
    pub move_speed_mult: f32,
    pub knockback_mult: f32,
    /// < 1 recharges faster.
    pub special_cooldown_mult: f32,
    pub melee_damage_mult: f32,
    pub melee_scale: f32,
    /// Degrees per second of steering toward the closest target ahead.
    pub homing: f32,
    /// Bonus fraction of bullet damage dealt again over time.
    pub damage_over_time: f32,
    pub dot_duration: f32,
    /// < 1: the target's movement multiplier.
    pub slow_on_hit: f32,
    pub slow_duration: f32,
    pub grow_damage_per_meter: f32,
    pub ricochet_damage_mult: f32,
    /// Rounds restored when dealing damage.
    pub reload_on_hit: i32,
    /// Revives available each round.
    pub phoenix_revives: i32,
    /// World surfaces a bullet can drill through.
    pub world_pierce_count: i32,
    /// Mines planted on bullet impact.
    pub impact_mines: i32,
    // Visuals.
    pub bullet_color: [f32; 3],
    pub bullet_speed_mult: f32,
    pub bullet_scale: f32,
    /// m/s^2 downward on bullets (0: laser flat).
    pub bullet_drop: f32,
    // Body (head and torso hitboxes grow with these).
    pub head_scale: f32,
    pub body_scale: f32,
    pub body_scale_axes: Vec3,
    /// Cosmetic per-bone scales (Mixamo bone name), multiplied by cards.
    pub bone_warps: Vec<(String, Vec3)>,
    pub max_hp_bonus: i32,
    /// Air jumps beyond the default double jump.
    pub extra_jumps: i32,
    // RMB.
    pub special: Special,
    pub special_echo_count: i32,
    pub special_reload_amount: i32,
    pub special_cooldown_refund_on_hit: f32,
    pub teleport_blast_radius: f32,
    // Dash (Shift).
    pub dash_iframes: bool,
    pub dash_end_melee: bool,
    pub dash_spawn_bomb: bool,
}

impl Default for Weapon {
    fn default() -> Self {
        Weapon {
            damage_mult: 1.0, fire_rate_mult: 1.0, reload_mult: 1.0, headshot_mult: 1.0,
            mag_size_bonus: 0, extra_projectiles: 0, ricochet_count: 0, body_ricochet_count: 0, last_shot_damage_mult: 1.0, silencer_stacks: 0,
            spread: BASE_SPREAD, recoil_per_shot: BASE_RECOIL, lifesteal: 0.0, explosive_radius: 0.0, explosive_damage: 0.0,
            move_speed_mult: 1.0, knockback_mult: 1.0, special_cooldown_mult: 1.0, melee_damage_mult: 1.0, melee_scale: 1.0,
            homing: 0.0, damage_over_time: 0.0, dot_duration: 3.0, slow_on_hit: 1.0, slow_duration: 0.0, grow_damage_per_meter: 0.0,
            ricochet_damage_mult: 1.0, reload_on_hit: 0, phoenix_revives: 0, world_pierce_count: 0, impact_mines: 0,
            bullet_color: BASE_BULLET_COLOR, bullet_speed_mult: 1.0, bullet_scale: 1.0, bullet_drop: BASE_BULLET_DROP,
            head_scale: 1.0, body_scale: 1.0, body_scale_axes: Vec3::ONE, bone_warps: Vec::new(), max_hp_bonus: 0, extra_jumps: 0,
            special: Special::Grenade, special_echo_count: 0, special_reload_amount: 0, special_cooldown_refund_on_hit: 0.0, teleport_blast_radius: 0.0,
            dash_iframes: false, dash_end_melee: false, dash_spawn_bomb: false,
        }
    }
}

impl Weapon {
    pub fn damage(&self) -> f32 { (BASE_DAMAGE * self.damage_mult).max(MIN_DAMAGE) }

    pub fn damage_for_shot(&self, last_in_mag: bool) -> f32 {
        let d = self.damage();
        if last_in_mag && self.last_shot_damage_mult > 1.0 { d * self.last_shot_damage_mult } else { d }
    }

    /// Seconds between triggers, in f64 like Godot (it decides the tick).
    pub fn fire_interval(&self) -> f64 {
        let raw = 0.22f64 / (self.fire_rate_mult as f64).max(0.01);
        let budget = self.shots_per_trigger() as f64 / MAX_PROJECTILES_PER_SECOND as f64;
        raw.max(budget)
    }

    pub fn mag_size(&self) -> u32 { (BASE_MAG_SIZE + self.mag_size_bonus).max(1) as u32 }
    pub fn reload_time(&self) -> f64 { 1.2f64 / (self.reload_mult as f64).max(0.01) }
    pub fn headshot(&self) -> f32 { BASE_HEADSHOT_MULT * self.headshot_mult }
    pub fn spread(&self) -> f32 { self.spread.max(0.0) }

    /// Precise guns kick less: recoil scales with sqrt(spread / base), 0.15..1.
    pub fn recoil(&self) -> f32 {
        let ratio = self.spread() / BASE_SPREAD;
        let m = ratio.max(0.0).sqrt().clamp(0.15, 1.0);
        (self.recoil_per_shot * m).max(0.0)
    }

    pub fn shots_per_trigger(&self) -> u32 { (1 + self.extra_projectiles).clamp(1, MAX_SHOTS_PER_TRIGGER) as u32 }
    pub fn projectiles_per_second(&self) -> f32 { (self.shots_per_trigger() as f64 / self.fire_interval().max(0.001)) as f32 }
    pub fn bullet_speed(&self) -> f32 { BASE_BULLET_SPEED * self.bullet_speed_mult }

    /// Visual size of the round, from damage (sqrt keeps it sane).
    pub fn bullet_scale(&self) -> f32 { self.bullet_scale * self.damage_mult.max(0.1).sqrt() }
    pub fn bullet_scale_for_shot(&self, last_in_mag: bool) -> f32 {
        if last_in_mag && self.last_shot_damage_mult > 1.0 { self.bullet_scale * (self.damage_mult * self.last_shot_damage_mult).max(0.1).sqrt() } else { self.bullet_scale() }
    }

    pub fn melee_damage(&self) -> i32 { (70.0 * self.melee_damage_mult) as i32 }

    pub fn knockback_from_damage(damage: f32, mult: f32, blast: bool) -> f32 {
        let f = damage.max(0.0) * KNOCKBACK_PER_DAMAGE;
        (if blast { f * BLAST_KNOCKBACK_MULT } else { f }) * mult.max(0.0)
    }

    pub fn knockback_force(&self, damage: f32, blast: bool, headshot: bool) -> f32 {
        let d = if headshot { damage / self.headshot().max(1.0) } else { damage };
        Self::knockback_from_damage(d, self.knockback_mult, blast)
    }

    /// Multiply a cosmetic bone scale (cards stack).
    pub fn warp(&mut self, bone: &str, by: Vec3) {
        match self.bone_warps.iter_mut().find(|(b, _)| b == bone) {
            Some((_, v)) => *v *= by,
            None => self.bone_warps.push((bone.into(), by)),
        }
    }
}

/// A field's value, for scripts and data cards.
#[derive(Clone, Debug, PartialEq)]
pub enum Value { Num(f32), Bool(bool), Vec(Vec3), Text(String) }

macro_rules! fields {
    ($($f:ident: $k:ident),* $(,)?) => {
        /// Every field scripts can read and write, with its kind.
        pub const FIELDS: &[(&str, &str)] = &[$((stringify!($f), stringify!($k))),*];
        impl Weapon {
            pub fn get(&self, name: &str) -> Option<Value> {
                match name {
                    $(stringify!($f) => Some(fields!(@get self.$f, $k)),)*
                    "special" => Some(Value::Text(self.special.name().into())),
                    _ => None,
                }
            }
            /// Set a field by name; an error says why not.
            pub fn set(&mut self, name: &str, v: Value) -> Result<(), String> {
                match (name, v) {
                    $((stringify!($f), fields!(@pat $k, x)) => { fields!(@set self.$f, $k, x); Ok(()) })*
                    ("special", Value::Text(s)) => { self.special = Special::named(&s).ok_or_else(|| format!("unknown special {s} (specials: {})", SPECIALS.map(|s| s.1).join(", ")))?; Ok(()) }
                    (n, v) => Err(match self.get(n) { Some(cur) => format!("{n} is {}, not {}", kind(&cur), kind(&v)), None => format!("no weapon field {n}") }),
                }
            }
        }
    };
    (@get $e:expr, f32) => { Value::Num($e) };
    (@get $e:expr, i32) => { Value::Num($e as f32) };
    (@get $e:expr, bool) => { Value::Bool($e) };
    (@get $e:expr, vec) => { Value::Vec($e) };
    (@get $e:expr, color) => { Value::Vec(Vec3::from($e)) };
    (@pat f32, $x:ident) => { Value::Num($x) };
    (@pat i32, $x:ident) => { Value::Num($x) };
    (@pat bool, $x:ident) => { Value::Bool($x) };
    (@pat vec, $x:ident) => { Value::Vec($x) };
    (@pat color, $x:ident) => { Value::Vec($x) };
    (@set $e:expr, f32, $x:ident) => { $e = $x };
    (@set $e:expr, i32, $x:ident) => { $e = $x.round() as i32 };
    (@set $e:expr, bool, $x:ident) => { $e = $x };
    (@set $e:expr, vec, $x:ident) => { $e = $x };
    (@set $e:expr, color, $x:ident) => { $e = $x.to_array() };
}

fn kind(v: &Value) -> &'static str { match v { Value::Num(_) => "a number", Value::Bool(_) => "a boolean", Value::Vec(_) => "a vector", Value::Text(_) => "a string" } }

fields! {
    damage_mult: f32, fire_rate_mult: f32, reload_mult: f32, headshot_mult: f32,
    mag_size_bonus: i32, extra_projectiles: i32, ricochet_count: i32, body_ricochet_count: i32, last_shot_damage_mult: f32, silencer_stacks: i32,
    spread: f32, recoil_per_shot: f32, lifesteal: f32, explosive_radius: f32, explosive_damage: f32, move_speed_mult: f32, knockback_mult: f32,
    special_cooldown_mult: f32, melee_damage_mult: f32, melee_scale: f32, homing: f32, damage_over_time: f32, dot_duration: f32,
    slow_on_hit: f32, slow_duration: f32, grow_damage_per_meter: f32, ricochet_damage_mult: f32, reload_on_hit: i32, phoenix_revives: i32,
    world_pierce_count: i32, impact_mines: i32, bullet_color: color, bullet_speed_mult: f32, bullet_scale: f32, bullet_drop: f32,
    head_scale: f32, body_scale: f32, body_scale_axes: vec, max_hp_bonus: i32, extra_jumps: i32,
    special_echo_count: i32, special_reload_amount: i32, special_cooldown_refund_on_hit: f32, teleport_blast_radius: f32,
    dash_iframes: bool, dash_end_melee: bool, dash_spawn_bomb: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_by_name_round_trip() {
        let mut w = Weapon::default();
        w.set("damage_mult", Value::Num(1.5)).unwrap();
        w.set("mag_size_bonus", Value::Num(20.0)).unwrap();
        w.set("special", Value::Text("zoom".into())).unwrap();
        w.set("dash_iframes", Value::Bool(true)).unwrap();
        assert_eq!((w.damage_mult, w.mag_size(), w.special, w.dash_iframes), (1.5, 25, Special::Zoom, true));
        assert!(w.set("damage_mult", Value::Bool(true)).unwrap_err().contains("is a number"));
        assert!(w.set("nope", Value::Num(1.0)).is_err());
        // Godot fire interval: base 0.22 s; the projectile budget caps shotguns.
        let s = Weapon { extra_projectiles: 15, fire_rate_mult: 100.0, ..Default::default() };
        assert_eq!(s.fire_interval(), 16.0 / 240.0f64);
    }
}
