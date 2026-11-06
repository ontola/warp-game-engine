//! Native VR: the same Racer and race, seen from the cockpit.
//!
//! The headset sits in a seat on the player's ship (`SEAT`, ship space); the
//! ship carries the play space, so the player looks around inside the cockpit
//! while the ship flies. `level` keeps the horizon level (comfort) instead of
//! rolling with the ship. The game's HUD and menus show on the head-locked
//! quad; motion blur, screen curvature and the chase camera's FOV changes are
//! off because the headset owns the projection.
use app::Game as _;
use glam::{Affine3A, Quat, Vec3};
use warp_xr::{Eye, PlaySpace, Pose, Tracking, HUD_SIZE, rig::level_seat};

const DT: f32 = 1.0 / 120.0;
/// Where the pilot's head sits on the ship: +Z ahead, +Y up, metres.
pub const SEAT: Vec3 = Vec3::new(0.0, 2.0, 0.5);
/// The chase view: behind and above the ship, always level (comfort).
pub const CHASE: Vec3 = Vec3::new(0.0, 3.2, -12.0);

pub struct NativeGame {
    game: crate::Racer,
    renderer: Option<render::Renderer>,
    output: Option<wgpu::Texture>,
    space: PlaySpace,
    accumulator: f32,
    pending: u32,
    /// Keep the horizon level instead of rolling with the ship.
    pub level: bool,
    /// `SEAT`, or `SPACERACER_SEAT=x,y,z` to try another on the headset.
    pub seat_offset: Vec3,
    /// Chase view instead of the cockpit: right stick press toggles,
    /// `SPACERACER_VIEW=chase` starts there.
    pub chase: bool,
}

fn seat_offset() -> Vec3 {
    std::env::var("SPACERACER_SEAT").ok().and_then(|v| {
        let n: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (n.len() == 3).then(|| Vec3::new(n[0], n[1], n[2]))
    }).unwrap_or(SEAT)
}

/// Process-wide, before the first race: the headset shell draws straight
/// into the eye images, which needs the minimum renderer.
fn use_minimum_renderer() { crate::visuals::MINIMUM.store(true, std::sync::atomic::Ordering::Relaxed); }

impl NativeGame {
    pub fn new(root: std::path::PathBuf) -> Result<Self, String> {
        let mut game = crate::Racer::new(root, 1, 35, 0, 1, false)?;
        game.random_biome = true;
        game.random_seed = true;
        game.blur_enabled = false;
        // Touch controllers read like a gamepad: triggers are throttle and brake.
        game.devices = vec![app::Device::Pad(0)];
        Ok(Self { game, renderer: None, output: None, space: PlaySpace::new(0.0), accumulator: 0.0, pending: 0, level: false, seat_offset: seat_offset(), chase: std::env::var("SPACERACER_VIEW").is_ok_and(|v| v == "chase") })
    }

    /// The seat on the local pilot's ship, interpolated between ticks.
    fn seat(&self, alpha: f32) -> Affine3A {
        let current = self.game.race.pose(0);
        let previous = self.game.previous.first().copied().unwrap_or(current);
        let rotation = Quat::from_mat3(&previous.basis).slerp(Quat::from_mat3(&current.basis), alpha);
        let ship = Affine3A::from_rotation_translation(rotation, previous.origin.lerp(current.origin, alpha));
        // The headset looks along -Z; the ship flies along +Z.
        let offset = if self.chase { CHASE } else { self.seat_offset };
        let seat = ship * Affine3A::from_rotation_translation(Quat::from_rotation_y(std::f32::consts::PI), offset);
        if self.level || self.chase { level_seat(seat) } else { seat }
    }
}

impl warp_xr::Game for NativeGame {
    fn refresh_rate(&self) -> Option<f32> { Some(120.0) }
    fn quit(&self) -> bool { self.game.quit() }
    fn frame(&mut self, gpu: &render::Gpu, t: &Tracking, size: [u32; 2], dt: f32, targets: Option<&[&wgpu::Texture]>) -> Result<warp_xr::FrameOutput, String> {
        self.space.update(t);
        if t.focused && t.controls.pressed.has(app::Buttons::RIGHT_STICK) { self.chase = !self.chase; }
        self.pending |= t.controls.pressed.0;
        if t.focused { self.accumulator = (self.accumulator + dt).min(DT * 4.0); } else { self.accumulator = 0.0; self.pending = 0; }
        while self.accumulator >= DT {
            let mut controls = t.controls;
            controls.pressed.0 = std::mem::take(&mut self.pending);
            self.game.tick(&[controls]);
            self.accumulator -= DT;
        }
        let alpha = self.accumulator / DT;
        let mut frame = app::Frame::new(gpu, &mut self.renderer, HUD_SIZE, dt, alpha);
        let mut draw = self.game.draw(&mut frame)?;
        // The quad is flat and head-locked: no simulated screen curvature.
        draw.hud.optics.clear();
        let seat = self.seat(alpha);
        let eyes: [Eye; 2] = t.eyes.map(|eye| Eye { pose: self.space.place_on(eye.pose, seat), ..eye });
        let renderer = self.renderer.as_ref().ok_or("SpaceRacer did not create its renderer")?;
        renderer.set_camera_up_axes(&eyes.map(|e| e.pose.orientation * Vec3::Y));
        renderer.set_camera_projections(&eyes.map(Eye::projection));
        let views: Vec<_> = eyes.iter().enumerate().map(|(i, e)| render::ViewRect { cam: e.camera(), x: i as u32 * size[0], y: 0, w: size[0], h: size[1] }).collect();
        if let Some(targets) = targets {
            renderer.try_render_eye_targets(gpu, &views, &draw.light, targets)?;
            return Ok(warp_xr::FrameOutput { color: None, hud: draw.hud });
        }
        if self.output.as_ref().is_none_or(|tex| tex.width() != size[0] * 2 || tex.height() != size[1]) {
            self.output = Some(render::Renderer::output_texture(gpu, size[0] * 2, size[1]));
        }
        let output = self.output.as_ref().unwrap();
        renderer.try_render_into(gpu, &views, &draw.light, output)?;
        Ok(warp_xr::FrameOutput { color: Some(output.clone()), hud: draw.hud })
    }
}

/// Headless stereo fixture on the desktop renderer: `spaceracer xr-shot --out
/// FILE.png [--ticks N] [--level] [--size W]`. Not a headset test.
pub fn shot(args: &[String]) -> Result<(), String> {
    use warp_xr::Game;
    let arg = |key: &str| args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone());
    let out = arg("--out").ok_or("xr-shot --out FILE.png")?;
    let eye = arg("--size").and_then(|s| s.parse().ok()).unwrap_or(640u32);
    crate::install_shaders(false);
    use_minimum_renderer();
    let gpu = render::Gpu::new()?;
    let mut game = NativeGame::new(crate::game_root(arg("--game-dir")))?;
    game.level = args.iter().any(|a| a == "--level");
    game.chase = game.chase || args.iter().any(|a| a == "--chase");
    // Close the start menu and fly: full throttle on the right trigger.
    game.game.menu = None;
    game.game.race_started = true;
    let fly = app::Controls { triggers: [0.0, 1.0], ..Default::default() };
    for _ in 0..arg("--ticks").and_then(|s| s.parse().ok()).unwrap_or(900u32) { game.game.tick(&[fly]); }
    let head = Pose { position: Vec3::new(0.0, 1.2, 0.0), orientation: Quat::IDENTITY };
    let eyes = [-0.032, 0.032].map(|x| Eye { pose: Pose { position: head.position + Vec3::X * x, ..head }, fov: [-0.8, 0.8, 0.8, -0.8] });
    let tracking = Tracking { eyes, head, aim: None, controls: fly, turn: 0.0, focused: true, recentered: false };
    // The Quest path: draw straight into two eye images, as android.rs does.
    let eye_texture = || gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("fixture eye"), size: wgpu::Extent3d { width: eye, height: eye, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8UnormSrgb, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
    let (left, right) = (eye_texture(), eye_texture());
    for _ in 0..3 { game.frame(&gpu, &tracking, [eye, eye], 0.0, Some(&[&left, &right]))?; }
    let (l, r) = (gpu.read_rgba8(&left), gpu.read_rgba8(&right));
    let mut both = Vec::with_capacity(l.len() * 2);
    for row in 0..eye as usize { let w = eye as usize * 4; both.extend_from_slice(&l[row * w..][..w]); both.extend_from_slice(&r[row * w..][..w]); }
    render::write_png(std::path::Path::new(&out.replace(".png", "-direct.png")), eye * 2, eye, &both)?;
    let mut frame = game.frame(&gpu, &tracking, [eye, eye], 0.0, None)?;
    for _ in 0..7 { frame = game.frame(&gpu, &tracking, [eye, eye], 0.0, None)?; }
    render::write_png(std::path::Path::new(&out), eye * 2, eye, &gpu.read_rgba8(frame.color.as_ref().unwrap()))?;
    let pose = game.game.race.pose(0);
    println!("SpaceRacer stereo fixture: {out}; ship at {:?}, speed {:.0}; GPU {}. This is not a headset test.", pose.origin, game.game.race.pilots[0].speed, gpu.info.name);
    Ok(())
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_tag("WarpXR").with_max_level(log::LevelFilter::Info));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
        let root = install_assets(&app)?;
        crate::install_shaders(false);
        use_minimum_renderer();
        warp_xr::run(app, move || NativeGame::new(root.clone())).map_err(|e| format!("{e:#}"))
    }));
    match result { Ok(Ok(())) => {}, Ok(Err(e)) => log::error!("Warp XR stopped: {e}"), Err(_) => log::error!("Warp XR panicked; inspect logcat") }
}

#[cfg(target_os = "android")]
fn install_assets(app: &android_activity::AndroidApp) -> Result<std::path::PathBuf, String> {
    use std::io::Read;
    let root = app.internal_data_path().ok_or("Android internal data path missing")?.join("spaceracer");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut asset = app.asset_manager().open(c"spaceracer.zip").ok_or("APK missing SpaceRacer assets")?;
    let mut bytes = Vec::new();
    asset.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    zip.extract(&root).map_err(|e| e.to_string())?;
    // Done before game/audio/renderer worker threads start.
    unsafe { std::env::set_var("KERN_PIPELINE_CACHE_DIR", root.join("pipeline-cache")); }
    std::env::set_current_dir(&root).map_err(|e| e.to_string())?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_seat_looks_where_the_ship_flies() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let game = NativeGame::new(root).unwrap();
        let pose = game.game.race.pose(0);
        let seat = game.seat(1.0);
        let ahead = seat.transform_vector3(Vec3::NEG_Z);
        assert!(ahead.dot(pose.basis * Vec3::Z) > 0.999);
        assert!((seat.translation.distance(pose.origin.into()) - SEAT.length()).abs() < 1e-3);
    }
}
