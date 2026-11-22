//! Ambient Warp Humans follow shared street schedules and streamed room furniture.
use app::{Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

fn eval(game: &mut LuaGame, code: &str) -> String {
    game.luau(code).unwrap_or_else(|e| panic!("{code}\n{e}"))
}

fn tick(game: &mut LuaGame, count: usize) {
    for _ in 0..count { game.tick(&[Controls::default(); 4]); }
    assert!(game.errors().is_empty(), "{:?}", game.errors());
}

fn load(seats: usize, seed: u64) -> LuaGame {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(&folder, &Overrides {
        players: Some(seats), seed: Some(seed), ..Default::default()
    }, None).unwrap();
    game.echo = false;
    tick(&mut game, 36);
    game
}

#[test]
fn downtown_draws_a_full_crowd_from_deterministic_sidewalk_schedules() {
    let sample = |seed| {
        let mut game = load(1, seed);
        eval(&mut game, r#"
            local p = state.population
            local seat = state.fleet.players[1]
            local close, active = 0, 0
            for id, actor in p.pedestrians.active do
                local c = p.residents[id]
                local distance = vector.magnitude(c.pos - seat.pos)
                if distance < 18 then close += 1 end
                assert(not c.building and distance < 75 and humans.head(actor))
                active += 1
            end
            assert(p.streetCount > 650 and active == 40 and close >= 4)
            assert(state.interiors.count == 0 and next(p.rooms) == nil)
            local routes = {}
            for i = 1, 6 do routes[i] = {p.residents[i].pos, p.residents[i].yaw} end
            return {p.streetCount, active, close, routes}
        "#)
    };
    let current = sample(1948);
    assert_eq!(current, sample(1948));
    assert_ne!(current, sample(2075));
    let stats: serde_json::Value = serde_json::from_str(&current).unwrap();
    eprintln!("{} outdoor residents, {} live, {} within 18 m", stats[0], stats[1], stats[2]);
}

#[test]
fn multiple_visitors_share_workers_and_four_rooms_get_moving_residents() {
    let mut game = load(4, 1948);
    eval(&mut game, r#"
        state.testRooms = {}
        for _, b in state.city.buildings do
            if b.kind=="hotel" and not state.testRooms[1] then state.testRooms[1]=b
            elseif b.kind=="office" and not state.testRooms[2] then state.testRooms[2]=b
            elseif b.kind=="warehouse" and not state.testRooms[3] then state.testRooms[3]=b
            elseif b.kind=="office" and not state.testRooms[4] then state.testRooms[4]=b end
        end
        for i=1,2 do
            world.place(state.fleet.players[i].body,state.testRooms[1].door + vector.create(i-.5,.85,2.5))
        end
    "#);
    tick(&mut game, 2);
    eval(&mut game, r#"
        local b, p = state.testRooms[1], state.population
        assert(state.interiors.count==1 and #p.rooms[b.id]==#state.interiors.activities[b.id])
        state.hotelIdentities = {}
        for _, idx in p.rooms[b.id] do
            local actor = p.pedestrians.active[idx]
            if actor then state.hotelIdentities[idx] = actor end
        end
        assert(next(state.hotelIdentities))
        for i, b in state.testRooms do
            world.place(state.fleet.players[i].body, b.door + vector.create(0,.85,2.5))
        end
    "#);
    tick(&mut game, 2);
    eval(&mut game, r#"
        for i, b in state.testRooms do
            world.place(state.fleet.players[i].body, b.door + vector.create(0,.85,-3))
        end
    "#);
    tick(&mut game, 32);
    let occupants = eval(&mut game, r#"
        local p = state.population
        local counts = {}
        state.walkers = {}
        local reused = 0
        for idx, old in state.hotelIdentities do
            if p.pedestrians.active[idx]==old then reused+=1 end
        end
        assert(reused >= 3, "the first visitor's neighbours keep their actors as other seats move")
        for i, b in state.testRooms do
            assert(state.fleet.players[i].building==b.id)
            local count, seated, walkers = 0, 0, 0
            for _, idx in p.rooms[b.id] do
                local c, actor = p.residents[idx], p.pedestrians.active[idx]
                if actor then
                    assert(humans.head(actor) and c.building==b.id)
                    if c.action=="sit" then
                        local hips = humans.joint(actor,"hips")
                        assert(hips and hips.y < c.pos.y + .90, "seated resident's hips are lowered")
                        seated+=1
                    elseif c.delta then
                        state.walkers[idx] = c.pos
                        walkers+=1
                    end
                    count+=1
                end
            end
            assert(count>=6 and walkers>=1)
            if b.kind~="warehouse" then assert(seated>=1) end
            counts[i]=count
        end
        return counts
    "#);
    tick(&mut game, 300);
    eval(&mut game, r#"
        local moved=0
        for idx, old in state.walkers do
            if vector.magnitude(state.population.residents[idx].pos-old)>.7 then moved+=1 end
        end
        assert(moved>=4, "indoor workers should walk their assigned aisles")
        state.hotelIds = state.population.rooms[state.testRooms[1].id]
        state.hotelActors = {}
        for _, idx in state.hotelIds do
            table.insert(state.hotelActors,state.population.pedestrians.active[idx])
        end
        player_left(players.get(1))
    "#);
    tick(&mut game, 3);
    eval(&mut game, r#"
        assert(state.population.rooms[state.testRooms[1].id]==nil and state.interiors.count==3)
        for _, idx in state.hotelIds do assert(state.population.residents[idx]==nil) end
        for _, actor in state.hotelActors do assert(humans.head(actor)==nil) end
        for i=2,4 do player_left(players.get(i)) end
    "#);
    tick(&mut game, 30);
    eval(&mut game, r#"
        assert(next(state.population.rooms)==nil and next(state.population.pedestrians.active)==nil)
        assert(humans.room("resident")==40 and humans.room("local")==4)
        assert(state.interiors.count==0)
    "#);
    eprintln!("Assigned indoor residents across four split-screen rooms: {occupants}");
}

#[test]
fn people_are_visible_through_a_window_and_unload_when_the_observer_walks_away() {
    let mut game = load(1, 1948);
    eval(&mut game, r#"
        for _, b in state.city.buildings do
            if b.kind=="office" and b.x>0 and b.z<0 then state.displayOffice=b; break end
        end
        local b, seat = state.displayOffice, state.fleet.players[1]
        seat.view = math.pi/2
        world.place(seat.body, vector.create(b.x+b.w/2+2,.85,b.z+1.4))
    "#);
    tick(&mut game, 26);
    eval(&mut game, r#"
        local b, p, seat = state.displayOffice, state.population, state.fleet.players[1]
        assert(seat.nearRoom==b.id and not seat.building and not seat.doorway)
        state.windowActors = {}
        for _, idx in p.rooms[b.id] do
            local actor = p.pedestrians.active[idx]
            if actor then table.insert(state.windowActors,actor) end
        end
        assert(#state.windowActors >= 6, "a lone observer outside the glass must see people indoors")
    "#);
    let away = Controls { mv: [0., -1.], ..Default::default() };
    for _ in 0..150 { game.tick(&[away]); }
    assert!(game.errors().is_empty(), "{:?}", game.errors());
    eval(&mut game, r#"
        local b, p = state.displayOffice, state.population
        assert(state.fleet.players[1].pos.x>b.x+b.w/2+7)
        assert(not p.rooms[b.id] and state.interiors.count==0)
        for _, actor in state.windowActors do assert(not humans.head(actor)) end
        for idx in p.pedestrians.active do assert(not p.residents[idx].building) end
    "#);
}
