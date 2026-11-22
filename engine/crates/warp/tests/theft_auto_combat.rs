//! Drive the real split-screen inputs through the city, its casts and its physics.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

fn city() -> LuaGame {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(
        &folder,
        &Overrides { players: Some(2), seed: Some(1948), ..Default::default() },
        None,
    ).unwrap();
    game.echo = false;
    tick(&mut game, [Controls::default(); 2], 2);
    game
}

fn check(game: &mut LuaGame, lua: &str) {
    game.luau(lua).unwrap_or_else(|err| panic!("{err}\n{lua}"));
}

fn tick(game: &mut LuaGame, inputs: [Controls; 2], count: usize) {
    for _ in 0..count { game.tick(&inputs); }
    assert!(game.errors().is_empty(), "{:?}", game.logs());
}

fn aim() -> Controls {
    Controls { held: Buttons(Buttons::LEFT_TRIGGER), ..Default::default() }
}

fn shoot() -> Controls {
    Controls {
        held: Buttons(Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER),
        pressed: Buttons(Buttons::RIGHT_TRIGGER),
        ..Default::default()
    }
}

// Ground the fixture on a sidewalk so cross traffic cannot knock down the shooter.
fn line_up(game: &mut LuaGame) {
    check(game, r#"
        local p=state.combat.cast.people.rook
        p.pos,p.route,p.arrived=vector.create(8,0,-40),nil,false
        world.place(p.body,p.pos+vector.create(0,.85,0))
        humans.set(p.actor,{pos=p.pos,pose="idle"})
        for _,seat in state.fleet.players do
            world.place(seat.body,p.pos+vector.create(-.46,.85,7))
            seat.view,seat.pitch=0,-.055
        end
    "#);
}

#[test]
fn free_roam_rivals_take_visible_hits_from_each_seat_and_return() {
    let mut game = city();
    check(&mut game, r#"
        local s=state.combat
        assert(s.enemies.rook.marker and s.enemies.vex.marker)
        assert(vector.magnitude(s.cast.people.rook.pos-state.fleet.players[1].pos)<25)
        assert(s.cast.people.rook.route and s.cast.people.vex.route)
    "#);
    line_up(&mut game);
    check(&mut game, r#"
        local target=state.combat.cast.people.rook.pos
        state.testWall=world.spawn("cube",{tag="city",solid=true,pos=target+vector.create(0,1.2,3),size=vector.create(2,3,.3)})
    "#);
    tick(&mut game, [aim(), aim()], 54);
    tick(&mut game, [Controls::default(), shoot()], 1);
    check(&mut game, r#"
        local s=state.combat
        assert(s.players[2].aiming and not s.players[2].onTarget and s.players[2].shotFlash>0)
        assert(s.players[1].shotFlash==0 and s.enemies.rook.health==2)
        assert(#world.find("combat-drop")==0 and #world.find("combat-shard")==0)
        world.remove(state.testWall)
    "#);
    tick(&mut game, [aim(), aim()], 25);
    tick(&mut game, [aim(), shoot()], 1);
    check(&mut game, r#"
        local s=state.combat
        assert(s.players[2].onTarget and s.players[2].hits==1 and s.enemies.rook.health==1)
        assert(not s.cast.people.rook.impact and s.enemies.rook.marker)
        assert(#world.find("combat-drop")>0 and #world.find("combat-splat")>0)
        assert(#world.find("combat-shard")==0)
    "#);
    tick(&mut game, [aim(), shoot()], 1);
    check(&mut game, "assert(state.combat.enemies.rook.health==1)");
    tick(&mut game, [shoot(), aim()], 1);
    check(&mut game, r#"
        local s=state.combat
        assert(s.players[1].defeats==1 and s.players[2].defeats==0)
        assert(s.cast.people.rook.impact.stage=="down" and not s.enemies.rook.marker)
        assert(s.enemies.vex.health==2 and s.enemies.vex.marker)
        assert(#world.find("combat-shard")>0 and #world.find("combat-splat")>0)
    "#);
    tick(&mut game, [Controls::default(); 2], 80);
    check(&mut game, "assert(not state.combat.cast.people.rook)");
    tick(&mut game, [Controls::default(); 2], 650);
    check(&mut game, r#"
        local s=state.combat
        assert(s.cast.people.rook and s.enemies.rook.health==2 and s.enemies.rook.marker)
        assert(s.players[1].hits==1 and s.players[2].hits==1)
        assert(s.players[1].prop and s.players[2].prop and s.players[1].shotFlash==0)
        assert(#world.find("combat-drop")==0 and #world.find("combat-shard")==0 and #world.find("combat-splat")==0)
    "#);
}

#[test]
fn a_contract_claims_only_its_owners_pistol_and_only_its_named_target() {
    let mut game = city();
    let story = r#"return {
        id="shared",title="Shared sidewalk",
        cast={{kind="person",key="actor",name="Cast",pos=vector.create(8,0,-40)}},
        steps={{op="contract",person="actor",range=32,text="Tag the marked actor."}},
    }"#;
    game.apply_story(story).unwrap();
    line_up(&mut game);
    tick(&mut game, [aim(), aim()], 54);
    check(&mut game, r#"
        assert(state.story.owner==1 and state.story.onTarget)
        assert(not state.combat.players[1].prop and state.combat.players[2].onTarget)
    "#);
    tick(&mut game, [shoot(), shoot()], 1);
    check(&mut game, r#"
        local s=state.combat
        assert(state.story.status=="complete" and state.storyCast:wasHit("actor"))
        assert(s.enemies.rook.health==1 and s.players[1].hits==0 and s.players[2].hits==1)
        assert(s.players[1].shotFlash==0 and s.players[2].shotFlash>0)
    "#);
    // On the very next tick ownership returns to normal, with the same cooldown.
    tick(&mut game, [shoot(), aim()], 1);
    check(&mut game, "assert(state.combat.enemies.rook.health==1 and state.combat.players[1].aiming)");
    tick(&mut game, [aim(), aim()], 25);
    tick(&mut game, [shoot(), aim()], 1);
    check(&mut game, "assert(state.combat.players[1].defeats==1)");
}

#[test]
fn an_unaimed_click_fires_on_foot_but_vehicle_throttle_never_fires() {
    let mut game = city();
    let click = Controls {
        pressed: Buttons(Buttons::RIGHT_TRIGGER),
        held: Buttons(Buttons::RIGHT_TRIGGER),
        ..Default::default()
    };
    line_up(&mut game);
    tick(&mut game, [click, Controls::default()], 1);
    check(&mut game, r#"
        assert(state.combat.players[1].shotFlash>0 and not state.combat.players[1].aiming)
        assert(state.combat.enemies.rook.health==1)
        world.place(state.fleet.players[1].body, state.fleet.vehicles[1].pos+vector.create(1.6,.85,0))
    "#);
    tick(&mut game, [Controls::default(); 2], 24);
    let enter = Controls { pressed: Buttons(Buttons::NORTH), ..Default::default() };
    tick(&mut game, [enter, Controls::default()], 1);
    tick(&mut game, [shoot(), aim()], 1);
    check(&mut game, r#"
        assert(state.fleet.players[1].mode=="drive")
        assert(not state.combat.players[1].aiming and not state.combat.players[1].prop)
        assert(state.combat.players[1].shotFlash==0)
        assert(state.combat.players[2].prop)
        player_left(players.get(2))
    "#);
    tick(&mut game, [Controls::default(); 2], 1);
    check(&mut game, "assert(not state.combat.players[2])");
}
