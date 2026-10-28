use growing_guns::{Event, World};
fn main() {
    for seed in 0..6u64 {
        let mut w = World::new(seed, &[true; 4]);
        let (mut shots, mut hits, mut heads, mut kills, mut lava, mut rounds, mut jumps, mut dashes) = (0, 0, 0, 0, 0, 0, 0, 0);
        let mut round_ticks = Vec::new(); let mut start = 0;
        for _ in 0..60 * 300 {
            w.step(&[]);
            for e in &w.events { match e {
                Event::Shot{..} => shots += 1, Event::Hit{head,..} => { hits += 1; if *head { heads += 1 } },
                Event::Kill{attacker: Some(_),..} => kills += 1, Event::Kill{attacker: None,..} => lava += 1,
                Event::RoundOver{..} => { rounds += 1; round_ticks.push(w.tick - start); }, Event::NewArena => start = w.tick,
                Event::Jump{..} => jumps += 1, Event::Dash{..} => dashes += 1, _ => {} } }
        }
        let avg = round_ticks.iter().sum::<u64>() as f32 / round_ticks.len().max(1) as f32 / 60.0;
        println!("seed {seed}: size {} {:?} rounds {rounds} avg {avg:.1}s shots {shots} hits {hits} ({:.0}%) heads {heads} kills {kills} lava {lava} jumps {jumps} dashes {dashes} scores {:?}",
            w.arena.size, w.arena.variant, 100.0 * hits as f32 / shots.max(1) as f32, w.players.iter().map(|p| p.score).collect::<Vec<_>>());
    }
}
