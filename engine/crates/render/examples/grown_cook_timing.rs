//! Where a grown plant's full cook spends its time, per age:
//! cargo run --release -p render --example grown_cook_timing -- [tuning.json] [ages...]
//! tuning.json is a pine-lab file ({"species": {...}, "look": {...}}); missing
//! fields keep the Jeffrey pine defaults.
use render::plant_growth::Species;
use render::plant_growth_assembly::{Look, Parts};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (tuning, ages): (Option<&String>, Vec<u32>) = match args.first() {
        Some(a) if a.ends_with(".json") => (Some(a), args[1..].iter().filter_map(|a| a.parse().ok()).collect()),
        _ => (None, args.iter().filter_map(|a| a.parse().ok()).collect()),
    };
    let ages = if ages.is_empty() { vec![5, 15, 30, 55] } else { ages };
    let (mut species, mut look) = (Species::jeffrey_pine(), Look::jeffrey_pine());
    if let Some(path) = tuning {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if let Some(s) = v.get("species") { species = serde_json::from_value(s.clone()).map_err(|e| e.to_string())?; }
        if let Some(l) = v.get("look") { look = serde_json::from_value(l.clone()).map_err(|e| e.to_string())?; }
    }
    let parts = Parts::for_look(&look);
    println!("{}", if tuning.is_some() { "tuning file" } else { "defaults" });
    for age in ages {
        let (plant, ms) = render::recipe_forest::cook_grown_stages(species, age, 1, &parts, look)?;
        let s = plant.stats();
        let levels: Vec<usize> = plant.proxy().map(|p| p.levels.iter().map(|l| l.voxels.len()).collect()).unwrap_or_default();
        println!("age {age:3}: grow {:7.0} ms  assemble {:5.0} ms  aggregate {:8.0} ms  proxy {:7.0} ms  | placements {:6}  branches {:2}  groups {:4}  volume assets {:4}  source tris {:9}  expanded tris {:9}  proxy voxels per level {levels:?}",
            ms[0], ms[1], ms[2], ms[3], s.placements, s.branches, s.geometry_groups, s.volume_assets + s.geometry_volume_assets, s.primitive_triangles, s.expanded_source_triangles);
    }
    Ok(())
}
