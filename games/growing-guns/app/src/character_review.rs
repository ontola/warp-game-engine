//! Close-ups of the roster for checking skin and hair lighting: each
//! character in its first clip's pose, from the front and three-quarter in
//! sun, and lit from behind (ears and fingers let light through), each with
//! skin and hair lighting and without. No window.
//!
//! `growing-guns character-review --out build/character-review [--only paladin] [--size 900x900] [--hair]`
use glam::{Mat4, Vec3};
use render::{Camera, Gpu, Renderer};

pub fn run(args: &crate::Args) -> Result<(), String> {
    use assets::skinned::Character;
    let out = std::path::Path::new(args.value("--out").unwrap_or("build/character-review"));
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let (w, h) = args.size((900, 900));
    if w < 64 || h < 64 || w > 4096 || h > 4096 { return Err("review size must be 64..4096 per axis".into()); }
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/characters");
    let mut gpu = Gpu::new()?;
    gpu.features.remove(wgpu::Features::TIMESTAMP_QUERY);
    let mut manifest = Vec::new();
    for name in ["paladin", "ch10", "knight"] {
        if args.value("--only").is_some_and(|only| only != name) { continue; }
        for skin in [false, true] {
            let mut model = Character::load(&dir.join(format!("{name}.json")))?;
            if skin { crate::view::characters::apply_skin_hair(&mut model, &dir)?; }
            // No roster character shows hair (the paladin's is under his
            // helmet): `--hair` lights the zombie's body as hair to check it.
            if skin && args.flag("--hair") && name == "ch10" { model.set_skin_hair(0, assets::skinned::SkinHair::Hair)?; }
            let mut scene = render::Scene::default();
            let floor = scene.add_material(render::scene::Material { tint: [0.16, 0.18, 0.21], layer: -1, roughness: 0.95, flags: 0, scale: 1.0, metal: 0.0 });
            let cube = scene.add_mesh(&render::primitives::cube(floor).mesh());
            scene.add_instance(cube, glam::Affine3A::from_scale_rotation_translation(Vec3::new(30.0, 0.1, 30.0), glam::Quat::IDENTITY, Vec3::new(0.0, -0.95, 0.0)));
            let body = crate::view::characters::model_transform(Vec3::ZERO, 0.0);
            model.add_actor(&mut scene, body)?;
            let clip = &model.clips[0];
            let head_socket: Mat4 = model.sample_socket(&clip.name, "Head", 0.0, clip.looping)?;
            let head = body.transform_point3(head_socket.w_axis.truncate()) + Vec3::Y * 0.08;
            // The first clip's pose turns the body a quarter from the
            // model's -Z; look at the face, as measured in these renders.
            let front = Vec3::Y.cross(-body.transform_vector3(Vec3::Z)).normalize();
            let side = front.cross(Vec3::Y).normalize();
            let shots: [(&str, Vec3, Vec3, f32, Vec3); 4] = [
                ("face", head, front, 0.75, (front * 0.8 + side * 0.9 + Vec3::Y * 1.2).normalize()),
                ("face-three-quarter", head, (front + side * 0.9).normalize(), 0.75, (front * 0.3 - side + Vec3::Y * 0.9).normalize()),
                ("backlit", head, (front + side * 0.35).normalize(), 0.75, (-front * 1.0 + side * 0.25 + Vec3::Y * 0.35).normalize()),
                ("body", head - Vec3::Y * 0.75, (front + side * 0.5).normalize(), 3.0, (front * 0.6 + side + Vec3::Y * 1.3).normalize()),
            ];
            let mut renderer: Option<Renderer> = None;
            for (view, target, towards, distance, sun) in shots {
                let light = render::Lighting { sun_dir: sun, sun_lux: 60000.0, sky_lux: 9000.0, sun_color: Vec3::new(1.0, 0.96, 0.9),
                    ev100: 14.0, taa: 0.0, fog_density: 0.0, ..Default::default() };
                let renderer = renderer.get_or_insert_with(|| Renderer::new(&gpu, &scene, &light));
                let camera = Camera { pos: target + towards * distance, target, fov_y: 28f32.to_radians(), near: 0.01 };
                let (rgba, _) = renderer.try_render_views(&gpu, &[render::ViewRect { cam: camera, x: 0, y: 0, w, h }], &light, w, h, &[])?;
                let file = format!("{name}-{view}-{}.png", if skin { "skin" } else { "plain" });
                render::write_png(&out.join(&file), w, h, &rgba)?;
                manifest.push(serde_json::json!({"character": name, "view": view, "skin_hair": skin, "image": file}));
                println!("{file}");
            }
        }
    }
    std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
