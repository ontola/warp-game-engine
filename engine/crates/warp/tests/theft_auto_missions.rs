//! Exercise the published author-source entry point and the city's mission systems.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

const PASSENGER: &str = include_str!("../../../../games/warp-theft-auto/story_passenger.module.luau");
const RACE: &str = include_str!("../../../../games/warp-theft-auto/story_race.module.luau");
const CONTRACT: &str = include_str!("../../../../games/warp-theft-auto/story_contract.module.luau");

fn load() -> LuaGame {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut g=LuaGame::load(&dir,&Overrides {seed:Some(1948),players:Some(1),..Default::default()},None).unwrap();
    g.echo=false;
    g.tick(&[Controls::default()]);
    g
}

fn eval(g: &mut LuaGame, code: &str) -> String {
    g.luau(code).unwrap_or_else(|e| panic!("{e}\n{code}"))
}

fn advance(g: &mut LuaGame, input: Controls, count: usize) {
    for _ in 0..count { g.tick(&[input]); }
    assert!(g.errors().is_empty(), "{:?}",g.logs());
}

fn press(bit: u32) -> Controls { Controls { pressed: Buttons(bit), ..Default::default() } }

#[test]
fn forest_rally_uses_the_baked_gravel_and_starts_untimed() {
    let mut game=load();
    let source=include_str!("../../../../games/warp-theft-auto/story_rally.module.luau");
    assert_eq!(game.apply_story(source).unwrap(), "Mariner Ridge Rally");
    eval(&mut game,r#"
        local cast=state.storyCast
        local car=cast.cars.kestrel
        local steps=state.story.run.definition.steps
        assert(#steps==12 and state.story.action=="enter" and state.story.raceTime==nil)
        for i=2,#steps do
            local at=steps[i].pos
            local floor=world.raycast(at+vector.create(0,2,0),vector.create(0,-1,0),4,{tag="city"})
            assert(floor and math.abs(floor.point.y-at.y)<.10,"floating rally gate "..i)
        end
        world.place(state.fleet.players[1].body,car.pos+vector.create(1.6,.85,0))
    "#);
    advance(&mut game,Controls::default(),2);
    advance(&mut game,press(Buttons::NORTH),1);
    eval(&mut game,"assert(state.story.step==2 and state.story.raceTime~=nil)");
}

#[test]
fn wrong_definitions_leave_the_active_story_running() {
    let mut g=load();
    assert_eq!(g.apply_story(RACE).unwrap(),"Exchange Street Sprint");
    for raw in [
        r#"{}"#,
        r#"{"id":"x","title":"X","cast":[],"steps":[{"op":"contract","person":"missing","text":"Tag."}]}"#,
        r#"{"id":"x","title":"X","cast":[],"steps":[{"op":"checkpoint","pos":{"x":1,"y":"bad","z":1},"text":"Drive."}]}"#,
        r#"{"id":"x","title":"X","cast":[],"steps":[{"op":"checkpoint","pos":[999,0,0],"text":"Drive."}]}"#,
        r#"{"id":"x","title":"X","cast":[],"steps":[{"op":"checkpoint","pos":[1,0,0],"typo":1,"text":"Drive."}]}"#,
    ] { assert!(g.apply_story_data(serde_json::from_str(raw).unwrap()).is_err()); }
    assert_eq!(eval(&mut g,"return state.story.id"),"race");
    assert!(g.errors().is_empty());
}

#[test]
fn passenger_walks_to_a_real_car_rides_and_leaves_at_the_stop() {
    let mut g=load();
    assert_eq!(g.apply_story(PASSENGER).unwrap(),"Last Ferry");
    let start=eval(&mut g,"state.maraStart=state.storyCast:position('mara'); return state.story.step");
    assert_eq!(start,"1");
    advance(&mut g,Controls::default(),620);
    eval(&mut g,r#"assert(state.story.step==2 and vector.magnitude(state.storyCast:position('mara')-state.maraStart)>10)"#);
    advance(&mut g,press(Buttons::NORTH),1);
    advance(&mut g,Controls::default(),360);
    eval(&mut g,r#"
        local s,p=state.storyCast,state.fleet.players[1]
        assert(state.story.step==3 and s.people.mara.rideId==p.vehicle and p.mode=="drive")
        state.rideStart=s:position("mara")
    "#);
    advance(&mut g,Controls {mv:[0.,1.],..Default::default()},110);
    eval(&mut g,r#"
        local s=state.storyCast
        local car=state.fleet.vehicles[s.people.mara.rideId]
        assert(vector.magnitude(s:position("mara")-state.rideStart)>3)
        assert(vector.magnitude(s:position("mara")-car.pos)<1.2)
        -- Deliver the same car to the authored ferry stop. The real cast checks
        -- door clearance, places the passenger, and walks the final foot route.
        vehicles.remove(car.native)
        car.native=vehicles.spawn(state.story.target,car.yaw)
        car.active=true
    "#);
    advance(&mut g,Controls::default(),4);
    eval(&mut g,"assert(state.story.step==4 and not state.storyCast.people.mara.rideId)");
    advance(&mut g,Controls::default(),1250);
    eval(&mut g,r#"assert(state.story.status=="complete" and state.storyCast:arrived("mara"))"#);
}

#[test]
fn race_accepts_only_the_marked_car_and_an_ordered_gate_crossing() {
    let mut g=load();
    g.apply_story(RACE).unwrap();
    eval(&mut g,r#"
        local p=state.fleet.players[1]
        world.place(p.body,state.storyCast.cars.comet.pos+vector.create(1.6,.85,0))
    "#);
    advance(&mut g,Controls::default(),2);
    advance(&mut g,press(Buttons::NORTH),1);
    eval(&mut g,"assert(state.story.step==2 and state.story.raceTime~=nil); state.fleet.players[1].vehicle=1");
    // Each steered test position goes through the real native fleet and city tick.
    // A swept crossing can span a whole gate within one simulation step.
    let at = |g: &mut LuaGame, where_: &str| {
        eval(g,&format!(r#"
            local p=state.fleet.players[1]
            local car=state.fleet.vehicles[p.vehicle]
            vehicles.remove(car.native)
            car.native=vehicles.spawn({where_},0)
            car.active=true
            "#));
        advance(g,Controls::default(),1);
    };
    at(&mut g,"state.story.target+vector.create(0,0,14)");
    at(&mut g,"state.story.target");
    eval(&mut g,"assert(state.story.step==2); state.fleet.players[1].vehicle=state.storyCast.cars.comet.index");
    at(&mut g,"state.story.target");
    at(&mut g,"state.story.target");
    eval(&mut g,"assert(state.story.step==2)");
    at(&mut g,"state.story.target+vector.create(0,0,13)");
    at(&mut g,"state.story.target+vector.create(0,0,-13)");
    eval(&mut g,"assert(state.story.step==3)");
    at(&mut g,"vector.create(-56,0,-8)");
    at(&mut g,"vector.create(-56,0,-7)");
    eval(&mut g,"assert(state.story.step==3)");
    for step in 3..=6 {
        eval(&mut g,&format!("assert(state.story.step=={step})"));
        at(&mut g,"state.story.target+vector.create(0,0,12)");
        at(&mut g,"state.story.target");
    }
    eval(&mut g,"assert(state.story.status=='complete' and state.story.raceTime>0); state.finishTime=state.story.raceTime");
    advance(&mut g,Controls::default(),6);
    eval(&mut g,"assert(state.story.raceTime==state.finishTime)");
}

#[test]
fn contract_uses_the_owner_camera_and_a_clear_shot() {
    let mut g=load();
    g.apply_story(CONTRACT).unwrap();
    advance(&mut g,Controls::default(),2);
    eval(&mut g,r#"
        local s,p=state.storyCast,state.fleet.players[1]
        s:pose("jack","idle")
        local target=s:position("jack")
        -- Isolate the wall test from pedestrians crossing in front of the cover.
        for _,c in state.population.street do
            if math.abs(c.pos.x-target.x)<3 and math.abs(c.pos.z-target.z)<13 then
                c.path,c.action=nil,"idle"
                c.pos+=vector.create(5,0,0)
                c.nextPos=c.pos
            end
        end
        state.population.selectionIn=0
        world.place(p.body,target+vector.create(-.46,.85,7))
        p.view,p.pitch=0,-.055
        state.testWall=world.spawn("cube",{tag="city",solid=true,pos=target+vector.create(0,1.2,3),size=vector.create(2,3,.3)})
    "#);
    let aim=Controls {held:Buttons(Buttons::LEFT_TRIGGER),..Default::default()};
    let shot=Controls {held:Buttons(Buttons::LEFT_TRIGGER|Buttons::RIGHT_TRIGGER),pressed:Buttons(Buttons::RIGHT_TRIGGER),..Default::default()};
    advance(&mut g,aim,60);
    advance(&mut g,shot,1);
    eval(&mut g,r#"
        assert(state.story.weapon=="Arcade pistol" and state.story.aiming and not state.story.onTarget)
        assert(not state.storyCast:wasHit("jack") and state.story.shotFlash>0)
        world.remove(state.testWall)
    "#);
    advance(&mut g,aim,28);
    eval(&mut g,r#"assert(state.story.onTarget)"#);
    advance(&mut g,shot,1);
    eval(&mut g,r#"assert(state.story.status=="complete" and state.storyCast:wasHit("jack") and state.storyCast.people.jack.impact)"#);
}

#[test]
fn authored_explosion_plays_once_then_finishes() {
    let mut game = load();
    let script = r#"return {id="signal",title="Signal",cast={},steps={
        {op="explosion",pos=vector.create(10,1,-9),radius=5,text="Signal from the docks"}
    }}"#;
    game.apply_story(script).unwrap();
    advance(&mut game, Controls::default(), 12);
    eval(&mut game, r#"assert(state.story.status=="complete")"#);
    assert_eq!(game.core().world.explosions.as_ref().unwrap().explosion_stats().full, 1);
    assert!(game.apply_story(&script.replace("radius=5", "radius=-1")).is_err());
}

#[test]
fn a_story_can_equip_and_rebuild_one_players_growing_gun() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut g = LuaGame::load(&dir,&Overrides {seed:Some(1948),players:Some(2),..Default::default()},None).unwrap();
    g.echo = false;
    let idle = [Controls::default();2];
    g.tick(&idle);
    let source = include_str!("../../../../games/warp-theft-auto/story_loadout.module.luau");
    assert_eq!(g.apply_story(source).unwrap(),"Custom loadout");
    g.tick(&idle);
    eval(&mut g,r#"
        local p=state.fleet.players[1]
        assert(p.weapon=="shotgun" and p.weaponBuilds.shotgun.pellets==4)
        assert(p.weaponBuilds.shotgun.magazine==8 and p.weaponBuilds.shotgun.spread==.05)
        assert(not state.fleet.players[2].weaponBuilds and state.fleet.players[2].weapon~="shotgun")
    "#);
    for bad in ["{pellets=1.5}", "{magazine=-2}", "{spread=0/0}", "{automatic=7}", "{typo=1}"] {
        let invalid = source.replace("{pellets=4, magazine=8, spread=.05}",bad);
        assert!(g.apply_story(&invalid).is_err(),"{bad}");
    }
    assert!(g.apply_story(&source.replace("\"shotgun\"","\"launcher\"")).is_err());
    eval(&mut g,"assert(state.fleet.players[1].weaponBuilds.shotgun.magazine==8)");
    g.apply_story(&source.replace("params=params,", "")).unwrap();
    g.tick(&idle);
    eval(&mut g,r#"
        local p=state.fleet.players[1]
        assert(p.weapon=="shotgun" and not p.weaponBuilds.shotgun)
    "#);
    assert!(g.errors().is_empty(), "{:?}",g.logs());
}

#[test]
fn road_deaths_fail_required_cast_only_and_never_award_a_contract_shot() {
    let target = r#"{op="contract",person="arden",text="Find Arden."}"#;
    let checkpoint = r#"{op="checkpoint",pos=vector.create(50,0,50),text="Drive to the meeting."}"#;
    let passenger = r#"{op="pickup",person="arden",text="Collect Arden."}"#;
    for (steps, required) in [
        (target.to_owned(), true),
        (format!("{checkpoint},{passenger}"), true),
        (checkpoint.to_owned(), false),
    ] {
        let mut g = load();
        g.apply_story(&format!(r#"return {{id="ride",title="Ride",cast={{
            {{kind="person",key="arden",name="Arden",pos=vector.create(3.85,0,-45)}}
        }},steps={{{steps}}}}}"#)).unwrap();
        advance(&mut g, Controls::default(), 169);
        advance(&mut g, press(Buttons::NORTH), 1);
        eval(&mut g, "assert(state.fleet.players[1].vehicle==1)");
        let gas = Controls { mv: [0.,1.], ..Default::default() };
        for _ in 0..330 {
            advance(&mut g, gas, 1);
            if eval(&mut g, "return state.storyCast.people.arden.dead") == "true" { break; }
        }
        eval(&mut g, r#"
            local p=state.storyCast.people.arden
            assert(p.dead and p.roadDeath and not p.shot and p.corpse)
            local shooter=state.combat.players[1]
            assert(shooter.hits==0 and shooter.defeats==0 and shooter.shotFlash==0)
            assert(not state.story.shotPeople)
        "#);
        assert_eq!(eval(&mut g, "return state.story.status"), if required { "failed" } else { "active" });
        if required {
            assert_eq!(eval(&mut g, "return state.story.objective"), "Arden was hit by a vehicle.");
            eval(&mut g, "assert(not state.story.target and not state.story.contractPerson)");
        }
    }
}


#[test]
fn edited_rally_keeps_the_active_turn_and_clock_when_checkpoint_flags_change() {
    let mut g=load();
    let rally=include_str!("../../../../games/warp-theft-auto/story_rally.module.luau");
    g.apply_story(rally).unwrap();
    eval(&mut g,r#"
        local car=state.storyCast.cars.kestrel
        state.rallyCar=car.native
        local old=car.pos
        local edit=get_layout()
        edit.rally.carAlong+=4
        assert(apply_layout(edit))
        assert(car.native==state.rallyCar and vector.magnitude(car.pos-old)>3.5)
        assert(vector.magnitude(state.story.target-car.pos)<.1)
        world.place(state.fleet.players[1].body,car.pos+vector.create(1.6,.85,0))
    "#);
    advance(&mut g,Controls::default(),2);
    advance(&mut g,press(Buttons::NORTH),1);
    advance(&mut g,Controls::default(),5);
    eval(&mut g,r#"
        local story=state.story
        assert(story.step==2 and story.raceTime>0)
        local first,timer=story.target,story.raceTime
        local edit=get_layout()
        edit.rally.bends[2][3]=1
        assert(apply_layout(edit))
        assert(story.total==13 and story.step==3 and story.target==first)
        assert(story.raceTime==timer and state.storyCast.cars.kestrel.native==state.rallyCar)
        edit.rally.bends[3][3]=nil
        assert(apply_layout(edit))
        assert(story.total==12 and story.step==3 and vector.magnitude(story.target-first)>20)
        assert(story.raceTime==timer and state.fleet.players[1].vehicle==state.storyCast.cars.kestrel.index)
        local oldCape=state.city.cape
        local count=world.count()
        edit.groves={}
        for i=1,16 do table.insert(edit.groves,{-695,-60,250,250}) end
        local accepted,err=apply_layout(edit)
        assert(not accepted and err and state.city.cape==oldCape and world.count()==count)
    "#);
}

#[test]
fn a_moved_ferry_carries_the_ship_dispatch_and_passenger_route_together() {
    let mut g=load();
    g.apply_story(PASSENGER).unwrap();
    eval(&mut g,r#"
        local hull=state.city.ferry
        local home=world.get(hull).pos
        assert(vector.magnitude(home-vector.create(210,0,46))<.01)
        local edit=get_layout()
        edit.places.ferryTerminal={164.5,-52}
        assert(apply_layout(edit))
        assert(hull==state.city.ferry)
        local ship=world.get(hull).pos
        assert(vector.magnitude(ship-vector.create(210,0,-65))<.01)
        local ferry=state.job.stops.drive[2]
        assert(ferry.name=="Ferry Terminal" and ferry.pos.z==-52)
        local steps=state.story.run.definition.steps
        assert(steps[3].pos==vector.create(109,0,-51))
        assert(steps[4].points[1]==vector.create(120,0,-49))
        assert(steps[4].points[2]==vector.create(120,0,-38))
    "#);
}
