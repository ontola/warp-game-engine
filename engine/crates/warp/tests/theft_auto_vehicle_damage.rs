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
fn a_real_crash_dents_both_body_ends_and_streams_only_the_hit_meshes() {
    let mut game = city();
    game.luau(r#"
        local v=vector.create
        local car=state.fleet.vehicles[1]
        assert(car.damage.total==0)
        local other=state.combat.cast:car("parked-test",{pos=car.pos+v(0,0,-22),yaw=0,paint="pearl"})
        state.crashOther=other.index
        local twin=state.combat.cast:car("unhit-test",{pos=other.pos+v(-8,0,0),yaw=0,paint="pearl"})
        state.crashTwin=twin.index
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
        assert(a.dentActive and b.dentActive and world.get(b.body).visible and world.get(b.wheels[1]).visible)
        local dent=b.damage.dents[1]
        assert(dent and dent.depth>.05 and #b.damage.dents<=8)
        local v=vector.create
        local twin=f.vehicles[state.crashTwin]
        assert(twin.damage.total==0 and not twin.dentActive)
        local probe=dent.center+v(0,.33,0)-dent.inward*2
        local before=vehicles.raycast(twin.native,"toyama",twin.pos+probe,dent.inward,4)
        local after=vehicles.raycast(b.native,"toyama",b.pos+probe,dent.inward,4)
        assert(before and after and after>before+.035,"hit at actual contact must sink locally")
        local far=v(if dent.center.x<0 then .59 else -.59,.65,5)
        local d1=vehicles.raycast(b.native,"toyama",b.pos+far,v(0,0,-1),8)
        local d2=vehicles.raycast(twin.native,"toyama",twin.pos+far,v(0,0,-1),8)
        assert(d1 and d2 and math.abs(d1-d2)<.12,"opposite end of the bumper should keep its shape")
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
        assert(not b.dentActive and world.get(b.body).visible and b.damage.total==state.savedDamage)
        world.place(state.fleet.players[1].body,b.pos+vector.create(3,.85,3))
    "#).unwrap();
    tick(&mut game,Controls::default(),22);
    game.luau("local b=state.fleet.vehicles[state.crashOther]; assert(b.dentActive and world.get(b.body).visible and b.damage.total==state.savedDamage)").unwrap();
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
            car.damage.dents={{center=vector.create(-.6,.4,-2),inward=vector.create(.1,0,1),radius=.85,depth=.32}}
            if i==13 then car.missingParts=4 end
        end
    "#).unwrap();
    tick(&mut game,Controls::default(),20);
    game.luau(r#"
        local cars=0
        for _,car in state.fleet.vehicles do if car.dentActive then cars+=1 end end
        assert(cars==8 and #world.find("damage-panel")==0 and #world.find("vehicle-detail")==0)
        assert(world.count()+96<=world.capacity())
    "#).unwrap();
}

#[test]
fn ordinary_damage_starts_a_shared_fire_that_finishes_after_everyone_leaves() {
    let mut game = city();
    game.luau(r#"
        local car=state.fleet.vehicles[1]
        car.damage.total=.78
        local seat=state.fleet.players[1]
        world.place(seat.body,car.pos+vector.create(0,.85,5.6))
        seat.view,seat.pitch=0,-.14
        state.fireBody,state.fireNative=car.body,car.native
    "#).unwrap();
    let fire = Controls { pressed: Buttons(Buttons::RIGHT_TRIGGER), ..Default::default() };
    tick(&mut game, fire, 1);
    game.luau(r#"
        local car=state.fleet.vehicles[1]
        assert(car.damage.total>.78 and car.damage.total<.82 and not car.burnRemaining)
    "#).unwrap();
    tick(&mut game, Controls::default(), 25);
    tick(&mut game, fire, 1);
    game.luau(r#"
        local car=state.fleet.vehicles[1]
        assert(car.burnRemaining>5.9 and car.fireOwner==1 and not car.wrecked)
        world.place(state.fleet.players[1].body,vector.create(0,.85,-150))
        world.place(state.fleet.players[2].body,car.pos+vector.create(3,.85,3))
    "#).unwrap();
    tick(&mut game, Controls::default(), 155);
    game.luau(r#"
        local f=state.fleet
        local car=f.vehicles[1]
        assert(f.firePulse>10 and car.burnRemaining>3 and car.burnRemaining<3.6)
        assert(not car.wrecked and car.body==state.fireBody and car.native==state.fireNative)
        state.lastFirePulse=f.firePulse
        world.place(f.players[2].body,vector.create(2,.85,-150))
    "#).unwrap();
    tick(&mut game, Controls::default(), 55);
    game.luau(r#"
        local f=state.fleet
        local car=f.vehicles[1]
        assert(f.firePulse==state.lastFirePulse and not car.wrecked)
        assert(car.burnRemaining<3 and car.charred and world.get(car.body).visible)
    "#).unwrap();
    tick(&mut game, Controls::default(), 160);
    game.luau(r#"
        local car=state.fleet.vehicles[1]
        assert(car.wrecked and car.charred and not car.burnRemaining)
        assert(car.damage.disabled and car.damage.total==1 and not car.voice)
        assert(car.native==state.fireNative and car.body==state.fireBody and world.exists(car.body))
        assert(car.missingParts==126 and car.missingWheels==15)
        assert(car.wreckFireRemaining>23 and state.fleet.firePulse==state.lastFirePulse)
        world.place(state.fleet.players[2].body,car.pos+vector.create(5,.85,5))
    "#).unwrap();

    // The distant blast has passed. A returning observer sees fresh wreck
    // effects until the same shared timer runs out, without a second explosion.
    tick(&mut game, Controls::default(), 300);
    let effects = |game: &LuaGame| game.core().world.explosions.as_ref().unwrap().len();
    assert!(effects(&game) >= 4);
    game.luau(r#"
        local car=state.fleet.vehicles[1]
        assert(car.wreckFireRemaining>18 and car.wreckFireRemaining<19)
        assert(state.fleet.firePulse>state.lastFirePulse)
    "#).unwrap();
    tick(&mut game, Controls::default(), 720);
    game.luau("assert(state.fleet.vehicles[1].wreckFireRemaining < 7)").unwrap();
    assert!(effects(&game) > 0);
    tick(&mut game, Controls::default(), 650);
    game.luau("assert(not state.fleet.vehicles[1].wreckFireRemaining and state.fleet.vehicles[1].charred)").unwrap();
    assert_eq!(effects(&game), 0);
    assert_eq!(game.core().world.explosions.as_ref().unwrap().explosion_stats().full, 1);
}
