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

fn parts_fixture(name: &str) -> (LuaGame, std::path::PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("theft-detach-{name}-{}", std::process::id()));
    let assets = dir.join("assets/cars");
    fs::create_dir_all(&assets).unwrap();
    for entry in fs::read_dir(city_dir().join("assets/cars")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), assets.join(entry.file_name())).unwrap();
    }
    for name in ["detachment.module.luau", "vehicles.module.luau", "charred.module.luau"] {
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
        local charred=require("charred")
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
            burst=function(i)
                local car=state.fleet.vehicles[i]
                charred.apply(car)
                return loose.explode(car)
            end,
            scorch=function(i) charred.apply(state.fleet.vehicles[i]) end,
            stream=function(pos)
                state.fleet.players[1].pos=pos
                models.streamDamage(state.fleet,1)
            end,
            drive=function(i) return models.damagedControls(state.fleet.vehicles[i],{throttle=1,brake=0,steer=0}) end,
        }
    "#).unwrap();
    let mut game = LuaGame::load(&dir, &Overrides::default(), None).unwrap();
    game.echo = false;
    tick(&mut game, Controls::default(), 1);
    (game, dir)
}

#[test]
fn localized_hits_remove_the_matching_wheel_and_door_and_bound_loose_parts() {
    let (mut game,dir)=parts_fixture("impacts");
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
fn a_wreck_scatters_its_original_parts_once_and_keeps_the_shared_debris_budget() {
    let (mut game, dir) = parts_fixture("explosion");
    game.luau(r#"
        local c=state.fleet.vehicles[1]
        hit(1,assert(world.part_center(c.model..'#part-door-lf')),1)
        hit(1,vector.create(.759,0,1.262),1)
    "#).unwrap();
    tick(&mut game, Controls::default(), 1);
    game.luau(r#"
        local v=vector.create
        local c=state.fleet.vehicles[1]
        state.oldModel=c.model
        for _,piece in state.fleet.debris do
            piece.vel,piece.rest,piece.settleRoll=v(0,0,0),true,0
            if piece.part==2 then state.close=piece.entity
            else
                state.far=piece.entity
                piece.pos=c.pos+v(0,.33,9)
                world.set(piece.entity,{pos=piece.pos})
            end
        end
        assert(state.close and state.far)
        -- This normal hit and the detonation occur in the same simulation tick.
        hit(1,assert(world.part_center(c.model..'#part-bonnet')),1)
        assert(#c.pendingParts==1 and not c.pendingParts[1].blast)
        c.rotation=v(.08,math.pi*.5,-.06)
        c.prevPos=c.pos-v(.10,0,0)
        assert(burst(1) and not burst(1))
        assert(c.missingParts==126 and c.missingWheels==15 and #c.pendingParts==8)
        for _,wheel in c.wheels do assert(not world.get(wheel).visible) end
    "#).unwrap();
    tick(&mut game, Controls::default(), 1);
    game.luau(r#"
        local c=state.fleet.vehicles[1]
        local byPart={}
        assert(#state.fleet.debris==10 and #world.find("vehicle-debris")==10)
        for _,piece in state.fleet.debris do
            assert(piece.source==c.body and not byPart[piece.part])
            byPart[piece.part]=piece
            if piece.entity~=state.far then
                assert(piece.vel.y>4 and not piece.rest and vector.magnitude(piece.spin)>4)
                assert(string.find(world.get(piece.entity).shape,"assets/cars/wreck",1,true)==1)
            end
        end
        assert(c.model~=state.oldModel and byPart[2].entity==state.close)
        assert(byPart[10].entity==state.far and byPart[10].rest)
        assert(not string.find(world.get(state.far).shape,"assets/cars/wreck",1,true))
        -- At a quarter turn the bonnet flies to -X, the boot to +X, relative
        -- to the incoming chassis velocity of +6 m/s along X.
        assert(byPart[1].vel.x<1 and byPart[6].vel.x>11)
        assert(not burst(1) and not c.pendingParts)
        assert(burst(2) and burst(3))
    "#).unwrap();
    tick(&mut game, Controls::default(), 1);
    game.luau(r#"
        local f=state.fleet
        assert(#f.debris==18 and #world.find('vehicle-debris')==18 and world.count()+80<=world.capacity())
        local counts,wheels={},{}
        for _,p in f.debris do
            counts[p.source]=(counts[p.source] or 0)+1
            if p.wheel then wheels[p.source]=true end
        end
        for i=2,3 do
            local id=f.vehicles[i].body
            assert(counts[id]==9 and wheels[id],"each wreck should get its share of panels and wheels")
        end
    "#).unwrap();
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

#[test]
fn scorch_preserves_the_dented_near_mesh_the_distant_shell_and_other_paint_instances() {
    let (mut game, dir) = parts_fixture("scorch-streaming");
    game.luau(r#"
        local c,twin=state.fleet.vehicles[1],state.fleet.vehicles[5]
        local original=world.get(c.body).shape
        assert(world.get(twin.body).shape==original)
        hit(1,assert(world.part_center(c.model..'#part-door-lf')),1)
        hit(1,vector.create(.759,0,-1.262),1)
        stream(c.pos)
        assert(c.dentActive and c.missingParts==4 and #c.damage.dents>0)
        scorch(1)
        assert(c.dentActive and world.get(c.body).shape=="assets/cars/wreck.gltf")
        assert(not world.get(c.wheels[2]).visible and world.get(c.wheels[1]).visible)
        assert(world.get(c.wheels[1]).shape=="assets/cars/wreck-wheel-left.gltf")
        assert(world.get(twin.body).shape==original and not twin.charred)

        local distant=c.pos+vector.create(100,0,0)
        stream(distant)
        assert(not c.dentActive and c.shellMask==4)
        local b=state.fleet.vehicles[2]
        hit(2,assert(world.part_center(b.model..'#part-bonnet')),1)
        scorch(2)
        assert(not b.dentActive and b.shellMask==2 and world.get(b.body).shape==c.model)
        stream(c.pos)
        assert(c.dentActive and b.dentActive and c.missingParts==4 and b.missingParts==2)
        assert(world.get(c.body).shape==b.model and world.get(twin.body).shape==original)
    "#).unwrap();
    drop(game);
    fs::remove_dir_all(dir).unwrap();
}
