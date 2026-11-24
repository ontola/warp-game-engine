//! Exercise the same author-data boundary used by the browser editor.
use app::{Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

const LAYOUT: &str = include_str!("../../../../games/warp-theft-auto/west_cape_layout.module.luau");
const RALLY: &str = include_str!("../../../../games/warp-theft-auto/story_rally.module.luau");

#[test]
fn editing_a_live_rally_bend_retains_both_seats_and_restores_the_bake() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(&dir, &Overrides {seed:Some(1948),players:Some(2),..Default::default()}, None).unwrap();
    let idle = [Controls::default(); 2];
    game.tick(&idle);
    game.apply_story(RALLY).unwrap();
    game.luau(r#"
        assert(world.get(world.find("west-cape")[1]).shape=="assets/west-cape.glb")
        state.testSeats={state.fleet.players[1].body,state.fleet.players[2].body}
        state.testCar=state.storyCast.cars.kestrel.native
        state.testMoney=state.job.cash
        state.testGate=state.story.run.definition.steps[4].pos
    "#).unwrap();
    let moved = LAYOUT.replace("{-738,-52,4}", "{-746,-52,4}");
    assert_ne!(moved, LAYOUT);
    assert_eq!(game.apply_story(&moved).unwrap(), "West Cape");
    game.tick(&idle);
    game.luau(r#"
        local third=state.story.run.definition.steps[4].pos
        local data=state.city.cape.data
        local xz=data.rally.gates[3]
        assert(math.abs(third.x-xz[1])<.002 and math.abs(third.z-xz[2])<.002)
        assert(math.abs(xz[1]-state.testGate.x)>6)
        assert(data.roads[3][21][1]==xz[1])
        assert(#world.find("west-cape")==1 and world.get(world.find("west-cape")[1]).shape=="mesh")
        assert(state.fleet.players[1].body==state.testSeats[1] and state.fleet.players[2].body==state.testSeats[2])
        assert(state.storyCast.cars.kestrel.native==state.testCar and state.job.cash==state.testMoney)
        assert(state.story.step==1 and state.story.action=="enter")
        state.testEditedCape=world.find("west-cape")[1]
        state.testEditedCount=world.count()
    "#).unwrap();
    // An incomplete definition or a duplicate submission must not
    // replace a live scene.
    let error=game.apply_story("return {kind='west-cape-layout', coast=false}").unwrap_err();
    assert!(!error.is_empty());
    assert_eq!(game.apply_story(&moved).unwrap(), "West Cape");
    game.luau(r#"
        assert(world.find("west-cape")[1]==state.testEditedCape and world.count()==state.testEditedCount)
        local copy=get_layout()
        copy.rally.bends[6][1]=1000
        assert(get_layout().rally.bends[6][1]==-746)
    "#).unwrap();
    assert_eq!(game.apply_story(LAYOUT).unwrap(), "West Cape");
    game.tick(&idle);
    game.luau(r#"
        assert(world.get(world.find("west-cape")[1]).shape=="assets/west-cape.glb")
        assert(state.story.run.definition.steps[4].pos==state.testGate)
        assert(state.storyCast.cars.kestrel.native==state.testCar)
    "#).unwrap();
    assert!(game.errors().is_empty(), "{:?}", game.errors());
}

#[test]
fn saving_the_map_source_updates_the_running_round() {
    use std::{fs, time::{SystemTime, UNIX_EPOCH}};
    fn copy_dir(src: &Path, dst: &Path) {
        fs::create_dir_all(dst).unwrap();
        for item in fs::read_dir(src).unwrap() {
            let item = item.unwrap();
            let target = dst.join(item.file_name());
            if item.file_type().unwrap().is_dir() { copy_dir(&item.path(), &target); }
            else { fs::copy(item.path(), target).unwrap(); }
        }
    }
    let src=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir=Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("layout-reload-{}-{stamp}", std::process::id()));
    copy_dir(&src,&dir);
    let mut game=LuaGame::load(&dir, &Overrides {seed:Some(1948),..Default::default()},None).unwrap();
    game.tick(&[Controls::default()]);
    game.luau("state.testCar=state.fleet.vehicles[1].native; state.testSeat=state.fleet.players[1].body").unwrap();
    let source=dir.join("west_cape_layout.module.luau");
    fs::write(&source,LAYOUT.replace("{-738,-52,4}","{-746,-52,4} --[[ edited ]]")).unwrap();
    assert!(game.reload().iter().any(|s| s.contains("main.luau: reloaded")));
    game.tick(&[Controls::default()]);
    game.luau(r#"
        assert(state.city.cape.data.rally.gates[3][1] < -739)
        assert(world.get(world.find("west-cape")[1]).shape=="mesh")
        assert(get_layout().rally.bends[6][1]==-746)
        assert(state.fleet.vehicles[1].native==state.testCar and state.fleet.players[1].body==state.testSeat)
    "#).unwrap();
    fs::write(&source,LAYOUT).unwrap();
    game.reload();
    game.tick(&[Controls::default()]);
    game.luau("assert(world.get(world.find('west-cape')[1]).shape=='assets/west-cape.glb')").unwrap();
    assert!(game.errors().is_empty(),"{:?}",game.errors());
    drop(game);
    fs::remove_dir_all(dir).unwrap();
}
