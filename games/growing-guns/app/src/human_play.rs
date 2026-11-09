use glam::Vec3;
use render::{
    Camera, Gpu,
    crowd::Seat,
    humans::{Far, ReviewRenderer},
    present::Presenter,
};
use std::{sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

struct Live {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    gpu: Gpu,
    config: wgpu::SurfaceConfiguration,
    presenter: Presenter,
    renderer: ReviewRenderer,
    color: wgpu::Texture,
    depth: wgpu::Texture,
    seats: Vec<Seat>,
    selected: usize,
    distance: f32,
    radius: f32,
    yaw: f32,
    pitch: f32,
    time: f32,
    clock: Instant,
    presented: bool,
    paused: bool,
    dragging: bool,
    cursor: Option<(f64, f64)>,
}
fn targets(g: &Gpu, w: u32, h: u32) -> (wgpu::Texture, wgpu::Texture) {
    let tex = |format, usage| {
        g.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let render = wgpu::TextureUsages::RENDER_ATTACHMENT;
    (
        tex(
            wgpu::TextureFormat::Rgba8Unorm,
            render | wgpu::TextureUsages::TEXTURE_BINDING,
        ),
        tex(wgpu::TextureFormat::Depth32Float, render),
    )
}
impl Live {
    fn new(el: &ActiveEventLoop, seed: u64) -> Result<Self, String> {
        let window = Arc::new(
            el.create_window(
                Window::default_attributes()
                    .with_title("Warp Humans")
                    .with_inner_size(winit::dpi::LogicalSize::new(1152., 720.)),
            )
            .map_err(|e| e.to_string())?,
        );
        let instance = render::gpu::instance();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let size = window.inner_size();
        let (gpu, config) =
            Gpu::for_surface(&instance, &surface, size.width.max(1), size.height.max(1))?;
        surface.configure(&gpu.device, &config);
        let (actors, seats, radius) = crate::human_review::population(seed)?;
        let renderer = ReviewRenderer::new(&gpu, &actors, Far::Impostor)?;
        let presenter = Presenter::new(&gpu, config.format);
        let (color, depth) = targets(&gpu, config.width, config.height);
        Ok(Self {
            window,
            surface,
            gpu,
            config,
            presenter,
            renderer,
            color,
            depth,
            seats,
            selected: 0,
            radius,
            distance: 3.4,
            yaw: 0.15,
            pitch: 0.04,
            time: 0.,
            clock: Instant::now(),
            presented: false,
            paused: false,
            dragging: false,
            cursor: None,
        })
    }
    fn zoom(&mut self, steps: f32) {
        self.distance = (self.distance * (-steps * 0.20).exp()).clamp(0.48, self.radius * 3.);
    }
    fn next(&mut self, delta: isize) {
        self.selected =
            (self.selected as isize + delta).rem_euclid(self.seats.len() as isize) as usize;
    }
    fn draw(&mut self) -> Result<(), String> {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        if (size.width, size.height) != (self.config.width, self.config.height) {
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&self.gpu.device, &self.config);
            (self.color, self.depth) = targets(&self.gpu, size.width, size.height);
        }
        let now = Instant::now();
        if !self.paused {
            self.time += (now - self.clock).as_secs_f32().min(0.1);
        }
        self.clock = now;
        let seat = &self.seats[self.selected];
        let wide = ((self.distance - 10.) / (self.radius * 1.8)).clamp(0., 1.);
        let head = (seat.pos
            + Vec3::Y * seat.scale * (0.92 - 0.36 * ((self.distance - 0.7) * 0.45).clamp(0., 1.)))
        .lerp(Vec3::Y * 12., wide);
        let yaw = self.yaw + seat.yaw;
        let pitch = (self.pitch + wide * 0.62).clamp(-1.4, 1.4);
        let dir = Vec3::new(
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        let cam = Camera {
            pos: head + dir * self.distance,
            target: head,
            near: 0.04,
            fov_y: 55f32.to_radians(),
        };
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.gpu.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            _ => return Err("surface acquisition failed".into()),
        };
        let sample = self
            .renderer
            .draw(&self.gpu, &cam, self.time, &self.color, &self.depth)?;
        if sample.counts[2] > 0 && !self.renderer.detail_ready {
            self.renderer.load_detail(&self.gpu);
        }
        self.presenter.present(
            &self.gpu,
            &self.color,
            &frame.texture.create_view(&Default::default()),
            None,
            None,
        )?;
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
        if !self.presented {
            self.presented = true;
            self.window.set_title("Warp Humans  |  Scroll: zoom   Drag: orbit   Left/Right: person   Home: stadium   End: face   Space: pause");
        }
        Ok(())
    }
}
struct Play {
    live: Option<Live>,
    seed: u64,
    error: Option<String>,
}
impl ApplicationHandler for Play {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.live.is_none() {
            match Live::new(el, self.seed) {
                Ok(l) => {
                    l.window.request_redraw();
                    self.live = Some(l)
                }
                Err(e) => {
                    self.error = Some(e);
                    el.exit();
                }
            }
        }
    }
    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(l) = self.live.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => {
                if let Err(e) = l.draw() {
                    self.error = Some(e);
                    el.exit();
                } else if !l.window.is_minimized().unwrap_or(false) {
                    l.window.request_redraw();
                }
            }
            WindowEvent::Resized(_) => l.window.request_redraw(),
            WindowEvent::Focused(false) => {
                l.dragging = false;
                l.cursor = None;
            }
            WindowEvent::MouseWheel { delta, .. } => l.zoom(match delta {
                MouseScrollDelta::LineDelta(_, y) => y,
                MouseScrollDelta::PixelDelta(p) => p.y as f32 / 75.,
            }),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => l.dragging = state == ElementState::Pressed,
            WindowEvent::CursorMoved { position, .. } => {
                let p = (position.x, position.y);
                if let Some((x, y)) = l.cursor {
                    if l.dragging {
                        l.yaw -= (p.0 - x) as f32 * 0.006;
                        l.pitch = (l.pitch + (p.1 - y) as f32 * 0.006).clamp(-1.3, 1.3);
                    }
                }
                l.cursor = Some(p);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.logical_key {
                    Key::Named(NamedKey::Escape) => el.exit(),
                    Key::Named(NamedKey::ArrowLeft) => l.next(-1),
                    Key::Named(NamedKey::ArrowRight) => l.next(1),
                    Key::Named(NamedKey::ArrowUp) => l.zoom(1.),
                    Key::Named(NamedKey::ArrowDown) => l.zoom(-1.),
                    Key::Named(NamedKey::Home) => l.distance = l.radius * 2.4,
                    Key::Named(NamedKey::End) => l.distance = 0.58,
                    Key::Named(NamedKey::Space) if !event.repeat => l.paused = !l.paused,
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
pub fn run(args: &crate::Args) -> Result<(), String> {
    let el = EventLoop::new().map_err(|e| e.to_string())?;
    let mut play = Play {
        live: None,
        seed: args.num("--seed", 52),
        error: None,
    };
    el.run_app(&mut play).map_err(|e| e.to_string())?;
    play.error.map_or(Ok(()), Err)
}
