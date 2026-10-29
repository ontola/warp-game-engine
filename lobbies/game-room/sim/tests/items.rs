use game_room_sim::world::{Body, Kind, World};

/// A bomb rolled along the floor: it lands, rolls (spin matching its speed)
/// and comes to rest; a box (blaster) slides to a stop lying flat.
#[test]
fn a_thrown_bomb_bounces_rolls_and_stops() {
    let mut w = World::new(7);
    let b = w.spawn_item(Kind::Bomb, -6.0, 2.0);
    w.items[b].vx = 9.0;
    w.items[b].vy = 5.0;
    let mut bounced = false;
    let mut rolled = false;
    for _ in 0..240 {
        w.step();
        let item = &w.items[b];
        if item.dead { break; }
        if item.vy > 1.0 && item.y < 1.0 { bounced = true; }
        let r = Body::of(Kind::Bomb).radius;
        if item.y == 0.0 && item.vx > 1.0 && (item.vx + item.av * r).abs() < 0.1 { rolled = true; }
    }
    assert!(bounced, "the bomb bounces when it lands");
    assert!(rolled, "then it rolls without slipping");
    // Fuse aside (a spawned bomb has none), it has come to rest.
    let item = &w.items[b];
    assert!(item.vx.abs() < 0.5, "slowed down: {}", item.vx);

    let g = w.spawn_item(Kind::Blaster, 6.0, 2.0);
    w.items[g].vx = -8.0;
    w.items[g].av = 11.0;
    for _ in 0..180 { w.step(); }
    let item = &w.items[g];
    assert_eq!(item.vx, 0.0);
    let flat = (item.spin / std::f64::consts::PI).round() * std::f64::consts::PI;
    assert!((item.spin - flat).abs() < 0.01, "lies flat: {}", item.spin);
}
