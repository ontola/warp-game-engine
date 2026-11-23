//! World geometry must follow exactly the same objective that advances the mission.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

const PASSENGER: &str = include_str!("../../../../games/warp-theft-auto/story_passenger.module.luau");

fn two_seats() -> LuaGame {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(
        &dir,
        &Overrides { players: Some(2), seed: Some(1948), ..Default::default() },
        None,
    ).unwrap();
    game.echo = false;
    tick(&mut game, 2);
    game
}

fn tick(game: &mut LuaGame, count: usize) {
    for _ in 0..count { game.tick(&[Controls::default(); 2]); }
    assert!(game.errors().is_empty(), "{:?}", game.errors());
}

fn eval(game: &mut LuaGame, script: &str) {
    game.luau(script).unwrap_or_else(|err| panic!("{script}\n{err}"));
}

#[test]
fn dispatch_and_moving_passenger_share_one_world_marker_between_views() {
    let mut game = two_seats();
    eval(&mut game, r#"
        assert(#world.find("objective-anchor") == 1 and #world.find("objective-beacon") == 2)
        local zone = world.get(world.find("objective-zone")[1])
        assert(zone.parent == state.waypoint.root and not zone.solid and zone.visible and zone.size.x == 18)
        assert(world.get(state.waypoint.root).pos == state.job.stops.drive[2].pos)
        state.testDispatchRoot = state.waypoint.root
        state.job.pause = 1
    "#);
    tick(&mut game, 1);
    eval(&mut game, r#"assert(not state.waypoint and not world.exists(state.testDispatchRoot) and #world.find("objective-beacon")==0)"#);
    game.apply_story(PASSENGER).unwrap();
    tick(&mut game, 1);
    eval(&mut game, r#"
        assert(#world.find("objective-anchor") == 1 and #world.find("objective-zone") == 0)
        state.testPersonRoot = state.waypoint.root
        state.testPersonStart = world.get(state.waypoint.root).pos
    "#);
    tick(&mut game, 80);
    eval(&mut game, r#"
        local p = world.get(state.testPersonRoot).pos
        assert(p == state.storyCast:position("mara") and vector.magnitude(p-state.testPersonStart)>1)
        for _,id in world.find("objective-beacon") do
            local b=world.get(id)
            assert(b.parent==state.testPersonRoot and b.visible and not b.solid and b.world_pos.y>p.y+2)
        end
    "#);
}

#[test]
fn custom_checkpoint_draws_the_authored_radius_and_clears_on_crossing() {
    let mut game = two_seats();
    game.apply_story(r#"
        local v = vector.create
        return {id="gate",title="Gate",cast={},steps={
            {op="checkpoint",pos=v(0,0,-38),radius=4.5,text="Through the gate."}
        }}
    "#).unwrap();
    tick(&mut game, 1);
    eval(&mut game,r#"
        local m = state.waypoint
        assert(m.kind=="checkpoint" and world.get(m.root).pos==state.story.target)
        assert(world.get(world.find("objective-zone")[1]).size.x==9 and #world.find("objective-post")==4)
        for _,id in world.find("objective-post") do
            local post=world.get(id)
            assert(not post.solid and math.abs(vector.magnitude(vector.create(post.pos.x,0,post.pos.z))-4.5)<.001)
        end
        local car=state.fleet.vehicles[1]
        world.place(state.fleet.players[1].body,car.pos+vector.create(1.4,.85,0))
    "#);
    tick(&mut game, 2);
    game.tick(&[Controls{pressed:Buttons(Buttons::NORTH),..Default::default()},Controls::default()]);
    tick(&mut game, 1);
    for offset in [10, 0] {
        eval(&mut game, &format!(r#"
            local seat=state.fleet.players[1]
            assert(seat.mode=="drive")
            local car=state.fleet.vehicles[seat.vehicle]
            vehicles.remove(car.native)
            car.native=vehicles.spawn(state.story.target+vector.create(0,0,{offset}),0)
            car.active=true
        "#));
        tick(&mut game, 1);
    }
    eval(&mut game, r#"
        assert(state.story.status=="complete" and not state.waypoint)
        for _,tag in {"objective-anchor","objective-zone","objective-post","objective-beacon"} do
            assert(#world.find(tag)==0)
        end
    "#);
}
