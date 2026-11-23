//! Exercise the shipped city with two controller seats and the real shared fleet.
//! Runs entirely on the CPU. The test moves directly to the distant heliport to
//! isolate boarding and pilot departure from the length of the drive there.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

#[test]
fn both_city_lawns_follow_their_diagonal_walks_and_tree_grates() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(&dir, &Overrides { seed: Some(1948), ..Default::default() }, None).unwrap();
    game.tick(&[Controls::default()]);
    assert!(game.errors().is_empty(), "{:?}", game.errors());

    let core = game.core();
    let field = core.world.scene.meadow.as_ref().expect("city lawn meadow");
    let sample = |x: f32, z: f32| {
        let ix = ((x - field.origin[0]) / field.cell).round() as usize;
        let iz = ((z - field.origin[1]) / field.cell).round() as usize;
        field.samples[iz * field.width as usize + ix]
    };
    for (x,z) in [(-84.,84.), (84.,-84.)] {
        let slab = core.world.entities.values().find(|e| {
            e.look.surface == Some(4) && e.pos.x == x && e.pos.z == z
        }).expect("lawn surface under the blades");
        for (dx,dz) in [(8.,0.), (0.,8.), (-8.,0.), (0.,-8.)] {
            let root = sample(x+dx,z+dz);
            assert_eq!(root[2], 1.);
            assert!((root[0] - slab.pos.y - slab.size.y / 2.).abs() < 0.001);
            assert!(root[1] > 0.9, "expected grass at {}, {}", x+dx,z+dz);
        }
        for dx in [-1.,1.] {
            for dz in [-1.,1.] {
                assert!(sample(x+dx*7.,z+dz*7.)[1] < 0.9, "paved diagonal");
                // The grate sits in the turf, clear of the diagonal path.
                let grate = sample(x+dx*13.5,z+dz*8.5);
                assert!(grate[2] == 0. || grate[1] < 0.6, "tree grate clearance {grate:?}");
                let path_x = x + dx*13.;
                let path_z = z + dz*13.;
                assert!(core.world.entities.values().all(|e| !e.solid || (e.pos.x-path_x).abs() > 1. || (e.pos.z-path_z).abs() > 1.),
                    "tree trunk obstructs the diagonal path");
            }
        }
    }
    for (x,z) in [(0.,0.), (28.,28.), (-84.,64.), (84.,-64.)] {
        let hardscape = sample(x,z);
        assert!(hardscape[2] == 0. || hardscape[1] < 0.9, "grass outside the lawns at {x}, {z}");
    }
}

#[test]
fn city_weather_follows_the_seed_and_survives_the_day_cycle() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let load_city = |seed| {
        let mut game = LuaGame::load(&dir, &Overrides { seed: Some(seed), ..Default::default() }, None).unwrap();
        game.tick(&[Controls::default()]);
        assert!(game.errors().is_empty(), "{:?}", game.errors());
        game
    };
    let mut game = load_city(1948);
    let clouds = game.state_json()["clouds"].clone();
    assert!((0.0..1.0).contains(&clouds["coverage"].as_f64().unwrap()));
    assert!(clouds["thickness"].as_f64().unwrap() > 0.0);
    assert!(clouds["wind"][0].as_f64().unwrap() > 0.0);
    assert_eq!(clouds["aerial"], false);

    for (hour, phase) in [(19, "sunset"), (22, "night"), (6, "sunrise"), (9, "morning"), (12, "noon"), (16, "afternoon")] {
        game.luau(&format!("state.clock.hour = {hour}")).unwrap();
        game.tick(&[Controls::default()]);
        let state = game.state_json();
        assert_eq!(state["sky"], "clock:clear");
        assert_eq!(state["state"]["clock"]["phase"], phase);
        assert!(state["sun_direction"][1].as_f64().unwrap().is_finite());
        assert_eq!(state["clouds"], clouds);
    }
    // In an eight-minute day each 60 Hz step advances the sun by 2π / 28,800.
    // Cross a former preset change and midnight during ordinary simulation ticks.
    for hour in [17.99, 23.99] {
        game.luau(&format!("state.clock.hour = {hour}")).unwrap();
        let mut last: Option<glam::DVec3> = None;
        for _ in 0..24 {
            game.tick(&[Controls::default()]);
            let warp::sky::SkyChoice::Clock { hour, .. } = game.core().sky else { panic!("clock lost"); };
            let dir = render::daylight::sun_at(hour).direction();
            if let Some(previous) = last {
                let distance = dir.distance(previous);
                assert!((0.00019..0.00025).contains(&distance), "solar step {distance} at {hour}");
            }
            last = Some(dir);
        }
    }
    game.restart();
    assert_eq!(game.state_json()["clouds"], clouds);
    assert_ne!(load_city(1949).state_json()["clouds"], clouds);
}

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
    let count: usize = g.luau("return #state.fleet.traffic").unwrap().parse().unwrap();
    assert_eq!(count, 12);
    let parked: usize = g.luau("return #state.fleet.vehicles-#state.fleet.traffic-2").unwrap().parse().unwrap();
    assert!(parked >= 25);
    eprintln!("Fleet starts with {parked} parked cars and {count} autonomous drivers");
    assert_eq!(g.luau("return state.fleet.vehicles[8].kind == 'helicopter' and state.fleet.vehicles[9].kind == 'helicopter'").unwrap(), "true");
    assert_eq!(g.luau("local cars=state.fleet.vehicles; for i,a in cars do for j=i+1,#cars do if vector.magnitude(a.pos-cars[j].pos)<5.9 then return false end end end; return true").unwrap(), "true");
    let mut stops = vec![0; count];
    let mut turns = vec![0; count];
    let mut travel = vec![0.0_f64; count];
    let mut waiting = vec![false; count];
    let mut prev_dir = vec![None; count];
    let mut prev_pos = vec![None; count];
    let mut last = String::new();
    for sample in 0..=240 {
        let rows = g.luau("local t={} for _,c in state.fleet.vehicles do if c.auto then table.insert(t,{c.pos.x,c.pos.z,c.yaw,c.speed}) end end return t").unwrap();
        let data: Vec<[f64;4]> = serde_json::from_str(&rows).unwrap();
        assert_eq!(data.len(), count);
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
    for i in 0..count {
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
            let car = format!("state.fleet.vehicles[state.fleet.traffic[{id}]]");
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
