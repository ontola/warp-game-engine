//! The port against the Godot original: the same scripted match, traced
//! every half second, must match the trace Godot printed
//! (gamenight lobbies/game-room/tests/warp_parity.gd) line for line.

use game_room_sim::world::{BTN_A, BTN_B, BTN_RT};
use game_room_sim::{Kind, World};

fn trace() -> Vec<String> {
    let mut w = World::new(7);
    for k in 0..4 {
        w.ensure_player(&format!("p{k}"), k);
    }
    use Kind::*;
    for (kind, x, y) in [
        (Bomb, -2.0, 1.0),
        (Blaster, 0.0, 1.0),
        (Ball, 2.0, 1.0),
        (Saber, -4.0, 1.0),
        (Laser, 4.0, 1.0),
        (Bazooka, 1.0, 5.0),
        (Popcorn, -1.0, 5.0),
        (Minigun, 3.0, 0.5),
    ] {
        w.spawn_item(kind, x, y);
    }
    let mut events = 0;
    let mut out = Vec::new();
    for t in 0..3600i64 {
        for k in 0..4i64 {
            let sx = ((t / 37 + k * 2) % 5 - 2) as f32 * 0.5;
            let sy = if (t / 53 + k) % 7 == 0 { 1.0 } else { 0.0 };
            let mut held = 0;
            if (t + k * 13) % 61 < 9 {
                held |= BTN_A;
            }
            if (t + k * 29) % 97 < 2 {
                held |= BTN_B;
            }
            if (t + k * 17) % 23 < 3 {
                held |= BTN_RT;
            }
            w.set_input(&format!("p{k}"), held, [sx, sy]);
        }
        w.step();
        events += w.take_events().len();
        if t % 30 == 29 {
            let mut line = format!("{t} e{events} i{} s{}", w.items.len(), w.shots.len());
            for k in 0..4 {
                let p = w.player(&format!("p{k}")).unwrap();
                line += &format!(
                    " | {:.5} {:.5} {:.5} {:.5} {} {}",
                    p.x,
                    p.y,
                    p.vx,
                    p.vy,
                    p.item.map_or(-1, |i| i as i64),
                    p.carrying.as_deref().unwrap_or("")
                );
            }
            out.push(line);
        }
    }
    out
}

#[test]
fn matches_godot_line_for_line() {
    let godot: Vec<&str> = include_str!("godot_trace.txt").lines().collect();
    let ours = trace();
    for (n, (a, b)) in godot.iter().zip(&ours).enumerate() {
        assert_eq!(a.trim_end(), b.trim_end(), "first difference at line {n}");
    }
    assert_eq!(godot.len(), ours.len());
}
