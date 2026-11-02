//! The forest density ladder: one recipe tree on flat ground, seen from 5 m
//! to 2.4 km, drawn by each tier of render::recipe_forest in turn (branch
//! source, full assembly as the GPU picks, cards, whole-plant proxy at
//! terrain_fly's and Sideways' settings), measured against the source:
//! covered pixels and their mean linear colour. A second scene times each
//! tier on a 20 x 20 grove. Headless; no game needed.
//!
//! cargo run --release -p render --example forest_ladder -- [OUT_DIR]
//! (default build/screenshots/forest-ladder: table.md, ladder.json and a
//! contact sheet per species). The recipes are copied from terrain_fly and
//! Sideways, as engine examples copy game data.
use glam::{Affine3A, Mat4, Quat, Vec3};
use render::aggregate_lod::Policy;
use render::cluster::{ClusterMesh, Part, Vertex};
use render::plant_recipe::{CrownProfile, FoliageKind, WoodyRecipe};
use render::recipe_forest::{self, Tree};
use render::scene::{GeometryCategory, Heightfield, Material};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

const SPRUCE: WoodyRecipe = WoodyRecipe {
    id: 0x5350_5255_4345, height: 16.0, trunk_radius: 0.22, crown_base: 0.18, crown_radius: 2.9, crown_profile: CrownProfile::Conical,
    tiers: 10, branches_per_tier: 3, branch_rise: 0.04, branch_radius: 0.065, stiffness: 0.72, foliage: FoliageKind::Needle,
    sprays_per_branch: 84, spray_scale: 1.15, spray_axial_bias: 0.40, foliage_colour: [0.065, 0.135, 0.045], wood_colour: [0.16, 0.105, 0.062],
    protected_trunk_height: 5.5,
};
const BIRCH: WoodyRecipe = WoodyRecipe {
    id: 0x4249_5243_48, height: 15.5, trunk_radius: 0.22, crown_base: 0.36, crown_radius: 1.80, crown_profile: CrownProfile::Columnar,
    tiers: 6, branches_per_tier: 5, branch_rise: 0.85, branch_radius: 0.045, stiffness: 0.45, foliage: FoliageKind::Broadleaf,
    sprays_per_branch: 48, spray_scale: 1.00, spray_axial_bias: 0.50, foliage_colour: [0.11, 0.20, 0.055], wood_colour: [0.45, 0.43, 0.36],
    protected_trunk_height: 5.5,
};
const DISTANCES: [f32; 10] = [5., 10., 20., 40., 80., 150., 300., 600., 1200., 2400.];
const W: u32 = 2560;
const H: u32 = 1440;
const FOV_DEG: f32 = 50.0;

#[derive(Clone, Copy, PartialEq)]
enum Tier { Source, Aggregate, Cards, ProxyFly, ProxySideways }
const TIERS: [(Tier, &str); 5] = [(Tier::Source, "source"), (Tier::Aggregate, "full (GPU picks)"), (Tier::Cards, "cards"),
    (Tier::ProxyFly, "proxy 4px/50m"), (Tier::ProxySideways, "proxy 8px/20m")];

fn ground(scene: &mut Scene, half: f32) {
    scene.sky = Some((16, 8, vec![[0.55, 0.7, 0.95, 1.]; 128]));
    let material = scene.add_material(Material { tint: [0.2, 0.18, 0.12], layer: -1, roughness: 1., flags: 0, scale: 1., metal: 0. });
    let vertices = [[-half, 0., half], [half, 0., half], [-half, 0., -half], [half, 0., -half]]
        .map(|pos| Vertex { pos, normal: Vec3::Y.to_array(), uv: [0.; 2] });
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices, indices: &[0, 1, 2, 1, 3, 2], material }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene.heightfield = Heightfield { origin: [-half, -half], cell: half / 50., w: 101, h: 101, heights: vec![0.0; 10201] };
}

/// `trees` drawn by `tier` (None: the empty ground, the background).
fn scene(tier: Option<Tier>, plant: &render::vegetation_asset::CookedPlant, cards: &render::plant_mesh_lod::PlantCards,
    trees: &[Tree], half: f32) -> Result<Scene, String> {
    let mut scene = Scene::default();
    ground(&mut scene, half);
    let Some(tier) = tier else { return Ok(scene) };
    let variants = std::slice::from_ref(plant);
    let mut game = recipe_forest::game_policy();
    if std::env::var("LADDER_MEASURED_CUTS").is_ok() { game.measured_source_cuts = true; }
    if let Some(e) = std::env::var("LADDER_MAX_ERR").ok().and_then(|v| v.parse().ok()) { game.max_error_pixels = e; }
    match tier {
        // The finest voxel error a policy accepts: groups stay source.
        // At most 12 full spruces fit the part-wind budget: in the grove the
        // nearest 12 are full and the rest proxies, as a game would have it.
        Tier::Source => { recipe_forest::plant(&mut scene, variants, trees, trees.len().min(12), Policy { max_error_pixels: 1e-3, ..game })?; }
        Tier::Aggregate => { recipe_forest::plant(&mut scene, variants, trees, trees.len().min(12), game)?; }
        Tier::ProxyFly => { recipe_forest::plant(&mut scene, variants, trees, 0, game)?; }
        Tier::ProxySideways => {
            // LADDER_PROXY=PX,FROM tries another proxy policy in this row.
            let (px, from) = std::env::var("LADDER_PROXY").ok().and_then(|v| v.split_once(',').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
                .unwrap_or((8.0, 20.0));
            recipe_forest::plant(&mut scene, variants, trees, 0, Policy { proxy_pixels: px, proxy_pixels_from: from, ..game })?;
        }
        Tier::Cards => {
            let layer = scene.add_image("ladder cards", cards.atlas.clone());
            let materials: Vec<u32> = cards.materials(layer).into_iter().map(|m| scene.add_material(m)).collect();
            let mesh = scene.add_mesh(&cards.mesh_with_materials(&materials));
            for t in trees {
                scene.add_classified_instance(mesh, Affine3A::from_scale_rotation_translation(Vec3::splat(t.scale),
                    Quat::from_rotation_y(t.yaw), t.pos), GeometryCategory::Trees);
            }
        }
    }
    Ok(scene)
}

fn lighting() -> Lighting {
    let (el, az) = (28f32.to_radians(), 135f32.to_radians());
    let lod = std::env::var("LADDER_LOD_PIXELS").ok().and_then(|v| v.parse().ok()).unwrap_or(Lighting::default().lod_pixels);
    Lighting { taa: 0., sharpen: 0., lod_pixels: lod, sun_dir: Vec3::new(az.sin() * el.cos(), el.sin(), -az.cos() * el.cos()), ..Default::default() }
}

/// The camera `d` metres from the tree, looking at its middle from a little
/// above, the sun at its side.
fn view(d: f32, height: f32) -> ViewRect {
    let target = Vec3::new(0., height * 0.5, 0.);
    let pos = Vec3::new(-d * 0.6, height * 0.5 + d * 0.12, d * 0.8);
    ViewRect { cam: Camera { pos, target, fov_y: FOV_DEG.to_radians(), near: 0.05 }, x: 0, y: 0, w: W, h: H }
}

fn linear(c: u8) -> f32 {
    let v = c as f32 / 255.;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Pixels that differ from the background, their count and mean linear RGB,
/// their bounding box, and their mass: the summed linear luminance change
/// against the background, what a viewer sees of a tree too small to resolve
/// (a faint fringe counts a little, not as a covered pixel).
/// Only pixels inside `region` (x0, y0, x1, y1, inclusive) count.
fn measure(img: &[u8], bg: &[u8], region: [u32; 4]) -> (usize, [f32; 3], [u32; 4], f32) {
    let (mut n, mut sum, mut bbox, mut mass) = (0usize, [0f32; 3], [u32::MAX, u32::MAX, 0, 0], 0f32);
    let lum = |c: &[u8]| 0.2126 * linear(c[0]) + 0.7152 * linear(c[1]) + 0.0722 * linear(c[2]);
    for (i, (p, b)) in img.chunks_exact(4).zip(bg.chunks_exact(4)).enumerate() {
        let (x, y) = (i as u32 % W, i as u32 / W);
        if x < region[0] || x > region[2] || y < region[1] || y > region[3] { continue; }
        mass += (lum(p) - lum(b)).abs();
        let diff = (0..3).map(|k| (p[k] as i32 - b[k] as i32).abs()).max().unwrap();
        if diff <= 6 { continue; }
        n += 1;
        for k in 0..3 { sum[k] += linear(p[k]); }
        bbox = [bbox[0].min(x), bbox[1].min(y), bbox[2].max(x), bbox[3].max(y)];
    }
    (n, if n > 0 { sum.map(|s| s / n as f32) } else { [0.; 3] }, bbox, mass)
}

fn main() -> Result<(), String> {
    // --only TIER[,TIER] (source, full, cards, proxy4, proxy8), --at M[,M],
    // --no-grove, --species spruce|birch: a quicker partial ladder.
    let args: Vec<String> = std::env::args().collect();
    let flag = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let only: Option<Vec<String>> = flag("--only").map(|s| s.split(',').map(String::from).collect());
    let at: Vec<f32> = flag("--at").map(|s| s.split(',').filter_map(|v| v.parse().ok()).collect()).unwrap_or(DISTANCES.to_vec());
    let species_only = flag("--species");
    let tier_key = |t: Tier| match t { Tier::Source => "source", Tier::Aggregate => "full", Tier::Cards => "cards",
        Tier::ProxyFly => "proxy4", Tier::ProxySideways => "proxy8" };
    let wanted = |t: Tier| t == Tier::Source || only.as_ref().is_none_or(|o| o.iter().any(|k| k == tier_key(t)));
    let out = args.iter().skip(1).find(|a| !a.starts_with("--") && args.windows(2).all(|w| !(w[1] == **a && w[0].starts_with("--"))))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/screenshots/forest-ladder"));
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/cook-cache");
    let species = [("spruce", SPRUCE), ("birch", BIRCH)];
    let cooked = recipe_forest::cook_variants(&species.map(|(_, r)| (r, 1)), Some(&cache))?;
    let cards = recipe_forest::cook_cards(&species.map(|(_, r)| (r, 1)), Some(&cache))?;
    let gpu = Gpu::new()?;
    let light = lighting();
    let mut table = String::from("# Forest density ladder\n\nCovered pixels and mean linear colour of covered pixels, each tier against the branch source at the same distance (2560x1440, 50 deg FOV, sun 28 deg, no TAA).\n");
    let mut report = Vec::new();
    for (si, (name, recipe)) in species.iter().enumerate() {
        if species_only.as_ref().is_some_and(|s| s != name) { continue; }
        let tree = [Tree { pos: Vec3::ZERO, yaw: 0.4, scale: 1.0, variant: 0 }];
        let render = |tier: Option<Tier>, d: f32| -> Result<Vec<u8>, String> {
            let s = scene(tier, &cooked[si].plant, &cards[si], &tree, 3000.)?;
            let renderer = Renderer::new(&gpu, &s, &light);
            let v = view(d, recipe.height);
            // A few frames so per-view selections and caches settle.
            for _ in 0..3 { renderer.try_render_views(&gpu, &[v], &light, W, H, &[])?; }
            Ok(renderer.try_render_views(&gpu, &[v], &light, W, H, &[])?.0)
        };
        table += &format!("\n## {name}\n\n| distance | tier | covered px | vs source | mass vs source | mean colour (linear) | colour vs source |\n|---|---|---|---|---|---|---|\n");
        let mut sheet: Vec<Vec<image::RgbaImage>> = vec![Vec::new(); TIERS.len()];
        for &d in &at {
            let bg = render(None, d)?;
            let mut reference = None;
            let mut crop = None;
            // The tree alone, not its shadow: tiers cast differently (a far
            // source may cast none), so only the source's crown and trunk
            // columns above the root count, found from the source below.
            let root = {
                let v = view(d, recipe.height).cam;
                let clip = Mat4::perspective_rh(v.fov_y, W as f32 / H as f32, v.near, 1e5) * Mat4::look_at_rh(v.pos, v.target, Vec3::Y) * Vec3::ZERO.extend(1.);
                ((0.5 - 0.5 * clip.y / clip.w) * H as f32) as u32
            };
            let mut region = [0, 0, W - 1, root.min(H - 1)];
            for (ti, &(tier, label)) in TIERS.iter().enumerate() {
                if !wanted(tier) { continue; }
                let img = render(Some(tier), d)?;
                if tier == Tier::Source {
                    // Columns from the crown (rows clear of the ground), then all rows above the root.
                    let tall = root as f32 - (measure(&img, &bg, [0, 0, W - 1, root.min(H - 1)]).2[1] as f32);
                    let b = measure(&img, &bg, [0, 0, W - 1, (root as f32 - 0.1 * tall).max(0.) as u32]).2;
                    let pad = ((b[2] - b[0]) / 10).max(2);
                    region = [b[0].saturating_sub(pad), 0, (b[2] + pad).min(W - 1), root.min(H - 1)];
                }
                let (n, colour, bbox, mass) = measure(&img, &bg, region);
                if tier == Tier::Source {
                    reference = Some((n, colour, mass));
                    // One crop per distance, around the source tree, square, padded.
                    let (w, h) = (bbox[2].saturating_sub(bbox[0]) + 1, bbox[3].saturating_sub(bbox[1]) + 1);
                    let side = (w.max(h) as f32 * 1.15) as u32 + 8;
                    let cx = (bbox[0] + bbox[2]) / 2;
                    let cy = (bbox[1] + bbox[3]) / 2;
                    let side = side.min(H);
                    crop = Some(((cx.saturating_sub(side / 2)).min(W - side), (cy.saturating_sub(side / 2)).min(H - side), side));
                }
                let (rn, rc, rm) = reference.unwrap();
                let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
                let cover = n as f32 / rn.max(1) as f32;
                let tone = lum(colour) / lum(rc).max(1e-6);
                table += &format!("| {d} m | {label} | {n} | {:.0} % | {:.0} % | {:.3} {:.3} {:.3} | {:.0} % |\n", cover * 100., mass / rm.max(1e-6) * 100., colour[0], colour[1], colour[2], tone * 100.);
                report.push(json!({"species": name, "distance_m": d, "tier": label, "covered_px": n, "cover_vs_source": cover, "mass_vs_source": mass / rm.max(1e-6),
                    "colour": colour, "luminance_vs_source": tone}));
                let (x, y, side) = crop.unwrap();
                let rgba = image::RgbaImage::from_raw(W, H, img).ok_or("image size")?;
                let cell = image::imageops::crop_imm(&rgba, x, y, side, side).to_image();
                sheet[ti].push(image::imageops::resize(&cell, 160, 160, image::imageops::FilterType::Triangle));
            }
            eprintln!("{name} {d} m done");
        }
        // Contact sheet: rows tiers, columns distances.
        let mut out_img = image::RgbaImage::from_pixel(160 * at.len() as u32, 160 * TIERS.len() as u32, image::Rgba([0, 0, 0, 255]));
        for (r, row) in sheet.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() { image::imageops::overlay(&mut out_img, cell, c as i64 * 160, r as i64 * 160); }
        }
        out_img.save(out.join(format!("{name}_ladder.png"))).map_err(|e| e.to_string())?;
    }
    std::fs::write(out.join("table.md"), &table).map_err(|e| e.to_string())?;
    if args.iter().any(|a| a == "--no-grove") { println!("{table}"); return Ok(()); }
    // Perf: a 20 x 20 grove, 8 m apart, seen from 60 m out and 20 m up.
    table += "\n## Grove timing (20 x 20 spruces, 2560x1440, median of 8 frames, ms; full tiers: nearest 12 full, rest proxies)\n\n| tier | GPU total | top passes |\n|---|---|---|\n";
    let mut grove: Vec<Tree> = (0..400).map(|i| Tree { pos: Vec3::new((i % 20) as f32 * 8. - 76., 0., (i / 20) as f32 * 8. - 76.),
        yaw: i as f32 * 0.7, scale: 0.9 + 0.2 * ((i * 37 % 11) as f32 / 10.), variant: 0 }).collect();
    let cam = ViewRect { cam: Camera { pos: Vec3::new(0., 20., 140.), target: Vec3::new(0., 6., 0.), fov_y: FOV_DEG.to_radians(), near: 0.05 },
        x: 0, y: 0, w: W, h: H };
    grove.sort_by(|a, b| a.pos.distance(cam.cam.pos).total_cmp(&b.pos.distance(cam.cam.pos)));
    for &(tier, label) in &TIERS {
        if !wanted(tier) { continue; }
        let s = scene(Some(tier), &cooked[0].plant, &cards[0], &grove, 3000.)?;
        let renderer = Renderer::new(&gpu, &s, &light);
        let mut totals = Vec::new();
        let mut passes: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        for frame in 0..11 {
            let (_, stats) = renderer.try_render_texture_profiled(&gpu, &[cam], &light, W, H)?;
            if frame < 3 { continue; }
            totals.push(stats.gpu_ms.iter().map(|p| p.1).sum::<f64>());
            for (n, ms) in &stats.gpu_ms { passes.entry(n.to_string()).or_default().push(*ms); }
        }
        let median = |v: &mut Vec<f64>| { v.sort_by(f64::total_cmp); v[v.len() / 2] };
        let total = median(&mut totals);
        let mut top: Vec<(String, f64)> = passes.into_iter().map(|(n, mut v)| (n, median(&mut v))).collect();
        top.sort_by(|a, b| b.1.total_cmp(&a.1));
        let top_text = top.iter().take(4).map(|(n, ms)| format!("{n} {ms:.1}")).collect::<Vec<_>>().join(", ");
        table += &format!("| {label} | {total:.1} | {top_text} |\n");
        report.push(json!({"grove_tier": label, "gpu_total_ms": total, "passes": top}));
        eprintln!("grove {label}: {total:.1} ms");
    }
    std::fs::write(out.join("table.md"), &table).map_err(|e| e.to_string())?;
    std::fs::write(out.join("ladder.json"), serde_json::to_string_pretty(&report).unwrap()).map_err(|e| e.to_string())?;
    println!("{table}");
    Ok(())
}
