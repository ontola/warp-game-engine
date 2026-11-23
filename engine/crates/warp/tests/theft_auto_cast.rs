//! Named mission actors use the shipped world, human pool, AI controller and fleet.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

#[test]
fn story_cast_walks_drives_rides_and_releases_owned_entities() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut g = LuaGame::load(&dir, &Overrides { seed: Some(1948), players: Some(1), ..Default::default() }, None).unwrap();
    g.echo = false;
    let idle = Controls::default();
    let board = Controls { pressed: Buttons(Buttons::NORTH), ..idle };
    g.tick(&[idle]);
    let story = std::fs::read_to_string(dir.join("story_passenger.module.luau")).unwrap();
    g.apply_story(&story).unwrap();
    let eval = |g: &mut LuaGame, code: &str| g.luau(code).unwrap_or_else(|e| panic!("{e} in {code}"));
    eval(&mut g, r#"
        local s=state.storyCast
        state.castTest=s
        local a=assert(s:person("driver",{name="Ari",look=17,pos=vector.create(-6.4,0,-19)}))
        local b=assert(s:person("guest",{name="Noor",look=39,pos=vector.create(-6,0,-27)}))
        local c=assert(s:car("silver",{paint="pearl",pos=vector.create(-2.7,0,-25)}))
        assert(c.paint=="pearl" and c.index==state.fleet.baseCars+1 and a.name=="Ari" and b.look==39)
        assert(s:walk("driver",{vector.create(-6.4,0,-23)},1.6))
    "#);
    let advance = |g: &mut LuaGame, n: usize| {
        for _ in 0..n { g.tick(&[Controls::default()]); }
        assert!(g.errors().is_empty(), "{:?}", g.errors());
    };
    advance(&mut g, 180);
    eval(&mut g, r#"
        local s=state.castTest
        assert(s:arrived("driver") and vector.magnitude(s:position("driver")-vector.create(-6.4,0,-23))<.15)
        assert(s:ride("driver","silver","driver") and s:ride("guest",s.cars.silver.index,"passenger"))
        assert(not s:ride("guest","silver","driver") and s.people.guest.pose=="sit")
        state.startCar=s:position("silver")
        assert(s:drive("silver",{vector.create(-2.7,0,-42)},6))
    "#);
    for _ in 0..900 {
        advance(&mut g,1);
        if eval(&mut g,"return state.castTest:arrived('silver')")=="true" { break; }
    }
    eval(&mut g, r#"
        local s=state.castTest; local c=s.cars.silver
        assert(s:arrived("silver") and vector.magnitude(c.pos-state.startCar)>14)
        assert(vector.magnitude(s:position("guest")-c.pos)<1.2)
        assert(vector.magnitude(s:position("guest")-s:position("driver"))>.75)
        for _,p in {s.people.driver,s.people.guest} do
            local head=humans.joint(p.actor,"head")
            assert(head and head.y-c.pos.y>.85 and head.y-c.pos.y<1.45,"seated head must fit under roof")
        end
        assert(s:dismount("driver"))
        local p=state.fleet.players[1]; world.place(p.body,c.pos+vector.create(1.95,.85,0))
    "#);
    g.tick(&[idle]); g.tick(&[board]);
    eval(&mut g,r#"
        local s=state.castTest; local c=s.cars.silver
        assert(c.owner==1 and s:dismount("guest"))
        assert(s:ride("guest",state.fleet.players[1].vehicle,"passenger"))
        state.castNative,state.castIndex=c.native,c.index
        local driver,guest=s.people.driver.body,s.people.guest.body
        assert(s:remove("driver") and s:remove("guest") and s:remove("silver"))
        assert(c.owner==1 and vehicles.get(c.native) and s.people.mara and next(s.cars)==nil)
        assert(not world.exists(driver) and not world.exists(guest) and world.exists(c.body))
        state.castBody=c.body
    "#);
    g.tick(&[board]); g.tick(&[idle]);
    eval(&mut g,r#"
        assert(state.fleet.players[1].mode=="walk")
        assert(not vehicles.get(state.castNative) and state.fleet.vehicles[state.castIndex]==nil)
        assert(not world.exists(state.castBody) and state.fleet.vehicles[8].kind=="helicopter")
        local before=world.count()
        local new=assert(state.castTest:car("next",{pos=vector.create(-2.7,0,-25)}))
        assert(new.index>state.castIndex)
        state.castTest:remove("next")
        assert(world.count()==before)
    "#);
    assert!(g.errors().is_empty(), "{:?}", g.errors());
}
