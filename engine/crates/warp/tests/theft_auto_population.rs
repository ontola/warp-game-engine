//! Ambient Warp Humans follow shared street schedules and streamed room furniture.
use app::{Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

fn eval(game: &mut LuaGame, code: &str) -> String {
    game.luau(code).unwrap_or_else(|e| panic!("{code}\n{e}"))
}

fn tick(game: &mut LuaGame, count: usize) {
    for _ in 0..count { game.tick(&[Controls::default(); 4]); }
    assert!(game.errors().is_empty(), "{:?}", game.logs());
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
    assert!(game.errors().is_empty(), "{:?}", game.logs());
    eval(&mut game, r#"
        local b, p = state.displayOffice, state.population
        assert(state.fleet.players[1].pos.x>b.x+b.w/2+7)
        assert(not p.rooms[b.id] and state.interiors.count==0)
        for _, actor in state.windowActors do assert(not humans.head(actor)) end
        for idx in p.pedestrians.active do assert(not p.residents[idx].building) end
    "#);
}

#[test]
fn walking_between_views_preserves_one_body_and_appearance_per_resident() {
    for seats in 1..=4 {
        let mut game = load(seats, 1948);
        let read = |game: &mut LuaGame, expr: &str| -> Vec<u32> {
            serde_json::from_str(&eval(game, expr)).unwrap()
        };
        let initial = read(&mut game, r#"
            local p, nearest, distance = state.population, nil, math.huge
            local viewer = state.fleet.players[1].pos
            for id, actor in p.pedestrians.active do
                local c = p.residents[id]
                local d = vector.magnitude(c.pos - viewer)
                if c.path and d < distance then nearest, distance = id, d end
            end
            state.tracked = nearest
            local c = p.residents[nearest]
            local looks = {}
            for _, row in p.street do looks[row.look] = true end
            local count=0
            for _ in looks do count += 1 end
            assert(count==72)
            for i=1,#p.street-1 do assert(p.street[i].look~=p.street[i+1].look) end
            return {nearest, p.pedestrians.active[nearest], c.look, p.streetCount}
        "#);
        let (id, actor, look) = (initial[0] as usize, initial[1], initial[2] as usize);
        {
            let core = game.core();
            assert_eq!(core.humans.people[&actor].look, look);
            assert!(core.humans.distant_pose("resident", id, 1.).is_none());
            let distant = (1..=initial[3] as usize).filter(|&i| core.humans.distant_pose("resident", i, 1.).is_some()).count();
            assert_eq!(distant, initial[3] as usize - 40);
        }
        eval(&mut game, r#"
            for _, seat in state.fleet.players do world.place(seat.body, vector.create(147,.85,150)) end
        "#);
        tick(&mut game, 28);
        let (start, background_look) = game.core().humans.distant_pose("resident", id, 1.).unwrap();
        assert_eq!(background_look, look);
        // This gait advances between the low-frequency roster updates.
        tick(&mut game, 6);
        let (end, _) = game.core().humans.distant_pose("resident", id, 1.).unwrap();
        assert!(end.distance(start) < 0.28);
        eval(&mut game, r#"
            local c = state.population.residents[state.tracked]
            world.place(state.fleet.players[1].body,c.pos+vector.create(0,.85,0))
        "#);
        tick(&mut game, 27);
        let handle = read(&mut game, "return {state.population.pedestrians.active[state.tracked]}")[0];
        let core = game.core();
        assert_ne!(handle, actor);
        assert_eq!(core.humans.people[&handle].look, look);
        assert!(core.humans.distant_pose("resident", id, 1.).is_none());
        assert!(core.humans.people[&handle].pos.distance(end) < 1.15);
    }
}

#[test]
fn residents_finish_different_shifts_away_from_observers() {
    let mut game = load(1, 1948);
    let sample = |g: &mut LuaGame, hour: f32| -> usize {
        eval(g, &format!("state.clock.hour={hour}; state.population.selectionIn=0"));
        tick(g, 1);
        eval(g, r#"
            local count=0
            for _,c in state.population.street do if c.active then count+=1 end end
            return count
        "#).parse().unwrap()
    };
    eval(&mut game, r#"
        local p, viewer = state.population, state.fleet.players[1].pos
        state.kept = {}
        for id,c in p.street do
            if not c.night and vector.magnitude(c.pos-viewer)<60 then table.insert(state.kept,id) end
        end
        assert(#state.kept>=4)
    "#);
    let counts: Vec<_> = [17.9, 18.5, 19.0, 19.5, 20.0, 20.5, 21.1].into_iter().map(|h| sample(&mut game,h)).collect();
    assert!(counts.windows(2).all(|p| p[0] > p[1]), "{counts:?}");
    eval(&mut game, "for _,id in state.kept do assert(state.population.street[id].active) end");
    let morning: Vec<_> = [5.2, 6., 7., 8.].into_iter().map(|h| sample(&mut game,h)).collect();
    assert!(morning.windows(2).all(|p| p[0] < p[1]), "{morning:?}");
}
