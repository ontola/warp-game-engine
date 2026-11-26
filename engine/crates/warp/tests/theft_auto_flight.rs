//! The actual two-seat flight path, including the camera and the rendered mount.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

fn tick(game: &mut LuaGame, count: usize, pilot: Controls, gunner: Controls) {
    for _ in 0..count {
        game.tick(&[pilot, gunner]);
    }
    assert!(game.errors().is_empty(), "{:?}", game.logs());
}

fn crew() -> LuaGame {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(&dir, &Overrides {
        players: Some(2), seed: Some(1948), ..Default::default()
    }, None).unwrap();
    game.echo = false;
    game.luau(r#"
        local h=state.fleet.vehicles[8]
        for id,p in state.fleet.players do
            world.place(p.body,h.pos+vector.create(if id==1 then 3.2 else -3.2,.85,-1.3))
        end
    "#).unwrap();
    let idle = Controls::default();
    let board = Controls { pressed: Buttons(Buttons::NORTH), ..idle };
    tick(&mut game, 2, idle, idle);
    tick(&mut game, 1, board, board);
    game.luau(r#"
        local h=state.fleet.vehicles[8]
        assert(h.owner==1 and h.gunner==2)
        assert(world.get(h.body).size.z>8 and world.get(h.tail).pos.z>7)
        assert(vector.magnitude(world.get(h.gunnerSeat).world_pos-world.get(h.pilotSeat).world_pos)>1.7)
        assert(world.get(h.searchlight).pos.z< -4.8)
    "#).unwrap();
    game
}

#[test]
fn collective_yaw_and_cyclic_control_separate_axes_and_preserve_inertia() {
    let mut game = crew();
    let idle = Controls::default();
    let lift = Controls { mv: [0., 0.85], ..idle };
    tick(&mut game, 90, lift, idle);
    game.luau(r#"
        local h=state.fleet.vehicles[8]
        assert(h.pos.y>7 and math.abs(h.vel.x)+math.abs(h.vel.z)<.01)
        state.takeoff=h.pos
    "#).unwrap();
    let yaw = Controls { mv: [0.7, 0.], ..idle };
    tick(&mut game, 35, yaw, idle);
    game.luau(r#"
        local h=state.fleet.vehicles[8]
        assert(h.yaw<-.32 and math.abs(h.roll)+math.abs(h.pitch)<.001)
        assert(vector.magnitude(vector.create(h.vel.x,0,h.vel.z))<.02)
    "#).unwrap();
    let diagonal = Controls { aim: [0.65, 0.8], ..idle };
    tick(&mut game, 55, diagonal, idle);
    game.luau(r#"
        local h=state.fleet.vehicles[8]
        local right=vector.create(math.cos(h.yaw),0,-math.sin(h.yaw))
        local forward=vector.create(-math.sin(h.yaw),0,-math.cos(h.yaw))
        assert(h.roll<-.30 and h.pitch<-.29)
        assert(vector.dot(h.vel,right)>2 and vector.dot(h.vel,forward)>2)
        state.flightMomentum=h.vel
    "#).unwrap();
    tick(&mut game, 1, idle, idle);
    game.luau("assert(vector.dot(state.fleet.vehicles[8].vel,state.flightMomentum)>10)").unwrap();
    tick(&mut game, 75, idle, idle);
    game.luau("local h=state.fleet.vehicles[8]; assert(math.abs(h.roll)+math.abs(h.pitch)<.01)").unwrap();
    let keyboard = Controls { held: Buttons(Buttons::DOWN | Buttons::LEFT), ..idle };
    tick(&mut game, 44, keyboard, idle);
    game.luau("local h=state.fleet.vehicles[8]; assert(h.pitch>.43 and h.roll>.5)").unwrap();
    let lower = Controls { mv: [0., -1.], ..idle };
    tick(&mut game, 40, lower, idle);
    game.luau("assert(state.fleet.vehicles[8].vel.y< -4)").unwrap();
}

#[test]
fn gunner_holds_world_direction_when_pilot_turns_and_zooms_until_a_stop_binds() {
    let mut game = crew();
    let idle = Controls::default();
    let rise = Controls { mv: [0.,1.], ..idle };
    tick(&mut game, 80, rise, idle);
    let aim = Controls { look: [-0.25,-0.20], ..idle };
    tick(&mut game, 1, idle, aim);
    game.luau("state.fixedSight=state.fleet.players[2].mountDir").unwrap();
    let manoeuvre = Controls { mv:[0.42,0.], aim:[0.4,0.55], ..idle };
    let zoom = Controls { held: Buttons(Buttons::LEFT_TRIGGER), ..idle };
    tick(&mut game, 45, manoeuvre, zoom);
    game.luau(r#"
        local p=state.fleet.players[2]
        local h=state.fleet.vehicles[8]
        local cam=camera.get(2)
        assert(h.yaw<-.26 and h.pitch<-.23 and h.roll<-.20)
        assert(cam.fov<24 and camera.get(1).fov>60)
        assert(vector.dot(state.fixedSight,p.mountDir)>.9999)
        assert(vector.dot(vector.normalize(cam.target-cam.pos),p.mountDir)>.99999)
        assert(vector.dot(vector.normalize(world.get(h.gunTip).world_pos-world.get(h.turret).world_pos),p.mountDir)>.99999)
    "#).unwrap();
    let spin = Controls { mv:[1.,0.], ..idle };
    tick(&mut game, 130, spin, idle);
    game.luau(r#"
        local p=state.fleet.players[2]
        assert(math.abs(p.turretYaw)>1.93)
        assert(vector.dot(state.fixedSight,p.mountDir)<.8)
        assert(camera.get(2).fov>50.9)
    "#).unwrap();
}

#[test]
fn gunner_hits_a_distant_crowd_proxy_only_with_a_clear_sightline() {
    let mut game = crew();
    let idle = Controls::default();
    let zoom = Controls { held: Buttons(Buttons::LEFT_TRIGGER), ..idle };
    let shoot = Controls { pressed: Buttons(Buttons::RIGHT_TRIGGER), ..zoom };
    game.luau(r#"
        local h=state.fleet.vehicles[8]
        world.place(h.body,vector.create(204,38,148))
        h.grounded=false
    "#).unwrap();
    tick(&mut game, 1, idle, zoom);
    game.luau(r#"
        local v=vector.create
        local p,h=state.fleet.players[2],state.fleet.vehicles[8]
        local feet=v(174,0,-145)
        local delta=feet+v(0,1,0)-world.get(h.turret).world_pos
        local dir=vector.normalize(delta)
        p.sightYaw,p.sightPitch=math.atan2(-dir.x,-dir.z),math.asin(dir.y)
        local pop=state.population
        for id,c in pop.street do
            if c.active and not pop.pedestrians.active[id] then
                c.path,c.delta,c.action,c.pos,c.night=nil,nil,"idle",feet,true
                state.farResident=id
                break
            end
        end
        assert(state.farResident)
        pop.selectionIn=0
        state.cover=world.spawn("cube",{pos=world.get(h.turret).world_pos+delta*.5,
            size=v(7,7,1),solid=true,tag="city"})
    "#).unwrap();
    tick(&mut game, 1, idle, zoom);
    game.luau(r#"
        local p=state.fleet.players[2]
        local c=state.population.street[state.farResident]
        assert(not state.population.pedestrians.active[state.farResident])
        local to=c.pos+vector.create(0,1,0)-p.mountPos
        assert(vector.magnitude(to)>280 and vector.dot(vector.normalize(to),p.mountDir)>.99999)
    "#).unwrap();
    tick(&mut game, 1, idle, shoot);
    game.luau(r#"
        assert(state.combat.players[2].turretShots==1 and not state.population.street[state.farResident].dead)
        world.remove(state.cover)
    "#).unwrap();
    tick(&mut game, 12, idle, zoom);
    tick(&mut game, 1, idle, shoot);
    game.luau(r#"
        local gun=state.combat.players[2]
        local c=state.population.street[state.farResident]
        assert(c.dead and c.shotUntil and not c.active,"the distant walker survived direct contact")
        assert(gun.hits==1 and gun.defeats==1 and gun.hitFlash>0 and vector.magnitude(c.pos-gun.hitPoint)<2)
        assert(state.combat.players[1].hits==0 and gun.turretShots==2)
    "#).unwrap();
}

#[test]
fn west_field_landing_still_completes_the_relay_with_collective_and_cyclic() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(&dir, &Overrides {
        players: Some(2), seed: Some(1948), ..Default::default()
    }, None).unwrap();
    game.echo = false;
    game.luau(r#"
        local f=state.fleet
        state.job.leg=2
        world.place(f.players[2].body,f.vehicles[9].pos+vector.create(-1.12,.85,3.0))
    "#).unwrap();
    let idle = Controls::default();
    tick(&mut game, 2, idle, idle);
    tick(&mut game, 1, idle, Controls { pressed: Buttons(Buttons::NORTH), ..idle });
    tick(&mut game, 55, idle, Controls { held: Buttons(Buttons::SOUTH), ..idle });
    let replay: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.join("scripts/crew-relay.json")).unwrap()
    ).unwrap();
    let reverse = replay["steps"].as_array().unwrap().iter()
        .find(|step| step["aim"].is_array()).unwrap();
    tick(&mut game, reverse["ticks"].as_u64().unwrap() as usize, idle,
        Controls { aim: [0., reverse["aim"][1].as_f64().unwrap() as f32], ..idle });
    tick(&mut game, 100, idle, idle);
    tick(&mut game, 125, idle, Controls { held: Buttons(Buttons::RIGHT_SHOULDER), ..idle });
    game.luau(r#"
        local h=state.fleet.vehicles[9]
        assert(h.grounded and h.speed<3)
        assert(state.job.leg==3 and state.job.lastDriver==2,string.format("West Field landing, mode %s at %.2f %.2f %.2f, speed %.2f, leg %d",
            state.fleet.players[2].mode,h.pos.x,h.pos.y,h.pos.z,h.speed,state.job.leg))
    "#).unwrap();
}

#[test]
fn apache_optics_show_the_camera_bearing_and_live_readouts_inside_each_viewport() {
    use warp::api::HudItem;
    let dir=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut g=LuaGame::load(&dir,&Overrides{players:Some(2),seed:Some(1948),..Default::default()},None).unwrap();
    let module=std::fs::read_to_string(dir.join("apache_hud.module.luau")).unwrap();
    g.luau(&format!("state.opticsFixture=(function()\n{module}\nend)()")).unwrap();
    for (w,h) in [(800,900),(640,720),(640,360)] {
        for (role,degrees) in [("gunner",359),("pilot",2)] {
            g.core().hud.clear();
            g.luau(&format!(r#"
                local v=vector.create
                local d=math.rad({degrees})
                camera.set(2,{{pos=v(0,0,0),target=v(math.sin(d),0,-math.cos(d))}})
                local size=math.max(.49,math.min({h}/610,{w}/810,1.7))
                local seat={{flightRole="{role}",turretYaw=.8,mountDir=v(0,0,-1)}}
                local car={{vel=v(3,2,4),pos=v(98,43,-12)}}
                state.opticsFixture.draw(seat,car,2,{w},0,{w},{h},size,{w}*1.60,90*size)
            "#)).unwrap();
            let core=g.core();
            let (x,y,cx)=(w as f32, h as f32, 1.5*w as f32);
            let mut heading=false;
            let mut velocity=false;
            let mut altitude=false;
            for item in &core.hud {
                match item {
                    HudItem::Box { rect:[left,top,width,height], .. } => {
                        assert!(*left>=x && *top>=0. && left+width<=2.*x && top+height<=y);
                        assert!(!(*left<=cx && left+width>cx && *top<=y*0.5 && top+height>y*0.5),
                            "the sight should leave its center open");
                    }
                    HudItem::Circle { center: [left, top], radius, .. } => {
                        assert!(left-radius>=x && left+radius<=2.*x && top-radius>=0. && top+radius<=y);
                        assert!((left-cx).hypot(top-y*0.5)>*radius);
                    }
                    HudItem::Text {text,pos:[left,top],..} => {
                        assert!(*left>=x && *left<2.*x && *top>=0. && *top<y);
                        if text==&format!("{degrees:03}") && (*left-cx).abs()<25. {
                            assert!(*top>=90.*(h as f32/610.).min(w as f32/810.).max(0.49));
                            heading=true;
                        }
                        velocity|=text=="18 KM/H";
                        altitude|=text=="43 M";
                    }
                }
            }
            assert!(heading,"missing or incorrect camera bearing for {role}");
            assert_eq!((velocity,altitude),(role=="gunner",role=="gunner"));
        }
    }
}


#[test]
fn two_local_pads_board_the_apache_and_its_light_reaches_ahead_at_altitude() {
    use app::{seat_controls, Device};
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut g = LuaGame::load(&dir, &Overrides { players: Some(2), seed: Some(1948), ..Default::default() }, None).unwrap();
    let idle = Controls::default();
    let tick = |g: &mut LuaGame, a, b| {
        let inputs = seat_controls(g, &[(Device::Keyboard, idle), (Device::Pad(9), a), (Device::Pad(5), b)]);
        assert_eq!(inputs, [a, b]);
        g.tick(&inputs);
        assert!(g.errors().is_empty(), "{:?}", g.errors());
    };
    g.luau(r#"
        local h=state.fleet.vehicles[8]
        for i,p in state.fleet.players do world.place(p.body,h.pos+vector.create(if i==1 then 3.2 else -3.2,.85,0)) end
        state.clock.hour=12
    "#).unwrap();
    // Both routed pads can resume the ordinary start menu and then board independently.
    let resume = Controls { pressed: Buttons(Buttons::SOUTH), ..idle };
    for (a, b) in [(resume, idle), (idle, resume)] {
        g.open_menu();
        tick(&mut g, a, b);
        assert!(!g.menu_open());
    }
    tick(&mut g, idle, idle);
    let board = Controls { pressed: Buttons(Buttons::NORTH), ..idle };
    tick(&mut g, board, board);
    g.luau(r#"
        local h=state.fleet.vehicles[8]
        assert(h.owner==1 and h.gunner==2)
        h.pos+=vector.create(0,70,0)
        world.place(h.body,h.pos+vector.create(0,1.7,0),h.yaw)
        h.grounded=false
    "#).unwrap();
    let fly = Controls { aim: [0.3, 0.9], ..idle };
    for _ in 0..30 { tick(&mut g, fly, idle); }
    let core = g.core();
    assert_eq!(core.spots.len(), 1);
    let lamp = &core.spots[0];
    let distance_to_road = -lamp.pos.y / lamp.dir.y;
    assert!(lamp.pos.y > 60.0 && distance_to_road < 0.8 * lamp.range);
    assert!((lamp.dir.x.hypot(lamp.dir.z) * distance_to_road) > 40.0);
}
