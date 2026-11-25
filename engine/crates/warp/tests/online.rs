//! Online couches over loopback, without a GPU: a follower mirrors the host's
//! world exactly (to the wire's millimetres), its people sit in the host's
//! roster and their pads move them there.
use app::{Buttons, Controls, Game};
use std::path::PathBuf;
use warp::{LuaGame, Overrides};

fn game(name: &str) -> LuaGame {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../games").join(name);
    let mut g = LuaGame::load(&dir, &Overrides::default(), None).unwrap();
    g.echo = false;
    g
}

fn run(name: &str, ticks: u32) -> (LuaGame, LuaGame) {
    let (mut host, mut follower) = (game(name), game(name));
    let ticket = host.host_online(true).unwrap();
    follower.follow_online(&ticket, true, [1280, 720]).unwrap();
    let push = Controls { mv: [1.0, 0.0], held: Buttons(Buttons::SOUTH), ..Default::default() };
    let (hp, fp) = (host.local_players(), follower.local_players());
    for _ in 0..ticks {
        host.tick(&vec![Controls::default(); hp]);
        std::thread::sleep(std::time::Duration::from_millis(1));
        follower.tick(&vec![push; fp]);
    }
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        follower.tick(&vec![push; fp]);
    }
    (host, follower)
}

#[test]
fn a_follower_mirrors_neon_siege() {
    let (host, follower) = run("neon-siege", 240);
    assert_eq!(follower.mirror_errors(&host), (0, 0), "{}", follower.online_state());
    assert!(!host.core().world.entities.is_empty());
    let online: Vec<_> = host.core().roster.iter().filter(|p| p.remote.is_some()).map(|p| p.id).collect();
    assert_eq!(online.len(), 1, "the follower's person sits in the host's roster");
    assert_eq!(follower.online_state()["players"], serde_json::json!(online));
    // Only this couch's pads are gathered on the host.
    assert_eq!(host.local_players(), 1);
    assert!(follower.errors().is_empty() && host.errors().is_empty(), "{:?} {:?}", host.errors(), follower.errors());
}

#[test]
fn a_follower_leaving_frees_its_seat() {
    let (mut host, follower) = run("hello-warp", 30);
    assert_eq!(host.core().roster.iter().filter(|p| p.remote.is_some()).count(), 1);
    drop(follower);
    for _ in 0..200 {
        host.tick(&[Controls::default()]);
        if host.core().roster.iter().all(|p| p.remote.is_none()) { return; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the follower's person is still seated");
}
