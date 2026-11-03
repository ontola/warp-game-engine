//! Bots play one biome for a while over several seeds; a panic shows the seed.
//! `cargo run --release -p spellstorm --example soak [biome] [seconds] [seeds]`
use spellstorm::{Seat, Settings, World};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let b: usize = a.get(1).and_then(|v| v.parse().ok()).unwrap_or(3);
    let secs: u32 = a.get(2).and_then(|v| v.parse().ok()).unwrap_or(180);
    let n: u64 = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(8);
    for seed in 0..n {
        let seats: Vec<Seat> = (0..4).map(Seat::bot).collect();
        let mut w = World::new(&seats, seed, Settings::default());
        w.run.biome_index = b;
        w.enter_level();
        eprintln!("seed {seed}");
        for _ in 0..60 * secs {
            w.step();
        }
        eprintln!("  ok, biome now {} kills {}", w.run.biome_index, w.stat("kills"));
    }
}
