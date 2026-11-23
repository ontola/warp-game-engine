//! Drive the real split-screen inputs through the city, its casts and its physics.
use app::{Buttons, Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

fn game_folder() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto")
}

fn city() -> LuaGame { load_city(&game_folder()) }

fn load_city(folder: &Path) -> LuaGame {
    let mut game = LuaGame::load(
        folder,
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
        local pop=state.population
        -- Clear this small sightline when a fixture needs an unobstructed target.
        for _,person in pop.street do
            if math.abs(person.pos.x-8)<3 and math.abs(person.pos.z+36)<10 then
                person.path,person.action=nil,"idle"
                person.pos+=vector.create(4,0,0)
                person.nextPos=person.pos
            end
        end
        pop.selectionIn=0
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
    tick(&mut game, [Controls::default(); 2], 730);
    check(&mut game, r#"
        local s,pop=state.combat,state.population
        local p=s.cast.people.rook
        assert(s.enemies.rook.health==0 and p.dead and p.corpse and pop.corpses[p.corpse])
        assert(humans.joint(p.corpse,"hips") and not s.enemies.rook.marker)
        -- Age the corpse while a second seat still observes it.
        pop.elapsed+=50
        s.enemies.rook.respawnIn=0
        world.place(state.fleet.players[1].body,vector.create(80,.85,78))
    "#);
    tick(&mut game, [Controls::default(); 2], 3);
    check(&mut game, r#"
        assert(state.combat.cast.people.rook.corpse)
        world.place(state.fleet.players[2].body,vector.create(81,.85,78))
    "#);
    tick(&mut game, [Controls::default(); 2], 4);
    check(&mut game, r#"
        assert(not state.combat.cast.people.rook)
        world.place(state.fleet.players[1].body,vector.create(8,.85,-36))
    "#);
    tick(&mut game, [Controls::default(); 2], 2);
    check(&mut game, r#"
        local s=state.combat
        assert(s.cast.people.rook and s.enemies.rook.health==2 and s.enemies.rook.marker)
        assert(s.players[1].hits==1 and s.players[2].hits==1)
        assert(s.players[1].prop and s.players[2].prop and s.players[1].shotFlash==0)
        assert(#world.find("combat-drop")==0 and #world.find("combat-shard")==0 and #world.find("combat-splat")==0)
    "#);
}

#[test]
fn walking_shooters_keep_both_hands_on_their_own_weapon_through_aim_and_recoil() {
    let mut game = city();
    line_up(&mut game);
    check(&mut game, r#"
        state.fleet.players[2].weapon="launcher"
        state.fleet.players[1].pitch=.22
        state.fleet.players[2].pitch=-.21
        state.checkHold=function(id,spacing)
            local p=state.combat.players[id]
            local actor=state.population.avatars[id]
            local wrist=humans.joint(actor,"hand_r")
            local left=humans.joint(actor,"hand_l")
            local yaw,pitch=p.turn.y,-p.turn.x
            local v=vector.create
            local leftAxis=v(math.cos(yaw),0,-math.sin(yaw))
            local forward=v(math.sin(yaw)*math.cos(pitch),math.sin(pitch),math.cos(yaw)*math.cos(pitch))
            local up=vector.cross(forward,leftAxis)
            local offset=leftAxis*spacing.x+up*spacing.y+forward*spacing.z
            assert(vector.magnitude(left-wrist-offset)<.022,"off-hand grip drift")
            assert(vector.magnitude(wrist+forward*.048-up*.015-world.get(p.prop).pos)<.025,"weapon left main hand")
        end
    "#);
    tick(&mut game,[Controls::default();2],16);
    check(&mut game, r#"
        state.checkHold(1,vector.create(.072,-.012,.045))
        state.checkHold(2,vector.create(.083,.028,.245))
        state.checkMuzzle=function(id,tag)
            local p=state.combat.players[id]
            local forward=vector.create(math.sin(p.turn.y)*math.cos(p.turn.x),-math.sin(p.turn.x),math.cos(p.turn.y)*math.cos(p.turn.x))
            local muzzle=-math.huge
            for _,partId in world.find(tag) do
                local part=world.get(partId)
                if part.parent==p.prop then
                    local half=if part.shape=="capsule" then part.size.y else part.size.z*.5
                    muzzle=math.max(muzzle,vector.dot(part.world_pos-world.get(p.prop).pos,forward)+half)
                end
            end
            local tip=vector.dot(p.tip-world.get(p.prop).pos,forward)
            assert(math.abs(tip-muzzle)<.003,"muzzle flash must start at the end of the rendered barrel")
        end
        state.checkMuzzle(2,"launcher-part")
        assert(state.combat.players[1].turn.x>.75)
        state.carry=humans.joint(state.population.avatars[1],"hand_r").y
    "#);
    tick(&mut game,[aim(),Controls::default()],36);
    check(&mut game, r#"
        local p=state.combat.players[1]
        state.checkHold(1,vector.create(.072,-.012,.045))
        assert(-p.turn.x>.15 and humans.joint(state.population.avatars[1],"hand_r").y>state.carry+.20)
        assert(state.combat.players[2].turn.x>.36)
        state.aimPitch=-p.turn.x
    "#);
    let strafe=Controls { mv:[1.0,0.0],..aim() };
    tick(&mut game,[shoot(),aim()],1);
    tick(&mut game,[strafe,aim()],3);
    check(&mut game, r#"
        assert(-state.combat.players[1].turn.x>state.aimPitch+.07)
        assert((state.combat.players[2].kick or 0)==0)
        state.checkHold(1,vector.create(.072,-.012,.045))
    "#);
    tick(&mut game,[strafe,aim()],33);
    check(&mut game, r#"
        assert(state.fleet.players[1].speed>1 and state.combat.players[1].kick<.005)
        state.checkHold(1,vector.create(.072,-.012,.045))
        state.checkHold(2,vector.create(.083,.028,.245))
        state.fleet.players[2].weapon="sniper"
    "#);
    tick(&mut game,[strafe,aim()],2);
    check(&mut game, r#"state.checkMuzzle(2,"growing-gun-part")"#);
}

#[test]
fn muzzle_flashes_follow_each_of_four_guns_and_release_their_street_light_slots() {
    let mut game = LuaGame::load(
        &game_folder(),
        &Overrides { players: Some(4), seed: Some(1948), ..Default::default() },
        None,
    ).unwrap();
    game.echo = false;
    game.tick(&[Controls::default(); 4]);
    check(&mut game, r#"
        local guns={"pistol","uzi","shotgun","launcher"}
        for i,seat in state.fleet.players do
            world.place(seat.body,vector.create(26+i*1.6,.85,39))
            seat.weapon,seat.view,seat.pitch=guns[i],0,.16
        end
        state.clock.hour,state.clock.secondsPerDay=22,1e15
        state.flashSample={}
    "#);
    for _ in 0..35 { game.tick(&[aim(); 4]); }
    let street_lamps = game.core().lamps.len();
    assert!(street_lamps > 0);
    game.tick(&[shoot(); 4]);
    check(&mut game, r#"
        assert(#world.find("sidearm-flash")==4)
        for i,p in state.combat.players do
            local f=world.get(p.flash)
            assert(f.parent==p.prop and vector.magnitude(f.world_pos-p.tip)<.006)
            assert(p.muzzleAge==0 and p.muzzleScale<=.55)
            state.flashSample[i]={id=p.flash,tip=p.tip,light=p.muzzleLight}
        end
        assert(state.combat.players[4].muzzleScale>state.combat.players[1].muzzleScale)
    "#);
    {
        let core = game.core();
        assert_eq!(core.lamps.len(), street_lamps+4);
        assert!(core.lamps[street_lamps..].iter().all(|l| l.range < 3.0));
    }
    for _ in 0..2 { game.tick(&[shoot(); 4]); }
    check(&mut game, r#"
        for i,p in state.combat.players do
            local saved=state.flashSample[i]
            assert(p.flash==saved.id and p.muzzleLight<saved.light)
            assert(vector.magnitude(p.tip-saved.tip)>.005)
            assert(vector.magnitude(world.get(p.flash).world_pos-p.tip)<.006)
        end
    "#);
    for _ in 0..4 { game.tick(&[aim(); 4]); }
    check(&mut game, r#"
        assert(#world.find("sidearm-flash")==0)
        for _,p in state.combat.players do assert(not p.muzzleLight) end
        state.fleet.players[1].weapon="rifle"
        state.fleet.players[2].weapon="sniper"
    "#);
    assert_eq!(game.core().lamps.len(),street_lamps);
    for _ in 0..32 { game.tick(&[aim(); 4]); }
    game.tick(&[shoot(),shoot(),aim(),aim()]);
    check(&mut game,r#"
        assert(#world.find("sidearm-flash")==2)
        for i=1,2 do local p=state.combat.players[i]; assert(world.get(p.flash).parent==p.prop) end
    "#);
    assert!(game.errors().is_empty(), "{:?}", game.logs());
}

#[test]
fn a_contract_uses_the_same_gun_and_a_bystander_in_front_intercepts_the_shot() {
    let mut game = city();
    let story = r#"return {
        id="shared",title="Shared sidewalk",
        cast={{kind="person",key="actor",name="Cast",pos=vector.create(8,0,-40)}},
        steps={{op="contract",person="actor",range=32,text="Tag the marked actor."}},
    }"#;
    game.apply_story(story).unwrap();
    line_up(&mut game);
    check(&mut game,r#"
        local pop=state.population
        for id in pop.pedestrians.active do
            if not pop.residents[id].building then state.bystander=id; break end
        end
        local c=pop.residents[state.bystander]
        c.path,c.action,c.pos=nil,"idle",vector.create(8,0,-37)
        c.nextPos=c.pos
        pop.selectionIn=0
    "#);
    tick(&mut game, [aim(), aim()], 54);
    check(&mut game, r#"
        assert(state.story.owner==1 and state.story.onTarget)
        assert(state.combat.players[1].prop and state.combat.players[2].onTarget)
    "#);
    tick(&mut game, [shoot(), aim()], 1);
    check(&mut game,r#"
        assert(state.story.status=="active" and not state.storyCast:wasHit("actor"))
        assert(state.population.residents[state.bystander].impact.stage=="down")
        assert(state.combat.players[1].hits==1 and state.combat.players[1].hitFlash>0)
    "#);
    tick(&mut game, [aim(), aim()], 24);
    tick(&mut game, [shoot(), shoot()], 1);
    check(&mut game, r#"
        local s=state.combat
        assert(state.story.status=="complete" and state.storyCast:wasHit("actor"))
        assert(s.enemies.rook.health==1 and s.players[1].hits==2 and s.players[2].hits==1)
        assert(s.players[1].shotFlash>0 and s.players[2].shotFlash>0)
    "#);
    // On the very next tick ownership returns to normal, with the same cooldown.
    tick(&mut game, [shoot(), aim()], 1);
    check(&mut game, "assert(state.combat.enemies.rook.health==1 and state.combat.players[1].aiming)");
    tick(&mut game, [aim(), aim()], 25);
    tick(&mut game, [shoot(), aim()], 1);
    check(&mut game, "assert(state.combat.players[1].defeats==3)");
}

#[test]
fn an_unfitted_resident_can_be_shot_at_its_rendered_position_and_stays_down() {
    let mut game=city();
    line_up(&mut game);
    tick(&mut game,[aim(),aim()],45);
    check(&mut game,r#"
        local pop=state.population
        for id,actor in pop.pedestrians.active do
            local c=pop.residents[id]
            if not c.building and not c.impact then
                state.bystander=id
                c.path,c.action,c.pos=nil,"idle",vector.create(8,0,-37)
                -- The distant sample walks across, perpendicular to the shot.
                c.nextPos=c.pos+vector.create(1.2,0,0)
                humans.remove(actor)
                pop.pedestrians.active[id]=nil
                break
            end
        end
        assert(state.bystander and not pop.pedestrians.active[state.bystander])
        pop.streetRevision+=1
        pop.selectionIn=1
        humans.distant("resident",pop.street,pop.pedestrians.active,.4)
    "#);
    tick(&mut game,[shoot(),Controls::default()],1);
    check(&mut game,r#"
        local pop=state.population
        local c=pop.residents[state.bystander]
        assert(c.dead and c.impact.stage=="down" and c.corpse and pop.corpses[c.corpse])
        assert(not pop.pedestrians.active[state.bystander])
        assert(c.shotUntil and state.combat.players[1].hits==1 and state.combat.enemies.rook.health==2)
        local gun=state.combat.players[1]
        assert(#world.find("combat-splat")>0 and gun.hitFlash>0 and gun.hitPoint)
        assert(vector.magnitude(gun.hitPoint-c.pos)<2 and gun.trace and world.exists(gun.trace))
        state.lastTrace=gun.trace
        state.firstHips=humans.joint(c.corpse,"hips")
    "#);
    tick(&mut game,[Controls::default();2],3);
    check(&mut game,r#"
        local c=state.population.residents[state.bystander]
        local travel=humans.joint(c.corpse,"hips")-state.firstHips
        assert(travel.x>.065 and travel.x<.24,"lost distant walk velocity: "..tostring(travel))
    "#);
    tick(&mut game,[Controls::default();2],13);
    check(&mut game,r#"
        local gun=state.combat.players[1]
        assert(not world.exists(state.lastTrace) and not gun.trace and gun.hitFlash==0 and not gun.hitPoint)
    "#);
    tick(&mut game,[Controls::default();2],254);
    check(&mut game,r#"
        local pop=state.population
        local c=pop.residents[state.bystander]
        assert(c.active==false and not pop.pedestrians.active[state.bystander])
        assert(c.dead and c.corpse and humans.joint(c.corpse,"hips"))
    "#);
}

#[test]
fn a_shot_can_reach_a_distant_walker_across_a_city_block() {
    let mut game=city();
    check(&mut game,r#"
        local p=state.fleet.players[1]
        -- The marked curb lane is occupied by the starter sedan at (3.7, 0, 11).
        -- This fixture needs an unobstructed 57 m sightline down the centre.
        world.place(p.body,vector.create(.15,.85,17))
        p.view,p.pitch=0,0
        local pop=state.population
        for id,c in pop.street do
            if c.active and not pop.pedestrians.active[id] then
                c.path,c.delta,c.action,c.pos=nil,nil,"idle",vector.create(.15,0,-40)
                c.nextPos,c.night=c.pos,true
                state.distantTarget=id
                break
            end
        end
        assert(state.distantTarget)
        pop.streetRevision+=1
        pop.selectionIn=1
        humans.distant("resident",pop.street,pop.pedestrians.active,.4)
    "#);
    let shot=Controls {pressed: Buttons(Buttons::SOUTH), ..Default::default()};
    tick(&mut game,[shot,Controls::default()],1);
    check(&mut game,r#"
        local c=state.population.street[state.distantTarget]
        local gun=state.combat.players[1]
        assert(c.shotUntil and gun.hits==1 and vector.magnitude(gun.hitPoint-c.pos)<2)
    "#);
}

#[test]
fn an_ordinary_seated_room_resident_reacts_to_the_pistol() {
    let mut game=city();
    check(&mut game,r#"
        for _,b in state.city.buildings do
            if b.kind=="office" then state.office=b; break end
        end
        local p=state.fleet.players[1]
        world.place(p.body,state.office.door+vector.create(-.46,.85,-1.1))
        p.view,p.pitch=0,-.15
        p.cam:snap()
    "#);
    tick(&mut game,[aim(),Controls::default()],30);
    check(&mut game,r#"
        local pop=state.population
        for id,actor in pop.pedestrians.active do
            local c=pop.residents[id]
            if c.building==state.office.id then
                state.worker=id
                c.delta,c.action,c.pos=nil,"sit",state.office.door+vector.create(0,0,-4.1)
                break
            end
        end
        assert(state.worker)
    "#);
    tick(&mut game,[aim(),Controls::default()],3);
    tick(&mut game,[shoot(),Controls::default()],1);
    check(&mut game,r#"
        local c=state.population.residents[state.worker]
        assert(c.pose=="sit" and c.impact and c.impact.stage=="down")
        assert(state.combat.players[1].hits==1 and #world.find("combat-drop")>0)
    "#);
    tick(&mut game,[Controls::default();2],270);
    check(&mut game,r#"assert(not state.population.residents[state.worker].active)"#);
}

// Use a private game folder so polling the edited modules exercises the normal
// hot-reload path while other tests and playtests keep running the original.
fn copy_fixture(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry=entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_fixture(&entry.path(),&to.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(),to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn space_fires_and_live_module_edits_keep_the_active_cast_contract_and_effects() {
    let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir=Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("theft-combat-{}-{stamp}",std::process::id()));
    copy_fixture(&game_folder(),&dir);
    let mut game=load_city(&dir);
    let space=Controls{pressed:Buttons(Buttons::SOUTH),..Default::default()};
    let cast=r#"{
        {kind="person",key="a",name="Arden",pos=vector.create(8,0,-40)},
        {kind="person",key="b",name="Bex",pos=vector.create(8,0,-46)}
    }"#;
    game.apply_story(&format!(r#"return {{id="two",title="Two contracts",cast={cast},steps={{
        {{op="contract",person="a",text="Find Arden."}},
        {{op="contract",person="b",text="Find Bex."}}
    }}}}"#)).unwrap();
    line_up(&mut game);
    check(&mut game,r#"
        local rook=state.combat.cast.people.rook
        rook.pos=vector.create(13,0,-40)
        world.place(rook.body,rook.pos+vector.create(0,.85,0))
        for _,p in state.fleet.players do
            world.place(p.body,vector.create(8,.85,-33))
            p.pitch=0
        end
    "#);
    tick(&mut game,[Controls::default(),space],1);
    check(&mut game,r#"
        assert(state.story.step==2 and state.story.status=="active" and state.story.contractPerson=="b")
        assert(state.combat.players[2].hits==1 and state.storyCast.people.a.impact)
    "#);
    check(&mut game,r#"
        state.keptBody=state.storyCast.people.b.body
        state.keptDrop=world.find("combat-drop")[1]
        assert(state.keptDrop and world.exists(state.keptDrop))
    "#);
    for module in ["combat","combat_fx","missions","story_cast"] {
        let path=dir.join(format!("{module}.module.luau"));
        let old=std::fs::read_to_string(&path).unwrap();
        let new=if module=="story_cast" {
            old.replace("return Cast\n","Cast.reloadedDuringTest=true\nreturn Cast\n")
               .replace("return Cast\r\n","Cast.reloadedDuringTest=true\r\nreturn Cast\r\n")
        } else { format!("{old}\n-- Hot reload while the first shot is still in flight.\n") };
        assert!(old!=new);
        std::fs::write(path,new).unwrap();
    }
    let changed=game.reload();
    assert!(changed.iter().any(|s|s.contains("main.luau: reloaded")),"{changed:?}");
    check(&mut game,r#"
        assert(state.storyCast.reloadedDuringTest and state.combat.cast.reloadedDuringTest)
        assert(state.storyCast.people.b.body==state.keptBody and state.story.step==2)
        assert(world.exists(state.keptDrop) and state.combat.players[2].hits==1)
    "#);
    // The first target still occupies the sightline immediately after reload.
    tick(&mut game,[space,Controls::default()],1);
    check(&mut game,r#"
        local p=state.combat.players[1]
        assert(p.hitFlash>0 and p.hits==0 and state.story.status=="active")
        local seat=state.fleet.players[1]
        local b=state.storyCast.people.b
        world.place(seat.body,b.pos+vector.create(0,.85,1.4))
        seat.view,seat.pitch=0,0
    "#);
    tick(&mut game,[Controls::default();2],30);
    tick(&mut game,[space,Controls::default()],1);
    check(&mut game,r#"
        local p=state.combat.players[1]
        assert(state.story.status=="complete" and state.storyCast.people.b.impact,
            tostring(p.hits).." hits, contact "..tostring(p.hitPoint)..", player "..tostring(state.fleet.players[1].pos)..", B "..tostring(state.storyCast.people.b.pos))
        local seat=state.fleet.players[1]
        world.place(seat.body,vector.create(8,.85,-33))
        seat.view,seat.pitch=0,0
    "#);
    tick(&mut game,[Controls::default();2],65);
    check(&mut game,r#"assert(not world.exists(state.keptDrop))"#);
    game.apply_story(&format!(r#"return {{id="escort",title="Escort",cast={cast},steps={{
        {{op="pickup",person="a",text="Pick up Arden."}}
    }}}}"#)).unwrap();
    tick(&mut game,[Controls::default();2],20);
    tick(&mut game,[space,Controls::default()],1);
    check(&mut game,r#"
        assert(state.story.status=="failed" and state.story.objective=="Arden was shot.")
        assert(state.storyCast.people.a.impact and state.storyCast.people.b.shot==nil)
    "#);
    drop(game);
    std::fs::remove_dir_all(dir).unwrap();
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
    check(&mut game,r#"
        local seat=state.fleet.players[1]
        world.place(seat.body,state.combat.cast.people.rook.pos+vector.create(0,.85,7))
    "#);
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

#[test]
fn simultaneous_shots_detonate_visible_cargo_once_and_keep_the_city_running() {
    let mut game = city();
    check(&mut game, r#"
        for id,crate in state.combat.crates do
            if crate.pos.x < 11 then state.cargo=id end
        end
        -- Isolate the sightlines above the junction. Use the actual cargo entity,
        -- two ordinary rivals and both seat triggers.
        local pos=vector.create(0,42.625,0)
        state.combat.crates[state.cargo].pos=pos
        world.place(state.cargo,pos)
        world.spawn("cube",{pos=vector.create(0,41.9,0),size=vector.create(30,.2,30),solid=true,tag="city"})
        for key,offset in {rook=vector.create(3,0,0),vex=vector.create(0,0,-9)} do
            local p=state.combat.cast.people[key]
            p.pos,p.route,p.arrived=pos+offset-vector.create(0,.625,0),nil,false
            world.place(p.body,p.pos+vector.create(0,.85,0))
            humans.set(p.actor,{pos=p.pos,pose="idle"})
        end
        for _,seat in state.fleet.players do
            world.place(seat.body,pos+vector.create(0,.225,5))
            seat.view,seat.pitch=0,-.13
        end
        state.wall=world.spawn("cube",{tag="city",solid=true,pos=pos+vector.create(0,0,2.5),size=vector.create(2,3,.4)})
    "#);
    let fire = Controls { pressed: Buttons(Buttons::RIGHT_TRIGGER), ..Default::default() };
    tick(&mut game, [fire;2], 1);
    check(&mut game, "assert(world.get(state.cargo).visible); world.remove(state.wall)");
    assert!(game.core().world.explosions.is_none());
    tick(&mut game, [Controls::default();2], 22);
    tick(&mut game, [fire;2], 1);
    check(&mut game, r#"
        local c=state.combat.crates[state.cargo]
        assert(not world.get(state.cargo).visible and not world.get(state.cargo).solid)
        assert(c.respawnAt-game.time()>14)
        local s=state.combat
        assert(s.players[1].defeats+s.players[2].defeats==1)
        assert(s.cast.people.rook.gibbed and s.enemies.rook.health==0)
        assert(s.enemies.vex.health>0 and s.enemies.vex.health<2)
        assert(not s.cast.people.vex.gibbed)
        local burst=s.effects.live.shard
        assert(#burst==7 and burst[1].blast and burst[7].blast)
        assert(vector.magnitude(burst[3].pos-burst[7].pos)>.75)
        assert(#s.effects.live.splat>=2 and #s.effects.live.drop>=8)
    "#);
    assert_eq!(game.core().world.explosions.as_ref().unwrap().explosion_stats().full, 1);
    check(&mut game, r#"
        local c=state.combat.crates[state.cargo]
        world.place(state.fleet.players[1].body,c.pos+vector.create(0,.225,0))
        c.respawnAt=game.time()
    "#);
    tick(&mut game, [Controls::default();2], 1);
    check(&mut game, r#"
        assert(not world.get(state.cargo).visible)
        world.place(state.fleet.players[1].body,state.combat.crates[state.cargo].pos+vector.create(0,.225,5))
    "#);
    tick(&mut game, [Controls::default();2], 32);
    check(&mut game, "assert(world.get(state.cargo).visible and world.get(state.cargo).solid)");
}

#[test]
fn parked_car_takes_panel_damage_from_gunfire_and_nearby_cargo() {
    let mut game=city();
    check(&mut game,r#"
        local car=state.fleet.vehicles[1]
        local seat=state.fleet.players[1]
        world.place(seat.body,car.pos+vector.create(0,.85,5.6))
        seat.view,seat.pitch=0,-.14
        state.carCover=world.spawn("cube",{pos=car.pos+vector.create(0,1.3,3.2),size=vector.create(2.3,2.6,.2),solid=true,tag="city"})
    "#);
    let fire=Controls {pressed:Buttons(Buttons::RIGHT_TRIGGER),..Default::default()};
    tick(&mut game,[fire,Controls::default()],1);
    check(&mut game,"local d=state.fleet.vehicles[1].damage; assert(not d or d.total==0); world.remove(state.carCover)");
    tick(&mut game,[Controls::default();2],24);
    tick(&mut game,[fire,Controls::default()],1);
    check(&mut game,r#"
        local car=state.fleet.vehicles[1]
        assert(car.damage and car.damage.zones.rear>0 and car.damage.total>0)
        state.bulletDamage=car.damage.total
        for id,crate in state.combat.crates do
            if crate.pos.x<11 then
                local pos=car.pos+vector.create(-3.4,.625,1.7)
                crate.pos=pos
                world.place(id,pos)
                local seat=state.fleet.players[1]
                world.place(seat.body,pos+vector.create(0,.225,5))
                seat.view,seat.pitch=0,-.13
                break
            end
        end
    "#);
    tick(&mut game,[Controls::default();2],24);
    tick(&mut game,[fire,Controls::default()],1);
    check(&mut game,r#"
        local d=state.fleet.vehicles[1].damage
        assert(d.total>state.bulletDamage and d.zones.left>0)
    "#);
}

#[test]
fn launcher_flight_sweeps_thin_cover_and_only_hits_people_on_the_exposed_side() {
    let mut game = city();
    line_up(&mut game);
    check(&mut game, r#"
        local seat=state.fleet.players[1]
        seat.weapon="launcher"
        seat.pitch=0
        local wall=vector.create(8,1.5,-38)
        state.launcherWall=world.spawn("cube",{tag="city",pos=wall,size=vector.create(6,3,.05),solid=true})
        local vex=state.combat.cast.people.vex
        vex.pos,vex.route,vex.arrived=vector.create(9.7,0,-36),nil,false
        world.place(vex.body,vex.pos+vector.create(0,.85,0))
        humans.set(vex.actor,{pos=vex.pos,pose="idle"})
    "#);
    let fire=Controls {pressed:Buttons(Buttons::RIGHT_TRIGGER),..Default::default()};
    tick(&mut game,[fire,Controls::default()],1);
    check(&mut game, r#"
        local s=state.combat
        assert(s.players[1].weapon=="launcher" and s.players[2].weapon=="pistol")
        assert(#s.projectiles==1 and s.projectiles[1].owner==1)
        assert(#world.find("launcher-projectile")==1 and #world.find("launcher-fuse")==1)
        assert(s.players[1].shotFlash>0 and s.players[2].shotFlash==0)
        state.firstProjectilePos=s.projectiles[1].pos
        assert(s.enemies.rook.health==2 and s.enemies.vex.health==2)
    "#);
    tick(&mut game,[fire,Controls::default()],2);
    check(&mut game, r#"
        assert(#state.combat.projectiles==1)
        assert(vector.magnitude(state.combat.projectiles[1].pos-state.firstProjectilePos)>.8)
        state.fleet.players[1].weapon="pistol"
    "#);
    assert!(game.core().world.explosions.is_none());
    tick(&mut game,[Controls::default();2],16);
    check(&mut game, r#"
        local s=state.combat
        assert(s.players[1].weapon=="pistol" and #s.projectiles==0)
        assert(#world.find("launcher-projectile")==0 and #world.find("launcher-fuse")==0)
        assert(s.enemies.rook.health==2 and not s.cast.people.rook.impact)
        assert(s.enemies.vex.health==0 and s.cast.people.vex.impact)
        assert(s.players[1].defeats>=1 and s.players[2].hits==0)
    "#);
    assert_eq!(game.core().world.explosions.as_ref().unwrap().explosion_stats().full,1);
}

#[test]
fn each_seat_owns_its_flying_launcher_bolt_and_empty_sky_shots_expire() {
    let mut game=city();
    check(&mut game,r#"
        for id,seat in state.fleet.players do
            seat.weapon="launcher"
            world.place(seat.body,vector.create(12*id,160,-20))
            seat.pitch=.29
            seat.view=if id==1 then .9 else -.9
        end
    "#);
    let fire=Controls {pressed:Buttons(Buttons::SOUTH),..Default::default()};
    tick(&mut game,[fire;2],1);
    check(&mut game,r#"
        local s=state.combat
        assert(#s.projectiles==2 and s.projectiles[1].owner==1 and s.projectiles[2].owner==2)
        assert(s.players[1].launcherCooldown>0 and s.players[2].launcherCooldown>0)
        assert(s.projectiles[1].dir.x<0 and s.projectiles[2].dir.x>0)
        state.initialVelocity=s.projectiles[1].velocity
        state.grenadeStart=s.projectiles[1].pos
    "#);
    tick(&mut game,[Controls::default();2],40);
    check(&mut game,r#"
        local g=state.combat.projectiles[1]
        assert(g.velocity.y<state.initialVelocity.y-10)
        assert(g.pos.y>state.grenadeStart.y+8 and g.pos.y<state.grenadeStart.y+state.initialVelocity.y*40/60-2)
    "#);
    tick(&mut game,[Controls::default();2],70);
    check(&mut game,r#"
        assert(#state.combat.projectiles==2 and #world.find("launcher-projectile")==2)
        assert(state.combat.players[1].launcherCooldown==0)
    "#);
    tick(&mut game,[Controls::default();2],24);
    check(&mut game,r#"
        assert(#state.combat.projectiles==0 and #world.find("launcher-projectile")==0 and #world.find("launcher-fuse")==0)
        assert(state.combat.players[1].hits==0 and state.combat.players[2].hits==0)
        local s=state.fleet.players[1]
        world.place(s.body,state.fleet.vehicles[1].pos+vector.create(1.6,.85,0))
    "#);
    assert_eq!(game.core().world.explosions.as_ref().unwrap().explosion_stats().full,2);
    tick(&mut game,[Controls::default();2],4);
    let enter=Controls {pressed:Buttons(Buttons::NORTH),..Default::default()};
    tick(&mut game,[enter,Controls::default()],1);
    tick(&mut game,[fire,Controls::default()],1);
    check(&mut game,r#"assert(state.fleet.players[1].mode=="drive" and not state.combat.players[1].prop and #state.combat.projectiles==0)"#);
}

#[test]
fn growing_guns_fire_rates_and_magazines_are_independent_for_each_trigger() {
    let mut game=city();
    check(&mut game,r#"
        for id,seat in state.fleet.players do
            world.place(seat.body,vector.create(20*id,180,-20))
            seat.pitch=.3
            seat.weapon=if id==1 then "uzi" else "sniper"
        end
    "#);
    let held=Controls {held:Buttons(Buttons::RIGHT_TRIGGER),..Default::default()};
    let fire=Controls {held:Buttons(Buttons::RIGHT_TRIGGER),pressed:Buttons(Buttons::RIGHT_TRIGGER),..Default::default()};
    tick(&mut game,[fire;2],1);
    check(&mut game,r#"
        local a,b=state.combat.players[1],state.combat.players[2]
        assert(a.prop and b.prop and a.trace and b.trace and a.ammo==24 and b.ammo==4)
        assert(a.shotNumber==1 and b.shotNumber==1)
        for _,player in {a,b} do
            local streak=world.get(player.trace)
            assert(streak.emissive==0 and streak.size.z<=.47 and streak.size.x<.015)
            assert(vector.magnitude(streak.pos-player.tip)<3)
        end
        state.firstStreak=world.get(a.trace).pos
    "#);
    tick(&mut game,[held;2],1);
    check(&mut game,r#"
        local a=state.combat.players[1]
        assert(a.trace and vector.magnitude(world.get(a.trace).pos-state.firstStreak)>5)
        assert(#state.combat.projectiles==0)
    "#);
    tick(&mut game,[held;2],5);
    check(&mut game,r#"
        local a=state.combat.players[1]
        assert(a.shotNumber==2 and not a.trace,"rapid follow-up should fire without a visible streak")
    "#);
    tick(&mut game,[held;2],54);
    check(&mut game,r#"
        local a,b=state.combat.players[1],state.combat.players[2]
        assert(a.shotNumber>=14 and a.shotNumber<=15 and a.ammo==25-a.shotNumber,a.shotNumber)
        assert(b.shotNumber==2 and b.ammo==3,b.shotNumber)
        assert(a.magazine==25 and b.magazine==5 and a.reloadRemaining==0)
        state.fleet.players[2].weapon="rifle"
    "#);
    tick(&mut game,[Controls::default(),fire],1);
    check(&mut game,r#"assert(state.combat.players[2].ammo==4 and state.combat.players[2].magazine==5)"#);
}

#[test]
fn live_shotgun_parameters_change_range_pellet_damage_and_reload_in_place() {
    let mut game=city();
    line_up(&mut game);
    check(&mut game,r#"
        local seat=state.fleet.players[1]
        world.place(seat.body,state.combat.cast.people.rook.pos+vector.create(0,.85,7))
        seat.view,seat.pitch=0,0
        state.fleet.players[2].weapon="shotgun"
    "#);
    let equip=|game:&mut LuaGame,range:u32| {
        game.apply_story(&format!(r#"return {{id="sample",title="Test loadout",cast={{}},steps={{{{
            op="equip",weapon="shotgun",params={{pellets=3,damage=.4,spread=0,range={range},
            automatic=false,magazine=2,reload=.35,interval=.10}},text="Equip"
        }}}}}}"#)).unwrap();
    };
    equip(&mut game,2);
    tick(&mut game,[Controls::default();2],2);
    let fire=Controls {held:Buttons(Buttons::RIGHT_TRIGGER),pressed:Buttons(Buttons::RIGHT_TRIGGER),..Default::default()};
    let held=Controls {held:Buttons(Buttons::RIGHT_TRIGGER),..Default::default()};
    tick(&mut game,[fire,Controls::default()],1);
    tick(&mut game,[held,Controls::default()],20);
    check(&mut game,r#"
        local c=state.combat
        assert(c.enemies.rook.health==2 and c.players[1].ammo==1 and c.players[1].shotNumber==1)
        assert(c.players[2].ammo==5)
        state.previousGun=c.players[1].prop
    "#);
    equip(&mut game,30);
    tick(&mut game,[Controls::default();2],2);
    check(&mut game,r#"assert(not world.exists(state.previousGun) and state.combat.players[1].ammo==1)"#);
    tick(&mut game,[fire,Controls::default()],1);
    check(&mut game,r#"
        local c=state.combat
        assert(math.abs(c.enemies.rook.health-.8)<.001,c.enemies.rook.health)
        assert(c.players[1].hits==1 and c.players[1].ammo==0 and c.players[1].reloadRemaining>.3)
        state.fleet.players[1].weapon="pistol"
    "#);
    tick(&mut game,[Controls::default();2],25);
    check(&mut game,r#"assert(not state.combat.players[1].ammo); state.fleet.players[1].weapon="shotgun""#);
    tick(&mut game,[Controls::default();2],1);
    check(&mut game,r#"
        local c=state.combat
        assert(c.players[1].ammo==2 and c.players[1].reloadRemaining==0)
        assert(c.players[2].ammo==5 and not c.players[2].shotNumber)
    "#);
}

#[test]
fn a_sniper_card_uses_its_contact_multiplier_and_a_spread_shot_draws_a_bounded_fan() {
    let mut game=city();
    line_up(&mut game);
    game.apply_story(r#"return {id="scope-test",title="Scope",cast={},steps={{
        op="equip",weapon="sniper",params={damage=.4,spread=0},text="Equip"
    }}}"#).unwrap();
    check(&mut game,r#"
        local seat=state.fleet.players[1]
        world.place(seat.body,state.combat.cast.people.rook.pos+vector.create(0,.85,7))
        seat.view,seat.pitch=0,.08
        state.fleet.players[2].weapon="shotgun"
    "#);
    tick(&mut game,[Controls::default();2],2);
    let fire=Controls {pressed:Buttons(Buttons::RIGHT_TRIGGER),..Default::default()};
    tick(&mut game,[fire,Controls::default()],1);
    check(&mut game,r#"
        local c=state.combat
        assert(math.abs(c.enemies.rook.health-.8)<.001,c.enemies.rook.health)
        assert(c.players[1].hits==1 and c.players[1].magazine==5)
        state.fleet.players[2].view=2.7
        state.fleet.players[2].pitch=.30
    "#);
    tick(&mut game,[Controls::default(),fire],1);
    check(&mut game,r#"
        local s=state.combat.players[2]
        assert(s.trace and (s.leftTrace or s.rightTrace))
        assert(#world.find("sidearm-trace")<=4)
    "#);
    tick(&mut game,[Controls::default();2],12);
    check(&mut game,"assert(#world.find('sidearm-trace')==0)");
}
#[test]
fn blast_passes_over_the_bonnet_but_the_cabin_protects_a_bystander() {
    let mut game = city();
    check(&mut game,r#"
        local v=vector.create
        local fleet, combat=state.fleet,state.combat
        local floor=v(0,42,0)
        local car=combat.cast:car("blast-cover",{pos=floor,yaw=0})
        local center=floor+v(-3,1.24,-2.06)
        world.spawn("cube",{tag="city",solid=true,pos=floor-v(0,.10,0),size=v(25,.2,25)})
        for id,crate in combat.crates do
            if crate.pos.x<11 then
                crate.pos=center; world.place(id,center); state.testCargo=id
            end
        end
        for key,z in {rook=-2.06,vex=2.06} do
            local p=combat.cast.people[key]
            p.pos,p.route,p.arrived=floor+v(3,.27,z),nil,false
            world.place(p.body,p.pos+v(0,.85,0))
            humans.set(p.actor,{pos=p.pos,pose="idle"})
            local delta=p.pos+v(0,.97,0)-center
            local legacy=world.raycast(center,delta,vector.magnitude(delta))
            assert(legacy and legacy.id==car.body,"both paths cross the coarse bounds")
            assert((vehicles.raycast(car.native,"toyama",center,delta,vector.magnitude(delta))~=nil)==(key=="vex"))
        end
        local seat=fleet.players[1]
        world.place(seat.body,center+v(0,-.39,5))
        seat.view,seat.pitch=0,0
    "#);
    let fire=Controls { pressed:Buttons(Buttons::RIGHT_TRIGGER), ..Default::default() };
    tick(&mut game,[fire,Controls::default()],1);
    check(&mut game,r#"
        assert(state.combat.crates[state.testCargo].respawnAt,"the shot must hit the cargo")
        assert(state.combat.enemies.rook.health<=0,"open air over the bonnet must transmit the blast")
        assert(state.combat.enemies.vex.health==2,"the closed cabin must block the blast")
    "#);
}

#[test]
fn a_settled_corpse_accepts_a_second_seats_shot_and_only_fragments_once() {
    let mut game = city();
    let idle = [Controls::default(); 2];
    let fire = Controls { pressed: Buttons(Buttons::RIGHT_TRIGGER), ..Default::default() };
    check(&mut game,r#"
        local v=vector.create
        local floor=v(8,24,-35)
        world.spawn("cube",{tag="city",pos=floor-v(0,.1,0),size=v(22,.2,22),solid=true})
        local s=state.combat
        local p=s.cast.people.rook
        p.pos,p.route,p.arrived=floor,nil,false
        world.place(p.body,floor+v(0,.85,0))
        humans.set(p.actor,{pos=floor,pose="idle"})
        assert(s.cast:kill("rook",v(0,.3,-1)))
        s.enemies.rook.health,s.enemies.rook.respawnIn,s.enemies.rook.waitClear=0,45,true
        state.fallen=p.corpse
        for i,seat in state.fleet.players do world.place(seat.body,floor+v(i,.85,5)) end
    "#);
    tick(&mut game,idle,320);
    check(&mut game,r#"
        local v=vector.create
        local head=humans.joint(state.fallen,"head")
        assert(head and state.population.corpses[state.fallen] and head.y<25)
        local seat=state.fleet.players[2]
        local feet=v(head.x,24,head.z+4)
        world.place(seat.body,feet+v(0,.85,0))
        seat.view,seat.pitch=0,math.atan2(head.y-feet.y-1.25,4)
        state.lastHead=head
    "#);
    tick(&mut game,[Controls::default(),fire],1);
    check(&mut game,r#"
        local player=state.combat.players[2]
        assert(player.hitFlash>0 and player.hits==0 and player.defeats==0)
        assert(vector.magnitude(player.hitPoint-state.lastHead)<.38)
        assert(state.population.corpses[state.fallen])
    "#);
    tick(&mut game,idle,15);
    check(&mut game,r#"
        local v=vector.create
        local head=humans.joint(state.fallen,"head")
        assert(vector.magnitude(head-state.lastHead)>.01)
        local c=state.combat
        for id,crate in c.crates do
            if not state.cargo then
                state.cargo=id
                crate.pos=v(head.x+2,24.6,head.z)
                world.place(id,crate.pos)
                local seat=state.fleet.players[1]
                world.place(seat.body,v(crate.pos.x,24.85,crate.pos.z+5))
                seat.view,seat.pitch=0,math.atan2(-.65,5)
            end
        end
    "#);
    tick(&mut game,[fire,Controls::default()],1);
    check(&mut game,r#"
        local c=state.combat
        assert(c.crates[state.cargo].respawnAt and c.cast.people.rook.gibbed)
        assert(not state.population.corpses[state.fallen] and not humans.joint(state.fallen,"hips"))
        assert(#c.effects.live.shard>=7 and #c.effects.live.splat>0)
        for _,p in c.players do assert(p.hits==0 and p.defeats==0) end
        state.fragmentSerial=c.effects.serial
        for id,crate in c.crates do
            if id~=state.cargo then
                state.secondCargo=id
                crate.pos=c.crates[state.cargo].pos
                world.place(id,crate.pos)
                break
            end
        end
    "#);
    tick(&mut game,idle,25);
    tick(&mut game,[fire,Controls::default()],1);
    check(&mut game,r#"
        local c=state.combat
        assert(c.crates[state.secondCargo].respawnAt,"second blast did not fire")
        assert(state.fragmentSerial==c.effects.serial,"consumed corpse produced another hit")
    "#);
}

#[test]
fn a_direct_grenade_wrecks_the_car_on_contact_and_releases_its_parts_once() {
    let mut game=city();
    let idle=[Controls::default();2];
    let fire=Controls {pressed:Buttons(Buttons::RIGHT_TRIGGER),..Default::default()};
    check(&mut game,r#"
        local v=vector.create
        local car=state.fleet.vehicles[1]
        local seat=state.fleet.players[1]
        world.place(seat.body,car.pos+v(0,.85,5.6))
        seat.weapon,seat.view,seat.pitch="launcher",0,-.14
        state.originalBody=car.body
        local protected=assert(state.combat.cast:car("protected",{pos=car.pos+v(3.3,0,0),paint="pearl"}))
        state.protected=protected.index
        world.spawn("cube",{tag="city",solid=true,pos=car.pos+v(1.6,1,0),size=v(.16,2.2,6)})
    "#);
    tick(&mut game,[fire,Controls::default()],1);
    check(&mut game,"assert(#state.combat.projectiles==1 and not state.fleet.vehicles[1].wrecked)");
    for _ in 0..24 {
        tick(&mut game,idle,1);
        if game.luau("return #state.combat.projectiles==0").unwrap()=="true" { break; }
    }
    check(&mut game,r#"
        local car=state.fleet.vehicles[1]
        assert(car.wrecked and not car.burnRemaining and car.damage.disabled,"direct grenade must bypass the fuse")
        assert(car.body==state.originalBody and world.exists(car.body))
        assert(not state.fleet.vehicles[state.protected].wrecked,"wall must block the close grenade")
        assert(state.fleet.vehicles[2].damage.total>0,"wreck blocked its own blast")
        assert(state.combat.players[1].hitFlash>0)
        assert(car.missingParts==126 and car.missingWheels==15)
    "#);
    assert_eq!(game.core().world.explosions.as_ref().unwrap().explosion_stats().full,1);
    tick(&mut game,idle,2);
    check(&mut game,r#"
        assert(#state.fleet.debris==10)
        for _,d in state.fleet.debris do assert(world.exists(d.entity) and vector.magnitude(d.vel)>1) end
    "#);
}

#[test]
fn a_pistol_can_ignite_a_damaged_car_before_its_delayed_blast() {
    let mut game=city();
    let idle=[Controls::default();2];
    let fire=Controls {pressed:Buttons(Buttons::RIGHT_TRIGGER),..Default::default()};
    check(&mut game,r#"
        local car=state.fleet.vehicles[1]
        car.damage.total=.80
        local seat=state.fleet.players[2]
        world.place(seat.body,car.pos+vector.create(0,.85,5.6))
        seat.weapon,seat.view,seat.pitch="pistol",0,-.14
    "#);
    tick(&mut game,[Controls::default(),fire],1);
    check(&mut game,r#"
        local car=state.fleet.vehicles[1]
        assert(car.burnRemaining and car.burnRemaining>5 and car.fireOwner==2)
        assert(not car.wrecked and car.missingParts==nil)
    "#);
    tick(&mut game,idle,240);
    check(&mut game,"local car=state.fleet.vehicles[1]; assert(car.burnRemaining>0 and not car.wrecked)");
    tick(&mut game,idle,135);
    check(&mut game,r#"
        local car=state.fleet.vehicles[1]
        assert(car.wrecked and car.missingParts==126 and car.missingWheels==15)
        assert(car.damage.disabled and not car.burnRemaining and #state.fleet.debris==10)
    "#);
}

#[test]
fn a_grenade_beside_a_story_car_detonates_it_and_ends_its_race() {
    let mut game=city();
    game.apply_story(r#"return {id="racecar",title="Race car",
        cast={{kind="car",key="coupe",name="Blue Coupe",pos=vector.create(1,0,-25)}},
        steps={{op="enter",car="coupe",text="Get in."}}}"#).unwrap();
    check(&mut game,r#"
        local v=vector.create
        local car=state.storyCast.cars.coupe
        local box=car.pos+v(2.55,.625,0)
        for id,crate in state.combat.crates do
            if crate.pos.x<11 then
                crate.pos=box
                world.place(id,box)
                state.targetCrate=id
            end
        end
        local seat=state.fleet.players[1]
        world.place(seat.body,box+v(0,.225,5))
        seat.weapon,seat.view,seat.pitch="launcher",0,-.13
    "#);
    let fire=Controls{pressed:Buttons(Buttons::RIGHT_TRIGGER),..Default::default()};
    tick(&mut game,[fire,Controls::default()],1);
    tick(&mut game,[Controls::default();2],24);
    check(&mut game,r#"
        assert(state.combat.crates[state.targetCrate].respawnAt)
        assert(state.storyCast.cars.coupe.wrecked and state.storyCast.cars.coupe.charred)
        assert(state.story.status=="failed" and state.story.objective=="Blue Coupe was destroyed.")
    "#);
}
