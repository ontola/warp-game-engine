//! Repeatable modelling review: real card application, gameplay meshes and
//! materials, fixed lighting, three views, and a manifest for review sheets.
use glam::{Affine3A, Quat, Vec3};
use render::{Camera, Gpu, Renderer};

pub fn run(args: &crate::Args) -> Result<(), String> {
    if args.flag("--grip") { return grip(args); }
    if args.flag("--growth") { return growth(args); }
    let out = std::path::Path::new(args.value("--out").unwrap_or("build/gun-review"));
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let (w, h) = args.size((1600, 1000));
    if w < 64 || h < 64 || w > 4096 || h > 4096 { return Err("review size must be 64..4096 per axis".into()); }
    let variants: &[(&str, &str, &[&str])] = &[
        ("base", "Basis", &[]),
        ("shotgun", "Shotgun (kort)", &["shotgun"]),
        ("uzi", "Uzi (kort)", &["uzi"]),
        ("poison", "Poison", &["poison"]),
        ("cooling", "Chilling Rounds", &["chilling_rounds"]),
        ("poison-cooling", "Poison + Chilling Rounds", &["poison", "chilling_rounds"]),
        ("sniper-poison", "Sniper + Poison + Heavy Rounds", &["sniper", "poison", "heavy_rounds"]),
        ("shotgun-heavy", "Shotgun + Extra Barrel + Heavy Rounds", &["shotgun", "extra_barrel", "heavy_rounds"]),
        ("minigun", "Uzi + Rapid Fire x2 + Big Mag", &["uzi", "rapid_fire", "rapid_fire", "big_mag"]),
        ("explosive", "Explosive Rounds x1", &["explosive"]),
        ("explosive-stacked", "Explosive Rounds x3", &["explosive", "explosive", "explosive"]),
        ("explosive-heavy", "Explosive + Heavy Rounds + Big Mag x2", &["explosive", "heavy_rounds", "big_mag", "big_mag"]),
        ("sniper-cooling", "Sniper + Chilling Rounds + Big Mag", &["sniper", "chilling_rounds", "big_mag"]),
    ];
    if let Some(only) = args.value("--only") && !variants.iter().any(|v| v.0 == only) {
        return Err(format!("unknown review variant {only}"));
    }
    let mut world = growing_guns::World::new(3, &[false]);
    let _scripts = crate::scripts::attach(&mut world)?;
    let mut gpu = Gpu::new()?;
    gpu.features.remove(wgpu::Features::TIMESTAMP_QUERY);
    let light = render::Lighting { sun_dir: Vec3::new(1.0, 1.6, 0.7).normalize(),
        sun_lux: 8000.0, sky_lux: 3500.0, sun_color: Vec3::ONE, ev100: 12.0,
        taa: 0.0, fog_density: 0.0, gi: 0.0, contact_occlusion: 0.0,
        reflection_occlusion: 0.0, lod_pixels: 0.0, ..Default::default() };
    let mut manifest = Vec::new();
    for &(id, title, cards) in variants {
        if args.value("--only").is_some_and(|only| only != id) { continue; }
        world.players[0] = growing_guns::Player::new(false);
        for &card in cards {
            let card = world.cards.find(card).ok_or_else(|| format!("missing card {card}"))?;
            world.give(0, card);
        }
        let stats = crate::gun::Stats::of(&world.players[0]);
        let (mut scene, lo, hi, parts, triangles) = crate::view::guns::review_scene(stats);
        apply_graphics(&mut scene);
        let mat = scene.add_material(render::scene::Material { tint: [0.16, 0.18, 0.21],
            layer: -1, roughness: 0.95, flags: 0, scale: 1.0, metal: 0.0 });
        let cube = scene.add_mesh(&render::primitives::cube(mat).mesh());
        scene.add_instance(cube, Affine3A::from_scale_rotation_translation(Vec3::new(30.0, 0.1, 30.0),
            Quat::IDENTITY, Vec3::new(0.0, lo.y - 0.09, 0.0)));
        scene.add_instance(cube, Affine3A::from_scale_rotation_translation(Vec3::new(0.1, 20.0, 30.0),
            Quat::IDENTITY, Vec3::new(-4.0, 5.0, 0.0)));
        let renderer = Renderer::new(&gpu, &scene, &light);
        let center = (lo + hi) * 0.5;
        let mut files = Vec::new();
        for (view, direction) in [("three-quarter", Vec3::new(1.0, 0.5, 0.65)),
            ("side", Vec3::new(1.0, 0.03, 0.0)), ("detail", Vec3::new(1.0, 0.08, 0.25))] {
            let dir = direction.normalize();
            let right = (-dir).cross(Vec3::Y).normalize();
            let up = right.cross(-dir);
            let fov = 32f32.to_radians();
            let tan = (fov * 0.5).tan();
            let mut distance = 0.1f32;
            for x in [lo.x, hi.x] { for y in [lo.y, hi.y] { for z in [lo.z, hi.z] {
                let p = Vec3::new(x, y, z) - center;
                distance = distance.max(p.dot(dir) + (p.dot(right).abs() / (tan * w as f32 / h as f32)).max(p.dot(up).abs() / tan));
            } } }
            let target = if view == "detail" { Vec3::new(0.0, -0.065, 0.035) } else { center };
            let distance = if view == "detail" { 0.75 } else { distance * 1.18 };
            let camera = Camera { pos: target + dir * distance, target, fov_y: fov, near: 0.01 };
            let (rgba, _) = renderer.try_render_views(&gpu, &[render::ViewRect { cam: camera, x: 0, y: 0, w, h }], &light, w, h, &[])?;
            let file = format!("{id}-{view}.png");
            render::write_png(&out.join(&file), w, h, &rgba)?;
            files.push(file);
        }
        manifest.push(serde_json::json!({"id":id,"title":title,"cards":cards,"parts":parts,"triangles":triangles,"simple_guns":crate::graphics::get().simple_guns,
            "bounds_m":[lo.to_array(),hi.to_array()],"visual_stats":format!("{stats:?}"),"images":files}));
        println!("{id}: {parts} parts, {triangles} triangles, cards {cards:?}");
    }
    std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Fixed-camera clips of the actual runtime morph, including topology changes.
/// Write 30 fps frames; no game window or synthetic interpolation is involved.
fn growth(args: &crate::Args) -> Result<(), String> {
    let out = std::path::Path::new(args.value("--out").unwrap_or("build/gun-growth-review"));
    let (w, h) = args.size((960, 600));
    if w < 64 || h < 64 || w > 4096 || h > 4096 { return Err("review size must be 64..4096 per axis".into()); }
    let cases: &[(&str, &[&str], &[&str])] = &[
        ("damage-rail", &[], &["heavy_rounds"]),
        ("sniper-scope", &[], &["sniper"]),
        ("drum-minigun", &["uzi"], &["uzi", "rapid_fire", "rapid_fire", "big_mag"]),
        ("elemental", &[], &["poison", "chilling_rounds"]),
        ("character-only", &[], &["acrobat"]),
    ];
    if args.value("--only").is_some_and(|id| !cases.iter().any(|c| c.0 == id)) { return Err("unknown growth case".into()); }
    let mut world = growing_guns::World::new(3, &[false]);
    let _scripts = crate::scripts::attach(&mut world)?;
    let mut stats = |cards: &[&str]| -> Result<crate::gun::Stats, String> {
        world.players[0] = growing_guns::Player::new(false);
        for &name in cards { let card = world.cards.find(name).ok_or_else(|| format!("missing card {name}"))?; world.give(0, card); }
        Ok(crate::gun::Stats::of(&world.players[0]))
    };
    let mut gpu = Gpu::new()?;
    gpu.features.remove(wgpu::Features::TIMESTAMP_QUERY);
    let light = render::Lighting { sun_dir: Vec3::new(1.0, 1.6, 0.7).normalize(),
        sun_lux: 8000.0, sky_lux: 3500.0, sun_color: Vec3::ONE, ev100: 12.0,
        taa: 0.0, fog_density: 0.0, gi: 0.0, contact_occlusion: 0.0,
        reflection_occlusion: 0.0, lod_pixels: 0.0, ..Default::default() };
    for &(id, before, after) in cases {
        if args.value("--only").is_some_and(|only| only != id) { continue; }
        let (from, to) = (stats(before)?, stats(after)?);
        let dir = out.join(id); std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut scene = render::Scene::default();
        apply_graphics(&mut scene);
        let guns = crate::view::guns::Guns::reserve(&mut scene, 1);
        let mut rig = crate::gun::Rig::with_detail(guns.detail); rig.set(from); guns.review_pose(&mut scene, &rig);
        let floor = scene.add_material(render::scene::Material { tint: [0.16, 0.18, 0.21], layer: -1, roughness: 0.95, flags: 0, scale: 1.0, metal: 0.0 });
        let cube = scene.add_mesh(&render::primitives::cube(floor).mesh());
        scene.add_instance(cube, Affine3A::from_scale_rotation_translation(Vec3::new(30.0, 0.1, 30.0), Quat::IDENTITY, Vec3::new(0.0, -0.8, 0.0)));
        scene.add_instance(cube, Affine3A::from_scale_rotation_translation(Vec3::new(0.1, 20.0, 30.0), Quat::IDENTITY, Vec3::new(-4.0, 5.0, 0.0)));
        let renderer = Renderer::new(&gpu, &scene, &light);
        let target = if id == "sniper-scope" { Vec3::new(0.0, -0.1, -0.65) } else { Vec3::new(0.0, -0.14, -0.15) };
        let distance = if id == "sniper-scope" { 3.4 } else { 1.85 };
        let camera = Camera { pos: target + Vec3::new(1.0, 0.20, 0.25).normalize() * distance, target, fov_y: 32f32.to_radians(), near: 0.01 };
        let mut active = Vec::new();
        for frame in 0..90 {
            if frame == 15 { rig.grow_to(to); }
            if frame >= 15 { rig.update(1.0 / 30.0); }
            active.push(rig.is_growing());
            guns.review_pose(&mut scene, &rig);
            renderer.update_instances(&gpu, 0, &scene.instances)?;
            let (rgba, _) = renderer.try_render_views(&gpu, &[render::ViewRect { cam: camera, x: 0, y: 0, w, h }], &light, w, h, &[])?;
            render::write_png(&dir.join(format!("frame-{frame:03}.png")), w, h, &rgba)?;
        }
        std::fs::write(dir.join("manifest.json"), serde_json::to_vec_pretty(&serde_json::json!({"from":before,"to":after,"fps":30,"morph_active":active})).unwrap()).map_err(|e| e.to_string())?;
        println!("{id}: 90 frames");
    }
    Ok(())
}

/// Review both the actual gun topology and the selected runtime lighting path.
fn apply_graphics(scene: &mut render::Scene) {
    let g = crate::graphics::get();
    scene.graphics.minimal_forward = g.minimal_forward;
    scene.graphics.simple_lighting = g.simple_lighting;
    scene.graphics.simple_mesh_culling = g.simple_blocks;
}
fn grip(args: &crate::Args) -> Result<(), String> {
    use crate::view::guns::Guns;
    use assets::skinned::Character;
    use std::path::Path;

    let name = args.value("--model").unwrap_or("knight");
    if !["knight", "paladin", "ch10"].contains(&name) { return Err("unknown roster model".into()); }
    let model = Character::load(&Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../assets/characters").join(format!("{name}.json")))?;
    let clip = args.value("--clip").unwrap_or("pistol_idle");
    let time = args.num("--time", 0.25f32);
    let pitch = args.num("--pitch", 0.0f32).to_radians();
    if !pitch.is_finite() || !time.is_finite() { return Err("expected finite pitch and time".into()); }
    let looped = clip != "pistol_jump";
    let palette = model.sample(clip, time, looped)?;
    let hand = model.sample_socket(clip, "RightHand", time, looped)?.w_axis.truncate();
    let arms = crate::view::characters::grip_arms(&model).ok_or("model needs two complete arms")?;
    let mut world = growing_guns::World::new(3, &[false]);
    let _scripts = crate::scripts::attach(&mut world)?;
    let base = crate::gun::Stats::of(&world.players[0]);
    let growth = args.flag("--growth");
    if growth && args.value("--card").is_none() { return Err("--grip --growth requires --card".into()); }
    if let Some(card) = args.value("--card") {
        let id = world.cards.find(card).ok_or_else(|| format!("unknown card {card}"))?;
        world.give(0, id);
    }
    let stats = crate::gun::Stats::of(&world.players[0]);
    let mut rig = crate::gun::Rig::default();
    rig.set(if growth { base } else { stats });
    let body = crate::view::characters::model_transform(Vec3::ZERO, 0.0);
    let camera_anchor = Guns::held_transform(body.transform_point3(hand), 0.0, pitch, rig.contacts_at(1.0).primary);
    if growth { rig.grow_to(stats); }
    let out = Path::new(args.value("--out").unwrap_or("build/grip-review"));
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let (w, h) = args.size((768, 640));
    if !(64..=4096).contains(&w) || !(64..=4096).contains(&h) { return Err("invalid preview size".into()); }
    let gpu = Gpu::new()?;
    let light = render::Lighting { sun_dir: Vec3::new(-0.6, 0.9, -0.6).normalize(),
        sky_lux: 4500.0, sun_lux: 9000.0, ev100: 12.0, taa: 0.0, fog_density: 0.0, ..Default::default() };
    let mut animation = crate::hand_animation::HandAnimation::load()?;
    for frame in 0..if growth { 25 } else { 2 } {
        if growth && frame > 0 {
            // Rig limits each update to 50 ms. Advance five simulation ticks per image.
            for _ in 0..5 { rig.update(crate::gun::GROW_SECONDS / 120.0); }
            if frame == 24 && rig.is_growing() { rig.update(1e-5); }
        }
        let apply = growth || frame == 1;
        let label = if growth { format!("frame-{frame:03}") } else if apply { "grip".into() } else { "clip".into() };
        let contacts = rig.contacts_at(1.0);
        let primary = contacts.primary;
        let (support, roll) = animation.pose(contacts, time + if growth { frame as f32 / 12.0 } else { 0.0 });
        let held = Guns::held_transform(body.transform_point3(hand), 0.0, pitch, primary);
        let target = body.inverse().transform_point3(held.transform_point3(support));
        let mut scene = render::Scene::default();
        let guns = Guns::reserve(&mut scene, 1);
        guns.review_pose(&mut scene, &rig);
        let mut pose = palette.clone();
        let primary_hand = if apply {
            let wrist = arms.apply(&mut pose, hand, target);
            arms.orient_support(&mut pose, crate::view::characters::grip_rotation(pitch) * Quat::from_rotation_x(roll));
            wrist
        } else { hand };
        let model_to_gun = Guns::held_transform(body.transform_point3(primary_hand), 0.0, pitch, primary).inverse() * body;
        for (material, vertices, indices, _) in model.posed_surfaces(&mut scene, &pose, model_to_gun) {
            let mesh = scene.add_mesh(&render::primitives::Primitive { material, vertices, indices }.mesh());
            scene.add_instance(mesh, Affine3A::IDENTITY);
        }
        let camera_space = if growth { model_to_gun * body.inverse() * camera_anchor } else { Affine3A::IDENTITY };
        let camera = Camera { pos: camera_space.transform_point3(Vec3::new(-1.65, 0.56, -0.95)),
            target: camera_space.transform_point3(Vec3::new(0.0, 0.0, -0.12)),
            near: 0.02, fov_y: 38f32.to_radians() };
        let renderer = Renderer::new(&gpu, &scene, &light);
        let (rgba, _) = renderer.try_render_views(&gpu, &[render::ViewRect { cam: camera, x: 0, y: 0, w, h }], &light, w, h, &[])?;
        render::write_png(&out.join(format!("{label}.png")), w, h, &rgba)?;
    }
    Ok(())
}
