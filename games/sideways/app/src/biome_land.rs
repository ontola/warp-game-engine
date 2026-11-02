//! Stages on the engine's procedural terrain (`--biome NAME`): the ground
//! under the road comes from render::biome's terrain for that biome instead
//! of Sideways' own noise, so one stage generator runs through temperate
//! forest, alpine valleys, polder, tundra or desert. The stage still lays
//! and banks its own road, ledges and streams on that ground.
use render::{biome::Biome, terrain_field};
use sideways::stage::Land;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The ground of biome `name`, each stage in its own part of the world and
/// shifted so its start lies at height 0.
pub fn land(name: &str) -> Result<Land, String> {
    let biome = Biome::named(name).ok_or_else(|| format!("unknown biome {name}; one of {}", render::biome::NAMES.join(", ")))?;
    let mut style = biome.terrain;
    // The stage cuts its own streams and fords; the terrain's dry channels
    // would cross the road without water in them.
    style.rivers = 0.0;
    style.streams = 0.0;
    let bases: Arc<Mutex<HashMap<u32, (f32, f32, f32)>>> = Arc::default();
    // What grows, against the temperate forest Sideways was made for.
    let home = Biome::named("temperate").expect("temperate biome");
    let ratio = |a: f32, b: f32| if b > 0.0 { a / b } else { 1.0 };
    let trees = ratio(biome.tree_density, home.tree_density);
    let grass = ratio(biome.terrain.grass, home.terrain.grass);
    let (g, h) = (biome.vegetation.ground.grass_dry, home.vegetation.ground.grass_dry);
    let tint = [ratio(g[0], h[0]), ratio(g[1], h[1]), ratio(g[2], h[2])];
    let ground = Arc::new(move |stage_seed: u32, x: f64, z: f64| {
        let (ox, oz, base) = *bases.lock().unwrap_or_else(|e| e.into_inner()).entry(stage_seed).or_insert_with(|| {
            // A spot kilometres apart per stage, where the land is open.
            let h = stage_seed.wrapping_mul(0x9e37_79b9);
            let (ox, oz) = ((h & 0xffff) as f32 * 0.5 - 16384.0, (h >> 16) as f32 * 0.5 - 16384.0);
            (ox, oz, terrain_field::eval(ox, oz, 0.0, 1, &style).pos.y)
        });
        (terrain_field::eval(x as f32 + ox, z as f32 + oz, 0.0, 1, &style).pos.y - base) as f64
    });
    Ok(Land { ground, trees, grass, ferns: trees.min(1.0), tint })
}
