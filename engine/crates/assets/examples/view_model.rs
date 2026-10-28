//! Look at a glTF model without a window: renders it on a ground plane
//! from `--views` angles into one PNG.
//!   cargo run -p assets --example view_model -- MODEL.gltf --out model.png [--views 4]
use app::{Controls, Draw, Frame, Game};
use glam::{Affine3A, Quat, Vec3};
use render::scene::Material;
use render::{primitives, Camera, Lighting, Scene};

struct Viewer { scene: Scene, center: Vec3, radius: f32, views: usize, uploaded: bool }

impl Game for Viewer {
    fn local_players(&self) -> usize { 1 }
    fn tick(&mut self, _: &[Controls]) {}
    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let light = Lighting { taa: 0.0, ..Default::default() };
        if !std::mem::replace(&mut self.uploaded, true) { frame.set_scene(&self.scene, &light); }
        let cameras = (0..self.views).map(|i| {
            let a = 0.6 + i as f32 * std::f32::consts::TAU / self.views as f32;
            let d = self.radius * 2.6;
            Camera { pos: self.center + Vec3::new(a.cos() * d, self.radius * 1.1, a.sin() * d), target: self.center, fov_y: 0.8, near: 0.01 }
        }).collect();
        Ok(Draw { cameras, light, hud: render::hud::HudDrawList::new(frame.size) })
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let path = args.get(1).filter(|a| !a.starts_with("--")).ok_or("view_model MODEL.gltf --out FILE.png")?;
    let out = arg("--out").unwrap_or_else(|| "model.png".into());
    let views = arg("--views").and_then(|v| v.parse().ok()).unwrap_or(4usize).clamp(1, 4);
    let mut scene = Scene::default();
    scene.sky = Some(primitives::gradient_sky([0.3, 0.45, 0.7], [0.75, 0.8, 0.85], [0.35, 0.33, 0.3]));
    let model = assets::gltf::load(&mut scene, path.as_ref())?;
    let (center, radius) = ((model.min + model.max) * 0.5, ((model.max - model.min).length() * 0.5).max(0.01));
    let ground_m = scene.add_material(Material { tint: [0.35; 3], layer: -1, roughness: 0.9, flags: 0, scale: 1.0, metal: 0.0 });
    let ground = scene.add_mesh(&primitives::cube(ground_m).mesh());
    scene.add_instance(ground, Affine3A::from_scale_rotation_translation(Vec3::new(radius * 8.0, radius * 0.02, radius * 8.0), Quat::IDENTITY, Vec3::new(center.x, model.min.y - radius * 0.01, center.z)));
    scene.add_instance(model.mesh, Affine3A::IDENTITY);
    println!("{path}: {} triangles, {} materials, {} textures, bounds {:?} .. {:?}", model.triangles, model.materials.len(), scene.textures.len(), model.min, model.max);
    let gpu = render::Gpu::new()?;
    let mut game = Viewer { scene, center, radius, views, uploaded: false };
    let rgba = app::capture(&mut game, &gpu, [1280, 720], 0, 60.0)?;
    render::write_png(out.as_ref(), 1280, 720, &rgba)?;
    println!("{out}");
    Ok(())
}
