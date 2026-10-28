//! Levels: every biome builds for a handful of seeds, the walkable band has
//! a start, three clearings and an exit, and the same seed builds the same
//! level twice.

use glam::Vec2;
use spellstorm::biome::BIOMES;
use spellstorm::level::Level;

#[test]
fn every_biome_builds() {
    for b in 0..BIOMES.len() {
        for seed in [1u64, 7, 42, 1234] {
            let l = Level::build(b, seed);
            let name = BIOMES[b].id;
            assert!(l.length() > 60.0, "{name} {seed} length {}", l.length());
            assert_eq!(l.clearings.len(), 3, "{name} {seed}");
            assert!(
                l.bound(l.start) < -0.5,
                "{name} {seed}: start outside ({})",
                l.bound(l.start)
            );
            assert!(l.bound(l.exit_at) < 0.0, "{name} {seed}: exit outside");
            assert!(!l.props.is_empty(), "{name} {seed}: no props");
            assert!(
                l.statics.len() > 10,
                "{name} {seed}: {} statics",
                l.statics.len()
            );
            // The start is not inside a wall or prop.
            let p = l.statics.push_out(l.start, 0.45);
            assert!(p.distance(l.start) < 0.5, "{name} {seed}: start blocked");
            let w = l.walls.len();
            let again = Level::build(b, seed);
            assert_eq!(again.walls.len(), w);
            assert_eq!(again.props.len(), l.props.len());
            for (a, b) in l.props.iter().zip(&again.props) {
                assert_eq!(a.kind, b.kind);
                assert_eq!(a.pos, b.pos);
            }
            println!(
                "{name:10} seed {seed:5}: len {:6.1} props {:4} walls {:4} statics {:4} torches {:3} lamps {:3}",
                l.length(),
                l.props.len(),
                w,
                l.statics.len(),
                l.torches.len(),
                l.lamps.len()
            );
        }
    }
}

#[test]
fn path_queries() {
    let l = Level::build(0, 7);
    let mid = l.point_at(l.length() * 0.5);
    assert!(l.path_dist(mid) < 0.1);
    let run = l.progress_of(mid);
    assert!((run - l.length() * 0.5).abs() < 1.0, "{run}");
    assert!(l.bound(mid) < 0.0);
    assert!(l.bound(Vec2::new(500.0, 0.0)) > 0.0);
}
