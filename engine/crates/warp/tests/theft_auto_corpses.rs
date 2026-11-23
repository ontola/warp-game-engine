//! Retained bodies share the fitted pool and survive mission and crowd streaming.
use app::{Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

fn copy_game(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() { copy_game(&entry.path(), &path); }
        else { std::fs::copy(entry.path(), path).unwrap(); }
    }
}

fn city() -> (LuaGame, std::path::PathBuf) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("corpse-roster-{}-{stamp}", std::process::id()));
    copy_game(&source, &dir);
    // Warp libraries are imported when the file loads. Expose these three to
    // test commands using the same module instances as the actual city.
    let main = dir.join("main.luau");
    let script = std::fs::read_to_string(&main).unwrap();
    std::fs::write(main, format!(r#"testImpacts=require("impacts")
testCorpses=require("corpses")
testCast=require("story_cast")
{script}"#)).unwrap();
    let mut g = LuaGame::load(&dir, &Overrides { players: Some(4), seed: Some(1948), ..Default::default() }, None).unwrap();
    for _ in 0..3 { g.tick(&[Controls::default(); 4]); }
    assert!(g.errors().is_empty(), "{:?}", g.logs());
    (g, dir)
}

fn check(g: &mut LuaGame, code: &str) {
    g.luau(code).unwrap_or_else(|err| panic!("{err}\n{code}"));
}

#[test]
fn a_fitted_resident_persists_for_the_last_observer_and_expires_offscreen() {
    let (mut g, dir) = city();
    check(&mut g, r#"
        local pop,fleet=state.population,state.fleet
        local impact=testImpacts
        for id,actor in pop.pedestrians.active do
            local c=pop.residents[id]
            if id<=pop.streetCount and c.active and not c.impact then
                assert(impact.shootResident(pop,fleet,id,c.pos,vector.create(1,.1,0),false))
                assert(c.dead and c.corpse==actor and pop.corpses[actor].person==c)
                assert(not pop.pedestrians.active[id] and not c.active)
                assert(not impact.shootResident(pop,fleet,id,c.pos,vector.zero,false))
                state.fallenActor,state.fallenSource=actor,id
                break
            end
        end
        assert(state.fallenActor)
    "#);
    let id = g.luau("return state.fallenSource").unwrap().parse::<usize>().unwrap();
    assert!(g.core().humans.distant_pose("resident", id, 1.).is_none());
    let module = dir.join("corpses.module.luau");
    let script = std::fs::read_to_string(&module).unwrap();
    std::fs::write(&module, format!("{script}\n-- live edit during this fall\n")).unwrap();
    let changes = g.reload();
    assert!(changes.iter().any(|v| v.contains("main.luau: reloaded")), "{changes:?}");
    for _ in 0..15 { g.tick(&[Controls::default(); 4]); }
    assert!(g.errors().is_empty(), "{:?}", g.logs());
    check(&mut g, r#"
        local pop,fleet,actor=state.population,state.fleet,state.fallenActor
        local c=pop.residents[state.fallenSource]
        local roster=testCorpses
        local body=assert(pop.corpses[actor])
        local roots={}
        local count=0
        roster.visit(pop,body.pos,body.pos,function(id,b)
            if id==actor then assert(b==body); count+=1 end
        end)
        assert(count==1)
        local far=body.pos+vector.create(250,0,250)
        roster.visit(pop,far,far,function(id) assert(id~=actor) end)
        for _,p in fleet.players do p.pos=far end
        pop.elapsed=body.born+44.9
        roster.tick(pop,fleet,roots)
        assert(c.corpse==actor and #roots>=1)
        fleet.players[4].pos=body.pos
        pop.elapsed=body.born+150
        roster.tick(pop,fleet,{})
        assert(c.corpse==actor and not pop.pedestrians.active[state.fallenSource])
        fleet.players[4].pos=far
        roster.tick(pop,fleet,{})
        assert(not c.corpse and not pop.corpses[actor] and c.dead and not c.active)
        assert(not humans.joint(actor,"hips") and not roster.gib(pop,actor))
    "#);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn mission_cleanup_live_rebind_and_budget_do_not_reset_retained_people() {
    let (mut g, dir) = city();
    check(&mut g, r#"
        local pop,fleet=state.population,state.fleet
        local C,roster=testCast,testCorpses
        local cast=C.new(fleet,pop)
        local pos=fleet.players[4].pos+vector.create(2,0,2)
        local p=assert(cast:person("courier",{pos=pos}))
        assert(cast:hit("courier",vector.create(1,.2,0)) and not p.dead)
        -- Simulate a named casualty already down when the module was edited.
        p.shot=true
        C.bind(cast,fleet,pop)
        assert(p.dead and p.corpse==p.actor and not fleet.storyPeople[p.actor])
        C.bind(cast,fleet,pop)
        assert(not cast:kill("courier",vector.zero,false))
        assert(not cast:walk("courier",{pos}))
        cast:clear()
        assert(not cast.people.courier and pop.corpses[p.actor].person==p)
        local replacement=assert(cast:person("courier",{pos=pos,look=(p.look+1)%8}))
        assert(replacement~=p and replacement.actor~=p.actor and not replacement.dead)
        assert(roster.gib(pop,p.actor) and not roster.gib(pop,p.actor))
        assert(not humans.joint(p.actor,"hips") and humans.joint(replacement.actor,"hips"))
        cast:clear()

        local _,capacity=humans.room("resident")
        local limit=math.clamp(math.ceil(capacity/3),1,10)
        for i=1,limit+5 do
            local key="extra"..i
            local actor=assert(cast:person(key,{pos=pos+vector.create(i*3,0,0)}))
            assert(cast:kill(key,vector.create(1,0,0),false))
            cast:clear()
            local count=0
            for _,b in pop.corpses do if b.pool=="resident" then count+=1 end end
            assert(count<=limit and count<=12)
        end
        local count=0
        roster.visit(pop,nil,nil,function() count+=1 end)
        assert(count>=3 and count<=limit)
        for id=1,4 do assert(humans.joint(pop.avatars[id],"hips")) end
    "#);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reentering_a_streamed_room_keeps_the_same_fallen_worker() {
    let (mut g, dir) = city();
    check(&mut g, r#"
        local fleet=state.fleet
        for _,b in state.city.buildings do
            if b.kind=="office" then state.office=b; break end
        end
        for _,seat in fleet.players do world.place(seat.body,state.office.door+vector.create(0,.85,-1.1)) end
    "#);
    for _ in 0..40 { g.tick(&[Controls::default(); 4]); }
    check(&mut g, r#"
        local pop=state.population
        for id,actor in pop.pedestrians.active do
            local c=pop.residents[id]
            if c.building==state.office.id then
                assert(testImpacts.shootResident(pop,state.fleet,id,c.pos,vector.zero,false))
                state.worker,state.workerActor=id,actor
                break
            end
        end
        assert(state.worker)
        for _,seat in state.fleet.players do
            world.place(seat.body,state.office.door+vector.create(105,.85,105))
        end
    "#);
    for _ in 0..35 { g.tick(&[Controls::default(); 4]); }
    check(&mut g, r#"
        local pop=state.population
        assert(not pop.rooms[state.office.id] and pop.absentWorkers[state.worker]==pop.corpses[state.workerActor].person)
        world.place(state.fleet.players[4].body,state.office.door+vector.create(0,.85,-1.1))
    "#);
    for _ in 0..40 { g.tick(&[Controls::default(); 4]); }
    check(&mut g, r#"
        local pop=state.population
        local c=assert(pop.residents[state.worker])
        assert(c==pop.corpses[state.workerActor].person and c.corpse==state.workerActor)
        assert(not c.active and not pop.pedestrians.active[state.worker])
        assert(humans.joint(state.workerActor,"hips"))
    "#);
    assert!(g.errors().is_empty(), "{:?}", g.logs());
    std::fs::remove_dir_all(dir).unwrap();
}
