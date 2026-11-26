//! Exercise the shipped city with two controller seats and the real shared fleet.
//! Runs entirely on the CPU. The test moves directly to the distant heliport to
//! isolate boarding and pilot departure from the length of the drive there.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

#[test]
fn cape_trunks_stream_for_two_seats_and_custom_routes_reach_the_lookout() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(&dir, &Overrides { seed: Some(1948), players: Some(2), ..Default::default() }, None).unwrap();
    let idle = [Controls::default(); 2];
    let settle = |g: &mut LuaGame| {
        for _ in 0..3 { g.tick(&idle); }
        assert!(g.errors().is_empty(), "{:?}", g.errors());
    };
    settle(&mut game);
    {
        let core = game.core();
        let suffix = if warp::phone() { "-low" } else { "" };
        for (tag, name, ext) in [
            ("woodland", "forest-stand", "glb"),
            ("trail-woodland", "trail-copse", "gltf"),
            ("woodland-tree", "street-tree", "glb"),
            ("street-tree", "street-tree", "glb"),
        ] {
            let asset = format!("assets/plants/{name}{suffix}.{ext}");
            let ids = core.world.find(tag);
            assert!(!ids.is_empty(), "the test route needs {tag}");
            for id in ids {
                assert_eq!(core.world.entities[&id].shape.name(), asset);
            }
        }
    }
    game.luau(r#"
        local c=state.city.cape
        assert(c.stands>20 and c.slopeTrees>0 and c.trailStands>=24 and next(c.active)==nil)
        assert(c.trees==c.stands*15+c.slopeTrees+c.trailStands*4 and #world.find("trail-woodland")==c.trailStands)
        assert(#world.find("woodland")==c.stands and #world.find("woodland-tree")==c.slopeTrees)
        local trunk=nil
        for _,cell in c.cells do
            for _,t in cell do
                assert(t[4]>t[5])
                if trunk==nil or t[5]>trunk[5] then trunk=t end
            end
        end
        assert(trunk and trunk[5]>10, "check the elevated woodland collision grid")
        local ground=world.raycast(vector.create(trunk[1],trunk[4]+1,trunk[2]),vector.create(0,-1,0),70,{tag="city"})
        assert(ground and math.abs(ground.point.y-trunk[5]-.04)<.01)
        state.highAbove=function(seat)
            world.place(state.fleet.players[seat].body,vector.create(trunk[1],trunk[4]+40,trunk[2]))
        end
        state.atTree=function(seat)
            world.place(state.fleet.players[seat].body,vector.create(trunk[1]+1.5,trunk[5]+.95,trunk[2]))
        end
        state.away=function(seat)
            world.place(state.fleet.players[seat].body,vector.create(6.1,.85,11))
        end
        state.nearTrees=function()
            local count=0
            for _,ids in c.active do
                for _,id in ids do assert(world.exists(id)); count+=1 end
            end
            return count
        end
        state.atTree(1)
    "#).unwrap();
    settle(&mut game);
    game.luau("state.firstTreeCount=state.nearTrees(); assert(state.firstTreeCount>0 and state.firstTreeCount<state.city.cape.trees); state.atTree(2)").unwrap();
    settle(&mut game);
    game.luau("assert(state.nearTrees()==state.firstTreeCount); state.away(1)").unwrap();
    settle(&mut game);
    game.luau("assert(state.nearTrees()==state.firstTreeCount); state.highAbove(2)").unwrap();
    settle(&mut game);
    game.luau("assert(state.nearTrees()==0); state.away(2)").unwrap();
    settle(&mut game);
    let cape_story = serde_json::json!({
        "id":"cape", "title":"Cape route", "cast":[],
        "steps":[{"op":"checkpoint","pos":[-1093,0,0],"text":"Reach the lookout"}]
    });
    assert_eq!(game.apply_story_data(cape_story.clone()).unwrap(), "Cape route");
    let mut offshore = cape_story;
    offshore["steps"][0]["pos"] = serde_json::json!([-1208,0,190]);
    assert!(game.apply_story_data(offshore.clone()).is_err());
    // Shore clearance is measured perpendicular to this steep bank.
    offshore["steps"][0]["pos"] = serde_json::json!([-1200,0,-48]);
    assert!(game.apply_story_data(offshore.clone()).is_err());
    offshore["steps"][0]["pos"] = serde_json::json!([-1200,0,-46.9]);
    assert!(game.apply_story_data(offshore).is_ok());
}

#[test]
fn both_city_lawns_follow_their_diagonal_walks_and_tree_grates() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(&dir, &Overrides { seed: Some(1948), ..Default::default() }, None).unwrap();
    game.tick(&[Controls::default()]);
    assert!(game.errors().is_empty(), "{:?}", game.errors());

    let core = game.core();
    assert_eq!(core.plants.status().0, 0, "the playtest uses static tree meshes");
    assert_eq!(core.world.find("street-tree").len(), 52);
    assert!(!core.world.find("cape-hedge").is_empty());
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
    check(&mut g,"state.fleet.vehicles[1].owner == 1 and state.fleet.vehicles[1].passenger == 2 and state.fleet.players[2].mode == 'ride'");
    run(&mut g, 24, none, Controls { mv: [1., 1.], look: [0.05, 0.], ..none });
    check(&mut g,"state.fleet.vehicles[1].throttle == 0 and math.abs(state.fleet.players[2].view-state.fleet.players[1].view)>0.7");
    check(&mut g,"vector.magnitude(camera.get(1).pos-camera.get(2).pos)>2");
    run(&mut g, 120, north, none);
    check(&mut g,"state.fleet.players[1].speed > 3 and state.fleet.vehicles[1].rpm > 1200");
    check(&mut g,"state.fleet.players[1].pos == state.fleet.players[2].pos and vector.magnitude(state.fleet.players[1].seatedPos-state.fleet.players[2].seatedPos) > 0.8");
    run(&mut g, 1, board, board);
    check(&mut g,"state.fleet.players[1].mode == 'drive'");
    check(&mut g,"state.fleet.players[2].mode == 'ride'");
    run(&mut g, 160, Controls { held: Buttons(Buttons::SOUTH), ..none }, none);
    run(&mut g, 1, none, board);
    check(&mut g,"state.fleet.players[2].mode == 'walk' and not state.fleet.vehicles[1].passenger and state.fleet.vehicles[1].owner == 1");
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

fn traffic_game() -> LuaGame {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(&dir, &Overrides {
        players: Some(2), seed: Some(1948), ..Default::default()
    }, None).unwrap();
    game.echo = false;
    game.luau(r#"
        local fleet=state.fleet
        state.motorist=fleet.vehicles[fleet.traffic[1]]
        -- Stand behind the moving car, in the roadway. A lateral offset
        -- gives us a clean near miss from the same shooter at the same range.
        state.lineUp=function(car,side,away)
            local v=vector.create
            local heading=v(-math.sin(car.yaw),0,-math.cos(car.yaw))
            local right=v(math.cos(car.yaw),0,-math.sin(car.yaw))
            local seat=fleet.players[2]
            world.place(seat.body,car.pos-heading*13+right*side+v(0,.85,0))
            seat.view=car.yaw+(if away then math.pi else 0)
            seat.pitch=-.035
        end
    "#).unwrap();
    game
}

fn traffic_ticks(game: &mut LuaGame, count: usize) {
    for _ in 0..count { game.tick(&[Controls::default();2]); }
    assert!(game.errors().is_empty(), "{:?}",game.logs());
}

fn shoot_from_second_seat(game: &mut LuaGame) {
    let shot=Controls { pressed:Buttons(Buttons::RIGHT_TRIGGER), ..Controls::default() };
    game.tick(&[Controls::default(),shot]);
    assert!(game.errors().is_empty(), "{:?}",game.logs());
}

#[test]
fn drivers_react_to_a_hit_or_close_pass_and_recover_after_the_last_threat() {
    let mut g=traffic_game();
    // Use the clear dock approach here; the obstruction case exercises collisions.
    g.luau("state.motorist=state.fleet.vehicles[state.fleet.traffic[2]]").unwrap();
    traffic_ticks(&mut g,12);
    g.luau(r#"
        local c=state.motorist
        state.lineUp(c,0,false)
        state.fleet.players[2].pistolCooldown=1
    "#).unwrap();
    shoot_from_second_seat(&mut g);
    g.luau(r#"
        assert(not state.combat.players[2].shotNumber and not state.motorist.panic)
        state.fleet.players[2].pistolCooldown=0
        state.lineUp(state.motorist,0,true)
    "#).unwrap();
    shoot_from_second_seat(&mut g);
    g.luau("assert(state.combat.players[2].shotNumber==1 and not state.motorist.panic)").unwrap();
    traffic_ticks(&mut g,28);

    g.luau(r#"
        local c=state.motorist
        state.lineUp(c,0,false)
        local behind=vector.create(math.sin(c.yaw),0,math.cos(c.yaw))
        state.cover=world.spawn("cube",{tag="city",solid=true,pos=c.pos+behind*8+vector.create(0,1.2,0),size=vector.create(5,3,.3)})
    "#).unwrap();
    shoot_from_second_seat(&mut g);
    g.luau(r#"
        assert(state.combat.players[2].shotNumber==2 and not state.motorist.panic)
        world.remove(state.cover)
    "#).unwrap();
    traffic_ticks(&mut g,28);
    g.luau("state.lineUp(state.motorist,-2.3,false); state.beforeDamage=state.motorist.damage.total").unwrap();
    shoot_from_second_seat(&mut g);
    g.luau(r#"
        local f,c=state.fleet,state.motorist
        assert(state.combat.players[2].shotNumber==3 and c.panic,"a shot beside the car should frighten its driver")
        assert(c.damage.total==state.beforeDamage,"the near miss should leave the car intact")
        for _,other in f.vehicles do
            if not other.auto or vector.magnitude(other.pos-c.pos)>100 then assert(not other.panic) end
        end
    "#).unwrap();
    traffic_ticks(&mut g,28);
    g.luau("state.lineUp(state.motorist,0,false); state.beforeDamage=state.motorist.damage.total; state.calmAt=state.motorist.panic.untilTime").unwrap();
    shoot_from_second_seat(&mut g);
    g.luau("local c=state.motorist; assert(c.damage.total>state.beforeDamage and c.panic.untilTime>state.calmAt,\"a direct hit should renew the alarm\")").unwrap();
    traffic_ticks(&mut g,660);
    g.luau("local c=state.motorist; assert(c.auto and c.driverActor and not c.panic,\"a safe driver should resume the circuit\")").unwrap();
}

#[test]
fn a_shot_at_a_car_taken_by_another_seat_leaves_that_player_in_control() {
    let mut g=traffic_game();
    g.luau(r#"
        local c=state.motorist
        local v=vector.create
        local forward=v(-math.sin(c.yaw),0,-math.cos(c.yaw))
        local right=v(math.cos(c.yaw),0,-math.sin(c.yaw))
        -- Leave the native boxes separate but obstruct all four dismount points.
        for i,offset in {right*2.9,-right*2.9,forward*5.2,-forward*5.2} do
            assert(state.combat.cast:car("block-"..i,{pos=c.pos+offset,yaw=c.yaw}))
        end
        world.place(state.fleet.players[1].body,c.pos+right*1.35+v(0,.85,0))
    "#).unwrap();
    traffic_ticks(&mut g,1);
    g.luau(r#"
        local c=state.motorist
        state.originalDriver=state.fleet.drivers.people[tostring(c.index)]
        local p=state.originalDriver
        assert(p.rideId==c.index and c.storyDriver==p.actor and humans.head(p.actor))
    "#).unwrap();
    let board=Controls { pressed:Buttons(Buttons::NORTH), ..Controls::default() };
    g.tick(&[board,Controls::default()]);
    g.luau(r#"
        local c,p=state.motorist,state.originalDriver
        assert(not c.owner and p.rideId==c.index and c.storyDriver==p.actor,
            "a blocked driver must keep the seat until they can step out safely")
        for i=1,4 do assert(state.combat.cast:remove("block-"..i)) end
        world.place(state.fleet.players[1].body,c.pos+vector.create(2.2,.85,0))
        world.place(state.fleet.players[2].body,c.pos+vector.create(-2.2,.85,0))
    "#).unwrap();
    traffic_ticks(&mut g,1);
    // The second player stays outside the shared car to fire the test shot.
    g.tick(&[board,Controls::default()]);
    g.luau(r#"
        local c,p=state.motorist,state.originalDriver
        assert(c.owner==1 and state.fleet.players[2].mode=="walk")
        assert(not c.auto and not c.storyDriver and not p.rideId)
        assert(not p.dead and humans.head(p.actor))
        state.exitFeet=p.pos
        state.lineUp(c,0,false)
    "#).unwrap();
    shoot_from_second_seat(&mut g);
    g.luau("local c=state.motorist; assert(c.damage.total>0 and c.owner==1 and not c.auto and not c.panic)").unwrap();
    traffic_ticks(&mut g,70);
    g.luau("local p=state.originalDriver; assert(not p.dead and humans.head(p.actor) and vector.magnitude(p.pos-state.exitFeet)>.9)").unwrap();
}

#[test]
fn a_threatened_driver_accelerates_and_takes_less_care_around_a_stopped_car() {
    let drive=|under_fire:bool| {
        let mut g=traffic_game();
        traffic_ticks(&mut g,90);
        g.luau(r#"
            local c=state.motorist
            local forward=vector.create(-math.sin(c.yaw),0,-math.cos(c.yaw))
            state.stopped=state.combat.cast:car("obstruction",{pos=c.pos+forward*18,yaw=c.yaw})
            state.lineUp(c,-2.3,false)
        "#).unwrap();
        if under_fire { shoot_from_second_seat(&mut g); } else { traffic_ticks(&mut g,1); }
        let mut peak=0_f64;
        for _ in 0..75 {
            traffic_ticks(&mut g,3);
            let speed:f64=g.luau("return math.abs(state.motorist.speed)").unwrap().parse().unwrap();
            peak=peak.max(speed);
        }
        let damage:f64=g.luau("return state.stopped.damage.total").unwrap().parse().unwrap();
        (peak,damage)
    };
    let (calm_speed,calm_damage)=drive(false);
    let (fleeing_speed,fleeing_damage)=drive(true);
    eprintln!("Stopped-car scenario: peak speeds {calm_speed:.2}/{fleeing_speed:.2} m/s and damage {calm_damage:.3}/{fleeing_damage:.3} (calm/panic)");
    assert!(fleeing_speed>calm_speed+1.5,"peak speeds {fleeing_speed} fleeing, {calm_speed} calm");
    assert!(fleeing_damage>calm_damage+0.025,"parked car damage {fleeing_damage} fleeing, {calm_damage} calm");
}

#[test]
fn an_airborne_grenade_only_frightens_the_driver_when_it_actually_passes_the_car() {
    let mut g=traffic_game();
    traffic_ticks(&mut g,12);
    g.luau(r#"
        local c=state.motorist
        state.lineUp(c,-2.3,false)
        state.fleet.players[2].weapon="launcher"
        local forward=vector.create(-math.sin(c.yaw),0,-math.cos(c.yaw))
        world.spawn("cube",{tag="city",solid=true,pos=c.pos+forward*12+vector.create(-2.3,1.2,0),size=vector.create(4,3,.3)})
    "#).unwrap();
    shoot_from_second_seat(&mut g);
    g.luau("assert(#state.combat.projectiles==1 and not state.motorist.panic)").unwrap();
    for _ in 0..45 {
        traffic_ticks(&mut g,1);
        if g.luau("return state.motorist.panic~=nil").unwrap()=="true" { break; }
    }
    g.luau(r#"
        local c=state.motorist
        assert(c.panic and not c.wrecked and c.damage.total==0)
        assert(#state.combat.projectiles==1,"a passing grenade should alarm the driver before it explodes")
    "#).unwrap();
}

#[test]
fn shots_through_an_open_sightline_hit_the_driver_while_door_metal_takes_the_hit() {
    let mut g=traffic_game();
    g.luau(r#"
        local c=state.motorist
        world.place(state.fleet.players[2].body,c.pos+vector.create(-5,.85,0))
    "#).unwrap();
    traffic_ticks(&mut g,8);
    g.luau(r#"
        local f,c=state.fleet,state.motorist
        local p=f.drivers.people[tostring(c.index)]
        assert(p and p.rideId==c.index and p.actor==c.storyDriver)
        state.testDriver=p
        state.driverShot=function(height)
            local right=vector.create(math.cos(c.yaw),0,-math.sin(c.yaw))
            local start=p.pos-right*4.8
            local seat=f.players[2]
            world.place(seat.body,start+vector.create(0,.85,0))
            seat.view=c.yaw-math.pi/2
            seat.pitch=math.atan2(height-1.25,4.8)
        end
        state.driverShot(.64)
    "#).unwrap();
    shoot_from_second_seat(&mut g);
    g.luau(r#"
        local c,p=state.motorist,state.testDriver
        assert(c.damage.total>0 and p.rideId==c.index and not p.dead,"the lower door should shield its occupant")
    "#).unwrap();
    traffic_ticks(&mut g,24);
    g.luau("state.driverShot(1.15)").unwrap();
    shoot_from_second_seat(&mut g);
    g.luau(r#"
        local c,p=state.motorist,state.testDriver
        assert(p.dead and not p.rideId and not c.storyDriver and not c.auto,"the side window gives a clear shot at its driver")
        assert(p.corpse==p.actor and state.combat.players[2].defeats==1,"the seated character keeps their ragdoll when hit")
    "#).unwrap();
}

#[test]
fn a_living_driver_gets_out_before_their_burning_car_explodes() {
    let mut g=traffic_game();
    traffic_ticks(&mut g,16);
    g.luau(r#"
        local c=state.motorist
        state.testDriver=state.fleet.drivers.people[tostring(c.index)]
        assert(state.testDriver.rideId==c.index)
        c.damage.total=.81
        state.lineUp(c,0,false)
    "#).unwrap();
    shoot_from_second_seat(&mut g);
    g.luau("assert(state.motorist.burnRemaining>0 and not state.testDriver.dead)").unwrap();
    traffic_ticks(&mut g,155);
    g.luau(r#"
        local c,p=state.motorist,state.testDriver
        assert(c.burnRemaining>0 and not c.wrecked and not c.auto)
        assert(not p.rideId and not p.dead and humans.head(p.actor) and vector.magnitude(p.pos-c.pos)>3)
    "#).unwrap();
}

#[test]
fn threatened_traffic_rolls_through_its_usual_stop() {
    let run=|threatened:bool| {
        let mut game=traffic_game();
        let mut stopped=false;
        for _ in 0..110 {
            traffic_ticks(&mut game,6);
            if game.luau("local c=state.motorist; return c.driver.waiting>0 and math.abs(c.speed)<.7").unwrap()=="true" {
                stopped=true;
                break;
            }
        }
        assert!(stopped,"driver missed the first routine stop");
        game.luau("state.stopAt=state.motorist.pos; state.lineUp(state.motorist,-2.3,false)").unwrap();
        if threatened { shoot_from_second_seat(&mut game); } else { traffic_ticks(&mut game,1); }
        traffic_ticks(&mut game,80);
        game.luau("return vector.magnitude(state.motorist.pos-state.stopAt)").unwrap().parse::<f64>().unwrap()
    };
    let calm=run(false);
    let fleeing=run(true);
    assert!(calm<1. && fleeing>calm+3.,"intersection travel after 1.35s: {calm:.2} m calm, {fleeing:.2} m frightened");
}

#[test]
fn a_driver_pinned_while_slowing_after_an_escape_leaves_the_car() {
    let mut game=traffic_game();
    traffic_ticks(&mut game,22);
    game.luau(r#"
        local f,c=state.fleet,state.motorist
        local heading=vector.create(-math.sin(c.yaw),0,-math.cos(c.yaw))
        local seat=f.players[2]
        world.place(seat.body,c.pos+heading*16+vector.create(0,.85,0))
        seat.view,seat.pitch=math.atan2(heading.x,heading.z),-.048
        state.runner=f.drivers.people[tostring(c.index)]
    "#).unwrap();
    traffic_ticks(&mut game,1);
    shoot_from_second_seat(&mut game);
    game.luau("assert(state.motorist.panic and state.motorist.driver.step==-1)").unwrap();
    let mut crashed_while_slowing=false;
    for _ in 0..135 {
        traffic_ticks(&mut game,7);
        let crashed=game.luau("local c=state.motorist; return not c.panic and c.damage.total>.20").unwrap();
        if crashed=="true" { crashed_while_slowing=true; }
        if game.luau("return state.runner.rideId==nil").unwrap()=="true" { break; }
    }
    assert!(crashed_while_slowing);
    game.luau("assert(not state.runner.dead and not state.runner.rideId and not state.motorist.auto)").unwrap();
}

#[test]
fn cape_gravel_tires_climb_the_same_ridge_as_walking_and_story_markers() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut g = LuaGame::load(&dir, &Overrides { seed: Some(1948), ..Default::default() }, None).unwrap();
    let gate = serde_json::json!({"id":"ridge","title":"Mariner Wood","cast":[],
        "steps":[{"op":"checkpoint","pos":[-591,0,-68.16],"text":"Climb the pass"}]});
    assert_eq!(g.apply_story_data(gate).unwrap(), "Mariner Wood");
    g.luau(r#"
        local start=state.story.target
        assert(start.y>12 and start.y<13)
        local down=vector.create(0,-1,0)
        local floor=world.raycast(start+vector.create(0,3,0),down,5,{tag="city"})
        assert(floor and math.abs(floor.point.y-start.y)<.015)
        state.testRallyCar=vehicles.spawn(start,math.atan2(24.39,.66))
        vehicles.drive(state.testRallyCar,{throttle=1})
    "#).unwrap();
    for _ in 0..240 {
        g.tick(&[Controls::default()]);
        if g.luau("return vehicles.get(state.testRallyCar).pos.x < -603").unwrap()=="true" { break; }
    }
    let sample = g.luau(r#"
        local car=vehicles.get(state.testRallyCar)
        local ground=world.raycast(car.pos+vector.create(0,2,0),vector.create(0,-1,0),4,{tag="city"})
        assert(car.pos.x< -603 and ground and car.pos.y>14)
        assert(car.pos.y-ground.point.y>.05 and car.pos.y-ground.point.y<.75)
        assert(car.contact>.4 and car.loose>.4, "expected gravel under the tyres")
        return {x=car.pos.x,height=ground.point.y,loose=car.loose,contact=car.contact}
    "#).unwrap();
    eprintln!("Mariner Wood climb: {sample}");
    g.luau(r#"
        local p=state.fleet.players[1]
        p.view=math.atan2(24.39,.66)
        world.place(p.body,state.story.target+vector.create(0,.85,0))
    "#).unwrap();
    for _ in 0..100 { g.tick(&[Controls { mv:[0.,1.], ..Default::default() }]); }
    g.luau(r#"
        local p=state.fleet.players[1]
        local floor=world.raycast(p.pos+vector.create(0,2,0),vector.create(0,-1,0),4,{tag="city"})
        assert(p.pos.x< -595 and p.pos.y>13 and floor and math.abs(p.pos.y-floor.point.y)<.06)
    "#).unwrap();
    assert!(g.errors().is_empty(), "{:?}",g.errors());
}


#[test]
fn live_gravel_width_changes_the_surface_under_a_cars_wheels() {
    let dir=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game=LuaGame::load(&dir,&Overrides {seed:Some(1948),..Default::default()},None).unwrap();
    let idle=[Controls::default()];
    game.tick(&idle);
    let spawn=r#"
        local ground=world.raycast(vector.create(-603,35,-64.3),vector.create(0,-1,0),40,{tag="city"})
        assert(ground)
        state.probeCar=vehicles.spawn(ground.point,math.atan2(24.39,.66))
        vehicles.drive(state.probeCar,{throttle=.2})
    "#;
    game.luau(spawn).unwrap();
    for _ in 0..25 { game.tick(&idle); }
    let before=game.luau(r#"
        local p=vehicles.get(state.probeCar)
        assert(p.contact>.4 and p.soft>.6 and p.loose<.2,"should start on grass")
        vehicles.remove(state.probeCar)
        return {soft=p.soft,loose=p.loose,contact=p.contact}
    "#).unwrap();
    game.luau(r#"
        local edit=get_layout()
        edit.rally.width=12
        assert(apply_layout(edit))
        assert(world.get(world.find("west-cape")[1]).shape=="mesh")
    "#).unwrap();
    game.luau(spawn).unwrap();
    for _ in 0..25 { game.tick(&idle); }
    let after=game.luau(r#"
        local p=vehicles.get(state.probeCar)
        assert(p.contact>.4 and p.loose>.6 and p.soft<.2,"should now be on gravel")
        return {soft=p.soft,loose=p.loose,contact=p.contact}
    "#).unwrap();
    eprintln!("Same wheel position before {before} after {after}");
    assert!(game.errors().is_empty(),"{:?}",game.errors());
}

#[test]
fn reversing_sedan_headlight_stays_ahead_of_its_interpolated_bumper() {
    use glam::Vec3;

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(&dir, &Overrides {
        players: Some(2), seed: Some(1948), ..Default::default()
    }, None).unwrap();
    let idle = Controls::default();
    let car: u32 = game.luau(r#"
        local car=state.fleet.vehicles[4]
        world.place(state.fleet.players[1].body,car.pos+vector.create(3.6,.85,9))
        world.place(state.fleet.players[2].body,car.pos+vector.create(1.25,.85,0))
        state.clock.hour=5.75
        return car.body
    "#).unwrap().parse().unwrap();
    game.tick(&[idle, idle]);
    game.tick(&[idle, Controls { pressed: Buttons(Buttons::NORTH), ..idle }]);
    game.luau("assert(state.fleet.players[1].mode=='walk' and state.fleet.vehicles[4].owner==2)").unwrap();

    let mut previous = game.core().world.get(car).unwrap().world_pose();
    let start_forward = previous.0 * Vec3::NEG_Z;
    let local_mount = Vec3::new(0., 0.31, -2.29);
    let local_aim = Vec3::new(0., -0.13, -1.).normalize();
    let mut reversed_fast = false;
    let mut turned = false;
    for tick in 0..161 {
        let steer = if (90..151).contains(&tick) { -0.16 } else { 0.0 };
        game.tick(&[idle, Controls { mv: [steer, -1.0], ..idle }]);
        let core = game.core();
        assert_eq!(core.spots.len(), 1);
        assert_eq!(core.spots[0].parent, Some(car));
        let (rot, pos) = core.world.get(car).unwrap().world_pose();
        reversed_fast |= (pos - previous.1).dot(rot * Vec3::Z) * core.tick_rate > 9.4;
        turned |= (rot * Vec3::NEG_Z).distance(start_forward) > 0.08;
        for alpha in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let drawn_rot = previous.0.slerp(rot, alpha);
            let drawn_pos = previous.1.lerp(pos, alpha);
            let beam = core.render_spots(alpha).next().expect("second seat's headlight");
            assert!(beam.pos.distance(drawn_pos + drawn_rot * local_mount) < 0.001,
                "beam slipped behind the drawn nose at tick {tick}, alpha {alpha}");
            assert!(beam.dir.distance(drawn_rot * local_aim) < 0.001,
                "beam did not follow the tilted body at tick {tick}, alpha {alpha}");
            assert!((0.0..beam.range).contains(&(-beam.pos.y / beam.dir.y)),
                "the light should reach the road");
        }
        previous = (rot, pos);
    }
    assert!(reversed_fast && turned, "exercise reverse at 34 km/h through a turn");
    assert!(game.errors().is_empty(), "{:?}", game.errors());
}

#[test]
fn shipped_trees_use_the_vegetation_cook_and_fit_the_city_triangle_budgets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto/assets/plants");
    for (suffix, tier, stand_cap, trail_cap, street_cap) in [
        ("", "standard", 14_270, 3_048, 7_294),
        ("-low", "phone", 1_810, 468, 256),
    ] {
        for (name, ext, cap) in [
            ("forest-stand", "glb", stand_cap),
            ("trail-copse", "gltf", trail_cap),
            ("street-tree", "glb", street_cap),
        ] {
            let bytes = std::fs::read(root.join(format!("{name}{suffix}.{ext}"))).unwrap();
            let json = if ext == "glb" {
                assert_eq!(&bytes[..4], b"glTF");
                let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
                &bytes[20..20 + len]
            } else {
                bytes.as_slice()
            };
            let doc: serde_json::Value = serde_json::from_slice(json).unwrap();
            let cook = &doc["asset"]["extras"]["warpVegetation"];
            assert_eq!(cook["tier"], tier, "{name}{suffix} needs a {tier} cook");
            assert_eq!(cook["trees"].as_array().unwrap().len(), if name == "street-tree" { 1 } else { 3 });
            for tree in cook["trees"].as_array().unwrap() {
                assert!(matches!(tree["source"].as_str(), Some("plant_growth" | "plant_recipe")));
                assert!(tree["source_leaf_samples"].as_u64().unwrap() > 0);
            }
            let mut triangles = 0;
            for node in doc["nodes"].as_array().unwrap() {
                let Some(mesh) = node["mesh"].as_u64() else { continue };
                for prim in doc["meshes"][mesh as usize]["primitives"].as_array().unwrap() {
                    let index = prim["indices"].as_u64().unwrap() as usize;
                    triangles += doc["accessors"][index]["count"].as_u64().unwrap() / 3;
                }
            }
            assert!(triangles > 0 && triangles <= cap, "{name}{suffix}: {triangles} triangles, budget {cap}");
        }
    }
}
