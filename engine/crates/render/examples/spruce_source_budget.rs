//! How many source clusters one grown spruce books per camera distance, and
//! how large its finest branch voxels are there (1440p, 70 degree vfov):
//! cargo run --release -p render --example spruce_source_budget -- [age]
use render::plant_growth::Species;
use render::plant_growth_assembly::{Look, Parts};

fn main() -> Result<(), String> {
    let age: u32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(42);
    let (species, look) = (Species::norway_spruce(), Look::norway_spruce());
    let (plant, ms) = render::recipe_forest::cook_grown_stages(species, age, 1, &Parts::for_look(&look), look)?;
    println!("cooked age {age} in {:.0} ms", ms.iter().sum::<f64>());
    let mut scene = render::scene::Scene::default();
    let placement = render::foliage_runtime::RootPlacement { up: glam::Vec3::Y, pos: glam::Vec3::ZERO, yaw: 0., scale: 1., variant: 0 };
    let policy = render::aggregate_lod::Policy { max_error_pixels: 1., allow_sampled_optics: true, measured_source_cuts: false, proxy_pixels: 1., proxy_pixels_from: 0., ..Default::default() };
    render::foliage_runtime::plant(&mut scene, std::slice::from_ref(&plant), &[placement], 8, policy)?;
    let spec = scene.gpu_forest.as_ref().unwrap();
    let sources: Vec<usize> = (0..spec.nodes.len()).filter(|&i| spec.nodes[i].links[3] >> 8 != 0).collect();
    let full: u64 = sources.iter().map(|&i| (spec.nodes[i].links[3] >> 8) as u64).sum();
    println!("nodes {} source nodes {} full DAG clusters {full}", spec.nodes.len(), sources.len());
    // Finest voxel per source branch: the voxel node right before it in its chain.
    let mut cells = Vec::new();
    for &i in &sources {
        if let Some(v) = (0..i).rev().find(|&j| spec.nodes[j].links[1] == spec.nodes[i].links[1] && spec.nodes[j].links[3] >> 8 == 0 && spec.nodes[j].links[0] != u32::MAX) {
            let n = &spec.nodes[v];
            cells.push(n.error[0] * (-(((n.links[3] >> 3) & 31) as f32)).exp2());
        }
    }
    cells.sort_by(f32::total_cmp);
    if !cells.is_empty() { println!("finest branch voxel cell m: min {:.3} median {:.3} max {:.3} (n {})", cells[0], cells[cells.len() / 2], cells[cells.len() - 1], cells.len()); }
    let ppm_at = |d: f32| 1440.0 / (2.0 * (35f32.to_radians()).tan() * d);
    println!("dist m | px/m | booked clusters (table) | median cell px");
    for d in [5.0f32, 10., 20., 30., 45., 60., 90., 120.] {
        let ppm = ppm_at(d);
        let t = 0.5 / ppm;
        use render::foliage_pages::source::{unpack_source_table, SOURCE_TABLE_FULL, SOURCE_TABLE_T0};
        let k = if t < SOURCE_TABLE_T0 { None } else { Some(((2.0 * (t / SOURCE_TABLE_T0).log2()).floor() as usize).min(23)) };
        let booked: u64 = sources.iter().map(|&i| { let full = spec.nodes[i].links[3] >> 8; match k { None => full as u64, Some(k) => { let v = unpack_source_table(&spec.source_tables[i])[k]; if v == SOURCE_TABLE_FULL { full as u64 } else { v.min(full) as u64 } } } }).sum();
        let mid = cells.get(cells.len() / 2).copied().unwrap_or(0.0);
        let (mut drawn, mut tris) = (0u64, 0u64);
        for inst in scene.instances.iter().filter(|i| i.cluster_count > 0) {
            for c in &scene.clusters[inst.first_cluster as usize..(inst.first_cluster + inst.cluster_count) as usize] {
                let tc = 1.0 / ppm;
                if c.own_error * inst.scale <= tc && c.parent_error * inst.scale > tc { drawn += 1; tris += c.tri_count as u64; }
            }
        }
        println!("{d:5.0} | {ppm:6.1} | {booked:8} | {:.1} | drawn clusters {drawn} tris {tris}", mid * ppm);
    }
    Ok(())
}
