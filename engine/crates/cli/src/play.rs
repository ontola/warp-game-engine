//! Opt-in interactive application. `shot` and benchmarks remain headless.
//! `play --smoke-render N` creates an invisible window for bounded validation.

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
    seed: u64, km: f64, views: usize, size: (u32, u32), weather: String,
    smoke_frames: u64, smoke_ticks: u64, capture: Option<String>, state_out: Option<String>, trace: Option<String>, fullscreen: bool,
    hud: bool, hud_profile: Option<String>, hud_warmup: usize,
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
    let hud_profile=value("--hud-profile");
    let hud_warmup=parse("--hud-warmup","8")? as usize;
    if hud_profile.is_some() && (smoke_frames==0 || smoke_frames>4096 || hud_warmup>=smoke_frames as usize) {
        return Err("HUD profiling needs --smoke-render 1..4096 and fewer warmup frames than rendered frames".into());
    }
    let hud=match opt("--hud","on").as_str(){"on"=>true,"off"=>false,_=>return Err("hud must be on or off".into())};
    Ok(Args { seed: parse("--seed", "1")?, km, views, size: (size[0], size[1]), weather: opt("--weather", "clear"), smoke_frames, smoke_ticks: parse("--smoke-ticks", "0")?, capture: value("--capture"), state_out: value("--state-out"), trace: value("--state-trace"), fullscreen: args.iter().any(|a| a == "--fullscreen"), hud,hud_profile,hud_warmup })
}

fn race(a: &Args) -> Race {
    let settings = Settings { stage_km: a.km, bots: a.views, rivals: 0, weather: Some(a.weather.clone()), ..Default::default() };
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

fn cameras(race: &Race, poses: &[CarPose], yaw: &[f32; 4], far: &[bool; 4]) -> Vec<Camera> {
    poses.iter().enumerate().take(4).map(|(i, car)| {
        let fwd = car.rot * Vec3::NEG_Z;
        let heading = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
        let back = Quat::from_rotation_y(yaw[i]) * -heading;
        let distance = if far[i] { 8.0 } else { 5.2 };
        let mut pos = car.pos + back * distance + Vec3::Y * if far[i] { 3.0 } else { 2.05 };
        pos.y = pos.y.max(race.stage.height(pos.x as f64, pos.z as f64) as f32 + 0.7);
        Camera { pos, target: car.pos + Vec3::Y * 0.9 + heading * 1.5, fov_y: 64f32.to_radians(), near: 0.1 }
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
    args: Args, race: Race, built: Built, light: Lighting, input: Input,
    window: Option<Arc<Window>>, graphics: Option<LiveGpu>, focused: bool, paused: bool,
    last_frame: Instant, started: Instant, last_title: Instant, last_trace: Instant, accumulator: f64,
    rendered: u64, presented: u64, initial_positions: Vec<[f32; 3]>, controls: Vec<Control>,
    yaw: [f32; 4], far: [bool; 4], error: Option<String>, exit_reason: String,
    host: Option<Client>, seats: Vec<Seat>, view_order: Vec<usize>, pending_settings: serde_json::Value,
    active_settings: serde_json::Value, input_edges: [(i8,bool);4], look: [[f32;2];4], finished_round: u32,
    resume_fullscreen: Option<Fullscreen>,
    hud_stats: serde_json::Value, hud_samples: Vec<serde_json::Value>,
}

impl App {
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
        let players:Vec<_>=self.race.players.iter().enumerate().map(|(i,p)| {
            let t=p.car.pose(&self.race.phys).translation;
            let c=self.controls.get(i).copied().unwrap_or_default();
            let input=serde_json::json!({"throttle":c.throttle,"brake":c.brake,"steer":c.steer,"handbrake":c.handbrake,"shift":c.shift,"recover":c.recover});
            serde_json::json!({"seat":self.seats.get(i).map(|s|s.index).unwrap_or(i),"player_id":self.seats.get(i).map(|s|&s.player_id),"name":p.name,"position":[t.x,t.y,t.z],"gear":p.car.gear,"manual":p.car.manual,"penalty":p.penalty,"finished":p.finished,"look":self.look.get(i),"camera_yaw":self.yaw.get(i),"camera_mode":self.far.get(i),"input":input})
        }).collect();
        serde_json::json!({
            "schema": 1, "mode": "play", "hidden": self.args.smoke_frames > 0, "race_frame": self.race.frame,
            "race_clock": self.race.clock, "race_round": self.race.round, "race_phase": self.race.phase,
            "paused": self.paused, "focused": self.focused, "rendered_frames": self.rendered, "presented_frames": self.presented,
            "window_visible":self.window.as_ref().and_then(|w|w.is_visible()).unwrap_or(false),
            "render_quiet":self.host.as_ref().is_some_and(|h|!h.state.is_running()),
            "physics_hz": 120, "car_substeps": sideways::car::SUBSTEPS, "taa": false, "ground_cover": false,
            "initial_positions": self.initial_positions, "exit_reason": self.exit_reason, "error": self.error,
            "host": self.host.as_ref().map(|h|h.state.snapshot()), "view_order": self.view_order.iter().map(|i|self.seats[*i].index).collect::<Vec<_>>(),
            "native_controllers":self.input.controllers(),"native_active_samples":self.input.native_active_samples,
            "active_settings": self.active_settings, "pending_settings": self.pending_settings,
            "players":players,
            "view_rects": render::split_rects(self.args.views, self.args.size.0, self.args.size.1),
            "hud": self.args.hud, "hud_stats": self.hud_stats,
        })
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
        if let Err(error)=self.write_hud_profile(){self.error=Some(error);}
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
        let cams = cameras(&self.race, &poses, &self.yaw, &self.far);
        // Frozen CPU grass is intentionally not carried into a moving scene:
        // it would stop at the original camera area. Streaming is separate work.
        self.built = visuals::build(&self.race.stage, &poses, &cams.iter().map(|c|c.pos).collect::<Vec<_>>(), false)?;
        self.light = lighting(self.race.weather);
        if let Some(g) = &mut self.graphics { g.renderer = Renderer::new(&g.gpu, &self.built.scene, &self.light); }
        self.last_frame = Instant::now(); self.accumulator = 0.0;
        Ok(())
    }

    fn profiles(&mut self, current: &[Seat]) {
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
    }

    fn apply_settings(&mut self) {
        self.active_settings=self.pending_settings.clone();
        if let Some(km)=self.active_settings["stage_km"].as_f64(){self.race.settings.stage_km=km.clamp(2.0,8.0);}
        if let Some(v)=self.active_settings["contact"].as_bool(){self.race.settings.contact=v;}
        if let Some(v)=self.active_settings["rivals"].as_u64(){self.race.settings.rivals=(v as usize).min(3);}
        if let Some(v)=self.active_settings["weather"].as_str(){self.race.settings.weather=if v=="random"{None}else{Some(v.into())};}
    }

    fn reset_round_input(&mut self) {
        // A new round creates new cameras, as in Godot. Seat order can change
        // here; no new player may inherit another player's camera/input state.
        self.yaw=[0.0;4];self.far=[false;4];self.look=[[0.0;2];4];
        self.input_edges=[(0,false);4];self.controls.clear();self.accumulator=0.0;
    }

    fn next_stage(&mut self)->Result<(),String>{
        self.apply_settings();
        if let Some(host)=&self.host {
            let seats=host.state.seats();
            if !seats.is_empty(){self.seats=seats;self.args.views=self.seats.len();self.race.settings.bots=self.seats.len();}
        }
        self.race.new_stage(&mut Timings::default());
        let seats=self.seats.clone();self.profiles(&seats);self.reset_round_input();
        self.rebuild()
    }

    fn poll_host(&mut self,event_loop:&ActiveEventLoop)->Result<bool,String>{
        let commands=self.host.as_mut().map(|h|h.poll()).unwrap_or_default();
        for command in commands {
            match command {
                Command::Connected=>{
                    let host=self.host.as_ref().unwrap();host.declare_settings(crate::gamenight::settings())?;host.declare_actions(crate::gamenight::actions())?;
                }
                Command::Prepare{session,seats}=>{
                    self.managed_screen(false);
                    self.seats=seats;self.args.views=self.seats.len().clamp(1,4);self.race.settings.bots=self.args.views;
                    self.apply_settings();
                    self.race=Race::new(self.args.seed,self.race.settings.clone(),sideways::spec::Cars::load(None).map_err(|e|e.to_string())?);
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
        }
        if self.input.take_key(KeyCode::KeyN) && self.host.is_none() { self.next_stage()?; }
        if self.input.take_key(KeyCode::KeyM) && self.host.is_none() {
            self.race.settings.manual = !self.race.settings.manual;
            for p in &mut self.race.players { p.car.manual = self.race.settings.manual; }
        }
        if self.input.take_key(KeyCode::F11) {
            if let Some(w) = &self.window { w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) }); }
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
        for (i, input) in inputs.iter().enumerate() {
            self.look[i]=input.look;
            if self.paused || !running { continue; }
            if input.control.shift!=0{self.input_edges[i].0=input.control.shift;}
            self.input_edges[i].1|=input.control.recover;
            if input.camera_pressed { self.far[i] = !self.far[i]; }
            let target = if input.look[0].hypot(input.look[1]) > 0.2 { -input.look[0].atan2(input.look[1]) } else { 0.0 };
            let delta = (target - self.yaw[i] + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            self.yaw[i] += delta * (1.0 - (-dt as f32 * 12.0).exp());
        }
        self.controls = inputs.iter().map(|i|i.control).collect();
        let old_round = self.race.round;
        let mut rebuilt=false;
        if !self.paused && running && (self.focused || self.args.smoke_frames > 0 || self.host.is_some()) {
            self.accumulator += if self.args.smoke_frames > 0 && self.host.is_none() { 8.0 / 120.0 } else { dt };
            let mut controls: Vec<_> = self.controls.iter().enumerate().map(|(i,c)| {
                if self.host.is_some()&&self.seats.get(i).is_some_and(|s|s.ai){None}else{let mut c=*c;c.shift=self.input_edges[i].0;c.recover=self.input_edges[i].1;Some(c)}
            }).collect();
            while self.accumulator >= 1.0 / 120.0 {
                if self.race.phase=="results" && self.race.results<=1.0/120.0 {
                    self.next_stage()?;rebuilt=true;break;
                }
                self.race.step_with_inputs(&mut Timings::default(), &controls);
                self.input_edges=[(0,false);4];
                for c in controls.iter_mut().flatten() { c.shift = 0; c.recover = false; }
                self.accumulator -= 1.0 / 120.0;
            }
        } else { self.accumulator = 0.0; }
        if self.race.round != old_round && !rebuilt { self.rebuild()?; }
        if self.race.phase=="results"&&self.finished_round!=self.race.round {
            self.finished_round=self.race.round;if let Some(host)=&self.host{host.notify_finished(&host.state.session)?;}
        }
        // Keep the host socket, neutral input and trace alive while the host
        // owns the screen. A warm/paused managed game must not consume a full
        // render frame or continue presenting behind the GameNight lobby.
        if !running {self.trace(false);return Ok(());}
        let graphics_started=Instant::now();
        if self.rendered==0 { println!("play: first frame entered (invisible={})", self.invisible()); }
        let poses = poses(&self.race);
        self.built.update_cars(&poses);
        let cams = cameras(&self.race, &poses, &self.yaw, &self.far);
        let invisible = self.invisible();
        let size = self.window.as_ref().unwrap().inner_size();
        if size.width == 0 || size.height == 0 { return Ok(()); }
        let layout_started=Instant::now();
        let draw_list=if self.args.hud {
            Some(crate::hud::build(&self.race,&self.view_order,&render::split_rects(self.args.views,size.width,size.height).into_iter().map(|(x,y,w,h)|[x,y,w,h]).collect::<Vec<_>>(),
                &self.race.players.iter().map(|p|p.name.clone()).collect::<Vec<_>>(),self.paused,[size.width,size.height]))
        } else {None};
        let layout_ms=layout_started.elapsed().as_secs_f64()*1000.0;
        let g = self.graphics.as_mut().ok_or("GPU not initialized")?;
        let prepare_started=Instant::now();
        if let (Some(hud),Some(list))=(&mut g.hud,&draw_list){
            let stats=hud.prepare(&g.gpu,list)?;
            self.hud_stats=serde_json::json!({"output_size":list.size,"text_runs":stats.text_runs,"glyphs":stats.glyphs,"vertices":stats.vertices,"reshaped":stats.reshaped});
        }
        let prepare_ms=prepare_started.elapsed().as_secs_f64()*1000.0;
        let first = self.built.cars.first().map(|c|c.first).unwrap_or(self.built.scene.instances.len());
        g.renderer.update_instances(&g.gpu, first, &self.built.scene.instances[first..])?;
        let ordered:Vec<_>=self.view_order.iter().filter_map(|i|cams.get(*i).copied()).collect();
        let views = crate::shot::view_rects(if ordered.is_empty(){&cams}else{&ordered}, self.args.views, size.width, size.height);
        let image = g.renderer.render_texture(&g.gpu, &views, &self.light, size.width, size.height);
        self.rendered += 1;
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
            g.presenter.present(&g.gpu,&image,&target.create_view(&Default::default()),g.hud.as_ref(),g.hud_profile.as_mut())?;
            hidden_target=Some(target);
        } else { match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                g.presenter.present(&g.gpu, &image, &frame.texture.create_view(&Default::default()),g.hud.as_ref(),None)?;
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
        if self.args.smoke_frames > 0 && self.rendered >= self.args.smoke_frames {
            if let Some(path) = &self.args.capture {
                let target=hidden_target.as_ref().ok_or("capture requires the final hidden presentation texture")?;
                if !matches!(target.format(),wgpu::TextureFormat::Rgba8Unorm|wgpu::TextureFormat::Rgba8UnormSrgb|wgpu::TextureFormat::Bgra8Unorm|wgpu::TextureFormat::Bgra8UnormSrgb){return Err("capture requires an 8-bit RGBA/BGRA surface".into());}
                render::write_png(Path::new(path), size.width, size.height, &g.gpu.read_rgba8(target))?;
            }
            g.gpu.wait();
            self.finish(event_loop, "smoke_frame_limit");
            return Ok(());
        }
        if self.last_title.elapsed() > Duration::from_millis(500) {
            self.last_title = Instant::now();
            let player = &self.race.players[0];
            let body = &self.race.phys.world.bodies[player.car.body];
            let speed = body.linvel().length() * 3.6;
            self.window.as_ref().unwrap().set_title(&format!("Sideways | {:.0} km/h | gear {} | {} {:.1}s{} | RT/W gas, LT/S brake, A/Space handbrake, X/R reset, Start/Esc pause, N next stage", speed, player.car.gear, self.race.phase, self.race.clock, if self.paused { " PAUSED" } else { "" }));
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
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now()+Duration::from_millis(if idle{8}else{1})));
        } else {
            event_loop.set_control_flow(ControlFlow::Poll);
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
    if a.smoke_ticks>0{return match smoke(&a){Ok(())=>0,Err(e)=>{eprintln!("play smoke: {e}");1}};}
    let race=race(&a);let poses=poses(&race);let cams=cameras(&race,&poses,&[0.0;4],&[false;4]);
    let built=match visuals::build(&race.stage,&poses,&cams.iter().map(|c|c.pos).collect::<Vec<_>>(),false){Ok(b)=>b,Err(e)=>{eprintln!("play scene: {e}");return 1;}};
    let light=lighting(race.weather);let now=Instant::now();
    let host=match Client::connect_from_env(){Ok(h)=>h,Err(e)=>{eprintln!("GameNight: {e}");return 1;}};
    let seats:Vec<_>=(0..a.views).map(|i|Seat{index:i,player_id:format!("standalone-{i}"),name:format!("Player {}",i+1),color:String::new(),controller:String::new(),ai:false,couch:Some(i as i64)}).collect();
    let settings=serde_json::json!({"gearbox":"automatic","stage_km":a.km,"rivals":0,"contact":true,"weather":a.weather});
    let view_order=(0..a.views).collect();
    let resume_fullscreen=if a.fullscreen{Some(Fullscreen::Borderless(None))}else{None};
    let mut app=App{args:a,race,built,light,input:Input::new(),window:None,graphics:None,focused:true,paused:false,last_frame:now,started:now,last_title:now,last_trace:now,accumulator:0.0,rendered:0,presented:0,initial_positions:poses.iter().map(|p|p.pos.to_array()).collect(),controls:vec![],yaw:[0.0;4],far:[false;4],error:None,exit_reason:String::new(),host,seats,view_order,pending_settings:settings.clone(),active_settings:settings,input_edges:[(0,false);4],look:[[0.0;2];4],finished_round:0,resume_fullscreen,hud_stats:serde_json::Value::Null,hud_samples:vec![]};
    let events=match EventLoop::new(){Ok(e)=>e,Err(e)=>{eprintln!("play event loop: {e}");return 1;}};
    if let Err(e)=events.run_app(&mut app){eprintln!("play event loop: {e}");return 1;}
    if let Some(e)=app.error{eprintln!("play: {e}");1}else{0}
}
