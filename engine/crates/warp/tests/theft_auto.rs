//! Exercise the shipped city with two controller seats and the real shared fleet.
//! Runs entirely on the CPU. The test moves directly to the distant heliport to
//! isolate boarding and pilot departure from the length of the drive there.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

#[test]
fn shared_car_then_helicopter_with_an_abrupt_pilot_departure() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut g = LuaGame::load(&dir, &Overrides { players: Some(2), seed: Some(1948), ..Default::default() }, None).unwrap();
    g.echo = false;
    let none = Controls::default();
    let board = Controls { pressed: Buttons(Buttons::NORTH), ..none };
    let north = Controls { mv: [0., 1.], ..none };
    let rise = Controls { held: Buttons(Buttons::SOUTH), ..none };
    let descend = Controls { held: Buttons(Buttons::RIGHT_SHOULDER), ..none };
    let run = |g: &mut LuaGame, ticks, a, b| {
        for _ in 0..ticks { g.tick(&[a,b]); }
        assert!(g.errors().is_empty(), "{:?}", g.errors());
    };
    let check = |g: &mut LuaGame, s: &str| { g.luau(&format!("assert({s})")).unwrap_or_else(|e| panic!("{s}: {e}")); };

    // The second pedestrian walks up the curb to the same parked car.
    run(&mut g, 157, none, north);
    check(&mut g,"state.fleet.players[2].nearby == 'car' and state.fleet.players[2].pos.z < 13");
    // Walk into its door. The physical model blocks further progress.
    run(&mut g, 36, none, Controls { mv: [-1.,0.], ..none });
    check(&mut g,"state.fleet.players[2].pos.x > 4.7 and state.fleet.players[2].pos.x < 5.8");
    run(&mut g, 1, board, board);
    check(&mut g,"state.fleet.vehicles[1].owner == 1 and state.fleet.players[2].mode == 'walk'");
    run(&mut g, 120, north, none);
    check(&mut g,"state.fleet.players[1].speed > 3 and state.fleet.vehicles[1].rpm > 1200");
    run(&mut g, 1, board, none);
    check(&mut g,"state.fleet.players[1].mode == 'drive'");
    run(&mut g, 160, Controls { held: Buttons(Buttons::SOUTH), ..none }, none);
    run(&mut g, 1, board, none);
    check(&mut g,"state.fleet.players[1].mode == 'walk' and state.fleet.vehicles[1].owner == nil");

    g.luau("for _,h in state.fleet.vehicles do if h.kind=='helicopter' then state.testHeli=h; break end end; world.place(state.fleet.players[1].body,state.testHeli.pos+vector.create(2.6,.85,0))").unwrap();
    run(&mut g, 1, none, none);
    run(&mut g, 1, board, none);
    run(&mut g, 100, rise, none);
    check(&mut g,"state.fleet.players[1].mode == 'fly' and state.testHeli.pos.y > 7");
    run(&mut g, 1, board, none);
    check(&mut g,"state.testHeli.owner == 1");
    run(&mut g, 240, descend, none);
    run(&mut g, 1, board, none);
    check(&mut g,"state.fleet.players[1].mode == 'walk' and state.testHeli.grounded == true");

    run(&mut g, 1, board, none);
    run(&mut g, 90, rise, none);
    g.luau("player_left(players.get(1))").unwrap();
    check(&mut g,"state.testHeli.owner == nil and state.fleet.players[1] == nil");
    run(&mut g, 230, none, none);
    check(&mut g,"state.testHeli.grounded and state.testHeli.pos.y < 0.1 and state.testHeli.speed < 0.1");
    g.restart();
    run(&mut g, 1, none, none);
    check(&mut g,"state.fleet.vehicles[1].native == 1 and state.fleet.players[1].mode == 'walk'");
}

#[test]
fn traffic_completes_turns_and_waits_for_boarding_at_intersections() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut g = LuaGame::load(&dir, &Overrides { players: Some(3), seed: Some(1948), ..Default::default() }, None).unwrap();
    g.echo = false;
    let mut stops = [0; 3];
    let mut turns = [0; 3];
    let mut travel = [0.0_f64; 3];
    let mut waiting = [false; 3];
    let mut prev_dir = [None; 3];
    let mut prev_pos = [None; 3];
    let mut last = String::new();
    for sample in 0..=240 {
        let rows = g.luau("local t={} for _,c in state.fleet.vehicles do if c.auto then table.insert(t,{c.pos.x,c.pos.z,c.yaw,c.speed}) end end return t").unwrap();
        let data: Vec<[f64;4]> = serde_json::from_str(&rows).unwrap();
        assert_eq!(data.len(), 3);
        for (i, [x,z,yaw,speed]) in data.into_iter().enumerate() {
            if let Some((px,pz)) = prev_pos[i] { travel[i] += f64::hypot(x-px,z-pz); }
            prev_pos[i] = Some((x,z));
            let on_crossing = |p: f64| (p - (p/56.).round()*56.).abs() < 15.;
            if sample > 8 && speed.abs() < 1.0 && on_crossing(x) && on_crossing(z) { waiting[i] = true; }
            if speed > 3. {
                if waiting[i] { stops[i] += 1; waiting[i] = false; }
                let quarter = yaw / std::f64::consts::FRAC_PI_2;
                if (quarter - quarter.round()).abs() < 0.18 {
                    let dir = (quarter.round() as i32).rem_euclid(4);
                    if prev_dir[i].is_some_and(|old| old != dir) { turns[i] += 1; }
                    prev_dir[i] = Some(dir);
                }
            }
        }
        if sample % 20 == 0 { last = rows; }
        if sample != 240 { for _ in 0..15 { g.tick(&[Controls::default();3]); } }
    }
    eprintln!("Traffic over 60 s: metres {travel:.0?}, stops and restarts {stops:?}, turns {turns:?}, final {last}");
    assert!(g.errors().is_empty(), "{:?}",g.errors());
    for i in 0..3 {
        assert!(travel[i] > 145. && stops[i] >= 2 && turns[i] >= 2, "traffic car {i} must progress between intersections");
    }

    // Put a waiting pedestrian at each car's next stop, then press the same
    // controller button a local player uses. Taking one car must leave the
    // other drivers' circuits running.
    let mut taken = [false;3];
    for _ in 0..240 {
        for (slot, owned) in taken.iter_mut().enumerate() {
            if *owned { continue; }
            let id = slot+1;
            let car = format!("state.fleet.vehicles[#state.fleet.vehicles-3+{id}]");
            let stage = format!("local c={car} if math.abs(c.speed)>1 then return false end world.place(state.fleet.players[{id}].body,c.pos+vector.create(2.2*math.cos(c.yaw),.85,-2.2*math.sin(c.yaw))) return true");
            if g.luau(&stage).unwrap() == "true" {
                g.tick(&[Controls::default();3]);
                let mut inputs = [Controls::default();3];
                inputs[slot].pressed = Buttons(Buttons::NORTH);
                g.tick(&inputs);
                assert_eq!(g.luau(&format!("return {car}.owner=={id} and {car}.auto==nil")).unwrap(),"true");
                *owned = true;
            }
        }
        if taken.iter().all(|t| *t) { break; }
        for _ in 0..15 { g.tick(&[Controls::default();3]); }
    }
    assert_eq!(taken,[true;3], "every traffic car must offer a boarding window");
    assert!(g.errors().is_empty(), "{:?}", g.errors());
}
