//! Opt-in interactive application. `shot` and benchmarks remain headless.
//! `play --smoke-render N` creates an invisible window for bounded validation.

#[path="stage_loading.rs"]
mod stage_loading;
#[path="frame_profile.rs"]
mod frame_profile;
#[path="scenario.rs"]
mod scenario;
#[path="cli_startup.rs"]
mod cli_startup;
#[path="render_diagnostic.rs"]
mod render_diagnostic;

use crate::{input::{Input, SeatInput}, visuals::{self, Built, CarPose}};
use glam::{Quat, Vec3};
use kern::Timings;
use render::{Camera, Gpu, Lighting, Renderer};
use sideways::race::{Control, Race, Settings};
use crate::gamenight::{Client, Command, Seat};
use std::{fs::OpenOptions, io::Write, path::Path, sync::Arc, time::{Duration, Instant}};
use winit::{application::ApplicationHandler, dpi::PhysicalSize, event::{ElementState, WindowEvent, MouseButton, MouseScrollDelta}, event_loop::{ActiveEventLoop, ControlFlow, EventLoop}, keyboard::{KeyCode, PhysicalKey}, window::{Fullscreen, Window, WindowId}};
use render::debug_ui::DebugKey;

#[derive(Clone)]
struct Args {
    upscale: render::upscale::UpscaleMode,
    nature: crate::nature::Nature,
    gpu_profile: Option<String>, gpu_warmup: usize, frame_profile:Option<String>,
    seed: u64, km: f64, views: usize, size: (u32, u32), weather: String,
    smoke_next_stage: bool,
    smoke_frames: u64, smoke_ticks: u64, capture: Option<String>, state_out: Option<String>, trace: Option<String>, fullscreen: bool,
    hud: bool, hud_profile: Option<String>, hud_warmup: usize,
    autoplay: bool,
    debug_ui: bool, debug_ui_script:Option<String>, start_paused:bool,
    /// Engine sound: off, pulse, or fluid for the players' own cars.
    sound: crate::audio_out::Mode,
}

/// Presentation time is independent of car simulation: losing keyboard focus
/// must freeze driving, but must not freeze foliage in a still-visible window.
/// Hidden smoke runs retain their exact 120 Hz simulation clock.
#[derive(Clone, Copy, Debug)]
struct AmbientClock {
    seconds: f64,
    simulation_time: bool,
}

impl AmbientClock {
    fn new(smoke: bool) -> Self { Self { seconds: 0.0, simulation_time: smoke } }

    fn advance(&mut self, dt: f64, simulation_frame: u64, visible: bool, running: bool, paused: bool) {
        if self.simulation_time {
            self.seconds = simulation_frame as f64 / 120.0;
        } else if visible && running && !paused && dt.is_finite() {
            // Match the frame loop's stall bound; resuming a hidden/paused
            // window must never catch up all the time spent away.
            self.seconds += dt.clamp(0.0, 0.25);
        }
    }

    fn wind(self, nature: crate::nature::Nature) -> render::wind::WindState {
        nature.wind(self.seconds)
    }

    fn metadata(self, nature: crate::nature::Nature) -> serde_json::Value {
        let mut metadata = nature.metadata(self.seconds);
        if nature.wind_time.is_none() && !self.simulation_time {
            metadata["wind"]["time_source"] = "visible_animation_clock".into();
        }
        metadata
    }
}

// Constructor switches for matched same-binary graphics comparisons. These
// are not runtime menu controls and do not claim visual acceptance.
fn foliage_variant_flags() -> serde_json::Value {
    let enabled = |key| std::env::var(key).as_deref() != Ok("0");
    serde_json::json!({
        "linear_cutout_rgb": enabled("KERN_CUTOUT_LINEAR_RGB"),
        "grass_leaf_brdf": enabled("KERN_GRASS_LEAF_BRDF"),
        "grass_sun_occlusion": enabled("KERN_GRASS_SUN_OCCLUSION")
    })
}

fn foliage_sun_metadata(stats:&render::FrameStats)->serde_json::Value {
    serde_json::json!({"prepare_cpu_ms":stats.foliage_sun_prepare_cpu_ms,
        "index_rebuilt":stats.foliage_sun_index_rebuilt,"revision":stats.foliage_sun_revision,
        "requested_pages":stats.foliage_sun_requested_pages,"resident_pages":stats.foliage_sun_resident_pages,
        "missing_pages":stats.foliage_sun_missing_pages,"updated_pages":stats.foliage_sun_updated_pages,
        "cache_bytes":stats.foliage_sun_cache_bytes,"clusters":stats.foliage_sun_clusters,"triangles":stats.foliage_sun_triangles})
}

fn static_sun_metadata(stats:&render::FrameStats)->serde_json::Value {
    serde_json::json!({"enabled":stats.static_sun_cache,"redrawn":stats.static_sun_redrawn,
        "reused":stats.static_sun_reused,"seed_bytes":stats.static_sun_seed_bytes,
        "gpu_bytes":stats.static_sun_gpu_bytes,"static_casters":stats.static_sun_casters,
        "dynamic_casters":stats.dynamic_sun_casters,"static_clusters":stats.static_sun_clusters,
        "static_triangles":stats.static_sun_triangles,"dynamic_clusters":stats.dynamic_sun_clusters,
        "dynamic_triangles":stats.dynamic_sun_triangles,"static_submitted_vertices":stats.static_sun_submitted_vertices,
        "guard_m":stats.sun_cache_guard_m,"guard_recentered":stats.sun_cache_guard_recentered,
        "wind_freeze_m":stats.wind_freeze_m,"wind_freeze_repinned":stats.wind_freeze_repinned,
        "extent_m":stats.shared_sun_extent_m,"texel_m":stats.shared_sun_texel_m,
        "scope":"Shared source sun only. Owner counts are CPU partition membership. Selected geometry counts are available only in synchronized diagnostics; GPU pass timestamps are separate. Depth copy plus dynamic raster is exact min(static,dynamic); extra allocation is not a timing saving."})
}

/// Record the actual constructed pool and last-applied streaming band, not
/// environment variables which may no longer describe the rendered scene.
fn grass_stream_metadata(built: &Built) -> serde_json::Value {
    built.ground_cover_stats().map(|s| serde_json::json!({
        "scene_profile":built.scene.grass_detail.label(),
        "detail_profile":s.detail_profile.label(),
        "fade_start_m":s.fade_start,"fade_end_m":s.fade_end,"residency_m":s.residency,
        "capacity":s.capacity,"active":s.active,"requested":s.requested,
        "budget_clipped":s.budget_clipped,"visible_budget_clipped":s.visible_budget_clipped,
        "cached_tiles":s.cached_tiles,"updates":s.updates,
        "last_update_ms":s.last_update_ms,"ground_bytes":s.ground_bytes,
        "capacity_scope":"shared_all_views"
    })).unwrap_or(serde_json::Value::Null)
}

/// Synchronous diagnostic only for an explicitly requested image capture.
/// These counts are separate from FrameStats' main visibility-buffer geometry:
/// meadow draws its own depth and colour passes, with no shadow-caster pass.
fn meadow_group_metadata(c:&render::meadow::GroupCacheStats)->serde_json::Value {
    serde_json::json!({"enabled":c.enabled,"frame":c.frame,"requested":c.requested,
        "hits":c.hits,"misses":c.misses,"cooked":c.cooked,"deferred":c.deferred,
        "resident":c.resident,"evictions":c.evictions,"active_pages":c.active_pages,
        "page_triangles":c.page_triangles,"producer_root_candidates":c.producer_root_candidates,
        "reference_roots_submitted":c.reference_roots_submitted,"update_budget":c.update_budget,
        "capacity":c.capacity,"gpu_bytes":c.gpu_bytes,"cpu_bounds_bytes":c.cpu_bounds_bytes,
        "request_cpu_ms":c.request_cpu_ms,"replacement_enabled":c.replacement_enabled,
        "certificate_pages":c.certificate_pages,"certified_pages":c.certified_pages,
        "useful_pages":c.useful_pages,"fallback_pages":c.fallback_pages,"certificate_pixels":c.certificate_pixels,
        "replaced_roots":c.replaced_roots,"saved_root_triangles":c.saved_root_triangles,
        "scope":"Shared scheduling/resident bytes; active pages and replacement counters per view. Certified replacement removes whole member roots before submission; uncertified pages retain reference roots. Saved root triangles are per pass, not pixels or GPU time. Surface pages, not 3D voxel transport."})
}

fn meadow_capture_metadata(renderer: &Renderer, gpu: &Gpu, scene: &render::Scene)
    -> Result<serde_json::Value, String> {
    let Some(field) = &scene.meadow else { return Ok(serde_json::Value::Null); };
    let stats = renderer.meadow_diagnostics(gpu)?;
    let pool_bytes: u64 = stats.iter().map(|s| s.pool_bytes).sum();
    let edge_shadow = renderer.meadow_edge_shadow_stats().map(|s| serde_json::json!({
        "enabled":s.enabled,"shared_bytes":s.shared_bytes,"radius_m":s.radius_m,
        "sky_strength":s.sky_strength,"sun_strength":s.sun_strength,"sun_reach_m":s.sun_reach_m,
        "fade_m":s.fade_m,"grid_dimension":s.grid_dimension,
        "scope":"Bounded GPU occupancy from the source's accepted roots and road-trimmed blade anchors. Separate soft ground/road ambient and short directional sun attenuation; no individual blade shadow map, no per-frame CPU root streaming."
    }));
    let views: Vec<_> = stats.iter().map(|s| {
        let mut value = serde_json::json!({
        "view":s.view,"visible_plants_per_lod":s.plants,"source_triangles_per_lod":s.triangles,
        "source_triangles_total":s.triangles.iter().sum::<u64>(),
        "depth_and_color_submitted_triangles_per_lod":s.triangles.map(|n|n*u64::from(s.source_geometry_passes)),
        "visibility_enabled":s.visibility_enabled,"visibility_target_bytes":s.visibility_target_bytes,
        "source_geometry_passes":s.source_geometry_passes,"submitted_source_triangles":s.submitted_source_triangles,
        "visibility_covered_pixels":s.visibility_covered_pixels,"visibility_unique_primitives":s.visibility_unique_primitives,
        "visibility_unique_roots":s.visibility_unique_roots,"visibility_invalid_ids":s.visibility_invalid_ids,
        "skip_hidden_shade_enabled":s.skip_hidden_shade_enabled,
        "opaque_shade_skipped_pixels":s.opaque_shade_skipped_pixels,"hidden_shade_buffer_bytes":s.hidden_shade_buffer_bytes,
        "indexed":s.indexed,"shared_index_bytes":s.index_bytes,
        "source_vertices_per_patch":s.source_vertices_per_patch,
        "depth_unique_vertices_per_patch":s.depth_unique_vertices_per_patch,
        "color_unique_vertices_per_patch":s.color_unique_vertices_per_patch,
        "depth_vertex_reference_budget":s.depth_vertex_reference_budget,
        "color_vertex_reference_budget":s.color_vertex_reference_budget,
        "vertex_reference_scope":"Distinct source indices times selected patches; ideal reuse budget, not measured vertex shader invocations or hardware cache hits.",
        "capacity_per_view":s.capacity_per_view,"pool_bytes":s.pool_bytes,
        "size_scale":s.size_scale,"density":s.density,"color_multiplier":s.color_multiplier,"lod_pixels":s.lod_pixels,
        "aggregate_enabled":s.aggregate_enabled,
        "aggregate_plants":s.aggregate_plants,"aggregate_triangles":s.aggregate_triangles,
        "aggregate_capacity":s.aggregate_capacity,"aggregate_pool_bytes":s.aggregate_pool_bytes,
        "aggregate_recipe_bytes":s.aggregate_recipe_bytes,"aggregate_cook_cpu_ms":s.aggregate_cook_ms,
        "aggregate_transition_metres":s.aggregate_transition_m,"aggregate_outer_fade_metres":s.aggregate_outer_fade_m,
        "aggregate_target_bytes":s.aggregate_target_bytes,
        "group_cache":meadow_group_metadata(&s.group_cache)
        });
        // Keep this expanding diagnostics object below serde_json's macro
        // recursion limit without increasing it for the entire application.
        value["aggregate_half_resolution"] = s.aggregate_half_resolution.into();
        value["aggregate_sample_prune_enabled"] = s.aggregate_sample_prune_enabled.into();
        value["spatial_filter"] = serde_json::json!({
            "requested":s.spatial_filter_requested,
            "enabled":s.spatial_filter_enabled,
            "max_strength":s.spatial_filter_max_strength,
            "blade_width_m":s.spatial_filter_blade_width_m,
            "scope":"Native windswept source colour only, filtered over depth-compatible grass neighbours as projected blades become subpixel. Original alpha and silhouette remain exact; no temporal history, geometry replacement or extra render target. Disabled with TAA/DLSS."
        });
        value["aggregate_target_size"] = serde_json::json!(s.aggregate_target_size);
        value["aggregate_resolution_scale"] = serde_json::json!(s.aggregate_resolution_scale);
        value["aggregate_resolution_scope"] = "Aggregate raster and illumination target relative to the per-view internal DLSS input; source grass, world roots, draw range and output resolution are unchanged. Reduced resolution is an opt-in optical approximation.".into();
        value
    }).collect();
    let memory = serde_json::json!({
        "shared_terrain_sample_bytes":field.samples.len()*std::mem::size_of::<[f32;4]>(),
        "shared_source_vertex_bytes":field.geometry.vertices.len()*std::mem::size_of::<render::meadow_data::GpuMeadowVertex>(),
        "shared_index_bytes":stats.first().map_or(0,|s|s.index_bytes),
        "shared_aggregate_recipe_bytes":stats.first().map_or(0,|s|s.aggregate_recipe_bytes),
        "shared_group_cache_bytes":stats.first().map_or(0,|s|s.group_cache.gpu_bytes),
        "aggregate_view_pool_bytes_total":stats.iter().map(|s|s.aggregate_pool_bytes).sum::<u64>(),
        "aggregate_view_target_bytes_total":stats.iter().map(|s|s.aggregate_target_bytes).sum::<u64>(),
        "view_pool_bytes_total":pool_bytes,
        "scope":"Terrain and source payloads are shared once; pools and aggregate targets are per allocated view. Includes the explicitly reported aggregate targets only; excludes other render targets, uniforms, indirect arguments, pipeline resources and allocator overhead. Not total VRAM."
    });
    Ok(serde_json::json!({
        "views":views,"memory":memory,"lod_order":[0,1,2],
        "substrate_enabled":renderer.meadow_substrate_enabled(),
        "edge_shadow":edge_shadow,
        "contact_ao":{
            "enabled":renderer.meadow_contact_ao_strength()>0.0,
            "strength":renderer.meadow_contact_ao_strength(),
            "max_radius_m":renderer.meadow_contact_ao_radius(),
            "scope":"Analytic retained-root basal footprints attenuate terrain diffuse skylight only; source rest coverage, conservative road setback, no camera/LOD factor. Not source-silhouette AO or a cast grass shadow."
        },
        "shadow_caster":false,"far_voxels":false,
        "distant_representation":if stats.iter().any(|s|s.aggregate_enabled) { "source_derived_directional_surface_samples" } else { "none" },
        "aggregate_limits":"Directional rest-source coverage/depth/normal/material samples with wind reprojection; not a voxel volume, multilayer transport or a grass shadow caster.",
        "main_frame_stats_include_meadow_triangles":false,
        "geometry_scope":"Actual GPU post-cull indirect lists. Source triangles count geometry once; submitted_source_triangles applies source_geometry_passes (one visibility pass or depth plus colour). Counts precede per-blade road habitat rejection, are neither rasterized pixels nor shadow triangles, and are separate from FrameStats.triangles.",
        "cpu_dependency":"Initial terrain and shared recipe upload only for meadow placement; GPU generates world-cell roots, LOD lists and branch animation. CPU supplies per-frame camera, lighting and wind parameters; no CPU per-root streaming.",
        "readback_scope":"Explicit F12 or scenario image capture only; synchronous readback is excluded from steady-frame timings."
    }))
}

fn args(args: &[String]) -> Result<Args, String> {
    let opt = |key: &str, default: &str| args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).cloned().unwrap_or(default.into());
    let parse = |key: &str, default: &str| opt(key, default).parse::<u64>().map_err(|_| format!("invalid {key}"));
    let size: Vec<u32> = opt("--size", "1280x720").split('x').map(str::parse).collect::<Result<_, _>>().map_err(|_| "size must be WIDTHxHEIGHT")?;
    if size.len() != 2 || size[0] < 64 || size[1] < 64 || size[0] > 7680 || size[1] > 4320 { return Err("size outside 64x64..7680x4320".into()); }
    let views = parse("--views", "1")? as usize;
    if !(1..=4).contains(&views) { return Err("views must be 1..=4".into()); }
    let km=opt("--km","3").parse::<f64>().map_err(|_|"invalid km")?;
    if !km.is_finite() || !(2.0..=8.0).contains(&km){return Err("km must be finite and in 2..=8".into());}
    let value = |key| args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).cloned();
    let smoke_frames=parse("--smoke-render","0")?;
    let smoke_next_stage=args.iter().any(|a|a=="--smoke-next-stage");
    if smoke_next_stage && (smoke_frames<2 || parse("--smoke-ticks","0")?>0) { return Err("--smoke-next-stage requires --smoke-render >=2 and no --smoke-ticks".into()); }
    let gpu_profile=value("--gpu-profile");
    if args.iter().any(|s|s=="--gpu-profile") && gpu_profile.as_ref().is_none_or(|s|s.starts_with("--")) { return Err("--gpu-profile requires a new JSONL output path".into()); }
    let gpu_warmup=parse("--gpu-warmup","30")? as usize;
    if gpu_profile.is_some() && smoke_frames>0 && gpu_warmup>=smoke_frames as usize {return Err("GPU profiling needs fewer warmup frames than rendered frames".into());}
    if gpu_profile.is_some() && parse("--smoke-ticks","0")?>0 {return Err("GPU profiling requires rendering, not --smoke-ticks".into());}
    let frame_profile=value("--frame-profile");
    if args.iter().any(|s|s=="--frame-profile") && frame_profile.as_ref().is_none_or(|s|s.starts_with("--")||s.trim().is_empty()) {return Err("--frame-profile requires a new JSONL output path".into());}
    if frame_profile.is_some() && parse("--smoke-ticks","0")?>0 {return Err("Frame profiling requires the event/render loop, not --smoke-ticks".into());}
    let hud_profile=value("--hud-profile");
    let hud_warmup=parse("--hud-warmup","8")? as usize;
    if hud_profile.is_some() && (smoke_frames==0 || smoke_frames>4096 || hud_warmup>=smoke_frames as usize) {
        return Err("HUD profiling needs --smoke-render 1..4096 and fewer warmup frames than rendered frames".into());
    }
    let hud=match opt("--hud","on").as_str(){"on"=>true,"off"=>false,_=>return Err("hud must be on or off".into())};
    Ok(Args { upscale:crate::upscale::parse(args)?,nature:crate::nature::Nature::parse(args)?,gpu_profile,gpu_warmup,frame_profile,seed: parse("--seed", "1")?, km, views, size: (size[0], size[1]), weather: opt("--weather", "clear"), smoke_next_stage, smoke_frames, smoke_ticks: parse("--smoke-ticks", "0")?, capture: value("--capture"), state_out: value("--state-out"), trace: value("--state-trace"), fullscreen: args.iter().any(|a| a == "--fullscreen"), hud,hud_profile,hud_warmup,autoplay:args.iter().any(|a|a=="--autoplay"),
        debug_ui:args.iter().any(|a|a=="--debug-ui"),debug_ui_script:value("--debug-ui-script"),
        start_paused:args.iter().any(|a|a=="--start-paused"),
        // Hidden smoke runs stay silent unless asked.
        sound: crate::audio_out::Mode::parse(&opt("--sound", if smoke_frames > 0 { "off" } else { "pulse" }))? })
}

fn race(a: &Args) -> Race {
    let settings = Settings { vegetation:a.nature.vegetation, stage_km: a.km, bots: a.views, rivals: 0, weather: Some(a.weather.clone()), ..Default::default() };
    Race::new(a.seed, settings, sideways::spec::Cars::load(None).expect("bundled car specifications"))
}

fn poses(race: &Race) -> Vec<CarPose> {
    let colors = [Vec3::new(0.35, 0.035, 0.025), Vec3::new(0.025, 0.15, 0.45), Vec3::new(0.45, 0.28, 0.025), Vec3::new(0.03, 0.3, 0.08)];
    race.players.iter().enumerate().map(|(i, p)| {
        let pose = p.car.pose(&race.phys);
        let q = pose.rotation;
        let t = pose.translation;
        CarPose { pos: Vec3::new(t.x, t.y, t.z), rot: Quat::from_xyzw(q.x, q.y, q.z, q.w), color: colors[i % 4], boxes: p.car.spec.boxes.clone(), model: p.car.spec.model.clone(), ride: p.car.spec.ride as f32,
            wheels: p.car.wheels.iter().map(|w| crate::car_model::Wheel { anchor: Vec3::new(w.anchor.x, w.anchor.y, w.anchor.z), compression: w.compression as f32, steer: w.steer as f32, spin: w.spin as f32 }).collect() }
    }).collect()
}

fn cameras(race: &Race, poses: &[CarPose], yaw: &[f32; 4], far: &[bool; 4], rest: &mut [crate::chase_camera::RestAnchor; 4]) -> Vec<Camera> {
    poses.iter().enumerate().take(4).map(|(i, car)| {
        let p = &race.players[i].car;
        let velocity = p.linvel(&race.phys); let angular = p.angvel(&race.phys);
        let (position, heading) = rest[i].sample(car.pos, car.rot,
            Vec3::new(velocity.x, velocity.y, velocity.z), Vec3::new(angular.x, angular.y, angular.z));
        let back = Quat::from_rotation_y(yaw[i]) * -heading;
        let distance = if far[i] { 8.0 } else { 5.2 };
        let mut pos = position + back * distance + Vec3::Y * if far[i] { 3.0 } else { 2.05 };
        pos.y = pos.y.max(race.stage.height(pos.x as f64, pos.z as f64) as f32 + 0.7);
        Camera { pos, target: position + Vec3::Y * 0.9 + heading * 1.5, fov_y: 64f32.to_radians(), near: 0.1 }
    }).collect()
}

fn lighting(weather: &str) -> Lighting {
    let mut light = Lighting::default();
    // Current TAA has no per-object motion vectors. Live driving must not
    // leave old car silhouettes behind, so the first playable uses no history.
    light.taa = 0.0;
    match weather {
        "overcast" => { light.sun_lux = 25_000.0; light.sky_lux = 14_000.0; light.ev100 = 12.5; light.shadow_strength = 0.6; }
        "mist" => { light.sun_lux = 30_000.0; light.sky_lux = 15_000.0; light.ev100 = 12.7; light.fog_density = 0.011; }
        "wet" => { light.sun_lux = 20_000.0; light.sky_lux = 12_000.0; light.ev100 = 12.2; light.shadow_strength = 0.5; }
        _ => {}
    }
    light
}

fn lighting_metadata(light:&Lighting)->serde_json::Value {
    serde_json::json!({"sun_lux":light.sun_lux,"shadow_strength":light.shadow_strength,"near_shadows":light.near_shadows,
        "gi":light.gi,"contact_occlusion":light.contact_occlusion,"reflection_occlusion":light.reflection_occlusion,"ev100":light.ev100})
}

pub(crate) fn rear_light_state(gear:i32,manual:bool,hold:bool,brake:f64,throttle:f64)->crate::car_headlights::RearLightState {
    // Same effective pedal as Car::drive and the original emissive rear lenses:
    // automatic reverse swaps the driving/braking pedals; hold is a full brake.
    let foot=if hold {1.0}else if gear>0||manual {brake}else{throttle};
    crate::car_headlights::RearLightState {brake:if foot.is_finite(){foot.clamp(0.0,1.0) as f32}else{0.0},reverse:gear<0}
}

pub(crate) fn local_lights_metadata(lights:&[render::headlights::LocalLight],vehicle_count:usize)->serde_json::Value {
    serde_json::json!({"mode":"bounded unshadowed direct vehicle and exhaust spill",
        "enabled":!lights.is_empty(),"vehicle_count":vehicle_count,"exhaust_count":lights.len().saturating_sub(vehicle_count),
        "rear_lamp_policy":crate::car_headlights::local_metadata(),
        "lights":lights.iter().enumerate().map(|(i,l)|serde_json::json!({
            "source":if i<vehicle_count {"rear_lamp"}else{"exhaust_flash"},
            "position":l.position.to_array(),"direction":l.direction.to_array(),"intensity_cd":l.intensity_cd.to_array(),
            "range_m":l.range_m,"inner_cos":l.inner_cos,"outer_cos":l.outer_cos,"source_radius_m":l.source_radius_m
        })).collect::<Vec<_>>(),
        "limits":["direct spill only; no local shadow, bounce or atmospheric scattering","nearby occluders can leak these short-range lights"]})
}

struct LiveGpu {
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    renderer: Renderer,
    presenter: render::present::Presenter,
    hud: Option<render::hud::HudRenderer>,
    hud_profile: Option<render::hud_profile::HudProfiler>,
}

/// Only the light presenter is owned by the window thread during startup.
/// Scene GPU construction runs after the first loading image is displayed.
struct StartupGpu {
    instance:wgpu::Instance,surface:wgpu::Surface<'static>,config:wgpu::SurfaceConfiguration,gpu:Gpu,
    presenter:render::present::Presenter,hud:render::hud::HudRenderer,
    hud_profile:Option<render::hud_profile::HudProfiler>,background:wgpu::Texture,
    build:stage_loading::Startup,
}

struct App {
    debug:crate::engine_debug::EngineDebug,
    upscale_error: Option<String>,
    gpu_profile: Option<std::io::BufWriter<std::fs::File>>,
    frame_profile:Option<frame_profile::Profiler>,
    scenario:Option<scenario::Runtime>,
    startup_timing:cli_startup::Timeline,requested_output_size:[u32;2],
    args: Args, race: Race, built: Built, light: Lighting, input: Input,
    sky_metadata: serde_json::Value,
    sky_controls: crate::procedural_sky::LiveSky,
    pause_menu: crate::procedural_sky::PauseMenu,
    window: Option<Arc<Window>>, graphics: Option<LiveGpu>, focused: bool, paused: bool,
    startup:Option<StartupGpu>,last_startup:serde_json::Value,
    last_frame: Instant, started: Instant, last_title: Instant, last_trace: Instant, accumulator: f64,
    ambient_clock: AmbientClock,
    rendered: u64, presented: u64, initial_positions: Vec<[f32; 3]>, controls: Vec<Control>,
    yaw: [f32; 4], far: [bool; 4], error: Option<String>, exit_reason: String,
    orbit_active: [bool; 4], camera_rest: [crate::chase_camera::RestAnchor; 4],
    host: Option<Client>, seats: Vec<Seat>, view_order: Vec<usize>, pending_settings: serde_json::Value,
    active_settings: serde_json::Value, input_edges: [(i8,bool);4], look: [[f32;2];4], finished_round: u32,
    resume_fullscreen: Option<Fullscreen>,
    /// Under a host: minimise on pause and take the screen back on start,
    /// asking until it takes (app::manners, as the Godot SDK's screen.gd).
    manners: app::manners::Manners,
    hud_stats: serde_json::Value, hud_samples: Vec<serde_json::Value>,
    screenshot_pending: bool, screenshot_toast: Option<(String, Instant)>,
    sound: Option<crate::audio_out::Sound>, sound_failed: bool,
    exhaust: crate::exhaust_fx::ExhaustFx,
    wheel_effects: crate::wheel_effects::WheelEffects,
    /// Car material data and shading shaders, reloaded when edited.
    live_edit: crate::car_look::LiveEdit,
    loading: Option<stage_loading::Loading>,
    tree_baseline:Option<crate::tree_tuning::Baseline>,
    tree_loading:Option<stage_loading::Startup>,
    tree_loading_settings:crate::tree_tuning::Settings,
    last_tree_load:serde_json::Value,
    last_stage_load: serde_json::Value,
    loading_error: Option<String>,
    cached_picture: Option<wgpu::Texture>,
    /// Two reused frame outputs, alternated so `cached_picture` (the last
    /// frame) is never the texture the next frame renders into.
    outputs: [Option<wgpu::Texture>; 2],
    deferred_host: Vec<Command>,
    smoke_stage_requested: bool,
    /// Interactive play only: render scale and grass reach follow the frame rate.
    quality: Option<crate::adaptive_quality::AdaptiveQuality>,
}

impl App {
    /// Engine sound: (re)start when the field of cars changes, then place
    /// every car for the players' cameras. A missing device turns it off.
    fn update_sound(&mut self, cams: &[Camera], dt: f32, invisible: bool) {
        if self.args.sound == crate::audio_out::Mode::Off || self.sound_failed { return; }
        if self.sound.as_ref().map(|s| s.cars()) != Some(self.race.players.len()) {
            self.sound = None;
            match crate::audio_out::Sound::start(&self.race, self.args.views, self.args.sound) {
                Ok(s) => self.sound = Some(s),
                Err(e) => { eprintln!("audio off: {e}"); self.sound_failed = true; return; }
            }
        }
        if let Some(s) = &mut self.sound {
            s.pause(self.paused || invisible);
            s.update(&self.race, &cams[..self.args.views.min(cams.len())], dt);
        }
    }

    fn invisible(&self) -> bool {
        self.args.smoke_frames > 0 || self.window.as_ref().is_some_and(|w|w.is_visible()==Some(false)||w.is_minimized()==Some(true))
    }

    fn smoke_expired(&self) -> bool {
        self.args.smoke_frames > 0 && self.started.elapsed() > Duration::from_secs(if self.scenario.is_some(){900}else{90})
    }

    fn managed_screen(&mut self, visible: bool) {
        let Some(window)=&self.window else{return;};
        // Hidden acceptance tests must never take over the desktop.
        if self.args.smoke_frames>0 {
            if !visible {window.set_visible(false);}
            return;
        }
        // Minimised, not invisible, so Alt+Tab can still reach a paused or
        // waiting game (the host then resumes or starts it).
        let wish=if !visible {app::Screen::Hidden} else if self.resume_fullscreen.is_some() {app::Screen::Fullscreen} else {app::Screen::Normal};
        if visible {window.set_visible(true);}
        else if window.is_visible()==Some(true) && window.is_minimized()!=Some(true) {self.resume_fullscreen=window.fullscreen();}
        self.manners.wish(wish,self.focused);
    }

    fn snapshot(&self) -> serde_json::Value {
        let grass=self.built.ground_cover_stats();
        let players:Vec<_>=self.race.players.iter().enumerate().map(|(i,p)| {
            let t=p.car.pose(&self.race.phys).translation;
            let c=self.controls.get(i).copied().unwrap_or_default();
            let input=serde_json::json!({"throttle":c.throttle,"brake":c.brake,"steer":c.steer,"handbrake":c.handbrake,"shift":c.shift,"recover":c.recover});
            serde_json::json!({"seat":self.seats.get(i).map(|s|s.index).unwrap_or(i),"player_id":self.seats.get(i).map(|s|&s.player_id),"name":p.name,"position":[t.x,t.y,t.z],"gear":p.car.gear,"manual":p.car.manual,"penalty":p.penalty,"finished":p.finished,"look":self.look.get(i),"camera_yaw":self.yaw.get(i),"camera_mode":self.far.get(i),"input":input})
        }).collect();
        let mut snapshot = serde_json::json!({
            "schema": 1, "mode": "play", "hidden": self.args.smoke_frames > 0, "race_frame": self.race.frame,
            "nature":self.ambient_clock.metadata(self.args.nature),
            "upscale":self.graphics.as_ref().map(|g|crate::upscale::metadata(self.args.upscale,&g.renderer.upscale_status())),
            "upscale_requested":self.args.upscale.as_str(),
            "upscale_error":self.upscale_error,
            "gpu_profile":self.args.gpu_profile,"gpu_profile_warmup_frames":self.args.gpu_warmup,
            "autoplay": self.args.autoplay && self.host.is_none(),
            "race_clock": self.race.clock, "race_round": self.race.round, "race_phase": self.race.phase,
            "paused": self.paused, "focused": self.focused, "rendered_frames": self.rendered, "presented_frames": self.presented,
            "window_visible":self.window.as_ref().and_then(|w|w.is_visible()).unwrap_or(false),
            "render_quiet":self.host.as_ref().is_some_and(|h|!h.state.is_running()),
            "physics_hz": 120, "car_substeps": sideways::car::SUBSTEPS, "taa": false, "ground_cover": grass.is_some(),
            "grass_detail_profile":self.built.scene.grass_detail.label(),
            "ground_cover_streaming":grass_stream_metadata(&self.built),
            "far_ground_cover":self.built.scene.far_grass.as_ref().map(|f|serde_json::json!({"origin":f.origin,"cell":f.cell,"point_grid":[f.width,f.height],"max_height":f.max_height,"profile":f.profile,"half_resolution":true,"outer_fade_m":[160,240]})),
            "experimental_voxel_tree_patch":self.built.scene.voxel_forest.as_ref().map(|f|serde_json::json!({"tree_count":f.instances.len(),"levels":f.levels.len(),"packed_bytes":f.byte_len(),"automatic_lod_transition":false})),
            "initial_positions": self.initial_positions, "exit_reason": self.exit_reason, "error": self.error,
            "host": self.host.as_ref().map(|h|h.state.snapshot()), "view_order": self.view_order.iter().map(|i|self.seats[*i].index).collect::<Vec<_>>(),
            "native_controllers":self.input.controllers(),"native_active_samples":self.input.native_active_samples,
            "active_settings": self.active_settings, "pending_settings": self.pending_settings,
            "players":players,
            "view_rects": render::split_rects(self.args.views, self.args.size.0, self.args.size.1),
            "hud": self.args.hud, "hud_stats": self.hud_stats,
        });
        snapshot["engine_debug"]=self.debug.metadata();
        snapshot["stage_loading"]=self.loading.as_ref().map(|l|l.metadata()).unwrap_or(serde_json::Value::Null);
        snapshot["tree_loading"]=self.tree_loading.as_ref().map(|l|l.metadata()).unwrap_or(serde_json::Value::Null);
        snapshot["last_tree_load"]=self.last_tree_load.clone();
        snapshot["startup_loading"]=self.startup.as_ref().map(|s|s.build.metadata()).unwrap_or(serde_json::Value::Null);
        snapshot["startup_report"]=self.last_startup.clone();
        snapshot["scenario"]=self.scenario.as_ref().map(|s|s.config.metadata()).unwrap_or(serde_json::Value::Null);
        snapshot["last_stage_load"]=self.last_stage_load.clone();
        snapshot["stage_loading_error"]=self.loading_error.clone().into();
        snapshot["crowds"]=crate::roadside_crowds::metadata(&self.built.scene);
        snapshot["exhaust"]=self.exhaust.metadata();
        snapshot["wheel_effects"]=self.wheel_effects.metadata();
        snapshot["procedural_grass"]=self.built.procedural_grass.clone().unwrap_or(serde_json::Value::Null);
        snapshot["tree_source"]=self.built.tree_source.clone();
        snapshot["sky"] = self.sky_metadata.clone();
        snapshot["live_sky_controls"] = self.sky_controls.metadata();
        snapshot
    }

    fn trace(&mut self, force: bool) {
        if !force && self.last_trace.elapsed() < Duration::from_millis(100) { return; }
        self.last_trace = Instant::now();
        if let Some(path) = &self.args.trace {
            match OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| writeln!(f,"{}", self.snapshot())) {
                Ok(()) => {}, Err(e) => eprintln!("state trace {path}: {e}"),
            }
        }
    }

    fn finish(&mut self, event_loop: &ActiveEventLoop, reason: &str) {
        self.exit_reason = reason.into();
        if self.loading.is_none() && self.tree_loading.is_none() && self.graphics.is_some() {if let Err(error)=self.write_hud_profile(){self.error=Some(error);}}
        if let Some(loading)=&self.loading {loading.cancel();}
        if let Some(startup)=&self.startup {startup.build.cancel();}
        if let Some(loading)=&self.tree_loading {loading.cancel();}
        if let Some(sound)=&self.sound {sound.pause(true);}
        if let Some(file)=&mut self.gpu_profile {if let Err(e)=file.flush(){self.error=Some(format!("GPU profile flush: {e}"));}}
        self.trace(true);
        if let Some(path) = &self.args.state_out {
            if let Err(e) = std::fs::write(path, serde_json::to_vec_pretty(&self.snapshot()).unwrap()) { self.error = Some(format!("state output: {e}")); }
        }
        println!("play stopped: {reason}; {} rendered, {} presented, tick {}", self.rendered, self.presented, self.race.frame);
        event_loop.exit();
    }

    fn write_hud_profile(&self)->Result<(),String>{
        let Some(path)=&self.args.hud_profile else{return Ok(());};
        let g=self.graphics.as_ref().ok_or("HUD profile: GPU not initialized")?;
        let gpu_times=g.hud_profile.as_ref().map(|p|p.read(&g.gpu)).transpose()?;
        if let Some(times)=&gpu_times{if times.len()!=self.hud_samples.len(){return Err("HUD profile sample count mismatch".into());}}
        let mut samples=self.hud_samples.clone();
        for (i,sample) in samples.iter_mut().enumerate(){sample["hud_gpu_ms"]=gpu_times.as_ref().and_then(|t|t.get(i)).map(|v|serde_json::json!(v)).unwrap_or(serde_json::Value::Null);}
        let measured=&samples[self.args.hud_warmup.min(samples.len())..];
        let summary=|key:&str|{
            let mut values:Vec<f64>=measured.iter().filter_map(|s|s[key].as_f64()).collect();
            if values.is_empty(){return serde_json::Value::Null;}
            values.sort_by(f64::total_cmp);let n=values.len();
            let median=if n%2==0{(values[n/2-1]+values[n/2])*0.5}else{values[n/2]};
            serde_json::json!({"median_ms":median,"p95_ms":values[((n as f64*0.95).ceil() as usize).saturating_sub(1).min(n-1)]})
        };
        let report=serde_json::json!({"schema":1,"mode":"hidden_dynamic_hud_profile","hud_enabled":self.args.hud,"size":self.args.size,"views":self.args.views,
            "adapter":g.gpu.info.name,"surface_format":format!("{:?}",g.config.format),"seed":self.args.seed,"weather":self.race.weather,"warmup_frames":self.args.hud_warmup,
            "measured_frames":measured.len(),"rendered_frames":self.rendered,"presented_frames":self.presented,"race_frame":self.race.frame,
            "initial_positions":self.initial_positions,"final_positions":poses(&self.race).iter().map(|p|p.pos.to_array()).collect::<Vec<_>>(),
            "hud_gpu":summary("hud_gpu_ms"),"hud_layout_cpu":summary("hud_layout_cpu_ms"),"hud_prepare_cpu":summary("hud_prepare_cpu_ms"),"graphics_wall":summary("graphics_wall_ms"),
            "limits":["GPU HUD pass excludes 3D rendering; all split HUDs share one pass.","CPU prepare includes text layout/cache and GPU uploads.","Hidden graphics wall includes GPU wait; not gameplay FPS or visible presentation.","No timestamp readback occurs inside normal play; profiling resolves each sample and reads once at completion."],"samples":samples});
        std::fs::write(path,serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|format!("HUD profile {path}: {e}"))
    }

    fn rebuild(&mut self) -> Result<(), String> {
        let poses = poses(&self.race);
        self.camera_rest = Default::default();
        let cams = cameras(&self.race, &poses, &self.yaw, &self.far, &mut self.camera_rest);
        let ordered:Vec<_>=self.view_order.iter().filter_map(|i|cams.get(*i)).take(self.args.views).collect();
        let positions:Vec<_>=if ordered.is_empty(){cams.iter().take(self.args.views).map(|c|c.pos).collect()}else{ordered.iter().map(|c|c.pos).collect()};
        self.built = visuals::build_live(&self.race.stage, &poses, &positions, self.live_edit.looks())?;
        let tree_settings=self.debug.applied_tree_settings();
        let baseline=crate::tree_tuning::Baseline::prepare_optional_stage(&mut self.built.scene,&mut self.race.phys,tree_settings)?;
        let tree_report=crate::tree_tuning::Baseline::stage_metadata(baseline.as_ref(),tree_settings);self.tree_baseline=baseline;
        for line in &self.built.log { eprintln!("scene rebuild: {line}"); }
        self.light = lighting(self.race.weather);
        self.sky_metadata = match self.sky_controls.restore(&mut self.built.scene,&mut self.light) {
            Some(metadata)=>metadata,
            None=>crate::procedural_sky::apply_environment(&mut self.built.scene, &mut self.light)?,
        };
        if let Some(g) = &mut self.graphics {
            let renderer = Renderer::new(&g.gpu, &self.built.scene, &self.light);
            renderer.set_upscale(&g.gpu,self.args.upscale)?;
            g.renderer = renderer;
            self.debug.reset_measurements(self.rendered);self.debug.gpu_unavailable=false;
        }
        self.debug.tree_committed(tree_settings,tree_report);
        self.last_frame = Instant::now(); self.accumulator = 0.0;
        Ok(())
    }

    fn profiles(&mut self, current: &[Seat]) {
        let old_order=self.view_order.clone();
        // Cars remain associated with their player IDs. Changing couch order,
        // names or controller tokens never teleports/reorders the simulation.
        for (i, seat) in self.seats.iter_mut().enumerate() {
            if let Some(updated)=current.iter().find(|s|if seat.player_id.is_empty(){s.player_id.is_empty()&&s.index==seat.index}else{s.player_id==seat.player_id}) {
                seat.name=updated.name.clone();seat.controller=updated.controller.clone();seat.couch=updated.couch;seat.index=updated.index;
                if let Some(p)=self.race.players.get_mut(i){p.name=updated.name.clone();}
            } else { seat.controller.clear(); }
        }
        self.view_order=(0..self.seats.len()).collect();
        self.view_order.sort_by_key(|i|(self.seats[*i].couch.unwrap_or(1000),self.seats[*i].index));
        if self.view_order!=old_order {if let Some(g)=&self.graphics {g.renderer.reset_history();}}
    }

    fn apply_settings(&mut self) {
        self.active_settings=self.pending_settings.clone();
        if let Some(km)=self.active_settings["stage_km"].as_f64(){self.race.settings.stage_km=km.clamp(2.0,8.0);}
        if let Some(v)=self.active_settings["contact"].as_bool(){self.race.settings.contact=v;}
        if let Some(v)=self.active_settings["rivals"].as_u64(){self.race.settings.rivals=(v as usize).min(3);}
        if let Some(v)=self.active_settings["weather"].as_str(){self.race.settings.weather=if v=="random"{None}else{Some(v.into())};}
    }

    fn request_sky_setting(&mut self,key:&str,value:&serde_json::Value,now:Instant)->Result<(),String> {
        let settings=self.sky_controls.desired().changed(key,value)?;
        self.sky_controls.request(settings,now)?;
        self.pending_settings["sky_time"]=settings.time_key().into();
        self.pending_settings["sky_weather"]=settings.atmosphere.key().into();
        Ok(())
    }

    fn debug_sky_requests(&mut self,now:Instant) {
        while let Some((key,value))=self.debug.take_sky_request() {
            if let Err(error)=self.request_sky_setting(key,&value,now) {self.sky_controls.failed(error);}
        }
        self.debug.sync_sky(self.sky_controls.desired(),self.sky_controls.status());
    }

    fn poll_sky_update(&mut self,now:Instant) {
        let Some(prepared)=self.sky_controls.poll(now) else{return;};
        let mut light=self.light.clone();prepared.settings.apply_presentation(&mut light);
        let result=if let Some(g)=&mut self.graphics {g.renderer.update_sky(&g.gpu,&prepared.sky,&mut light)}else{Ok(())};
        match result {
            Ok(())=>{
                prepared.apply_scene(&mut self.built.scene,&mut light);
                self.light=light;self.sky_metadata=prepared.metadata.clone();
                self.active_settings["sky_time"]=prepared.settings.time_key().into();
                self.active_settings["sky_weather"]=prepared.settings.atmosphere.key().into();
                eprintln!("live sky applied: revision {}, {} / {}",prepared.revision,prepared.settings.time_key(),prepared.settings.atmosphere.key());
                self.sky_controls.accepted(prepared);self.debug.reset_measurements(self.rendered);self.trace(true);
            }
            Err(error)=>{eprintln!("live sky unchanged: {error}");self.sky_controls.failed(error);}
        }
    }

    fn standalone_menu(&mut self,now:Instant)->Result<(),String> {
        if !self.paused || self.host.is_some(){return Ok(());}
        let (axis,confirm)=self.input.menu_state(self.focused);
        if self.debug.ui.open {self.pause_menu.suspend(axis,confirm);return Ok(());}
        use crate::procedural_sky::MenuAction;
        match self.pause_menu.navigate(axis,confirm,now) {
            Some(MenuAction::Resume)=>self.paused=false,
            Some(MenuAction::NewStage)=>{self.next_stage()?;self.paused=false;},
            Some(MenuAction::Time(delta))=>{
                let time=self.sky_controls.desired().time.step(delta);
                if let Err(error)=self.request_sky_setting("sky_time",&time.key().into(),now) {self.sky_controls.failed(error);}
            }
            Some(MenuAction::Atmosphere(delta))=>{
                let weather=self.sky_controls.desired().atmosphere.step(delta);
                if let Err(error)=self.request_sky_setting("sky_weather",&weather.key().into(),now) {self.sky_controls.failed(error);}
            }
            None=>{}
        }
        Ok(())
    }

    fn sync_debug_focus(&mut self,was_open:bool) {
        if was_open!=self.debug.ui.open {
            self.input.clear_keys();
            let (axis,confirm)=self.input.menu_state(self.focused);
            self.pause_menu.suspend(axis,confirm);
        }
    }

    fn reset_round_input(&mut self) {
        // A new round creates new cameras, as in Godot. Seat order can change
        // here; no new player may inherit another player's camera/input state.
        self.yaw=[0.0;4];self.far=[false;4];self.look=[[0.0;2];4];
        self.orbit_active=[false;4];self.camera_rest=Default::default();
        self.input_edges=[(0,false);4];self.controls.clear();self.accumulator=0.0;
    }

    fn queue_tree_edit(&mut self,settings:crate::tree_tuning::Settings)->Result<(),String> {
        settings.validate()?;
        if self.tree_baseline.is_none() {
            self.tree_baseline=Some(crate::tree_tuning::Baseline::capture(&self.built.scene,&self.race.phys)?);
        }
        let baseline=self.tree_baseline.as_ref().unwrap();
        baseline.apply_scene(&mut self.built.scene,settings)?;
        let scene=std::mem::take(&mut self.built.scene);
        self.tree_loading=Some(stage_loading::Startup::new(scene,self.light.clone(),self.args.upscale));
        self.tree_loading_settings=settings;
        self.debug.cancel_interaction();self.accumulator=0.0;self.input_edges=[(0,false);4];
        self.last_tree_load=serde_json::json!({"outcome":"queued","round":self.race.round,"race_frame":self.race.frame,"clock":self.race.clock});
        Ok(())
    }

    /// Reuse the graphics worker and presentation gate without regenerating
    /// the route or moving the race. No renderer work runs beside its rebuild.
    fn tree_loading_frame(&mut self)->Result<(),String> {
        self.last_frame=Instant::now();self.accumulator=0.0;self.input.poll();
        for seat in 0..4 {let _=self.input.seat(seat,false);}
        self.input.clear_keys();self.input_edges=[(0,false);4];
        if let Some(host)=&mut self.host{host.state.suppress_buttons();}
        if let Some(sound)=&self.sound{sound.pause(true);}
        if let Some(result)=self.tree_loading.as_mut().and_then(|l|l.poll()) {
            let mut job=self.tree_loading.take().unwrap();let mut report=job.metadata();
            let mut ready=match result {Ok(ready)=>ready,Err(error)=>{
                let mut scene=job.take_failed_scene().ok_or_else(||format!("tree rebuild failed without recoverable scene: {error}"))?;
                let applied=self.debug.applied_tree_settings();
                self.tree_baseline.as_ref().ok_or("tree baseline lost")?.apply_scene(&mut scene,applied)?;
                self.built.scene=scene;self.debug.tree_failed(&error);self.paused=true;
                report["outcome"]="failed".into();report["error"]=error.into();self.last_tree_load=report;self.trace(true);return Ok(());
            }};
            self.tree_baseline.as_ref().ok_or("tree baseline lost")?.apply_physics(&mut self.race.phys,self.tree_loading_settings)?;
            self.built.scene=ready.scene;
            let g=self.graphics.as_mut().ok_or("GPU missing during tree apply")?;
            ready.renderer.take_runtime_effects_from(&g.gpu,&mut g.renderer)?;
            g.renderer=ready.renderer;
            let metadata=self.tree_baseline.as_ref().unwrap().metadata(self.tree_loading_settings);
            self.debug.tree_committed(self.tree_loading_settings,metadata);
            self.debug.reset_measurements(self.rendered);self.debug.gpu_unavailable=false;
            report["outcome"]="ready".into();report["round"]=self.race.round.into();report["race_frame"]=self.race.frame.into();report["clock"]=self.race.clock.into();
            report["car_positions"]=serde_json::json!(poses(&self.race).iter().map(|p|p.pos.to_array()).collect::<Vec<_>>());
            self.last_tree_load=report;self.last_frame=Instant::now();self.trace(true);return Ok(());
        }
        if self.host.as_ref().is_some_and(|h|!h.state.is_running()){self.trace(false);return Ok(());}
        let size=self.window.as_ref().ok_or("tree edit window missing")?.inner_size();
        if size.width==0 || size.height==0{return Ok(());}
        let invisible=self.invisible();
        let list=self.tree_loading.as_ref().unwrap().hud_titled([size.width,size.height],"APPLYING TREE CHANGES");
        let g=self.graphics.as_mut().ok_or("tree edit GPU missing")?;
        if g.hud.is_none(){g.hud=Some(render::hud::HudRenderer::new(&g.gpu,g.config.format)?);}
        g.hud.as_mut().unwrap().prepare(&g.gpu,&list)?;
        // A normal rendered frame precedes UI interaction. Defensive fallback
        // also supports scripted Apply on the first scenario frame.
        if self.cached_picture.is_none() {
            let texture=g.gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("vegetation loading background"),size:wgpu::Extent3d{width:1,height:1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
            g.gpu.queue.write_texture(wgpu::TexelCopyTextureInfo{texture:&texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},&[8,12,18,255],wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(4),rows_per_image:Some(1)},texture.size());self.cached_picture=Some(texture);
        }
        let picture=self.cached_picture.as_ref().unwrap();let mut displayed=false;
        if invisible {
            let target=g.gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("hidden vegetation presentation"),size:wgpu::Extent3d{width:size.width,height:size.height,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:g.config.format,usage:wgpu::TextureUsages::RENDER_ATTACHMENT,view_formats:&[]});
            g.presenter.present(&g.gpu,picture,&target.create_view(&Default::default()),g.hud.as_ref(),None)?;displayed=true;
        } else {match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)|wgpu::CurrentSurfaceTexture::Suboptimal(frame)=>{
                g.presenter.present(&g.gpu,picture,&frame.texture.create_view(&Default::default()),g.hud.as_ref(),None)?;
                self.window.as_ref().unwrap().pre_present_notify();g.gpu.queue.present(frame);self.presented+=1;displayed=true;
            }
            wgpu::CurrentSurfaceTexture::Outdated=>g.surface.configure(&g.gpu.device,&g.config),
            wgpu::CurrentSurfaceTexture::Lost=>{g.surface=g.instance.create_surface(self.window.as_ref().unwrap().clone()).map_err(|e|e.to_string())?;g.surface.configure(&g.gpu.device,&g.config);}
            wgpu::CurrentSurfaceTexture::Timeout|wgpu::CurrentSurfaceTexture::Occluded=>{},
            wgpu::CurrentSurfaceTexture::Validation=>return Err("tree edit surface validation failure".into()),
        }}
        self.tree_loading.as_mut().unwrap().displayed(&g.gpu,displayed);self.trace(false);Ok(())
    }

    fn next_stage(&mut self)->Result<(),String>{
        if self.loading.is_some(){return Ok(());}
        let mut settings=self.race.settings.clone();
        let values=self.pending_settings.clone();
        if let Some(km)=values["stage_km"].as_f64(){settings.stage_km=km.clamp(2.0,8.0);}
        if let Some(v)=values["contact"].as_bool(){settings.contact=v;}
        if let Some(v)=values["rivals"].as_u64(){settings.rivals=(v as usize).min(3);}
        if let Some(v)=values["weather"].as_str(){settings.weather=if v=="random"{None}else{Some(v.into())};}
        let seats=self.host.as_ref().map(|h|h.state.seats()).filter(|s|!s.is_empty()).unwrap_or_else(||self.seats.clone());
        let views=seats.len().clamp(1,4);settings.bots=views;
        let request=stage_loading::Request {seed:stage_loading::Seed::next(&self.race,settings),seats,views,settings:values,
            looks:self.live_edit.looks().clone(),sky:self.sky_controls.applied_snapshot(),upscale:self.args.upscale,
            tree_settings:self.debug.applied_tree_settings()};
        let loading=stage_loading::Loading::new(request);
        {
            let mut p=loading.progress.lock().unwrap_or_else(|e|e.into_inner());
            p.report["queued_at_app_ms"]=(self.started.elapsed().as_secs_f64()*1000.0).into();
            p.report["from_round"]=self.race.round.into();p.report["from_frame"]=self.race.frame.into();
            p.report["from_noise_seed"]=self.race.stage.noise_seed.into();p.report["from_detail_seed"]=self.race.stage.detail_seed.into();
        }
        self.loading=Some(loading);self.loading_error=None;self.accumulator=0.0;self.input_edges=[(0,false);4];
        self.controls.clear();self.look=[[0.0;2];4];self.screenshot_pending=false;
        if let Some(sound)=&self.sound{sound.pause(true);}
        eprintln!("next stage: queued after round {}; waiting for loading display before construction",self.race.round);
        self.trace(true);Ok(())
    }

    fn startup_frame(&mut self)->Result<(),String> {
        self.last_frame=Instant::now();self.accumulator=0.0;self.input.poll();
        for seat in 0..4 {let _=self.input.seat(seat,false);}
        self.input.clear_keys();self.input_edges=[(0,false);4];
        if let Some(result)=self.startup.as_mut().and_then(|s|s.build.poll()) {
            let startup=self.startup.take().unwrap();let mut report=startup.build.metadata();
            let ready=match result {Ok(ready)=>ready,Err(error)=>{
                report["outcome"]="failed".into();report["error"]=error.clone().into();self.last_startup=report;return Err(error);
            }};
            self.built.scene=ready.scene;
            if let Some(file)=&mut self.gpu_profile {
                let header=serde_json::json!({"kind":"header","schema":1,"adapter":startup.gpu.info.name,"backend":format!("{:?}",startup.gpu.info.backend),
                    "seed":self.args.seed,"km":self.args.km,"views":self.args.views,"output_size":[startup.config.width,startup.config.height],"warmup_frames":self.args.gpu_warmup,
                    "upscale":crate::upscale::metadata(self.args.upscale,&ready.renderer.upscale_status()),
                    "nature":self.ambient_clock.metadata(self.args.nature),"autoplay":self.args.autoplay&&self.host.is_none(),"sky":self.sky_metadata,
                    "grass_detail_profile":self.built.scene.grass_detail.label(),"ground_cover_streaming":grass_stream_metadata(&self.built),
                    "taa":self.light.taa,"lod_pixels":self.light.lod_pixels,"far_grass":self.built.scene.far_grass.is_some(),"foliage_constructor_flags":foliage_variant_flags(),
                    "voxel_forest":self.built.scene.voxel_forest.is_some(),"initial_instances":self.built.scene.instances.len(),
                    "scope":"Synchronous same-frame GPU pass timestamps; excludes HUD, presentation, simulation and uninstrumented work. Profiling changes frame pacing; not gameplay FPS."});
                writeln!(file,"{header}").and_then(|_|file.flush()).map_err(|e|format!("GPU profile header: {e}"))?;
            }
            report["outcome"]="ready".into();report["race_frame_at_install"]=self.race.frame.into();
            report["elapsed_ms"]=(startup.build.started.elapsed().as_secs_f64()*1000.0).into();
            eprintln!("startup graphics ready: {report}");self.last_startup=report;
            self.startup_timing.mark("graphics_worker_pending");
            self.startup_timing.emit("graphics_ready",self.last_startup.clone());
            self.graphics=Some(LiveGpu {instance:startup.instance,surface:startup.surface,config:startup.config,
                gpu:startup.gpu,renderer:ready.renderer,presenter:startup.presenter,hud:Some(startup.hud),hud_profile:startup.hud_profile});
            self.input.clear_keys();self.input_edges=[(0,false);4];self.last_frame=Instant::now();self.trace(true);return Ok(());
        }
        let window=self.window.as_ref().ok_or("startup window missing")?.clone();let size=window.inner_size();
        if size.width==0 || size.height==0{return Ok(());}
        let invisible=self.invisible();let startup=self.startup.as_mut().ok_or("startup presentation missing")?;
        if startup.config.width!=size.width || startup.config.height!=size.height {
            startup.config.width=size.width;startup.config.height=size.height;startup.surface.configure(&startup.gpu.device,&startup.config);
        }
        let list=startup.build.hud([size.width,size.height]);startup.hud.prepare(&startup.gpu,&list)?;
        let mut displayed=false;
        if invisible {
            let target=startup.gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("hidden startup presentation"),
                size:wgpu::Extent3d {width:size.width,height:size.height,depth_or_array_layers:1},mip_level_count:1,sample_count:1,
                dimension:wgpu::TextureDimension::D2,format:startup.config.format,
                usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
            startup.presenter.present(&startup.gpu,&startup.background,&target.create_view(&Default::default()),Some(&startup.hud),None)?;
            if startup.build.display_frames()==0 {
                startup.gpu.wait();
                if let Some(path)=&self.args.capture {render::write_png(Path::new(&format!("{path}.startup.png")),size.width,size.height,&startup.gpu.read_rgba8(&target))?;}
            }
            displayed=true;
        } else {match startup.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)|wgpu::CurrentSurfaceTexture::Suboptimal(frame)=>{
                startup.presenter.present(&startup.gpu,&startup.background,&frame.texture.create_view(&Default::default()),Some(&startup.hud),None)?;
                window.pre_present_notify();startup.gpu.queue.present(frame);displayed=true;
            }
            wgpu::CurrentSurfaceTexture::Outdated=>startup.surface.configure(&startup.gpu.device,&startup.config),
            wgpu::CurrentSurfaceTexture::Lost=>{startup.surface=startup.instance.create_surface(window).map_err(|e|e.to_string())?;startup.surface.configure(&startup.gpu.device,&startup.config);}
            wgpu::CurrentSurfaceTexture::Timeout|wgpu::CurrentSurfaceTexture::Occluded=>{},
            wgpu::CurrentSurfaceTexture::Validation=>return Err("startup surface validation failure".into()),
        }}
        if displayed && startup.build.display_frames()==0 {
            eprintln!("startup: loading frame {} before graphics worker",if invisible {"completed offscreen"}else{"presented"});
            self.startup_timing.mark("first_loading_frame");
            self.startup_timing.emit("first_loading_frame",serde_json::json!({"hidden":invisible,"output_size":[size.width,size.height],"requested_output_size":self.requested_output_size}));
        }
        startup.build.displayed(&startup.gpu,displayed);self.trace(false);Ok(())
    }

    /// The worker owns scene construction and GPU resource creation. While it
    /// runs, this path only presents a cached image plus HUD, never the old
    /// renderer, live shader reload, sky update or a second device error scope.
    fn loading_frame(&mut self)->Result<(),String>{
        self.last_frame=Instant::now();self.accumulator=0.0;self.input.poll();
        for seat in 0..4 {let _=self.input.seat(seat,false);}
        self.input.clear_keys();
        self.input_edges=[(0,false);4];
        if let Some(host)=&mut self.host{host.state.suppress_buttons();}
        if let Some(sound)=&self.sound{sound.pause(true);}
        if let Some(result)=self.loading.as_mut().and_then(|l|l.poll()) {
            let loading=self.loading.take().unwrap();
            let mut report=loading.metadata();
            if loading.cancelled() {
                report["outcome"]="cancelled".into();
            } else {match result {
                Ok(loaded)=>{
                    let installing=Instant::now();
                    self.race=loaded.race;self.built=loaded.built;self.light=loaded.light;self.sky_metadata=loaded.sky;
                    let tree_report=crate::tree_tuning::Baseline::stage_metadata(loaded.tree_baseline.as_ref(),loaded.tree_settings);
                    self.tree_baseline=loaded.tree_baseline;
                    self.debug.tree_committed(loaded.tree_settings,tree_report);
                    self.exhaust=crate::exhaust_fx::ExhaustFx::new(self.args.seed);self.exhaust.step(&self.race);
                    self.wheel_effects=crate::wheel_effects::WheelEffects::new(self.args.seed);self.wheel_effects.step(&self.race);
                    self.graphics.as_mut().ok_or("GPU missing at stage commit")?.renderer=loaded.renderer;
                    self.debug.reset_measurements(self.rendered);self.debug.gpu_unavailable=false;
                    self.seats=loading.seats.clone();self.args.views=loading.views;self.active_settings=loading.settings.clone();
                    let seats=self.seats.clone();self.profiles(&seats);self.reset_round_input();
                    self.initial_positions=poses(&self.race).iter().map(|p|p.pos.to_array()).collect();
                    self.sound=None;self.last_frame=Instant::now();self.loading_error=None;
                    report["install_ms"]=(installing.elapsed().as_secs_f64()*1000.0).into();
                    report["elapsed_ms"]=(loading.started.elapsed().as_secs_f64()*1000.0).into();
                    report["outcome"]="ready".into();report["to_round"]=self.race.round.into();report["to_frame"]=self.race.frame.into();
                    report["to_noise_seed"]=self.race.stage.noise_seed.into();report["to_detail_seed"]=self.race.stage.detail_seed.into();
                    report["committed_at_app_ms"]=(self.started.elapsed().as_secs_f64()*1000.0).into();
                    for line in &self.built.log{eprintln!("scene rebuild: {line}");}
                }
                Err(error)=>{
                    eprintln!("next stage failed; previous stage retained: {error}");
                    report["outcome"]="failed".into();report["error"]=error.clone().into();
                    self.loading_error=Some(error.clone());self.paused=true;
                    self.screenshot_toast=Some((format!("Stage could not load: {error}"),Instant::now()+Duration::from_secs(30)));
                }
            }}
            eprintln!("next stage report: {report}");
            if let Some(file)=&mut self.gpu_profile{writeln!(file,"{}",serde_json::json!({"kind":"stage_load","report":report})).map_err(|e|format!("stage profile: {e}"))?;}
            self.last_stage_load=report;self.trace(true);
            if self.args.smoke_next_stage {if let Some(error)=&self.loading_error{return Err(format!("next-stage smoke failed: {error}"));}}
            return Ok(());
        }
        // Hosted pause/prepare owns the screen. Jobs may finish offscreen, but
        // a pending job is not launched until its loading status can be shown.
        if self.host.as_ref().is_some_and(|h|!h.state.is_running()){self.trace(false);return Ok(());}
        let size=self.window.as_ref().ok_or("loading window missing")?.inner_size();
        if size.width==0 || size.height==0{return Ok(());}
        let invisible=self.invisible();
        let list=self.loading.as_ref().unwrap().hud([size.width,size.height]);
        let g=self.graphics.as_mut().ok_or("loading GPU missing")?;
        // Ensure HUD and fallback resources exist before the worker starts.
        if g.hud.is_none(){g.hud=Some(render::hud::HudRenderer::new(&g.gpu,g.config.format)?);}
        g.hud.as_mut().unwrap().prepare(&g.gpu,&list)?;
        if self.cached_picture.is_none(){
            let texture=g.gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("loading background"),size:wgpu::Extent3d{width:1,height:1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
            g.gpu.queue.write_texture(wgpu::TexelCopyTextureInfo{texture:&texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},&[8,12,18,255],wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(4),rows_per_image:Some(1)},texture.size());
            self.cached_picture=Some(texture);
        }
        let picture=self.cached_picture.as_ref().unwrap();
        let mut displayed=false;
        if invisible {
            let target=g.gpu.device.create_texture(&wgpu::TextureDescriptor{label:Some("hidden loading presentation"),size:wgpu::Extent3d{width:size.width,height:size.height,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:g.config.format,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
            g.presenter.present(&g.gpu,picture,&target.create_view(&Default::default()),g.hud.as_ref(),None)?;
            // Only the first hidden loading frame is read back, before worker
            // creation. This proves the actual status image, not a UI mock-up.
            if self.loading.as_ref().unwrap().display_frames==0 {
                g.gpu.wait();
                if let Some(path)=&self.args.capture {render::write_png(Path::new(&format!("{path}.loading.png")),size.width,size.height,&g.gpu.read_rgba8(&target))?;}
            }
            displayed=true;
        } else {match g.surface.get_current_texture(){
            wgpu::CurrentSurfaceTexture::Success(frame)|wgpu::CurrentSurfaceTexture::Suboptimal(frame)=>{
                g.presenter.present(&g.gpu,picture,&frame.texture.create_view(&Default::default()),g.hud.as_ref(),None)?;
                self.window.as_ref().unwrap().pre_present_notify();g.gpu.queue.present(frame);self.presented+=1;displayed=true;
            }
            wgpu::CurrentSurfaceTexture::Outdated=>g.surface.configure(&g.gpu.device,&g.config),
            wgpu::CurrentSurfaceTexture::Lost=>{g.surface=g.instance.create_surface(self.window.as_ref().unwrap().clone()).map_err(|e|e.to_string())?;g.surface.configure(&g.gpu.device,&g.config);}
            wgpu::CurrentSurfaceTexture::Timeout|wgpu::CurrentSurfaceTexture::Occluded=>{},
            wgpu::CurrentSurfaceTexture::Validation=>return Err("loading surface validation failure".into()),
        }}
        if displayed {
            let loading=self.loading.as_mut().unwrap();
            if loading.display_frames==0{
                loading.progress.lock().unwrap_or_else(|e|e.into_inner()).report["first_loading_display_at_app_ms"]=(self.started.elapsed().as_secs_f64()*1000.0).into();
                eprintln!("next stage: loading frame {} before worker",if invisible{"completed offscreen"}else{"presented"});
            }
            loading.displayed(&g.gpu)?;
        }
        self.trace(false);Ok(())
    }

    fn poll_host(&mut self,event_loop:&ActiveEventLoop)->Result<bool,String>{
        // Preserve Client::poll's transaction boundary: do not advance host
        // state past a deferred Prepare before its ready acknowledgement.
        let commands=if self.loading.is_none() && self.tree_loading.is_none() && !self.deferred_host.is_empty(){
            std::mem::take(&mut self.deferred_host)
        } else if self.deferred_host.iter().any(|c|matches!(c,Command::Prepare{..})) {
            vec![]
        } else {self.host.as_mut().map(|h|h.poll()).unwrap_or_default()};
        for command in commands {
            if (self.loading.is_some() || self.tree_loading.is_some()) && matches!(&command,Command::Prepare{..}|Command::Setting{..}|Command::Roster{..}|Command::PlayersUpdated{..}|Command::Action{..}) {
                if matches!(&command,Command::Prepare{..}) {if let Some(loading)=&self.loading {loading.cancel();}}
                self.deferred_host.push(command);continue;
            }
            match command {
                Command::Connected=>{
                    let mut settings=crate::gamenight::settings();
                    for (key,value) in [("sky_time",self.sky_controls.desired().time_key()),("sky_weather",self.sky_controls.desired().atmosphere.key())] {
                        if let Some(setting)=settings.as_array_mut().and_then(|s|s.iter_mut().find(|s|s["key"]==key)){setting["default"]=value.into();}
                    }
                    if let Some(setting)=settings.as_array_mut().and_then(|s|s.iter_mut().find(|s|s["key"]=="upscale")) {setting["default"]=serde_json::json!(self.args.upscale.as_str());}
                    let host=self.host.as_ref().unwrap();host.declare_settings(settings)?;host.declare_actions(crate::gamenight::actions())?;
                }
                Command::Prepare{session,seats}=>{
                    self.managed_screen(false);
                    self.seats=seats;self.args.views=self.seats.len().clamp(1,4);self.race.settings.bots=self.args.views;
                    self.apply_settings();
                    self.race=Race::new(self.args.seed,self.race.settings.clone(),sideways::spec::Cars::load(None).map_err(|e|e.to_string())?);
                    self.exhaust=crate::exhaust_fx::ExhaustFx::new(self.args.seed);self.exhaust.step(&self.race);
                    self.wheel_effects=crate::wheel_effects::WheelEffects::new(self.args.seed);self.wheel_effects.step(&self.race);
                    self.ambient_clock=AmbientClock::new(self.args.smoke_frames>0);
                    let seats=self.seats.clone();self.profiles(&seats);self.paused=false;self.finished_round=0;self.reset_round_input();
                    self.rebuild()?;
                    let host=self.host.as_mut().unwrap();host.notify_participation(&session,false)?;host.notify_ready(&session)?;
                }
                Command::Start{..}|Command::Resume{..}=>{self.paused=false;self.input_edges=[(0,false);4];self.accumulator=0.0;self.last_frame=Instant::now();self.managed_screen(true);}
                Command::Pause{..}=>{self.accumulator=0.0;self.input_edges=[(0,false);4];self.managed_screen(false);}
                Command::Dispose{session}=>{
                    let superseded=self.host.as_ref().is_some_and(|h|!h.state.session.is_empty()&&h.state.session!=session);
                    if !superseded{self.finish(event_loop,"host_dispose");return Ok(false);}
                }
                Command::Roster{seats}|Command::PlayersUpdated{seats}=>{self.profiles(&seats);self.input_edges=[(0,false);4];}
                Command::Setting{key,value}=>{
                    if key=="sky_time" || key=="sky_weather" {
                        if let Err(error)=self.request_sky_setting(&key,&value,Instant::now()) {
                            eprintln!("live sky setting ignored: {error}");self.sky_controls.failed(error);
                        }
                        continue;
                    }
                    if key=="upscale" {
                        let result=crate::upscale::select(&value,&mut self.args.upscale,|mode| {
                            let g=self.graphics.as_ref().ok_or("GPU not initialized for upscaling selection")?;
                            g.renderer.set_upscale(&g.gpu,mode)
                        });
                        match result {
                            Ok(())=>{self.upscale_error=None;self.active_settings[&key]=value.clone();self.pending_settings[&key]=value;}
                            Err(reason)=>{
                                let message=format!("Upscaling unchanged ({}): {reason}",self.args.upscale.as_str());
                                eprintln!("{message}");self.upscale_error=Some(message.clone());
                                self.screenshot_toast=Some((message.clone(),Instant::now()+Duration::from_secs(10)));
                                if let Some(host)=&self.host {host.room_log(&message)?;}
                            }
                        }
                        continue;
                    }
                    self.pending_settings[&key]=value.clone();
                    if key=="gearbox" {
                        self.active_settings[&key]=value.clone();self.race.settings.manual=value.as_str()==Some("manual");
                        for p in &mut self.race.players{p.car.manual=self.race.settings.manual;}
                    }
                }
                Command::Action{key,..} if key=="new_stage"=>{self.next_stage()?;}
                Command::Disconnected(reason)=>{eprintln!("GameNight disconnected: {reason}");self.finish(event_loop,"host_disconnected");return Ok(false);}
                Command::Error(reason)=>return Err(format!("GameNight: {reason}")),
                _=>{}
            }
            self.trace(true);
        }
        Ok(true)
    }

    fn drain_frame_gpu(&mut self,call_id:u64) {
        let Some(g)=&self.graphics else{return;};
        let drain=g.renderer.drain_gpu_profiles(&g.gpu);
        self.debug.ingest(&drain,self.rendered);
        let Some(profile)=&mut self.frame_profile else{return;};
        for sample in &drain.samples {
            let intervals:Vec<_>=sample.intervals.iter().map(|interval|serde_json::json!({
                "name":interval.name,"start_ms":interval.start_ms,"end_ms":interval.end_ms,
                "parent":interval.parent,"view_index":interval.view_index})).collect();
            let view_pass_ms:Vec<_>=sample.view_pass_ms().into_iter().map(|(view_index,passes)|
                serde_json::json!({"view_index":view_index,"gpu_pass_ms":passes})).collect();
            profile.record(serde_json::json!({"kind":"gpu_frame","frame_id":sample.frame_id,"render_sequence":sample.render_sequence,
                "gpu_pass_ms":sample.gpu_ms,"instrumented_gpu_ms":sample.instrumented_gpu_ms,"covered_interval_ms":sample.covered_interval_ms,
                "covered_union_ms":sample.covered_union_ms,
                "gpu_pass_intervals":intervals,"gpu_view_pass_ms":view_pass_ms,
                "output_size":sample.output_size,"views":sample.views,"received_call_id":call_id}));
        }
        profile.record(serde_json::json!({"kind":"gpu_status","call_id":call_id,"round":self.race.round,"pending":drain.pending,
            "dropped":drain.dropped,"errors":drain.errors,"enabled":drain.enabled,"supported":drain.supported}));
    }

    fn apply_scenario_section(&mut self)->Result<bool,String> {
        let Some(scenario)=&mut self.scenario else{return Ok(false);};
        let Some(frame)=scenario.config.frame(self.rendered) else{return Ok(false);};
        if scenario.applied_section==Some(frame.index){return Ok(false);}
        let index=frame.index;let name=frame.section.name.clone();
        if let Some(prepared)=scenario.skies[index].as_ref() {
            let previous=scenario.applied_section.and_then(|i|scenario.skies[i].as_ref());
            if !previous.is_some_and(|p|Arc::ptr_eq(&p.sky,&prepared.sky)&&p.settings==prepared.settings) {
                let g=self.graphics.as_mut().ok_or("scenario GPU not ready")?;
                g.renderer.update_sky(&g.gpu,&prepared.sky,&mut self.light)?;
                prepared.apply_scene(&mut self.built.scene,&mut self.light);self.sky_metadata=prepared.metadata.clone();
                self.sky_controls.accepted(prepared.clone());
            }
        }
        if let Some(g)=&self.graphics {g.renderer.reset_history();}
        scenario.applied_section=Some(index);eprintln!("scenario section {}: {} at tick {}",index+1,name,frame.target_tick);Ok(true)
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let (debug_start,debug_cadence)=self.debug.begin_frame();
        let rendered_before=self.rendered;
        let mut trace=self.frame_profile.as_mut().map(|p|p.begin());
        if trace.is_some() || self.debug.ui.open {let call_id=trace.as_ref().map_or(0,|t|t.call_id);
            frame_profile::section(&mut trace,"gpu_profile_poll");self.drain_frame_gpu(call_id);
        }
        let presented_before=self.presented;
        let result=self.frame_body(event_loop,&mut trace);
        self.debug.end_frame(self.rendered,debug_start,debug_cadence,self.rendered>rendered_before);
        if let Some(trace)=trace {
            let output_size=self.graphics.as_ref().map(|g|[g.config.width,g.config.height])
                .or_else(||self.startup.as_ref().map(|g|[g.config.width,g.config.height]))
                .or_else(||self.window.as_ref().map(|w|{let s=w.inner_size();[s.width,s.height]}));
            let mut metadata=serde_json::json!({"presented":self.presented>presented_before,"views":self.args.views,
                "engine_debug":self.debug.metadata(),
                "output_size":output_size,"requested_output_size":self.requested_output_size,"race_frame":self.race.frame,"round":self.race.round,
                "paused":self.paused,"focused":self.focused,"hidden":self.invisible(),
                "error":result.as_ref().err()});
            if let Some(scenario)=&self.scenario {if let Some(frame)=scenario.config.frame(trace.frame_id.map(|id|id-1).unwrap_or(self.rendered)) {
                metadata["scenario"]=scenario.config.name.clone().into();metadata["section"]=frame.section.name.clone().into();
                metadata["section_frame"]=frame.local_frame.into();metadata["warmup"]=frame.warmup.into();metadata["target_tick"]=frame.target_tick.into();
            }}
            let row=trace.finish(metadata);
            if let Some(profile)=&mut self.frame_profile {profile.record(row);}
        }
        result
    }

    fn frame_body(&mut self, event_loop: &ActiveEventLoop,trace:&mut Option<frame_profile::Trace>) -> Result<(), String> {
        frame_profile::section(trace,"input_host");
        // Host commands are buffered until the real renderer is installed;
        // no command can rebuild/upload into the temporary empty scene.
        if self.startup.is_some(){frame_profile::phase(trace,"startup_loading");frame_profile::section(trace,"startup_loading");return self.startup_frame();}
        if self.tree_loading.is_some(){
            if !self.poll_host(event_loop)? {return Ok(());}
            frame_profile::phase(trace,"vegetation_loading");frame_profile::section(trace,"vegetation_loading");return self.tree_loading_frame();
        }
        let scenario_section_changed=self.apply_scenario_section()?;
        if !self.poll_host(event_loop)?{return Ok(());}
        if self.loading.is_some(){frame_profile::phase(trace,"stage_loading");frame_profile::section(trace,"stage_loading");return self.loading_frame();}
        self.debug_sky_requests(Instant::now());
        self.debug.script_frame(self.rendered+1,[self.args.size.0,self.args.size.1]);
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f64().min(0.25);
        self.last_frame = now;
        if self.focused {self.debug.tick(dt,[self.args.size.0,self.args.size.1]);}
        self.debug_sky_requests(now);
        if let Some(settings)=self.debug.take_tree_apply() {
            match self.queue_tree_edit(settings) {
                Ok(())=>{frame_profile::phase(trace,"vegetation_loading");return self.tree_loading_frame();},
                Err(error)=>self.debug.tree_failed(&error),
            }
        }
        self.input.poll();
        let mut inputs: Vec<SeatInput> = (0..self.args.views).map(|i|self.input.seat(i, self.focused)).collect();
        if let Some(host)=&mut self.host {
            for (i,input) in inputs.iter_mut().enumerate(){
                let seat=self.seats.get(i);
                let remote=seat.filter(|s|!s.ai&&!s.controller.is_empty()).map(|s|host.state.controls_for_seat(s.index,now,true)).unwrap_or_default();
                *input=SeatInput{control:remote.control,look:[remote.look[0] as f32,remote.look[1] as f32],camera_pressed:remote.camera_pressed,pause_pressed:remote.start_pressed};
            }
        }
        let debug_owned_input=self.debug.ui.open;
        if self.scenario.is_none() && inputs.iter().any(|i|i.pause_pressed) && debug_owned_input {
            self.debug.key(DebugKey::Toggle,[self.args.size.0,self.args.size.1]);
            let (axis,confirm)=self.input.menu_state(self.focused);self.pause_menu.suspend(axis,confirm);
            self.input.clear_keys();if let Some(host)=&mut self.host{host.state.suppress_buttons();}
        } else if self.scenario.is_none() && inputs.iter().any(|i|i.pause_pressed) {
            self.paused = !self.paused;if let Some(host)=&mut self.host{host.state.suppress_buttons();if self.paused{host.request_overlay()?;}}self.trace(true);
            if self.paused && self.host.is_none(){let (axis,confirm)=self.input.menu_state(self.focused);self.pause_menu.opened(axis,confirm);}
        }
        if self.scenario.is_none() && !debug_owned_input {self.standalone_menu(now)?;}
        else if debug_owned_input {let (axis,confirm)=self.input.menu_state(self.focused);self.pause_menu.suspend(axis,confirm);}
        if self.loading.is_some(){frame_profile::phase(trace,"stage_loading");frame_profile::section(trace,"stage_loading");return self.loading_frame();}
        self.poll_sky_update(now);
        if self.input.take_key(KeyCode::KeyN) && !debug_owned_input && self.host.is_none() && self.scenario.is_none() { self.next_stage()?;frame_profile::phase(trace,"stage_loading");frame_profile::section(trace,"stage_loading");return self.loading_frame(); }
        if self.input.take_key(KeyCode::KeyM) && !debug_owned_input && self.host.is_none() && self.scenario.is_none() {
            self.race.settings.manual = !self.race.settings.manual;
            for p in &mut self.race.players { p.car.manual = self.race.settings.manual; }
        }
        if self.input.take_key(KeyCode::F11) {
            if let Some(w) = &self.window { w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) }); }
        }
        // Input::take_key consumes only the press edge, not OS key repeats.
        if self.input.take_key(KeyCode::F12) && self.focused && !self.invisible() {
            self.screenshot_pending = true;
        }
        if self.args.smoke_frames > 0 && self.host.is_none() && self.scenario.is_none() {
            self.race.phase = "racing"; self.race.clock = self.race.clock.max(0.0);
            for i in &mut inputs { i.control = Control { throttle: 0.75, steer: 0.04, ..Default::default() }; }
        }
        let running=self.host.as_ref().map(|h|h.state.is_running()).unwrap_or(true);
        if running && self.focused && !self.invisible() {
            if let Some(host)=&mut self.host {host.report_activity()?;}
        }
        if self.paused || !running {self.input_edges=[(0,false);4];for input in &mut inputs{*input=SeatInput::default();}}
        if self.scenario.is_some(){for input in &mut inputs{*input=SeatInput::default();}}
        let mut history_cut=scenario_section_changed;
        for (i, input) in inputs.iter().enumerate() {
            self.look[i]=input.look;
            if self.paused || !running { continue; }
            if input.control.shift!=0{self.input_edges[i].0=input.control.shift;}
            self.input_edges[i].1|=input.control.recover;
            if input.camera_pressed { self.far[i] = !self.far[i]; history_cut=true; }
            self.yaw[i] = crate::chase_camera::orbit(self.yaw[i], input.look, dt as f32, &mut self.orbit_active[i]);
        }
        self.controls = inputs.iter().map(|i|i.control).collect();
        frame_profile::phase(trace,if self.paused {"paused"}else if !running {"host_idle"}else{"gameplay"});
        frame_profile::section(trace,"physics");
        let old_round = self.race.round;
        let mut rebuilt=false;
        let mut simulated_ticks=0u32;
        let autoplay=self.args.autoplay&&self.host.is_none();
        if let Some(scenario)=&self.scenario {
            let frame=scenario.config.frame(self.rendered).ok_or("scenario exhausted before rendering")?;
            let (race,exhaust,wheels)=(&mut self.race,&mut self.exhaust,&mut self.wheel_effects);
            simulated_ticks=scenario::advance_to(race,frame.target_tick,|race|{exhaust.step(race);wheels.step(race);})?;
            self.accumulator=0.0;
        } else if !self.paused && running && (self.focused || autoplay || self.args.smoke_frames > 0 || self.host.is_some()) {
            self.accumulator += if self.args.smoke_frames > 0 && self.host.is_none() { 8.0 / 120.0 } else { dt };
            let mut controls: Vec<_> = self.controls.iter().enumerate().map(|(i,c)| {
                if autoplay || (self.host.is_some()&&self.seats.get(i).is_some_and(|s|s.ai)){None}else{let mut c=*c;c.shift=self.input_edges[i].0;c.recover=self.input_edges[i].1;Some(c)}
            }).collect();
            while self.accumulator >= 1.0 / 120.0 {
                if self.race.phase=="results" && self.race.results<=1.0/120.0 {
                    self.next_stage()?;rebuilt=true;break;
                }
                history_cut |= controls.iter().flatten().any(|c|c.recover);
                self.race.step_with_inputs(&mut Timings::default(), &controls);
                self.exhaust.step(&self.race);
                self.wheel_effects.step(&self.race);
                simulated_ticks+=1;
                self.input_edges=[(0,false);4];
                for c in controls.iter_mut().flatten() { c.shift = 0; c.recover = false; }
                self.accumulator -= 1.0 / 120.0;
            }
        } else { self.accumulator = 0.0; }
        frame_profile::section(trace,"scene_grass_update");
        if self.loading.is_some(){frame_profile::phase(trace,"stage_loading");frame_profile::section(trace,"stage_loading");return self.loading_frame();}
        if self.race.round != old_round && !rebuilt { self.rebuild()?; }
        if self.race.phase=="results"&&self.finished_round!=self.race.round {
            self.finished_round=self.race.round;if let Some(host)=&self.host{host.notify_finished(&host.state.session)?;}
        }
        // Keep the host socket, neutral input and trace alive while the host
        // owns the screen. A warm/paused managed game must not consume a full
        // render frame or continue presenting behind the GameNight lobby.
        if !running {if let Some(s)=&self.sound{s.pause(true);}self.trace(false);return Ok(());}
        let graphics_started=Instant::now();
        if let Some(q)=&mut self.quality {
            if !self.paused && self.focused && !rebuilt && q.frame(dt*1000.0,now) {println!("play: adaptive quality, {}",q.label());}
        }
        if self.rendered==0 { println!("play: first frame entered (invisible={})", self.invisible()); }
        let poses = poses(&self.race);
        self.built.update_cars(&poses);
        let cams = cameras(&self.race, &poses, &self.yaw, &self.far, &mut self.camera_rest);
        let cams=self.scenario.as_ref().map(|s|s.config.cameras(self.rendered,&cams)).unwrap_or(cams);
        let invisible = self.invisible();
        self.update_sound(&cams, dt as f32, invisible);
        let size = self.window.as_ref().unwrap().inner_size();
        if size.width == 0 || size.height == 0 { return Ok(()); }
        let render_light=self.debug.effective_light(&self.light);
        self.ambient_clock.advance(dt, self.race.frame, !invisible, running, self.paused);
        let ordered:Vec<_>=self.view_order.iter().filter_map(|i|cams.get(*i).copied()).collect();
        let (render_w,render_h)=self.quality.as_ref().map_or((size.width,size.height),|q|q.render_size(size.width,size.height));
        let views = crate::shot::view_rects(if ordered.is_empty(){&cams}else{&ordered}, self.args.views, render_w, render_h);
        // The fixed pool sits before the cars. World-cell changes upload only
        // its dirty range; unchanged camera anchors require no grass upload.
        let grass_dirty=self.built.update_ground_cover(&views.iter().map(|v|v.cam.pos).collect::<Vec<_>>());
        frame_profile::section(trace,"hud_layout");
        let layout_started=Instant::now();
        if self.screenshot_toast.as_ref().is_some_and(|(_,until)|now>=*until){self.screenshot_toast=None;}
        let mut draw_list=if self.paused && self.host.is_none() {
            let settings=self.sky_controls.desired();
            Some(crate::hud::live_pause_menu([size.width,size.height],self.pause_menu.selected,
                settings.time_label(),settings.atmosphere.label(),self.sky_controls.status()))
        } else if self.args.hud {
            Some(crate::hud::build(&self.race,&self.view_order,&render::split_rects(self.args.views,size.width,size.height).into_iter().map(|(x,y,w,h)|[x,y,w,h]).collect::<Vec<_>>(),
                &self.race.players.iter().map(|p|p.name.clone()).collect::<Vec<_>>(),self.paused,[size.width,size.height]))
        } else {None};
        if let (Some(list),Some((message,_)))=(&mut draw_list,&self.screenshot_toast){crate::hud::screenshot_toast(list,message);}
        if self.debug.ui.open {
            let list=draw_list.get_or_insert_with(||render::hud::HudDrawList::new([size.width,size.height]));
            list.begin_overlay();
            self.debug.draw(list);
        }
        let layout_ms=layout_started.elapsed().as_secs_f64()*1000.0;
        frame_profile::section(trace,"hud_prepare");
        let g = self.graphics.as_mut().ok_or("GPU not initialized")?;
        g.renderer.set_debug_options(self.quality.as_ref().map_or(self.debug.options,|q|q.apply(self.debug.options)))?;
        if self.debug.take_history_reset(){g.renderer.reset_history();}
        if draw_list.is_some() && g.hud.is_none() {g.hud=Some(render::hud::HudRenderer::new(&g.gpu,g.config.format)?);}
        let prepare_started=Instant::now();
        if let (Some(hud),Some(list))=(&mut g.hud,&draw_list){
            let stats=hud.prepare(&g.gpu,list)?;
            self.hud_stats=serde_json::json!({"output_size":list.size,"text_runs":stats.text_runs,"glyphs":stats.glyphs,"vertices":stats.vertices,"reshaped":stats.reshaped});
        }
        let prepare_ms=prepare_started.elapsed().as_secs_f64()*1000.0;
        frame_profile::section(trace,"scene_uploads");
        let first = self.built.cars.first().map(|c|c.first).unwrap_or(self.built.scene.instances.len());
        if let Some(range)=grass_dirty {g.renderer.update_instances(&g.gpu,range.start,&self.built.scene.instances[range])?;}
        g.renderer.update_instances(&g.gpu, first, &self.built.scene.instances[first..])?;
        if let Some((log,_))=self.live_edit.poll(Duration::from_millis(250),&g.gpu,&mut g.renderer,&self.built.car_slots){
            for l in log{eprintln!("live edit: {l}");}
            // Keep the CPU copy in step with live paint edits before applying
            // this frame's independent per-car lamp status.
            self.built.scene.car_materials=self.live_edit.looks().materials(&self.built.car_slots)?;
        }
        g.renderer.set_wind(&g.gpu,&self.ambient_clock.wind(self.args.nature))?;
        g.renderer.set_animation_time(self.ambient_clock.seconds)?;
        let headlight_indices:Vec<_>=if self.view_order.is_empty(){(0..views.len()).collect()}else{self.view_order.iter().copied().take(views.len()).collect()};
        let headlights=if self.debug.options.headlights_enabled {crate::car_headlights::from_poses(&poses,&headlight_indices,&render_light)}else{vec![]};
        g.renderer.set_headlights(&headlights)?;
        let rear_states:Vec<_>=self.race.players.iter().map(|p|{
            let c=&p.car;rear_light_state(c.gear,c.manual,c.hold,c.brake,c.throttle)
        }).collect();
        let mut local_lights=if self.debug.options.headlights_enabled {
            crate::car_headlights::local_from_poses(&poses,&headlight_indices,&render_light,&rear_states)
        }else{vec![]};
        let vehicle_local_light_count=local_lights.len();
        if self.debug.options.exhaust_enabled {local_lights.extend(self.exhaust.lights());}
        // Upload the empty list too, so releasing a pedal/flash or disabling a
        // feature cannot reuse the previous frame's illumination.
        g.renderer.set_local_lights(&local_lights)?;
        g.renderer.set_exhaust(&g.gpu,&self.exhaust.sprites())?;
        self.wheel_effects.upload(&g.gpu,&mut g.renderer)?;
        g.renderer.set_lighting_focus(&headlight_indices.iter().filter_map(|i|poses.get(*i).map(|p|p.pos)).collect::<Vec<_>>())?;
        if let Some(scenario)=&self.scenario {
            let fixed=scenario.config.frame(self.rendered).is_some_and(|f|!f.section.cameras.is_empty());
            let focus:Vec<_>=if fixed {views.iter().map(|v|v.cam.target).collect()}else{headlight_indices.iter().filter_map(|i|poses.get(*i).map(|p|p.pos)).collect()};
            g.renderer.set_lighting_focus(&focus)?;g.renderer.set_sun_shadow_focus(&focus)?;
        }
        let rear_lamps=crate::car_lamps::from_race(&self.race,&render_light,poses.len());
        if crate::car_lamps::apply(&mut self.built.scene.car_materials,&self.built.car_slots,&rear_lamps)? {
            g.renderer.set_car_materials(&g.gpu,&self.built.scene.car_materials)?;
        }
        // Hidden standalone smoke advances a deterministic number of physics
        // ticks per rendered frame. Interactive and hosted play use wall time.
        let frame_time_ms=if let Some(s)=&self.scenario {s.config.ticks_per_frame.max(1) as f32*(1000.0/120.0)}else if self.args.smoke_frames>0 && self.host.is_none() {
            simulated_ticks as f32*(1000.0/120.0)
        } else {(dt*1000.0) as f32};
        g.renderer.set_frame_time_ms(frame_time_ms);
        if history_cut {g.renderer.reset_history();}
        // F12 and the final hidden smoke capture measure this same frame;
        // normal play retains the no-readback render path.
        let measure_profile=self.gpu_profile.is_some() && self.rendered>=self.args.gpu_warmup as u64;
        let stage_smoke_done=self.args.smoke_next_stage && self.last_stage_load["outcome"]=="ready";
        let scenario_capture=self.scenario.as_ref().and_then(|s|s.config.capture_path(self.rendered+1));
        let measure_capture=measure_profile || self.screenshot_pending || scenario_capture.is_some() || (self.args.smoke_frames>0 && (stage_smoke_done || self.rendered+1>=self.args.smoke_frames) && self.args.capture.is_some());
        let want_gpu=(self.debug.ui.open || self.frame_profile.is_some()) && !self.debug.gpu_unavailable;
        if let Err(error)=g.renderer.set_async_gpu_profiling(&g.gpu,want_gpu) {
            eprintln!("debug: GPU timing unavailable: {error}; CPU timing continues");
            self.debug.gpu_unavailable=true;
            if let Some(profile)=&mut self.frame_profile {profile.gpu_unavailable=true;
                profile.record(serde_json::json!({"kind":"gpu_status","enabled":false,"supported":false,"error":error}));}
        } else if want_gpu {g.renderer.set_gpu_profile_frame_id(self.rendered+1);}
        if let Some(t)=trace {t.diagnostic_sync=measure_capture||invisible||self.args.hud_profile.is_some();}
        if let Some(t)=trace {t.render_state_hash=Some(frame_profile::render_state_hash(&views,&render_light,&poses));}
        frame_profile::section(trace,"render_encode_submit");
        let (image,capture_stats)=if measure_capture {
            // Captures keep the full output size, so the HUD lines up and
            // F12 shows every pixel; adaptive quality applies to play only.
            let full=crate::shot::view_rects(if ordered.is_empty(){&cams}else{&ordered}, self.args.views, size.width, size.height);
            let (image,stats)=g.renderer.try_render_texture_profiled(&g.gpu,&full,&render_light,size.width,size.height)?;
            (image,Some(stats))
        } else {
            let k=(self.rendered%2) as usize;
            if self.outputs[k].as_ref().is_none_or(|t|(t.width(),t.height())!=(render_w,render_h)) {
                self.outputs[k]=Some(render::Renderer::output_texture(&g.gpu,render_w,render_h));
            }
            let output=self.outputs[k].clone().unwrap();
            g.renderer.try_render_into(&g.gpu, &views, &render_light, &output)?;
            (output,None)
        };
        self.cached_picture=Some(image.clone());
        self.rendered += 1;
        if let Some(t)=trace {t.frame_id=Some(self.rendered);}
        frame_profile::section(trace,"unattributed");
        if measure_profile {
            let stats=capture_stats.as_ref().expect("profiled frame has stats");
            let mut sample=serde_json::json!({"kind":"frame","rendered_frame":self.rendered,"race_frame":self.race.frame,"race_clock":self.race.clock,
                "round":self.race.round,"phase":self.race.phase,"paused":self.paused,"weather":self.race.weather,
                "stage_noise_seed":self.race.stage.noise_seed,"stage_detail_seed":self.race.stage.detail_seed,
                "nature":self.ambient_clock.metadata(self.args.nature),"output_size":[size.width,size.height],"sky":self.sky_metadata,
                "crowds":crate::roadside_crowds::metadata(&self.built.scene),
                "grass_detail_profile":self.built.scene.grass_detail.label(),"ground_cover_streaming":grass_stream_metadata(&self.built),
                "upscale":crate::upscale::metadata(self.args.upscale,&stats.upscale),
                "frame_time_ms":frame_time_ms.clamp(1.0,100.0),"history_cut":history_cut,
                "views":views.iter().map(|v|serde_json::json!({"rect":[v.x,v.y,v.w,v.h],"position":v.cam.pos.to_array(),"target":v.cam.target.to_array()})).collect::<Vec<_>>(),
                "car_positions":poses.iter().map(|p|p.pos.to_array()).collect::<Vec<_>>(),
                "timings_available":!stats.gpu_ms.is_empty(),"gpu_pass_ms":stats.gpu_ms,
                "instrumented_gpu_ms":if stats.gpu_ms.is_empty(){None}else{Some(stats.gpu_ms.iter().map(|(_,v)|v).sum::<f64>())},
                "clusters":stats.clusters,"triangles":stats.triangles,"shadow_clusters":stats.shadow_clusters,"shadow_triangles":stats.shadow_triangles,"submitted_vertices":stats.submitted_vertices,"shadow_submitted_vertices":stats.shadow_submitted_vertices,"early_grass_rejects":stats.early_grass_rejects,"cluster_buckets":stats.cluster_buckets,"early_grass_cull":stats.early_grass_cull,
                "canopy_updated_levels":stats.canopy_updated_levels,"canopy_pitch_m":stats.canopy_pitch_m,"camera_sun_visibility":stats.camera_sun_visibility});
            sample["foliage_sun"]=foliage_sun_metadata(stats);
            sample["static_sun"]=static_sun_metadata(stats);
            sample["instance_groups"]=stats.instance_groups.into();
            sample["cluster_traversal"]=stats.cluster_traversal.into();
            sample["sun_instance_groups"]=stats.sun_instance_groups.into();
            sample["headlight_instance_groups"]=stats.headlight_instance_groups.into();
            writeln!(self.gpu_profile.as_mut().unwrap(),"{sample}").map_err(|e|format!("GPU profile: {e}"))?;
        }
        let presented_before=self.presented;
        let mut hidden_target=None;
        if self.rendered==1 { println!("play: first scene frame submitted"); }
        frame_profile::section(trace,"present_acquire");
        if invisible {
            // Win32 does not guarantee redraw events or an acquirable swapchain
            // image for invisible windows. Exercise the identical blit pipeline
            // against the actual surface format without pretending to present.
            let target=g.gpu.device.create_texture(&wgpu::TextureDescriptor {
                label:Some("hidden presentation target"),
                size:wgpu::Extent3d{width:size.width,height:size.height,depth_or_array_layers:1},
                mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,
                format:g.config.format,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[],
            });
            frame_profile::section(trace,"present_submit");
            g.presenter.present(&g.gpu,&image,&target.create_view(&Default::default()),g.hud.as_ref().filter(|_|draw_list.is_some()),g.hud_profile.as_mut())?;
            hidden_target=Some(target);
        } else { match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                frame_profile::section(trace,"present_submit");
                g.presenter.present(&g.gpu, &image, &frame.texture.create_view(&Default::default()),g.hud.as_ref().filter(|_|draw_list.is_some()),None)?;
                self.window.as_ref().unwrap().pre_present_notify();
                g.gpu.queue.present(frame); self.presented += 1;
            }
            wgpu::CurrentSurfaceTexture::Outdated => g.surface.configure(&g.gpu.device, &g.config),
            wgpu::CurrentSurfaceTexture::Lost => {
                g.surface = g.instance.create_surface(self.window.as_ref().unwrap().clone()).map_err(|e|e.to_string())?;
                g.surface.configure(&g.gpu.device, &g.config);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {},
            wgpu::CurrentSurfaceTexture::Validation => return Err("surface validation failure".into()),
        }}
        frame_profile::section(trace,"diagnostic_wait");
        // Hidden smoke deliberately waits: there is no swapchain backpressure
        // to bound queued frames. Its samples remain diagnostic_sync; actual
        // asynchronous frame-budget measurement uses the visible scenario.
        if invisible {g.gpu.wait();}
        frame_profile::section(trace,"unattributed");
        if self.args.hud_profile.is_some(){self.hud_samples.push(serde_json::json!({"frame":self.rendered,"race_frame":self.race.frame,"hud_layout_cpu_ms":layout_ms,"hud_prepare_cpu_ms":prepare_ms,"graphics_wall_ms":graphics_started.elapsed().as_secs_f64()*1000.0,"stats":self.hud_stats}));}
        if self.rendered==1 { println!("play: first frame complete (presented={})",self.presented); }
        // Loading frames are presented too, but launcher readiness must mean
        // an actual scene frame, including after a delayed surface recovery.
        if self.presented>presented_before && self.last_startup.get("first_visible_frame_at_app_ms").is_none() {
            self.last_startup["first_visible_frame_at_app_ms"]=(self.started.elapsed().as_secs_f64()*1000.0).into();
            println!("play: first visible frame complete");
        }
        if self.last_startup.get("first_scene_frame_at_app_ms").is_none() && (invisible||self.presented>presented_before) {
            self.last_startup["first_scene_frame_at_app_ms"]=(self.started.elapsed().as_secs_f64()*1000.0).into();
            self.last_startup["first_scene_rendered_frame"]=self.rendered.into();
            self.startup_timing.mark("first_scene_frame");self.startup_timing.complete();
            self.startup_timing.emit("first_scene_frame",serde_json::json!({"hidden":invisible,"output_size":[size.width,size.height],"requested_output_size":self.requested_output_size,"rendered_frame":self.rendered,"worker":self.last_startup}));
        }
        if self.last_stage_load["outcome"]=="ready" && self.last_stage_load.get("first_new_stage_frame_at_app_ms").is_none() && (invisible||self.presented>presented_before) {
            self.last_stage_load["first_new_stage_frame_at_app_ms"]=(self.started.elapsed().as_secs_f64()*1000.0).into();
            self.last_stage_load["first_new_stage_rendered_frame"]=self.rendered.into();
            eprintln!("next stage: first new-stage frame complete, round {}",self.race.round);
        }
        if self.args.smoke_next_stage && !self.smoke_stage_requested && (invisible||self.presented>presented_before) {
            self.smoke_stage_requested=true;self.next_stage()?;return Ok(());
        }
        let mut meadow_capture = None;
        if let Some(path)=scenario_capture {
            frame_profile::section(trace,"diagnostic_readback");
            let capture=self.scenario.as_ref().unwrap().config.capture;
            let (capture_width,capture_height,rgba)=g.presenter.capture_rgba_scaled(&g.gpu,&image,g.config.format,g.hud.as_ref().filter(|_|draw_list.is_some()),capture.max_width)?;
            capture.write(&path,capture_width,capture_height,&rgba)?;
            let scenario=self.scenario.as_ref().unwrap();let frame=scenario.config.frame(self.rendered-1).unwrap();
            let render_diagnostic=render_diagnostic::capture(&g.renderer,&g.gpu,&self.built.scene,views.len(),self.rendered)?;
            meadow_capture=Some(meadow_capture_metadata(&g.renderer,&g.gpu,&self.built.scene)?);
            let mut metadata=serde_json::json!({"scenario":scenario.config.metadata(),"frame_id":self.rendered,"section":frame.section.name,
                "engine_debug":self.debug.metadata(),
                "render_lighting":lighting_metadata(&render_light),
                "static_sun":capture_stats.as_ref().map(static_sun_metadata),
                "tree_occlusion":render_diagnostic["tree_occlusion"],"tree_source":self.built.tree_source,
                "render_diagnostic":render_diagnostic,"meadow":meadow_capture.as_ref(),
                "section_frame":frame.local_frame,"warmup":frame.warmup,"race_frame":self.race.frame,"round":self.race.round,
                "seed":self.args.seed,"stage_noise_seed":self.race.stage.noise_seed,"stage_detail_seed":self.race.stage.detail_seed,
                "source_output_size":[size.width,size.height],"capture_size":[capture_width,capture_height],"capture_format":capture.name(),
                "views":views.iter().map(|v|serde_json::json!({"rect":[v.x,v.y,v.w,v.h],"position":v.cam.pos.to_array(),"target":v.cam.target.to_array(),"fov_y":v.cam.fov_y})).collect::<Vec<_>>(),
                "player_positions":poses.iter().map(|p|p.pos.to_array()).collect::<Vec<_>>(),"sky":self.sky_metadata,
                "headlights":crate::car_headlights::metadata_for_poses(&headlights,&poses,&headlight_indices),
                "local_lights":local_lights_metadata(&local_lights,vehicle_local_light_count),
                "sun_direction":render_light.sun_dir.to_array(),"sun_lux":render_light.sun_lux,"sky_lux":render_light.sky_lux,"ev100":render_light.ev100,
                "scope":"Actual frame captured from the shared scenario renderer; capture readback is synchronous and excluded from steady-frame summaries."});
            if std::env::var("KERN_MEADOW_CAPTURE_RAW").as_deref()==Ok("1") {
                metadata["meadow_raw"] = render_diagnostic::capture_meadow_raw(&g.renderer,&g.gpu,&path,self.rendered)?;
            }
            std::fs::write(path.with_extension("json"),serde_json::to_vec_pretty(&metadata).unwrap()).map_err(|e|format!("scenario capture metadata: {e}"))?;
            frame_profile::section(trace,"unattributed");
        }
        if self.args.smoke_frames > 0 && (if self.args.smoke_next_stage {stage_smoke_done}else{self.rendered>=self.args.smoke_frames}) {
            if let Some(t)=trace {t.diagnostic_sync=true;}
            frame_profile::section(trace,"diagnostic_readback");
            if let Some(views)=g.renderer.far_grass_diagnostics(&g.gpu)? {
                for (index,view) in views.iter().enumerate() {
                    println!("far grass view {index}: traced={}, overflow={}, max_iterations={}",view.traced_rays,view.overflow_rays,view.max_iterations);
                    if view.overflow_rays>0 {return Err(format!("far grass view {index} had {} incomplete rays",view.overflow_rays));}
                }
            }
            if let Some(path) = &self.args.capture {
                let target=hidden_target.as_ref().ok_or("capture requires the final hidden presentation texture")?;
                if !matches!(target.format(),wgpu::TextureFormat::Rgba8Unorm|wgpu::TextureFormat::Rgba8UnormSrgb|wgpu::TextureFormat::Bgra8Unorm|wgpu::TextureFormat::Bgra8UnormSrgb){return Err("capture requires an 8-bit RGBA/BGRA surface".into());}
                render::write_png(Path::new(path), size.width, size.height, &g.gpu.read_rgba8(target))?;
                if let Some(stats)=&capture_stats {
                    let metrics=serde_json::json!({"schema":1,"capture":path,"output_size":[size.width,size.height],"views":views.len(),"upscale":crate::upscale::metadata(self.args.upscale,&stats.upscale),"gpu_pass_ms":stats.gpu_ms,"timings_available":!stats.gpu_ms.is_empty(),"scope":"same-frame scene passes; excludes HUD, presentation and CPU; not gameplay FPS","triangles":stats.triangles,"clusters":stats.clusters,"submitted_vertices":stats.submitted_vertices,"shadow_submitted_vertices":stats.shadow_submitted_vertices,"early_grass_rejects":stats.early_grass_rejects,"cluster_buckets":stats.cluster_buckets,"early_grass_cull":stats.early_grass_cull,"nature":self.ambient_clock.metadata(self.args.nature),"grass_detail_profile":self.built.scene.grass_detail.label(),"ground_cover_streaming":grass_stream_metadata(&self.built),"canopy_updated_levels":stats.canopy_updated_levels,"canopy_pitch_m":stats.canopy_pitch_m,"camera_sun_visibility":stats.camera_sun_visibility});
                    std::fs::write(format!("{path}.metrics.json"),serde_json::to_vec_pretty(&metrics).unwrap()).map_err(|e|format!("capture metrics: {e}"))?;
                }
            }
            g.gpu.wait();
            self.finish(event_loop, "smoke_frame_limit");
            return Ok(());
        }
        if self.scenario.as_ref().is_some_and(|s|self.rendered>=s.config.total_frames) {
            self.finish(event_loop,"scenario_frame_limit");return Ok(());
        }
        if self.screenshot_pending && self.presented>presented_before {
            frame_profile::section(trace,"diagnostic_readback");
            self.screenshot_pending=false;
            let g=self.graphics.as_ref().unwrap();
            let meadow=match meadow_capture {
                Some(metadata)=>metadata,
                None=>meadow_capture_metadata(&g.renderer,&g.gpu,&self.built.scene)?,
            };
            let capture_profile=capture_stats.as_ref().map(|s|serde_json::json!({"upscale":crate::upscale::metadata(self.args.upscale,&s.upscale),"gpu_pass_ms":s.gpu_ms,"timings_available":!s.gpu_ms.is_empty(),"triangles":s.triangles,"clusters":s.clusters,"submitted_vertices":s.submitted_vertices,"shadow_submitted_vertices":s.shadow_submitted_vertices,"early_grass_rejects":s.early_grass_rejects,"cluster_buckets":s.cluster_buckets,"early_grass_cull":s.early_grass_cull,"canopy_updated_levels":s.canopy_updated_levels,"canopy_pitch_m":s.canopy_pitch_m,"camera_sun_visibility":s.camera_sun_visibility,"scope":"same-frame scene passes; excludes HUD, presentation and CPU; not gameplay FPS"}));
            let mut metadata=serde_json::json!({
                "schema":1,"seed":self.args.seed,"stage_noise_seed":self.race.stage.noise_seed,"stage_detail_seed":self.race.stage.detail_seed,"weather":self.race.weather,"state":self.snapshot(),
                "player_poses":poses.iter().enumerate().map(|(i,p)|serde_json::json!({"index":i,"position":p.pos.to_array(),"rotation_xyzw":p.rot.to_array(),"model":p.model})).collect::<Vec<_>>(),
                "views":views.iter().map(|v|serde_json::json!({"rect":[v.x,v.y,v.w,v.h],"camera":{"position":v.cam.pos.to_array(),"target":v.cam.target.to_array(),"fov_y":v.cam.fov_y,"near":v.cam.near}})).collect::<Vec<_>>(),
                "render":{"output_size":[size.width,size.height],"surface_format":format!("{:?}",g.config.format),"adapter":g.gpu.info.name,"backend":format!("{:?}",g.gpu.info.backend),"hud":self.args.hud,
                    "profile":capture_profile,
                    "sun_direction":render_light.sun_dir.to_array(),"sun_lux":render_light.sun_lux,"sky_lux":render_light.sky_lux,"ev100":render_light.ev100,"sky":self.sky_metadata,
                    "headlights":crate::car_headlights::metadata_for_poses(&headlights,&poses,&headlight_indices),
                    "rear_lamps":crate::car_lamps::metadata(&rear_lamps,&self.built.car_slots),
                    "exhaust":self.exhaust.metadata(),"wheel_effects":self.wheel_effects.metadata(),"procedural_grass":self.built.procedural_grass,
                    "lighting_focus":headlight_indices.iter().filter_map(|i|poses.get(*i).map(|p|p.pos.to_array())).collect::<Vec<_>>(),
                    "legacy_light_lookahead":std::env::var("KERN_LEGACY_LIGHT_LOOKAHEAD").is_ok_and(|v|v=="1"),
                    "compact_canopy_support":std::env::var("KERN_CANOPY_COMPACT_SUPPORT").is_ok_and(|v|v=="1"),
                    "sun_receiver_plane":std::env::var("KERN_SUN_RECEIVER_PLANE").is_ok_and(|v|v=="1"),
                    "sun_scene_depth":std::env::var("KERN_SUN_SCENE_DEPTH").is_ok_and(|v|v=="1"),
                    "sun_terrain_error":std::env::var("KERN_SUN_TERRAIN_ERROR").is_ok_and(|v|v=="1"),
                    "sun_surface_source":std::env::var("KERN_SUN_SURFACE_SOURCE").is_ok_and(|v|v=="1"),
                    "rock_variation":std::env::var("KERN_ROCK_VARIATION").is_ok_and(|v|v=="1"),
                    "adaptive_cliff":std::env::var("SIDEWAYS_ADAPTIVE_CLIFF").is_ok_and(|v|v=="1"),
                    "taa":render_light.taa,"lod_pixels":render_light.lod_pixels,"gi":render_light.gi,"contact_occlusion":render_light.contact_occlusion,
                    "reflection_occlusion":render_light.reflection_occlusion,"near_shadows":render_light.near_shadows,"debug":render_light.debug},
            });
            metadata["render"]["engine_debug"]=self.debug.metadata();
            metadata["render"]["adaptive_quality"]=self.quality.as_ref().map_or(serde_json::Value::Null,|q|q.metadata());
            metadata["render"]["local_lights"]=local_lights_metadata(&local_lights,vehicle_local_light_count);
            metadata["render"]["render_lighting"]=lighting_metadata(&render_light);
            metadata["render"]["tree_source"]=self.built.tree_source.clone();
            metadata["render"]["camera_sun_occlusion"]=serde_json::json!(std::env::var("KERN_CAMERA_SUN_OCCLUSION").as_deref()!=Ok("0"));
            metadata["render"]["meadow"]=meadow;
            metadata["render"]["far_sun_shadow"]=serde_json::json!(std::env::var("KERN_FAR_SUN_SHADOW").as_deref()==Ok("1"));
            metadata["render"]["world_terrain_visibility"]=serde_json::json!(std::env::var("KERN_WORLD_TERRAIN_VISIBILITY").as_deref()==Ok("1"));
            metadata["render"]["terrain_node_budget_setting"]=serde_json::json!(std::env::var("KERN_TERRAIN_NODE_BUDGET").ok());
            metadata["render"]["far_sun_lod_setting"]=serde_json::json!(std::env::var("KERN_FAR_SUN_LOD").ok());
            if let Some(s)=capture_stats.as_ref() {
                metadata["render"]["profile"]["foliage_sun"]=foliage_sun_metadata(s);
                metadata["render"]["profile"]["static_sun"]=static_sun_metadata(s);
                metadata["render"]["profile"]["far_sun"]=serde_json::json!({"cache_updated":s.far_sun_cache_updated,
                    "half_extent_m":s.far_sun_extent_m,"texel_m":s.far_sun_texel_m,"clusters":s.far_sun_clusters,
                    "triangles":s.far_sun_triangles,"submitted_vertices":s.far_sun_submitted_vertices});
            }
            let saved=g.presenter.capture_rgba(&g.gpu,&image,g.config.format,g.hud.as_ref().filter(|_|draw_list.is_some()))
                .and_then(|rgba|crate::screenshot::save(self.window.as_ref().unwrap(),size.width,size.height,&rgba,metadata));
            let message=match saved{Ok(message)=>message,Err(error)=>{eprintln!("screenshot failed: {error}");format!("Screenshot failed: {error}")}};
            self.window.as_ref().unwrap().set_title(&format!("Sideways | {message}"));
            self.screenshot_toast=Some((message,Instant::now()+Duration::from_secs(4)));
        }
        frame_profile::section(trace,"unattributed");
        if self.last_title.elapsed() > Duration::from_millis(500) {
            self.last_title = Instant::now();
            let player = &self.race.players[0];
            let body = &self.race.phys.world.bodies[player.car.body];
            let speed = body.linvel().length() * 3.6;
            self.window.as_ref().unwrap().set_title(&format!("Sideways | {:.0} km/h | gear {} | {} {:.1}s{} | RT/W gas, LT/S brake, A/Space handbrake, X/R reset, Start/Esc pause, N next stage, F12 screenshot{}", speed, player.car.gear, self.race.phase, self.race.clock, if self.paused { " PAUSED" } else { "" }, self.screenshot_toast.as_ref().map(|(text,_)|format!(" | {text}")).unwrap_or_default()));
        }
        self.trace(false);
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }
        self.startup_timing.mark("event_loop_wait");
        let result = (|| -> Result<(), String> {
            // Do not expose the unpainted Win32 surface while device, HUD
            // and presentation pipelines are being initialized.
            let attrs = Window::default_attributes().with_title("Sideways | loading stage").with_inner_size(PhysicalSize::new(self.args.size.0,self.args.size.1)).with_visible(false).with_fullscreen(if self.args.fullscreen && self.args.smoke_frames == 0 && self.host.is_none() { Some(Fullscreen::Borderless(None)) } else { None });
            let window = Arc::new(event_loop.create_window(attrs).map_err(|e|e.to_string())?);
            let instance = render::gpu::instance();
            let surface = instance.create_surface(window.clone()).map_err(|e|e.to_string())?;
            let size = window.inner_size();
            self.startup_timing.mark("window_surface_creation");
            let (gpu, config) = Gpu::for_surface(&instance,&surface,size.width,size.height)?;
            surface.configure(&gpu.device,&config);
            self.startup_timing.mark("device_surface_setup");
            let presenter = render::present::Presenter::new(&gpu,config.format);
            // The loading screen also exists with the gameplay HUD disabled.
            let hud=render::hud::HudRenderer::new(&gpu,config.format)?;
            let hud_profile=if self.args.hud_profile.is_some()&&self.args.hud {render::hud_profile::HudProfiler::new(&gpu,self.args.smoke_frames as u32)}else{None};
            let background=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("startup background"),
                size:wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,
                dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,
                usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
            gpu.queue.write_texture(background.as_image_copy(),&[8,12,18,255],wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(4),rows_per_image:Some(1)},background.size());
            self.startup_timing.mark("loading_presenter_hud_setup");
            let build=stage_loading::Startup::new(std::mem::take(&mut self.built.scene),self.light.clone(),self.args.upscale);
            println!("play GPU: {}, {}x{}, {} views, hidden={}, 120 Hz / {} substeps",gpu.info.name,size.width,size.height,self.args.views,self.args.smoke_frames>0,sideways::car::SUBSTEPS);
            self.startup=Some(StartupGpu {instance,surface,config,gpu,presenter,hud,hud_profile,background,build});
            self.window=Some(window.clone()); self.last_frame=Instant::now(); self.started=Instant::now();
            // Measurement runs keep a fixed image; play holds the display's rate.
            let measured=self.scenario.is_some()||self.args.smoke_frames>0||self.gpu_profile.is_some()||self.frame_profile.is_some();
            let refresh=window.current_monitor().and_then(|m|m.refresh_rate_millihertz()).map(|mhz|f64::from(mhz)/1000.0);
            self.quality=if measured {None} else {crate::adaptive_quality::AdaptiveQuality::new(refresh,Instant::now())};
            if let Some(q)=&self.quality {println!("play: adaptive quality, {}",q.label());}
            if self.args.smoke_frames==0 && self.host.is_none(){window.set_visible(true);window.request_redraw();}
            Ok(())
        })();
        if let Err(e)=result { self.error=Some(e); self.finish(event_loop,"initialization_error"); }
    }

    fn window_event(&mut self,event_loop:&ActiveEventLoop,_id:WindowId,event:WindowEvent) {
        if self.tree_loading.is_some() && matches!(&event,WindowEvent::KeyboardInput{..}|WindowEvent::CursorMoved{..}|WindowEvent::MouseInput{..}|WindowEvent::MouseWheel{..}) {
            self.debug.cancel_interaction();self.input.clear_keys();return;
        }
        match event {
            WindowEvent::CloseRequested=>self.finish(event_loop,"window_closed"),
            WindowEvent::Focused(focused)=>{self.focused=focused;self.last_frame=Instant::now();if !focused{self.debug.cancel_interaction();self.input.clear_keys();self.input_edges=[(0,false);4];self.accumulator=0.0;}else if let Some(host)=&self.host{let _=host.request_start();}}
            WindowEvent::KeyboardInput{event,..}=>if let PhysicalKey::Code(code)=event.physical_key {
                let pressed=event.state==ElementState::Pressed;
                let key=match code {KeyCode::F3=>Some(DebugKey::Toggle),KeyCode::Escape=>Some(DebugKey::Close),
                    KeyCode::ArrowUp=>Some(DebugKey::Up),KeyCode::ArrowDown=>Some(DebugKey::Down),
                    KeyCode::ArrowLeft=>Some(DebugKey::Left),KeyCode::ArrowRight=>Some(DebugKey::Right),
                    KeyCode::Enter=>Some(DebugKey::Enter),KeyCode::PageUp=>Some(DebugKey::PageUp),KeyCode::PageDown=>Some(DebugKey::PageDown),
                    KeyCode::KeyW if self.paused=>Some(DebugKey::Up),KeyCode::KeyS if self.paused=>Some(DebugKey::Down),
                    KeyCode::KeyA if self.paused=>Some(DebugKey::Left),KeyCode::KeyD if self.paused=>Some(DebugKey::Right),
                    KeyCode::Space if self.paused=>Some(DebugKey::Enter),_=>None};
                if key.is_some() && (code==KeyCode::F3 || self.debug.ui.open) {
                    let before=self.debug.ui.open;
                    if pressed && !(code==KeyCode::F3 && event.repeat) {self.debug.key(key.unwrap(),[self.args.size.0,self.args.size.1]);}
                    self.sync_debug_focus(before);
                    self.input.key(code,false);
                } else {self.input.key(code,pressed);}
            },
            WindowEvent::CursorMoved{position,..}=>self.debug.pointer([position.x as f32,position.y as f32]),
            WindowEvent::MouseInput{state,button:MouseButton::Left,..}=>{
                let before=self.debug.ui.open;
                if state==ElementState::Pressed {self.debug.click([self.args.size.0,self.args.size.1]);}else{self.debug.release([self.args.size.0,self.args.size.1]);}
                self.sync_debug_focus(before);
            },
            WindowEvent::MouseWheel{delta,..}=>{let lines=match delta {MouseScrollDelta::LineDelta(_,y)=>-y,MouseScrollDelta::PixelDelta(p)=>-p.y as f32/40.0};self.debug.scroll(lines,[self.args.size.0,self.args.size.1]);},
            WindowEvent::Resized(size)=>{self.debug.cancel_interaction();if size.width>0&&size.height>0{self.args.size=(size.width,size.height);if let Some(g)=&mut self.graphics{g.config.width=size.width;g.config.height=size.height;g.surface.configure(&g.gpu.device,&g.config);g.renderer.reset_history();}}},
            WindowEvent::RedrawRequested=>if !self.invisible() && !event_loop.exiting(){
                if let Err(e)=self.frame(event_loop){eprintln!("play: {e}");self.error=Some(e);self.finish(event_loop,"frame_error");}
            },
            _=>{}
        }
    }

    fn about_to_wait(&mut self,event_loop:&ActiveEventLoop) {
        if event_loop.exiting(){return;}
        // Check outside RedrawRequested: hidden Win32 windows may never emit it.
        if self.smoke_expired(){self.finish(event_loop,"smoke_timeout");return;}
        if self.window.is_none(){return;}
        if self.host.is_some() && self.args.smoke_frames==0 {
            let fullscreen=self.resume_fullscreen.is_some();
            if let Some(window)=&self.window {self.manners.step(window,self.focused,fullscreen);}
        }
        if self.invisible() {
            if let Err(e)=self.frame(event_loop){eprintln!("play: {e}");self.error=Some(e);self.finish(event_loop,"frame_error");}
            let idle=self.host.as_ref().is_some_and(|h|!h.state.is_running());
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now()+Duration::from_millis(if self.loading.is_some()||self.startup.is_some()||self.tree_loading.is_some(){16}else if idle{8}else{1})));
        } else {
            if self.loading.is_some()||self.startup.is_some()||self.tree_loading.is_some() {
                let next=self.last_frame+Duration::from_millis(16);
                event_loop.set_control_flow(ControlFlow::WaitUntil(next));
                if Instant::now()<next{return;}
            } else {event_loop.set_control_flow(ControlFlow::Poll);}
            self.window.as_ref().unwrap().request_redraw();
        }
    }
}

fn smoke(a:&Args)->Result<(),String>{
    let mut race=race(a);let initial=poses(&race).iter().map(|p|p.pos.to_array()).collect::<Vec<_>>();
    race.clock=0.0;race.phase="racing";
    for tick in 0..a.smoke_ticks{
        let control=Control{throttle:if tick<a.smoke_ticks*3/4{0.8}else{0.0},brake:if tick>=a.smoke_ticks*3/4{1.0}else{0.0},steer:if tick>120&&tick<240{0.1}else{0.0},..Default::default()};
        race.step_with_inputs(&mut Timings::default(),&vec![Some(control);a.views]);
    }
    let final_positions=poses(&race).iter().map(|p|p.pos.to_array()).collect::<Vec<_>>();
    let state=serde_json::json!({"mode":"manual_input_smoke","ticks":race.frame,"physics_hz":120,"car_substeps":sideways::car::SUBSTEPS,"initial_positions":initial,"positions":final_positions,"views":a.views});
    if let Some(path)=&a.state_out{std::fs::write(path,serde_json::to_vec_pretty(&state).unwrap()).map_err(|e|e.to_string())?;}
    println!("{state}");Ok(())
}

pub fn run(raw:&[String])->i32{
    let mut startup_timing=cli_startup::Timeline::new();
    if raw.iter().any(|a|a=="--list-controllers"){return match Input::list(){Ok(_)=>0,Err(e)=>{eprintln!("controllers: {e}");1}};}
    if let Some(i)=raw.iter().position(|a|a=="--probe-controllers") {
        let seconds=match raw.get(i+1).and_then(|v|v.parse::<u64>().ok()).filter(|s|(1..=60).contains(s)) {
            Some(s)=>s,None=>{eprintln!("controller probe duration must be 1..=60 seconds");return 2;}
        };
        let out=raw.iter().position(|a|a=="--state-out").and_then(|i|raw.get(i+1)).map(String::as_str);
        return match Input::probe(seconds,out){Ok(())=>0,Err(e)=>{eprintln!("controller probe: {e}");1}};
    }
    let scenario_config=(||->Result<Option<scenario::Scenario>,String>{
        let Some(index)=raw.iter().position(|v|v=="--scenario") else {
            if raw.iter().any(|v|v=="--scenario-output"){return Err("--scenario-output requires --scenario".into());}return Ok(None);
        };
        let path=raw.get(index+1).filter(|v|!v.starts_with("--")).ok_or("--scenario requires a TOML path")?;
        let mut scenario=scenario::Scenario::load(Path::new(path))?;
        if let Some(index)=raw.iter().position(|v|v=="--scenario-output") {
            scenario.capture_dir=Some(raw.get(index+1).filter(|v|!v.starts_with("--")).ok_or("--scenario-output requires a directory")?.into());
        }
        Ok(Some(scenario))
    })();
    let scenario_config=match scenario_config {Ok(value)=>value,Err(error)=>{eprintln!("play: {error}");return 2;}};
    let effective=match scenario_config.as_ref().map(|s|s.arguments(raw)).transpose(){Ok(args)=>args.unwrap_or_else(||raw.to_vec()),Err(error)=>{eprintln!("play: {error}");return 2;}};
    let a=match args(&effective){Ok(a)=>a,Err(e)=>{eprintln!("play: {e}");return 2;}};
    let mut debug=match crate::engine_debug::EngineDebug::new(a.debug_ui,a.debug_ui_script.as_deref()) {Ok(ui)=>ui,Err(error)=>{eprintln!("debug UI / saved baseline: {error}");return 2;}};
    let render_diagnostic=match render::geometry_diagnostics::Config::from_environment() {
        Ok(config)=>render_diagnostic::config(&config),Err(error)=>{eprintln!("play: {error}");return 2;}
    };
    let gpu_profile=match a.gpu_profile.as_ref().map(|path|OpenOptions::new().write(true).create_new(true).open(path).map(std::io::BufWriter::new)).transpose() {
        Ok(file)=>file,Err(e)=>{eprintln!("GPU profile requires a new writable file: {e}");return 2;}
    };
    let frame_profile=match a.frame_profile.as_ref().map(|path|frame_profile::Profiler::new(path,serde_json::json!({
        "seed":a.seed,"views":a.views,"output_size":[a.size.0,a.size.1],"upscale":a.upscale.as_str(),
        "render_diagnostic":render_diagnostic,"render_state_hash_contract":"FNV1a64 of view rectangles/camera float bits, all Lighting fields and car pose/model state; same executable only, non-cryptographic replay consistency check.",
        "synchronous_gpu_profile":a.gpu_profile.is_some(),"hidden_smoke":a.smoke_frames>0,"scenario":scenario_config.as_ref().map(|s|s.metadata()),
        "cpu_section_contract":"Sequential checkpoints; repeated named sections are accumulated without nesting.",
        "gpu_scope":"Four-slot delayed timestamp ring. Only completed samples are recorded; no GPU polling wait is added. Final pending samples may be missing."}))).transpose() {
        Ok(profile)=>profile,Err(error)=>{eprintln!("play: {error}");return 2;}
    };
    if a.smoke_ticks>0{return match smoke(&a){Ok(())=>0,Err(e)=>{eprintln!("play smoke: {e}");1}};}
    startup_timing.mark("arguments_and_profile_setup");
    let mut race=race(&a);
    startup_timing.mark("race_stage_physics");
    if let Some(s)=&scenario_config {if let Err(error)=scenario::advance_to(&mut race,s.sections[0].start_tick,|_|{}){eprintln!("scenario: {error}");return 2;}}
    startup_timing.mark("scenario_initial_fastforward");
    let poses=poses(&race);let mut camera_rest=Default::default();
    let cams=cameras(&race,&poses,&[0.0;4],&[false;4],&mut camera_rest);
    let cams=scenario_config.as_ref().map(|s|s.cameras(0,&cams)).unwrap_or(cams);
    let looks=match crate::car_look::Looks::load(None,vec![]){Ok(l)=>l,Err(e)=>{eprintln!("play: {e}");return 2;}};
    startup_timing.mark("initial_camera_and_materials");
    let mut built=match visuals::build_live(&race.stage,&poses,&cams.iter().take(a.views).map(|c|c.pos).collect::<Vec<_>>(),&looks){Ok(b)=>b,Err(e)=>{eprintln!("play scene: {e}");return 1;}};
    let tree_settings=debug.applied_tree_settings();
    let tree_baseline=match crate::tree_tuning::Baseline::prepare_optional_stage(&mut built.scene,&mut race.phys,tree_settings) {
        Ok(baseline)=>baseline,Err(error)=>{eprintln!("play tree baseline: {error}");return 1;}
    };
    debug.tree_committed(tree_settings,crate::tree_tuning::Baseline::stage_metadata(tree_baseline.as_ref(),tree_settings));
    startup_timing.mark("initial_scene_cpu");
    for line in &built.log { eprintln!("scene initial: {line}"); }
    let mut light=lighting(race.weather);
    let mut sky_metadata=match crate::procedural_sky::apply_environment(&mut built.scene,&mut light){Ok(metadata)=>metadata,Err(e)=>{eprintln!("play sky: {e}");return 1;}};
    startup_timing.mark("environment_sky");
    let mut scenario=match scenario_config.map(scenario::Runtime::prepare).transpose(){Ok(value)=>value,Err(error)=>{eprintln!("scenario: {error}");return 2;}};
    if let Some(s)=&mut scenario {if let Some(prepared)=&s.skies[0] {prepared.apply_scene(&mut built.scene,&mut light);sky_metadata=prepared.metadata.clone();}s.applied_section=Some(0);}
    startup_timing.mark("scenario_sky_presets");
    let now=Instant::now();
    let host=match Client::connect_from_env(){Ok(h)=>h,Err(e)=>{eprintln!("GameNight: {e}");return 1;}};
    if scenario.is_some()&&host.is_some(){eprintln!("scenario runs are standalone; remove GameNight host environment");return 2;}
    if a.smoke_next_stage && host.is_some(){eprintln!("--smoke-next-stage is standalone validation only");return 2;}
    let seats:Vec<_>=(0..a.views).map(|i|Seat{index:i,player_id:format!("standalone-{i}"),name:format!("Player {}",i+1),color:String::new(),controller:String::new(),ai:false,couch:Some(i as i64)}).collect();
    let sky_controls=crate::procedural_sky::LiveSky::new(&sky_metadata,&light);
    let settings=serde_json::json!({"gearbox":"automatic","stage_km":a.km,"rivals":0,"contact":true,"weather":a.weather,"upscale":a.upscale.as_str(),"sky_time":sky_controls.desired().time_key(),"sky_weather":"clear"});
    let view_order=(0..a.views).collect();
    let resume_fullscreen=if a.fullscreen{Some(Fullscreen::Borderless(None))}else{None};
    let ambient_clock=AmbientClock::new(a.smoke_frames>0||scenario.is_some());
    let mut exhaust=crate::exhaust_fx::ExhaustFx::new(a.seed);exhaust.step(&race);
    let mut wheel_effects=crate::wheel_effects::WheelEffects::new(a.seed);wheel_effects.step(&race);
    let start_paused=a.start_paused;
    let mut app=App{debug,upscale_error:None,gpu_profile,frame_profile,scenario,startup_timing,requested_output_size:[a.size.0,a.size.1],args:a,race,built,light,exhaust,wheel_effects,sky_metadata,sky_controls,pause_menu:Default::default(),input:Input::new(),window:None,graphics:None,startup:None,last_startup:serde_json::Value::Null,focused:true,paused:start_paused,last_frame:now,started:now,last_title:now,last_trace:now,accumulator:0.0,ambient_clock,rendered:0,presented:0,initial_positions:poses.iter().map(|p|p.pos.to_array()).collect(),controls:vec![],yaw:[0.0;4],far:[false;4],orbit_active:[false;4],camera_rest,error:None,exit_reason:String::new(),host,seats,view_order,pending_settings:settings.clone(),active_settings:settings,input_edges:[(0,false);4],look:[[0.0;2];4],finished_round:0,resume_fullscreen,manners:Default::default(),hud_stats:serde_json::Value::Null,hud_samples:vec![],screenshot_pending:false,screenshot_toast:None,live_edit:crate::car_look::LiveEdit::new(looks),sound:None,sound_failed:false,loading:None,tree_baseline,tree_loading:None,tree_loading_settings:tree_settings,last_tree_load:serde_json::Value::Null,last_stage_load:serde_json::Value::Null,loading_error:None,cached_picture:None,outputs:[None,None],deferred_host:vec![],smoke_stage_requested:false,quality:None};
    app.startup_timing.mark("host_input_application_setup");
    let events=match EventLoop::new(){Ok(e)=>e,Err(e)=>{eprintln!("play event loop: {e}");return 1;}};
    app.startup_timing.mark("event_loop_creation");
    app.startup_timing.emit("pre_window",serde_json::json!({"requested_output_size":app.requested_output_size,"seed":app.args.seed,"views":app.args.views,"scenario":app.scenario.as_ref().map(|s|s.config.name.as_str())}));
    if let Err(e)=events.run_app(&mut app){eprintln!("play event loop: {e}");return 1;}
    app.drain_frame_gpu(0);
    if let Some(profile)=&mut app.frame_profile {if let Err(error)=profile.finish(){eprintln!("play: {error}");return 1;}}
    app.startup_timing.emit("run_end",serde_json::json!({"rendered_frames":app.rendered,"presented_frames":app.presented,"exit_reason":app.exit_reason,"error":app.error}));
    if let Some(e)=app.error{eprintln!("play: {e}");1}else{0}
}

#[cfg(test)]
mod local_light_tests {
    use super::*;
    #[test]
    fn rear_spill_follows_effective_pedal_and_release_without_history() {
        assert_eq!(rear_light_state(1,false,false,0.6,0.0).brake,0.6);
        assert_eq!(rear_light_state(-1,false,false,1.0,0.0).brake,0.0);
        assert_eq!(rear_light_state(-1,false,false,0.0,0.7).brake,0.7);
        assert_eq!(rear_light_state(-1,true,false,0.4,1.0).brake,0.4);
        assert_eq!(rear_light_state(1,false,true,0.0,0.0).brake,1.0);
        assert_eq!(rear_light_state(1,false,false,f64::NAN,0.0).brake,0.0);
        assert!(!rear_light_state(1,false,false,0.0,0.0).reverse);
        assert!(rear_light_state(-1,false,false,0.0,0.0).reverse);
        let empty=local_lights_metadata(&[],0);
        assert_eq!(empty["enabled"],false);assert_eq!(empty["vehicle_count"],0);assert_eq!(empty["exhaust_count"],0);
        assert_eq!(empty["lights"].as_array().unwrap().len(),0);
    }
}

#[cfg(test)]
mod frame_profile_args_tests {
    use super::*;
    fn parse(values:&[&str])->Result<Args,String>{args(&values.iter().map(|v|(*v).to_owned()).collect::<Vec<_>>())}
    #[test] fn frame_profile_requires_a_path_and_an_actual_frame_loop() {
        assert!(parse(&["--frame-profile"]).is_err());
        assert!(parse(&["--frame-profile","--autoplay"]).is_err());
        assert!(parse(&["--frame-profile"," "]).is_err());
        assert!(parse(&["--frame-profile","frames.jsonl","--smoke-ticks","10"]).is_err());
        let value=parse(&["--frame-profile","frames.jsonl"]).unwrap();
        assert_eq!(value.frame_profile.as_deref(),Some("frames.jsonl"));assert!(value.gpu_profile.is_none());
        assert!(parse(&["--frame-profile","frames.jsonl","--gpu-profile","sync.jsonl"]).is_ok());
    }
}

#[cfg(test)]
mod ambient_clock_tests {
    use super::*;

    #[test]
    fn visible_foliage_moves_while_unfocused_car_ticks_stay_frozen() {
        let nature = crate::nature::Nature::default();
        let mut clock = AmbientClock::new(false);
        let root = Vec3::new(12.0, 3.0, -19.0);
        let before = clock.wind(nature).sample_shear(root, true);
        // This is the visible focus-loss path: the frame loop keeps drawing,
        // but the independent car simulation stays on exactly the same tick.
        for _ in 0..60 { clock.advance(1.0 / 60.0, 480, true, true, false); }
        let wind = clock.wind(nature);
        assert!((wind.time_seconds - 1.0).abs() < 1e-12);
        assert!((wind.sample_shear(root, true) - before).length() > 1e-5);
        let metadata = clock.metadata(nature);
        assert_eq!(metadata["wind"]["time_seconds"].as_f64(), Some(wind.time_seconds));
        assert_eq!(metadata["wind"]["time_source"], "visible_animation_clock");
    }

    #[test]
    fn pause_hidden_and_host_idle_freeze_without_resume_catchup() {
        let mut clock = AmbientClock::new(false);
        clock.advance(0.125, 15, true, true, false);
        // Local pause, a hidden window, and hosted idle each freeze foliage.
        for (visible, running, paused) in [(true, true, true), (false, true, false), (true, false, false)] {
            for _ in 0..120 { clock.advance(0.25, 15, visible, running, paused); }
            assert_eq!(clock.seconds, 0.125);
        }
        clock.advance(0.125, 30, true, true, false);
        assert_eq!(clock.seconds, 0.25);
    }

    #[test]
    fn hidden_smoke_phase_is_tick_deterministic_despite_wall_pacing() {
        let mut fast = AmbientClock::new(true);
        let mut slow = AmbientClock::new(true);
        for frame in [8, 16, 16, 24, 120] {
            fast.advance(0.001, frame, false, true, frame == 16);
            slow.advance(0.2, frame, false, true, frame == 16);
            assert_eq!(fast.seconds, frame as f64 / 120.0);
            assert_eq!(fast.seconds, slow.seconds);
        }
        let metadata = fast.metadata(crate::nature::Nature::default());
        assert_eq!(metadata["wind"]["time_source"], "simulation_ticks_120hz");
        assert_eq!(metadata["wind"]["time_seconds"], 1.0);
    }

    #[test]
    fn fixed_phase_and_capture_metadata_match_the_actual_wind_state() {
        let nature = crate::nature::Nature { wind_time: Some(13.25), ..Default::default() };
        for smoke in [false, true] {
            let mut clock = AmbientClock::new(smoke);
            for frame in [0, 8, 60, 360] {
                clock.advance(0.2, frame, true, true, false);
                let wind = clock.wind(nature);
                assert_eq!(wind.time_seconds, 13.25);
                let metadata = clock.metadata(nature);
                assert_eq!(metadata["wind"]["time_seconds"].as_f64(), Some(wind.time_seconds));
                assert_eq!(metadata["wind"]["time_source"], "fixed");
            }
        }
    }
}
