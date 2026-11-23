//! Exercise an actual local seat while the other seat continues playing.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

fn city() -> LuaGame {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load_with_storage(
        &dir,
        &Overrides { players: Some(2), seed: Some(1948), ..Default::default() },
        None,
        warp::storage::Location::Memory,
    ).unwrap();
    game.echo = false;
    game.tick(&[Controls::default(); 2]);
    game
}

fn tick(game: &mut LuaGame, first: Controls, second: Controls) {
    game.tick(&[first, second]);
    assert!(game.errors().is_empty(), "{:?}", game.errors());
}

fn check(game: &mut LuaGame, source: &str) {
    game.luau(source).unwrap_or_else(|error| panic!("{error}\n{source}"));
}

#[test]
fn analog_wheel_reaches_the_whole_catalog_without_turning_or_firing() {
    let mut game = city();
    check(&mut game, "state.beforeWheelView=state.fleet.players[1].view");
    let other = Controls {
        held: Buttons(Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER),
        pressed: Buttons(Buttons::RIGHT_TRIGGER),
        ..Default::default()
    };
    let held = Buttons(Buttons::LEFT_SHOULDER | Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER);
    let names = ["pistol", "launcher", "rifle", "shotgun", "uzi", "sniper"];
    for (i, name) in names.iter().enumerate() {
        let angle = (i as f32) * std::f32::consts::TAU / names.len() as f32;
        tick(&mut game, Controls {
            held, aim: [angle.sin(), angle.cos()], look: [0.09, 0.03],
            pressed: Buttons(Buttons::RIGHT_TRIGGER), ..Default::default()
        }, if i == 0 { other } else { Controls::default() });
        check(&mut game, &format!(r#"
            local p=state.fleet.players[1]
            assert(p.weaponWheelOpen and p.weaponChoice=="{name}")
            assert(p.view==state.beforeWheelView and not p.aiming and p.walk.speed==3.8)
            assert(state.combat.players[1].shotFlash==0 and not state.fleet.players[2].weaponWheelOpen)
        "#));
        if i == 0 { check(&mut game, "assert(state.combat.players[2].shotFlash>0)"); }
    }
    tick(&mut game, other, Controls::default());
    check(&mut game, r#"
        local p=state.fleet.players[1]
        assert(p.weapon=="sniper" and not p.weaponWheelOpen and not p.weaponChoice)
        assert(state.combat.players[1].shotFlash==0)
    "#);
    tick(&mut game, other, Controls::default());
    check(&mut game, "assert(state.combat.players[1].shotFlash==0)");
    tick(&mut game, Controls::default(), Controls::default());
}

#[test]
fn mouse_accumulates_small_deltas_and_dpad_cycles_in_either_direction() {
    let mut game = city();
    let wheel = Buttons(Buttons::LEFT_SHOULDER);
    let still = Controls::default();
    let mouse = Controls { held: wheel, look: [0.004, 0.0024], ..still };
    for _ in 0..10 { tick(&mut game, still, mouse); }
    check(&mut game, "assert(state.fleet.players[2].weaponChoice=='launcher')");
    let step = |b| Controls { held: wheel, pressed: Buttons(b), ..still };
    tick(&mut game, still, step(Buttons::RIGHT));
    check(&mut game, "assert(state.fleet.players[2].weaponChoice=='rifle')");
    tick(&mut game, still, step(Buttons::UP));
    check(&mut game, "assert(state.fleet.players[2].weaponChoice=='launcher')");
    tick(&mut game, still, still);
    check(&mut game, "assert(state.fleet.players[2].weapon=='launcher' and not state.fleet.players[1].weaponWheelOpen)");
    tick(&mut game, still, Controls { held: Buttons(Buttons::LEFT_SHOULDER | Buttons::RIGHT_SHOULDER), ..still });
    check(&mut game, "assert(state.fleet.players[2].weaponChoice=='launcher' and state.fleet.players[2].walk.speed==6.5)");
    tick(&mut game, still, still);
    tick(&mut game, Controls { pressed: Buttons(Buttons::WEST), ..still }, still);
    tick(&mut game, Controls { held: wheel, ..still }, still);
    check(&mut game, "assert(state.storyBoard.owner==1 and not state.fleet.players[1].weaponWheelOpen)");
    check(&mut game, r#"
        local car=state.fleet.vehicles[1]
        world.place(state.fleet.players[2].body,car.pos+vector.create(2,.85,0))
    "#);
    tick(&mut game, still, still);
    tick(&mut game, still, Controls { pressed: Buttons(Buttons::NORTH), ..still });
    tick(&mut game, still, Controls { held: Buttons(Buttons::LEFT_SHOULDER | Buttons::RIGHT_TRIGGER), ..still });
    check(&mut game, r#"
        local seat=state.fleet.players[2]
        assert(seat.mode=="drive" and not seat.weaponWheelOpen)
        assert(state.fleet.vehicles[seat.vehicle].throttle==1)
    "#);
}
