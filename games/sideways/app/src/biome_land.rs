//! Stages on the engine's procedural terrain (`--biome NAME`): the ground
//! under the road comes from the shared biome. Sideways lays and banks the
//! gravel road and cuts the streambeds that cross it.
use render::{biome::Biome, terrain_field};
use sideways::stage::{Land, LandLayout};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Sideways' original Finnish stage has its own terrain generator.
pub const FINLAND: &str = "finland";

pub fn choices() -> impl Iterator<Item = &'static str> {
    std::iter::once(FINLAND).chain(render::biome::NAMES)
}

pub fn step(current: &str, delta: i8) -> &'static str {
    let count = render::biome::NAMES.len() + 1;
    let current = choices().position(|name| name == current).unwrap_or(0);
    let next = (current as isize + isize::from(delta)).rem_euclid(count as isize);
    choices().nth(next as usize).expect("wrapped biome index")
}

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
    let layout = if biome.grown_flora() { LandLayout::BiomeFlora } else { LandLayout::Original };
    Ok(make_land(biome, layout))
}

fn named(name: &str) -> Result<Biome, String> {
    Biome::named(name).ok_or_else(|| format!("unknown biome {name}; one of {}", render::biome::NAMES.join(", ")))
}

pub fn land_for_layout(name: &str, layout: LandLayout) -> Result<Land, String> {
    let biome = named(name)?;
    if layout == LandLayout::BiomeFlora && !biome.grown_flora() {
        return Err(format!("{} has no biome-flora stage layout", biome.name));
    }
    Ok(make_land(biome, layout))
}

fn trunk_profile(biome: &Biome) -> sideways::stage::TrunkProfile {
    // The asset cook and driving physics use the same growth rules, age and
    // seed. Grow once per species, then reuse the lower bole at every root.
    let flora = biome.flora;
    let shapes: Vec<_> = flora.trees.iter().filter(|(_, weight)| *weight > 0.0).map(|&(name, _)| {
        let form = render::plant_species::grown(name).expect("grown biome tree");
        let plant = render::plant_growth::grow(&form.rules, form.age, form.seed);
        let mut tip = glam::Vec3::ZERO;
        for stem in plant.segments.iter().filter(|s| s.order == 0) {
            if stem.to.y >= 5.5 {
                tip = stem.from.lerp(stem.to, (5.5 - stem.from.y) / (stem.to.y - stem.from.y));
                break;
            }
            tip = stem.to;
        }
        sideways::stage::TrunkShape { tip: tip.to_array(), radius: plant.trunk_radius() }
    }).collect();
    Arc::new(move |stage, x, z| shapes[crate::plant_population::biome_tree(stage, &flora, x, z)])
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
    let trunk_profile = (layout == LandLayout::BiomeFlora).then(|| trunk_profile(&biome));
    Land { biome: biome.name, layout, ground, trunk_profile, trees, tree_scale, grass, ferns, flowing_water, tint }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdcompat::Pcg;
    use sideways::{stage::Stage, surfaces::Surface};

    #[test]
    fn all_lobby_choices_are_playable_from_the_standalone_menu() {
        let mut name = FINLAND;
        for expected in choices().skip(1).chain(std::iter::once(FINLAND)) {
            let next = step(name, 1);
            assert_eq!(next, expected);
            assert_eq!(step(next, -1), name);
            assert!(select(next).is_ok());
            name = next;
        }
    }

    #[test]
    fn established_rally_stages_keep_their_original_layout() {
        for name in ["temperate", "swiss", "alpine", "lowland", "polder", "tundra"] {
            let land = land(name).unwrap();
            assert_eq!(land.layout, LandLayout::Original, "{name}");
            assert!(land.trunk_profile.is_none());
            assert!(land_for_layout(name, LandLayout::BiomeFlora).is_err());
            assert_eq!(land.tree_scale, 1.0);
            assert_eq!(land.ferns, land.trees.min(1.0));
        }
        for name in ["desert", "norway", "rainforest", "congo", "desert-broom", "norway-willow", "rainforest-dracaena", "desert-grass", "rainforest-fern", "norway-dryas"] {
            assert_eq!(land(name).unwrap().layout, LandLayout::BiomeFlora, "{name}");
        }
    }

    #[test]
    fn native_colliders_take_their_size_and_lean_from_the_rendered_growth_species() {
        use sideways::{builder::{Spot, V3Ser}, physics::Physics};
        for name in ["norway", "rainforest"] {
            let stage=Stage::generate_on(&mut Pcg::new(1),200.0,Some(land(name).unwrap()));
            let biome=Biome::named(name).unwrap();
            let profiles=stage.land.as_ref().unwrap().trunk_profile.as_ref().unwrap();
            let spot=Spot{pos:V3Ser(17.0,8.5,-70.0),scale:1.2,scale_y:1.08,yaw:1.3};
            let foot=glam::Vec3::new(spot.pos.0,spot.pos.1,spot.pos.2);
            let root=crate::plant_population::tree_root(&stage,&biome,foot,spot.yaw,spot.scale);
            let species=biome.species()[root.placement.variant].0;
            let form=render::plant_species::grown(species).unwrap();
            let plant=render::plant_growth::grow(&form.rules,form.age,form.seed);
            let profile=profiles(&stage,foot.x,foot.z);
            let mut physics=Physics::new();
            physics.add_grown_trunk(&spot,profile);
            let (_,collider)=physics.world.colliders.iter().next().unwrap();
            let cylinder=collider.shape().as_cylinder().unwrap();
            assert_eq!(cylinder.radius,plant.trunk_radius()*root.placement.scale);
            let mid=glam::Vec3::from_array(collider.translation().to_array());
            let up=glam::Quat::from_array(collider.rotation().to_array())*glam::Vec3::Y;
            assert!((mid-up*cylinder.half_height).abs_diff_eq(foot,1e-5));
            let top=foot+glam::Quat::from_rotation_y(spot.yaw)*glam::Vec3::from_array(profile.tip)*spot.scale;
            assert!((mid+up*cylinder.half_height).abs_diff_eq(top,1e-5));
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
