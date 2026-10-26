//! Explicit CPU-only source-clump bake/preview. Never starts a GPU or game.
//! `cargo run -p cli --release --example grass_voxel_bake -- --out FRESH_DIR`
#![allow(dead_code)]
#[path = "../src/ground_cover.rs"] mod ground_cover;
#[path = "../src/live_ground_cover.rs"] mod live_ground_cover;
#[path = "../src/grass_voxel_asset.rs"] mod grass_voxel_asset;

use glam::{Vec2, Vec3};
use image::{Rgba, RgbaImage};
use render::{foliage_voxel::{VoxelAsset, VoxelLevel}, foliage_volume_data::{self, PackLimits}};
use serde_json::{json, Value};
use sideways::{builder, stage::{Stage, CHUNK, RUNOUT}, vegetation::{Season, Vegetation}};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, time::Instant};

type Pixel = [f32; 4]; // Premultiplied linear raw photo RGB and opacity.
fn linear(v: u8) -> f32 { let x = v as f32 / 255.0; if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) } }
fn srgb(v: f32) -> u8 { let x = v.clamp(0.0, 1.0); ((if x <= 0.0031308 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }) * 255.0 + 0.5) as u8 }
fn fingerprint(bytes: impl IntoIterator<Item = u8>) -> String {
    let hash = bytes.into_iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("fnv1a64:{hash:016x}")
}

struct Camera { root: Vec3, direction: Vec3, right: Vec3, up: Vec3, span: Vec2, size: [u32; 2] }
impl Camera {
    fn new(azimuth: f32) -> Self {
        let az = azimuth.to_radians(); let elevation = 8f32.to_radians();
        let direction = -Vec3::new(az.sin() * elevation.cos(), elevation.sin(), az.cos() * elevation.cos());
        let right = direction.cross(Vec3::Y).normalize(); let up = right.cross(direction);
        Self { root: Vec3::Y * 0.25 - direction * 2.0, direction, right, up,
            span: Vec2::new(0.86, 0.7166667), size: [192, 160] }
    }
    fn ray(&self, x: u32, y: u32, sx: f32, sy: f32) -> Vec3 {
        self.root + self.right * ((x as f32 + sx) / self.size[0] as f32 - 0.5) * self.span.x
            + self.up * (0.5 - (y as f32 + sy) / self.size[1] as f32) * self.span.y
    }
}
fn texture_sample(texture: &RgbaImage, uv: Vec2) -> Pixel {
    let at = uv.clamp(Vec2::ZERO, Vec2::ONE) * Vec2::new(texture.width() as f32, texture.height() as f32) - Vec2::splat(0.5);
    let base = at.floor(); let f = at - base; let mut out = [0.0; 4];
    for y in 0..2 { for x in 0..2 {
        let p = texture.get_pixel((base.x as i64 + x).clamp(0, texture.width() as i64 - 1) as u32,
            (base.y as i64 + y).clamp(0, texture.height() as i64 - 1) as u32);
        let weight = (if x == 0 { 1.0 - f.x } else { f.x }) * (if y == 0 { 1.0 - f.y } else { f.y });
        for k in 0..3 { out[k] += linear(p[k]) * weight; } out[3] += p[3] as f32 / 255.0 * weight;
    }} out
}
fn source_ray(asset: &grass_voxel_asset::ClumpAsset, origin: Vec3, direction: Vec3) -> Pixel {
    let mut depth = f32::INFINITY; let mut result = [0.0; 4];
    for triangle in asset.indices.chunks_exact(3) {
        let v = [asset.vertices[triangle[0] as usize], asset.vertices[triangle[1] as usize], asset.vertices[triangle[2] as usize]];
        let p = v.map(|v| Vec3::from(v.pos)); let e1 = p[1] - p[0]; let e2 = p[2] - p[0];
        let cross = direction.cross(e2); let det = e1.dot(cross);
        if det.abs() < 1e-8 { continue; }
        let s = origin - p[0]; let u = s.dot(cross) / det; let q = s.cross(e1); let w = direction.dot(q) / det;
        if u < 0.0 || w < 0.0 || u + w > 1.0 { continue; }
        let distance = e2.dot(q) / det; if distance < 0.0 || distance >= depth { continue; }
        let uv = Vec2::from(v[0].uv) * (1.0 - u - w) + Vec2::from(v[1].uv) * u + Vec2::from(v[2].uv) * w;
        let sample = texture_sample(&asset.photo, uv);
        if sample[3] >= 0.5 { result = [sample[0], sample[1], sample[2], 1.0]; depth = distance; }
    } result
}
fn ray_box(origin: Vec3, direction: Vec3, lo: Vec3, hi: Vec3) -> Option<(f32, f32)> {
    let mut enter = 0.0f32; let mut exit = f32::INFINITY;
    for k in 0..3 {
        if direction[k].abs() < 1e-8 { if origin[k] < lo[k] || origin[k] > hi[k] { return None; } }
        else { let a = (lo[k] - origin[k]) / direction[k]; let b = (hi[k] - origin[k]) / direction[k]; enter = enter.max(a.min(b)); exit = exit.min(a.max(b)); }
    } (exit > enter).then_some((enter, exit))
}
struct Volume<'a> { level: &'a VoxelLevel, origin: Vec3, index: Vec<u32>, sigma: Vec<f32> }
impl<'a> Volume<'a> {
    fn new(asset: &VoxelAsset, level: &'a VoxelLevel, direction: Vec3) -> Self {
        let mut index = vec![0; level.dimensions.iter().map(|v| *v as usize).product()];
        for (i, voxel) in level.voxels.iter().enumerate() {
            let [x, y, z] = voxel.coord; let [w, h, _] = level.dimensions;
            index[(x + w * (y + h * z)) as usize] = i as u32 + 1;
        }
        Self { level, origin: Vec3::from(asset.origin), index,
            sigma: level.voxels.iter().map(|v| v.density_along(level.cell_size, direction.to_array())).collect() }
    }
    fn ray(&self, origin: Vec3, direction: Vec3) -> Pixel {
        let cell = self.level.cell_size; let dims = self.level.dimensions;
        let hi = self.origin + Vec3::from(dims.map(|v| v as f32)) * cell;
        let Some((mut t, exit)) = ray_box(origin, direction, self.origin, hi) else { return [0.0; 4]; };
        let mut coord = ((origin + direction * (t + cell * 1e-4) - self.origin) / cell).floor().as_ivec3().to_array();
        let mut next = [f32::INFINITY; 3]; let mut delta = [f32::INFINITY; 3]; let mut step = [0; 3];
        for k in 0..3 {
            coord[k] = coord[k].clamp(0, dims[k] as i32 - 1);
            if direction[k].abs() > 1e-8 {
                step[k] = if direction[k] > 0.0 { 1 } else { -1 };
                let edge = self.origin[k] + (coord[k] + if step[k] > 0 { 1 } else { 0 }) as f32 * cell;
                next[k] = (edge - origin[k]) / direction[k]; delta[k] = cell / direction[k].abs();
            }
        }
        let mut result = [0.0; 4]; let mut transmittance = 1.0;
        for _ in 0..dims.iter().sum::<u32>() + 3 {
            if coord.iter().enumerate().any(|(k, v)| *v < 0 || *v >= dims[k] as i32) || t >= exit { break; }
            let end = next[0].min(next[1]).min(next[2]).min(exit);
            let slot = self.index[(coord[0] as u32 + dims[0] * (coord[1] as u32 + dims[1] * coord[2] as u32)) as usize];
            if slot > 0 {
                let i = slot as usize - 1; let alpha = 1.0 - (-self.sigma[i] * (end - t).max(0.0)).exp();
                for k in 0..3 { result[k] += transmittance * alpha * self.level.voxels[i].albedo[k]; }
                transmittance *= 1.0 - alpha;
            }
            for k in 0..3 { if next[k] <= end + 1e-6 { coord[k] += step[k]; next[k] += delta[k]; } }
            t = end;
        }
        result[3] = 1.0 - transmittance; result
    }
}
fn image_for(camera: &Camera, sample: impl Fn(Vec3, Vec3) -> Pixel) -> Vec<Pixel> {
    let mut result = Vec::with_capacity((camera.size[0] * camera.size[1]) as usize);
    for y in 0..camera.size[1] { for x in 0..camera.size[0] {
        let mut pixel = [0.0; 4];
        for sy in [0.25, 0.75] { for sx in [0.25, 0.75] {
            let sample = sample(camera.ray(x, y, sx, sy), camera.direction);
            for k in 0..4 { pixel[k] += sample[k] * 0.25; }
        }} result.push(pixel);
    }} result
}
fn filtered_surface_image(surface:&grass_voxel_asset::SurfaceClump,camera:&Camera,samples:u32,mip_scale:f32)->Vec<Pixel> {
    let dx=camera.right*camera.span.x/camera.size[0]as f32/samples as f32*mip_scale;
    let dy=-camera.up*camera.span.y/camera.size[1]as f32/samples as f32*mip_scale;
    let mut image=Vec::new();
    for y in 0..camera.size[1] {for x in 0..camera.size[0] {
        let mut p=[0.0;4];
        for sy in 0..samples {for sx in 0..samples {
            let v=surface.ray(camera.ray(x,y,(sx as f32+0.5)/samples as f32,(sy as f32+0.5)/samples as f32),camera.direction,dx,dy);
            for k in 0..4 {p[k]+=v[k]/(samples*samples)as f32;}
        }}image.push(p);
    }} image
}
fn opacity_metrics(pixels:&[Pixel],reference:&[Pixel])->Value {
    let coverage: f64=pixels.iter().map(|p|p[3]as f64).sum();let target: f64=reference.iter().map(|p|p[3]as f64).sum();
    let mae=pixels.iter().zip(reference).map(|(a,b)|(a[3]-b[3]).abs()as f64).sum::<f64>()/pixels.len()as f64;
    let rgb_mae=pixels.iter().zip(reference).map(|(a,b)|(0..3).map(|k|(a[k]-b[k]).abs()as f64).sum::<f64>()/3.0).sum::<f64>()/pixels.len()as f64;
    json!({"opacity_pixels":coverage,"reference_opacity_pixels":target,"opacity_ratio":coverage/target,"opacity_mae":mae,"premultiplied_rgb_mae":rgb_mae})
}
fn projected_preview(asset:&grass_voxel_asset::ClumpAsset,out:&Path)->Result<Value,String> {
    let surface=grass_voxel_asset::SurfaceClump::from_source(&asset.vertices,&asset.photo)?;
    let mut sheet=RgbaImage::new(32*4,32*45);let mut rows=Vec::new();let mut row=0;
    for (view_height,mip_scale,label) in [(2160u32,1.0,"native_output"),(1440,2.0/3.0,"quality_input_with_output_mip_bias"),(1076,1.0,"native_four_way_panel")] {for distance in [40.0f32,55.0,73.0,100.0,111.0] {for azimuth in [0.0,15.0,45.0] {
        let pixel_m=2.0*distance*(64f32.to_radians()*0.5).tan()/view_height as f32;
        let mut camera=Camera::new(azimuth);camera.size=[32,32];camera.span=Vec2::splat(pixel_m*32.0);
        // Numerical source integration resolves subpixel overlap correlation.
        // Each subray still uses coverage mips for the remaining footprint.
        let reference=filtered_surface_image(&surface,&camera,16,1.0);
        let single=filtered_surface_image(&surface,&camera,1,mip_scale);
        let quad=filtered_surface_image(&surface,&camera,2,mip_scale);
        let level=asset.volume.levels.iter().rposition(|l|l.cell_size/pixel_m<=0.5).unwrap_or(0);
        let volume=Volume::new(&asset.volume,&asset.volume.levels[level],camera.direction);
        // Match the reference box integration, avoiding a lucky central ray.
        let mut medium=Vec::new();
        for y in 0..32 {for x in 0..32 {let mut p=[0.0;4];
            for sy in 0..16 {for sx in 0..16 {let v=volume.ray(camera.ray(x,y,(sx as f32+0.5)/16.0,(sy as f32+0.5)/16.0),camera.direction);for k in 0..4 {p[k]+=v[k]/256.0;}}}
            medium.push(p);
        }}
        let convergence=if view_height==2160&&distance==73.0&&azimuth==15.0 {
            Some(opacity_metrics(&reference,&filtered_surface_image(&surface,&camera,32,1.0)))
        }else{None};
        let metrics=[("source_16x16",&reference),("medium",&medium),("surface_one_sample",&single),("surface_2x2",&quad)]
            .into_iter().enumerate().map(|(column,(name,pixels))|{
                for y in 0..32 {for x in 0..32 {let p=pixels[(y*32+x)as usize];sheet.put_pixel(column as u32*32+x,row*32+y,
                    Rgba([srgb(p[0]+0.035*(1.0-p[3])),srgb(p[1]+0.035*(1.0-p[3])),srgb(p[2]+0.035*(1.0-p[3])),255]));}}
                json!({"column":column,"name":name,"metrics":opacity_metrics(pixels,&reference)})
            }).collect::<Vec<_>>();
        rows.push(json!({"row":row,"mode":label,"view_height":view_height,"mip_derivative_scale":mip_scale,"distance_m":distance,"azimuth_degrees":azimuth,"pixel_world_m":pixel_m,
            "tuft_height_pixels":ground_cover::HEIGHT/pixel_m,"volume_level":level,"voxel_pixel_width":asset.volume.levels[level].cell_size/pixel_m,"columns":metrics,"source_16_vs_32_convergence":convergence}));row+=1;
    }}}
    sheet.save(out.join("projected-source-medium-surface.png")).map_err(|e|e.to_string())?;
    image::imageops::resize(&sheet,sheet.width()*4,sheet.height()*4,image::imageops::FilterType::Nearest)
        .save(out.join("projected-source-medium-surface-zoom4.png")).map_err(|e|e.to_string())?;
    Ok(json!({"files":["projected-source-medium-surface.png","projected-source-medium-surface-zoom4.png"],"rows":rows,
        "projection":"64-degree vertical FOV; orthographic local approximation at40/55/73/100/111m; 2160px output,1440px Quality input with2/3 derivative mip bias, or1076px four-way4K panel; fixed physical source .5m height. Screenshot r15d flat grass measured73-111m; transition53-57m",
        "source":"16x16 numerical pixel integration of actual source planes with alpha>=.5 base mask and area-preserving premultiplied-linear coverage mips; anisotropic up to8taps; one case compared against32x32",
        "candidate":"Surface coverage attenuates at the actual three source sheets using T*=1-C, not Beer-Lambert. One-sample and2x2 variants quantify subpixel overlap-correlation error. This hybrid is NOT a completed volume renderer or runtime change.",
        "limitations":["Coverage-moment mips are physically filtered source coverage, not the runtime globally alpha-rescaled threshold mip chain; no exact production AA match claimed","1440px case uses actual2/3 derivative mip bias for candidates; source oracle integrates the full input pixel. No temporal jitter/DLSS reconstruction is simulated","Local orthographic camera, no perspective derivative variation across a single tuft","Single plant comparison; inter-plant overlap, wind, motion vectors, shadows and cost still need actual renderer validation","Area-density volume keeps original data, no global extinction multiplier or darkening"]}))
}
fn preview(asset: &grass_voxel_asset::ClumpAsset, out: &Path) -> Result<Value, String> {
    let mut sheet = RgbaImage::new(192 * 5, 160 * 3); let mut rows = Vec::new();
    for (row, azimuth) in [0.0, 15.0, 45.0].into_iter().enumerate() {
        let camera = Camera::new(azimuth);
        let source = image_for(&camera, |o, d| source_ray(asset, o, d));
        let mut comparisons = Vec::new();
        for column in 0..5 {
            let pixels = if column == 0 { source.clone() } else {
                let volume = Volume::new(&asset.volume, &asset.volume.levels[column - 1], camera.direction);
                image_for(&camera, |o, d| volume.ray(o, d))
            };
            for y in 0..160 { for x in 0..192 {
                let p = pixels[(y * 192 + x) as usize];
                sheet.put_pixel(column as u32 * 192 + x, row as u32 * 160 + y,
                    Rgba([srgb(p[0] + 0.035 * (1.0 - p[3])), srgb(p[1] + 0.035 * (1.0 - p[3])), srgb(p[2] + 0.035 * (1.0 - p[3])), 255]));
            }}
            let alpha: f64 = pixels.iter().map(|p| p[3] as f64).sum();
            let source_alpha: f64 = source.iter().map(|p| p[3] as f64).sum();
            let mae: f64 = pixels.iter().zip(&source).map(|(a, b)| (a[3] - b[3]).abs() as f64).sum::<f64>() / pixels.len() as f64;
            comparisons.push(json!({"column":column,"level":if column==0{None}else{Some(column-1)},
                "summed_opacity_pixels":alpha,"source_opacity_pixels":source_alpha,"opacity_ratio":alpha/source_alpha,"opacity_mae":mae}));
        }
        rows.push(json!({"row":row,"azimuth_degrees":azimuth,"elevation_degrees":8,"columns":comparisons}));
    }
    sheet.save(out.join("source-and-voxel-lods.png")).map_err(|e| e.to_string())?;
    Ok(json!({"file":"source-and-voxel-lods.png","tile_size":[192,160],"rows":rows,
        "method":"Unlit raw linear photo albedo, 2x2 image supersampling. Source: clamped bilinear LOD0 plus alpha>=0.5. Volume: exact-cell DDA and directional Beer-Lambert; no lighting, filtering, wind, transition or GPU.",
        "limitation":"Source thresholds after bilinear; baker thresholds before coverage mips. Mass conservation does not guarantee projected opacity/silhouette equality. Coarser cells spread mass; this comparison reports that error rather than hiding it."}))
}
fn write_packed(asset: &VoxelAsset, out: &Path) -> Result<Value, String> {
    let packed = foliage_volume_data::pack(std::slice::from_ref(asset), &[], &[], PackLimits {
        max_assets: 1, levels_per_asset: asset.levels.len(), max_dense_indices: 2_000_000,
        max_sparse_cells: 65_536, ..Default::default() })?;
    let mut bytes = Vec::new();
    for v in [0x504d4c43u32, 1, packed.levels.len() as u32, packed.indices.len() as u32, packed.cells.len() as u32] { bytes.extend(v.to_le_bytes()); }
    for l in &packed.levels { for v in l.origin_cell { bytes.extend(v.to_le_bytes()); } for v in l.dims_offset { bytes.extend(v.to_le_bytes()); } }
    for v in &packed.indices { bytes.extend(v.to_le_bytes()); }
    for cell in &packed.cells {
        for v in cell.albedo_rough.into_iter().chain(cell.extinction.into_iter().flatten()).chain(cell.normals.into_iter().flatten()) { bytes.extend(v.to_le_bytes()); }
    }
    fs::write(out.join("clump-volume.bin"), &bytes).map_err(|e| e.to_string())?;
    Ok(json!({"file":"clump-volume.bin","bytes":bytes.len(),"fingerprint":fingerprint(bytes),
        "encoding":"little endian: u32 magic CLMP, version=1, level_count, index_count, cell_count; level records 32B; u32 dense indices (0 empty, cell+1 otherwise); sparse cells 112B exactly foliage_volume_data::GpuCell. No per-instance volume copies.",
        "dense_indices":packed.indices.len(),"sparse_cells":packed.cells.len(),"gpu_resources_created":false}))
}
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() % 2 != 0 || args.is_empty() { return Err("usage: grass_voxel_bake --out FRESH_DIR [--repo REPO] [--cell 0.02] [--seed 1] [--at 300] [--growth 1] [--season summer]".into()); }
    let mut options = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        if !["--out","--repo","--cell","--seed","--at","--growth","--season"].contains(&pair[0].as_str()) || options.insert(pair[0].as_str(), pair[1].as_str()).is_some() { return Err(format!("unknown/repeated option {}",pair[0])); }
    }
    let out = PathBuf::from(*options.get("--out").ok_or("--out is required")?);
    if out.exists() { return Err("output directory must be fresh".into()); }
    let repo = options.get("--repo").map(sideways::game_assets::diagnostic_root).unwrap_or_else(sideways::game_assets::root);
    let cell: f32 = options.get("--cell").unwrap_or(&"0.02").parse::<f32>().map_err(|e|e.to_string())?;
    let seed = options.get("--seed").unwrap_or(&"1").parse::<u64>().map_err(|e|e.to_string())?;
    let at = options.get("--at").unwrap_or(&"300").parse::<f64>().map_err(|e|e.to_string())?;
    if !at.is_finite() || !(0.0..=2800.0).contains(&at) { return Err("--at must be finite and within 0..2800 m".into()); }
    let growth = options.get("--growth").unwrap_or(&"1").parse::<f32>().map_err(|e|e.to_string())?;
    let season = options.get("--season").unwrap_or(&"summer").parse::<Season>()?;
    let vegetation = Vegetation::new(seed, growth, season)?;
    let started = Instant::now(); let asset = grass_voxel_asset::bake_clump(&repo, cell)?;
    let bake_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut rng = gdcompat::Pcg::new(seed); rng.next_u32(); let mut stage = Stage::generate(&mut rng, 3000.0 + RUNOUT); stage.vegetation = vegetation;
    let p = stage.point_at(at); let dir = stage.dir_at(at);
    let centre = Vec2::new(p.x, p.z) + Vec2::new(-dir.z, dir.x) * (stage.width_at(at) as f32 + 6.0);
    let base = (centre / live_ground_cover::CELL).floor().as_ivec2().to_array();
    let min = [base[0] - 10, base[1] - 10]; let max = [base[0] + 10, base[1] + 10];
    let low = (Vec2::from(min.map(|v|v as f32)) * live_ground_cover::CELL - Vec2::ONE) / CHUNK as f32;
    let high = (Vec2::from(max.map(|v|v as f32)) * live_ground_cover::CELL + Vec2::ONE) / CHUNK as f32;
    let mut chunks = Vec::new(); for z in low.y.floor() as i32..=high.y.floor() as i32 { for x in low.x.floor() as i32..=high.x.floor() as i32 { chunks.push(builder::terrain_chunk(&stage,(x,z))); }}
    let patch = grass_voxel_asset::build_patch(&chunks, min, max, vegetation)?;
    if patch.instances.is_empty() { return Err("selected stage patch has no accepted live grass; choose another --at".into()); }
    let mut levels = Vec::new();
    for (index, level) in asset.volume.levels.iter().enumerate() {
        let mut mass = [0.0f64; 23];
        let mut color_min = [f32::INFINITY; 3]; let mut color_max = [0.0f32; 3];
        for v in &level.voxels {
            mass[0] += v.surface_area as f64;
            for k in 0..13 { mass[1+k] += v.directional_area[k] as f64; }
            for k in 0..3 { mass[14+k] += v.albedo[k] as f64 * v.surface_area as f64; color_min[k]=color_min[k].min(v.albedo[k]);color_max[k]=color_max[k].max(v.albedo[k]); }
            for k in 0..6 { mass[17+k] += v.normal_weights[k] as f64 * v.surface_area as f64; }
        }
        let dense: u32 = level.dimensions.iter().product();
        levels.push(json!({"level":index,"cell_m":level.cell_size,"dimensions":level.dimensions,"occupied":level.voxels.len(),"empty":dense as usize-level.voxels.len(),
            "covered_area_m2":mass[0],"directional_area_m2":&mass[1..14],"linear_rgb_area":&mass[14..17],"normal_lobe_area":&mass[17..23],"spatial_color_min":color_min,"spatial_color_max":color_max}));
    }
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let packed = write_packed(&asset.volume, &out)?;
    let preview = preview(&asset, &out)?;
    let projected = projected_preview(&asset, &out)?;
    let instances: Vec<Value> = patch.instances.iter().map(|p|json!({"id":p.id,"asset":p.asset,"root":p.root,"scale":p.scale,"yaw":p.yaw,"linear_tint":p.tint})).collect();
    let groups: Vec<Value> = patch.groups.iter().map(|g|json!({"id":g.id,"first":g.first,"count":g.count,"rest_bounds":g.bounds})).collect();
    let patch_json = json!({"seed":seed,"stage_noise_seed":stage.noise_seed,"stage_detail_seed":stage.detail_seed,"at_m":at,"vegetation":vegetation,
        "world_cell_m":live_ground_cover::CELL,"group_cells":8,"cell_bounds_min_inclusive":min,"cell_bounds_max_exclusive":max,"groups":groups,"instances":instances});
    fs::write(out.join("patch-instances.json"),serde_json::to_vec_pretty(&patch_json).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let geometry_bytes = asset.vertices.iter().flat_map(|v|v.pos.into_iter().chain(v.normal).chain(v.uv).flat_map(f32::to_le_bytes)).chain(asset.indices.iter().flat_map(|v|v.to_le_bytes()));
    let report = json!({"runtime_enabled":false,"gpu_used":false,"live_representation":"unchanged statistical four-height-bin mean field",
        "source":{"license":"Poly Haven grass_medium_01, CC0, existing CREDITS.md","texture_size":asset.photo.dimensions(),"photo_fingerprint":fingerprint(asset.photo.as_raw().iter().copied()),
            "geometry_fingerprint":fingerprint(geometry_bytes),"vertices":asset.vertices.len(),"triangles":asset.indices.len()/3,"width_m":ground_cover::WIDTH,"height_m":ground_cover::HEIGHT,"alpha_cut":0.5,"uv_address":"clamp"},
        "asset":{"origin":asset.volume.origin,"source_bounds":asset.volume.source_bounds,"bake_ms":bake_ms,"samples":asset.volume.stats.samples,"levels":levels,"packed":packed},
        "patch":{"file":"patch-instances.json","instances":patch.instances.len(),"groups":patch.groups.len(),"unique_assets":1,"native_chunks":chunks.len(),"rest_transform_matches_live":true},
        "preview":preview,"projected_preview":projected,"total_ms":started.elapsed().as_secs_f64()*1000.0,
        "integration_contract":{"identity":"stage epoch + signed world cell; never pool slot or view index","placement":"same live candidate function; no camera and no extra random seed",
            "color":"raw linear photo RGB in cells; apply seasonal tint exactly once and same near local-height factor 1.25*(.8+.2*h), wetness separately; no independent far tint",
            "normals":"stored +/-Y moments are authored card proxies, not real blades. Use shared near grass statistical leaf material BRDF until source normals are improved for both representations",
            "wind":"same existing WindState sample at exact root, grass=true; inverse shear the volume ray and inverse-transpose shading normals. Bounds include filter support and maximum wind displacement",
            "motion":"retain owner and previous/current transforms/wind; recycled slot or new stage invalidates history. Current experimental tree-volume path does not supply this grass motion",
            "runtime":"shared assets plus spatial groups; future GPU placement consumes same native triangle/road/hash rules. Do not attach thousands of tufts to the bounded 256-tree brute-force renderer",
            "lod":"select projected representation error per group; blend extinction and premultiplied color/moments, no opaque cubes, density thinning or size changes. Native transition is not implemented here"}});
    fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    println!("{}",json!({"out":out,"instances":patch.instances.len(),"shared_assets":1,"bake_ms":bake_ms,"gpu_used":false}));
    Ok(())
}
fn main() { if let Err(error)=run() { eprintln!("grass_voxel_bake: {error}"); std::process::exit(1); } }
