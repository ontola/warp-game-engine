use game_room_sim::world::*;

#[test]
fn b_lifts_a_player_standing_a_little_higher() {
    let mut w = World::new(1);
    w.ensure_player("a", 0);
    w.ensure_player("b", 1);
    for (id, x, y) in [("a", 3.7, 0.0), ("b", 3.0, 0.9)] {
        let p = w.player_mut(id).unwrap();
        p.x = x; p.y = y; p.spawn = 0.0;
    }
    for _ in 0..30 { w.set_input("a", 0, [0.0; 2]); w.set_input("b", 0, [0.0; 2]); w.step(); }
    let (a, b) = (w.player("a").unwrap(), w.player("b").unwrap());
    assert!(a.y.abs() < 0.01 && (b.y - 0.9).abs() < 0.01, "a on the floor, b on the TV console: {} {}", a.y, b.y);
    assert_eq!(w.carry_target(w.index("a").unwrap()), w.index("b"));
}
