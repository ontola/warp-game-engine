//! The shipped city uses the same fitted body for firing, impacts and all paint variants.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

fn city() -> LuaGame {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(&path, &Overrides { seed: Some(1948), players: Some(2), ..Default::default() }, None).unwrap();
    game.echo = false;
    tick(&mut game, Controls::default(), 1);
    game
}

fn tick(game: &mut LuaGame, input: Controls, n: usize) {
    for _ in 0..n { game.tick(&[input, Controls::default()]); }
    assert!(game.errors().is_empty(), "{:?}", game.logs());
}

#[test]
fn a_real_crash_crushes_both_body_ends_and_streams_their_panels() {
    let mut game = city();
    game.luau(r#"
        local v=vector.create
        local car=state.fleet.vehicles[1]
        assert(car.damage.total==0)
        local other=state.combat.cast:car("parked-test",{pos=car.pos+v(0,0,-22),yaw=0,paint="pearl"})
        state.crashOther=other.index
        local origin=other.pos+v(0,1.1,5)
        assert(vehicles.raycast(other.native,"toyama",origin,v(0,0,-1),10))
        assert(not vehicles.raycast(other.native,"toyama",origin+v(0,.55,0),v(0,0,-1),10))
        state.initialRear=vehicles.raycast(other.native,"toyama",other.pos+v(0,.7,5),v(0,0,-1),10)
        local surface=vehicles.closest_point(other.native,"toyama",other.pos+v(0,.55,6))
        assert(surface.z-other.pos.z>2 and surface.z-other.pos.z<2.3)
    "#).unwrap();
    let gas=Controls { mv:[0.,1.], ..Controls::default() };
    tick(&mut game, Controls { pressed:Buttons(Buttons::NORTH), ..Controls::default() },1);
    for _ in 0..320 {
        tick(&mut game,gas,1);
        if game.luau("return state.fleet.vehicles[state.crashOther].damage.total>.04").unwrap()=="true" { break; }
    }
    tick(&mut game,Controls::default(),16);
    game.luau(r#"
        local f=state.fleet; local a,b=f.vehicles[1],f.vehicles[state.crashOther]
        assert(a.damage.zones.front>.055 and b.damage.zones.rear>.055)
        assert(a.damage.zones.front>a.damage.zones.rear and b.damage.zones.rear>b.damage.zones.front)
        assert(a.panels and b.panels and not world.get(b.body).visible)
        assert(world.get(b.wheels[1]).visible and world.get(b.panels[8]).visible)
        local panel=world.get(b.panels[7])
        assert(panel.size.z<.96 and panel.pos.z>0)
        local v=vector.create
        local rear=vehicles.raycast(b.native,"toyama",b.pos+v(0,.7,5),v(0,0,-1),10)
        assert(rear and rear>state.initialRear+.04)
        local zip=vehicles.closest_point(b.native,"toyama",b.pos+v(0,.7,5))
        assert(zip.z<b.pos.z+2.14)
        -- A severely folded bumper no longer intercepts fire at its old outline.
        local origin=a.pos+v(0,.8,-5)
        vehicles.crush_query(a.native,"toyama",v(-.48,0,-1.04),v(.48,0,1.16),v(1,1,1),v(.31,1,1))
        local shortened=vehicles.closest_point(a.native,"toyama",origin)
        assert(vector.magnitude(origin-shortened)>3.5)
        state.savedDamage=b.damage.total
        -- The same parked car keeps its damage after both cameras leave.
        for _,p in f.players do
            if p.vehicle then f.vehicles[p.vehicle].owner=nil; p.vehicle=nil; p.mode="walk" end
            world.place(p.body,vector.create(400,.85,400))
        end
    "#).unwrap();
    tick(&mut game,Controls::default(),22);
    game.luau(r#"
        local b=state.fleet.vehicles[state.crashOther]
        assert(not b.panels and world.get(b.body).visible and b.damage.total==state.savedDamage)
        world.place(state.fleet.players[1].body,b.pos+vector.create(3,.85,3))
    "#).unwrap();
    tick(&mut game,Controls::default(),22);
    game.luau("local b=state.fleet.vehicles[state.crashOther]; assert(b.panels and not world.get(b.body).visible and b.damage.total==state.savedDamage)").unwrap();
}

#[test]
fn scripted_fleet_damage_stays_bounded_across_split_screen() {
    let mut game = city();
    game.luau(r#"
        local f=state.fleet
        for i=1,13 do
            local car=state.combat.cast:car("damage-stream-"..i,{pos=vector.create(1+3*i,0,-8),yaw=.2})
            car.damage.total=.7
            car.damage.zones.front=.75
            car.damage.zones.right=.4
        end
    "#).unwrap();
    tick(&mut game,Controls::default(),20);
    game.luau(r#"
        local cars=0
        for _,car in state.fleet.vehicles do if car.panels then cars+=1 end end
        assert(cars==8 and #world.find("damage-panel")==72)
        assert(world.count()+96<=world.capacity())
    "#).unwrap();
}
