//! The Rust-built room matches the Godot Game Room export line for line.
//!
//! Regenerate the fixture after changing room.gd: export with
//! `godot --path lobbies/game-room -- --warp-export=DIR` in gamenight, then
//! `game-room listing --dump DIR > tests/godot_room.txt`.

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
