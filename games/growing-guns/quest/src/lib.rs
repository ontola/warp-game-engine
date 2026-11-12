//! The actual Growing Guns sim, compiled for the headset. No server or network
//! simulation. The mobile presentation is independent of the desktop renderer.
use glam::Vec3;
use growing_guns::{World, Input, TrackedAim, Event, cards::{Book, CardDef}};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

#[derive(Default, Deserialize)]
#[serde(default)]
struct Controls {
    mv: [f32; 2],
    yaw: f32,
    pitch: f32,
    fire: bool,
    jump: bool,
    reload: bool,
    special: bool,
    pick: Option<u8>,
    room: [f32; 3],
    aim: Option<Aim>,
}
#[derive(Deserialize)]
struct Aim { offset: [f32; 3], direction: [f32; 3] }

/// Deliberately small, data-only card deck for this demo. The desktop's Luau
/// host and all 49 cards are not included in the WebAssembly build.
fn deck() -> Book {
    let card = |id: &str, name: &str, desc: &str, field: &str, value, mul, firepower| {
        let mut c = CardDef::new(id, name);
        c.desc = desc.into();
        c.firepower = firepower;
        if mul { c.mul.push((field.into(), value)); } else { c.add.push((field.into(), value)); }
        c
    };
    Book::new(vec![
        card("dmg_up", "HEAVY HITTER", "+50% damage", "damage_mult", 1.5, true, true),
        card("healthy", "HEALTHY", "+35 maximum health", "max_hp_bonus", 35., false, false),
        card("rapid_fire", "RAPID FIRE", "60% faster fire", "fire_rate_mult", 1.6, true, true),
        card("big_mag", "BIG MAG", "+20 rounds", "mag_size_bonus", 20., false, true),
        card("quick_reload", "QUICK RELOAD", "Reload twice as fast", "reload_mult", 2., true, true),
        card("ricochet", "RICOCHET", "Two extra wall bounces", "ricochet_count", 2., false, true),
        card("extra_barrel", "EXTRA BARREL", "One extra projectile", "extra_projectiles", 1., false, true),
        card("lifesteal", "LIFESTEAL", "Heal for 40% of damage", "lifesteal", 0.4, false, false),
    ])
}

#[wasm_bindgen]
pub struct QuestGame { world: World, revision: u32, room_applied: Vec3 }

#[wasm_bindgen]
impl QuestGame {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: u32) -> Self {
        let world = World::new_with(seed as u64, &[false, true, true, true], |w| {
            w.cards = Arc::new(deck());
            w.modifier_chance = 0.;
            w.rounds_to_win = 3;
            w.rules.body_damage = 0.4;
            // Pickups with unrepresented special visuals are outside the demo.
            w.rules.pickup_rate = 0.;
        });
        Self { world, revision: 1, room_applied: Vec3::ZERO }
    }

    /// Exactly one 60 Hz tick. Browser frame timing belongs to Warp's XR shell.
    pub fn tick(&mut self, controls: &str) -> Result<String, JsError> {
        let c: Controls = serde_json::from_str(controls).map_err(|e| JsError::new(&e.to_string()))?;
        self.advance(c);
        Ok(serde_json::to_string(&self.world.events).unwrap())
    }

    /// Compact presentation snapshot; never serialize the collision grid at
    /// display frequency. JS interpolates the last two simulation snapshots.
    pub fn frame(&self) -> String {
        let w = &self.world;
        json!({
            "tick": w.tick, "round": w.round, "phase": format!("{:?}", w.phase),
            "revision": self.revision, "roomApplied": self.room_applied.to_array(), "cage": w.cage_left(),
            "winner": w.round_winner(), "lava": w.arena.lava_surface(),
            "players": w.players.iter().enumerate().map(|(i, p)| json!({
                "armed": !w.awaiting_rifle(i), "groundGun": w.ground_rifle(i).map(|p| p.to_array()),
                "pos": p.pos.to_array(), "yaw": p.yaw, "pitch": p.pitch,
                "alive": p.targetable(), "ghost": p.ghost,
                "hp": p.health, "maxHp": p.max_health(), "ammo": p.ammo,
                "mag": p.mag_size(), "reload": p.reloading, "score": p.score,
                "cards": p.cards.len(), "shot": p.since_shot,
                "offer": p.offer.as_ref().map(|o| o.iter().filter_map(|c| w.cards.get(*c))
                    .map(|d| json!({"name": d.name, "desc": d.desc})).collect::<Vec<_>>()),
            })).collect::<Vec<_>>(),
            "bullets": w.bullets.iter().map(|b| json!([b.pos.x,b.pos.y,b.pos.z,b.owner,b.vel.x,b.vel.y,b.vel.z])).collect::<Vec<_>>(),
            "grenades": w.projectiles.iter().map(|b| [b.pos.x,b.pos.y,b.pos.z,b.vel.x,b.vel.y,b.vel.z]).collect::<Vec<_>>(),
        }).to_string()
    }

    /// One interleaved record per live collision chunk or stadium piece:
    /// centre.xyz, size.xyz, yaw, material (0 floor, 1 stone, 2 island, 3 stadium).
    pub fn geometry(&self) -> Vec<f32> {
        let mut data = Vec::new();
        for c in self.world.arena.chunks.iter().filter(|c| c.alive) {
            use growing_guns::shape::Kind;
            let material = match c.kind { Kind::Floor => 0., Kind::Island => 2., _ => 1. };
            data.extend_from_slice(&[c.center.x,c.center.y,c.center.z,c.half.x*2.,c.half.y*2.,c.half.z*2.,c.yaw,material]);
        }
        for p in self.world.arena.stadium.pieces() {
            data.extend_from_slice(&[p.center.x,p.center.y,p.center.z,p.size.x,p.size.y,p.size.z,p.yaw,3.]);
        }
        data
    }

    pub fn restart(&mut self) {
        self.room_applied = Vec3::ZERO;
        self.world.restart_match();
        self.revision += 1;
    }
}

impl QuestGame {
    fn advance(&mut self, c: Controls) {
        let w = &mut self.world;
        let p = &mut w.players[0];
        // Room-scale XZ movement uses the same collision capsule as sticks.
        // Tracking-origin changes are rebased by the shell, not teleports.
        let room = Vec3::from_array(c.room) * Vec3::new(1.,0.,1.);
        self.room_applied = Vec3::ZERO;
        if room.is_finite() && room.length() < 0.5 && (p.targetable() || p.ghost) {
            let before = p.pos;
            let mut v = room;
            kern::collide::Mover { radius: growing_guns::RADIUS, half_height: growing_guns::HALF_HEIGHT }
                .slide(&w.arena.grid, &mut p.pos, &mut v, 1., 0.1);
            self.room_applied = p.pos - before;
        }
        let finite = |v: f32| if v.is_finite() { v } else { 0. };
        let input = Input {
            mv: c.mv.map(|v| finite(v).clamp(-1.,1.) * 0.25),
            look: [angle(finite(c.yaw) - p.yaw), (finite(c.pitch) - p.pitch).clamp(-1.4,1.4)],
            fire: c.fire, jump: c.jump, reload: c.reload, special: c.special,
            pick: c.pick, ..Default::default()
        };
        let aim = c.aim.map(|a| TrackedAim { offset: a.offset.into(), direction: a.direction.into() });
        w.step_tracked(&[input], &[aim]);
        if !w.breakage.removed.is_empty() || w.events.iter().any(|e| matches!(e, Event::NewArena)) {
            self.revision += 1;
        }
    }
}
fn angle(v: f32) -> f32 { (v + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_sim_runs_and_mobile_geometry_matches_live_collision_chunks() {
        let mut g = QuestGame::new(3);
        let count = g.world.arena.chunks.iter().filter(|c| c.alive).count() + g.world.arena.stadium.pieces().len();
        assert_eq!(g.geometry().len(), count * 8);
        for _ in 0..600 { g.advance(Controls::default()); }
        assert_eq!(g.world.tick, 600);
        assert!(g.world.players.iter().all(|p| p.pos.is_finite()));
        assert!(g.world.players.iter().skip(1).any(|p| p.since_shot < 9.));
        assert!(serde_json::from_str::<serde_json::Value>(&g.frame()).is_ok());
    }
    #[test]
    fn tracked_shot_uses_hand_not_head_and_desktop_path_resets_it() {
        let mut g = QuestGame::new(3);
        g.world.release_cage();
        g.world.bots_paused = true;
        let p = &mut g.world.players[0];
        p.pos = Vec3::new(0.,60.,0.); p.yaw = 0.; p.pitch = 0.;
        let aim = TrackedAim { offset: Vec3::new(0.4,0.3,0.), direction: Vec3::X };
        g.world.step_tracked(&[Input { fire: true, ..Default::default() }], &[Some(aim)]);
        let (from, dir) = g.world.events.iter().find_map(|e| if let Event::Shot { player: 0, from, dir } = e { Some((*from,*dir)) } else { None }).expect("tracked shot");
        assert!(dir.dot(Vec3::X) > 0.99);
        assert!((from - g.world.players[0].pos - aim.offset).length() < 0.01);
        g.world.players[0].rifle_cd = 0.;
        g.world.step(&[Input { fire: true, ..Default::default() }]);
        let dir = g.world.events.iter().find_map(|e| if let Event::Shot { player: 0, dir, .. } = e { Some(*dir) } else { None }).unwrap();
        assert!(dir.dot(Vec3::NEG_Z) > 0.98);
    }
    #[test]
    fn controller_cannot_fire_from_the_far_side_of_cover() {
        let mut g = QuestGame::new(3);
        g.world.release_cage();
        g.world.bots_paused = true;
        g.world.players[0].pos = Vec3::new(0.,60.,0.);
        g.world.arena.grid = kern::collide::BoxGrid::new(vec![
            kern::collide::YawBox::new(Vec3::new(0.3,60.7,0.),Vec3::new(0.1,2.,2.),0.)
        ], 4.);
        let aim = TrackedAim { offset: Vec3::new(0.6,0.7,0.), direction: Vec3::X };
        g.world.step_tracked(&[Input { fire: true, ..Default::default() }], &[Some(aim)]);
        let from = g.world.events.iter().find_map(|e| if let Event::Shot { player: 0, from, .. } = e { Some(*from) } else { None }).unwrap();
        assert!(from.x < 0.25, "muzzle must remain on the near side of cover: {from:?}");
    }
    #[test]
    fn invalid_controller_pose_does_not_poison_the_simulation() {
        for direction in [Vec3::ZERO, Vec3::splat(f32::NAN), Vec3::splat(f32::MAX)] {
            let mut g = QuestGame::new(3);
            g.world.release_cage();
            g.world.players[0].pos = Vec3::new(0.,60.,0.);
            g.world.step_tracked(&[Input { fire: true, ..Default::default() }],
                &[Some(TrackedAim { offset: Vec3::ZERO, direction })]);
            assert!(g.world.bullets.iter().all(|b| b.pos.is_finite() && b.vel.is_finite()));
        }
    }
    #[test]
    fn no_tracked_input_preserves_desktop_simulation() {
        let mut a = QuestGame::new(12).world;
        a.release_cage();
        let mut b = a.clone();
        for _ in 0..120 {
            a.step(&[Input { fire: true, ..Default::default() }]);
            b.step_tracked(&[Input { fire: true, ..Default::default() }], &[]);
        }
        assert_eq!(kern::digest_of(&a), kern::digest_of(&b));
    }
    #[test]
    fn card_pick_and_rematch_work_without_luau() {
        let mut g = QuestGame::new(3);
        g.world.players[0].offer = Some(vec![growing_guns::cards::Card(0)]);
        g.advance(Controls { pick: Some(0), ..Default::default() });
        assert!(g.world.players[0].weapon.damage_mult >= 1.5);
        assert_eq!(g.world.players[0].cards.len(), 1);
        g.restart();
        assert!(g.world.players[0].cards.is_empty());
        assert_ne!(g.world.phase, growing_guns::Phase::MatchOver);
    }
}
