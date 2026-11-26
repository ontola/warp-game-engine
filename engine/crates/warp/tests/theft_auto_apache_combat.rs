//! Arcade pilot and gunner regressions using the shipped city and controller inputs.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

#[test]
fn both_apache_stations_can_fire_and_switch_pilots_independently() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut g = LuaGame::load(&dir, &Overrides { players: Some(3), seed: Some(1948), ..Default::default() }, None).unwrap();
    g.echo = false;
    let idle = Controls::default();
    let board = Controls { pressed: Buttons(Buttons::NORTH), ..idle };
    let rise = Controls { held: Buttons(Buttons::SOUTH), ..idle };
    let fire = Controls { held: Buttons(Buttons::RIGHT_TRIGGER), ..idle };
    let step = |g: &mut LuaGame, count, controls: [Controls; 3]| {
        for _ in 0..count { g.tick(&controls); }
        assert!(g.errors().is_empty(), "{:?}", g.logs());
    };
    g.luau(r#"
        local v=vector.create
        local f=state.fleet
        local h=f.vehicles[8]
        assert(h.gunTip and f.vehicles[9].gunTip and #h.rocketMounts==2)
        for i,p in f.players do
            world.place(p.body,h.pos+v(if i==1 then 2.7 else -2.7,.85,if i==3 then 1.1 else 0))
        end
    "#).unwrap();
    step(&mut g, 2, [idle;3]);
    step(&mut g, 1, [board;3]);
    g.luau(r#"
        local f=state.fleet
        local h=f.vehicles[8]
        assert(h.owner==1 and h.gunner==2 and f.players[3].mode=="walk")
        assert(f.players[1].flightRole=="pilot" and f.players[2].flightRole=="gunner")
        assert(f.players[1].seatedPos~=f.players[2].seatedPos)
    "#).unwrap();
    step(&mut g, 1, [idle, board, idle]);
    g.luau("assert(state.fleet.vehicles[8].owner==1 and not state.fleet.vehicles[8].gunner and state.fleet.players[2].mode=='walk')").unwrap();
    step(&mut g, 1, [idle, board, idle]);
    step(&mut g, 68, [rise, idle, idle]);
    step(&mut g, 1, [idle, Controls { look:[0.72,-0.20], ..idle }, idle]);
    step(&mut g, 1, [idle, board, board]);
    g.luau(r#"
        local f=state.fleet
        local a,b=f.players[1],f.players[2]
        assert(a.pos.y>4 and f.vehicles[8].gunner==2 and not f.players[3].vehicle)
        assert(vector.dot(a.mountDir,b.mountDir)<.92,"turret followed the pilot's rockets")
        local sight=camera.get(2)
        assert(vector.dot(vector.normalize(sight.target-sight.pos),b.mountDir)>.999)
        assert(vector.magnitude(camera.get(1).pos-sight.pos)>4)
        assert(not state.combat.players[1].rockets and not state.combat.players[2].turretShots)
        local id=next(state.combat.crates)
        local target=b.mountPos+b.mountDir*13
        state.turretCrate=id
        state.combat.crates[id].pos=target
        world.place(id,target)
    "#).unwrap();
    step(&mut g, 1, [fire, fire, idle]);
    g.luau(r#"
        local c=state.combat
        assert(#c.projectiles==1 and c.projectiles[1].owner==1)
        assert(c.players[1].rockets==1 and not c.players[1].turretShots)
        assert(c.players[2].turretShots==1 and not c.players[2].rockets)
        assert(world.get(c.players[2].flash).hide_views==2^players.get(2).view,"muzzle flash occluded own optic")
        assert(c.crates[state.turretCrate].respawnAt and c.players[2].hitFlash>0,"gunner never hit the target")
    "#).unwrap();
    step(&mut g, 3, [fire, fire, idle]);
    g.luau("assert(state.combat.players[1].rockets==1 and state.combat.players[2].turretShots==1)").unwrap();
    g.luau("player_left(players.get(1))").unwrap();
    step(&mut g, 1, [idle;3]);
    g.luau(r#"
        local f=state.fleet
        assert(f.vehicles[8].owner==2 and not f.vehicles[8].gunner)
        assert(f.players[2].flightRole=="pilot" and not f.players[1])
        for _,shot in state.combat.projectiles do assert(shot.owner~=1) end
    "#).unwrap();
    step(&mut g, 55, [idle, rise, idle]);
    step(&mut g, 1, [idle, fire, idle]);
    g.luau("assert(state.combat.players[2].rockets==1 and state.combat.players[2].turretShots==1)").unwrap();
}

#[test]
fn apache_cannon_uses_one_compact_growing_guns_blast_at_first_contact() {
    let dir=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut g=LuaGame::load(&dir,&Overrides{players:Some(2),seed:Some(1948),..Default::default()},None).unwrap();
    g.echo=false;
    let idle=Controls::default();
    let board=Controls{pressed:Buttons(Buttons::NORTH),..idle};
    let rise=Controls{held:Buttons(Buttons::SOUTH),..idle};
    let fire=Controls{pressed:Buttons(Buttons::RIGHT_TRIGGER),..idle};
    let ticks=|g:&mut LuaGame,n,a,b| {
        for _ in 0..n {g.tick(&[a,b]);}
        assert!(g.errors().is_empty(),"{:?}",g.logs());
    };
    g.luau(r#"
        local h=state.fleet.vehicles[8]
        for id,p in state.fleet.players do world.place(p.body,h.pos+vector.create(if id==1 then 2.6 else -2.6,.85,0)) end
    "#).unwrap();
    ticks(&mut g,2,idle,idle);
    ticks(&mut g,1,board,board);
    ticks(&mut g,100,rise,idle);
    ticks(&mut g,55,idle,idle);
    // Look up over the harbour. A miss must never burst at maximum range.
    ticks(&mut g,1,idle,Controls{look:[1.57,1.0],..idle});
    ticks(&mut g,1,idle,fire);
    g.luau("assert(state.combat.players[2].turretShots==1)").unwrap();
    let count=|g:&LuaGame|g.core().world.explosions.as_ref().map_or(0,|fx|fx.explosion_stats().requested);
    assert_eq!(count(&g),0,"an empty sky shot created a blast");
    ticks(&mut g,1,idle,Controls{look:[-1.57,-0.88],..idle});
    ticks(&mut g,12,idle,idle);
    // A flat raised quay gives the shell a known first contact. Two exposed
    // civilians, a neighbour behind a thin wall and a distant one flank it.
    g.luau(r#"
        local p=state.fleet.players[2]
        local v=vector.create
        local at=p.mountPos+p.mountDir*12
        state.shellPoint=at
        world.spawn("cube",{pos=at-v(0,.15,0),size=v(13,.30,13),solid=true,tag="city"})
        world.spawn("cube",{pos=at+v(-.64,1.1,0),size=v(.12,2.2,2.1),solid=true,tag="city"})
        state.testResident=function(id,pos)
            local c=state.population.street[id]
            c.path,c.action,c.pos,c.nextPos,c.night,c.panic=nil,"idle",pos,nil,true,nil
            state.population.selectionIn=0
        end
        for i,offset in {1.05,2.15,-1.3,5.3} do state.testResident(i,at+v(offset,0,0)) end
    "#).unwrap();
    ticks(&mut g,1,idle,fire);
    g.luau(r#"
        local c=state.population.street
        assert(c[1].blastWounds and c[2].blastWounds and c[1].blastWounds>c[2].blastWounds,"missing splash falloff")
        assert(not c[3].blastWounds and not c[3].dead and not c[4].blastWounds and not c[4].dead,"splash escaped cover or radius")
        assert(state.combat.players[1].hits==0 and state.combat.players[2].hits==2)
    "#).unwrap();
    let at: [f32;3]=serde_json::from_str(&g.luau("return state.shellPoint").unwrap()).unwrap();
    {
        let c=g.core();
        let fx=c.world.explosions.as_ref().unwrap();
        let stats=fx.explosion_stats();
        assert_eq!((stats.requested,stats.full),(1,1));
        assert!(!fx.is_empty());
        let lights=fx.lights(c.tick as f32/c.tick_rate);
        assert!(lights.iter().any(|p|p.position.distance(glam::Vec3::from(at))<0.035 && p.range_m<2.2));
        assert!(lights.iter().all(|p|p.range_m<2.2),"large grenade flash on a small cannon shell");
    }
    ticks(&mut g,12,idle,idle);
    // A close direct contact is credited exactly once, ahead of the quay.
    g.luau(r#"
        local p=state.fleet.players[2]
        state.testResident(5,p.mountPos+p.mountDir*4.6-vector.create(0,1,0))
        state.oldHits=state.combat.players[2].hits
    "#).unwrap();
    ticks(&mut g,1,idle,fire);
    g.luau(r#"
        assert(state.population.street[5].dead)
        assert(state.combat.players[2].hits==state.oldHits+1 and state.combat.players[2].defeats==1)
        assert(state.combat.players[1].defeats==0 and #state.combat.projectiles==0)
    "#).unwrap();
    assert_eq!(count(&g),2);
}
