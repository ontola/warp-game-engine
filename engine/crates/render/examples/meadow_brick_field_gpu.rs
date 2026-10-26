//! Explicit serial GPU proof for bounded current-pose meadow optical bricks.
//! Run only with --out FRESH_DIR. No gameplay/default renderer path is changed.
//! Point quadrature and volume transport approximate source coverage; image
//! differences are evidence, never hidden behind an invented parity threshold.
use glam::{Affine3A, Mat4, Vec2, Vec3, Vec3Swizzles};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::debug_settings::DebugRenderOptions;
use render::foliage_assembly::MeadowBladeBudget;
use render::foliage_volume_data::GpuCell;
use render::meadow_brick_field::{BrickPlan, BrickSpec, MIPS};
use render::meadow_data::{MeadowField, MeadowGeometry, ROOT_CELL};
use render::meadow_voxel_samples::{MeadowVoxelSamples, SampleConfig};
use render::scene::{Heightfield, Material};
use render::wind::WindState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet}, path::Path, time::Instant};

const VIEW_W: u32 = 640;
const VIEW_H: u32 = 360;
const W: u32 = VIEW_W * 2;
const H: u32 = VIEW_H * 2;
const WARMUP: usize = 4;
const STEADY: usize = 20;
const HABITAT_HALF: f32 = 1.8;
const MASS_RELATIVE_LIMIT: f64 = 1e-4;
const AB_STEADY: usize = 8;
const OPTICAL_SS: u32 = 2;

fn optical_views(scale:u32)->Vec<ViewRect> {
    // Keep the complete habitat inside the source's unfaded60m circle. The
    // last two views share an angle and straddle the first4px mip transition.
    [(8f32,20f32,0f32),(26.,2.,135.),(50.,8.,0.),(56.,8.,0.)].into_iter().enumerate().map(|(i,(distance,elevation,azimuth))| {
        let az=azimuth.to_radians();let target=Vec3::new(0.,0.15,0.);
        ViewRect {cam:Camera {pos:target+Vec3::new(az.sin()*distance,elevation.to_radians().tan()*distance,az.cos()*distance),
            target,fov_y:50f32.to_radians(),near:0.05},x:i as u32%2*VIEW_W*scale,y:i as u32/2*VIEW_H*scale,w:VIEW_W*scale,h:VIEW_H*scale}
    }).collect()
}

fn optical_light(index:usize)->Lighting {
    if index<2 {return light(index==1);}
    // An explicit diagnostic night ambient, not a moon/GI claim. A nonzero
    // sky tests normal response without daylight overwhelming the comparison.
    Lighting {sun_dir:Vec3::new(0.2,-0.4,-0.8).normalize(),sun_lux:0.,sky_lux:10.,ev100:6.,..light(false)}
}

fn optical_sky(light: &Lighting) -> Result<render::atmosphere::BakedSky, String> {
    // Renderer::new calibrates an HDRI once. Merely changing Lighting.sky_lux
    // later does not replace its texture/SH. Commit a matching physical sky
    // for every fixture light case, including the10-lux night case.
    let rgb_lux = Vec3::new(0.65 / 0.75, 1.0, 0.95 / 0.75) * light.sky_lux;
    let radiance = rgb_lux / std::f32::consts::PI;
    let mut sh = [[0.0; 4]; 9];
    sh[0] = (rgb_lux / 0.886227).extend(0.0).to_array();
    let solar = render::atmosphere::SolarState::from_degrees(
        f64::from(light.sun_dir.y).asin().to_degrees(),
        f64::from(light.sun_dir.x).atan2(f64::from(-light.sun_dir.z)).to_degrees())?;
    let night_ambient = if light.sun_lux == 0.0 { rgb_lux } else { Vec3::ZERO };
    let sky = render::atmosphere::BakedSky {
        width:16, height:8, pixels:vec![radiance.extend(1.0).to_array();128],
        sun_dir:light.sun_dir, sun_rgb_lux:light.sun_color * light.sun_lux,
        sky_horizontal_rgb_lux:rgb_lux, sh, horizon:radiance,
        fog_phase_g:0.0, fog_scattering_albedo:Vec3::ONE, fog_height_m:1000.0,
        night_ambient_rgb_lux:night_ambient,
        night:render::atmosphere::NightSky { ambient_rgb_lux:night_ambient.as_dvec3(), star_radiance:0.0, ..Default::default() },
        night_stars:[0.0;4], solar,
        quality:render::atmosphere::BakeQuality { sky_width:16, sky_height:8, ..Default::default() },
    };
    sky.validate()?;
    Ok(sky)
}

fn commit_optical_sky(gpu: &Gpu, source: &mut Renderer, volume: &mut Renderer, light: &Lighting) -> Result<Value, String> {
    let sky = optical_sky(light)?;
    // Keep the requested direct-light/exposure decomposition unchanged. The
    // payload and original light encode the identical sun RGB illuminance.
    source.update_sky(gpu, &sky, &mut light.clone())?;
    volume.update_sky(gpu, &sky, &mut light.clone())?;
    Ok(json!({"kind":"constant physical diagnostic sky","committed_texture_and_sh":true,
        "horizontal_rgb_lux":sky.sky_horizontal_rgb_lux.to_array(),"radiance_rgb":sky.horizon.to_array(),
        "sh0":sky.sh[0],"sky_radiance_sha256":digest(bytemuck::cast_slice(&sky.pixels)),
        "sun_rgb_lux":sky.sun_rgb_lux.to_array(),"ev100":light.ev100,
        "scope":"Analytic constant environment; no atmosphere, stars, moon or GI model. Texture and diffuse SH are committed together, independent of frozen source optical cells."}))
}

fn decode_layer(bytes:&[u8],width:u32,height:u32)->Result<Vec<[f32;4]>,String> {
    if bytes.len()!=width as usize*height as usize*8 {return Err("optical layer dimensions differ".into());}
    bytes.chunks_exact(8).map(|p| {
        let c=std::array::from_fn(|i|half(u16::from_le_bytes([p[i*2],p[i*2+1]])));
        if c.iter().any(|v|!v.is_finite()) || c[..3].iter().any(|v|*v<0.) || !(0.0..=1.0).contains(&c[3]) {
            Err("nonfinite/negative optical layer or alpha outside[0,1]".into())
        } else {Ok(c)}
    }).collect()
}

fn downsample_layer(source:&[[f32;4]],width:u32,height:u32,scale:u32)->Result<Vec<[f32;4]>,String> {
    if scale==0 || source.len()!=width as usize*height as usize || width%scale!=0 || height%scale!=0 {
        return Err("invalid optical supersample dimensions".into());
    }
    let mut result=vec![[0.;4];(width/scale*height/scale)as usize];
    for y in 0..height/scale {for x in 0..width/scale {
        let mut sum=[0f64;4];
        for dy in 0..scale {for dx in 0..scale {let p=source[((y*scale+dy)*width+x*scale+dx)as usize];
            for c in 0..4 {sum[c]+=f64::from(p[c]);}
        }}
        result[(y*(width/scale)+x)as usize]=sum.map(|s|(s/f64::from(scale*scale))as f32);
    }}
    Ok(result)
}

fn downsample_rgba(source:&[u8],width:u32,height:u32,scale:u32)->Result<Vec<u8>,String> {
    if source.len()!=width as usize*height as usize*4 {return Err("invalid optical reference image size".into());}
    let pixels:Vec<[f32;4]>=source.chunks_exact(4).map(|p|std::array::from_fn(|c|f32::from(p[c]))).collect();
    Ok(downsample_layer(&pixels,width,height,scale)?.into_iter().flat_map(|p|p.map(|v|v.round().clamp(0.,255.)as u8)).collect())
}

fn roi_metrics(source:&[[f32;4]],volume:&[[f32;4]],source_image:&[u8],volume_image:&[u8],width:u32,height:u32)->Result<Value,String> {
    let count=width as usize*height as usize;
    if source.len()!=count || volume.len()!=count || source_image.len()!=count*4 || volume_image.len()!=count*4 {
        return Err("optical ROI inputs have different dimensions".into());
    }
    let mut low=[width,height];let mut high=[0,0];let mut support=0u64;
    let mut source_area=0f64;let mut volume_area=0f64;let mut under=0f64;let mut over=0f64;
    let mut source_radiance=[0f64;3];let mut volume_radiance=[0f64;3];
    let mut rgb_abs=0f64;let mut rgb_max=0u8;let mut luma_signed=0f64;let mut luma_abs=0f64;
    let mut conditional_luma_abs=0f64;let mut common_weight=0f64;let mut soft_intersection=0f64;let mut soft_union=0f64;
    let luma=|c:[f64;3]|c[0]*0.2126+c[1]*0.7152+c[2]*0.0722;
    for i in 0..count {
        let a=source[i];let b=volume[i];
        if a.iter().chain(b.iter()).any(|v|!v.is_finite()) || !(0.0..=1.0).contains(&a[3]) || !(0.0..=1.0).contains(&b[3]) {
            return Err("invalid optical metric alpha/radiance".into());
        }
        let sa=f64::from(a[3]);let va=f64::from(b[3]);
        source_area+=sa;volume_area+=va;under+=(sa-va).max(0.);over+=(va-sa).max(0.);
        soft_intersection+=sa.min(va);soft_union+=sa.max(va);
        for c in 0..3 {source_radiance[c]+=f64::from(a[c]);volume_radiance[c]+=f64::from(b[c]);}
        let shared=sa.min(va);
        if shared>0. {
            let a_luma=luma(std::array::from_fn(|c|f64::from(a[c])/sa));
            let b_luma=luma(std::array::from_fn(|c|f64::from(b[c])/va));
            conditional_luma_abs+=(a_luma-b_luma).abs()*shared;common_weight+=shared;
        }
        // The actual source fractional mask and actual volume alpha define
        // the ROI. No whole-frame background can dilute its colour error.
        if sa>0. || va>1.0/1024. {
            support+=1;let xy=[i as u32%width,i as u32/width];
            for axis in 0..2 {low[axis]=low[axis].min(xy[axis]);high[axis]=high[axis].max(xy[axis]+1);}
            let mut delta=[0f64;3];
            for c in 0..3 {let diff=source_image[i*4+c].abs_diff(volume_image[i*4+c]);rgb_abs+=f64::from(diff);rgb_max=rgb_max.max(diff);
                delta[c]=f64::from(volume_image[i*4+c])-f64::from(source_image[i*4+c]);}
            let d=luma(delta);luma_signed+=d;luma_abs+=d.abs();
        }
    }
    if source_area<=0. || support==0 {return Err("source coverage oracle is empty".into());}
    let conditional_source=luma(source_radiance)/source_area;
    let conditional_volume=(volume_area>0.).then(||luma(volume_radiance)/volume_area);
    Ok(json!({"roi_bounds_xyxy":[low[0],low[1],high[0],high[1]],"roi_union_pixels":support,
        "source_fractional_coverage_pixels":source_area,"volume_alpha_sum_pixels":volume_area,
        "coverage_delta_percent":100.*(volume_area-source_area)/source_area,
        "missing_fractional_coverage_pixels":under,"extra_fractional_coverage_pixels":over,
        "soft_coverage_iou":soft_intersection/soft_union.max(1e-30),
        "roi_mean_abs_rgb_u8":rgb_abs/(support as f64*3.),"roi_max_channel_u8":rgb_max,
        "roi_signed_luma_delta_u8":luma_signed/support as f64,"roi_mean_abs_luma_delta_u8":luma_abs/support as f64,
        "source_conditional_hdr_luma":conditional_source,"volume_conditional_hdr_luma":conditional_volume,
        "conditional_hdr_luma_delta_percent":conditional_volume.and_then(|v|(conditional_source>0.).then(||100.*(v-conditional_source)/conditional_source)),
        "overlap_weighted_conditional_hdr_luma_mae":(common_weight>0.).then(||conditional_luma_abs/common_weight),
        "optical_parity_accepted":false,"replacement_authorized":false,
        "scope":"Coverage is actual source RGBA16 alpha area-averaged from2x linear dimensions versus actual volume alpha. HDR is premultiplied radiance; conditional luma divides by coverage. RGB/luma image errors use only union support. Supersampling is finite quadrature, not exact continuous coverage. No tolerance invents optical acceptance."}))
}

fn image_view(image:&[u8],output_width:u32,view:&ViewRect)->Vec<u8> {
    (view.y..view.y+view.h).flat_map(|y|image[((y*output_width+view.x)*4)as usize..((y*output_width+view.x+view.w)*4)as usize].iter().copied()).collect()
}

fn optical_montage(out:&Path,name:&str,source:&[u8],volume:&[u8],cameras:&[ViewRect],metrics:&[Value])->Result<(),String> {
    const PANEL_W:u32=320;const PANEL_H:u32=180;
    let mut sheet=image::RgbImage::from_pixel(PANEL_W*3,PANEL_H*4,image::Rgb([24,24,24]));
    for (i,(view,metric)) in cameras.iter().zip(metrics).enumerate() {
        let bounds=metric["roi_bounds_xyxy"].as_array().ok_or("missing optical ROI")?;
        let b:Vec<u32>=bounds.iter().map(|n|n.as_u64().unwrap()as u32).collect();
        let left=b[0].saturating_sub(10);let top=b[1].saturating_sub(10);
        let width=(b[2]+10).min(view.w)-left;let height=(b[3]+10).min(view.h)-top;
        let a=image_view(source,W,view);let v=image_view(volume,W,view);
        let difference:Vec<u8>=a.iter().zip(&v).enumerate().map(|(j,(x,y))|if j%4==3 {255}else{x.abs_diff(*y).saturating_mul(4)}).collect();
        for (column,data) in [a,v,difference].into_iter().enumerate() {
            let input=image::RgbaImage::from_raw(view.w,view.h,data).ok_or("invalid montage image")?;
            let crop=image::imageops::crop_imm(&input,left,top,width,height).to_image();
            let scale=(PANEL_W as f64/f64::from(width)).min(PANEL_H as f64/f64::from(height)).min(4.);
            let w=(f64::from(width)*scale).round().max(1.)as u32;let h=(f64::from(height)*scale).round().max(1.)as u32;
            let resized=image::imageops::resize(&crop,w,h,image::imageops::FilterType::Nearest);
            for y in 0..h {for x in 0..w {let p=resized.get_pixel(x,y);sheet.put_pixel(column as u32*PANEL_W+(PANEL_W-w)/2+x,i as u32*PANEL_H+(PANEL_H-h)/2+y,image::Rgb([p[0],p[1],p[2]]));}}
        }
    }
    let file=std::fs::File::create(out.join(format!("{name}-source-volume-difference.jpg"))).map_err(|e|e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(file,94).encode(sheet.as_raw(),sheet.width(),sheet.height(),image::ExtendedColorType::Rgb8).map_err(|e|e.to_string())
}

fn optical_projection(view: &ViewRect) -> Mat4 {
    Mat4::perspective_infinite_reverse_rh(view.cam.fov_y, view.w as f32 / view.h as f32, view.cam.near)
        * Mat4::look_at_rh(view.cam.pos, view.cam.target, Vec3::Y)
}

fn optical_source_counts(gpu: &Gpu, renderer: &Renderer, size: [u32; 2]) -> Result<Value, String> {
    let counts = renderer.meadow_diagnostics(gpu)?;
    if counts.len() != 4 || counts.iter().any(|d| d.aggregate_enabled || d.visibility_invalid_ids != 0)
        || counts.iter().any(|d| d.plants.iter().all(|n| *n == 0)) {
        return Err("optical source reference needs four exercised source-only views with valid IDs".into());
    }
    Ok(json!({"input_size_per_view":size,"finest_geometry_for_all_selected_lods":true,
        "views":counts.iter().map(|d|json!({"view":d.view,"plants":d.plants,"triangles":d.triangles,
            "source_geometry_passes":d.source_geometry_passes,"submitted_source_triangles":d.submitted_source_triangles,
            "visibility_enabled":d.visibility_enabled,"visibility_covered_pixels":d.visibility_covered_pixels,
            "visibility_unique_primitives":d.visibility_unique_primitives,"visibility_invalid_ids":d.visibility_invalid_ids,
            "density":d.density,"size_scale":d.size_scale,"color_multiplier":d.color_multiplier})).collect::<Vec<_>>()}))
}

fn optical_source_layers(gpu: &Gpu, renderer: &Renderer, scale: u32) -> Result<Vec<Vec<[f32; 4]>>, String> {
    let textures = renderer.meadow_source_fixture_layers();
    if textures.len() != 4 { return Err("missing four actual source RGBA16 layers".into()); }
    textures.iter().map(|texture| {
        let (w, h) = (VIEW_W * scale, VIEW_H * scale);
        if texture.width() != w || texture.height() != h { return Err("unexpected source layer dimensions".into()); }
        let pixels = decode_layer(&read_rgba16(gpu, texture)?, w, h)?;
        downsample_layer(&pixels, w, h, scale)
    }).collect()
}

fn optical_timed_render(gpu: &Gpu, renderer: &Renderer, cameras: &[ViewRect], lighting: &Lighting,
    frozen: bool) -> Result<(Vec<u8>, Vec<BTreeMap<String, f64>>), String> {
    let mut rows = Vec::new();
    let mut output = None;
    for frame in 0..WARMUP + AB_STEADY {
        let (texture, stats) = renderer.try_render_texture_profiled(gpu, cameras, lighting, W, H)?;
        let row = if frozen { frozen_timing(stats)? } else {
            let mut row = BTreeMap::new();
            for (name, ms) in stats.gpu_ms {
                if !ms.is_finite() || ms < 0.0 { return Err("invalid source GPU timestamp".into()); }
                *row.entry(name.to_owned()).or_insert(0.0) += ms;
            }
            if row.is_empty() { return Err("source GPU timestamp queries unavailable".into()); }
            row
        };
        if frame >= WARMUP { rows.push(row); }
        output = Some(texture);
    }
    // Explicit readback is outside all steady timestamp intervals.
    Ok((gpu.read_rgba8(&output.unwrap()), rows))
}

fn optical_work(capture: &JumpCapture, budget: f32, smooth: bool, plan: &BrickPlan,
    cameras: &[ViewRect]) -> Result<Value, String> {
    let mut rows = Vec::new();
    for (view, d) in cameras.iter().zip(&capture.stats) {
        if d.lod_pixels != budget || d.smooth_mips_enabled != smooth {
            return Err(format!("optical mip mode differs from request: {d:?}"));
        }
        let projection = optical_projection(view);
        let bounds = plan.levels.chunks_exact(MIPS).map(|levels|
            render::vegetation_brick_render::brick_projected_error_bound(projection, [view.w, view.h], &levels[0]))
            .collect::<Result<Vec<_>, _>>()?;
        let bound = bounds.iter().copied().fold(0f32, f32::max);
        rows.push(json!({"view":d.view,"input_size":d.input_size,"generation":d.expected_generation,
            "groups":d.groups,"invalid_groups":d.invalid_groups,"failed_rays":d.failed_rays,
            "active_tiles":d.active_tiles,"scheduled_pixels":d.scheduled_pixels,"traced_rays":d.traced_rays,
            "cell_steps":d.cell_steps,"empty_jump_regions":d.empty_jump_regions,"empty_jump_fallbacks":d.empty_jump_fallbacks,
            "actual_traversal_iterations":d.cell_steps+d.empty_jump_regions,"occupied_samples":d.occupied_samples,
            "lod_pixels":d.lod_pixels,"smooth_mips_enabled":d.smooth_mips_enabled,
            "mip_transition_octaves":d.mip_transition_octaves,"max_selected_mip":d.max_selected_mip,
            "traversal_mip_hits":d.traversal_mip_hits,"blended_brick_hits":d.blended_brick_hits,
            "blended_cell_segments":d.blended_cell_segments,"optical_cell_reads":d.optical_cell_reads,
            "finest_over_budget_hits":d.finest_over_budget_hits,"max_finest_projected_pixels":d.max_finest_projected_pixels,
            "whole_brick_finest_bound_pixels":bound,"per_brick_finest_bound_pixels":bounds,
            "spatially_eligible":bound<=budget,"source_retention_required_by_spatial_bound":bound>budget,
            "optical_parity_accepted":false,"replacement_authorized":false,
            "provider_bytes":d.shared_provider_bytes,"target_bytes":d.target_bytes}));
    }
    Ok(json!(rows))
}

fn optical_masks(layers: &[Vec<[f32; 4]>], cameras: &[ViewRect]) -> Vec<u8> {
    let mut result = vec![0; (W * H * 4) as usize];
    for (layer, view) in layers.iter().zip(cameras) {
        for y in 0..view.h { for x in 0..view.w {
            let alpha = (layer[(y * view.w + x) as usize][3] * 255.0).round().clamp(0.0, 255.0) as u8;
            let offset = (((view.y + y) * W + view.x + x) * 4) as usize;
            result[offset..offset + 4].copy_from_slice(&[alpha, alpha, alpha, 255]);
        }}
    }
    result
}

fn run_optical_lod(out: &Path, night_only: bool) -> Result<Value, String> {
    let mut geometry = MeadowGeometry::cook_with_blade_budget(73, 0.0, MeadowBladeBudget::Blades120)?;
    let original_lod_triangles = geometry.lod_ranges.map(|variant| variant.map(|range| range[1] / 3));
    for variant in 0..3 {
        geometry.lod_ranges[variant] = [geometry.lod_ranges[variant][0]; 3];
        geometry.lod_errors[variant] = [0.0; 3];
    }
    geometry.validate()?;
    let support = enclosure(&geometry)?;
    let samples = MeadowVoxelSamples::cook(&geometry, SampleConfig { spacing: 0.16, max_per_triangle: 4, ..Default::default() })?;
    let plan = brick_plan(support, &samples, -1.0)?;
    let cameras = optical_views(1);
    let supersampled = optical_views(OPTICAL_SS);
    let gpu = Gpu::new()?;
    let mut source = Renderer::new(&gpu, &scene(geometry.clone())?, &light(false));
    let mut volume = Renderer::new(&gpu, &scene(geometry.clone())?, &light(false));
    source.set_debug_options(DebugRenderOptions::default())?;
    volume.set_debug_options(DebugRenderOptions::default())?;
    volume.install_meadow_brick_fixture(&gpu, &samples, plan.clone(), 2.0)?;
    volume.set_meadow_brick_fixture_empty_skip(false)?;
    volume.set_meadow_brick_fixture_empty_jump(true)?;
    let camera_json: Vec<_> = cameras.iter().map(|v| json!({"position":v.cam.pos.to_array(),
        "target":v.cam.target.to_array(),"fov_y":v.cam.fov_y,"near":v.cam.near,"rect":[v.x,v.y,v.w,v.h],
        "horizontal_distance_m":(v.cam.pos-v.cam.target).xz().length()})).collect();
    let wind_times: &[f32] = if night_only { &[1.25] } else { &[1.25, 4.75, 8.0] };
    let light_cases: &[(usize, &str)] = if night_only { &[(2,"night-ambient")] }
        else { &[(0,"noon"),(1,"low-sun"),(2,"night-ambient")] };
    let configuration = json!({"schema":1,"fixture":"matched finest-source versus optical world-brick LOD",
        "native":true,"output_size":[W,H],"input_per_view":[VIEW_W,VIEW_H],"views":4,
        "reference_supersample_linear":OPTICAL_SS,"reference_input_per_view":[VIEW_W*OPTICAL_SS,VIEW_H*OPTICAL_SS],
        "source_blades_per_root":120,"source_lod_forced_finest":true,"original_lod_triangles":original_lod_triangles,
        "source_geometry_sha256":digest(bytemuck::cast_slice(&geometry.vertices)),
        "samples_sha256":digest(bytemuck::cast_slice(&samples.samples)),"source_sample_bytes":samples.bytes(),
        "sample_spacing_m":samples.config.spacing,"sample_radius_m":samples.stats.max_sample_radius,
        "source_area_m2":samples.stats.source_area,"sample_area_m2":samples.stats.packed_area,
        "fine_cell_m":plan.levels[0].origin_cell[3],"source_fade_start_m":60.0,"source_radius_m":64.0,
        "source_habitat_half_m":HABITAT_HALF,"source_maximum_support_m":support,
        "consumer_lod_pixel_budgets":[2,4],"true_empty_jump":true,"old_empty_read_bypass":false,
        "producer_generations":wind_times.len(),"light_cases":light_cases.iter().map(|(_,name)|name).collect::<Vec<_>>(),"wind_times":wind_times,
        "sky_revision":"per-case physical texture and analytic SH committed together; replaces stale daytime environment in candidate-a",
        "warmup_per_timing_block":WARMUP,"steady_per_timing_block":AB_STEADY,"cameras":camera_json,
        "montage_columns":["2x source downsampled","native volume","absolute RGB difference times4"],
        "montage_rows":"camera order:8m oblique,26m grazing,50m and56m matched angle",
        "scope":"One enclosed flat habitat, four immutable GPU-produced bricks per wind. All120 blades retained in the independent finest source. Pixel budgets are diagnostic fixture controls, not changed production defaults. Spatial eligibility does not imply optical acceptance. Native timing excludes supersampling, readback and producer work."});
    write_json(&out.join("configuration.json"), &configuration)?;
    let mut cases = Vec::new();
    let mut identities = Vec::new();
    let mut blended_hits = 0u64;
    let mut over_budget_hits = 0u64;
    for (wind_index, &time) in wind_times.iter().enumerate() {
        let wind = WindState { time_seconds: f64::from(time), strength: 0.6, direction: Vec2::new(0.8, 0.6), ..Default::default() };
        source.set_wind(&gpu, &wind)?;
        volume.set_wind(&gpu, &wind)?;
        volume.set_meadow_brick_fixture_frozen(false)?;
        let (_, produced) = volume.try_render_texture_profiled(&gpu, &cameras, &light(false), W, H)?;
        if !render::meadow_brick_field::PASS_NAMES.iter().all(|name| produced.gpu_ms.iter().any(|(n, _)| n == name)) {
            return Err("optical fixture producer generation was not observed".into());
        }
        let (provider, _) = provider_report(&gpu, &volume, &plan, true)?;
        volume.set_meadow_brick_fixture_frozen(true)?;
        let identity = frozen_identity(&gpu, &volume)?;
        let generation = identity["generation"].as_u64().ok_or("missing optical generation")? as u32;
        identities.push(identity.clone());
        write_json(&out.join(format!("wind-{wind_index}-provider.json")), &json!({"provider":provider,"identity":identity}))?;
        for &(light_index, light_name) in light_cases {
            let lighting = optical_light(light_index);
            let sky = commit_optical_sky(&gpu, &mut source, &mut volume, &lighting)?;
            let name = format!("{light_name}-wind-{wind_index}");
            write_json(&out.join("checkpoint.json"), &json!({"case":name,"phase":"source_reference"}))?;
            let (native_image, source_rows) = optical_timed_render(&gpu, &source, &cameras, &lighting, false)?;
            let native_counts = optical_source_counts(&gpu, &source, [VIEW_W, VIEW_H])?;
            let native_layers = optical_source_layers(&gpu, &source, 1)?;
            let ss_texture = source.try_render_texture(&gpu, &supersampled, &lighting, W * OPTICAL_SS, H * OPTICAL_SS)?;
            let ss_image = downsample_rgba(&gpu.read_rgba8(&ss_texture), W * OPTICAL_SS, H * OPTICAL_SS, OPTICAL_SS)?;
            let ss_layers = optical_source_layers(&gpu, &source, OPTICAL_SS)?;
            let ss_counts = optical_source_counts(&gpu, &source, [VIEW_W * OPTICAL_SS, VIEW_H * OPTICAL_SS])?;
            let source_aliasing = cameras.iter().enumerate().map(|(i, v)| roi_metrics(&ss_layers[i], &native_layers[i],
                &image_view(&ss_image, W, v), &image_view(&native_image, W, v), v.w, v.h)).collect::<Result<Vec<_>, _>>()?;
            let source_info = json!({"native_timing":timing(&source_rows),"native_timing_samples":source_rows,
                "native_counts":native_counts,"supersampled_counts":ss_counts,
                "native_source_versus_supersampled_source":source_aliasing,
                "supersampled_composite_sha256":digest(&ss_image),"supersampled_layer_f32_sha256":ss_layers.iter()
                    .map(|layer|digest(bytemuck::cast_slice(layer))).collect::<Vec<_>>()});
            write_json(&out.join(format!("{name}-source.json")), &source_info)?;
            let mut comparisons = Vec::new();
            // Alternate old/smooth order by case to avoid always placing one
            // method first. These bounded optical timings are not an ABBA FPS claim.
            for budget in [2.0, 4.0] {
                let order = if (wind_index + light_index) % 2 == 0 { [false, true] } else { [true, false] };
                for smooth in order {
                    volume.set_meadow_brick_fixture_lod_pixels(budget)?;
                    volume.set_meadow_brick_fixture_smooth_mips(smooth)?;
                    let mode = if smooth { "smooth" } else { "single" };
                    let label = format!("{name}-{mode}-{}px", budget as u32);
                    let (image, rows) = optical_timed_render(&gpu, &volume, &cameras, &lighting, true)?;
                    let capture = jump_capture(&gpu, &volume, image, generation, true)?;
                    let work = optical_work(&capture, budget, smooth, &plan, &cameras)?;
                    let layers = capture.layers.iter().map(|raw| decode_layer(raw, VIEW_W, VIEW_H)).collect::<Result<Vec<_>, _>>()?;
                    let metrics = cameras.iter().enumerate().map(|(i, v)| roi_metrics(&ss_layers[i], &layers[i],
                        &image_view(&ss_image, W, v), &image_view(&capture.image, W, v), v.w, v.h)).collect::<Result<Vec<_>, _>>()?;
                    optical_montage(out, &label, &ss_image, &capture.image, &cameras, &metrics)?;
                    optical_montage(out, &format!("{label}-coverage"), &optical_masks(&ss_layers, &cameras),
                        &optical_masks(&layers, &cameras), &cameras, &metrics)?;
                    blended_hits += capture.stats.iter().map(|d| d.blended_brick_hits).sum::<u64>();
                    over_budget_hits += capture.stats.iter().map(|d| d.finest_over_budget_hits).sum::<u64>();
                    let report = json!({"name":label,"mechanical_pass":true,"optical_parity_accepted":false,
                        "replacement_authorized":false,"work":work,"metrics":metrics,
                        "native_consumer_timing":consumer_only_timing(&rows),"all_timing_samples":rows,
                        "composite_sha256":digest(&capture.image),"rgba16_layer_sha256":capture.layers.iter().map(|raw|digest(raw)).collect::<Vec<_>>(),
                        "preview":format!("{label}-source-volume-difference.jpg"),
                        "coverage_preview":format!("{label}-coverage-source-volume-difference.jpg")});
                    write_json(&out.join(format!("{label}.json")), &report)?;
                    comparisons.push(report);
                }
            }
            if frozen_identity(&gpu, &volume)? != identity { return Err(format!("{name}: provider changed during optical comparisons")); }
            cases.push(json!({"name":name,"wind_time":time,"wind_strength":wind.strength,
                "wind_direction":wind.direction.to_array(),"sun_lux":lighting.sun_lux,"sun_direction":lighting.sun_dir.to_array(),
                "sky_lux":lighting.sky_lux,"ev100":lighting.ev100,"sky":sky,"provider_identity":identity,
                "provider_unchanged":true,"source":source_info,"comparisons":comparisons}));
            write_json(&out.join("checkpoint.json"), &json!({"phase":"optical_case_complete","case":name,"completed_cases":cases.len()}))?;
        }
    }
    if identities.windows(2).any(|p| p[0]["cells_sha256"] == p[1]["cells_sha256"]) {
        return Err("different wind poses produced identical optical fields".into());
    }
    if blended_hits == 0 || over_budget_hits == 0 {
        return Err("optical matrix did not exercise both blended mip work and finest-over-budget source retention".into());
    }
    Ok(json!({"pass":true,"mechanical_pass":true,"optical_parity_accepted":false,"replacement_authorized":false,
        "configuration":configuration,"cases":cases,"blended_brick_hits":blended_hits,"finest_over_budget_hits":over_budget_hits,
        "limitations":["Finite2x source supersampling is a better coverage reference, not continuous ground truth.",
            "Cell support, point quadrature, normal moments and Beer transport can differ substantially from source union and lighting.",
            "Over-budget volume-only views are diagnostic failure examples; they never authorize source replacement.",
            "All timing samples are small native four-view fixture work, not production4K/DLSS or complete field/FPS cost.",
            "No temporal reconstruction, terrain slope, full-map streaming or automatic source/volume transition acceptance."]}))
}

const SWEEP_STEPS: usize = 16;
const SWEEP_START_M: f32 = 32.0;
const SWEEP_END_M: f32 = 53.0;
fn sweep_distance(step: usize) -> f32 {
    SWEEP_START_M + step as f32 * (SWEEP_END_M-SWEEP_START_M)/(SWEEP_STEPS-1) as f32
}
fn sweep_views(step: usize, scale: u32) -> Vec<ViewRect> {
    let distance = sweep_distance(step);
    [(8f32,0f32),(2.,135.),(20.,270.),(8.,180.)].into_iter().enumerate().map(|(i,(elevation,azimuth))| {
        let az = azimuth.to_radians();
        let target = Vec3::new(0.0,0.15,0.0);
        ViewRect { cam:Camera { pos:target+Vec3::new(az.sin()*distance,elevation.to_radians().tan()*distance,az.cos()*distance),
            target,fov_y:65f32.to_radians(),near:0.05 },
            x:i as u32%2*VIEW_W*scale,y:i as u32/2*VIEW_H*scale,w:VIEW_W*scale,h:VIEW_H*scale }
    }).collect()
}

fn sweep_delta(previous: &[Value], current: &[Value]) -> Value {
    json!(previous.iter().zip(current).enumerate().map(|(view,(a,b))| {
        let delta = |key:&str| a[key].as_f64().zip(b[key].as_f64()).map(|(x,y)|y-x);
        json!({"view":view,"coverage_error_change_pp":delta("coverage_delta_percent"),
            "conditional_hdr_luma_error_change_pp":delta("conditional_hdr_luma_delta_percent"),
            "roi_rgb_mae_change_u8":delta("roi_mean_abs_rgb_u8"),
            "source_coverage_change_pixels":delta("source_fractional_coverage_pixels"),
            "volume_alpha_sum_change_pixels":delta("volume_alpha_sum_pixels")})
    }).collect::<Vec<_>>())
}

fn sweep_residual(previous_source:&[Vec<[f32;4]>], previous_volume:&[Vec<[f32;4]>],
    source:&[Vec<[f32;4]>], volume:&[Vec<[f32;4]>]) -> Value {
    json!((0..source.len()).map(|view| {
        let mut sum = 0f64;
        let mut maximum = 0f64;
        let mut count = 0u64;
        for (((a0,b0),a1),b1) in previous_source[view].iter().zip(&previous_volume[view]).zip(&source[view]).zip(&volume[view]) {
            if a0[3].max(a1[3]).max(b0[3]).max(b1[3]) <= 1.0/1024.0 {continue;}
            let difference = ((f64::from(b1[3])-f64::from(a1[3]))-(f64::from(b0[3])-f64::from(a0[3]))).abs();
            sum += difference;
            maximum = maximum.max(difference);
            count += 1;
        }
        json!({"view":view,"union_support_pixels":count,"mean_absolute_alpha_residual_change":if count>0 {sum/count as f64}else{0.0},
            "max_absolute_alpha_residual_change":maximum,
            "scope":"Change in (volume alpha minus supersampled source alpha) at the same screen pixels. Consecutive camera positions are not motion registered; this residual still includes representation parallax/shape differences and is not an isolated temporal-pop magnitude."})
    }).collect::<Vec<_>>())
}

const SWEEP_PANEL_W: u32 = 240;
const SWEEP_PANEL_H: u32 = 135;
const SWEEP_ROW_H: u32 = SWEEP_PANEL_H + 10;
fn sweep_sheet_row(sheet: &mut image::RgbImage, step:usize, view:&ViewRect, inputs:[&[u8];3]) -> Result<(),String> {
    for (column, input) in inputs.into_iter().enumerate() {
        let panel = image::RgbaImage::from_raw(VIEW_W,VIEW_H,image_view(input,W,view)).ok_or("invalid sweep image")?;
        // Fixed screen crop and scale for every frame/mode. Unlike independent
        // ROI montages this preserves visible movement and avoids crop zooms.
        let crop = image::imageops::crop_imm(&panel,240,135,160,90).to_image();
        let resized = image::imageops::resize(&crop,SWEEP_PANEL_W,SWEEP_PANEL_H,image::imageops::FilterType::Nearest);
        for y in 0..SWEEP_PANEL_H {for x in 0..SWEEP_PANEL_W {
            let p = resized.get_pixel(x,y);
            sheet.put_pixel(column as u32*SWEEP_PANEL_W+x,step as u32*SWEEP_ROW_H+y,image::Rgb([p[0],p[1],p[2]]));
        }}
    }
    Ok(())
}
fn save_sweep_sheet(out:&Path,name:&str,sheet:&image::RgbImage)->Result<Value,String> {
    let path = out.join(format!("{name}.jpg"));
    let file = std::fs::File::create(&path).map_err(|e|e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(file,94).encode(sheet.as_raw(),sheet.width(),sheet.height(),image::ExtendedColorType::Rgb8)
        .map_err(|e|e.to_string())?;
    Ok(json!({"file":format!("{name}.jpg"),"sha256":digest(&std::fs::read(path).map_err(|e|e.to_string())?),
        "columns":["2x source downsampled","single mip","smooth mip"],"rows":"16 consecutive distances32..53m; top row32m, step1.4m",
        "source_screen_crop_xywh":[240,135,160,90],"fixed_preview_panel_size":[SWEEP_PANEL_W,SWEEP_PANEL_H]}))
}

fn run_optical_sweep(out:&Path)->Result<Value,String> {
    let mut geometry = MeadowGeometry::cook_with_blade_budget(73,0.0,MeadowBladeBudget::Blades120)?;
    for variant in 0..3 {
        geometry.lod_ranges[variant]=[geometry.lod_ranges[variant][0];3];
        geometry.lod_errors[variant]=[0.0;3];
    }
    geometry.validate()?;
    let support = enclosure(&geometry)?;
    let samples = MeadowVoxelSamples::cook(&geometry,SampleConfig {spacing:0.16,max_per_triangle:4,..Default::default()})?;
    let plan = brick_plan(support,&samples,-1.0)?;
    let wind = WindState {time_seconds:1.25,strength:0.6,direction:Vec2::new(0.8,0.6),..Default::default()};
    let gpu = Gpu::new()?;
    let mut source = Renderer::new(&gpu,&scene(geometry.clone())?,&light(false));
    let mut volume = Renderer::new(&gpu,&scene(geometry.clone())?,&light(false));
    source.set_debug_options(DebugRenderOptions::default())?;
    volume.set_debug_options(DebugRenderOptions::default())?;
    source.set_wind(&gpu,&wind)?;
    volume.set_wind(&gpu,&wind)?;
    volume.install_meadow_brick_fixture(&gpu,&samples,plan.clone(),4.0)?;
    volume.set_meadow_brick_fixture_empty_skip(false)?;
    volume.set_meadow_brick_fixture_empty_jump(true)?;
    let first = sweep_views(0,1);
    volume.try_render_texture_profiled(&gpu,&first,&light(false),W,H)?;
    let (provider,_) = provider_report(&gpu,&volume,&plan,true)?;
    volume.set_meadow_brick_fixture_frozen(true)?;
    let identity = frozen_identity(&gpu,&volume)?;
    let generation = identity["generation"].as_u64().ok_or("missing sweep generation")? as u32;
    let configuration = json!({"schema":1,"fixture":"consecutive native optical mip sweep","native":true,
        "output_size":[W,H],"input_per_view":[VIEW_W,VIEW_H],"views":4,"steps":SWEEP_STEPS,
        "distance_start_m":SWEEP_START_M,"distance_step_m":(SWEEP_END_M-SWEEP_START_M)/(SWEEP_STEPS-1) as f32,
        "distance_end_m":SWEEP_END_M,"fov_y_degrees":65.0,
        "view_elevations_degrees":[8,2,20,8],"view_azimuths_degrees":[0,135,270,180],
        "consumer_lod_pixels":4.0,"reference_supersample_linear":OPTICAL_SS,"source_lod_forced_finest":true,
        "source_blades_per_root":120,"fine_cell_m":plan.levels[0].origin_cell[3],"source_fade_start_m":60.0,
        "source_geometry_sha256":digest(bytemuck::cast_slice(&geometry.vertices)),
        "source_sample_sha256":digest(bytemuck::cast_slice(&samples.samples)),
        "wind_time":wind.time_seconds,"wind_strength":wind.strength,"wind_direction":wind.direction.to_array(),
        "provider":provider,"provider_identity":identity,"lights":["noon","low-sun"],
        "timing_scope":"One synchronous GPU sample per actual camera step/mode after4 warmups per light. Source SS/readback are excluded. This is a transition diagnostic, not a steady-state or FPS benchmark.",
        "scope":"Same4px input-space budget as the original matrix. A wider65-degree FOV brings the complete first transition inside the unfaded source radius; it is not a changed production setting. No optical acceptance or automatic source omission."});
    write_json(&out.join("configuration.json"),&configuration)?;
    let mut lights = Vec::new();
    for (light_index,name) in ["noon","low-sun"].into_iter().enumerate() {
        let lighting = optical_light(light_index);
        let sky = commit_optical_sky(&gpu,&mut source,&mut volume,&lighting)?;
        for _ in 0..WARMUP {
            source.try_render_texture_profiled(&gpu,&first,&lighting,W,H)?;
            for smooth in [false,true] {
                volume.set_meadow_brick_fixture_smooth_mips(smooth)?;
                volume.try_render_texture_profiled(&gpu,&first,&lighting,W,H)?;
            }
        }
        let blank = || image::RgbImage::from_pixel(SWEEP_PANEL_W*3,SWEEP_ROW_H*SWEEP_STEPS as u32,image::Rgb([24,24,24]));
        let mut color_sheets:Vec<_> = (0..4).map(|_|blank()).collect();
        let mut coverage_sheets:Vec<_> = (0..4).map(|_|blank()).collect();
        let mut frames = Vec::new();
        let mut previous_metrics:Option<[Vec<Value>;2]> = None;
        let mut previous_layers:Option<(Vec<Vec<[f32;4]>>,[Vec<Vec<[f32;4]>>;2])> = None;
        let mut first_hits = [[0u64;4];2];
        let mut last_hits = [[0u64;4];2];
        let mut blend_segments = 0u64;
        for step in 0..SWEEP_STEPS {
            let cameras = sweep_views(step,1);
            let ss_cameras = sweep_views(step,OPTICAL_SS);
            let label = format!("{name}-step-{step:02}");
            write_json(&out.join("checkpoint.json"),&json!({"phase":"optical_sweep","frame":label}))?;
            let (_,source_stats) = source.try_render_texture_profiled(&gpu,&cameras,&lighting,W,H)?;
            if source_stats.gpu_ms.is_empty() || source_stats.gpu_ms.iter().any(|(_,ms)|!ms.is_finite() || *ms<0.0) {
                return Err("sweep source GPU timestamps are missing or invalid".into());
            }
            let source_counts = optical_source_counts(&gpu,&source,[VIEW_W,VIEW_H])?;
            let source_samples:Vec<_> = source_stats.gpu_ms.into_iter().map(|(pass,ms)|json!({"pass":pass,"ms":ms})).collect();
            let source_texture = source.try_render_texture(&gpu,&ss_cameras,&lighting,W*OPTICAL_SS,H*OPTICAL_SS)?;
            let source_image = downsample_rgba(&gpu.read_rgba8(&source_texture),W*OPTICAL_SS,H*OPTICAL_SS,OPTICAL_SS)?;
            let source_layers = optical_source_layers(&gpu,&source,OPTICAL_SS)?;
            let order = if step%2==0 {[false,true]}else{[true,false]};
            let mut captures:[Option<JumpCapture>;2] = [None,None];
            let mut timings:[Option<BTreeMap<String,f64>>;2] = [None,None];
            for smooth in order {
                volume.set_meadow_brick_fixture_smooth_mips(smooth)?;
                let (texture,stats) = volume.try_render_texture_profiled(&gpu,&cameras,&lighting,W,H)?;
                timings[usize::from(smooth)] = Some(frozen_timing(stats)?);
                captures[usize::from(smooth)] = Some(jump_capture(&gpu,&volume,gpu.read_rgba8(&texture),generation,true)?);
            }
            let captures = captures.map(Option::unwrap);
            let mut layers:[Vec<Vec<[f32;4]>>;2] = [Vec::new(),Vec::new()];
            let mut metrics:[Vec<Value>;2] = [Vec::new(),Vec::new()];
            let mut reports = Vec::new();
            for (mode,capture) in captures.iter().enumerate() {
                let work = optical_work(capture,4.0,mode==1,&plan,&cameras)?;
                layers[mode] = capture.layers.iter().map(|raw|decode_layer(raw,VIEW_W,VIEW_H)).collect::<Result<Vec<_>,_>>()?;
                metrics[mode] = cameras.iter().enumerate().map(|(i,v)|roi_metrics(&source_layers[i],&layers[mode][i],
                    &image_view(&source_image,W,v),&image_view(&capture.image,W,v),v.w,v.h)).collect::<Result<Vec<_>,_>>()?;
                for d in &capture.stats {
                    for mip in 0..4 {
                        if step==0 {first_hits[mode][mip]+=d.traversal_mip_hits[mip];}
                        if step+1==SWEEP_STEPS {last_hits[mode][mip]+=d.traversal_mip_hits[mip];}
                    }
                    if mode==1 {blend_segments+=d.blended_cell_segments;}
                }
                reports.push(json!({"smooth":mode==1,"work":work,"metrics":metrics[mode],
                    "timing_sample":timings[mode],"delta_from_previous":previous_metrics.as_ref().map(|old|sweep_delta(&old[mode],&metrics[mode])),
                    "alpha_residual_change":previous_layers.as_ref().map(|(old_source,old_volume)|sweep_residual(old_source,&old_volume[mode],&source_layers,&layers[mode])),
                    "composite_sha256":digest(&capture.image),"rgba16_layer_sha256":capture.layers.iter().map(|raw|digest(raw)).collect::<Vec<_>>()}));
            }
            let source_mask = optical_masks(&source_layers,&cameras);
            let hard_mask = optical_masks(&layers[0],&cameras);
            let smooth_mask = optical_masks(&layers[1],&cameras);
            for (view,camera) in cameras.iter().enumerate() {
                for mode in 0..2 {
                    let b = metrics[mode][view]["roi_bounds_xyxy"].as_array().ok_or("missing sweep support")?;
                    if b[0].as_u64().unwrap()<240 || b[1].as_u64().unwrap()<135 || b[2].as_u64().unwrap()>400 || b[3].as_u64().unwrap()>225 {
                        return Err(format!("{label}/view{view}: fixed sweep review crop would clip actual vegetation support"));
                    }
                }
                sweep_sheet_row(&mut color_sheets[view],step,camera,[&source_image,&captures[0].image,&captures[1].image])?;
                sweep_sheet_row(&mut coverage_sheets[view],step,camera,[&source_mask,&hard_mask,&smooth_mask])?;
            }
            let frame = json!({"step":step,"horizontal_distance_m":sweep_distance(step),"mode_order":order,
                "cameras":cameras.iter().map(|v|json!({"position":v.cam.pos.to_array(),"target":v.cam.target.to_array(),"fov_y":v.cam.fov_y})).collect::<Vec<_>>(),
                "source_counts":source_counts,"source_timing_sample":source_samples,
                "source_composite_sha256":digest(&source_image),"comparisons":reports,
                "optical_parity_accepted":false,"replacement_authorized":false});
            write_json(&out.join(format!("{label}.json")),&frame)?;
            frames.push(frame);
            previous_metrics = Some(metrics);
            previous_layers = Some((source_layers,layers));
        }
        if first_hits.iter().any(|hits|hits[0]==0 || hits[1..].iter().any(|n|*n!=0))
            || last_hits.iter().any(|hits|hits[1]==0 || hits[0]!=0 || hits[2..].iter().any(|n|*n!=0))
            || blend_segments==0 {
            return Err(format!("sweep did not exercise complete first transition: first={first_hits:?} last={last_hits:?} blended={blend_segments}"));
        }
        if frozen_identity(&gpu,&volume)?!=identity {return Err("provider changed during optical sweep".into());}
        let mut images = Vec::new();
        for view in 0..4 {
            images.push(save_sweep_sheet(out,&format!("{name}-view{view}-color-sequence"),&color_sheets[view])?);
            images.push(save_sweep_sheet(out,&format!("{name}-view{view}-coverage-sequence"),&coverage_sheets[view])?);
        }
        lights.push(json!({"name":name,"sky":sky,"frames":frames,"images":images,
            "first_traversal_mip_hits":first_hits,"last_traversal_mip_hits":last_hits,
            "blended_segments":blend_segments,"complete_first_transition_exercised":true,"provider_unchanged":true}));
    }
    Ok(json!({"pass":true,"mechanical_pass":true,"optical_parity_accepted":false,"replacement_authorized":false,
        "configuration":configuration,"lights":lights,
        "limits":["16 discrete camera steps and one fixed wind pose; no temporal reconstruction or continuous animation acceptance.",
            "Source-relative coverage/brightness changes isolate some camera effects but cannot prove lossless optics.",
            "The11.25cm source sample grid remains too coarse to reconstruct individual blade silhouettes near the camera.",
            "Cell-area/normal moments and Beer transport are approximations; conserved mass does not establish matching projected union."]}))
}

fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn write_json(path: &Path, data: &Value) -> Result<(), String> {
    std::fs::write(path, serde_json::to_vec_pretty(data).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}
fn read_buffer(gpu: &Gpu, source: &wgpu::Buffer) -> Result<Vec<u8>, String> {
    let target = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("meadow brick explicit fixture readback"), size: source.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &target, 0, source.size());
    gpu.queue.submit([encoder.finish()]);
    map(gpu, &target)
}
fn map(gpu: &Gpu, buffer: &wgpu::Buffer) -> Result<Vec<u8>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
    gpu.wait();
    rx.recv().map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
    let bytes = buffer.slice(..).get_mapped_range().map_err(|e|e.to_string())?.to_vec();
    buffer.unmap();
    Ok(bytes)
}
fn read_rgba16(gpu: &Gpu, texture: &wgpu::Texture) -> Result<Vec<u8>, String> {
    if texture.format() != wgpu::TextureFormat::Rgba16Float { return Err("unexpected brick target format".into()); }
    let row = (texture.width() * 8).div_ceil(256) * 256;
    let target = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("meadow brick HDR layer readback"), size: u64::from(row) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(texture.as_image_copy(), wgpu::TexelCopyBufferInfo {
        buffer: &target, layout: wgpu::TexelCopyBufferLayout {
            offset: 0, bytes_per_row: Some(row), rows_per_image: Some(texture.height()),
        },
    }, wgpu::Extent3d { width: texture.width(), height: texture.height(), depth_or_array_layers: 1 });
    gpu.queue.submit([encoder.finish()]);
    let padded = map(gpu, &target)?;
    Ok(padded.chunks_exact(row as usize).flat_map(|line|line[..texture.width() as usize*8].iter().copied()).collect())
}
fn half(bits: u16) -> f32 {
    let exponent = (bits >> 10) & 31; let fraction = bits & 1023;
    if exponent == 0 { return (if bits & 0x8000 == 0 {1.0} else {-1.0}) * fraction as f32 / 16_777_216.0; }
    f32::from_bits(((bits as u32 & 0x8000) << 16)
        | ((if exponent == 31 {255} else {exponent as u32+112}) << 23) | ((fraction as u32) << 13))
}
fn save_image(out: &Path, name: &str, rgba: &[u8]) -> Result<(), String> {
    render::write_png(&out.join(format!("{name}.png")), W, H, rgba)?;
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p|p[..3].iter().copied()).collect();
    let file = std::fs::File::create(out.join(format!("{name}.jpg"))).map_err(|e|e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(file, 90)
        .encode(&rgb, W, H, image::ExtendedColorType::Rgb8).map_err(|e|e.to_string())
}
fn enclosure(geometry: &MeadowGeometry) -> Result<f32, String> {
    // All roots require a valid ground triangle within the finite field. Yaw
    // and branch rotation preserve full anchor-relative length; ground is flat.
    // The shared producer's maximum smooth local growth is exactly1.3.
    let support = geometry.max_anchor_radius + geometry.max_blade_radius * 1.3;
    let interior_half = 8.0 * ROOT_CELL;
    if !support.is_finite() || HABITAT_HALF + support >= interior_half {
        return Err(format!("source habitat plus full wind/growth support escapes bricks: {HABITAT_HALF}+{support}>={interior_half}"));
    }
    Ok(support)
}
fn scene(geometry: MeadowGeometry) -> Result<Scene, String> {
    enclosure(&geometry)?;
    let mut scene = Scene::default();
    scene.sky = Some((16, 8, vec![[0.65, 0.75, 0.95, 1.0]; 128]));
    let material = scene.add_material(Material {
        tint: [0.16, 0.12, 0.075], layer: -1, roughness: 1.0,
        flags: 0, scale: 1.0, metal: 0.0,
    });
    let vertices = [[-100.,-0.015,100.], [100.,-0.015,100.], [-100.,-0.015,-100.], [100.,-0.015,-100.]]
        .map(|pos|Vertex { pos, normal: Vec3::Y.to_array(), uv: [0.;2] });
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &vertices, indices: &[0,1,2,1,3,2], material,
    }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene.heightfield = Heightfield { origin: [-100.;2], cell: 2., w: 101, h: 101, heights: vec![-0.015;10201] };
    let field = MeadowField {
        origin: [-HABITAT_HALF;2], cell: HABITAT_HALF/8., width: 17, height: 17,
        samples: vec![[0.,1_000_000.,1.,0.];17*17], seed: 73, growth: 1., dry_season: 0.,
        radius: 64., fade_start: 60., geometry,
    };
    field.validate()?;
    scene.meadow = Some(field);
    Ok(scene)
}
fn brick_plan(support: f32, samples: &MeadowVoxelSamples, lower_y: f32) -> Result<BrickPlan, String> {
    BrickPlan::new([[-8,-8],[0,-8],[-8,0],[0,0]].map(|root_origin|BrickSpec {
        root_origin, lower_y, cells_xz: 32, cells_y: 24,
    }).to_vec(), ROOT_CELL, support + 0.0001, samples.ranges.iter().map(|r|r[1]).max().unwrap())
}
fn views() -> Vec<ViewRect> {
    [Vec3::new(0.,3.5,7.), Vec3::new(5.5,0.65,6.), Vec3::new(-10.,7.,-13.), Vec3::new(0.,9.,55.)]
        .into_iter().enumerate().map(|(i,pos)|ViewRect {
            cam: Camera { pos, target: Vec3::new(0.,0.15,0.), fov_y: 50f32.to_radians(), near: 0.05 },
            x: i as u32%2*VIEW_W, y: i as u32/2*VIEW_H, w: VIEW_W, h: VIEW_H,
        }).collect()
}
fn light(low: bool) -> Lighting {
    Lighting {
        sun_dir: if low {Vec3::new(0.2,0.12,-0.97).normalize()} else {Vec3::new(0.4,0.8,-0.3).normalize()},
        sun_lux: if low {20_000.} else {80_000.}, sky_lux: if low {4_000.} else {18_000.},
        ev100: if low {13.} else {15.}, taa: 0., sharpen: 0.,
        gi: 0., contact_occlusion: 0., reflection_occlusion: 0., near_shadows: 0.,
        fog_density: 0., fog_start: 1e6, fog_end: 2e6, fog_culling: false,
        lod_pixels: 0., ..Default::default()
    }
}
fn quantile(values: &[f64], q: f64) -> f64 {
    let mut sorted = values.to_vec(); sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len()-1) as f64*q).round() as usize]
}
fn timing(rows: &[BTreeMap<String,f64>]) -> Value {
    let names: BTreeSet<_> = rows.iter().flat_map(|r|r.keys().cloned()).collect();
    let mut passes = serde_json::Map::new();
    for name in names {
        let values: Vec<_> = rows.iter().filter_map(|r|r.get(&name).copied()).collect();
        passes.insert(name, json!({"samples_present":values.len(),"median_ms":quantile(&values,0.5),
            "p95_ms":quantile(&values,0.95),"min_ms":quantile(&values,0.),"max_ms":quantile(&values,1.)}));
    }
    let sums: Vec<f64> = rows.iter().map(|r|r.values().sum()).collect();
    json!({"clean_samples":rows.len(),"warmup_excluded":WARMUP,"passes":passes,
        "timed_pass_sum_median_ms":quantile(&sums,0.5),"timed_pass_sum_p95_ms":quantile(&sums,0.95),
        "scope":"Explicit synchronous GPU timestamp samples; sum of instrumented passes is not covered GPU interval, cadence or FPS. CPU readbacks and PNG/JPEG writes happen afterward."})
}
fn render_case(gpu: &Gpu, renderer: &Renderer, cameras: &[ViewRect], light: &Lighting, wind: &WindState)
    -> Result<(Vec<u8>,Value),String> {
    renderer.set_wind(gpu, wind)?;
    let mut samples = Vec::new(); let mut texture = None;
    for frame in 0..WARMUP+STEADY {
        let (output,stats) = renderer.try_render_texture_profiled(gpu,cameras,light,W,H)?;
        let mut passes = BTreeMap::new();
        for (name,ms) in stats.gpu_ms {
            if !ms.is_finite() || ms<0. {return Err(format!("invalid GPU timestamp {name} {ms}"));}
            *passes.entry(name.to_owned()).or_insert(0.) += ms;
        }
        if frame>=WARMUP {samples.push(passes);}
        texture=Some(output);
    }
    if samples.iter().any(|r|r.is_empty()) {return Err("GPU timestamps unavailable; cannot claim measured fixture timing".into());}
    Ok((gpu.read_rgba8(&texture.unwrap()),json!({"summary":timing(&samples),"samples":samples})))
}
fn optical_difference(source: &[u8], volume: &[u8], bare: &[u8], cameras: &[ViewRect]) -> Value {
    let mut rows=Vec::new();
    for (i,v) in cameras.iter().enumerate() {
        let mut hist=[0usize;256];let mut sum=0u64;let mut changed=0usize;
        let mut source_changed=0usize;let mut volume_changed=0usize;let mut source_sum=[0u64;3];let mut volume_sum=[0u64;3];
        for y in v.y..v.y+v.h {for x in v.x..v.x+v.w {
            let at=((y*W+x)*4)as usize;let mut max=0u8;let mut a=0u8;let mut b=0u8;
            for c in 0..3 {
                let d=source[at+c].abs_diff(volume[at+c]);hist[d as usize]+=1;sum+=u64::from(d);max=max.max(d);
                a=a.max(source[at+c].abs_diff(bare[at+c]));b=b.max(volume[at+c].abs_diff(bare[at+c]));
                source_sum[c]+=u64::from(source[at+c]);volume_sum[c]+=u64::from(volume[at+c]);
            }
            changed+=usize::from(max>0);source_changed+=usize::from(a>2);volume_changed+=usize::from(b>2);
        }}
        let channels=(v.w*v.h*3)as usize;
        let percentile=|p:f64| {let target=(channels as f64*p).ceil()as usize;let mut seen=0;hist.iter().position(|n|{seen+=*n;seen>=target}).unwrap_or(255)};
        rows.push(json!({"view":i,"mean_abs_rgb_u8":sum as f64/channels as f64,"p95_channel_error":percentile(0.95),
            "p99_channel_error":percentile(0.99),"max_channel_error":hist.iter().rposition(|n|*n>0).unwrap_or(0),
            "different_pixels":changed,"source_rgb_difference_from_soil_gt2_pixels":source_changed,
            "volume_rgb_difference_from_soil_gt2_pixels":volume_changed,
            "source_mean_rgb":source_sum.map(|n|n as f64/f64::from(v.w*v.h)),
            "volume_mean_rgb":volume_sum.map(|n|n as f64/f64::from(v.w*v.h))}));
    }
    json!({"views":rows,"optical_parity_accepted":false,
        "scope":"Direct image differences, not a pass gate. Soil-difference pixel counts are contrast heuristics, not true geometric coverage. Native non-temporal rendering; point quadrature and mip homogenization are approximations."})
}
fn mass_error(reference: &[f64;13], candidate: &[f64;13]) -> f64 {
    reference.iter().zip(candidate).map(|(a,b)|(a-b).abs()/a.abs().max(1e-8)).fold(0.,f64::max)
}
fn provider_report(gpu: &Gpu, renderer: &Renderer, plan: &BrickPlan, expect_valid: bool)
    -> Result<(Value,Vec<u8>),String> {
    let (generation,levels,cells,status,bytes)=renderer.meadow_brick_fixture_buffers().ok_or("missing brick provider")?;
    if generation==0 {return Err("brick provider never encoded".into());}
    if read_buffer(gpu,&levels)?!=bytemuck::cast_slice::<_,u8>(&plan.levels) {return Err("GPU level descriptors differ from plan".into());}
    let state=read_buffer(gpu,&status)?;let mut publication=Vec::new();let mut any_samples=false;
    for (group,chunk) in state.chunks_exact(16).enumerate() {
        let words:[u32;4]=std::array::from_fn(|i|u32::from_le_bytes(chunk[i*4..i*4+4].try_into().unwrap()));
        if words[0]!=generation {return Err(format!("stale brick generation {group}: {words:?} expected{generation}"));}
        if expect_valid && (words[1]!=generation || words[2]!=0) {return Err(format!("unpublished/failed brick {group}: {words:?}"));}
        if !expect_valid && (words[1]!=0 || words[2]&2==0) {return Err(format!("invalid-Y brick was not rejected {group}: {words:?}"));}
        any_samples|=words[3]>0;
        publication.push(json!({"group":group,"generation":words[0],"ready_generation":words[1],"error_bits":words[2],"binned_samples":words[3]}));
    }
    if expect_valid && !any_samples {return Err("valid provider contains no source samples".into());}
    let raw=read_buffer(gpu,&cells)?;
    if raw.len()!=plan.total_cells as usize*std::mem::size_of::<GpuCell>() {return Err("unexpected optical allocation".into());}
    let values:Vec<GpuCell>=raw.chunks_exact(176).map(bytemuck::pod_read_unaligned).collect();
    let mut reports=Vec::new();let mut total_occupied=0;
    for (group,levels) in plan.levels.chunks_exact(MIPS).enumerate() {
        let mut finest=[0f64;13];let mut rows=Vec::new();
        for (mip,l) in levels.iter().enumerate() {
            let count=(l.dims_offset[0]*l.dims_offset[1]*l.dims_offset[2])as usize;
            let mut integral=[0f64;13];let mut occupied=0usize;
            for c in &values[l.dims_offset[3]as usize..][..count] {
                if !bytemuck::cast_slice::<GpuCell,f32>(std::slice::from_ref(c)).iter().all(|v|v.is_finite()) {
                    return Err(format!("nonfinite cell in group{group}/mip{mip}"));
                }
                let sigma:Vec<f32>=c.extinction.iter().flatten().copied().take(13).collect();
                if sigma.iter().any(|s|*s<0.) {return Err("negative extinction".into());}
                for axis in 0..13 {integral[axis]+=f64::from(sigma[axis])*f64::from(l.origin_cell[3]).powi(3);}
                if sigma.iter().any(|s|*s>0.) {
                    occupied+=1;
                    let sum:f32=c.normals.iter().map(|n|n[3]).sum();
                    if (sum-1.).abs()>1e-4 || c.normals.iter().any(|n|n[3]<0. || Vec3::from_slice(n).length()>1.0001)
                        || !(0.0..=1.0).contains(&c.albedo_rough[3]) || c.extinction[3][1]!=1.0 {
                        return Err("invalid occupied optical normal/material closure".into());
                    }
                }
            }
            if mip==0 {finest=integral;total_occupied+=occupied;}
            let error=mass_error(&finest,&integral);
            if expect_valid && error>MASS_RELATIVE_LIMIT {return Err(format!("directional mip mass mismatch group{group}/mip{mip}: {error}"));}
            rows.push(json!({"mip":mip,"cells":count,"occupied":occupied,"cell_metres":l.origin_cell[3],
                "integrated_directional_area_m2":integral,"max_relative_mass_error":error}));
        }
        reports.push(json!({"group":group,"mips":rows}));
    }
    if expect_valid && total_occupied==0 {return Err("all finest cells empty".into());}
    Ok((json!({"generation":generation,"gpu_bytes":bytes,"publication":publication,
        "finite":true,"finest_occupied_cells":total_occupied,"mass_relative_limit":MASS_RELATIVE_LIMIT,
        "groups":reports,"cells_sha256":digest(&raw)}),raw))
}
fn consumer_report(gpu: &Gpu, renderer: &Renderer, out: &Path, prefix: &str, invalid: bool) -> Result<Value,String> {
    let diagnostics=renderer.meadow_brick_fixture_diagnostics(gpu)?;
    let layers=renderer.meadow_brick_fixture_layers();
    // Persist every raw counter BEFORE interpreting any of them. A failed
    // traversal must not erase the measurements that explain its failure.
    let raw_rows:Vec<_>=diagnostics.iter().map(|d|json!({
        "view":d.view,"input_size":d.input_size,"groups":d.groups,"expected_generation":d.expected_generation,
        "invalid_groups":d.invalid_groups,"active_tiles":d.active_tiles,"scheduled_pixels":d.scheduled_pixels,
        "traced_rays":d.traced_rays,"max_tile_groups":d.max_tile_groups,"max_ray_groups":d.max_ray_groups,
        "max_cell_steps":d.max_cell_steps,"max_selected_mip":d.max_selected_mip,"cell_steps":d.cell_steps,
        "occupied_samples":d.occupied_samples,"failed_rays":d.failed_rays,"step_bound":d.step_bound,
        "step_limit_failures":d.step_limit_failures,"coordinate_failures":d.coordinate_failures,"progress_failures":d.progress_failures,
        "empty_skip_enabled":d.empty_skip_enabled,"skipped_empty_cells":d.skipped_empty_cells,"empty_skip_regions":d.empty_skip_regions,
        "empty_jump_enabled":d.empty_jump_enabled,"empty_jump_regions":d.empty_jump_regions,"empty_jump_fallbacks":d.empty_jump_fallbacks,
        "shared_provider_bytes":d.shared_provider_bytes,"target_bytes":d.target_bytes,
    })).collect();
    let raw_path=out.join(format!("{prefix}-consumer-raw.json"));
    write_json(&raw_path,&json!({"validation":"pending","expected_invalid":invalid,
        "diagnostic_view_count":diagnostics.len(),"layer_count":layers.len(),"views":raw_rows,
        "layer_sizes":layers.iter().map(|l|[l.width(),l.height()]).collect::<Vec<_>>(),
        "captured_image":format!("{prefix}-bricks.png")}))?;
    if diagnostics.len()!=4 || layers.len()!=4 {
        return Err(format!("{prefix}: expected4 independent views, diagnostics={} layers={}; raw {}",
            diagnostics.len(),layers.len(),raw_path.display()));
    }
    let mut rows=Vec::new();let mut total_covered=0usize;
    for (i,(d,layer)) in diagnostics.iter().zip(layers).enumerate() {
        if d.failed_rays!=0 || d.max_cell_steps>d.step_bound {
            return Err(format!("{prefix}: view{} failed_rays={} (step_limit={} coordinate={} progress={}) max_cell_steps={} step_bound={} traced_rays={} cell_steps={}; image {prefix}-bricks.png; raw {}",
                d.view,d.failed_rays,d.step_limit_failures,d.coordinate_failures,d.progress_failures,
                d.max_cell_steps,d.step_bound,d.traced_rays,d.cell_steps,raw_path.display()));
        }
        if d.skipped_empty_cells>d.cell_steps || d.empty_skip_regions>d.skipped_empty_cells
            || (!d.empty_skip_enabled && (d.skipped_empty_cells!=0 || d.empty_skip_regions!=0)) {
            return Err(format!("{prefix}: view{} inconsistent empty-skip counters; raw {}",d.view,raw_path.display()));
        }
        if d.invalid_groups!=if invalid {4} else {0} {
            return Err(format!("{prefix}: view{} invalid_groups={} expected{}; raw {}",
                d.view,d.invalid_groups,if invalid {4} else {0},raw_path.display()));
        }
        if d.input_size!=[VIEW_W,VIEW_H] || layer.width()!=VIEW_W || layer.height()!=VIEW_H {
            return Err(format!("{prefix}: view{} input_size={:?}, layer={}x{}, expected{VIEW_W}x{VIEW_H}; raw {}",
                d.view,d.input_size,layer.width(),layer.height(),raw_path.display()));
        }
        let raw=read_rgba16(gpu,&layer)?;let mut sum=0f64;let mut covered=0usize;let mut alpha=Vec::new();
        for pixel in raw.chunks_exact(8) {
            let p:[f32;4]=std::array::from_fn(|c|half(u16::from_le_bytes([pixel[c*2],pixel[c*2+1]])));
            if !p.iter().all(|v|v.is_finite()) || !(0.0..=1.001).contains(&p[3]) {return Err(format!("{prefix}: view{} nonfinite/invalid brick HDR output {p:?}; raw {}",d.view,raw_path.display()));}
            if invalid && p.iter().any(|v|*v!=0.) {return Err(format!("{prefix}: view{} unpublished provider produced a visible layer {p:?}; raw {}",d.view,raw_path.display()));}
            sum+=f64::from(p[3]);covered+=usize::from(p[3]>0.001);
            let a=(p[3].clamp(0.,1.)*255.).round()as u8;alpha.extend_from_slice(&[a,a,a,255]);
        }
        render::write_png(&out.join(format!("{prefix}-alpha-{i}.png")),VIEW_W,VIEW_H,&alpha)?;
        total_covered+=covered;
        rows.push(json!({"view":i,"generation":d.expected_generation,"groups":d.groups,"invalid_groups":d.invalid_groups,
            "active_tiles":d.active_tiles,"scheduled_pixels":d.scheduled_pixels,"traced_rays":d.traced_rays,
            "max_tile_groups":d.max_tile_groups,"max_ray_groups":d.max_ray_groups,"max_cell_steps":d.max_cell_steps,
            "max_selected_mip":d.max_selected_mip,"cell_steps":d.cell_steps,"occupied_samples":d.occupied_samples,
            "failed_rays":d.failed_rays,"step_limit_failures":d.step_limit_failures,
            "coordinate_failures":d.coordinate_failures,"progress_failures":d.progress_failures,
            "empty_skip_enabled":d.empty_skip_enabled,"skipped_empty_cells":d.skipped_empty_cells,"empty_skip_regions":d.empty_skip_regions,
            "empty_jump_enabled":d.empty_jump_enabled,"empty_jump_regions":d.empty_jump_regions,"empty_jump_fallbacks":d.empty_jump_fallbacks,
            "step_bound":d.step_bound,"provider_bytes":d.shared_provider_bytes,
            "target_bytes":d.target_bytes,"covered_pixels_alpha_gt_001":covered,"alpha_sum":sum}));
    }
    if !invalid && total_covered==0 {return Err("no visible volumetric coverage in any view".into());}
    Ok(json!(rows))
}

// This experiment changes only the consumer. The ordinary optical fixture
// above deliberately regenerates its field, so separate runs cannot establish
// bit-exact traversal parity: atomic producer reductions may round differently.
struct JumpCapture {
    image: Vec<u8>,
    layers: Vec<Vec<u8>>,
    stats: Vec<render::vegetation_brick_render::VegetationBrickStats>,
}
fn frozen_identity(gpu:&Gpu, renderer:&Renderer)->Result<Value,String> {
    let (generation,levels,cells,status,bytes)=renderer.meadow_brick_fixture_buffers().ok_or("missing frozen provider")?;
    Ok(json!({"generation":generation,"levels_sha256":digest(&read_buffer(gpu,&levels)?),
        "cells_sha256":digest(&read_buffer(gpu,&cells)?),"status_sha256":digest(&read_buffer(gpu,&status)?),"allocation_bytes":bytes}))
}
fn frozen_timing(stats:render::FrameStats)->Result<BTreeMap<String,f64>,String> {
    let mut rows=BTreeMap::new();
    for (name,ms) in stats.gpu_ms {
        if render::meadow_brick_field::PASS_NAMES.contains(&name) {
            return Err(format!("immutable consumer experiment unexpectedly encoded producer pass {name}"));
        }
        if !ms.is_finite() || ms<0. {return Err(format!("invalid GPU timestamp {name}: {ms}"));}
        *rows.entry(name.to_owned()).or_insert(0.)+=ms;
    }
    for name in ["meadow brick tiles","meadow brick integrate","meadow brick composite"] {
        if !rows.contains_key(name) {return Err(format!("missing actual consumer GPU timestamp: {name}"));}
    }
    Ok(rows)
}
fn jump_capture(gpu:&Gpu, renderer:&Renderer, image:Vec<u8>, generation:u32, jumping:bool)->Result<JumpCapture,String> {
    let stats=renderer.meadow_brick_fixture_diagnostics(gpu)?;
    let textures=renderer.meadow_brick_fixture_layers();
    if stats.len()!=4 || textures.len()!=4 || image.len()!=(W*H*4) as usize {
        return Err("immutable consumer experiment needs four actual views and the requested output size".into());
    }
    let mut layers=Vec::new();
    for (view,(d,texture)) in stats.iter().zip(textures).enumerate() {
        if d.view!=view || d.input_size!=[VIEW_W,VIEW_H] || texture.width()!=VIEW_W || texture.height()!=VIEW_H
            || d.groups!=4 || d.expected_generation!=generation
            || d.invalid_groups!=0 || d.failed_rays!=0 || d.max_cell_steps>d.step_bound
            || d.step_limit_failures!=0 || d.coordinate_failures!=0 || d.progress_failures!=0 {
            return Err(format!("invalid immutable-field consumer diagnostic: {d:?}"));
        }
        if d.empty_skip_enabled || d.skipped_empty_cells!=0 || d.empty_skip_regions!=0 || d.empty_jump_enabled!=jumping
            || (!jumping && (d.empty_jump_regions!=0 || d.empty_jump_fallbacks!=0)) {
            return Err(format!("consumer mode was not the requested true-jump-only mode: {d:?}"));
        }
        let bytes=read_rgba16(gpu,&texture)?;
        for pixel in bytes.chunks_exact(8) {
            let rgba:[f32;4]=std::array::from_fn(|c|half(u16::from_le_bytes([pixel[c*2],pixel[c*2+1]])));
            if rgba.iter().any(|v|!v.is_finite()) || !(0.0..=1.001).contains(&rgba[3]) {
                return Err(format!("invalid immutable consumer HDR pixel: {rgba:?}"));
            }
        }
        layers.push(bytes);
    }
    if stats.iter().all(|d|d.occupied_samples==0) {return Err("consumer comparison never integrated occupied source cells".into());}
    Ok(JumpCapture {image,layers,stats})
}
fn jump_stats(capture:&JumpCapture)->Value {
    json!(capture.stats.iter().zip(&capture.layers).map(|(d,raw)|json!({
        "view":d.view,"input_size":d.input_size,"generation":d.expected_generation,"groups":d.groups,
        "invalid_groups":d.invalid_groups,"active_tiles":d.active_tiles,"scheduled_pixels":d.scheduled_pixels,
        "traced_rays":d.traced_rays,"max_tile_groups":d.max_tile_groups,"max_ray_groups":d.max_ray_groups,
        "max_cell_steps":d.max_cell_steps,"max_selected_mip":d.max_selected_mip,"cell_steps":d.cell_steps,
        "occupied_samples":d.occupied_samples,"failed_rays":d.failed_rays,"step_bound":d.step_bound,
        "step_limit_failures":d.step_limit_failures,"coordinate_failures":d.coordinate_failures,"progress_failures":d.progress_failures,
        "empty_skip_enabled":d.empty_skip_enabled,"skipped_empty_cells":d.skipped_empty_cells,"empty_skip_regions":d.empty_skip_regions,
        "empty_jump_enabled":d.empty_jump_enabled,"empty_jump_regions":d.empty_jump_regions,"empty_jump_fallbacks":d.empty_jump_fallbacks,
        "actual_traversal_iterations":d.cell_steps+d.empty_jump_regions,"layer_rgba16_sha256":digest(raw),
        "provider_bytes":d.shared_provider_bytes,"target_bytes":d.target_bytes,
    })).collect::<Vec<_>>())
}
fn byte_comparison(a:&[u8],b:&[u8])->Value {
    let different=a.iter().zip(b).filter(|(a,b)|a!=b).count()+a.len().abs_diff(b.len());
    json!({"exact":different==0,"different_bytes":different,"reference_bytes":a.len(),"candidate_bytes":b.len(),
        "reference_sha256":digest(a),"candidate_sha256":digest(b)})
}
fn jump_parity(reference:&JumpCapture,candidate:&JumpCapture)->Value {
    let layers:Vec<_>=reference.layers.iter().zip(&candidate.layers).enumerate()
        .map(|(view,(a,b))|json!({"view":view,"comparison":byte_comparison(a,b)})).collect();
    let identities:Vec<_>=reference.stats.iter().zip(&candidate.stats).map(|(a,b)| {
        a.input_size==b.input_size && a.expected_generation==b.expected_generation && a.groups==b.groups
            && a.active_tiles==b.active_tiles && a.scheduled_pixels==b.scheduled_pixels && a.traced_rays==b.traced_rays
            && a.max_tile_groups==b.max_tile_groups && a.max_ray_groups==b.max_ray_groups
            && a.max_selected_mip==b.max_selected_mip && a.occupied_samples==b.occupied_samples
    }).collect();
    let image=byte_comparison(&reference.image,&candidate.image);
    let exact=reference.layers.len()==candidate.layers.len() && image["exact"]==true
        && layers.iter().all(|v|v["comparison"]["exact"]==true) && identities.iter().all(|v|*v);
    json!({"pass":exact,"raw_rgba16_layers":layers,"composited_rgba8":image,"view_work_identity":identities,
        "scope":"Exact byte equality, including raw premultiplied HDR and alpha. Same provider generation and buffers; no tolerance and no independent producer rerun."})
}
fn save_jump_failure(out:&Path,name:&str,reference:&JumpCapture,candidate:&JumpCapture,parity:&Value)->Result<(),String> {
    write_json(&out.join(format!("{name}-parity-failure.json")),parity)?;
    for (label,capture) in [("reference",reference),("candidate",candidate)] {
        save_image(out,&format!("{name}-{label}-failure"),&capture.image)?;
        for (view,bytes) in capture.layers.iter().enumerate() {
            std::fs::write(out.join(format!("{name}-{label}-view{view}.rgba16")),bytes).map_err(|e|e.to_string())?;
        }
    }
    write_json(&out.join("checkpoint.json"),&json!({"case":name,"phase":"exact_consumer_parity_failed",
        "report":format!("{name}-parity-failure.json"),"raw_rgba16_width":VIEW_W,"raw_rgba16_height":VIEW_H}))
}
fn consumer_only_timing(rows:&[BTreeMap<String,f64>])->Value {
    let filtered:Vec<_>=rows.iter().map(|row|row.iter().filter(|(name,_)|name.starts_with("meadow brick "))
        .map(|(name,value)|(name.clone(),*value)).collect()).collect();
    let mut result=timing(&filtered);
    result.as_object_mut().unwrap().remove("warmup_excluded");
    result["warmup_excluded_per_block"]=json!(WARMUP);
    result["scope"]=json!("Four-view consumer tiles/clear, integration and composition GPU timestamps only. Producer, startup, warmup, readbacks and image writes excluded. Pass sums are not whole-frame time or FPS.");
    result
}
fn jump_order(pose:usize)->[bool;4] {
    if pose%2==0 {[false,true,true,false]}else{[true,false,false,true]}
}
fn run_empty_jump_ab(out:&Path)->Result<Value,String> {
    let mut geometry=MeadowGeometry::cook_with_blade_budget(73,0.,MeadowBladeBudget::Blades120)?;
    for variant in 0..3 {geometry.lod_ranges[variant]=[geometry.lod_ranges[variant][0];3];geometry.lod_errors[variant]=[0.;3];}
    geometry.validate()?;let support=enclosure(&geometry)?;
    let samples=MeadowVoxelSamples::cook(&geometry,SampleConfig {spacing:0.16,max_per_triangle:4,..Default::default()})?;
    let plan=brick_plan(support,&samples,-1.)?;
    let cameras=views();let gpu=Gpu::new()?;
    let mut renderer=Renderer::new(&gpu,&scene(geometry.clone())?,&light(false));
    renderer.install_meadow_brick_fixture(&gpu,&samples,plan.clone(),1.)?;
    renderer.set_meadow_brick_fixture_empty_skip(false)?;
    renderer.set_debug_options(DebugRenderOptions::default())?;
    let configuration=json!({"schema":1,"fixture":"same immutable GPU field empty-space jump A/B","native":true,
        "source_blades_per_root":120,"output_size":[W,H],"view_size":[VIEW_W,VIEW_H],"views":4,
        "source_geometry_sha256":digest(bytemuck::cast_slice(&geometry.vertices)),"samples_sha256":digest(bytemuck::cast_slice(&samples.samples)),
        "producer_runs_per_pose":1,"consumer_lod_pixels":1.0,"old_empty_read_bypass":false,
        "warmup_per_block":WARMUP,"steady_per_block":AB_STEADY,"blocks_per_pose":4,"steady_samples_per_condition_per_pose":AB_STEADY*2,
        "order":"ABBA for even poses, BAAB for odd poses; A=reference, B=true coarse empty-space jump",
        "cameras":cameras.iter().map(|v|json!({"position":v.cam.pos.to_array(),"target":v.cam.target.to_array(),"fov_y":v.cam.fov_y,"near":v.cam.near,"rect":[v.x,v.y,v.w,v.h]})).collect::<Vec<_>>(),
        "scope":"Consumer parity and work avoidance only, using one actually GPU-produced field per pose. Optical parity against source triangles, temporal reconstruction, full-field scale and gameplay performance are not asserted."});
    write_json(&out.join("configuration.json"),&configuration)?;
    let mut cases=Vec::new();let mut saw_reduction=false;
    for (low,sun) in [(false,"noon"),(true,"low-sun")] {for (wind_index,time) in [1.25,4.75].into_iter().enumerate() {
        let name=format!("{sun}-wind-{wind_index}");let lighting=light(low);
        let wind=WindState {time_seconds:time,strength:0.6,direction:Vec2::new(0.8,0.6),..Default::default()};
        renderer.set_meadow_brick_fixture_frozen(false)?;renderer.set_meadow_brick_fixture_empty_jump(false)?;
        renderer.set_wind(&gpu,&wind)?;
        let (_,producer_stats)=renderer.try_render_texture_profiled(&gpu,&cameras,&lighting,W,H)?;
        if !render::meadow_brick_field::PASS_NAMES.iter().all(|name|producer_stats.gpu_ms.iter().any(|(n,_)|n==name)) {
            return Err("field generation was not instrumented before freeze".into());
        }
        let (provider,_)=provider_report(&gpu,&renderer,&plan,true)?;
        renderer.set_meadow_brick_fixture_frozen(true)?;
        let identity=frozen_identity(&gpu,&renderer)?;
        let generation=identity["generation"].as_u64().ok_or("missing generation")? as u32;
        let mut probes=Vec::new();
        for jumping in [false,true] {
            renderer.set_meadow_brick_fixture_empty_jump(jumping)?;
            let (image,stats)=renderer.try_render_texture_profiled(&gpu,&cameras,&lighting,W,H)?;
            frozen_timing(stats)?; // correctness frames are never steady samples
            probes.push(jump_capture(&gpu,&renderer,gpu.read_rgba8(&image),generation,jumping)?);
        }
        let parity=jump_parity(&probes[0],&probes[1]);
        let after_probe=frozen_identity(&gpu,&renderer)?;
        write_json(&out.join(format!("{name}-first-frame.json")),&json!({"provider":provider,"frozen_identity":identity,
            "provider_after_probe":after_probe,"provider_unchanged":after_probe==identity,
            "parity":parity,"reference":jump_stats(&probes[0]),"jump":jump_stats(&probes[1])}))?;
        if after_probe!=identity {return Err(format!("{name}: provider changed during first-frame consumer comparison"));}
        if parity["pass"]!=true {save_jump_failure(out,&name,&probes[0],&probes[1],&parity)?;return Err(format!("{name}: exact first-frame consumer parity failed"));}
        for (label,capture) in [("reference",&probes[0]),("jump",&probes[1])] {save_image(out,&format!("{name}-{label}"),&capture.image)?;}
        let order=jump_order(cases.len());
        let mut pooled:[Vec<BTreeMap<String,f64>>;2]=[Vec::new(),Vec::new()];let mut blocks=Vec::new();
        for (block,jumping) in order.into_iter().enumerate() {
            renderer.set_meadow_brick_fixture_empty_jump(jumping)?;
            let mut rows=Vec::new();let mut last=None;
            for frame in 0..WARMUP+AB_STEADY {
                let (image,stats)=renderer.try_render_texture_profiled(&gpu,&cameras,&lighting,W,H)?;
                let row=frozen_timing(stats)?;
                if frame>=WARMUP {rows.push(row);}
                last=Some(image);
            }
            let capture=jump_capture(&gpu,&renderer,gpu.read_rgba8(&last.unwrap()),generation,jumping)?;
            let check=jump_parity(&probes[0],&capture);
            if check["pass"]!=true {let label=format!("{name}-block-{block}");save_jump_failure(out,&label,&probes[0],&capture,&check)?;return Err(format!("{label}: exact steady consumer parity failed"));}
            let ref_steps:u64=probes[0].stats.iter().map(|d|d.cell_steps).sum();
            let actual_steps:u64=capture.stats.iter().map(|d|d.cell_steps+d.empty_jump_regions).sum();
            let jumped:u64=capture.stats.iter().map(|d|d.empty_jump_regions).sum();
            if jumping && jumped>0 && actual_steps<ref_steps {saw_reduction=true;}
            let after=frozen_identity(&gpu,&renderer)?;
            if after!=identity {return Err(format!("{name}/block{block}: immutable provider bytes or generation changed"));}
            let block_timing=consumer_only_timing(&rows);
            pooled[usize::from(jumping)].extend(rows.iter().cloned());
            let row=json!({"block":block,"jump":jumping,"warmup_excluded":WARMUP,"steady_samples":rows,
                "timing":block_timing,
                "consumer":jump_stats(&capture),"exact_parity":check,"provider_unchanged":true,
                "reference_traversal_iterations":ref_steps,"actual_traversal_iterations":actual_steps});
            write_json(&out.join(format!("{name}-block-{block}.json")),&row)?;blocks.push(row);
        }
        let row=json!({"name":name,"sun_lux":lighting.sun_lux,"sun_direction":lighting.sun_dir.to_array(),
            "wind_time":time,"wind_strength":wind.strength,"wind_direction":wind.direction.to_array(),
            "provider":provider,"frozen_identity":identity,"first_frame_parity":parity,
            "order":order,"blocks":blocks,"pooled_reference":consumer_only_timing(&pooled[0]),"pooled_jump":consumer_only_timing(&pooled[1])});
        write_json(&out.join(format!("{name}.json")),&row)?;cases.push(row);
        write_json(&out.join("checkpoint.json"),&json!({"phase":"immutable_consumer_case_complete","case":name,"completed_cases":cases.len()}))?;
    }}
    if !saw_reduction {return Err("true empty jumping did not reduce actual traversal iterations in any tested pose/view set".into());}
    Ok(json!({"pass":true,"mechanical_pass":true,"exact_consumer_parity":true,"optical_parity_accepted":false,
        "actual_traversal_reduction_exercised":true,"configuration":configuration,"cases":cases,
        "limits":["One enclosed flat field with four bricks and four native views.","Source optical approximation is unchanged and is not validated by consumer equality.",
            "Producer cost excluded deliberately; this experiment isolates traversal work, not total field rendering or gameplay FPS.","No previous-pose motion or DLSS claims."]}))
}

fn run(out: &Path) -> Result<Value,String> {
    let cook_start=Instant::now();
    let mut geometry=MeadowGeometry::cook_with_blade_budget(73,0.,MeadowBladeBudget::Blades120)?;
    let original_lod_triangles=geometry.lod_ranges.map(|v|v.map(|r|r[1]/3));
    // Independent finest-source oracle: force every selected source LOD to the
    // original finest stream. It still retains exactly120 blades/root.
    for variant in 0..3 {geometry.lod_ranges[variant]=[geometry.lod_ranges[variant][0];3];geometry.lod_errors[variant]=[0.;3];}
    geometry.validate()?;
    let support=enclosure(&geometry)?;
    let samples=MeadowVoxelSamples::cook(&geometry,SampleConfig {spacing:0.16,max_per_triangle:4,..Default::default()})?;
    let cook_ms=cook_start.elapsed().as_secs_f64()*1000.;
    let plan=brick_plan(support,&samples,-1.)?;
    let metadata=json!({"schema":1,"fixture":"bounded shared source meadow optical bricks","native":true,
        "requested_empty_skip":std::env::var("KERN_BRICK_EMPTY_SKIP").as_deref()==Ok("1"),
        "requested_empty_jump":std::env::var("KERN_BRICK_EMPTY_JUMP").as_deref()==Ok("1"),
        "empty_skip_counter_scope":"Selected optical-mip positive-length cell intervals whose reads/optics were bypassed using exactly empty ancestors. Logical face traversal remains included in cell_steps; cells are ray samples, not unique world cells.",
        "output_size":[W,H],"view_size":[VIEW_W,VIEW_H],"seed":73,"source_blades_per_root":120,
        "original_source_lod_triangles":original_lod_triangles,"oracle":"all source LOD ranges point to original finest triangles",
        "habitat_half_extent_m":HABITAT_HALF,"maximum_deformed_support_m":support,
        "covered_brick_half_extent_m":ROOT_CELL*8.,"habitat_plus_support_enclosed":true,
        "source_geometry_sha256":digest(bytemuck::cast_slice(&geometry.vertices)),
        "samples_sha256":digest(bytemuck::cast_slice(&samples.samples)),"source_sample_bytes":samples.bytes(),
        "samples_per_variant":samples.ranges.map(|r|r[1]),"sample_spacing_m":samples.config.spacing,
        "max_samples_per_triangle":samples.config.max_per_triangle,"sample_radius_m":samples.stats.max_sample_radius,
        "capped_triangles":samples.stats.capped_triangles,"source_area_m2":samples.stats.source_area,
        "packed_area_m2":samples.stats.packed_area,"cook_cpu_ms":cook_ms,"plan_sample_jobs":plan.sample_jobs,
        "plan_halo_roots":plan.halo_roots,"plan_root_evaluations":plan.root_count(),"plan_cells":plan.total_cells,
        "scope":"Actual finest-source point quadrature into current-pose world bricks. Not exact triangle/voxel intersection, stochastic union, full-field streaming, DLSS, or gameplay GI. No source deletion is authorized outside this completely enclosed diagnostic habitat."});
    write_json(&out.join("configuration.json"),&metadata)?;
    let source_scene=scene(geometry.clone())?;let volume_scene=scene(geometry.clone())?;
    let gpu=Gpu::new()?;let source=Renderer::new(&gpu,&source_scene,&light(false));
    let mut volume=Renderer::new(&gpu,&volume_scene,&light(false));
    volume.install_meadow_brick_fixture(&gpu,&samples,plan.clone(),1.)?;
    let options=DebugRenderOptions::default();source.set_debug_options(options)?;volume.set_debug_options(options)?;
    let cameras=views();let cameras_json:Vec<_>=cameras.iter().map(|v|json!({"position":v.cam.pos.to_array(),"target":v.cam.target.to_array(),
        "fov_y":v.cam.fov_y,"near":v.cam.near,"rect":[v.x,v.y,v.w,v.h]})).collect();
    let mut cases=Vec::new();let mut earlier_cells:Option<Vec<u8>>=None;let mut changed_wind_cells=0usize;
    let mut wind_max_relative_cell_difference=0f32;
    for (low,name) in [(false,"noon"),(true,"low-sun")] {for (time_index,time) in [1.25,4.75].into_iter().enumerate() {
        let name=format!("{name}-wind-{time_index}");let lighting=light(low);
        let wind=WindState {time_seconds:time,strength:0.6,direction:Vec2::new(0.8,0.6),..Default::default()};
        let (source_image,source_timing)=render_case(&gpu,&source,&cameras,&lighting,&wind)?;
        write_json(&out.join(format!("{name}-source-timing.json")),&source_timing)?;
        save_image(out,&format!("{name}-source"),&source_image)?;
        let source_counts=source.meadow_diagnostics(&gpu)?;
        if source_counts.len()!=4 || source_counts.iter().any(|d|d.aggregate_enabled) || source_counts.iter().all(|d|d.plants.iter().all(|n|*n==0)) {
            return Err("source oracle absent or aggregate unexpectedly enabled".into());
        }
        let (volume_image,volume_timing)=render_case(&gpu,&volume,&cameras,&lighting,&wind)?;
        write_json(&out.join(format!("{name}-bricks-timing.json")),&volume_timing)?;
        save_image(out,&format!("{name}-bricks"),&volume_image)?;
        let (provider,raw)=provider_report(&gpu,&volume,&plan,true)?;
        write_json(&out.join(format!("{name}-provider.json")),&provider)?;
        if !low && time_index==0 {earlier_cells=Some(raw.clone());}
        if !low && time_index==1 {
            changed_wind_cells=earlier_cells.as_ref().unwrap().chunks_exact(176).zip(raw.chunks_exact(176)).filter(|(a,b)|a!=b).count();
            for (a,b) in earlier_cells.as_ref().unwrap().chunks_exact(4).zip(raw.chunks_exact(4)) {
                let a=f32::from_le_bytes(a.try_into().unwrap());let b=f32::from_le_bytes(b.try_into().unwrap());
                wind_max_relative_cell_difference=wind_max_relative_cell_difference.max((a-b).abs()/a.abs().max(b.abs()).max(1.));
            }
            if changed_wind_cells==0 || wind_max_relative_cell_difference<0.001 {
                return Err("different wind times did not materially change the optical cells".into());
            }
        }
        source.set_debug_options(DebugRenderOptions {meadow_enabled:false,..options})?;
        let bare=gpu.read_rgba8(&source.try_render_texture(&gpu,&cameras,&lighting,W,H)?);
        source.set_debug_options(options)?;
        let difference:Vec<u8>=source_image.chunks_exact(4).zip(volume_image.chunks_exact(4))
            .flat_map(|(a,b)|[a[0].abs_diff(b[0]).saturating_mul(4),a[1].abs_diff(b[1]).saturating_mul(4),a[2].abs_diff(b[2]).saturating_mul(4),255]).collect();
        render::write_png(&out.join(format!("{name}-difference-x4.png")),W,H,&difference)?;
        let counts:Vec<_>=source_counts.iter().map(|d|json!({"view":d.view,"plants":d.plants,"triangles":d.triangles,
            "density":d.density,"size_scale":d.size_scale,"color_multiplier":d.color_multiplier})).collect();
        let optics=optical_difference(&source_image,&volume_image,&bare,&cameras);
        write_json(&out.join(format!("{name}-optics.json")),&optics)?;
        let mut row=json!({"name":name,"validation":"consumer_pending","cameras":cameras_json,"wind":{"time_seconds":time,"strength":wind.strength,"direction":wind.direction.to_array(),"grass_shear":wind.grass_shear},
            "lighting":{"sun_direction":lighting.sun_dir.to_array(),"sun_lux":lighting.sun_lux,"sky_lux":lighting.sky_lux,"ev100":lighting.ev100,
                "gi":lighting.gi,"near_shadows":lighting.near_shadows,"fog_density":lighting.fog_density,"taa":lighting.taa},
            "source_counts":counts,"source_timing":source_timing,"brick_timing":volume_timing,"provider":provider,
            "images":{"source_sha256":digest(&source_image),"brick_sha256":digest(&volume_image)},
            "optics":optics});
        // A complete pre-validation checkpoint survives failed ray counters.
        write_json(&out.join(format!("{name}.json")),&row)?;
        write_json(&out.join("checkpoint.json"),&json!({"case":name,"phase":"consumer_pending",
            "case_report":format!("{name}.json"),"source_image":format!("{name}-source.png"),
            "brick_image":format!("{name}-bricks.png"),"completed_cases":cases.len()}))?;
        match consumer_report(&gpu,&volume,out,&name,false) {
            Ok(consumer)=>{row["consumer"]=consumer;row["validation"]=json!("passed");},
            Err(error)=>{
                row["validation"]=json!("consumer_failed");row["error"]=json!(error);
                write_json(&out.join(format!("{name}.json")),&row)?;
                write_json(&out.join("checkpoint.json"),&json!({"case":name,"phase":"consumer_failed","error":error,
                    "case_report":format!("{name}.json"),"raw_consumer":format!("{name}-consumer-raw.json"),
                    "source_image":format!("{name}-source.png"),"brick_image":format!("{name}-bricks.png"),
                    "completed_cases":cases.len()}))?;
                return Err(error);
            }
        }
        write_json(&out.join(format!("{name}.json")),&row)?;cases.push(row);
    }}
    // One additional, explicitly coarse diagnostic exercises the consumer's
    // source-derived mip selection. Its single frame is not a timing sample.
    volume.install_meadow_brick_fixture(&gpu,&samples,plan.clone(),8.)?;
    let (coarse_texture,coarse_stats)=volume.try_render_texture_profiled(&gpu,&cameras,&light(false),W,H)?;
    write_json(&out.join("coarse-lod8-timing.json"),&json!({"gpu_ms":coarse_stats.gpu_ms,"scope":"single diagnostic frame, not steady timing"}))?;
    save_image(out,"coarse-lod8-bricks",&gpu.read_rgba8(&coarse_texture))?;
    let (coarse_provider,_)=provider_report(&gpu,&volume,&plan,true)?;
    write_json(&out.join("coarse-lod8-provider.json"),&coarse_provider)?;
    write_json(&out.join("checkpoint.json"),&json!({"case":"coarse-lod8","phase":"consumer_pending","completed_cases":cases.len()}))?;
    let coarse_consumer=consumer_report(&gpu,&volume,out,"coarse-lod8",false)?;
    if !coarse_consumer.as_array().unwrap().iter().any(|v|v["max_selected_mip"].as_u64().unwrap_or(0)>0) {
        return Err("coarse diagnostic failed to exercise any source-derived mip".into());
    }
    // The same actual source with a deliberately wrong vertical slab must
    // remain unpublished. There is no hidden clamp, partial-cell acceptance,
    // or accidental use of last frame's valid output.
    let invalid_plan=brick_plan(support,&samples,4.)?;
    volume.install_meadow_brick_fixture(&gpu,&samples,invalid_plan.clone(),1.)?;
    let (invalid_texture,invalid_stats)=volume.try_render_texture_profiled(&gpu,&cameras,&light(false),W,H)?;
    write_json(&out.join("invalid-y-timing.json"),&json!({"gpu_ms":invalid_stats.gpu_ms,"scope":"single diagnostic frame, not steady timing"}))?;
    save_image(out,"invalid-y-bricks",&gpu.read_rgba8(&invalid_texture))?;
    let (invalid_provider,_)=provider_report(&gpu,&volume,&invalid_plan,false)?;
    write_json(&out.join("invalid-y-provider.json"),&invalid_provider)?;
    write_json(&out.join("checkpoint.json"),&json!({"case":"invalid-y","phase":"consumer_pending","completed_cases":cases.len()}))?;
    let invalid_consumer=consumer_report(&gpu,&volume,out,"invalid-y",true)?;
    Ok(json!({"pass":true,"mechanical_pass":true,"optical_parity_accepted":false,"configuration":metadata,
        "cases":cases,"wind_changed_finest_or_mip_cells":changed_wind_cells,
        "wind_max_relative_cell_difference":wind_max_relative_cell_difference,
        "coarse_mip_diagnostic":{"lod_pixels":8.,"provider":coarse_provider,"consumer":coarse_consumer,"gpu_ms":coarse_stats.gpu_ms},
        "invalid_y":{"pass":true,"provider":invalid_provider,"consumer":invalid_consumer,"gpu_ms":invalid_stats.gpu_ms},
        "remaining_limits":["Bounded four-brick field only; no general source fallback or streaming.",
            "Native current-pose rendering; no previous-pose volume motion or DLSS validation.",
            "Point area quadrature, normal moments and directional extinction do not preserve exact source silhouette/union coverage.",
            "GPU pass times include producer and all four consumers; no gameplay FPS or8.33ms claim."]}))
}
fn main() {
    let args:Vec<_>=std::env::args().collect();
    let empty_jump_ab=args.iter().any(|arg|arg=="--empty-jump-ab");
    let optical_lod=args.iter().any(|arg|arg=="--optical-lod");
    let optical_night=args.iter().any(|arg|arg=="--optical-night");
    let optical_sweep=args.iter().any(|arg|arg=="--optical-sweep");
    let optical=optical_lod || optical_night || optical_sweep;
    if [empty_jump_ab,optical_lod,optical_night,optical_sweep].into_iter().filter(|mode|*mode).count()>1 {
        eprintln!("choose only one fixture experiment mode");std::process::exit(2);
    }
    let Some(out)=args.windows(2).find(|p|p[0]=="--out").map(|p|std::path::PathBuf::from(&p[1])) else {
        eprintln!("usage: meadow_brick_field_gpu --out FRESH_DIR [--empty-jump-ab | --optical-lod | --optical-night | --optical-sweep]");std::process::exit(2);
    };
    if out.exists() {eprintln!("output directory already exists: {}",out.display());std::process::exit(2);}
    if let Err(e)=std::fs::create_dir_all(&out) {eprintln!("{e}");std::process::exit(2);}
    // Single-threaded process startup, before wgpu or any renderer threads.
    // The fixture records fixed controls instead of inheriting live experiments.
    unsafe {
        for (name,value) in [("KERN_MEADOW_AGGREGATE","0"),("KERN_MEADOW_GROUP_CACHE","0"),
            ("KERN_MEADOW_GROUP_REPLACE","0"),("KERN_MEADOW_SUBSTRATE","0"),
            ("KERN_TREE_OCCLUSION","0"),("KERN_STATIC_SUN_CACHE","0")] {std::env::set_var(name,value);}
        std::env::remove_var("KERN_DIAGNOSTIC_HIDE");
        if empty_jump_ab || optical {
            // Both consumer modes are selected explicitly in this process.
            std::env::set_var("KERN_BRICK_EMPTY_SKIP","0");
            std::env::set_var("KERN_BRICK_EMPTY_JUMP","0");
        }
        if optical {
            std::env::set_var("KERN_BRICK_SMOOTH_MIPS","0");
            std::env::set_var("KERN_MEADOW_VISIBILITY","1");
            std::env::set_var("KERN_MEADOW_SKIP_HIDDEN_SHADE","0");
        }
    }
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
        if optical_sweep {run_optical_sweep(&out)} else if optical {run_optical_lod(&out,optical_night)}
        else if empty_jump_ab {run_empty_jump_ab(&out)} else {run(&out)}))
        .unwrap_or_else(|panic|Err(panic.downcast_ref::<String>().cloned()
            .or_else(||panic.downcast_ref::<&str>().map(|s|(*s).to_owned()))
            .unwrap_or_else(||"GPU fixture panicked".to_owned())));
    match result {
        Ok(report)=>{if let Err(e)=write_json(&out.join("report.json"),&report) {eprintln!("{e}");std::process::exit(1);}
            let _=write_json(&out.join("checkpoint.json"),&json!({"phase":"complete","final_status":"passed","report":"report.json"}));
            println!("PASS {}: {}",if optical {"matched optical measurements completed; source replacement and optical parity are not accepted"}
                else if empty_jump_ab {"same-field consumer equality and traversal checks; source optical parity is not asserted"}
                else {"mechanical brick fixture; optical differences recorded, not parity accepted"},out.display());}
        Err(error)=>{
            let checkpoint_path=out.join("checkpoint.json");
            let mut checkpoint=std::fs::read(&checkpoint_path).ok()
                .and_then(|bytes|serde_json::from_slice::<Value>(&bytes).ok()).filter(Value::is_object).unwrap_or_else(||json!({}));
            checkpoint["final_status"]=json!("failed");checkpoint["error"]=json!(error);
            let _=write_json(&checkpoint_path,&checkpoint);
            let _=write_json(&out.join("report.json"),&json!({"pass":false,"error":error,"partial_evidence_checkpoint":"checkpoint.json"}));
            eprintln!("{error}");std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optical_sky_commits_consistent_radiance_and_sh_at_night_without_changing_exposure() {
        for index in 0..3 {
            let light = optical_light(index);
            let sky = optical_sky(&light).unwrap();
            sky.validate().unwrap();
            let sh_irradiance = Vec3::from_slice(&sky.sh[0]) * 0.886227;
            let texture_irradiance = Vec3::from_slice(&sky.pixels[0]) * std::f32::consts::PI;
            assert!(sh_irradiance.abs_diff_eq(sky.sky_horizontal_rgb_lux, 0.002));
            assert!(texture_irradiance.abs_diff_eq(sky.sky_horizontal_rgb_lux, 0.002));
            assert_eq!(sky.sky_horizontal_rgb_lux.y, light.sky_lux);
            assert_eq!(sky.sun_rgb_lux, light.sun_color * light.sun_lux);
        }
        let night = optical_sky(&optical_light(2)).unwrap();
        let noon = optical_sky(&optical_light(0)).unwrap();
        assert!((night.sh[0][1] / noon.sh[0][1] - 10.0 / 18000.0).abs() < 1e-8);
        assert!(night.pixels[0][1] < 4.0);
        assert_eq!(optical_light(2).ev100, 6.0);
        assert_eq!(night.night_stars, [0.0;4]);
    }
    #[test]
    fn sweep_brackets_full_four_pixel_transition_without_source_distance_fade() {
        use render::vegetation_brick_render::select_brick_mips;
        for step in 0..SWEEP_STEPS {
            for (view,ss) in sweep_views(step,1).iter().zip(sweep_views(step,2)) {
                assert_eq!(view.cam.pos,ss.cam.pos);
                assert_eq!(optical_projection(view),optical_projection(&ss));
                for x in [-HABITAT_HALF-0.018,HABITAT_HALF+0.018] {
                    for z in [-HABITAT_HALF-0.018,HABITAT_HALF+0.018] {
                        assert!(view.cam.pos.xz().distance(Vec2::new(x,z)) < 60.0);
                    }
                }
            }
        }
        let first = sweep_views(0,1);
        let last = sweep_views(SWEEP_STEPS-1,1);
        for (a,b) in first.iter().zip(last) {
            // The previous35m start checked only the centre ray; the far
            // corners of a20-degree view could already choose mip1. The
            // projected Jacobian is smallest on the centre ray. Therefore
            // this MINIMUM first-mip entry threshold plus the maximum norm
            // over the enclosing box corners bounds every possible entry.
            let diameter = 0.1125f64 * 3.0f64.sqrt();
            let focal_pixels = VIEW_H as f64 / (2.0 * (a.cam.fov_y as f64 * 0.5).tan());
            let minimum_coarse_entry = 2.0 * (diameter * focal_pixels * 2.0f64.sqrt() + 4.0 * diameter * 0.5) / 4.0;
            for x in [-3.6,3.6] {for y in [-1.0,1.7] {for z in [-3.6,3.6] {
                let corner = glam::DVec3::new(x,y,z);
                assert!(a.cam.pos.as_dvec3().distance(corner) < minimum_coarse_entry,
                    "starting brick corner can already reach mip1");
            }}}
            let start = select_brick_mips(optical_projection(a),[VIEW_W,VIEW_H],a.cam.target-a.cam.pos,
                a.cam.pos.distance(a.cam.target),0.1125,4.0,true).unwrap();
            let end = select_brick_mips(optical_projection(&b),[VIEW_W,VIEW_H],b.cam.target-b.cam.pos,
                b.cam.pos.distance(b.cam.target)-5.2,0.1125,4.0,true).unwrap();
            assert_eq!((start.fine_mip,start.coarse_mip),(0,0));
            assert_eq!((end.fine_mip,end.coarse_mip),(1,1));
        }
        assert_eq!(sweep_distance(0),32.0);
        assert_eq!(sweep_distance(SWEEP_STEPS-1),53.0);
    }
    #[test]
    fn sweep_alpha_residual_removes_identical_camera_coverage_change() {
        let old_source = vec![vec![[0.0,0.0,0.0,0.2],[0.0;4]]];
        let new_source = vec![vec![[0.0;4],[0.0,0.0,0.0,0.6]]];
        let equal = sweep_residual(&old_source,&old_source,&new_source,&new_source);
        assert_eq!(equal[0]["mean_absolute_alpha_residual_change"],0.0);
        let mut changed = new_source.clone();
        changed[0][1][3] = 0.8;
        let delta = sweep_residual(&old_source,&old_source,&new_source,&changed);
        assert!((delta[0]["mean_absolute_alpha_residual_change"].as_f64().unwrap()-0.1).abs()<1e-7);
        assert_eq!(delta[0]["union_support_pixels"],2);
    }
    #[test]
    fn optical_supersample_preserves_fractional_union_and_premultiplied_radiance() {
        let pixels = [[2.0, 1.0, 0.5, 1.0], [0.0; 4], [0.0; 4], [0.0; 4]];
        let averaged = downsample_layer(&pixels, 2, 2, 2).unwrap();
        assert_eq!(averaged, vec![[0.5, 0.25, 0.125, 0.25]]);
        assert_eq!(averaged[0][0] / averaged[0][3], 2.0);
        assert!(downsample_layer(&pixels, 2, 2, 0).is_err());
        assert!(downsample_layer(&pixels, 2, 2, 3).is_err());
        assert!(downsample_layer(&pixels[..3], 2, 2, 2).is_err());
    }
    #[test]
    fn optical_roi_cannot_dilute_loss_with_background_or_authorize_replacement() {
        let mut source = vec![[0.0; 4]; 100];
        let mut volume = source.clone();
        source[44] = [0.2, 0.3, 0.1, 0.5];
        volume[44] = [0.1, 0.15, 0.05, 0.25];
        let source_image = vec![40; 400];
        let mut volume_image = source_image.clone();
        volume_image[44 * 4] = 70;
        // Differences outside both vegetation masks must not enter the metric.
        volume_image[0] = 255;
        let report = roi_metrics(&source, &volume, &source_image, &volume_image, 10, 10).unwrap();
        assert_eq!(report["roi_union_pixels"], 1);
        assert_eq!(report["roi_bounds_xyxy"], json!([4,4,5,5]));
        assert_eq!(report["coverage_delta_percent"], -50.0);
        assert_eq!(report["roi_mean_abs_rgb_u8"], 10.0);
        assert_eq!(report["soft_coverage_iou"], 0.5);
        assert!(report["conditional_hdr_luma_delta_percent"].as_f64().unwrap().abs() < 1e-8);
        assert_eq!(report["optical_parity_accepted"], false);
        assert_eq!(report["replacement_authorized"], false);
        volume[45] = [0.1, 0.15, 0.05, 0.25];
        let shifted = roi_metrics(&source, &volume, &source_image, &volume_image, 10, 10).unwrap();
        assert_eq!(shifted["coverage_delta_percent"], 0.0);
        assert_eq!(shifted["missing_fractional_coverage_pixels"], 0.25);
        assert_eq!(shifted["extra_fractional_coverage_pixels"], 0.25);
        assert!(shifted["soft_coverage_iou"].as_f64().unwrap() < 0.34);
        volume[0][0] = f32::NAN;
        assert!(roi_metrics(&source, &volume, &source_image, &volume_image, 10, 10).is_err());
    }
    #[test]
    fn optical_layer_rejects_nonfinite_half_values_and_malformed_sizes() {
        assert!(decode_layer(&[0; 7], 1, 1).is_err());
        let mut pixel = [0u8; 8];
        pixel[..2].copy_from_slice(&0x7c00u16.to_le_bytes()); // positive infinity
        assert!(decode_layer(&pixel, 1, 1).is_err());
        pixel[..2].copy_from_slice(&0x8001u16.to_le_bytes()); // negative subnormal radiance
        assert!(decode_layer(&pixel, 1, 1).is_err());
        pixel[..2].fill(0);
        pixel[6..8].copy_from_slice(&0x4000u16.to_le_bytes()); // alpha2
        assert!(decode_layer(&pixel, 1, 1).is_err());
    }
    #[test]
    fn optical_views_match_supersample_rays_and_remain_inside_source_fade() {
        let views = optical_views(1);
        for (native, ss) in views.iter().zip(optical_views(2)) {
            assert_eq!(native.cam.pos, ss.cam.pos);
            assert_eq!(native.cam.target, ss.cam.target);
            assert_eq!(optical_projection(native), optical_projection(&ss));
            assert_eq!([ss.w, ss.h], [native.w * 2, native.h * 2]);
            // Root selection/fade uses horizontal root distance, not canopy
            // extent. Test every extreme habitat root, including jitter bound.
            for x in [-HABITAT_HALF - 0.018, HABITAT_HALF + 0.018] {
                for z in [-HABITAT_HALF - 0.018, HABITAT_HALF + 0.018] {
                    assert!(native.cam.pos.xz().distance(Vec2::new(x, z)) < 60.0);
                }
            }
        }
        assert!(views[1].cam.pos.y < 1.1); // genuine grazing case
        assert_eq!(optical_light(2).sun_lux, 0.0);
        assert_eq!(optical_light(2).sky_lux, 10.0);
    }
    fn fake_capture()->JumpCapture {
        JumpCapture {image:vec![17,23,41,255],layers:vec![vec![0;8];4],stats:(0..4).map(|view|
            render::vegetation_brick_render::VegetationBrickStats {view,input_size:[VIEW_W,VIEW_H],groups:4,
                expected_generation:7,occupied_samples:5,cell_steps:80,..Default::default()}).collect()}
    }
    #[test]
    fn immutable_parity_detects_one_hdr_byte_and_missing_work_without_tolerance() {
        let reference=fake_capture();let mut candidate=fake_capture();
        candidate.stats[0].cell_steps=20;candidate.stats[0].empty_jump_regions=4;
        assert_eq!(jump_parity(&reference,&candidate)["pass"],true);
        candidate.layers[2][3]=1;
        assert_eq!(jump_parity(&reference,&candidate)["pass"],false);
        candidate.layers[2][3]=0;candidate.stats[1].occupied_samples-=1;
        assert_eq!(jump_parity(&reference,&candidate)["pass"],false);
        candidate.stats[1].occupied_samples+=1;candidate.image[0]+=1;
        assert_eq!(jump_parity(&reference,&candidate)["pass"],false);
        assert_eq!(byte_comparison(&[1,2],&[1])["different_bytes"],1);
    }
    #[test]
    fn frozen_timing_rejects_producer_and_reports_consumer_only_gpu_cost() {
        let mut stats=render::FrameStats::default();
        stats.gpu_ms=vec![("meadow brick tiles",0.2),("meadow brick integrate",1.),("meadow brick composite",0.1),("shade",9.)];
        let row=frozen_timing(stats.clone()).unwrap();
        let report=consumer_only_timing(&[row]);
        assert!((report["timed_pass_sum_median_ms"].as_f64().unwrap()-1.3).abs()<1e-9);
        assert!(report["passes"].get("shade").is_none());
        stats.gpu_ms.push((render::meadow_brick_field::PASS_NAMES[0],0.01));
        assert!(frozen_timing(stats).is_err());
    }
    #[test]
    fn jump_order_balances_both_modes_in_every_block_position() {
        let mut jumps=[0;4];
        for pose in 0..4 {
            let order=jump_order(pose);
            assert_eq!(order.iter().filter(|mode|**mode).count(),2);
            assert_eq!(order[0],order[3]);assert_eq!(order[1],order[2]);
            for (position,mode) in order.into_iter().enumerate() {jumps[position]+=usize::from(mode);}
        }
        assert_eq!(jumps,[2;4]);
    }
    #[test]
    fn directional_mass_oracle_detects_lost_cells_without_normalizing_it_away() {
        let original=std::array::from_fn(|i|1.0+i as f64*0.31);
        let mut lost=original;lost[7]*=0.99;
        assert!(mass_error(&original,&lost)>MASS_RELATIVE_LIMIT);
        let rounded=original.map(|x|x*(1.+1e-6));
        assert!(mass_error(&original,&rounded)<MASS_RELATIVE_LIMIT);
    }
    #[test]
    fn actual_recipe_habitat_and_support_fit_all_four_bricks() {
        let geometry=MeadowGeometry::cook_with_blade_budget(73,0.,MeadowBladeBudget::Blades120).unwrap();
        let support=enclosure(&geometry).unwrap();
        assert!(support>geometry.max_anchor_radius);
        let mut invalid=geometry;invalid.max_blade_radius=4.;
        assert!(enclosure(&invalid).is_err());
        let half=ROOT_CELL*8.;
        for root in [-HABITAT_HALF,HABITAT_HALF] {
            for extent in [-support,support] {assert!((root+extent).abs()<half);}
        }
    }
}
