//! How strong each tome is in play: three bots run the first level with
//! every wizard holding the tome from the start, over several seeds, against
//! a run without one. Prints damage dealt, damage taken and how far they got.
//! `cargo run --release -p spellstorm --example tome_power [seconds] [seeds]`

use kern::Sim;
use spellstorm::{relics, Seat, Settings, World};
use std::collections::HashMap;

struct Out {
    dealt: f64,
    taken: f64,
    far: f32,
    kills: u64,
}

fn play(tome: Option<usize>, seed: u64, secs: u32) -> Out {
    let seats: Vec<Seat> = (0..3).map(Seat::bot).collect();
    let mut w = World::new(&seats, seed, Settings::default());
    if let Some(t) = tome {
        for id in w.wizards.clone() {
            w.give_relic(id, t);
        }
    }
    let mut hp: HashMap<u64, f64> = HashMap::new();
    let (mut dealt, mut taken, mut far) = (0.0, 0.0, 0.0f32);
    for _ in 0..60 * secs {
        w.step();
        for &id in w.enemies.iter().chain(&w.corpses) {
            let Some(a) = w.actor(id) else { continue };
            let k = id.to_bits().get();
            let was = *hp.entry(k).or_insert(a.max_hp);
            if a.hp < was {
                dealt += was - a.hp.max(0.0);
            }
            hp.insert(k, a.hp.max(0.0));
        }
        for &id in &w.wizards {
            let a = w.a(id);
            let k = id.to_bits().get();
            let was = *hp.entry(k).or_insert(a.hp);
            if a.hp < was {
                taken += was - a.hp.max(0.0);
            }
            hp.insert(k, a.hp.max(0.0));
        }
        far = far.max(w.lead_progress());
    }
    Out { dealt, taken, far, kills: w.stat("kills") }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let secs: u32 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(90);
    let seeds: u64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(6);
    let all = relics::all();
    let mut jobs: Vec<Option<usize>> = vec![None];
    jobs.extend((0..all.len()).map(Some));
    let results: Vec<(Option<usize>, Out)> = std::thread::scope(|s| {
        let hs: Vec<_> = jobs
            .iter()
            .map(|&t| {
                s.spawn(move || {
                    let mut sum = Out { dealt: 0.0, taken: 0.0, far: 0.0, kills: 0 };
                    for seed in 0..seeds {
                        let o = play(t, 100 + seed, secs);
                        sum.dealt += o.dealt;
                        sum.taken += o.taken;
                        sum.far += o.far;
                        sum.kills += o.kills;
                    }
                    (t, sum)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let base = &results[0].1;
    println!("{:<22} {:>7} {:>7} {:>6} {:>6}  {:>6}", "tome", "dealt", "taken", "far", "kills", "score");
    for (t, o) in &results {
        let name = t.map(|i| all[i].name.clone()).unwrap_or("(none)".into());
        // Score: damage out per damage in, against the run without a tome.
        let score = (o.dealt / base.dealt) * (base.taken / o.taken.max(1.0)).sqrt();
        println!(
            "{:<22} {:>7.2} {:>7.2} {:>6.2} {:>6.2}  {:>6.2}",
            name,
            o.dealt / base.dealt,
            o.taken / base.taken,
            o.far / base.far,
            o.kills as f64 / base.kills.max(1) as f64,
            score
        );
    }
}
