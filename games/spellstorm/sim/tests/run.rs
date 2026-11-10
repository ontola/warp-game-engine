//! The whole game with bots at the controllers (Godot's `--sim` report):
//! every mechanic comes up within a few minutes, nothing panics, and the
//! same seed plays out the same run bit for bit.

use kern::{Digest, Sim};
use spellstorm::{Seat, Settings, World};

fn bots(n: usize, seed: u64) -> World {
    let seats: Vec<Seat> = (0..n).map(Seat::bot).collect();
    World::new(&seats, seed, Settings::default())
}

fn digest(w: &World) -> String {
    let mut d = Digest::default();
    w.digest(&mut d);
    d.hex16()
}

#[test]
fn bots_play_a_level() {
    let mut w = bots(3, 7);
    let mut far = 0.0f32;
    for _ in 0..60 * 150 {
        w.step();
        far = far.max(w.lead_progress());
    }
    let s = |k: &str| w.stat(k);
    eprintln!(
        "{:?} furthest {far:.0} m, now {:?} in biome {}",
        w.stats, w.run.state, w.run.biome_index
    );
    for k in [
        "shots",
        "casts",
        "kills",
        "souls",
        "bullets",
        "swings",
        "rolls",
        "props_broken",
        "globes",
        "deflects",
    ] {
        assert!(s(k) > 0, "no {k}: {:?}", w.stats);
    }
    // Past both shrines: the wards opened once everyone took a tome.
    assert!(far > w.level.clearing_at[1], "the bots got only {far} m in");
}

#[test]
fn same_seed_same_run() {
    let mut a = bots(2, 42);
    let mut b = bots(2, 42);
    for i in 0..60 * 40 {
        a.step();
        b.step();
        if i % 60 == 0 {
            assert_eq!(digest(&a), digest(&b), "diverged at tick {i}");
        }
    }
    assert_eq!(digest(&a), digest(&b));
    let mut c = bots(2, 43);
    for _ in 0..60 * 40 {
        c.step();
    }
    assert_ne!(digest(&a), digest(&c), "another seed played the same");
}

#[test]
fn every_biome_survives_bots() {
    for b in 0..spellstorm::biome::BIOMES.len() {
        let mut w = bots(2, 3 + b as u64);
        w.run.biome_index = b;
        w.enter_level();
        for _ in 0..60 * 30 {
            w.step();
        }
        assert!(w.stat("shots") > 0, "biome {b}: {:?}", w.stats);
    }
}

#[test]
fn shrines_rise_ahead_and_before_their_ward() {
    // Wherever the wizards stand when a shrine rises, its tomes are not
    // behind the furthest one (Joep: they ran back for them), and never
    // past a shut ward (they could not be reached).
    for seed in [7, 11, 23] {
        let mut w = bots(3, seed);
        let mut seen = 0;
        for _ in 0..60 * 150 {
            w.step();
            if w.run.altars.len() > seen {
                let p = |v: glam::Vec3| w.level.progress_of(glam::Vec2::new(v.x, v.z));
                let front = w.wizards.iter().filter(|&&id| w.alive(id)).map(|&id| p(w.pos(id))).fold(f32::MIN, f32::max);
                for a in &w.run.altars[seen..] {
                    let at = p(a.pos);
                    // Up against a shut wall is as far ahead as anything can be.
                    let at_wall = w.run.wards.iter().any(|wd| wd.k >= 0 && !wd.opening && (wd.past(glam::Vec2::new(a.pos.x, a.pos.z)).0 + 2.0).abs() < 0.5);
                    assert!(at_wall || at + 2.5 >= front, "seed {seed}: shrine {} altar at {at} m, behind the front at {front} m (clearings {:?})", seen / 4, w.level.clearing_at);
                    for wd in w.run.wards.iter().filter(|wd| wd.k == (seen / 4) as i32 && !wd.opening) {
                        let (d, within) = wd.past(glam::Vec2::new(a.pos.x, a.pos.z));
                        assert!(!within || d < -1.0 || d > 12.0, "seed {seed}: an altar at {:?}, {d} m past the ward", a.pos);
                    }
                }
                seen = w.run.altars.len();
            }
        }
        assert!(seen > 0, "seed {seed}: no shrine rose");
    }
}

#[test]
fn nobody_gets_past_a_shut_wall_and_unlocks_wait_for_it() {
    // Joep: monsters must never be behind a wall of mist, wizards wait at it
    // until it opens, and a shrine's new element comes as you walk through.
    for seed in [7, 11, 23] {
        let mut w = bots(3, seed);
        let mut known: Vec<i32> = w.wizards.iter().map(|&id| w.wiz(id).unlocks).collect();
        for _ in 0..60 * 150 {
            w.step();
            for wd in w.run.wards.iter().filter(|wd| !wd.opening) {
                for &id in w.wizards.iter().chain(&w.enemies) {
                    if !w.alive(id) {
                        continue;
                    }
                    let p = w.pos(id);
                    let q = glam::Vec2::new(p.x, p.z);
                    let (d, within) = wd.past(q);
                    let ahead = w.level.progress_of(q) - wd.at;
                    let behind = d > 0.3 && ((within && d < 4.0 && ahead.abs() < 12.0) || (ahead > 0.3 && ahead < 25.0));
                    assert!(!behind, "seed {seed} tick {}: {} {d} m past a shut wall (k {})", w.tick, if w.a(id).wizard { "a wizard" } else { "a monster" }, wd.k);
                }
            }
            for (i, &id) in w.wizards.iter().enumerate() {
                let u = w.wiz(id).unlocks;
                if u > known[i] {
                    known[i] = u;
                    for wd in w.run.wards.iter().filter(|wd| wd.k >= 0 && !wd.opening) {
                        let p = w.pos(id);
                        let q = glam::Vec2::new(p.x, p.z);
                        let (d, within) = wd.past(q);
                        let near = (w.level.progress_of(q) - wd.at).abs() < 8.0;
                        assert!(!(within && near && d > -6.0 && d < 0.0), "seed {seed} tick {}: unlocked to {u} while {d} m before shut wall {}", w.tick, wd.k);
                    }
                }
            }
        }
    }
}

#[test]
fn a_shrine_still_offers_tomes_when_every_tome_is_carried() {
    // Joep, level 4: both wizards carried all 32 tomes, so the shrine raised
    // none, said "take your tome" and its wall never opened. Now it offers
    // repeats, and the bots take them.
    let mut w = bots(2, 7);
    let n = spellstorm::relics::all().len();
    for id in w.wizards.clone() {
        for r in 0..n {
            w.give_relic(id, r);
        }
    }
    let mut opened = false;
    for _ in 0..60 * 150 {
        w.step();
        if w.run.wards.iter().any(|wd| wd.k == 0 && wd.opening) {
            opened = true;
            break;
        }
    }
    assert!(!w.run.altars.is_empty(), "a shrine offers repeats once every tome is carried");
    assert!(opened, "the first wall must open");
}

fn tome(name: &str) -> usize {
    spellstorm::relics::all().iter().position(|t| t.name == name).unwrap()
}

#[test]
fn phoenix_feather_survives_one_blow_a_level() {
    let mut w = World::new(&[Seat::player(0, "Joep")], 7, Settings::default());
    let id = w.wizards[0];
    w.give_relic(id, tome("Phoenix Feather"));
    let big = [(spellstorm::el::El::Fire, 1.0)];
    w.hit(id, &big, 1e6, None, glam::Vec3::X, 1.0);
    assert!(w.a(id).alive && w.a(id).hp == 1.0);
    w.am(id).invuln = 0.0;
    w.hit(id, &big, 1e6, None, glam::Vec3::X, 1.0);
    assert!(!w.a(id).alive);
}

#[test]
fn vampire_fang_heals_the_striker() {
    let mut w = World::new(&[Seat::player(0, "Joep"), Seat::bot(1)], 7, Settings::default());
    let (a, b) = (w.wizards[0], w.wizards[1]);
    w.give_relic(a, tome("Vampire Fang"));
    w.am(a).hp = 50.0;
    w.am(b).team = 9;
    let dealt = w.hit(b, &[(spellstorm::el::El::Fire, 1.0)], 5.0, Some(a), glam::Vec3::X, 1.0);
    assert!(dealt > 0.0 && w.a(a).hp > 50.0, "{dealt} {}", w.a(a).hp);
}

#[test]
fn the_boss_arena_shuts_behind_the_party() {
    // Joep: in the boss's arena the whole of it is yours, and a wall of
    // mist shuts the way back once everyone is in.
    for seed in [7u64, 11] {
        let mut w = bots(2, seed);
        w.step();
        let at = w.level.clearing_at[2] - w.level.clearings[2].z;
        w.skip_to(at - 8.0);
        let mut shut = None;
        for _ in 0..60 * 60 {
            w.step();
            if w.run.biome_index != 0 {
                break;
            }
            if let Some(door) = w.run.wards.iter().find(|wd| wd.back && !wd.opening) {
                shut.get_or_insert(w.tick);
                for &id in &w.wizards {
                    let p = w.pos(id);
                    let prog = w.level.progress_of(glam::Vec2::new(p.x, p.z));
                    assert!(prog > door.at - 0.6, "seed {seed} tick {}: a wizard got back out of the arena ({prog} < {})", w.tick, door.at);
                }
            }
        }
        assert!(shut.is_some(), "seed {seed}: the arena never shut behind the party");
    }
}

#[test]
fn the_level_setting_goes_straight_there() {
    // Joep: pick a level in the host's settings and play it now.
    let mut w = bots(2, 7);
    w.step();
    w.set_option("level", "1. The Forsaken Chapel");
    assert_eq!(w.run.biome_index, 0);
    w.set_option("level", "4. The Drowned Caves");
    assert_eq!(w.run.biome_index, 3);
    for _ in 0..120 {
        w.step();
    }
    let tick = w.tick;
    // The same value again (the host resending on a reconnect) keeps the run.
    w.set_option("level", "4. The Drowned Caves");
    assert_eq!(w.run.biome_index, 3);
    assert_eq!(w.tick, tick);
    w.set_option("level", "1");
    assert_eq!(w.run.biome_index, 0);
}

#[test]
fn tomes_rise_on_the_path_clear_of_props() {
    // Joep: a shrine's tomes stood far out in the grass, one inside a
    // building, and the party took them for missing.
    let mut total = 0;
    for b in 0..spellstorm::biome::BIOMES.len() {
        for seed in 1u64..=std::env::var("TOME_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(12) {
            let mut w = bots(2, seed * 10 + b as u64);
            w.step();
            w.start_at(b as i32);
            let mut seen = 0;
            for _ in 0..60 * 100 {
                w.step();
                if w.run.biome_index != b {
                    break;
                }
                total += w.run.altars.len().saturating_sub(seen);
                for a in &w.run.altars[seen.min(w.run.altars.len())..] {
                    let q = glam::Vec2::new(a.pos.x, a.pos.z);
                    let on = w.level.point_at(w.level.progress_of(q));
                    assert!(q.distance(on) < spellstorm::level::PATH_W + 4.0, "biome {b} seed {seed}: tome {} {:.1} m off the path", a.shrine, q.distance(on));
                    assert!(w.level.statics.push_out(q, 0.6).distance(q) < 0.05, "biome {b} seed {seed}: tome {} inside a prop", a.shrine);
                }
                seen = w.run.altars.len();
            }
        }
    }
    assert!(total > 10, "only {total} tomes rose");
}

#[test]
fn a_boss_grows_with_the_tomes_the_party_carries() {
    // Joep: the Drowned King went down too easily with a stacked build.
    let mut w = bots(2, 7);
    w.step();
    let bare = w.boss_toughness();
    for &id in &w.wizards.clone() {
        let mut wz = w.ecs.get::<&mut spellstorm::wizard::Wizard>(id).unwrap();
        wz.relics.extend([0usize; 10]);
    }
    let stacked = w.boss_toughness();
    assert!(bare > 1.0, "a boss is tougher than its level makes it ({bare})");
    assert!(stacked > bare * 2.0, "ten tomes each make a boss much tougher ({bare} -> {stacked})");
}

#[test]
fn r3_shakes_the_staff_empty() {
    // Joep: the right stick's button shakes the staff; what was loaded is
    // gone and no mana comes back.
    let mut s = Settings::default();
    s.unlock_all = true;
    let mut w = World::new(&[Seat::player(0, "Ann")], 3, s);
    for _ in 0..30 {
        w.step();
    }
    let id = w.wizards[0];
    {
        let mut wz = w.wiz_mut(id);
        wz.queue.slots = vec![spellstorm::el::El::Poison, spellstorm::el::El::Poison];
    }
    let mana = w.wiz(id).mana;
    let tap = 1u32 << spellstorm::wizard::Pad::RS;
    w.wiz_mut(id).input.held = tap;
    w.step();
    w.wiz_mut(id).input.held = 0;
    w.step();
    let wz = w.wiz(id);
    assert!(wz.queue.slots.is_empty(), "a tap of R3 shakes the staff empty");
    assert!(wz.mana <= mana + 1.0, "shaking gives no mana back");
    assert!(wz.shake_t > 0.0 && wz.used & spellstorm::wizard::used::SHAKE != 0);
}
