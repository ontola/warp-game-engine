//! Exercise ground-floor streaming through real controller, collision and seat hooks.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

fn eval(game: &mut LuaGame, code: &str) -> String {
    game.luau(code).unwrap_or_else(|err| panic!("{code}\n{err}"))
}

fn run(game: &mut LuaGame, ticks: usize, inputs: &[Controls]) {
    for _ in 0..ticks {
        game.tick(inputs);
    }
    assert!(game.errors().is_empty(), "{:?}", game.errors());
}

fn start(players: usize, seed: u64) -> LuaGame {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(
        &dir,
        &Overrides { players: Some(players), seed: Some(seed), ..Default::default() },
        None,
    ).unwrap();
    game.echo = false;
    run(&mut game, 12, &vec![Controls::default(); players]);
    eval(&mut game, r#"
        assert(state.interiors.count == 0 and next(state.interiors.active) == nil)
        state.testLots = {}
        for _, b in state.city.buildings do
            if b.kind == "office" and state.testLots.office and not state.testLots.other then
                state.testLots.other = b
            end
            if not state.testLots[b.kind] then state.testLots[b.kind] = b end
            local shell = world.get(b.shell)
            assert(shell.solid and shell.visible)
        end
        state.baseCount = world.count()
    "#);
    game
}

fn stage(game: &mut LuaGame, seat: usize, kind: &str, dx: f32, dz: f32) {
    eval(game, &format!(r#"
        local p, b = state.fleet.players[{seat}], state.testLots.{kind}
        p.view = 0
        p.walk.vel = vector.create(0, 0, 0)
        world.place(p.body, b.door + vector.create({dx}, .85, {dz}))
    "#));
}

// Travel to a signed distance from the door along its centerline. Sample every
// few ticks so the checks tolerate tuning the walker without hiding a blocked door.
fn walk_to(game: &mut LuaGame, input: Controls, target: f32) {
    let entering = input.mv[1] > 0.;
    let mut distance = 0.0;
    for _ in 0..120 {
        run(game, 6, &[input]);
        distance = eval(game, "return state.fleet.players[1].pos.z - state.lot.door.z")
            .parse::<f32>().unwrap();
        if (entering && distance <= target) || (!entering && distance >= target) {
            return;
        }
    }
    panic!("walker stopped {distance:.2} m from the door while moving toward {target:.2} m");
}

// Ignore the enclosing box and capture actual furnishings. Position is relative
// to the building; transient entity IDs are deliberately excluded.
fn furnishings(game: &mut LuaGame, kind: &str) -> String {
    eval(game, &format!(r#"
        local b = state.testLots.{kind}
        local pieces = {{}}
        for _, id in state.interiors.active[b.id] do
            local e = world.get(id)
            local dx, dz = e.pos.x - b.x, e.pos.z - b.z
            if math.abs(dx) < b.w / 2 - .5 and math.abs(dz) < b.d / 2 - .5
                and e.pos.y > .15 and e.pos.y < 3 and e.size.y > .15 then
                table.insert(pieces, {{dx, e.pos.y, dz, e.size, e.rot, e.shape, e.color}})
            end
        end
        assert(#pieces > 2, "expected several pieces of furniture")
        return pieces
    "#))
}

#[test]
fn each_door_can_be_walked_through_and_regenerates_the_same_furnishings() {
    let mut game = start(1, 1948);
    let still = Controls::default();
    let enter = Controls { mv: [0., 1.], ..still };
    let leave = Controls { mv: [0., -1.], ..still };
    let mut warehouse = String::new();

    for kind in ["hotel", "office", "warehouse"] {
        eval(&mut game, &format!("state.lot = state.testLots.{kind}"));
        stage(&mut game, 1, kind, 0., 5.65);
        run(&mut game, 12, &[still]);
        eval(&mut game, "assert(state.interiors.count == 0 and world.count() == state.baseCount)");

        walk_to(&mut game, enter, 3.8);
        eval(&mut game, r#"
            local b, p = state.lot, state.fleet.players[1]
            assert(p.pos.z > b.door.z and p.pos.z < b.door.z + 4.5)
            assert(state.interiors.count == 1 and p.building == nil)
            state.roomIds = state.interiors.active[b.id]
            assert(world.count() == state.baseCount + #state.roomIds)
            local shell = world.get(b.shell)
            assert(not shell.visible and not shell.solid)
            local ray = vector.create(0, 0, -1)
            for _, x in {-3.5, 0, 3.5} do
                local hit = world.raycast(b.door + vector.create(x, 1, 1), ray, 2, {tag="city"})
                assert((hit ~= nil) == (x ~= 0), "wall and open doorway must match the facade")
            end
        "#);
        let layout = furnishings(&mut game, kind);
        if kind == "warehouse" {
            warehouse.clone_from(&layout);
        }
        walk_to(&mut game, enter, -8.2);
        eval(&mut game, r#"
            local b, p = state.lot, state.fleet.players[1]
            assert(p.pos.z < b.door.z - 7 and p.building == b.id)
            assert(state.interiors.active[b.id] == state.roomIds)
        "#);
        walk_to(&mut game, leave, 5.4);
        eval(&mut game, r#"
            local b, p = state.lot, state.fleet.players[1]
            assert(p.pos.z > b.door.z + 4.5 and p.pos.z < b.door.z + 7)
            assert(p.building == nil and state.interiors.active[b.id] == state.roomIds)
        "#);
        walk_to(&mut game, leave, 8.5);
        eval(&mut game, r#"
            local b = state.lot
            assert(state.interiors.count == 0 and not state.interiors.active[b.id])
            assert(world.count() == state.baseCount)
            for _, id in state.roomIds do assert(not world.exists(id)) end
            local shell = world.get(b.shell)
            assert(shell.solid and shell.visible)
        "#);
        walk_to(&mut game, enter, 3.4);
        assert_eq!(furnishings(&mut game, kind), layout, "{kind} changed on re-entry");
        walk_to(&mut game, leave, 8.5);
        eval(&mut game, "assert(state.interiors.count == 0)");
    }

    // Warehouse footprint is constant across seeds; this comparison isolates
    // variation in the cargo arrangement and finish.
    let mut other = start(1, 925);
    stage(&mut other, 1, "warehouse", 0., 3.);
    run(&mut other, 12, &[still]);
    assert_ne!(furnishings(&mut other, "warehouse"), warehouse);
}

#[test]
fn multiplayer_visitors_share_rooms_and_release_them_on_departure() {
    let mut game = start(4, 1948);
    let still = [Controls::default(); 4];
    stage(&mut game, 1, "hotel", -0.5, 3.);
    stage(&mut game, 2, "hotel", 0.5, 3.);
    run(&mut game, 12, &still);
    eval(&mut game, r#"
        state.hotelRoom = state.interiors.active[state.testLots.hotel.id]
        assert(state.interiors.count == 1)
        assert(world.count() == state.baseCount + #state.hotelRoom)
    "#);

    let mut walk = still;
    walk[0].mv = [0., 1.];
    walk[1].mv = [0., 1.];
    run(&mut game, 125, &walk);
    eval(&mut game, r#"
        for i = 1, 2 do assert(state.fleet.players[i].building == state.testLots.hotel.id) end
    "#);
    stage(&mut game, 1, "office", 0., 3.);
    stage(&mut game, 3, "warehouse", 0., 3.);
    stage(&mut game, 4, "other", 0., 3.);
    run(&mut game, 12, &still);
    let peak = eval(&mut game, r#"
        local total = state.baseCount
        assert(state.interiors.count == 4)
        assert(state.interiors.active[state.testLots.hotel.id] == state.hotelRoom)
        for _, b in state.testLots do
            local ids = state.interiors.active[b.id]
            for _, id in ids do assert(world.exists(id)) end
            total += #ids
        end
        assert(world.count() == total and total < 1024)
        return total
    "#);
    assert_eq!(game.state_json()["room_lighting"]["active"], 4);
    eprintln!("Four simultaneous rooms use {peak} entities");
    eval(&mut game, "player_left(players.get(2))");
    run(&mut game, 12, &still);
    assert_eq!(game.state_json()["room_lighting"]["active"], 3);
    eval(&mut game, r#"
        assert(state.interiors.count == 3 and not state.interiors.active[state.testLots.hotel.id])
        for _, id in state.hotelRoom do assert(not world.exists(id)) end
        for _, i in {1, 3, 4} do player_left(players.get(i)) end
    "#);
    run(&mut game, 12, &still);
    eval(&mut game, r#"
        assert(state.interiors.count == 0 and next(state.interiors.active) == nil)
        for _, b in state.testLots do assert(world.get(b.shell).solid) end
        assert(world.count() == state.baseCount - 4)
    "#);
    assert_eq!(game.state_json()["room_lighting"]["active"], 0);
}

#[test]
fn roofs_cars_and_helicopters_do_not_stream_a_ground_floor() {
    let mut game = start(2, 1948);
    let still = [Controls::default(); 2];
    let board = Controls { pressed: Buttons(Buttons::NORTH), ..still[0] };
    eval(&mut game, r#"
        local b = state.testLots.hotel
        local down = vector.create(0, -1, 0)
        local roof = world.raycast(vector.create(b.x, 90, b.door.z - 1), down, 90, {tag="city"})
        assert(roof and roof.point.y > 4)
        world.place(state.fleet.players[1].body, roof.point + vector.create(0, .85, 0))
    "#);
    run(&mut game, 18, &still);
    eval(&mut game, "assert(state.fleet.players[1].pos.y > 4 and state.interiors.count == 0)");

    // Stage a parked car within streaming distance. Its driver boards from
    // beyond the load radius. Pilot the city's existing helicopter at the docks.
    eval(&mut game, r#"
        local b = state.testLots.hotel
        local car = state.fleet.vehicles[1]
        car.pos, car.yaw = b.door + vector.create(0, 0, 3.1), math.pi / 2
        car.native = vehicles.spawn(car.pos, car.yaw)
        world.place(car.body, car.pos + vector.create(0, .33, 0), car.yaw)
        for _, c in state.fleet.vehicles do
            if c.kind == "helicopter" then state.pilotHeli = c; break end
        end
        world.place(state.fleet.players[2].body, state.pilotHeli.pos + vector.create(2.6, .85, 0))
    "#);
    stage(&mut game, 1, "hotel", 0., 6.2);
    run(&mut game, 12, &still);
    run(&mut game, 1, &[board, board]);
    eval(&mut game, r#"
        assert(state.fleet.players[1].mode == "drive" and state.fleet.players[2].mode == "fly")
        local heli = state.pilotHeli
        heli.pos = state.testLots.warehouse.door + vector.create(0, 0, 3.5)
        world.place(heli.body, heli.pos + vector.create(0, 1, 0))
    "#);
    run(&mut game, 15, &still);
    eval(&mut game, r#"
        assert(vector.magnitude(state.fleet.players[1].pos - state.testLots.hotel.door) < 4.5)
        assert(vector.magnitude(state.fleet.players[2].pos - state.testLots.warehouse.door) < 4.5)
        assert(state.interiors.count == 0 and next(state.interiors.active) == nil)
    "#);
}

#[test]
fn approaching_glass_streams_the_room_and_the_pane_blocks_both_directions() {
    let mut game = start(1, 1948);
    let still = [Controls::default()];
    for kind in ["office", "hotel", "warehouse"] {
        eval(&mut game, &format!(r#"
            local b, p = state.testLots.{kind}, state.fleet.players[1]
            local blockX = (math.floor(b.x/56)+.5)*56
            -- Choose the outward wall of a paired hotel, and the quay's city-facing side.
            local side = if b.kind=="warehouse" or (b.kind=="hotel" and b.x<blockX) then -1 else 1
            local centerZ = if b.kind=="warehouse" then 0 else 1.4
            state.windowSide = side
            state.aisleZ = b.z+centerZ+1.05
            p.view = 0
            p.walk.vel = vector.create(0,0,0)
            world.place(p.body,vector.create(b.x+side*(b.w/2+9),.85,state.aisleZ))
        "#));
        run(&mut game, 8, &still);
        eval(&mut game, &format!("assert(not state.interiors.active[state.testLots.{kind}.id])"));
        eval(&mut game, &format!(r#"
            local b = state.testLots.{kind}
            world.place(state.fleet.players[1].body,vector.create(b.x+state.windowSide*(b.w/2+3),.85,state.aisleZ))
        "#));
        run(&mut game, 8, &still);
        let side = eval(&mut game, &format!(r#"
            local b, p = state.testLots.{kind}, state.fleet.players[1]
            assert(state.interiors.active[b.id] and not p.doorway and not p.building)
            local function glass_at(point, normal)
                local id
                for _, sign in {{-1,1}} do
                    local hit = world.raycast(point+sign*.4*normal,-sign*normal,.8,{{tag="city"}})
                    assert(hit and math.abs(hit.distance-.398)<.008, "expected the thin glazing")
                    local e = world.get(hit.id)
                    assert(e.shape=="pane" and e.solid and e.glass>0 and e.glass<.10)
                    assert(not id or id==hit.id)
                    id = hit.id
                end
            end
            local side = state.windowSide
            local y = if b.kind=="warehouse" then 2.98 else 1.55
            glass_at(vector.create(b.x+side*(b.w/2-.115),y,state.aisleZ),vector.create(side,0,0))
            glass_at(vector.create(b.x+2.43,1.55,b.door.z-.115),vector.create(0,0,1))
            return side
        "#)).parse::<f32>().unwrap();
        run(&mut game, 120, &[Controls { mv: [-side,0.], ..Default::default() }]);
        eval(&mut game, &format!(r#"
            local b, p = state.testLots.{kind}, state.fleet.players[1]
            local dist = (p.pos.x-b.x)*state.windowSide-b.w/2
            assert(dist>.25 and dist<.55 and not p.building, "walker crossed the glazed wall")
        "#));
    }
}

#[test]
fn room_activities_place_people_on_chairs_or_clear_floor() {
    for seed in [1948, 925] {
        let mut game = start(1, seed);
        for kind in ["office", "hotel", "warehouse"] {
            stage(&mut game, 1, kind, 0., 3.);
            run(&mut game, 10, &[Controls::default()]);
            eval(&mut game, &format!(r#"
                local b = state.testLots.{kind}
                local stations = state.interiors.activities[b.id]
                assert(#stations>=8)
                local walkers, sitters = 0, 0
                local function clear(p,role)
                    assert(math.abs(p.x-b.x)<b.w/2-.45 and math.abs(p.z-b.z)<b.d/2-.45)
                    for _, y in {{.43,.94,1.49}} do
                        for _, id in world.overlap(p+vector.create(0,y,0),.26,{{tag="city"}}) do
                            assert(not world.get(id).solid, role.." intersects blocker "..id)
                        end
                    end
                end
                for _, a in stations do
                    assert(a.role and a.pose and a.pos.y==0)
                    if a.pose=="sit" then
                        local chair = world.raycast(a.pos+vector.create(0,1.0,0),vector.create(0,-1,0),.8,{{tag="city"}})
                        assert(chair and chair.distance>.30 and chair.distance<.55, "missing seat under "..a.role)
                        sitters+=1
                    else
                        clear(a.pos,a.role)
                    end
                    if a.to then
                        assert(a.pose=="walk")
                        local length = vector.magnitude(a.to-a.pos)
                        assert(length>2)
                        local steps = math.ceil(length/.22)
                        for i=1,steps do clear(a.pos+(a.to-a.pos)*(i/steps),a.role) end
                        walkers+=1
                    end
                end
                assert(walkers>=2 and (b.kind=="warehouse" or sitters>=3))
            "#));
        }
    }
}
