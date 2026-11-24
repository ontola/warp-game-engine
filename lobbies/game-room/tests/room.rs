//! The Rust-built room matches the Godot Game Room export line for line.
//!
//! Regenerate the fixture after changing room.gd: export with
//! `godot --path lobbies/game-room -- --warp-export=DIR` in gamenight, then
//! `game-room listing --dump DIR > tests/godot_room.txt`.
//! Warp-only change since: the highest ledge (x -1.2..1.2, top 9.2) is gone,
//! so drop its four lines from a fresh export; the couch arms are 2 cm
//! deeper (no z-fighting with the seat).

#[test]
fn the_room_matches_the_godot_export() {
    let mut room = game_room::room::build();
    room.pose(0.0, 0.0, &Default::default());
    let ours = game_room::fixture::listing(&room.dump, &room.bin, &room.hidden);
    let godot = include_str!("godot_room.txt");
    let diff: Vec<String> = godot.lines().zip(ours.lines()).filter(|(a, b)| a != b).take(8).map(|(a, b)| format!("godot {a}\nrust  {b}")).collect();
    assert!(diff.is_empty(), "room differs from Godot:\n{}", diff.join("\n"));
    assert_eq!(godot.lines().count(), ours.lines().count(), "line count differs");
}

#[test]
fn every_screen_has_a_painter() {
    use game_room::paint::{paint, Canvas, Prompt, Show};
    let room = game_room::room::build();
    let prompts = room.prompts.iter().map(|p| (p.name.clone(), Prompt {
        items: p.items.iter().map(|(k, t)| (k.clone(), t.clone(), true)).collect(), enabled: true, ..Default::default() })).collect();
    let mut list = render::hud::HudDrawList::new([render::screen::SIZE; 2]);
    for s in &room.screens {
        let mut canvas = Canvas::new(&mut list, [0, 0, s.size[0], s.size[1]]);
        assert!(paint(&mut canvas, &s.name, &Show::default(), &prompts), "no painter for {}", s.name);
    }
    assert!(!list.text.is_empty());
}
