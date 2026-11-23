//! A real city collision and a bounded multi-car damage fixture using the baked Toyama meshes.
use app::{Buttons, Controls, Game};
use std::{fs, path::Path};
use warp::{LuaGame, Overrides};

fn city_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto")
}
fn tick(game: &mut LuaGame, control: Controls, count: usize) {
    for _ in 0..count {
        game.tick(&[control]);
    }
    assert!(game.errors().is_empty(), "{:?}", game.logs());
}
#[test]
fn a_severe_real_rear_end_collision_releases_a_painted_panel() {
    let mut game = LuaGame::load(
        &city_dir(),
        &Overrides {
            seed: Some(1948),
            players: Some(1),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    game.echo = false;
    tick(&mut game, Controls::default(), 2);
    game.luau(r#"
        local first=state.fleet.vehicles[1]
        local parked=assert(state.combat.cast:car("struck",{pos=first.pos+vector.create(.12,0,-62),yaw=.04,paint="pearl"}))
        state.struck=parked.index
    "#).unwrap();
    tick(
        &mut game,
        Controls {
            pressed: Buttons(Buttons::NORTH),
            ..Default::default()
        },
        1,
    );
    let gas = Controls {
        mv: [0., 1.],
        ..Default::default()
    };
    for _ in 0..850 {
        tick(&mut game, gas, 1);
        if game.luau("local f=state.fleet; return (f.vehicles[1].missingParts or 0)~=0 or (f.vehicles[state.struck].missingParts or 0)~=0").unwrap()=="true" {break;}
    }
    let data=game.luau("local f=state.fleet; local a,b=f.vehicles[1],f.vehicles[state.struck]; return {speed=a.speed,z=a.pos.z,front=a.partStress,rear=b.partStress,frontDamage=a.damage.total,rearDamage=b.damage.total,parts=#f.debris}").unwrap();
    assert_eq!(
        game.luau("return #world.find('vehicle-debris')>0").unwrap(),
        "true",
        "{data}"
    );
    assert_eq!(
        game.luau(
            r#"for _,id in world.find("vehicle-debris") do
        local body=world.get(id)
        if string.find(body.shape,'#part-',1,true) and not body.solid then return true end
    end; return false"#
        )
        .unwrap(),
        "true",
        "{data}"
    );
}

#[test]
fn localized_hits_remove_the_matching_wheel_and_door_and_bound_loose_parts() {
    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("theft-detach-{}", std::process::id()));
    let assets = dir.join("assets/cars");
    fs::create_dir_all(&assets).unwrap();
    for entry in fs::read_dir(city_dir().join("assets/cars")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), assets.join(entry.file_name())).unwrap();
    }
    for name in ["detachment.module.luau", "vehicles.module.luau"] {
        fs::copy(city_dir().join(name), dir.join(name)).unwrap();
    }
    fs::write(
        dir.join("game.toml"),
        "title='Loose parts test'\nid='loose-parts-test'\nmax_entities=250\n",
    )
    .unwrap();
    fs::write(dir.join("main.luau"),r#"
        local models=require("vehicles")
        local loose=require("detachment")
        local v=vector.create
        return {
            init=function()
                world.spawn("cube",{tag="city",pos=v(0,-.5,0),size=v(200,1,200)})
                vehicles.reset("city")
                models.prepareContacts()
                state.fleet={vehicles={},players={{pos=v(0,0,0)}}}
                for i=1,24 do state.fleet.vehicles[i]=models.car(v((i%6)*4,0,math.floor(i/6)*6),0,i) end
            end,
            tick=function(dt) loose.tick(state.fleet,dt) end,
            hit=function(i,p,amount)
                local car=state.fleet.vehicles[i]
                return models.hit(car,car.pos+v(0,.33,0)+p,amount)
            end,
            drive=function(i) return models.damagedControls(state.fleet.vehicles[i],{throttle=1,brake=0,steer=0}) end,
        }
    "#).unwrap();
    let mut game = LuaGame::load(&dir, &Overrides::default(), None).unwrap();
    game.echo = false;
    tick(&mut game, Controls::default(), 1);
    game.luau(
        r#"
        local c=state.fleet.vehicles[1]
        local door=assert(world.part_center(c.model..'#part-door-lf'))
        for i=1,4 do hit(1,door,.10) end
        assert(not c.missingParts)
        hit(1,door,.56)
        assert(c.missingParts==bit32.lshift(1,2))
        local hub=vector.create(.759,0,-1.262)
        hit(1,hub,1)
        assert(c.missingWheels==2 and not world.get(c.wheels[2]).visible)
        assert(world.get(c.wheels[1]).visible and drive(1).throttle<.8)
        hit(1,hub,1); hit(1,door,1)
        assert(#c.pendingParts==2)
    "#,
    )
    .unwrap();
    tick(&mut game, Controls::default(), 1);
    game.luau(
        r#"
        assert(#world.find("vehicle-debris")==2)
        for i=2,24 do
            local c=state.fleet.vehicles[i]
            hit(i,assert(world.part_center(c.model..'#part-bonnet')),1)
        end
    "#,
    )
    .unwrap();
    tick(&mut game, Controls::default(), 1);
    game.luau("assert(#world.find('vehicle-debris')<=18 and world.count()+80<=world.capacity())")
        .unwrap();
    tick(&mut game, Controls::default(), 310);
    game.luau("for _,p in state.fleet.debris do assert(p.pos.y>=0 and p.pos.y<1 and p.rest) end; assert(#state.fleet.debris==18)").unwrap();
    tick(&mut game, Controls::default(), 2200);
    game.luau("assert(#state.fleet.debris==0 and #world.find('vehicle-debris')==0 and state.fleet.vehicles[1].missingWheels==2)").unwrap();
    drop(game);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn firing_repeatedly_into_the_same_deforming_door_breaks_its_original_hinges() {
    let mut game = LuaGame::load(
        &city_dir(),
        &Overrides {
            seed: Some(1948),
            players: Some(1),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    game.echo = false;
    tick(&mut game, Controls::default(), 1);
    game.luau(r#"
        local v=vector.create
        local c,p=state.fleet.vehicles[1],state.fleet.players[1]
        p.cam.look=function() camera.set(1,{pos=c.pos+v(-4.65,1.12,-1.48),target=c.pos+v(-.78,.71,-.3),fov=40}) end
        p.cam:look()
        world.place(p.body,c.pos+v(-5.8,.85,-1.9))
        p.view=math.pi
    "#).unwrap();
    let aim = Controls {
        held: Buttons(Buttons::LEFT_TRIGGER),
        ..Default::default()
    };
    let fire = Controls {
        held: Buttons(Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER),
        pressed: Buttons(Buttons::RIGHT_TRIGGER),
        ..Default::default()
    };
    tick(&mut game, aim, 24);
    for n in 1..=19 {
        tick(&mut game, fire, 1);
        tick(&mut game, aim, 30);
        if n == 4 {
            game.luau("local c=state.fleet.vehicles[1]; assert(not c.missingParts and c.damage.dents[1].depth>.1)").unwrap();
        }
        if game
            .luau("return state.fleet.vehicles[1].missingParts==4")
            .unwrap()
            == "true"
        {
            break;
        }
    }
    let details=game.luau("local c=state.fleet.vehicles[1]; return {stress=c.partStress,dents=c.damage.dents,missing=c.missingParts}").unwrap();
    assert_eq!(game.luau("local c=state.fleet.vehicles[1]; return c.missingParts==4 and #world.find('vehicle-debris')==1").unwrap(),"true","{details}");
}
