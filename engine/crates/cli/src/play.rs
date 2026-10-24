//! Opt-in interactive application. `shot` and benchmarks remain headless.
//! `play --smoke-render N` creates an invisible window for bounded validation.

#[path="stage_loading.rs"]
mod stage_loading;

use crate::{input::{Input, SeatInput}, visuals::{self, Built, CarPose}};
use glam::{Quat, Vec3};
use kern::Timings;
use render::{Camera, Gpu, Lighting, Renderer};
use sideways::race::{Control, Race, Settings};
use crate::gamenight::{Client, Command, Seat};
use std::{fs::OpenOptions, io::Write, path::Path, sync::Arc, time::{Duration, Instant}};
use winit::{application::ApplicationHandler, dpi::PhysicalSize, event::{ElementState, WindowEvent}, event_loop::{ActiveEventLoop, ControlFlow, EventLoop}, keyboard::{KeyCode, PhysicalKey}, window::{Fullscreen, Window, WindowId}};

#[derive(Clone)]
struct Args {
    upscale: render::upscale::UpscaleMode,
    nature: crate::nature::Nature,
    gpu_profile: Option<String>, gpu_warmup: usize,
    seed: u64, km: f64, views: usize, size: (u32, u32), weather: String,
    smoke_next_stage: bool,
    smoke_frames: u64, smoke_ticks: u64, capture: Option<String>, state_out: Option<String>, trace: Option<String>, fullscreen: bool,
    hud: bool, hud_profile: Option<String>, hud_warmup: usize,
    autoplay: bool,
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
    let hud_profile=value("--hud-profile");
    let hud_warmup=parse("--hud-warmup","8")? as usize;
    if hud_profile.is_some() && (smoke_frames==0 || smoke_frames>4096 || hud_warmup>=smoke_frames as usize) {
        return Err("HUD profiling needs --smoke-render 1..4096 and fewer warmup frames than rendered frames".into());
    }
    let hud=match opt("--hud","on").as_str(){"on"=>true,"off"=>false,_=>return Err("hud must be on or off".into())};
    Ok(Args { upscale:crate::upscale::parse(args)?,nature:crate::nature::Nature::parse(args)?,gpu_profile,gpu_warmup,seed: parse("--seed", "1")?, km, views, size: (size[0], size[1]), weather: opt("--weather", "clear"), smoke_next_stage, smoke_frames, smoke_ticks: parse("--smoke-ticks", "0")?, capture: value("--capture"), state_out: value("--state-out"), trace: value("--state-trace"), fullscreen: args.iter().any(|a| a == "--fullscreen"), hud,hud_profile,hud_warmup,autoplay:args.iter().any(|a|a=="--autoplay"),
        // Hidden smoke runs stay silent unless asked.
        sound: crate::audio_out::Mode::parse(&opt("--sound", if smoke_frames > 0 { "off" } else { "fluid" }))? })
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

struct App {
    upscale_error: Option<String>,
    gpu_profile: Option<std::io::BufWriter<std::fs::File>>,
    args: Args, race: Race, built: Built, light: Lighting, input: Input,
    sky_metadata: serde_json::Value,
    sky_controls: crate::procedural_sky::LiveSky,
    pause_menu: crate::procedural_sky::PauseMenu,
    window: Option<Arc<Window>>, graphics: Option<LiveGpu>, focused: bool, paused: bool,
    last_frame: Instant, started: Instant, last_title: Instant, last_trace: Instant, accumulator: f64,
    ambient_clock: AmbientClock,
    rendered: u64, presented: u64, initial_positions: Vec<[f32; 3]>, controls: Vec<Control>,
    yaw: [f32; 4], far: [bool; 4], error: Option<String>, exit_reason: String,
    orbit_active: [bool; 4], camera_rest: [crate::chase_camera::RestAnchor; 4],
    host: Option<Client>, seats: Vec<Seat>, view_order: Vec<usize>, pending_settings: serde_json::Value,
    active_settings: serde_json::Value, input_edges: [(i8,bool);4], look: [[f32;2];4], finished_round: u32,
    resume_fullscreen: Option<Fullscreen>,
    hud_stats: serde_json::Value, hud_samples: Vec<serde_json::Value>,
    screenshot_pending: bool, screenshot_toast: Option<(String, Instant)>,
    sound: Option<crate::audio_out::Sound>, sound_failed: bool,
    exhaust: crate::exhaust_fx::ExhaustFx,
    wheel_effects: crate::wheel_effects::WheelEffects,
    /// Car material data and shading shaders, reloaded when edited.
    live_edit: crate::car_look::LiveEdit,
    loading: Option<stage_loading::Loading>,
    last_stage_load: serde_json::Value,
    loading_error: Option<String>,
    cached_picture: Option<wgpu::Texture>,
    deferred_host: Vec<Command>,
    smoke_stage_requested: bool,
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
        self.args.smoke_frames > 0 || self.window.as_ref().is_some_and(|w|w.is_visible()==Some(false))
    }

    fn smoke_expired(&self) -> bool {
        self.args.smoke_frames > 0 && self.started.elapsed() > Duration::from_secs(90)
    }

    fn managed_screen(&mut self, visible: bool) {
        let Some(window)=&self.window else{return;};
        if visible {
            // Hidden acceptance tests must never take over the desktop.
            if self.args.smoke_frames==0 {
                window.set_fullscreen(self.resume_fullscreen.clone());
                window.set_visible(true);
            }
        } else {
            if window.is_visible()==Some(true){self.resume_fullscreen=window.fullscreen();}
            window.set_fullscreen(None);
            window.set_visible(false);
        }
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
        snapshot["stage_loading"]=self.loading.as_ref().map(|l|l.metadata()).unwrap_or(serde_json::Value::Null);
        snapshot["last_stage_load"]=self.last_stage_load.clone();
        snapshot["stage_loading_error"]=self.loading_error.clone().into();
        snapshot["crowds"]=crate::roadside_crowds::metadata(&self.built.scene);
        snapshot["exhaust"]=self.exhaust.metadata();
        snapshot["wheel_effects"]=self.wheel_effects.metadata();
        snapshot["procedural_grass"]=self.built.procedural_grass.clone().unwrap_or(serde_json::Value::Null);
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
        if self.loading.is_none() {if let Err(error)=self.write_hud_profile(){self.error=Some(error);}}
        if let Some(loading)=&self.loading {loading.cancel();}
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
        }
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
                self.sky_controls.accepted(prepared);self.trace(true);
            }
            Err(error)=>{eprintln!("live sky unchanged: {error}");self.sky_controls.failed(error);}
        }
    }

    fn standalone_menu(&mut self,now:Instant)->Result<(),String> {
        if !self.paused || self.host.is_some(){return Ok(());}
        let (axis,confirm)=self.input.menu_state(self.focused);
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

    fn reset_round_input(&mut self) {
        // A new round creates new cameras, as in Godot. Seat order can change
        // here; no new player may inherit another player's camera/input state.
        self.yaw=[0.0;4];self.far=[false;4];self.look=[[0.0;2];4];
        self.orbit_active=[false;4];self.camera_rest=Default::default();
        self.input_edges=[(0,false);4];self.controls.clear();self.accumulator=0.0;
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
            looks:self.live_edit.looks().clone(),sky:self.sky_controls.applied_snapshot(),upscale:self.args.upscale};
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
                    self.exhaust=crate::exhaust_fx::ExhaustFx::new(self.args.seed);self.exhaust.step(&self.race);
                    self.wheel_effects=crate::wheel_effects::WheelEffects::new(self.args.seed);self.wheel_effects.step(&self.race);
                    self.graphics.as_mut().ok_or("GPU missing at stage commit")?.renderer=loaded.renderer;
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
        let commands=if self.loading.is_none() && !self.deferred_host.is_empty(){
            std::mem::take(&mut self.deferred_host)
        } else if self.deferred_host.iter().any(|c|matches!(c,Command::Prepare{..})) {
            vec![]
        } else {self.host.as_mut().map(|h|h.poll()).unwrap_or_default()};
        for command in commands {
            if self.loading.is_some() && matches!(&command,Command::Prepare{..}|Command::Setting{..}|Command::Roster{..}|Command::PlayersUpdated{..}) {
                if matches!(&command,Command::Prepare{..}){self.loading.as_ref().unwrap().cancel();}
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

    fn frame(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if !self.poll_host(event_loop)?{return Ok(());}
        if self.loading.is_some(){return self.loading_frame();}
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f64().min(0.25);
        self.last_frame = now;
        self.input.poll();
        let mut inputs: Vec<SeatInput> = (0..self.args.views).map(|i|self.input.seat(i, self.focused)).collect();
        if let Some(host)=&mut self.host {
            for (i,input) in inputs.iter_mut().enumerate(){
                let seat=self.seats.get(i);
                let remote=seat.filter(|s|!s.ai&&!s.controller.is_empty()).map(|s|host.state.controls_for_seat(s.index,now,true)).unwrap_or_default();
                *input=SeatInput{control:remote.control,look:[remote.look[0] as f32,remote.look[1] as f32],camera_pressed:remote.camera_pressed,pause_pressed:remote.start_pressed};
            }
        }
        if inputs.iter().any(|i|i.pause_pressed) {
            self.paused = !self.paused;if let Some(host)=&mut self.host{host.state.suppress_buttons();if self.paused{host.request_overlay()?;}}self.trace(true);
            if self.paused && self.host.is_none(){let (axis,confirm)=self.input.menu_state(self.focused);self.pause_menu.opened(axis,confirm);}
        }
        self.standalone_menu(now)?;
        if self.loading.is_some(){return self.loading_frame();}
        self.poll_sky_update(now);
        if self.input.take_key(KeyCode::KeyN) && self.host.is_none() { self.next_stage()?;return self.loading_frame(); }
        if self.input.take_key(KeyCode::KeyM) && self.host.is_none() {
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
        if self.args.smoke_frames > 0 && self.host.is_none() {
            self.race.phase = "racing"; self.race.clock = self.race.clock.max(0.0);
            for i in &mut inputs { i.control = Control { throttle: 0.75, steer: 0.04, ..Default::default() }; }
        }
        let running=self.host.as_ref().map(|h|h.state.is_running()).unwrap_or(true);
        if running && self.focused && !self.invisible() {
            if let Some(host)=&mut self.host {host.report_activity()?;}
        }
        if self.paused || !running {self.input_edges=[(0,false);4];for input in &mut inputs{*input=SeatInput::default();}}
        let mut history_cut=false;
        for (i, input) in inputs.iter().enumerate() {
            self.look[i]=input.look;
            if self.paused || !running { continue; }
            if input.control.shift!=0{self.input_edges[i].0=input.control.shift;}
            self.input_edges[i].1|=input.control.recover;
            if input.camera_pressed { self.far[i] = !self.far[i]; history_cut=true; }
            self.yaw[i] = crate::chase_camera::orbit(self.yaw[i], input.look, dt as f32, &mut self.orbit_active[i]);
        }
        self.controls = inputs.iter().map(|i|i.control).collect();
        let old_round = self.race.round;
        let mut rebuilt=false;
        let mut simulated_ticks=0u32;
        let autoplay=self.args.autoplay&&self.host.is_none();
        if !self.paused && running && (self.focused || autoplay || self.args.smoke_frames > 0 || self.host.is_some()) {
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
        if self.loading.is_some(){return self.loading_frame();}
        if self.race.round != old_round && !rebuilt { self.rebuild()?; }
        if self.race.phase=="results"&&self.finished_round!=self.race.round {
            self.finished_round=self.race.round;if let Some(host)=&self.host{host.notify_finished(&host.state.session)?;}
        }
        // Keep the host socket, neutral input and trace alive while the host
        // owns the screen. A warm/paused managed game must not consume a full
        // render frame or continue presenting behind the GameNight lobby.
        if !running {if let Some(s)=&self.sound{s.pause(true);}self.trace(false);return Ok(());}
        let graphics_started=Instant::now();
        if self.rendered==0 { println!("play: first frame entered (invisible={})", self.invisible()); }
        let poses = poses(&self.race);
        self.built.update_cars(&poses);
        let cams = cameras(&self.race, &poses, &self.yaw, &self.far, &mut self.camera_rest);
        let invisible = self.invisible();
        self.update_sound(&cams, dt as f32, invisible);
        let size = self.window.as_ref().unwrap().inner_size();
        if size.width == 0 || size.height == 0 { return Ok(()); }
        self.ambient_clock.advance(dt, self.race.frame, !invisible, running, self.paused);
        let ordered:Vec<_>=self.view_order.iter().filter_map(|i|cams.get(*i).copied()).collect();
        let views = crate::shot::view_rects(if ordered.is_empty(){&cams}else{&ordered}, self.args.views, size.width, size.height);
        // The fixed pool sits before the cars. World-cell changes upload only
        // its dirty range; unchanged camera anchors require no grass upload.
        let grass_dirty=self.built.update_ground_cover(&views.iter().map(|v|v.cam.pos).collect::<Vec<_>>());
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
        let layout_ms=layout_started.elapsed().as_secs_f64()*1000.0;
        let g = self.graphics.as_mut().ok_or("GPU not initialized")?;
        if draw_list.is_some() && g.hud.is_none() {g.hud=Some(render::hud::HudRenderer::new(&g.gpu,g.config.format)?);}
        let prepare_started=Instant::now();
        if let (Some(hud),Some(list))=(&mut g.hud,&draw_list){
            let stats=hud.prepare(&g.gpu,list)?;
            self.hud_stats=serde_json::json!({"output_size":list.size,"text_runs":stats.text_runs,"glyphs":stats.glyphs,"vertices":stats.vertices,"reshaped":stats.reshaped});
        }
        let prepare_ms=prepare_started.elapsed().as_secs_f64()*1000.0;
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
        let headlights=crate::car_headlights::from_poses(&poses,&headlight_indices,&self.light);
        g.renderer.set_headlights(&headlights)?;
        g.renderer.set_exhaust(&g.gpu,&self.exhaust.sprites())?;
        self.wheel_effects.upload(&g.gpu,&mut g.renderer)?;
        g.renderer.set_lighting_focus(&headlight_indices.iter().filter_map(|i|poses.get(*i).map(|p|p.pos)).collect::<Vec<_>>())?;
        let rear_lamps=crate::car_lamps::from_race(&self.race,&self.light,poses.len());
        if crate::car_lamps::apply(&mut self.built.scene.car_materials,&self.built.car_slots,&rear_lamps)? {
            g.renderer.set_car_materials(&g.gpu,&self.built.scene.car_materials)?;
        }
        // Hidden standalone smoke advances a deterministic number of physics
        // ticks per rendered frame. Interactive and hosted play use wall time.
        let frame_time_ms=if self.args.smoke_frames>0 && self.host.is_none() {
            simulated_ticks as f32*(1000.0/120.0)
        } else {(dt*1000.0) as f32};
        g.renderer.set_frame_time_ms(frame_time_ms);
        if history_cut {g.renderer.reset_history();}
        // F12 and the final hidden smoke capture measure this same frame;
        // normal play retains the no-readback render path.
        let measure_profile=self.gpu_profile.is_some() && self.rendered>=self.args.gpu_warmup as u64;
        let stage_smoke_done=self.args.smoke_next_stage && self.last_stage_load["outcome"]=="ready";
        let measure_capture=measure_profile || self.screenshot_pending || (self.args.smoke_frames>0 && (stage_smoke_done || self.rendered+1>=self.args.smoke_frames) && self.args.capture.is_some());
        let (image,capture_stats)=if measure_capture {
            let (image,stats)=g.renderer.try_render_texture_profiled(&g.gpu,&views,&self.light,size.width,size.height)?;
            (image,Some(stats))
        } else {(g.renderer.try_render_texture(&g.gpu, &views, &self.light, size.width, size.height)?,None)};
        self.cached_picture=Some(image.clone());
        self.rendered += 1;
        if measure_profile {
            let stats=capture_stats.as_ref().expect("profiled frame has stats");
            let sample=serde_json::json!({"kind":"frame","rendered_frame":self.rendered,"race_frame":self.race.frame,"race_clock":self.race.clock,
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
                "canopy_updated_levels":stats.canopy_updated_levels,"canopy_pitch_m":stats.canopy_pitch_m});
            writeln!(self.gpu_profile.as_mut().unwrap(),"{sample}").map_err(|e|format!("GPU profile: {e}"))?;
        }
        let presented_before=self.presented;
        let mut hidden_target=None;
        if self.rendered==1 { println!("play: first scene frame submitted"); }
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
            g.presenter.present(&g.gpu,&image,&target.create_view(&Default::default()),g.hud.as_ref().filter(|_|draw_list.is_some()),g.hud_profile.as_mut())?;
            hidden_target=Some(target);
        } else { match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
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
        if invisible {g.gpu.wait();}
        if self.args.hud_profile.is_some(){self.hud_samples.push(serde_json::json!({"frame":self.rendered,"race_frame":self.race.frame,"hud_layout_cpu_ms":layout_ms,"hud_prepare_cpu_ms":prepare_ms,"graphics_wall_ms":graphics_started.elapsed().as_secs_f64()*1000.0,"stats":self.hud_stats}));}
        if self.rendered==1 { println!("play: first frame complete (presented={})",self.presented); }
        if presented_before==0 && self.presented>0 { println!("play: first visible frame complete"); }
        if self.last_stage_load["outcome"]=="ready" && self.last_stage_load.get("first_new_stage_frame_at_app_ms").is_none() && (invisible||self.presented>presented_before) {
            self.last_stage_load["first_new_stage_frame_at_app_ms"]=(self.started.elapsed().as_secs_f64()*1000.0).into();
            self.last_stage_load["first_new_stage_rendered_frame"]=self.rendered.into();
            eprintln!("next stage: first new-stage frame complete, round {}",self.race.round);
        }
        if self.args.smoke_next_stage && !self.smoke_stage_requested && (invisible||self.presented>presented_before) {
            self.smoke_stage_requested=true;self.next_stage()?;return Ok(());
        }
        if self.args.smoke_frames > 0 && (if self.args.smoke_next_stage {stage_smoke_done}else{self.rendered>=self.args.smoke_frames}) {
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
                    let metrics=serde_json::json!({"schema":1,"capture":path,"output_size":[size.width,size.height],"views":views.len(),"upscale":crate::upscale::metadata(self.args.upscale,&stats.upscale),"gpu_pass_ms":stats.gpu_ms,"timings_available":!stats.gpu_ms.is_empty(),"scope":"same-frame scene passes; excludes HUD, presentation and CPU; not gameplay FPS","triangles":stats.triangles,"clusters":stats.clusters,"submitted_vertices":stats.submitted_vertices,"shadow_submitted_vertices":stats.shadow_submitted_vertices,"early_grass_rejects":stats.early_grass_rejects,"cluster_buckets":stats.cluster_buckets,"early_grass_cull":stats.early_grass_cull,"nature":self.ambient_clock.metadata(self.args.nature),"grass_detail_profile":self.built.scene.grass_detail.label(),"ground_cover_streaming":grass_stream_metadata(&self.built),"canopy_updated_levels":stats.canopy_updated_levels,"canopy_pitch_m":stats.canopy_pitch_m});
                    std::fs::write(format!("{path}.metrics.json"),serde_json::to_vec_pretty(&metrics).unwrap()).map_err(|e|format!("capture metrics: {e}"))?;
                }
            }
            g.gpu.wait();
            self.finish(event_loop, "smoke_frame_limit");
            return Ok(());
        }
        if self.screenshot_pending && self.presented>presented_before {
            self.screenshot_pending=false;
            let g=self.graphics.as_ref().unwrap();
            let metadata=serde_json::json!({
                "schema":1,"seed":self.args.seed,"stage_noise_seed":self.race.stage.noise_seed,"stage_detail_seed":self.race.stage.detail_seed,"weather":self.race.weather,"state":self.snapshot(),
                "player_poses":poses.iter().enumerate().map(|(i,p)|serde_json::json!({"index":i,"position":p.pos.to_array(),"rotation_xyzw":p.rot.to_array(),"model":p.model})).collect::<Vec<_>>(),
                "views":views.iter().map(|v|serde_json::json!({"rect":[v.x,v.y,v.w,v.h],"camera":{"position":v.cam.pos.to_array(),"target":v.cam.target.to_array(),"fov_y":v.cam.fov_y,"near":v.cam.near}})).collect::<Vec<_>>(),
                "render":{"output_size":[size.width,size.height],"surface_format":format!("{:?}",g.config.format),"adapter":g.gpu.info.name,"backend":format!("{:?}",g.gpu.info.backend),"hud":self.args.hud,
                    "profile":capture_stats.as_ref().map(|s|serde_json::json!({"upscale":crate::upscale::metadata(self.args.upscale,&s.upscale),"gpu_pass_ms":s.gpu_ms,"timings_available":!s.gpu_ms.is_empty(),"triangles":s.triangles,"clusters":s.clusters,"submitted_vertices":s.submitted_vertices,"shadow_submitted_vertices":s.shadow_submitted_vertices,"early_grass_rejects":s.early_grass_rejects,"cluster_buckets":s.cluster_buckets,"early_grass_cull":s.early_grass_cull,"canopy_updated_levels":s.canopy_updated_levels,"canopy_pitch_m":s.canopy_pitch_m,"scope":"same-frame scene passes; excludes HUD, presentation and CPU; not gameplay FPS"})),
                    "sun_direction":self.light.sun_dir.to_array(),"sun_lux":self.light.sun_lux,"sky_lux":self.light.sky_lux,"ev100":self.light.ev100,"sky":self.sky_metadata,
                    "headlights":crate::car_headlights::metadata_for_poses(&headlights,&poses,&headlight_indices),
                    "rear_lamps":crate::car_lamps::metadata(&rear_lamps,&self.built.car_slots),
                    "exhaust":self.exhaust.metadata(),"wheel_effects":self.wheel_effects.metadata(),"procedural_grass":self.built.procedural_grass,
                    "lighting_focus":headlight_indices.iter().filter_map(|i|poses.get(*i).map(|p|p.pos.to_array())).collect::<Vec<_>>(),
                    "legacy_light_lookahead":std::env::var("KERN_LEGACY_LIGHT_LOOKAHEAD").is_ok_and(|v|v=="1"),
                    "compact_canopy_support":std::env::var("KERN_CANOPY_COMPACT_SUPPORT").is_ok_and(|v|v=="1"),
                    "taa":self.light.taa,"lod_pixels":self.light.lod_pixels,"gi":self.light.gi,"contact_occlusion":self.light.contact_occlusion,
                    "reflection_occlusion":self.light.reflection_occlusion,"near_shadows":self.light.near_shadows,"debug":self.light.debug},
            });
            let saved=g.presenter.capture_rgba(&g.gpu,&image,g.config.format,g.hud.as_ref().filter(|_|draw_list.is_some()))
                .and_then(|rgba|crate::screenshot::save(self.window.as_ref().unwrap(),size.width,size.height,&rgba,metadata));
            let message=match saved{Ok(message)=>message,Err(error)=>{eprintln!("screenshot failed: {error}");format!("Screenshot failed: {error}")}};
            self.window.as_ref().unwrap().set_title(&format!("Sideways | {message}"));
            self.screenshot_toast=Some((message,Instant::now()+Duration::from_secs(4)));
        }
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
        let result = (|| -> Result<(), String> {
            let attrs = Window::default_attributes().with_title("Sideways | loading stage").with_inner_size(PhysicalSize::new(self.args.size.0,self.args.size.1)).with_visible(self.args.smoke_frames == 0 && self.host.is_none()).with_fullscreen(if self.args.fullscreen && self.args.smoke_frames == 0 && self.host.is_none() { Some(Fullscreen::Borderless(None)) } else { None });
            let window = Arc::new(event_loop.create_window(attrs).map_err(|e|e.to_string())?);
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
            let surface = instance.create_surface(window.clone()).map_err(|e|e.to_string())?;
            let size = window.inner_size();
            let (gpu, config) = Gpu::for_surface(&instance,&surface,size.width,size.height)?;
            surface.configure(&gpu.device,&config);
            let presenter = render::present::Presenter::new(&gpu,config.format);
            let hud=if self.args.hud {Some(render::hud::HudRenderer::new(&gpu,config.format)?)}else{None};
            let hud_profile=if self.args.hud_profile.is_some()&&self.args.hud {render::hud_profile::HudProfiler::new(&gpu,self.args.smoke_frames as u32)}else{None};
            let renderer = Renderer::new(&gpu,&self.built.scene,&self.light);
            renderer.set_upscale(&gpu,self.args.upscale)?;
            if let Some(file)=&mut self.gpu_profile {
                let header=serde_json::json!({"kind":"header","schema":1,"adapter":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),
                    "seed":self.args.seed,"km":self.args.km,"views":self.args.views,"output_size":[size.width,size.height],"warmup_frames":self.args.gpu_warmup,
                    "upscale":crate::upscale::metadata(self.args.upscale,&renderer.upscale_status()),
                    "nature":self.ambient_clock.metadata(self.args.nature),"autoplay":self.args.autoplay&&self.host.is_none(),"sky":self.sky_metadata,
                    "grass_detail_profile":self.built.scene.grass_detail.label(),"ground_cover_streaming":grass_stream_metadata(&self.built),
                    "taa":self.light.taa,"lod_pixels":self.light.lod_pixels,"far_grass":self.built.scene.far_grass.is_some(),"foliage_constructor_flags":foliage_variant_flags(),
                    "voxel_forest":self.built.scene.voxel_forest.is_some(),"initial_instances":self.built.scene.instances.len(),
                    "scope":"Synchronous same-frame GPU pass timestamps; excludes HUD, presentation, simulation and uninstrumented work. Profiling changes frame pacing; not gameplay FPS."});
                writeln!(file,"{header}").and_then(|_|file.flush()).map_err(|e|format!("GPU profile header: {e}"))?;
            }
            println!("play GPU: {}, {}x{}, {} views, hidden={}, 120 Hz / {} substeps",gpu.info.name,size.width,size.height,self.args.views,self.args.smoke_frames>0,sideways::car::SUBSTEPS);
            self.graphics=Some(LiveGpu{instance,surface,config,gpu,renderer,presenter,hud,hud_profile});
            self.window=Some(window); self.last_frame=Instant::now(); self.started=Instant::now();
            Ok(())
        })();
        if let Err(e)=result { self.error=Some(e); self.finish(event_loop,"initialization_error"); }
    }

    fn window_event(&mut self,event_loop:&ActiveEventLoop,_id:WindowId,event:WindowEvent) {
        match event {
            WindowEvent::CloseRequested=>self.finish(event_loop,"window_closed"),
            WindowEvent::Focused(focused)=>{self.focused=focused;self.last_frame=Instant::now();if !focused{self.input.clear_keys();self.input_edges=[(0,false);4];self.accumulator=0.0;}else if let Some(host)=&self.host{let _=host.request_start();}}
            WindowEvent::KeyboardInput{event,..}=>if let PhysicalKey::Code(code)=event.physical_key{self.input.key(code,event.state==ElementState::Pressed);},
            WindowEvent::Resized(size)=>if size.width>0&&size.height>0{if let Some(g)=&mut self.graphics{g.config.width=size.width;g.config.height=size.height;g.surface.configure(&g.gpu.device,&g.config);g.renderer.reset_history();self.args.size=(size.width,size.height);}},
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
        if self.invisible() {
            if let Err(e)=self.frame(event_loop){eprintln!("play: {e}");self.error=Some(e);self.finish(event_loop,"frame_error");}
            let idle=self.host.as_ref().is_some_and(|h|!h.state.is_running());
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now()+Duration::from_millis(if self.loading.is_some(){16}else if idle{8}else{1})));
        } else {
            if self.loading.is_some() {
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
    if raw.iter().any(|a|a=="--list-controllers"){return match Input::list(){Ok(_)=>0,Err(e)=>{eprintln!("controllers: {e}");1}};}
    if let Some(i)=raw.iter().position(|a|a=="--probe-controllers") {
        let seconds=match raw.get(i+1).and_then(|v|v.parse::<u64>().ok()).filter(|s|(1..=60).contains(s)) {
            Some(s)=>s,None=>{eprintln!("controller probe duration must be 1..=60 seconds");return 2;}
        };
        let out=raw.iter().position(|a|a=="--state-out").and_then(|i|raw.get(i+1)).map(String::as_str);
        return match Input::probe(seconds,out){Ok(())=>0,Err(e)=>{eprintln!("controller probe: {e}");1}};
    }
    let a=match args(raw){Ok(a)=>a,Err(e)=>{eprintln!("play: {e}");return 2;}};
    let gpu_profile=match a.gpu_profile.as_ref().map(|path|OpenOptions::new().write(true).create_new(true).open(path).map(std::io::BufWriter::new)).transpose() {
        Ok(file)=>file,Err(e)=>{eprintln!("GPU profile requires a new writable file: {e}");return 2;}
    };
    if a.smoke_ticks>0{return match smoke(&a){Ok(())=>0,Err(e)=>{eprintln!("play smoke: {e}");1}};}
    let race=race(&a);let poses=poses(&race);let mut camera_rest=Default::default();
    let cams=cameras(&race,&poses,&[0.0;4],&[false;4],&mut camera_rest);
    let looks=match crate::car_look::Looks::load(None,vec![]){Ok(l)=>l,Err(e)=>{eprintln!("play: {e}");return 2;}};
    let mut built=match visuals::build_live(&race.stage,&poses,&cams.iter().take(a.views).map(|c|c.pos).collect::<Vec<_>>(),&looks){Ok(b)=>b,Err(e)=>{eprintln!("play scene: {e}");return 1;}};
    for line in &built.log { eprintln!("scene initial: {line}"); }
    let mut light=lighting(race.weather);
    let sky_metadata=match crate::procedural_sky::apply_environment(&mut built.scene,&mut light){Ok(metadata)=>metadata,Err(e)=>{eprintln!("play sky: {e}");return 1;}};
    let now=Instant::now();
    let host=match Client::connect_from_env(){Ok(h)=>h,Err(e)=>{eprintln!("GameNight: {e}");return 1;}};
    if a.smoke_next_stage && host.is_some(){eprintln!("--smoke-next-stage is standalone validation only");return 2;}
    let seats:Vec<_>=(0..a.views).map(|i|Seat{index:i,player_id:format!("standalone-{i}"),name:format!("Player {}",i+1),color:String::new(),controller:String::new(),ai:false,couch:Some(i as i64)}).collect();
    let sky_controls=crate::procedural_sky::LiveSky::new(&sky_metadata,&light);
    let settings=serde_json::json!({"gearbox":"automatic","stage_km":a.km,"rivals":0,"contact":true,"weather":a.weather,"upscale":a.upscale.as_str(),"sky_time":sky_controls.desired().time_key(),"sky_weather":"clear"});
    let view_order=(0..a.views).collect();
    let resume_fullscreen=if a.fullscreen{Some(Fullscreen::Borderless(None))}else{None};
    let ambient_clock=AmbientClock::new(a.smoke_frames>0);
    let mut exhaust=crate::exhaust_fx::ExhaustFx::new(a.seed);exhaust.step(&race);
    let mut wheel_effects=crate::wheel_effects::WheelEffects::new(a.seed);wheel_effects.step(&race);
    let mut app=App{upscale_error:None,gpu_profile,args:a,race,built,light,exhaust,wheel_effects,sky_metadata,sky_controls,pause_menu:Default::default(),input:Input::new(),window:None,graphics:None,focused:true,paused:false,last_frame:now,started:now,last_title:now,last_trace:now,accumulator:0.0,ambient_clock,rendered:0,presented:0,initial_positions:poses.iter().map(|p|p.pos.to_array()).collect(),controls:vec![],yaw:[0.0;4],far:[false;4],orbit_active:[false;4],camera_rest,error:None,exit_reason:String::new(),host,seats,view_order,pending_settings:settings.clone(),active_settings:settings,input_edges:[(0,false);4],look:[[0.0;2];4],finished_round:0,resume_fullscreen,hud_stats:serde_json::Value::Null,hud_samples:vec![],screenshot_pending:false,screenshot_toast:None,live_edit:crate::car_look::LiveEdit::new(looks),sound:None,sound_failed:false,loading:None,last_stage_load:serde_json::Value::Null,loading_error:None,cached_picture:None,deferred_host:vec![],smoke_stage_requested:false};
    let events=match EventLoop::new(){Ok(e)=>e,Err(e)=>{eprintln!("play event loop: {e}");return 1;}};
    if let Err(e)=events.run_app(&mut app){eprintln!("play event loop: {e}");return 1;}
    if let Some(e)=app.error{eprintln!("play: {e}");1}else{0}
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
