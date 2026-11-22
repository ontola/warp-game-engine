//! Contacts in the shipped city, using its real driver inputs and Rapier cars.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

fn city() -> LuaGame {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut g = LuaGame::load(&dir, &Overrides { seed: Some(1948), players: Some(2), ..Default::default() }, None).unwrap();
    g.echo = false;
    g.tick(&[Controls::default(); 2]);
    g
}

fn ticks(g: &mut LuaGame, count: usize, a: Controls, b: Controls) {
    for _ in 0..count { g.tick(&[a, b]); }
    assert!(g.errors().is_empty(), "{:?}", g.errors());
}

fn yes(g: &mut LuaGame, code: &str) -> bool { g.luau(code).unwrap() == "true" }

fn resident(g: &mut LuaGame) {
    g.luau("local p=state.population; for id,actor in p.pedestrians.active do local c=p.residents[id]; if not c.building and not c.impact then state.target=id; c.path=nil; c.action='idle'; break end end; assert(state.target)").unwrap();
}

#[test]
fn the_driver_can_knock_down_a_resident_and_a_different_seat_who_stands_and_walks_again() {
    let mut g = city();
    let idle = Controls::default();
    let drive = Controls { mv: [0., 1.], ..idle };
    let board = Controls { pressed: Buttons(Buttons::NORTH), ..idle };
    resident(&mut g);
    ticks(&mut g, 1, board, idle);
    for _ in 0..260 {
        ticks(&mut g, 1, drive, idle);
        if yes(&mut g, "return state.fleet.vehicles[1].speed > 7") { break; }
    }
    assert!(yes(&mut g, "return state.fleet.vehicles[1].owner==1 and state.fleet.vehicles[1].speed>7"));
    // Move a low-detail street resident into the path. No regular crowd
    // refresh is due before contact, so the collision must fit their rig.
    g.luau("local car=state.fleet.vehicles[1]; local d=vector.normalize(car.pos-car.prevPos); local pop=state.population; local p=pop.residents[state.target]; p.pos=car.pos+d*5; p.nextPos=p.pos; local active=pop.pedestrians.active; if active[state.target] then humans.remove(active[state.target]); active[state.target]=nil end; p.active=true; pop.selectionIn=1; pop.streetRevision+=1; humans.distant('resident',pop.street,active,.4); world.place(state.fleet.players[2].body,car.pos+d*11+vector.create(0,.85,0))").unwrap();
    let mut resident_fell = false;
    let mut player_fell = false;
    for _ in 0..160 {
        ticks(&mut g, 1, drive, idle);
        resident_fell |= yes(&mut g, "local c=state.population.residents[state.target]; return c.impact~=nil and c.impact.stage=='down' and c.impact.vehicle==1");
        player_fell |= yes(&mut g, "local p=state.fleet.players[2]; return p.impact~=nil and p.impact.stage=='down' and p.impact.vehicle==1");
        if resident_fell && player_fell { break; }
    }
    assert!(resident_fell && player_fell, "both the resident and second player should collide with the first player's car");
    let mut rose = false;
    let brake = Controls { mv: [0., -1.], ..idle };
    let mut stopped = false;
    for i in 0..1200 {
        stopped |= yes(&mut g, "return math.abs(state.fleet.vehicles[1].speed)<.4");
        ticks(&mut g, 1, if stopped { idle } else { brake }, if i < 20 { board } else { idle });
        rose |= yes(&mut g, "local p=state.fleet.players[2]; return p.impact~=nil and p.impact.stage=='rise'");
        if rose && yes(&mut g, "return state.fleet.players[2].impact==nil and state.population.residents[state.target].impact==nil") { break; }
    }
    assert!(rose);
    let final_state = g.luau("local p=state.fleet.players[2]; local c=state.population.residents[state.target]; return {mode=p.mode,y=p.pos.y,player=p.impact,resident=c.impact,feet=c.pos}").unwrap();
    assert!(yes(&mut g, "local p=state.fleet.players[2]; state.stood=p.pos; return not p.impact and p.mode=='walk' and p.pos.y>-.4 and p.pos.y<1 and state.population.residents[state.target].impact==nil"), "{final_state}");
    ticks(&mut g, 45, idle, drive);
    assert!(yes(&mut g, "return vector.magnitude(state.fleet.players[2].pos-state.stood)>1"));
    assert!(yes(&mut g, "return state.fleet.vehicles[1].owner==1"));
}

#[test]
fn parked_and_creeping_cars_separate_people_and_a_moving_traffic_car_can_hit_them() {
    let mut g = city();
    let idle = Controls::default();
    resident(&mut g);
    g.luau("local car=state.fleet.vehicles[1]; state.population.residents[state.target].pos=car.pos+vector.create(.95,0,0)").unwrap();
    ticks(&mut g, 2, idle, idle);
    assert!(yes(&mut g, "local c=state.population.residents[state.target]; return (not c.impact or c.impact.stage~='down') and c.pos.x-state.fleet.vehicles[1].pos.x>1.28"));
    let board = Controls { pressed: Buttons(Buttons::NORTH), ..idle };
    ticks(&mut g, 1, board, idle);
    let creep = Controls { mv: [0., 0.3], ..idle };
    for _ in 0..100 {
        ticks(&mut g, 1, creep, idle);
        if yes(&mut g, "return state.fleet.vehicles[1].speed>.6") { break; }
    }
    assert!(yes(&mut g, "local car=state.fleet.vehicles[1]; return car.speed>.5 and car.speed<2.8"));
    g.luau("local car=state.fleet.vehicles[1]; local c=state.population.residents[state.target]; c.pos=car.pos+vector.normalize(car.pos-car.prevPos)*2.35").unwrap();
    ticks(&mut g, 1, creep, idle);
    assert!(yes(&mut g, "local c=state.population.residents[state.target]; return not c.impact or c.impact.stage~='down'"));

    // Stage the second pedestrian immediately in front of an actual AI car after it accelerates.
    for _ in 0..300 {
        ticks(&mut g, 1, idle, idle);
        if yes(&mut g, "for i,car in state.fleet.vehicles do if car.auto and car.speed>5 then state.testTraffic=i; return true end end; return false") { break; }
    }
    g.luau("local car=state.fleet.vehicles[state.testTraffic]; world.place(state.fleet.players[2].body,car.pos+vector.normalize(car.pos-car.prevPos)*2.51+vector.create(0,.85,0))").unwrap();
    ticks(&mut g, 4, idle, idle);
    assert!(yes(&mut g, "local p=state.fleet.players[2]; return p.impact~=nil and p.impact.stage=='down' and p.impact.vehicle==state.testTraffic"));
}
