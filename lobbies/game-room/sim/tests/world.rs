//! Behaviour of the room platformer, one mechanic per test.

use game_room_sim::layout;
use game_room_sim::world::{BTN_A, BTN_B, BTN_DOWN, BTN_RT};
use game_room_sim::{Ev, Kind, World};

fn settle(w: &mut World, ticks: usize) {
    for _ in 0..ticks {
        w.step();
    }
}

fn tap(w: &mut World, id: &str, held: u32, stick: [f32; 2]) {
    w.set_input(id, held, stick);
    w.step();
    w.set_input(id, 0, stick);
}

#[test]
fn players_fall_to_the_floor_and_jump_onto_the_tv_console() {
    let mut w = World::new(7);
    w.ensure_player("a", 4);
    settle(&mut w, 60);
    let p = w.player("a").unwrap();
    assert!(p.grounded && p.y == 0.0, "lands on the floor");
    w.player_mut("a").unwrap().x = -2.0;
    w.set_input("a", BTN_A, [0.0, 0.0]);
    settle(&mut w, 90);
    let p = w.player("a").unwrap();
    assert!(p.grounded && (p.y - layout::TV_CONSOLE.top).abs() < 1e-9, "stands on the console at {}", p.y);
    assert!(w.take_events().iter().any(|e| e.ev == Ev::Jump));
}

#[test]
fn down_and_jump_drops_through_a_platform() {
    let mut w = World::new(7);
    w.ensure_player("a", 4);
    w.player_mut("a").unwrap().x = 0.0;
    w.set_input("a", BTN_A, [0.0, 0.0]);
    settle(&mut w, 90);
    w.set_input("a", 0, [0.0, 0.0]);
    settle(&mut w, 2);
    tap(&mut w, "a", BTN_A | BTN_DOWN, [0.0, 1.0]);
    settle(&mut w, 40);
    let p = w.player("a").unwrap();
    assert!(p.grounded && p.y == 0.0, "back on the floor at {}", p.y);
}

#[test]
fn b_picks_up_and_throws_an_item() {
    let mut w = World::new(7);
    w.ensure_player("a", 4);
    settle(&mut w, 60);
    let x = w.player("a").unwrap().x;
    w.spawn_item(Kind::Blaster, x + 0.3, 0.0);
    settle(&mut w, 5);
    tap(&mut w, "a", BTN_B, [0.0, 0.0]);
    let id = w.player("a").unwrap().item.expect("holding the blaster");
    tap(&mut w, "a", BTN_RT, [0.0, 0.0]);
    assert_eq!(w.shots.len(), 1, "RT fires");
    settle(&mut w, 20);
    tap(&mut w, "a", BTN_B, [0.0, 0.0]);
    assert!(w.player("a").unwrap().item.is_none());
    let item = &w.items[w.item_index(Some(id)).unwrap()];
    assert!(item.holder.is_none() && item.vx.abs() > 5.0, "thrown");
}

#[test]
fn a_held_bomb_explodes_and_sends_everyone_flying() {
    let mut w = World::new(7);
    w.ensure_player("a", 4);
    w.ensure_player("b", 1);
    settle(&mut w, 60);
    let ax = w.player("a").unwrap().x;
    w.player_mut("b").unwrap().x = ax + 1.0;
    w.spawn_item(Kind::Bomb, ax, 0.0);
    settle(&mut w, 5);
    tap(&mut w, "a", BTN_B, [0.0, 0.0]);
    assert!(w.player("a").unwrap().item.is_some(), "the fuse is lit");
    settle(&mut w, 200);
    let events = w.take_events();
    assert!(events.iter().any(|e| e.ev == Ev::Explode));
    assert!(w.items.iter().all(|i| i.kind != Kind::Bomb), "the bomb is gone");
    assert!(w.player("b").unwrap().stun > 0.0 || w.player("b").unwrap().x > ax + 2.0, "b was knocked away");
}

#[test]
fn a_carried_player_thrown_into_the_exit_door_is_ejected() {
    let mut w = World::new(7);
    w.ensure_player("a", 4);
    w.ensure_player("b", 1);
    settle(&mut w, 60);
    w.player_mut("a").unwrap().x = -8.5;
    w.player_mut("b").unwrap().x = -8.0;
    settle(&mut w, 2);
    tap(&mut w, "a", BTN_B, [0.0, 0.0]);
    assert_eq!(w.player("a").unwrap().carrying.as_deref(), Some("b"));
    // Turn to face the door, then throw.
    w.set_input("a", 0, [-1.0, 0.0]);
    settle(&mut w, 3);
    tap(&mut w, "a", BTN_B, [-1.0, 0.0]);
    w.set_input("a", 0, [0.0, 0.0]);
    settle(&mut w, 60);
    assert_eq!(w.ejected, vec!["b".to_string()]);
}

#[test]
fn input_from_a_carried_player_wriggles_free() {
    let mut w = World::new(7);
    w.ensure_player("a", 4);
    w.ensure_player("b", 1);
    settle(&mut w, 60);
    w.player_mut("b").unwrap().x = w.player("a").unwrap().x + 0.5;
    settle(&mut w, 2);
    tap(&mut w, "a", BTN_B, [0.0, 0.0]);
    assert!(w.player("b").unwrap().carried_by.is_some());
    w.set_input("b", BTN_A, [0.0, 0.0]);
    assert!(w.player("b").unwrap().carried_by.is_none() && w.player("a").unwrap().carrying.is_none());
}

#[test]
fn items_drop_from_the_ceiling_over_time() {
    let mut w = World::new(7);
    w.ensure_player("a", 4);
    settle(&mut w, 60 * 30);
    let drops = w.take_events().iter().filter(|e| e.ev == Ev::Drop).count();
    assert_eq!(drops, 3, "at 4, 13 and 22 seconds");
}

#[test]
fn the_same_inputs_give_the_same_digest() {
    let run = || {
        let mut w = World::new(11);
        for k in 0..3 {
            w.ensure_player(&format!("p{k}"), k);
        }
        for t in 0..1200u32 {
            for k in 0..3u32 {
                let held = if (t + k * 7) % 40 < 5 { BTN_A } else { 0 } | if (t + k) % 50 == 0 { BTN_B | BTN_RT } else { 0 };
                w.set_input(&format!("p{k}"), held, [((t / 30 + k) % 3) as f32 - 1.0, 0.0]);
            }
            w.step();
        }
        kern::digest_of(&w)
    };
    assert_eq!(run(), run());
}
