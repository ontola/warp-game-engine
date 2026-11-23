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


#[test]
fn sedan_bumper_takes_the_legs_first_then_the_torso_reaches_the_bonnet() {
    let mut game = city();
    let idle = Controls::default();
    let gas = Controls { mv: [0., 1.], ..idle };
    let board = Controls { pressed: Buttons(Buttons::NORTH), ..idle };
    ticks(&mut game, 1, board, idle);
    for _ in 0..260 {
        ticks(&mut game, 1, gas, idle);
        if yes(&mut game, "return state.fleet.vehicles[1].speed>8") { break; }
    }
    assert!(yes(&mut game, "return state.fleet.vehicles[1].speed>8"));
    game.luau(r#"
        local car=state.fleet.vehicles[1]
        local d=vector.normalize(car.pos-car.prevPos)
        state.testDir=vector.normalize(vector.create(d.x,0,d.z))
        local actor=state.fleet.players[2]
        world.place(actor.body,car.pos+state.testDir*6+vector.create(0,.85,0))
        local total=0; for _ in state.fleet.vehicles do total+=1 end
        for i=total+1,68 do
            state.combat.cast:car("extra"..i,{pos=vector.create(550+i*6,0,500)})
        end
    "#).unwrap();
    let mut fell = false;
    for _ in 0..65 {
        ticks(&mut game, 1, gas, idle);
        if yes(&mut game, "local p=state.fleet.players[2];return p.impact and p.impact.stage=='down'") { fell=true; break; }
    }
    assert!(fell, "a nearby native car must remain a collider when a story adds distant vehicles");
    game.luau(r#"
        local h=state.population.avatars[2]
        state.testFeet=(humans.joint(h,"foot_l")+humans.joint(h,"foot_r"))*.5
        state.testChest=humans.joint(h,"chest")
        state.legLead=0;state.chestOverHood=false;state.hoodFrames=0;state.firstImpactSpeed=state.fleet.vehicles[1].speed
    "#).unwrap();
    for frame in 0..35 {
        ticks(&mut game, 1, gas, idle);
        game.luau(&format!(r#"
            local h=state.population.avatars[2]
            local feet=(humans.joint(h,"foot_l")+humans.joint(h,"foot_r"))*.5
            local chest=humans.joint(h,"chest")
            local car=state.fleet.vehicles[1]
            local forward=vector.create(-math.sin(car.yaw),0,-math.cos(car.yaw))
            local right=vector.create(math.cos(car.yaw),0,-math.sin(car.yaw))
            local localChest=chest-car.pos
            local z=vector.dot(localChest,forward)
            local lead=vector.dot((feet-state.testFeet)-(chest-state.testChest),state.testDir)
            if {frame}<18 then state.legLead=math.max(state.legLead,lead) end
            local hips=humans.joint(h,"hips")-car.pos
            local hipZ=vector.dot(hips,forward)
            -- At this portion of the bonnet the top falls from .96 m to .81 m.
            -- The supporting pelvis capsule is approximately .16 m in radius.
            local top=.96-.15*math.clamp((hipZ-1.47)/.39,0,1)
            local gap=hips.y-top
            if state.legLead>.20 and z>.83 and z<1.85 and hipZ>.95 and hipZ<1.90
                and gap>.02 and gap<.25 and hipZ-z>.25 and math.abs(hips.y-localChest.y)<.22
                and math.abs(vector.dot(localChest,right))<.89 then
                state.hoodFrames+=1
                if not state.chestOverHood then state.chestOverHood={{frame={frame},height=localChest.y,forward=z}} end
            end
        "#)).unwrap();
    }
    let stats = game.luau("return {legLead=state.legLead,overHood=state.chestOverHood,hoodFrames=state.hoodFrames,speed=state.firstImpactSpeed}").unwrap();
    println!("sedan contact {stats}");
    assert!(yes(&mut game, "return state.legLead>.20 and state.hoodFrames>=2"),"{stats}");
}

#[test]
fn traffic_passing_a_flank_does_not_launch_a_pedestrian_and_reverse_contact_is_shared() {
    let mut game = city();
    let idle = Controls::default();
    let gas = Controls { mv: [0., 1.], ..idle };
    let reverse = Controls { mv: [0., -1.], ..idle };
    let board = Controls { pressed: Buttons(Buttons::NORTH), ..idle };
    ticks(&mut game, 1, board, idle);
    for _ in 0..260 {
        ticks(&mut game, 1, gas, idle);
        if yes(&mut game, "return state.fleet.vehicles[1].speed>7") { break; }
    }
    game.luau(r#"
        local c=state.fleet.vehicles[1]
        local side=vector.create(math.cos(c.yaw),0,-math.sin(c.yaw))
        world.place(state.fleet.players[2].body,c.pos+side*1.23+vector.create(0,.85,0))
    "#).unwrap();
    ticks(&mut game, 15, gas, idle);
    assert!(yes(&mut game, "return not state.fleet.players[2].impact"));
    for _ in 0..600 {
        ticks(&mut game, 1, reverse, idle);
        if yes(&mut game, "local c=state.fleet.vehicles[1];return c.gear==-1 and math.abs(c.speed)>4") { break; }
    }
    assert!(yes(&mut game, "local c=state.fleet.vehicles[1];return c.gear==-1 and math.abs(c.speed)>4"));
    game.apply_story(include_str!("../../../../games/warp-theft-auto/story_contract.module.luau")).unwrap();
    ticks(&mut game, 2, reverse, idle);
    game.luau(r#"
        local c=state.fleet.vehicles[1]
        local forward=vector.create(-math.sin(c.yaw),0,-math.cos(c.yaw))
        local side=vector.create(math.cos(c.yaw),0,-math.sin(c.yaw))
        local p=state.storyCast.people.jack
        p.pos=c.pos-forward*4.5-side*.65
        world.place(p.body,p.pos+vector.create(0,.85,0))
        humans.set(p.actor,{pos=p.pos,pose="walk"})
        state.crossingStart,state.crossingSide=p.pos,side
        assert(state.storyCast:walk("jack",{p.pos+side*3.5},1.85))
    "#).unwrap();
    let mut fell=false;
    for _ in 0..50 {
        ticks(&mut game, 1, reverse, idle);
        if yes(&mut game, "local p=state.storyCast.people.jack;return p.wasHit and p.impact and p.impact.stage=='down' and p.impact.vehicle==1") { fell=true; break; }
    }
    assert!(yes(&mut game, "return vector.dot(state.storyCast.people.jack.pos-state.crossingStart,state.crossingSide)>.15"));
    assert!(fell);
}

#[test]
fn a_resident_rolls_onto_the_sedan_without_a_second_launch_when_the_driver_brakes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let idle = Controls::default();
    let board = Controls {
        pressed: Buttons(Buttons::NORTH),
        ..idle
    };
    let gas = Controls {
        mv: [0., 1.],
        ..idle
    };
    let brake = Controls {
        mv: [0., -1.],
        ..idle
    };

    for (target_z, expected_speed) in [(-0.12, 10.39), (-10.8, 15.97)] {
        let mut game = LuaGame::load(
            &dir,
            &Overrides {
                seed: Some(1948),
                players: Some(1),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        game.echo = false;
        game.luau(&format!(
            r#"
            local p=state.population.street[1]
            p.pos,p.yaw,p.action=vector.create(3.85,0,{target_z}),math.pi/2,"idle"
            p.path,p.delta,p.previousFeet=nil,nil,nil
            p.nextPos,p.nextYaw,p.pose=p.pos,p.yaw,"idle"
            state.population.selectionIn=0
        "#
        ))
        .unwrap();
        for _ in 0..169 {
            game.tick(&[idle]);
        }
        game.tick(&[board]);
        for _ in 0..2 {
            game.tick(&[idle]);
        }
        assert!(yes(&mut game, "return state.fleet.players[1].vehicle==1"));
        for _ in 0..220 {
            game.tick(&[gas]);
            if yes(
                &mut game,
                "local p=state.population.street[1]; return p.impact and p.impact.vehicle==1",
            ) {
                break;
            }
        }
        let speed: f64 = game
            .luau("return state.fleet.vehicles[1].speed")
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            yes(
                &mut game,
                "return state.population.street[1].impact.vehicle==1"
            ) && (speed - expected_speed).abs() < 0.1,
            "contact at {speed} m/s"
        );

        game.luau("state.testRoll={hips=0,head=0,feet=0,bonnet=0}")
            .unwrap();
        // Keep measuring through the roof and rear windscreen, where a leg trapped
        // inside the moving shell used to eject the whole rig after mounting the bonnet.
        for age in 1..=118 {
            game.tick(&[if age <= 18 { gas } else { brake }]);
            game.luau(r#"
                local r=state.testRoll
                local a=assert(state.population.pedestrians.active[1])
                local car=state.fleet.vehicles[1]
                local hips=humans.joint(a,"hips")-car.pos
                local head=humans.joint(a,"head")-car.pos
                r.hips=math.max(r.hips,hips.y)
                r.head=math.max(r.head,head.y)
                r.feet=math.max(r.feet,humans.joint(a,"foot_l").y-car.pos.y,humans.joint(a,"foot_r").y-car.pos.y)
                local forward=vector.create(-math.sin(car.yaw),0,-math.cos(car.yaw))
                local z=vector.dot(hips,forward)
                if z>1.0 and z<1.9 and hips.y>.93 and hips.y<1.45
                    and vector.dot(hips-head,forward)>.35 and math.abs(hips.y-head.y)<.25 then
                    r.bonnet+=1
                end
            "#).unwrap();
        }
        let report = game.luau("return state.testRoll").unwrap();
        assert!(game.errors().is_empty(), "{:?}", game.errors());
        // The roof is 1.45 m high. Allow a cartwheel above it, including feet overhead;
        // the original unstable contact instead took hips past 3 m at both speeds.
        assert!(
            yes(
                &mut game,
                "local r=state.testRoll; return r.bonnet>=2 and r.hips<2.45 and r.head<2.7 and r.feet<3.25"
            ),
            "{speed} m/s: {report}"
        );
    }
}
