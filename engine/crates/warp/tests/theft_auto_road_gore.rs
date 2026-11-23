//! Real city driver input, fitted residents, retained rigs and the shared Growing Guns effects.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

fn city() -> LuaGame {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(
        &dir,
        &Overrides {
            seed: Some(1948),
            players: Some(1),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    game.echo = false;
    game
}

fn advance(game: &mut LuaGame, input: Controls, count: usize) {
    for _ in 0..count {
        game.tick(&[input]);
    }
    assert!(game.errors().is_empty(), "{:?}", game.logs());
}

fn board(game: &mut LuaGame) {
    advance(
        game,
        Controls {
            pressed: Buttons(Buttons::NORTH),
            ..Default::default()
        },
        1,
    );
    game.luau("assert(state.fleet.players[1].vehicle==1)")
        .unwrap();
}

fn gas() -> Controls {
    Controls {
        mv: [0., 1.],
        ..Default::default()
    }
}

#[test]
fn bumper_severity_drives_blood_and_leaves_a_targetable_body_only_for_a_hard_hit() {
    for (z, lethal) in [(-0.12, false), (-25.0, true)] {
        let mut game = city();
        game.luau(&format!(
            r#"
            local p=state.population.street[1]
            p.pos,p.yaw,p.action=vector.create(3.85,0,{z}),math.pi/2,"idle"
            p.path,p.delta,p.previousFeet=nil,nil,nil
            p.nextPos,p.nextYaw,p.pose=p.pos,p.yaw,"idle"
            state.population.selectionIn=0
        "#
        ))
        .unwrap();
        advance(&mut game, Controls::default(), 169);
        board(&mut game);
        for _ in 0..330 {
            advance(&mut game, gas(), 1);
            if game.luau("local p=state.population.street[1]; return p.dead or (p.impact and p.impact.stage=='down')").unwrap()=="true" {break;}
        }
        let details=game.luau("local p=state.population.street[1]; return {speed=state.fleet.vehicles[1].speed,dead=p.dead,corpse=p.corpse,effects=state.combat.effects.serial,impact=p.impact}").unwrap();
        assert_eq!(
            game.luau("return #world.find('combat-drop')>0 and #world.find('combat-splat')>0")
                .unwrap(),
            "true",
            "{details}"
        );
        assert_eq!(game.luau("local p=state.population.street[1]; return p.dead==true and state.population.corpses[p.corpse]~=nil").unwrap(),lethal.to_string(),"{details}");
        if !lethal {
            assert_eq!(
                game.luau("return #world.find('combat-shard')").unwrap(),
                "0"
            );
        }
    }
}

fn fallen_cast(game: &mut LuaGame, side: f32, distance: f32) {
    advance(game, Controls::default(), 5);
    game.luau(&format!(
        r#"
        local c=state.fleet.vehicles[1]
        local p=state.combat.cast.people.rook
        p.pos,p.route,p.arrived=c.pos+vector.create({side},0,-{distance}),nil,false
        world.place(p.body,p.pos+vector.create(0,.85,0))
        humans.set(p.actor,{{pos=p.pos,yaw=0,pose="idle"}})
        state.roadActor=p.actor
        assert(state.combat.cast:kill("rook",vector.create(0,0,-1),false))
        state.combat.enemies.rook.health,state.combat.enemies.rook.respawnIn=0,90
    "#
    ))
    .unwrap();
    advance(game, Controls::default(), 170);
    game.luau("assert(state.population.corpses[state.roadActor]); state.crossings=0")
        .unwrap();
}

fn record_crossings(game: &mut LuaGame) -> bool {
    game.luau(
        r#"
        for _,hit in state.population.roadHits do
            if hit.corpse==state.roadActor and hit.vehicle==1 then
                assert(hit.kind=="wheel")
                state.crossings+=1
                state.roadSpeed=hit.speed
                return true
            end
        end
        return false
    "#,
    )
    .unwrap()
        == "true"
}

#[test]
fn the_front_and_rear_wheels_register_one_crossing_of_a_settled_body() {
    for (distance, fast) in [(7., false), (24., true)] {
        let mut game = city();
        fallen_cast(&mut game, 0.76, distance);
        board(&mut game);
        for _ in 0..280 {
            advance(&mut game, gas(), 1);
            if record_crossings(&mut game) {
                break;
            }
        }
        let contact=game.luau("return {speed=state.roadSpeed,body=humans.joint(state.roadActor,'hips'),shards=#world.find('combat-shard')}").unwrap();
        assert_eq!(
            game.luau("return state.crossings==1 and #world.find('combat-drop')>0")
                .unwrap(),
            "true",
            "{contact}"
        );
        assert_eq!(game.luau("return state.population.corpses[state.roadActor]==nil and #world.find('combat-shard')>=7").unwrap(),fast.to_string(),"{contact}");
        for _ in 0..40 {
            advance(&mut game, gas(), 1);
            record_crossings(&mut game);
        }
        assert_eq!(
            game.luau("return state.crossings==1").unwrap(),
            "true",
            "front/rear tire double count: {contact}"
        );
    }
}

#[test]
fn a_slow_wheel_or_a_fast_car_in_the_adjacent_track_leaves_the_body_intact() {
    for (side, slow) in [(0.76, true), (2.7, false)] {
        let mut game = city();
        fallen_cast(&mut game, side, 7.);
        board(&mut game);
        for _ in 0..650 {
            let input =
                if slow && game.luau("return state.fleet.vehicles[1].speed>2").unwrap() == "true" {
                    Controls::default()
                } else {
                    gas()
                };
            advance(&mut game, input, 1);
            assert!(!record_crossings(&mut game));
            if game.luau("return state.fleet.vehicles[1].pos.z<state.population.corpses[state.roadActor].pos.z-4").unwrap()=="true" {break;}
        }
        game.luau("local body=assert(state.population.corpses[state.roadActor]); assert(state.fleet.vehicles[1].pos.z<body.pos.z-3 and state.crossings==0)").unwrap();
    }
}
