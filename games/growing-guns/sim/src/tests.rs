//! Parity checks against the Godot numbers (see the module docs for where
//! each comes from).
use super::*;
use shape::{Block, Kind};

/// Two humans on an endless flat floor, cage open, 20 m apart.
fn flat_world() -> World {
    let mut w = World::new(1, &[false, false]);
    w.cards = Arc::new(cards::test_book());
    w.modifier = None;
    w.arena.blocks = vec![Block::new(Vec3::new(0.0, -1.5, 0.0), Vec3::new(400.0, 3.0, 400.0), 0.0, Kind::Floor)];
    w.arena.build_chunks();
    w.release_cage();
    w.pickups.clear();
    w.players[0].pos = Vec3::new(0.0, HALF_HEIGHT, 0.0);
    w.players[1].pos = Vec3::new(0.0, HALF_HEIGHT, -20.0);
    w.players[0].yaw = 0.0;
    w.players[1].yaw = 0.0;
    w.step(&[Input::default(); 2]);
    w
}

fn idle(w: &mut World, ticks: usize) { for _ in 0..ticks { w.step(&vec![Input::default(); w.players.len()]); } }

fn fire() -> Input { Input { fire: true, ..Default::default() } }

/// Player 0 aims at player 1's torso, perfectly still, no bloom.
fn aim_at(w: &mut World, y: f32) {
    let eye = w.players[0].eye();
    let to = w.players[1].pos + Vec3::Y * y - eye;
    let d = to.normalize();
    w.players[0].yaw = (-d.x).atan2(-d.z);
    w.players[0].pitch = d.y.asin();
    w.players[0].recoil_pitch = 0.0;
    w.players[0].bloom = 0.0;
    w.players[0].weapon.spread = 0.0;
}

fn kills(w: &World, victim: usize) -> bool { w.events.iter().any(|e| matches!(e, Event::Kill { victim: v, .. } if *v == victim)) }

#[test]
fn walking_reaches_walk_speed_and_stays_on_the_floor() {
    let mut w = flat_world();
    let fwd = Input { mv: [0.0, 1.0], ..Default::default() };
    for _ in 0..60 { w.step(&[fwd, Input::default()]); }
    let p = &w.players[0];
    assert!(p.on_floor);
    assert!((p.vel.length() - WALK_SPEED).abs() < 0.01, "{}", p.vel);
    assert!((p.pos.y - HALF_HEIGHT).abs() < 0.02, "{}", p.pos);
}

#[test]
fn jumps_leave_at_full_speed_and_ledges_leave_two_air_jumps() {
    let mut w = flat_world();
    w.step(&[Input { jump: true, ..Default::default() }, Input::default()]);
    assert_eq!(w.players[0].vel.y, JUMP_VELOCITY - 0.0, "gravity comes before the jump");
    assert_eq!(w.players[0].jumps_left, 1);
    let mut peak: f32 = 0.0;
    for _ in 0..60 { w.step(&[Input::default(); 2]); peak = peak.max(w.players[0].pos.y); }
    assert!((peak - HALF_HEIGHT - 1.35).abs() < 0.1, "{peak}");
    // Acrobat: two extra air jumps.
    w.players[0].weapon.extra_jumps = 2;
    idle(&mut w, 2);
    assert_eq!(w.players[0].jumps_left, 4, "grounded: 2 + extra");
}

#[test]
fn legacy_dash_input_and_modifiers_have_no_effect() {
    let mut ordinary = flat_world();
    let mut legacy = flat_world();
    legacy.players[0].weapon.dash_iframes = true;
    legacy.players[0].weapon.dash_end_melee = true;
    legacy.players[0].weapon.dash_spawn_bomb = true;
    legacy.players[0].dash_charges = 2;
    for _ in 0..180 {
        let input = Input { mv: [0.0, 1.0], ..Default::default() };
        ordinary.step(&[input, Input::default()]);
        legacy.step(&[Input { dash: true, ..input }, Input::default()]);
        assert_eq!(legacy.players[0].pos, ordinary.players[0].pos);
        assert_eq!(legacy.players[0].vel, ordinary.players[0].vel);
        assert_eq!(legacy.players[0].iframes, ordinary.players[0].iframes);
        assert_eq!(legacy.players[0].dash_charges, 0);
        assert_eq!(legacy.players[0].dash_timer, 0.0);
        assert!(!legacy.events.iter().any(|e| matches!(e, Event::Dash { .. })));
        assert_eq!(legacy.projectiles.len(), ordinary.projectiles.len());
    }
}

#[test]
fn wall_jump_pushes_away() {
    let mut w = flat_world();
    w.arena.blocks.push(Block::new(Vec3::new(0.0, 5.0, -3.0), Vec3::new(10.0, 10.0, 1.0), 0.0, Kind::Wall));
    w.arena.build_chunks();
    let fwd = Input { mv: [0.0, 1.0], ..Default::default() };
    for _ in 0..40 { w.step(&[fwd, Input::default()]); }
    w.step(&[Input { jump: true, ..fwd }, Input::default()]);
    for _ in 0..5 { w.step(&[fwd, Input::default()]); }
    assert!(w.players[0].wall_normal.is_some());
    let vz = w.players[0].vel.z;
    w.step(&[Input { jump: true, ..fwd }, Input::default()]);
    assert!(w.players[0].vel.y > 9.0);
    assert!(w.players[0].vel.z > vz, "pushed off the wall");
}

#[test]
fn five_body_shots_kill_and_the_win_opens_offers_after_a_moment() {
    let mut w = flat_world();
    let mut shots = 0;
    for _ in 0..400 {
        aim_at(&mut w, 0.16);
        w.step(&[fire(), Input::default()]);
        shots += w.events.iter().filter(|e| matches!(e, Event::Shot { player: 0, .. })).count();
        if kills(&w, 1) { break; }
    }
    assert_eq!(shots, 5, "22 damage a shot, 100 health");
    assert_eq!(w.players[0].score, 1);
    assert!(w.players[1].offer.is_none(), "offers open 0.35 s after the win");
    idle(&mut w, 21);
    assert!(w.players[1].offer.as_ref().is_some_and(|o| o.len() == 3));
    assert_eq!(w.phase, Phase::Picking);
    assert!(w.players[0].offer.is_none(), "the surviving winner never picks");
}

#[test]
fn headshots_double_and_damage_truncates() {
    let mut w = flat_world();
    aim_at(&mut w, 0.64);
    w.players[0].weapon.damage_mult = 0.9;
    w.players[0].weapon.bullet_drop = 0.0;
    w.players[1].health = 1000;
    for _ in 0..20 { w.step(&[fire(), Input::default()]); if w.players[1].health < 1000 { break; } }
    assert_eq!(1000 - w.players[1].health, 39, "int(19.8 * 2)");
}

#[test]
fn cadence_reload_and_auto_reload_follow_godot_ticks() {
    let mut w = flat_world();
    w.players[1].pos.y = -100.0;
    let mut shot_ticks = Vec::new();
    for t in 0..200 {
        w.players[0].pitch = 0.3;
        w.step(&[fire(), Input::default()]);
        if w.events.iter().any(|e| matches!(e, Event::Shot { player: 0, .. })) { shot_ticks.push(t); }
    }
    let gaps: Vec<i32> = shot_ticks.windows(2).map(|p| p[1] - p[0]).collect();
    // 0.22 s = 14 ticks between shots; after the 5th, a 1.2 s reload (72 ticks).
    assert_eq!(&gaps[..5], &[14, 14, 14, 14, 72], "{gaps:?}");
}

#[test]
fn shotgun_pellets_cost_one_round() {
    let mut w = flat_world();
    w.players[0].weapon.extra_projectiles = 2;
    w.step(&[fire(), Input::default()]);
    assert_eq!(w.events.iter().filter(|e| matches!(e, Event::Shot { player: 0, .. })).count(), 3);
    assert_eq!(w.players[0].ammo, 4);
}

#[test]
fn bullets_fall_and_ricochet_and_drill() {
    // Drop: a level shot lands short of a far wall's centre line.
    let mut w = flat_world();
    w.players[1].pos.y = -100.0;
    w.players[0].weapon.spread = 0.0;
    w.step(&[fire(), Input::default()]);
    let vy0 = w.bullets[0].vel.y;
    idle(&mut w, 10);
    assert!(w.bullets[0].vel.y < vy0 - 4.0, "30 m/s² drop");
    // Ricochet: a shot into the floor bounces.
    let mut w = flat_world();
    w.players[1].pos.y = -100.0;
    w.players[0].weapon.ricochet_count = 2;
    w.players[0].pitch = -0.6;
    w.step(&[fire(), Input::default()]);
    let mut bounces = 0;
    for _ in 0..30 { idle(&mut w, 1); bounces += w.events.iter().filter(|e| matches!(e, Event::Ricochet { .. })).count(); }
    assert_eq!(bounces, 1, "off the floor, then up into the sky");
    assert!(w.bullets.len() == 1 && w.bullets[0].ricochet_left == 1 && w.bullets[0].vel.y > 0.0);
    // Drill: through a wall to a player behind it.
    let mut w = flat_world();
    w.arena.blocks.push(Block::new(Vec3::new(0.0, 3.0, -10.0), Vec3::new(10.0, 6.0, 1.0), 0.0, Kind::Wall));
    w.arena.build_chunks();
    aim_at(&mut w, 0.16);
    w.players[0].weapon.world_pierce_count = 2;
    w.players[1].health = 1000;
    for _ in 0..20 { w.step(&[fire(), Input::default()]); aim_at(&mut w, 0.16); w.players[0].weapon.world_pierce_count = 2; }
    assert!(w.players[1].health < 1000, "drilled through");
}

#[test]
fn poison_replaces_direct_damage_and_ticks_every_16_ticks() {
    let mut w = flat_world();
    aim_at(&mut w, 0.16);
    w.players[0].weapon.damage_over_time = 0.7;
    w.players[0].weapon.dot_duration = 3.5;
    w.step(&[fire(), Input::default()]);
    w.players[0].ammo = 0;
    let mut hits = Vec::new();
    for t in 0..240 {
        w.step(&[Input::default(); 2]);
        for e in &w.events { if let Event::Hit { victim: 1, damage, .. } = e { hits.push((t, *damage)); } }
    }
    assert_eq!(hits.iter().map(|h| h.1).sum::<i32>(), 37, "round(22 * 1.7)");
    assert_eq!(hits[1].0 - hits[0].0, 16);
    assert_eq!(hits[0].1, 3, "ceil(10.57 * 16/60)");
}

#[test]
fn slow_lifesteal_and_scavenger() {
    let mut w = flat_world();
    aim_at(&mut w, 0.16);
    let wp = &mut w.players[0].weapon;
    (wp.slow_on_hit, wp.slow_duration, wp.lifesteal, wp.reload_on_hit) = (0.48, 2.6, 0.4, 2);
    w.players[0].health = 50;
    for _ in 0..20 { w.step(&[fire(), Input::default()]); if w.players[1].health < 100 { break; } }
    assert_eq!(w.players[1].slow_mult, 0.48);
    assert_eq!(w.players[0].health, 50 + 8, "int(22 * 0.4)");
    assert_eq!(w.players[0].ammo, 5, "fired one, scavenged two, capped at the mag");
}

#[test]
fn phoenix_rises_and_drops_back() {
    let mut w = flat_world();
    w.players[1].phoenix_charges = 1;
    w.players[1].health = 10;
    aim_at(&mut w, 0.16);
    for _ in 0..30 { w.step(&[fire(), Input::default()]); if w.players[1].phoenix.is_some() { break; } }
    assert!(w.players[1].phoenix.is_some() && w.players[1].alive);
    assert_eq!(w.players[1].health, 35);
    let y0 = w.players[1].pos.y;
    w.players[0].ammo = 0;
    idle(&mut w, 60);
    assert!(w.players[1].pos.y > y0 + 4.0, "rising");
    idle(&mut w, 70);
    assert!(w.players[1].launching.is_some() || w.players[1].on_floor);
    assert_eq!(w.players[0].score, 0, "no kill");
}

#[test]
fn invulnerability_blocks_damage() {
    let mut w = flat_world();
    w.players[1].iframes = 0.2;
    w.step(&[Input::default(); 2]);
    assert!(w.players[1].iframes > 0.0);
    w.players[1].poison_left = 0.0;
    assert!(!w.apply_damage(1, 50, Some(0), player::Blow::plain(Vec3::ZERO)));
    assert_eq!(w.players[1].health, 100);
}

#[test]
fn grenades_explode_on_contact_and_hurt_late() {
    let mut w = flat_world();
    w.players[1].pos = Vec3::new(0.0, HALF_HEIGHT, -8.0);
    w.players[0].pitch = -0.3;
    w.step(&[Input { special: true, ..Default::default() }, Input::default()]);
    assert_eq!(w.projectiles.len(), 1);
    assert!((w.players[0].special_cd - 3.0).abs() < 1e-9);
    let mut boom = None;
    for t in 0..200 {
        w.step(&[Input::default(); 2]);
        if boom.is_none() && w.events.iter().any(|e| matches!(e, Event::Explosion { .. })) { boom = Some(t); }
        if w.players[1].health < 100 { break; }
    }
    assert!(boom.is_some() && w.projectiles.is_empty(), "went off on the floor");
    assert!(w.players[1].health < 100);
}

#[test]
fn teleport_sword_backstab_and_echo() {
    let mut w = flat_world();
    w.players[0].weapon.special = Special::Teleport;
    w.players[0].weapon.special_echo_count = 1;
    w.players[0].pitch = -0.2;
    w.step(&[Input { special: true, ..Default::default() }, Input::default()]);
    let tps = |w: &World| w.events.iter().filter(|e| matches!(e, Event::Teleport { .. })).count();
    assert_eq!(tps(&w), 1);
    let mut echoed = 0;
    for _ in 0..12 { w.step(&[Input::default(); 2]); echoed += tps(&w); }
    assert_eq!(echoed, 1, "echo teleports again 0.16 s later");
    // Backstab: player 1 faces away (-Z), player 0 behind it swings along -Z.
    let mut w = flat_world();
    w.players[0].weapon.special = Special::Sword;
    w.players[0].pos = Vec3::new(0.0, HALF_HEIGHT, -17.5);
    w.players[0].pitch = 0.0;
    w.step(&[Input { special: true, ..Default::default() }, Input::default()]);
    assert!(kills(&w, 1), "9999 from behind");
}

#[test]
fn air_strike_echo_lands_twice_and_ion_ignores_cover() {
    let mut w = flat_world();
    w.players[0].weapon.special = Special::AirStrike;
    w.players[0].weapon.special_echo_count = 1;
    w.players[0].pitch = -0.4;
    w.players[1].pos.y = -100.0;
    w.step(&[Input { special: true, ..Default::default() }, Input::default()]);
    idle(&mut w, 12);
    assert_eq!(w.strikes.len(), 2, "echoed air strikes are not swallowed");
    // Ion cannon: a wall between, still at least 101 damage.
    let mut w = flat_world();
    w.arena.blocks.push(Block::new(Vec3::new(0.0, 4.0, -5.0), Vec3::new(30.0, 8.0, 1.0), 0.0, Kind::Wall));
    w.arena.build_chunks();
    w.players[1].health = 500;
    w.strikes.push(Strike { kind: StrikeKind::IonCannon, target: Vec3::new(0.0, 0.0, 10.0), owner: Some(0), age: 0.0 });
    let mut ion_events = 0;
    for _ in 0..230 {
        idle(&mut w, 1);
        ion_events += w.events.iter().filter(|e| matches!(e, Event::Explosion { kind: ExplosionKind::Ion, .. })).count();
    }
    assert_eq!(ion_events, 1, "ion presentation must fire once, independently of blast radius");
    assert!(500 - w.players[1].health >= 101);
}

#[test]
fn eliminated_human_gets_stationary_howitzer_after_card_pick() {
    let mut w = World::new_with(3, &[false, true, true, true], |w| w.modifier_chance=0.0);
    w.cards=Arc::new(cards::test_book()); w.release_cage(); w.bots_paused=true;
    w.queue_damage(0,1000,Some(1)); w.step(&[]);
    assert!(w.players[0].ghost && w.players[0].offer.is_some());
    assert!(w.players[0].howitzer.is_none());
    w.step(&[Input {pick:Some(0),..Default::default()}]); w.step(&[]);
    let at=w.players[0].howitzer.expect("audience station after pick");
    assert!(!w.arena.stadium.outside(at));
    assert!(at.x.abs().max(at.z.abs())>w.arena.size*0.5);
    assert!((at.y-HALF_HEIGHT-w.arena.stadium.ground(at)).abs()<0.01);
    for _ in 0..60 {w.step(&[Input {mv:[1.0,1.0],jump:true,dash:true,look:[0.01,0.002],..Default::default()}]);}
    assert_eq!(w.players[0].pos,at); assert_eq!(w.players[0].vel,Vec3::ZERO);
    assert!(!w.players[0].targetable()); assert_eq!(w.players[0].health,0);
    w.step(&[Input {fire:true,..Default::default()}]);
    assert!(w.events.iter().any(|e|matches!(e,Event::Shot {player:0,..})));
    assert_eq!(w.projectiles.len(),1); assert_eq!(w.projectiles[0].kind,ProjectileKind::Howitzer);
    assert!(w.projectiles[0].vel.y>0.0);
    assert!(w.bullets.is_empty() && w.strikes.is_empty());
    w.next_round(); assert!(w.players[0].howitzer.is_none() && w.players[0].alive && !w.players[0].ghost);
}

#[test]
fn howitzer_station_is_seeded_and_client_prediction_cannot_walk_away() {
    let mut locations=Vec::new();
    for seed in 1..5 {
        let mut a=World::new(seed,&[false,true,true]);a.release_cage();a.bots_paused=true;
        a.players[0].ghost=true;a.players[0].alive=false;
        let mut b=a.clone();a.step(&[]);b.step(&[]);
        assert_eq!(a.players[0].howitzer,b.players[0].howitzer);
        let at=a.players[0].pos;locations.push(at);
        a.predict_player(0,Input {mv:[1.0,1.0],jump:true,look:[0.1,0.1],..Default::default()});
        assert_eq!(a.players[0].pos,at);
    }
    assert!(locations.windows(2).any(|p|p[0]!=p[1]));
}

#[test]
fn parked_ghost_bots_do_not_move_or_respawn() {
    let mut w = flat_world();
    let p = &mut w.players[0];
    p.alive = false;
    p.ghost = true;
    p.bot = true;
    p.pos = Vec3::new(0.0, -500.0, 0.0);
    let parked = p.pos;
    idle(&mut w, 120);
    assert_eq!(w.players[0].pos, parked);
}

#[test]
fn old_explosion_events_keep_conventional_effects() {
    let old = r#"{"Explosion":{"pos":[0.0,1.0,2.0],"radius":38.0,"owner":0}}"#;
    let event: Event = serde_json::from_str(old).unwrap();
    assert!(matches!(event, Event::Explosion { kind: ExplosionKind::Conventional, .. }));
}

#[test]
fn lava_burns_in_the_band_and_kills_below() {
    let mut w = flat_world();
    w.arena.blocks.clear();
    w.arena.build_chunks();
    w.players[1].pos = Vec3::new(0.0, -2.0, 0.0);
    w.players[1].phoenix_charges = 1;
    let mut burns = 0;
    // f64 time: 30 x 1/60 is just under 0.5, so the first burn is tick 31.
    for _ in 0..31 {
        w.players[1].pos.y = -2.0;
        w.players[1].vel = Vec3::ZERO;
        idle(&mut w, 1);
        burns += w.events.iter().filter(|e| matches!(e, Event::Hit { victim: 1, damage: 15, .. })).count();
    }
    assert_eq!(burns, 1, "15 per half second");
    w.players[1].pos.y = -3.5;
    idle(&mut w, 1);
    assert!(!w.players[1].alive, "the pool skips the phoenix");
}

#[test]
fn same_tick_trades_go_to_whoever_died_second_and_both_pick() {
    let mut w = flat_world();
    w.players[0].health = 1;
    w.players[1].health = 1;
    w.apply_damage(0, 5, Some(1), player::Blow::plain(Vec3::ZERO));
    w.apply_damage(1, 5, Some(0), player::Blow::plain(Vec3::ZERO));
    assert_eq!(w.players[1].score, 1);
    idle(&mut w, 22);
    assert!(w.players[0].offer.is_some() && w.players[1].offer.is_some());
}

#[test]
fn solo_humans_have_no_pick_timeout_and_bots_pick_after_a_beat() {
    let mut w = flat_world();
    w.players[1].bot = true;
    w.players[0].health = 1;
    w.apply_damage(0, 5, Some(1), player::Blow::plain(Vec3::ZERO));
    idle(&mut w, 60 * 15);
    assert!(w.players[0].offer.is_some(), "one human: waits forever");
    let round = w.round;
    w.step(&[Input { pick: Some(1), ..Default::default() }, Input::default()]);
    assert_eq!(w.round, round + 1, "the next round starts at once");
    assert!(w.players[0].frozen, "in the cage");
    assert_eq!(w.cage_left().map(|t| (t * 60.0).round()), Some(164.0));
}

#[test]
fn ten_wins_end_the_match() {
    let mut w = flat_world();
    w.players[0].score = 9;
    w.players[1].health = 1;
    w.apply_damage(1, 5, Some(0), player::Blow::plain(Vec3::ZERO));
    assert_eq!(w.phase, Phase::MatchOver);
    assert!(w.events.contains(&Event::MatchOver { winner: 0 }));
    w.restart_match();
    assert_eq!((w.players[0].score, w.round, w.phase), (0, 1, Phase::Playing));
}

#[test]
fn explosive_rounds_dig_through_the_floor_into_the_lava() {
    let mut w = World::new(3, &[false, false]);
    w.modifier = None;
    w.release_cage();
    let spot = Vec3::new(5.0, HALF_HEIGHT + 0.05, 5.0);
    w.players[0].pos = spot;
    w.players[1].pos = Vec3::new(4.0, HALF_HEIGHT + 0.05, 9.0);
    let wp = &mut w.players[0].weapon;
    (wp.explosive_radius, wp.explosive_damage, wp.mag_size_bonus) = (9.0, 75.0, 200);
    w.players[0].ammo = 200;
    w.players[0].health = 100_000;
    w.players[1].health = 100_000;
    let before = w.arena.alive_chunks();
    let (mut booms, mut splashed, mut burned) = (0, false, false);
    for _ in 0..1200 {
        w.players[0].pitch = -1.4;
        w.step(&[fire(), Input::default()]);
        booms += w.events.iter().filter(|e| matches!(e, Event::Explosion { .. })).count();
        splashed |= w.events.iter().any(|e| matches!(e, Event::Hit { victim: 1, attacker: 0, .. }));
        if !w.players[0].alive { burned = true; break; }
    }
    assert!(booms > 0);
    assert!(w.arena.alive_chunks() < before, "nothing broke");
    assert!(splashed, "a player 4 m away in the open takes splash");
    assert!(burned, "dug through the floor into the lava: {}", w.players[0].pos);
}

#[test]
fn pickups_fall_and_heal_past_max() {
    let mut w = flat_world();
    w.pickups.push(Pickup { kind: PickupKind::Heart, pos: Vec3::new(0.0, 3.0, 0.0), vy: 0.0, grounded: None });
    idle(&mut w, 60);
    assert!(w.pickups.is_empty());
    assert_eq!(w.players[0].health, 150);
}

/// Hooks see hits before they land and can change them, and run in the
/// tick; card scripts mutate the weapon.
#[test]
fn hooks_change_damage_and_run_in_the_tick() {
    use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
    #[derive(Default)]
    struct H { ticks: AtomicU32, picks: AtomicU32 }
    impl Hooks for H {
        fn hit(&self, w: &mut World, h: &HitInfo) -> i32 { w.players[h.attacker].health += 1; h.damage * 10 }
        fn tick(&self, _: &mut World) { self.ticks.fetch_add(1, Relaxed); }
        fn apply_card(&self, _: &cards::CardDef, w: &mut Weapon) { w.explosive_radius += 3.0; }
        fn pick(&self, _: &mut World, _: usize, _: cards::Card) { self.picks.fetch_add(1, Relaxed); }
    }
    let h = Arc::new(H::default());
    let mut w = flat_world();
    w.hooks = HookSlot(Some(h.clone()));
    aim_at(&mut w, 0.16);
    for _ in 0..30 { w.step(&[fire(), Input::default()]); if kills(&w, 1) { break; } }
    assert!(!w.players[1].alive, "x10 damage kills with the first hit");
    assert_eq!(w.players[0].health, MAX_HEALTH + 1, "the hook ran once, for the one hit");
    assert!(h.ticks.load(Relaxed) > 0);
    w.give(0, cards::Card(0));
    let wp = &w.players[0].weapon;
    assert_eq!((h.picks.load(Relaxed), wp.explosive_radius, wp.damage_mult), (1, 3.0, 1.5));
}

#[test]
fn bots_play_rounds_deterministically() {
    let run = || {
        let mut w = World::new(7, &[true; 4]);
        let mut rounds = 0;
        for _ in 0..60 * 120 {
            w.step(&[]);
            rounds += w.events.iter().filter(|e| matches!(e, Event::RoundOver { .. })).count();
        }
        (w.digest(), rounds, w.players.iter().map(|p| p.score).sum::<u32>())
    };
    let a = run();
    assert_eq!(a, run());
    assert!(a.1 >= 2, "bots finished {} rounds in two minutes", a.1);
}

/// Coop on an endless flat floor, cage open, bots paused (enemies stand
/// still and hold fire). `bots[i]` marks an empty seat.
fn coop_world(bots: &[bool]) -> World {
    let mut w = World::new_with(1, bots, |w| w.coop = true);
    w.cards = Arc::new(cards::test_book());
    w.arena.blocks = vec![Block::new(Vec3::new(0.0, -1.5, 0.0), Vec3::new(400.0, 3.0, 400.0), 0.0, Kind::Floor)];
    w.arena.build_chunks();
    w.release_cage();
    w.pickups.clear();
    w.bots_paused = true;
    for (k, p) in w.players.iter_mut().enumerate().filter(|(_, p)| !p.bot) { p.pos = Vec3::new(3.0 * k as f32, HALF_HEIGHT, 0.0); }
    w
}

fn enemies(w: &World) -> Vec<usize> { (w.waves.seats..w.players.len()).collect() }

#[test]
fn coop_wave_one_has_three_enemies_rising_one_by_one() {
    let mut w = coop_world(&[false, true]);
    assert_eq!((w.waves.wave, w.waves.total, w.waves.seats, w.players.len()), (1, 3, 2, 2));
    assert!(!w.players[1].alive && w.players[1].ghost, "an empty seat is out of play");
    assert_eq!(w.players[0].health, 300, "humans have triple health");
    let mut spawned = Vec::new();
    let mut telegraphed = 0;
    for t in 0..60 * 8 {
        let n = w.players.len();
        let had_incoming = w.waves.incoming.is_some();
        idle(&mut w, 1);
        if w.waves.incoming.is_some() && !had_incoming { telegraphed += 1; }
        if w.players.len() > n {
            spawned.push(t);
            let e = &w.players[n];
            assert!(e.bot && e.alive && e.pos.y < 0.0, "rises from below the floor: {}", e.pos);
            assert_eq!(e.health, 50, "a grunt has half of 100");
            assert_eq!(w.waves.archetype(n), Some(Archetype::Grunt));
        }
    }
    assert_eq!(telegraphed, 3);
    // Pop at 0.65 s, rise 0.5 s later; the next pop 1.35 s after that.
    assert_eq!(spawned.len(), 3, "{spawned:?}");
    assert!((68..=70).contains(&spawned[0]), "{spawned:?}");
    for g in spawned.windows(2) { assert!((109..=113).contains(&(g[1] - g[0])), "{spawned:?}"); }
    for i in enemies(&w) {
        let p = &w.players[i];
        assert!(!w.waves.emerging(i) && (p.pos.y - HALF_HEIGHT).abs() < 0.05, "standing after the rise: {}", p.pos);
        assert!(glam::Vec2::new(p.pos.x, p.pos.z).length() >= 8.0, "away from the human");
    }
}

#[test]
fn coop_enemy_kills_count_and_the_wave_end_opens_offers_for_humans_only() {
    let mut w = coop_world(&[false, true]);
    let mut killed = 0;
    for _ in 0..60 * 10 {
        idle(&mut w, 1);
        for i in enemies(&w) {
            if w.players[i].alive {
                // Rising enemies can be hurt.
                assert!(w.apply_damage(i, 1000, Some(0), player::Blow::plain(w.players[i].pos)));
                killed += 1;
                assert_eq!(w.waves.kills, killed);
            }
        }
        if w.phase == Phase::Picking { break; }
    }
    assert_eq!((killed, w.phase), (3, Phase::Picking));
    assert!(w.players[0].offer.as_ref().is_some_and(|o| o.len() == 3));
    assert!(w.players[1..].iter().all(|p| p.offer.is_none()), "bots and empty seats never pick");
    w.step(&[Input { pick: Some(0), ..Default::default() }, Input::default()]);
    assert_eq!((w.round, w.waves.wave, w.phase), (2, 2, Phase::Playing));
    assert_eq!(w.players.len(), 2, "the last wave's enemies are gone");
    assert_eq!((w.waves.total, w.waves.kills), (4, 0));
    assert!(w.events.contains(&Event::NewArena));
}

#[test]
fn coop_down_and_revive_by_a_teammate() {
    let mut w = coop_world(&[false, false]);
    assert_eq!(w.waves.total, 4, "one more enemy per extra human");
    let corpse = w.players[1].pos;
    w.events.clear();
    assert!(w.apply_damage(1, 1000, None, player::Blow::plain(corpse)));
    let p = &w.players[1];
    assert!(p.downed && p.alive && p.health == 0 && !kills(&w, 1), "down, not dead");
    assert!(!w.apply_damage(1, 10, None, player::Blow::plain(corpse)), "invulnerable while down");
    idle(&mut w, 120);
    assert!((w.players[1].pos.y - corpse.y - 10.0).abs() < 1e-3, "rose 10 m: {}", w.players[1].pos);
    assert!(w.waves.downed(1).is_some_and(|d| d.progress == 0.0), "3 m away does not revive");
    // The teammate stands where the body fell.
    let mut ticks = 0;
    while w.players[1].downed && ticks < 400 {
        w.players[0].pos = corpse + Vec3::new(0.5, 0.0, 0.0);
        idle(&mut w, 1);
        ticks += 1;
    }
    assert_eq!(ticks, 180, "three seconds");
    let p = &w.players[1];
    assert!(p.launching.is_some() && p.pos.y > 50.0, "a sky drop: {}", p.pos);
    assert_eq!(p.health, 300);
    idle(&mut w, 60);
    let p = &w.players[1];
    assert!(p.on_floor && glam::Vec2::new(p.pos.x - corpse.x, p.pos.z - corpse.z).length() < 0.01, "{}", p.pos);
}

#[test]
fn coop_the_last_human_down_ends_the_match() {
    let mut w = coop_world(&[false, false]);
    w.apply_damage(1, 1000, None, player::Blow::plain(Vec3::ZERO));
    assert!(w.players[1].downed);
    w.events.clear();
    w.apply_damage(0, 1000, None, player::Blow::plain(Vec3::ZERO));
    assert!(kills(&w, 0) && !w.players[0].alive, "nobody left to revive: a real death");
    assert_eq!(w.phase, Phase::MatchOver);
    assert!(w.players.iter().all(|p| p.frozen) && w.waves.downed.is_empty());
    let n = w.players.len();
    idle(&mut w, 300);
    assert_eq!(w.players.len(), n, "no more enemies");
    // Alone: the first death ends the run; a rematch starts again at wave 1.
    let mut w = coop_world(&[false, true]);
    w.apply_damage(0, 1000, None, player::Blow::plain(Vec3::ZERO));
    assert_eq!(w.phase, Phase::MatchOver);
    w.restart_match();
    assert_eq!((w.waves.wave, w.phase, w.players.len()), (1, Phase::Playing, 2));
}

#[test]
fn coop_allies_cannot_hurt_each_other() {
    let mut w = coop_world(&[false, false]);
    assert!(!w.apply_damage(1, 50, Some(0), player::Blow::plain(Vec3::ZERO)));
    // Splash on the teammate, from player 0.
    w.queue_explosion(w.players[1].pos, 4.0, 100.0, 0);
    idle(&mut w, 2);
    assert_eq!(w.players[1].health, 300);
    while w.players.len() < 4 { idle(&mut w, 1); }
    let (a, b) = (2, 3);
    assert!(!w.apply_damage(a, 10, Some(b), player::Blow::plain(Vec3::ZERO)), "enemies spare each other");
    assert!(w.apply_damage(a, 10, Some(0), player::Blow::plain(Vec3::ZERO)));
    assert!(w.apply_damage(0, 10, Some(a), player::Blow::plain(Vec3::ZERO)));
}

#[test]
fn coop_archetypes_follow_the_wave() {
    let mut rng = rng::Rng::new(1);
    let picks: Vec<Archetype> = (0..12).map(|k| Archetype::pick_for(8, k, &mut rng)).collect();
    assert_eq!((picks[0], picks[6], picks[5], picks[4]), (Archetype::Demolition, Archetype::Grenadier, Archetype::Sniper, Archetype::FlatFragger));
    let g = Archetype::Grenadier.weapon(5);
    assert_eq!((g.max_hp_bonus, g.body_scale, g.explosive_damage), (60, 1.28, 43.0));
    assert!((g.explosive_radius - 4.0).abs() < 1e-6);
    let mut p = Player::new(true);
    p.weapon = g;
    assert_eq!(p.max_health(), 160);
    assert_eq!(Archetype::Demolition.weapon(10).special, Special::ClusterGrenade);
    assert_eq!((coop::enemy_count(1, 1), coop::enemy_count(5, 3)), (3, 9));
}

#[test]
fn coop_bots_play_waves_deterministically() {
    let run = || {
        let mut w = World::new_with(5, &[false, true], |w| w.coop = true);
        for _ in 0..60 * 30 { w.step(&[Input::default(), Input::default()]); }
        (w.digest(), w.players.len(), w.waves.kills, w.phase)
    };
    let a = run();
    assert_eq!(a, run());
    assert!(a.1 > 2, "enemies came: {a:?}");
}

#[test]
fn coop_waves_go_on_with_bots_fighting() {
    // Real arenas, live bots; the human cannot die and kills each enemy
    // once it stands, then takes the first card.
    let mut w = World::new_with(9, &[false, true], |w| w.coop = true);
    w.cards = Arc::new(cards::test_book());
    let mut seen = Vec::new();
    for _ in 0..60 * 200 {
        w.players[0].health = w.players[0].health.max(1000);
        let pick = w.players[0].offer.as_ref().map(|_| 0u8);
        w.step(&[Input { pick, ..Default::default() }, Input::default()]);
        for i in w.waves.seats..w.players.len() {
            if w.players[i].alive && !w.waves.emerging(i) {
                seen.push((w.waves.wave, w.waves.archetype(i)));
                w.apply_damage(i, 10_000, Some(0), player::Blow::plain(w.players[i].pos));
            }
        }
        if seen.last().is_some_and(|s| s.0 == 8) { break; }
    }
    assert_eq!(w.waves.wave, 8, "{:?}", w.waves);
    assert_eq!(w.players[0].cards.len(), 7, "a card after every wave");
    assert!(seen.contains(&(8, Some(Archetype::Demolition))), "wave 8 opens with a bomber");
    assert!(seen.iter().any(|s| s.1 == Some(Archetype::FlatFragger)));
}

#[test]
fn howitzer_shells_arc_explode_and_respect_reload() {
    let mut w=flat_world();w.bots_paused=true;
    w.players[0].ghost=true;w.players[0].alive=false;
    w.step(&[]);w.players[0].special_cd=0.0;
    let fire=Input {fire:true,..Default::default()};
    w.step(&[fire]);let vy=w.projectiles[0].vel.y;
    w.step(&[fire]);assert_eq!(w.projectiles.len(),1);assert!(w.projectiles[0].vel.y<vy);
    // Place an already airborne shell just above the floor: impact, no bounce.
    w.projectiles[0].pos=Vec3::new(0.0,0.5,0.0);w.projectiles[0].vel=Vec3::NEG_Y*42.0;
    w.step(&[fire]);
    assert!(w.projectiles.is_empty());
    assert!(w.events.iter().any(|e|matches!(e,Event::Explosion {radius,..} if *radius==10.0)));
    assert!(w.players[0].special_cd>4.0);
    assert!(!w.events.iter().any(|e|matches!(e,Event::Shot {..})));
}

/// Island arenas (Godot `_bot_on_lava_map`): bots keep to the islands,
/// jump between them and still fight. Their falls come from Godot's lapses
/// (a 0.6 s window of recklessness in 7% of seconds), not from moving on
/// purpose (bot.rs).
#[test]
fn bots_keep_to_the_islands_and_jump_between_them() {
    let (mut careless, mut careful, mut kills, mut moves) = (0, 0, 0, 0);
    for seed in [1, 2, 3, 4] {
        let mut w = World::new_with(seed, &[true; 4], |w| { w.force_island = true; w.modifier_chance = 0.0; });
        assert_eq!(w.arena.variant, arena::Variant::Island);
        let mut on: Vec<Option<usize>> = vec![None; 4];
        // When each bot last had a lapse or was high above the islands
        // (the cage, a sky drop).
        let mut wild = [0u64; 4];
        for _ in 0..60 * 120 {
            w.step(&[]);
            for (i, p) in w.players.iter().enumerate() {
                if w.brains[i].lapse() || p.pos.y > 12.0 { wild[i] = w.tick; }
            }
            for e in &w.events {
                match *e {
                    Event::Lava { player, fall: true } if !w.lava.active => if w.tick - wild[player] < 90 { careless += 1 } else { careful += 1 },
                    Event::Kill { victim, attacker: Some(a), .. } if a != victim => kills += 1,
                    _ => {}
                }
            }
            // Which island each bot stands on; count changes.
            for (i, p) in w.players.iter().enumerate() {
                if !p.on_floor || !p.alive { continue; }
                let here = w.arena.islands.iter().position(|&(t, r)| (p.pos.x - t.x).abs() <= r && (p.pos.z - t.z).abs() <= r && (p.pos.y - HALF_HEIGHT - t.y).abs() < 0.3);
                if here.is_some() && here != on[i] { if on[i].is_some() { moves += 1; } on[i] = here; }
            }
            if w.events.iter().any(|e| matches!(e, Event::NewArena)) { on = vec![None; 4]; }
        }
    }
    eprintln!("islands: {kills} kills, {moves} island changes, falls: {careful} careful, {careless} in a lapse or a drop");
    // Walking/double-jumping has less reach than the removed dash. Still
    // require combat and repeated safe island crossings over these four seeds.
    assert!(kills >= 10, "the bots fight: {kills} kills");
    assert!(moves >= 30, "the bots move between islands: {moves}");
    assert!(careful <= 8, "careful bots rarely fall: {careful}");
}


#[test]
fn corpse_paths_follow_flight_and_stop_at_gameplay_hits() {
    for hitscan in [false,true] {
        let mut w=flat_world();
        if hitscan { w.players[0].weapon.bullet_speed_mult=20.0; }
        let origin=Vec3::new(5.0,2.0,0.0);
        w.events.clear();
        w.spawn_bullet(0,origin,Vec3::NEG_Y,false);
        if !hitscan {
            assert!(!w.events.iter().any(|e| matches!(e,Event::BulletPath{..})), "projectiles must travel before hitting a corpse");
            for _ in 0..60 { w.step_bullets(); if w.bullets.is_empty() {break;} }
        }
        let paths:Vec<_>=w.events.iter().filter_map(|e| if let Event::BulletPath{from,to,damage}=e {Some((*from,*to,*damage))} else {None}).collect();
        assert!(!paths.is_empty());
        assert!(paths.iter().all(|(a,b,d)| a.is_finite() && b.is_finite() && b.y>=-0.001 && *d>0.0), "a shot must not reach corpses below its floor hit");
        assert!(paths.last().unwrap().1.y.abs()<0.001);
        for pair in paths.windows(2) { assert!(pair[0].1.abs_diff_eq(pair[1].0,1e-5), "no duplicate swept segment"); }
    }
}

#[test]
fn removing_a_card_keeps_other_copies_and_rebuilds_stats() {
    let mut w = flat_world();
    let card = w.cards.find("big_mag").unwrap();
    w.give(0, card);
    let single = w.players[0].weapon.clone();
    w.give(0, card);
    assert!(w.remove_card(0, card));
    assert_eq!(w.players[0].cards, vec![card]);
    assert_eq!(w.players[0].weapon.mag_size_bonus, single.mag_size_bonus);
    assert_eq!(w.players[0].weapon.fire_rate_mult, single.fire_rate_mult);
    assert!(w.players[0].ammo <= w.players[0].mag_size());
    assert!(w.remove_card(0, card));
    assert!(!w.remove_card(0, card));
    assert_eq!(w.players[0].weapon.fire_rate_mult, Weapon::default().fire_rate_mult);
}

#[test]
fn crossing_audience_boundary_kills_humans_and_bots_even_with_iframes() {
    for bot in [false, true] {
        let mut w=World::new(71,&[bot,false,false,false]);w.release_cage();
        w.players[0].pos=Vec3::new(w.arena.stadium.wall_r()+5.0,90.0,0.0);
        w.players[0].iframes=10.0;w.players[0].phoenix_charges=1;
        idle(&mut w,1);
        assert!(!w.players[0].alive);assert!(kills(&w,0));
        assert!(!w.events.iter().any(|e|matches!(e,Event::Lava{player:0,..})));
        if !bot {
            w.players[0].offer=None; idle(&mut w,1);
            assert!(!w.arena.stadium.outside(w.players[0].pos),"ghost returns inside after choosing a card");
        }
    }
}

#[test]
fn audience_boundary_keeps_corner_seats_and_high_jumps_playable() {
    let mut w=flat_world();let r=w.arena.stadium.wall_r();
    for pos in [Vec3::new(r-1.0,120.0,0.0),Vec3::new(r*0.83,120.0,r*0.83),Vec3::new(0.0,200.0,0.0)] {
        w.players[0].pos=pos;w.players[0].vel=Vec3::ZERO;idle(&mut w,1);assert!(w.players[0].alive);
    }
    assert!(w.arena.stadium.outside(Vec3::new(r,r*0.0,r)));
}

#[test]
fn pickups_lost_to_island_lava_release_their_slot() {
    let mut w = World::new(1, &[false, true]);
    w.arena = crate::arena::Arena::generate_with(1, 64.0, 64.0, true);
    w.pickups.clear();
    w.pickups.push(Pickup { kind: PickupKind::Heart, pos: Vec3::new(0., -1., 0.), vy: -10., grounded: None });
    w.step_pickups();
    assert!(w.pickups.is_empty());
}

#[test]
fn siege_bolt_kills_on_contact_without_blast_and_stops_at_stone() {
    for wall in [false,true] {
        let mut w=flat_world();
        w.players[1].health=1200; w.players[1].iframes=0.0;
        if wall {
            w.arena.blocks.push(Block::new(Vec3::new(0.0,2.0,-18.5),Vec3::new(4.0,4.0,0.5),0.0,Kind::Floor));
            w.arena.build_chunks();
        }
        w.events.clear();
        w.spawn_projectile(ProjectileKind::SiegeBolt,Vec3::new(0.0,HALF_HEIGHT,-17.0),Vec3::NEG_Z*90.0,0);
        for _ in 0..5 { w.step_projectiles(); }
        assert_eq!(w.players[1].alive,wall);
        assert!(w.projectiles.is_empty());
        assert!(!w.events.iter().any(|e|matches!(e,Event::Explosion {..})));
    }
}

#[test]
fn audience_weapon_is_random_fixed_for_life_and_has_lower_aim_offset() {
    let mut choices=[false;2];
    for seed in 1..33 {
        let mut w=World::new(seed,&[false,false,false,false]);
        w.release_cage(); w.players[0].ghost=true;w.players[0].alive=false;w.players[0].offer=None;
        w.player_tick(0,Input::default());
        let choice=w.players[0].siege_crossbow;
        choices[usize::from(choice)]=true;
        w.player_tick(0,Input {special:true,..Default::default()});
        assert_eq!(w.players[0].siege_crossbow,choice);
        w.players[0].special_cd=0.0;
        w.player_tick(0,fire());
        assert_eq!(w.projectiles.last().unwrap().kind,if choice {ProjectileKind::SiegeBolt} else {ProjectileKind::Howitzer});
        assert!(w.players[0].howitzer_elevation() < 0.25);
    }
    assert!(choices.into_iter().all(|x|x));
}

#[test]
fn crossbow_zoom_is_held_predicted_and_bolts_are_faster() {
    let mut w=flat_world();
    let p=&mut w.players[0]; p.ghost=true;p.alive=false;p.howitzer=Some(p.pos);p.siege_crossbow=true;p.special_cd=0.0;
    let mut predicted=w.clone();
    let input=Input {special:true,look:[0.1,0.1],..Default::default()};
    w.player_tick(0,input); predicted.predict_player(0,input);
    assert!(w.players[0].zooming && predicted.players[0].zooming);
    assert_eq!(w.players[0].pitch,predicted.players[0].pitch);
    w.player_tick(0,fire());
    assert!(!w.players[0].zooming);
    assert!((w.projectiles.last().unwrap().vel.length()-150.0).abs()<0.01);
    w.players[0].siege_crossbow=false;
    w.player_tick(0,input); assert!(!w.players[0].zooming);
}

#[test]
fn recoil_hops_but_shooting_at_the_floor_cannot_fly() {
    let mut w = flat_world();
    let p = &mut w.players[0];
    (p.weapon.damage_mult, p.weapon.extra_projectiles, p.weapon.fire_rate_mult, p.weapon.mag_size_bonus) = (6.0, 7, 4.0, 200);
    p.ammo = p.weapon.mag_size() as u32;
    p.pitch = -PITCH_LIMIT;
    let mut peak: f32 = 0.0;
    for _ in 0..180 {
        w.players[0].pitch = -PITCH_LIMIT;
        w.players[0].recoil_pitch = 0.0;
        w.step(&[fire(), Input::default()]);
        peak = peak.max(w.players[0].pos.y - HALF_HEIGHT);
    }
    let p = &w.players[0];
    assert!(p.recoil_lift > 0.0 || p.on_floor, "the shots pushed the player");
    // A 4.5 m/s hop climbs 4.5² / (2 · 30) = 0.34 m; a jump reaches 1.35 m.
    assert!(peak > 0.05 && peak < 0.45, "{peak}");
}

#[test]
fn a_headset_players_view_and_gun_arrive_with_their_input() {
    let mut w = flat_world();
    let aim = TrackedAim { offset: Vec3::new(0.2, 0.4, -0.3), direction: Vec3::new(0.0, 0.0, -2.0) };
    let input = Input { head: Some([1.0, 0.25]), tracked: Some(aim), ..Default::default() };
    w.step(&[input, Input::default()]);
    let p = &w.players[0];
    assert_eq!((p.yaw, p.pitch), (1.0, 0.25));
    assert_eq!(p.tracked.unwrap().direction, Vec3::NEG_Z, "normalised, and in the snapshot for the others");
    assert!(w.tracked_ray(0).is_some(), "the shot follows the hand");
    assert_eq!(w.players[1].tracked, None);
    // Nonsense from the network changes nothing.
    w.step(&[Input { head: Some([f32::NAN, 0.0]), tracked: Some(TrackedAim { offset: Vec3::splat(9.0), direction: Vec3::Z }), ..Default::default() }, Input::default()]);
    assert_eq!(w.players[0].yaw, 1.0);
    assert_eq!(w.players[0].tracked, None);
}
