//! Stages on the engine's procedural terrain (`--biome NAME`): the ground
//! under the road comes from the shared biome. Sideways lays and banks the
//! gravel road and cuts the streambeds that cross it.
use render::{biome::Biome, terrain_field};
use sideways::stage::{Land, LandLayout};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Sideways' original Finnish stage has its own terrain generator.
pub const FINLAND: &str = "finland";

pub fn select(name: &str) -> Result<Option<Land>, String> {
    if name == FINLAND { Ok(None) } else { land(name).map(Some) }
}

pub fn from_args(args: &[String]) -> Result<Option<Land>, String> {
    if let Some(i) = args.iter().position(|arg| arg == "--biome") {
        return select(args.get(i + 1).ok_or("--biome needs a name")?);
    }
    select(&std::env::var("SIDEWAYS_BIOME").unwrap_or_else(|_| FINLAND.into()))
}

/// The ground of biome `name`, each stage in its own part of the world and
/// shifted so its start lies at height 0.
pub fn land(name: &str) -> Result<Land, String> {
    let biome = named(name)?;
    let layout = match biome.name {
        "desert" | "norway" | "rainforest" => LandLayout::BiomeFlora,
        _ => LandLayout::Original,
    };
    Ok(make_land(biome, layout))
}

fn named(name: &str) -> Result<Biome, String> {
    Biome::named(name).ok_or_else(|| format!("unknown biome {name}; one of {}", render::biome::NAMES.join(", ")))
}

pub fn land_for_layout(name: &str, layout: LandLayout) -> Result<Land, String> {
    Ok(make_land(named(name)?, layout))
}

fn make_land(biome: Biome, layout: LandLayout) -> Land {
    let mut style = biome.terrain;
    // The stage cuts its own streams and fords; the terrain's dry channels
    // would cross the road without water in them.
    style.rivers = 0.0;
    style.streams = 0.0;
    let bases: Arc<Mutex<HashMap<u32, (f32, f32, f32)>>> = Arc::default();
    // What grows, against the temperate forest Sideways was made for.
    let home = Biome::named("temperate").expect("temperate biome");
    let ratio = |a: f32, b: f32| if b > 0.0 { a / b } else { 1.0 };
    let trees = ratio(biome.tree_density, home.tree_density).powi(2);
    let grass = ratio(biome.terrain.grass, home.terrain.grass);
    // The recipe renderer plants shrubs at Sideways' fern habitat sites.
    // Dune scrub survives even when the climate supports no trees or grass.
    let (ferns, tree_scale, flowing_water) = match layout {
        LandLayout::Original => (trees.min(1.0), 1.0, true),
        LandLayout::BiomeFlora => (
            ratio(biome.flora.shrub_density, home.flora.shrub_density).min(1.0),
            biome.flora.tree_scale,
            biome.terrain.rivers > 0.0 || biome.terrain.streams > 0.0,
        ),
    };
    let (g, h) = (biome.vegetation.ground.grass_dry, home.vegetation.ground.grass_dry);
    // Mild and clamped: the tint also falls on Sideways' soil and verge,
    // where a full ratio turned them pink.
    let tint = [0, 1, 2].map(|i| (1.0 + 0.6 * (ratio(g[i], h[i]) - 1.0)).clamp(0.7, 1.35));
    let ground = Arc::new(move |stage_seed: u32, x: f64, z: f64| {
        let (ox, oz, base) = *bases.lock().unwrap_or_else(|e| e.into_inner()).entry(stage_seed).or_insert_with(|| {
            // A spot kilometres apart per stage, where the land is open.
            let h = stage_seed.wrapping_mul(0x9e37_79b9);
            let (ox, oz) = ((h & 0xffff) as f32 * 0.5 - 16384.0, (h >> 16) as f32 * 0.5 - 16384.0);
            (ox, oz, terrain_field::eval(ox, oz, 0.0, 1, &style).pos.y)
        });
        (terrain_field::eval(x as f32 + ox, z as f32 + oz, 0.0, 1, &style).pos.y - base) as f64
    });
    Land { biome: biome.name, layout, ground, trees, tree_scale, grass, ferns, flowing_water, tint }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdcompat::Pcg;
    use sideways::{stage::Stage, surfaces::Surface};

    #[test]
    fn established_rally_stages_keep_their_original_layout() {
        for name in ["temperate", "swiss", "alpine", "lowland", "polder", "tundra"] {
            let land = land(name).unwrap();
            assert_eq!(land.layout, LandLayout::Original, "{name}");
            assert_eq!(land.tree_scale, 1.0);
            assert_eq!(land.ferns, land.trees.min(1.0));
        }
        for name in ["desert", "norway", "rainforest", "congo"] {
            assert_eq!(land(name).unwrap().layout, LandLayout::BiomeFlora, "{name}");
        }
    }

    #[test]
    fn desert_streambeds_keep_the_seeded_road_and_stay_dry() {
        let dry = land("desert").unwrap();
        assert_eq!((dry.trees, dry.grass), (0.0, 0.0));
        assert!(dry.ferns > 0.0);
        let wet = land_for_layout("desert", LandLayout::Original).unwrap();
        assert_eq!(wet.ferns, 0.0);
        let dry = Stage::generate_on(&mut Pcg::new(1), 3140.0, Some(dry));
        let wet = Stage::generate_on(&mut Pcg::new(1), 3140.0, Some(wet));
        assert_eq!(dry.points, wet.points);
        assert!(!dry.rivers.is_empty());
        for river in &dry.rivers {
            let p = dry.points[river.cross];
            assert_eq!(dry.road_surface_at(river.cross as f64 * sideways::stage::STEP), Surface::Gravel);
            assert!(wet.water_level(p.x as f64, p.z as f64).is_finite());
            assert_eq!(dry.water_level(p.x as f64, p.z as f64), f64::NEG_INFINITY);
        }
    }
}
