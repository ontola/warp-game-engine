//! CPU-only comparison of retained pine cards and sparse surface-density LODs.
//! Build explicitly with `cargo build -p cli --release --example foliage_voxel_preview`.
//! No GPU, window, controller, GameNight or game-process API is used. One thread.

use glam::{Vec2, Vec3};
use image::{Rgba, RgbaImage};
use render::cluster::{Part, Vertex, read_kmesh};
use render::foliage_voxel::{BakeConfig, VoxelAsset, VoxelLevel, bake};
use render::scene::{ALPHA_TEST, Material, TWO_SIDED, TextureSource};
use serde_json::json;
use std::{collections::HashMap, path::{Path, PathBuf}, time::Instant};

type Pixel = [f32; 4]; // Premultiplied linear RGB and covered fraction.

struct Args { out: PathBuf, repo: PathBuf, variant: String, width: u32, height: u32, ss: u32, cell: f32 }

fn args() -> Result<Args, String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.is_empty() || raw.iter().any(|a| a == "--help") {
        return Err("usage: foliage_voxel_preview --out FRESH_DIRECTORY [--repo REPO] [--variant a|b|c] [--size 256x384] [--supersample 2] [--cell 0.25]".into());
    }
    if raw.len() % 2 != 0 { return Err("expected --key value pairs".into()); }
    let mut map = HashMap::new();
    for pair in raw.chunks_exact(2) {
        if !["--out", "--repo", "--variant", "--size", "--supersample", "--cell"].contains(&pair[0].as_str()) || map.insert(pair[0].as_str(), pair[1].as_str()).is_some() {
            return Err(format!("unknown or repeated option {}", pair[0]));
        }
    }
    let size = map.get("--size").copied().unwrap_or("256x384");
    let (w, h) = size.split_once('x').ok_or("size must be WIDTHxHEIGHT")?;
    let width = w.parse::<u32>().map_err(|e| e.to_string())?;
    let height = h.parse::<u32>().map_err(|e| e.to_string())?;
    let ss = map.get("--supersample").unwrap_or(&"2").parse::<u32>().map_err(|e| e.to_string())?;
    let cell = map.get("--cell").unwrap_or(&"0.25").parse::<f32>().map_err(|e| e.to_string())?;
    let variant = map.get("--variant").copied().unwrap_or("a").to_string();
    if !(64..=512).contains(&width) || !(64..=768).contains(&height) || !(1..=2).contains(&ss)
        || !cell.is_finite() || !(0.125..=1.0).contains(&cell) || !["a", "b", "c"].contains(&variant.as_str()) {
        return Err("bounded preview requires 64..512 x 64..768 pixels, 1..2 supersamples per axis, cell 0.125..1 m, and variant a/b/c".into());
    }
    Ok(Args { out: PathBuf::from(map.get("--out").ok_or("--out is required")?),
        repo: map.get("--repo").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap().to_path_buf()),
        variant, width, height, ss, cell })
}

fn linear(x: u8) -> f32 { let x = x as f32 / 255.0; if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) } }
fn srgb(x: f32) -> u8 { let x = x.clamp(0.0, 1.0); ((if x <= 0.0031308 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }) * 255.0 + 0.5) as u8 }
fn fingerprint(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3));
    format!("fnv1a64:{hash:016x}")
}

struct Camera { centre: Vec3, right: Vec3, up: Vec3, dir: Vec3, distance: f32, span: [f32; 2], size: [u32; 2] }
impl Camera {
    fn new(bounds: [[f32; 3]; 2], azimuth: f32, width: u32, height: u32) -> Self {
        let lo = Vec3::from(bounds[0]); let hi = Vec3::from(bounds[1]); let centre = (lo + hi) * 0.5;
        let az = azimuth.to_radians(); let el = 8f32.to_radians();
        let dir = -Vec3::new(az.sin() * el.cos(), el.sin(), az.cos() * el.cos());
        let right = dir.cross(Vec3::Y).normalize(); let up = right.cross(dir);
        let half = (hi - lo) * 0.5;
        let projected_w = 2.0 * right.abs().dot(half); let projected_h = 2.0 * up.abs().dot(half);
        let aspect = width as f32 / height as f32;
        let h = projected_h.max(projected_w / aspect) * 1.10;
        Self { centre, right, up, dir, distance: (hi - lo).length() + 10.0, span: [h * aspect, h], size: [width, height] }
    }
    fn project(&self, point: Vec3) -> Vec3 {
        let d = point - self.centre;
        Vec3::new((d.dot(self.right) / self.span[0] + 0.5) * self.size[0] as f32,
            (0.5 - d.dot(self.up) / self.span[1]) * self.size[1] as f32, d.dot(self.dir) + self.distance)
    }
    fn ray(&self, x: u32, y: u32) -> Vec3 {
        self.centre - self.dir * self.distance
            + self.right * (((x as f32 + 0.5) / self.size[0] as f32 - 0.5) * self.span[0])
            + self.up * ((0.5 - (y as f32 + 0.5) / self.size[1] as f32) * self.span[1])
    }
}

fn texture_sample(texture: &RgbaImage, uv: Vec2) -> Pixel {
    // The source-card reference uses base-level bilinear sampling plus 2x2
    // image supersampling, not an assertion of exact native anisotropic mips.
    let at = (uv - uv.floor()) * Vec2::new(texture.width() as f32, texture.height() as f32) - Vec2::splat(0.5);
    let base = at.floor(); let f = at - base; let mut out = [0.0; 4];
    for y in 0..2 { for x in 0..2 {
        let pixel = texture.get_pixel((base.x as i64 + x).rem_euclid(texture.width() as i64) as u32,
            (base.y as i64 + y).rem_euclid(texture.height() as i64) as u32);
        let w = (if x == 0 { 1.0 - f.x } else { f.x }) * (if y == 0 { 1.0 - f.y } else { f.y });
        for k in 0..3 { out[k] += linear(pixel[k]) * w; }
        out[3] += pixel[3] as f32 / 255.0 * w;
    }}
    out
}

fn source_cards(vertices: &[Vertex], indices: &[u32], texture: &RgbaImage, material: Material, camera: &Camera, cut: f32) -> Vec<Pixel> {
    let [w, h] = camera.size; let count = (w * h) as usize;
    let mut pixels = vec![[0.0; 4]; count]; let mut depths = vec![f32::INFINITY; count];
    for triangle in indices.chunks_exact(3) {
        let v: [Vertex; 3] = std::array::from_fn(|k| vertices[triangle[k] as usize]);
        let p = v.map(|v| camera.project(Vec3::from(v.pos)));
        let e1 = p[1] - p[0]; let e2 = p[2] - p[0];
        let determinant = e1.x * e2.y - e1.y * e2.x;
        if determinant.abs() < 1e-8 { continue; }
        let lo = p[0].min(p[1]).min(p[2]); let hi = p[0].max(p[1]).max(p[2]);
        let x0 = lo.x.floor().max(0.0) as u32; let x1 = (hi.x.ceil().max(0.0) as u32).min(w);
        let y0 = lo.y.floor().max(0.0) as u32; let y1 = (hi.y.ceil().max(0.0) as u32).min(h);
        for y in y0..y1 { for x in x0..x1 {
            let d = Vec2::new(x as f32 + 0.5 - p[0].x, y as f32 + 0.5 - p[0].y);
            let b1 = (d.x * e2.y - d.y * e2.x) / determinant;
            let b2 = (e1.x * d.y - e1.y * d.x) / determinant;
            if b1 < 0.0 || b2 < 0.0 || b1 + b2 > 1.0 { continue; }
            let b0 = 1.0 - b1 - b2; let depth = p[0].z * b0 + p[1].z * b1 + p[2].z * b2;
            let index = (y * w + x) as usize;
            if depth >= depths[index] { continue; }
            let uv = Vec2::from(v[0].uv) * b0 + Vec2::from(v[1].uv) * b1 + Vec2::from(v[2].uv) * b2;
            let sample = texture_sample(texture, uv);
            if sample[3] < cut { continue; }
            pixels[index] = [sample[0] * material.tint[0], sample[1] * material.tint[1], sample[2] * material.tint[2], 1.0];
            depths[index] = depth;
        }}
    }
    pixels
}

fn ray_box(origin: Vec3, dir: Vec3, lo: Vec3, hi: Vec3) -> Option<(f32, f32)> {
    let mut enter = 0.0f32; let mut exit = f32::INFINITY;
    for axis in 0..3 {
        if dir[axis].abs() < 1e-8 {
            if origin[axis] < lo[axis] || origin[axis] > hi[axis] { return None; }
        } else {
            let a = (lo[axis] - origin[axis]) / dir[axis]; let b = (hi[axis] - origin[axis]) / dir[axis];
            enter = enter.max(a.min(b)); exit = exit.min(a.max(b));
        }
    }
    (exit > enter).then_some((enter, exit))
}

fn volume(asset: &VoxelAsset, level: &VoxelLevel, camera: &Camera) -> Vec<Pixel> {
    let lookup: HashMap<[u32; 3], usize> = level.voxels.iter().enumerate().map(|(i, v)| (v.coord, i)).collect();
    let bounds = level.local_bounds(asset.origin); let lo = Vec3::from(bounds[0]); let hi = Vec3::from(bounds[1]);
    let cell = level.cell_size; let [w, h] = camera.size;
    let mut pixels = vec![[0.0; 4]; (w * h) as usize];
    let density: Vec<f32> = level.voxels.iter().map(|v| Vec3::from(v.projected_density(cell)).dot(camera.dir.abs())).collect();
    for y in 0..h { for x in 0..w {
        let origin = camera.ray(x, y);
        let Some((mut t, exit)) = ray_box(origin, camera.dir, lo, hi) else { continue; };
        // Exact cell-boundary DDA: opacity integrates over the actual segment
        // in each sparse cell; it is independent of an arbitrary ray step size.
        let mut coord = ((origin + camera.dir * (t + cell * 1e-4) - lo) / cell).floor().as_ivec3().to_array();
        let mut next = [f32::INFINITY; 3]; let mut delta = [f32::INFINITY; 3]; let mut sign = [0i32; 3];
        for a in 0..3 {
            coord[a] = coord[a].clamp(0, level.dimensions[a] as i32 - 1);
            if camera.dir[a].abs() > 1e-8 {
                sign[a] = if camera.dir[a] > 0.0 { 1 } else { -1 };
                let boundary = lo[a] + (coord[a] + if sign[a] > 0 { 1 } else { 0 }) as f32 * cell;
                next[a] = (boundary - origin[a]) / camera.dir[a]; delta[a] = cell / camera.dir[a].abs();
            }
        }
        let mut color = Vec3::ZERO; let mut transmittance = 1.0f32;
        let max_steps = level.dimensions.iter().sum::<u32>() + 3;
        for _ in 0..max_steps {
            let end = next[0].min(next[1]).min(next[2]).min(exit);
            if let Some(&i) = lookup.get(&coord.map(|c| c as u32)) {
                let alpha = 1.0 - (-density[i] * (end - t).max(0.0)).exp();
                color += Vec3::from(level.voxels[i].albedo) * (transmittance * alpha);
                transmittance *= 1.0 - alpha;
            }
            if end >= exit || transmittance < 1e-4 { break; }
            for a in 0..3 { if next[a] <= end + cell * 1e-5 { coord[a] += sign[a]; next[a] += delta[a]; } }
            if (0..3).any(|a| coord[a] < 0 || coord[a] >= level.dimensions[a] as i32) { break; }
            t = end;
        }
        pixels[(y * w + x) as usize] = [color.x, color.y, color.z, 1.0 - transmittance];
    }}
    pixels
}

fn resolve(pixels: &[Pixel], width: u32, height: u32, ss: u32) -> Vec<Pixel> {
    let mut out = vec![[0.0; 4]; (width * height) as usize];
    for y in 0..height { for x in 0..width { for sy in 0..ss { for sx in 0..ss {
        let src = pixels[((y * ss + sy) * width * ss + x * ss + sx) as usize];
        for k in 0..4 { out[(y * width + x) as usize][k] += src[k] / (ss * ss) as f32; }
    }}}}
    out
}

fn glyph(c: char) -> [u8; 7] {
    match c {
        'A'=>[14,17,17,31,17,17,17], 'B'=>[30,17,17,30,17,17,30], 'C'=>[14,17,16,16,16,17,14],
        'D'=>[30,17,17,17,17,17,30], 'E'=>[31,16,16,30,16,16,31], 'F'=>[31,16,16,30,16,16,16],
        'G'=>[14,17,16,23,17,17,15], 'H'=>[17,17,17,31,17,17,17], 'I'=>[31,4,4,4,4,4,31],
        'J'=>[7,2,2,2,18,18,12], 'K'=>[17,18,20,24,20,18,17], 'L'=>[16,16,16,16,16,16,31],
        'M'=>[17,27,21,21,17,17,17], 'N'=>[17,25,25,21,19,19,17], 'O'=>[14,17,17,17,17,17,14],
        'P'=>[30,17,17,30,16,16,16], 'Q'=>[14,17,17,17,21,18,13], 'R'=>[30,17,17,30,20,18,17],
        'S'=>[15,16,16,14,1,1,30], 'T'=>[31,4,4,4,4,4,4], 'U'=>[17,17,17,17,17,17,14],
        'V'=>[17,17,17,17,17,10,4], 'W'=>[17,17,17,21,21,21,10], 'X'=>[17,17,10,4,10,17,17],
        'Y'=>[17,17,10,4,4,4,4], 'Z'=>[31,1,2,4,8,16,31],
        '0'=>[14,17,19,21,25,17,14], '1'=>[4,12,4,4,4,4,14], '2'=>[14,17,1,2,4,8,31],
        '3'=>[30,1,1,14,1,1,30], '4'=>[2,6,10,18,31,2,2], '5'=>[31,16,16,30,1,1,30],
        '6'=>[14,16,16,30,17,17,14], '7'=>[31,1,2,4,8,8,8], '8'=>[14,17,17,14,17,17,14],
        '9'=>[14,17,17,15,1,1,14], '.'=>[0,0,0,0,0,6,6], '-'=>[0,0,0,31,0,0,0], _=>[0;7],
    }
}
fn label(image: &mut RgbaImage, x: u32, y: u32, text: &str) {
    for (i, c) in text.chars().enumerate() { for (gy, row) in glyph(c).iter().enumerate() { for gx in 0..5 {
        if row & (1 << (4 - gx)) != 0 { for dy in 0..2 { for dx in 0..2 {
            let px = x + i as u32 * 12 + gx * 2 + dx; let py = y + gy as u32 * 2 + dy;
            if px < image.width() && py < image.height() { image.put_pixel(px, py, Rgba([230,235,241,255])); }
        }}}
    }}}
}

fn paste(image: &mut RgbaImage, pixels: &[Pixel], offset: [u32; 2], size: [u32; 2], mask: bool) {
    for y in 0..size[1] { for x in 0..size[0] {
        let pixel = pixels[(y * size[0] + x) as usize];
        let c = if mask { [pixel[3]; 3] } else {
            let bg = if (x / 16 + y / 16) % 2 == 0 { 0.22 } else { 0.18 };
            [pixel[0] + bg * (1.0 - pixel[3]), pixel[1] + bg * (1.0 - pixel[3]), pixel[2] + bg * (1.0 - pixel[3])]
        };
        image.put_pixel(offset[0] + x, offset[1] + y, Rgba([srgb(c[0]), srgb(c[1]), srgb(c[2]), 255]));
    }}
}

fn run(a: Args) -> Result<(), String> {
    let started = Instant::now();
    if a.out.exists() { return Err(format!("refusing to overwrite existing output {}", a.out.display())); }
    let mesh_path = a.repo.join(format!("engine/assets/fir_{}_near.kmesh", a.variant));
    let texture_path = a.repo.join("assets/trees/authored_pine_albedo.png");
    let mesh_bytes = std::fs::read(&mesh_path).map_err(|e| e.to_string())?;
    let texture_bytes = std::fs::read(&texture_path).map_err(|e| e.to_string())?;
    let surfaces = read_kmesh(&mesh_bytes)?;
    let mut vertices = Vec::new(); let mut indices = Vec::new(); let mut selected = Vec::new();
    for (name, verts, idx) in surfaces {
        // Exact same material selection as visuals.rs: non-wood surfaces are
        // the retained authored crown. Never substitute a synthetic ellipsoid.
        if name.contains("trunk") || name.contains("bark") { continue; }
        selected.push(json!({"material_name":name,"vertices":verts.len(),"triangles":idx.len()/3}));
        let base = vertices.len() as u32; indices.extend(idx.into_iter().map(|i| i + base)); vertices.extend(verts);
    }
    if indices.is_empty() { return Err("retained mesh contains no crown surfaces".into()); }
    let texture = TextureSource { name: texture_path.display().to_string(), rgba: image::load_from_memory(&texture_bytes).map_err(|e| e.to_string())?.to_rgba8(), normal: None };
    let material = Material { tint: [0.62,0.66,0.55], layer: 0, roughness: 0.9, flags: ALPHA_TEST | TWO_SIDED, scale: 1.0, metal: 0.0 };
    let config = BakeConfig { cell_size: a.cell, levels: 3, ..Default::default() };
    let begin = Instant::now();
    let asset = bake(&[Part { vertices: &vertices, indices: &indices, material: 0 }], &[material], std::slice::from_ref(&texture), config)?;
    let bake_ms = begin.elapsed().as_secs_f64() * 1000.0;
    let bounds = asset.levels.last().unwrap().local_bounds(asset.origin);
    let gap = 8; let row_height = a.height + 52; let mut atlas = RgbaImage::from_pixel(4 * (a.width + gap) + gap, 3 * row_height + gap, Rgba([19,24,30,255]));
    let mut masks = atlas.clone(); let mut comparisons = Vec::new();
    for (row, azimuth) in [0.0f32, 60.0, 120.0].into_iter().enumerate() {
        let camera = Camera::new(bounds, azimuth, a.width * a.ss, a.height * a.ss);
        let begin = Instant::now();
        let source = resolve(&source_cards(&vertices, &indices, &texture.rgba, material, &camera, config.alpha_cut), a.width, a.height, a.ss);
        let source_ms = begin.elapsed().as_secs_f64() * 1000.0;
        let source_coverage: f64 = source.iter().map(|p| p[3] as f64).sum();
        let top = gap + row as u32 * row_height;
        for image in [&mut atlas, &mut masks] { label(image, gap, top, &format!("AZ {azimuth:.0} DEG - UNLIT CPU")); label(image, gap, top + 22, "SOURCE CARDS"); }
        paste(&mut atlas, &source, [gap, top + 44], [a.width, a.height], false);
        paste(&mut masks, &source, [gap, top + 44], [a.width, a.height], true);
        let mut levels = Vec::new();
        for (i, level) in asset.levels.iter().enumerate() {
            let begin = Instant::now(); let rendered = resolve(&volume(&asset, level, &camera), a.width, a.height, a.ss);
            let render_ms = begin.elapsed().as_secs_f64() * 1000.0;
            let coverage: f64 = rendered.iter().map(|p| p[3] as f64).sum();
            let mut squared = 0.0; let mut absolute = 0.0; let mut intersection = 0usize; let mut union = 0usize;
            for (actual, expected) in rendered.iter().zip(&source) {
                let d = actual[3] as f64 - expected[3] as f64; squared += d * d; absolute += d.abs();
                intersection += (actual[3] >= 0.5 && expected[3] >= 0.5) as usize;
                union += (actual[3] >= 0.5 || expected[3] >= 0.5) as usize;
            }
            let left = gap + (i as u32 + 1) * (a.width + gap);
            for image in [&mut atlas, &mut masks] { label(image, left, top + 22, &format!("VOXEL {:.2}M", level.cell_size)); }
            paste(&mut atlas, &rendered, [left, top + 44], [a.width, a.height], false);
            paste(&mut masks, &rendered, [left, top + 44], [a.width, a.height], true);
            levels.push(json!({"level":i,"cell_m":level.cell_size,"raymarch_ms":render_ms,"coverage_pixels":coverage,
                "coverage_ratio_to_cards":coverage/source_coverage.max(1e-12),"alpha_rmse":(squared/source.len() as f64).sqrt(),
                "alpha_mae":absolute/source.len() as f64,"silhouette_iou_at_half":intersection as f64/union.max(1) as f64}));
        }
        comparisons.push(json!({"azimuth_degrees":azimuth,"elevation_degrees":8,"camera_centre":camera.centre.to_array(),
            "camera_direction":camera.dir.to_array(),"camera_span_m":camera.span,"camera_distance":camera.distance,"source_raster_ms":source_ms,
            "source_coverage_pixels":source_coverage,"levels":levels}));
    }
    if let Some(parent) = a.out.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    std::fs::create_dir(&a.out).map_err(|e| e.to_string())?;
    atlas.save(a.out.join("comparison-albedo.png")).map_err(|e| e.to_string())?;
    masks.save(a.out.join("comparison-coverage.png")).map_err(|e| e.to_string())?;
    let mass: Vec<_> = asset.levels.iter().enumerate().map(|(i, level)| {
        let surface: f64 = level.voxels.iter().map(|v| v.surface_area as f64).sum();
        let projected: [f64; 3] = std::array::from_fn(|k| level.voxels.iter().map(|v| v.projected_area[k] as f64).sum());
        json!({"level":i,"cell_m":level.cell_size,"dimensions":level.dimensions,"sparse_cells":level.voxels.len(),
            "surface_area_m2":surface,"projected_area_m2":projected,"cell_data_bytes":level.voxels.len()*std::mem::size_of::<render::foliage_voxel::Voxel>()})
    }).collect();
    let report = json!({"schema":1,"kind":"original CPU sparse foliage prototype; not native gameplay or GPU performance",
        "source":{"mesh":mesh_path,"mesh_fingerprint":fingerprint(&mesh_bytes),"texture":texture_path,"texture_fingerprint":fingerprint(&texture_bytes),
            "selected_surfaces":selected,"excluded":"trunk and bark surfaces","tint_linear":material.tint,"alpha_cut":config.alpha_cut},
        "render":{"panel_size":[a.width,a.height],"supersamples_per_axis":a.ss,"cpu_threads":1,"gpu_used":false,
            "color":"unlit linear albedo, premultiplied integration, sRGB PNG","ray_method":"exact sparse cell DDA; Beer-Lambert extinction from absolute-axis projected area"},
        "bake":{"ms":bake_ms,"triangles":asset.stats.triangles,"samples":asset.stats.samples,"prepared_texture_texels":asset.stats.prepared_texture_texels,
            "max_samples":config.max_samples,"max_voxels":config.max_voxels,"origin":asset.origin,"source_bounds":asset.source_bounds,"levels":mass},
        "comparisons":comparisons,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,
        "limitations":["Surface/projected area conservation does not guarantee equal image opacity or silhouette.",
            "Three absolute-axis projected areas approximate diagonal extinction and may overestimate it.",
            "Source cards use base-level bilinear alpha then cutoff plus image supersampling. Bake thresholds before coverage mip filtering; these are deliberately reported as separate methods.",
            "No lighting, normal maps, wind, animation, shadows, GPU upload, runtime LOD switching, or performance qualification.",
            "Coarse cells intentionally spread coverage across empty local space; block edges remain visible in this unfiltered first raymarch prototype."]});
    std::fs::write(a.out.join("report.json"), serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    println!("CPU foliage preview: {} (bake {:.1} ms; total {:.1} ms)", a.out.display(), bake_ms, started.elapsed().as_secs_f64()*1000.0);
    Ok(())
}

fn main() { if let Err(e) = args().and_then(run) { eprintln!("{e}"); std::process::exit(1); } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slab_intersection_handles_parallel_empty_and_inside_rays() {
        assert_eq!(ray_box(Vec3::new(-2.0,0.5,0.5),Vec3::X,Vec3::ZERO,Vec3::ONE),Some((2.0,3.0)));
        assert_eq!(ray_box(Vec3::new(-2.0,2.0,0.5),Vec3::X,Vec3::ZERO,Vec3::ONE),None);
        assert_eq!(ray_box(Vec3::splat(0.5),Vec3::X,Vec3::ZERO,Vec3::ONE),Some((0.0,0.5)));
    }
    #[test]
    fn supersampling_resolves_premultiplied_color_and_coverage_together() {
        let out=resolve(&[[0.4,0.2,0.1,1.0],[0.0;4],[0.0;4],[0.0;4]],1,1,2);
        assert_eq!(out,vec![[0.1,0.05,0.025,0.25]]);
    }
}
