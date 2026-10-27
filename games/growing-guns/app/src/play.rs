//! The windowed game: fixed 60 Hz simulation, 1 to 4 local players in split
//! screen (keyboard and mouse plus gamepads), the rest bots.
use crate::session::Session;
use crate::{hud, view, Args};
use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};
use glam::Vec3;
use growing_guns::{Event, Input, World, DT};
use render::{Gpu, Lighting, Renderer, ViewRect};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

const MOUSE_SENS: f32 = 0.0022;
const PAD_LOOK: f32 = 3.0;
const PAD_CURVE: f32 = 1.5;
const PAD_DEADZONE: f32 = 0.18;

/// Hit markers, damage flash and gun bob from this tick's events.
pub fn feedback(fb: &mut [hud::Feedback], world: &World, dt: f32) {
    for (i, f) in fb.iter_mut().enumerate() {
        f.hit = (f.hit - dt).max(0.0);
        f.hurt = (f.hurt - dt).max(0.0);
        if let Some(p) = world.players.get(i) {
            let speed = (p.vel.x * p.vel.x + p.vel.z * p.vel.z).sqrt();
            f.bob += speed * dt * 0.75;
        }
    }
    for e in &world.events {
        if let Event::Hit { victim, attacker, head, .. } = *e {
            if let Some(f) = fb.get_mut(attacker) { f.hit = 0.25; f.headshot = head; }
            if let Some(f) = fb.get_mut(victim) { f.hurt = 0.35; }
        }
    }
}

/// Edge-latched buttons: pressed since the last tick consumed them.
#[derive(Default)]
struct Latch { jump: bool, dash: bool, reload: bool }

#[derive(Default)]
struct Kbm { keys: HashSet<KeyCode>, fire: bool, look: [f32; 2], latch: Latch }

struct Pad { id: GamepadId, latch: Latch }

struct Gfx {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    renderer: Renderer,
    presenter: render::present::Presenter,
    hud: render::hud::HudRenderer,
}

struct App {
    fullscreen: bool,
    kbm_enabled: bool,
    humans: usize,
    world: World,
    prev: Vec<Vec3>,
    vis: view::Visuals,
    light: Lighting,
    fb: Vec<hud::Feedback>,
    gfx: Option<Gfx>,
    window: Option<Arc<Window>>,
    gilrs: Option<Gilrs>,
    kbm: Kbm,
    pads: Vec<Pad>,
    grabbed: bool,
    acc: f32,
    last: Instant,
    error: Option<String>,
    /// Smoke test: exit after this many frames and report the frame rate.
    max_frames: u64,
    frames: u64,
    started: Instant,
    session: Session,
    since_snap: f32,
    /// World player index for each local view.
    slots: Vec<usize>,
}

pub fn run(args: &Args) -> Result<(), String> {
    let total = args.num("--total", 4usize).clamp(1, 8);
    let humans = args.num("--players", 1usize).clamp(1, 4).min(total);
    let seed = args.num("--seed", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs()));
    let lan = args.flag("--lan");
    let (session, world, slots) = if let Some(ticket) = args.value("--join") {
        let (session, world, slots) = Session::join(ticket, humans as u8, lan)?;
        println!("joined: playing as {:?}", slots.iter().map(|s| s + 1).collect::<Vec<_>>());
        (session, world, slots)
    } else {
        let bots: Vec<bool> = (0..total).map(|i| i >= humans).collect();
        let session = if args.flag("--host") { Session::host(lan)? } else { Session::Local };
        if let Some(t) = session.ticket() {
            println!("hosting. Others join with:\n  growing-guns play --join {t}{}", if lan { " --lan" } else { "" });
            let _ = std::fs::write("growing-guns-ticket.txt", t);
        }
        (session, World::new(seed, &bots), (0..humans).collect())
    };
    let total = world.players.len();
    let vis = view::Visuals::build(&world);
    let gilrs = Gilrs::new().map_err(|e| eprintln!("gamepads unavailable: {e}")).ok();
    let mut app = App {
        fullscreen: args.flag("--fullscreen"), kbm_enabled: !args.flag("--no-kbm"), humans,
        prev: world.players.iter().map(|p| p.pos).collect(), world, vis, light: view::lighting(),
        fb: vec![hud::Feedback::default(); total], gfx: None, window: None, gilrs, kbm: Kbm::default(),
        pads: Vec::new(), grabbed: false, acc: 0.0, last: Instant::now(), error: None,
        max_frames: args.num("--frames", 0u64), frames: 0, started: Instant::now(), session, slots, since_snap: 0.0,
    };
    if let Some(g) = &app.gilrs { app.pads = g.gamepads().map(|(id, _)| Pad { id, latch: Latch::default() }).collect(); }
    println!("growing guns: seed {seed}, {humans} local players, {} bots, {} gamepads", total - humans, app.pads.len());
    let events = EventLoop::new().map_err(|e| e.to_string())?;
    events.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}

fn stick(v: f32) -> f32 {
    let a = v.abs();
    if a < PAD_DEADZONE { 0.0 } else { v.signum() * ((a - PAD_DEADZONE) / (1.0 - PAD_DEADZONE)).powf(PAD_CURVE) }
}

impl App {
    /// Devices in order (keyboard first, then pads) drive local players in
    /// order. With one local player every device drives player 0.
    fn inputs(&mut self) -> Vec<Input> {
        let mut devices = Vec::new();
        if self.kbm_enabled {
            let k = &mut self.kbm;
            let key = |c| k.keys.contains(&c) as i32 as f32;
            devices.push(Input {
                mv: [key(KeyCode::KeyD) - key(KeyCode::KeyA), key(KeyCode::KeyW) - key(KeyCode::KeyS)],
                look: [-k.look[0] * MOUSE_SENS, -k.look[1] * MOUSE_SENS],
                jump: std::mem::take(&mut k.latch.jump),
                dash: std::mem::take(&mut k.latch.dash),
                reload: std::mem::take(&mut k.latch.reload),
                fire: k.fire,
            });
            k.look = [0.0; 2];
        }
        if let Some(g) = &self.gilrs {
            for pad in &mut self.pads {
                let gp = g.gamepad(pad.id);
                if !gp.is_connected() { continue; }
                let (lx, ly) = (stick(gp.value(Axis::LeftStickX)), stick(gp.value(Axis::LeftStickY)));
                let (rx, ry) = (stick(gp.value(Axis::RightStickX)), stick(gp.value(Axis::RightStickY)));
                let fire = gp.is_pressed(Button::RightTrigger2) || gp.button_data(Button::RightTrigger2).is_some_and(|b| b.value() > 0.4);
                devices.push(Input {
                    mv: [lx, ly],
                    look: [-rx * PAD_LOOK * DT, ry * PAD_LOOK * DT],
                    jump: std::mem::take(&mut pad.latch.jump),
                    dash: std::mem::take(&mut pad.latch.dash),
                    reload: std::mem::take(&mut pad.latch.reload),
                    fire,
                });
            }
        }
        let mut out = vec![Input::default(); self.humans];
        for (k, d) in devices.into_iter().enumerate() {
            let slot = if self.humans == 1 { 0 } else { k };
            if slot >= self.humans { break; }
            let o = &mut out[slot];
            o.mv = [(o.mv[0] + d.mv[0]).clamp(-1.0, 1.0), (o.mv[1] + d.mv[1]).clamp(-1.0, 1.0)];
            o.look = [o.look[0] + d.look[0], o.look[1] + d.look[1]];
            o.jump |= d.jump;
            o.dash |= d.dash;
            o.reload |= d.reload;
            o.fire |= d.fire;
        }
        out
    }

    fn poll_pads(&mut self) {
        let Some(g) = &mut self.gilrs else { return };
        while let Some(ev) = g.next_event() {
            match ev.event {
                EventType::Connected => {
                    if !self.pads.iter().any(|p| p.id == ev.id) { self.pads.push(Pad { id: ev.id, latch: Latch::default() }); }
                }
                EventType::ButtonPressed(b, _) => {
                    if let Some(p) = self.pads.iter_mut().find(|p| p.id == ev.id) {
                        match b {
                            Button::South => p.latch.jump = true,
                            Button::East | Button::RightTrigger | Button::LeftTrigger2 => p.latch.dash = true,
                            Button::West => p.latch.reload = true,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn tick(&mut self) -> Result<(), String> {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        self.poll_pads();
        self.acc += dt;
        let mut steps = 0;
        let mut rebuilt = false;
        let client = self.session.is_client();
        while self.acc >= DT && steps < 6 {
            self.acc -= DT;
            steps += 1;
            let local = self.inputs();
            if client { self.session.send(local); continue; }
            let mut inputs = vec![Input::default(); self.world.players.len()];
            for (k, &s) in self.slots.iter().enumerate() { inputs[s] = local[k]; }
            self.session.poll(&mut self.world);
            self.session.remote_inputs(&mut inputs);
            self.prev = self.world.players.iter().map(|p| p.pos).collect();
            self.world.step(&inputs);
            if self.world.events.contains(&Event::NewArena) {
                self.vis = view::Visuals::build(&self.world);
                self.prev = self.world.players.iter().map(|p| p.pos).collect();
                rebuilt = true;
            }
            self.vis.absorb(&self.world);
            feedback(&mut self.fb, &self.world, DT);
            self.session.publish(&self.world);
        }
        if client {
            let before: Vec<Vec3> = self.world.players.iter().map(|p| p.pos).collect();
            let (vis, fb) = (&mut self.vis, &mut self.fb);
            let (snaps, arena) = self.session.receive(&mut self.world, &self.slots, |w| { vis.absorb(w); feedback(fb, w, DT); });
            if snaps > 0 { self.prev = before; self.since_snap = 0.0; } else { self.since_snap += dt; }
            if arena { self.vis = view::Visuals::build(&self.world); self.prev = self.world.players.iter().map(|p| p.pos).collect(); rebuilt = true; }
        }
        let alpha = if client { (self.since_snap / DT).clamp(0.0, 1.0) } else { (self.acc / DT).clamp(0.0, 1.0) };
        self.vis.pose(&self.world, &self.prev, alpha, dt);
        let Some(g) = &mut self.gfx else { return Ok(()) };
        if rebuilt { g.renderer = Renderer::new(&g.gpu, &self.vis.scene, &self.light); }
        g.renderer.update_instances(&g.gpu, self.vis.first_dynamic, self.vis.dynamic())?;
        g.renderer.set_local_lights(&self.vis.local_lights())?;
        let (w, h) = (g.config.width, g.config.height);
        let rects = render::split_rects(self.humans, w, h);
        let views: Vec<ViewRect> = rects.iter().enumerate()
            .map(|(i, &(x, y, vw, vh))| ViewRect { cam: view::camera(&self.world, self.slots[i], &self.prev, alpha), x, y, w: vw, h: vh }).collect();
        let picture = g.renderer.try_render_texture(&g.gpu, &views, &self.light, w, h)?;
        let mut list = render::hud::HudDrawList::new([w, h]);
        for (i, &(x, y, vw, vh)) in rects.iter().enumerate() {
            let s = self.slots[i];
            hud::view(&mut list, &self.world, s, [x as f32, y as f32, vw as f32, vh as f32], &self.fb[s]);
        }
        let mut status = self.session.status(&self.slots).unwrap_or_default();
        if !self.grabbed && self.kbm_enabled { status = format!("click to capture the mouse, Esc releases   {status}"); }
        hud::global(&mut list, &self.world, (!status.is_empty()).then_some(status.as_str()));
        g.hud.prepare(&g.gpu, &list)?;
        match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                g.presenter.present(&g.gpu, &picture, &frame.texture.create_view(&Default::default()), Some(&g.hud), None)?;
                if let Some(win) = &self.window { win.pre_present_notify(); }
                g.gpu.queue.present(frame);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => g.surface.configure(&g.gpu.device, &g.config),
            _ => {}
        }
        Ok(())
    }

    fn grab(&mut self, on: bool) {
        let Some(w) = &self.window else { return };
        if on {
            let ok = w.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined)).is_ok();
            w.set_cursor_visible(!ok);
            self.grabbed = ok;
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
            self.grabbed = false;
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() { return; }
        let result = (|| -> Result<(), String> {
            let attrs = Window::default_attributes().with_title("Growing Guns | Warp").with_inner_size(PhysicalSize::new(1600, 900))
                .with_fullscreen(self.fullscreen.then_some(Fullscreen::Borderless(None)));
            let window = Arc::new(event_loop.create_window(attrs).map_err(|e| e.to_string())?);
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
            let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
            let size = window.inner_size();
            let (gpu, config) = Gpu::for_surface(&instance, &surface, size.width, size.height)?;
            surface.configure(&gpu.device, &config);
            let presenter = render::present::Presenter::new(&gpu, config.format);
            let hud = render::hud::HudRenderer::new(&gpu, config.format)?;
            let renderer = Renderer::new(&gpu, &self.vis.scene, &self.light);
            println!("GPU {}, {}x{}", gpu.info.name, size.width, size.height);
            self.gfx = Some(Gfx { surface, config, gpu, renderer, presenter, hud });
            self.window = Some(window.clone());
            self.last = Instant::now();
            self.started = Instant::now();
            window.request_redraw();
            Ok(())
        })();
        if let Err(e) = result { self.error = Some(e); event_loop.exit(); }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.grabbed { self.kbm.look[0] += delta.0 as f32; self.kbm.look[1] += delta.1 as f32; }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gfx {
                    g.config.width = size.width.max(1);
                    g.config.height = size.height.max(1);
                    g.surface.configure(&g.gpu.device, &g.config);
                }
            }
            WindowEvent::Focused(false) => self.grab(false),
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    if state == ElementState::Pressed && !self.grabbed && self.kbm_enabled { self.grab(true); return; }
                    self.kbm.fire = state == ElementState::Pressed && self.grabbed;
                }
                if button == MouseButton::Right && state == ElementState::Pressed { self.kbm.latch.dash = true; }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                if event.state == ElementState::Pressed {
                    if !event.repeat {
                        match code {
                            KeyCode::Space => self.kbm.latch.jump = true,
                            KeyCode::ShiftLeft | KeyCode::ShiftRight => self.kbm.latch.dash = true,
                            KeyCode::KeyR => self.kbm.latch.reload = true,
                            KeyCode::Escape => { if self.grabbed { self.grab(false) } else { event_loop.exit() } }
                            KeyCode::F11 => {
                                if let Some(w) = &self.window {
                                    w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) });
                                }
                            }
                            _ => {}
                        }
                    }
                    self.kbm.keys.insert(code);
                } else {
                    self.kbm.keys.remove(&code);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.tick() { self.error = Some(e); event_loop.exit(); return; }
                self.frames += 1;
                if self.max_frames > 0 && self.frames >= self.max_frames {
                    let s = self.started.elapsed().as_secs_f64();
                    println!("smoke: {} frames in {s:.1} s ({:.1} fps), tick {}, round {}, digest {:016x}",
                        self.frames, self.frames as f64 / s, self.world.tick, self.world.round, self.world.digest());
                    event_loop.exit();
                    return;
                }
                if let Some(w) = &self.window { w.request_redraw(); }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Poll);
    }
}
