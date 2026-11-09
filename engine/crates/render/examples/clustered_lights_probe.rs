//! Clustered local lights: a plaza of pillars under hundreds of lamps, some
//! shadowed. Checks that one clustered light matches the same local light,
//! that hundreds light the plaza, and that shadowed lamps leave the far side
//! of a pillar dark. Run: clustered_lights_probe --out FRESH_DIRECTORY
//! [--lights 400] [--shadowed 24] [--size 960x540] [--frames 12]
use glam::{Affine3A, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::clustered_lights::ClusteredLight;
use render::headlights::LocalLight;
use render::scene::Material;
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::json;
use std::path::PathBuf;

fn quad(scene: &mut Scene, points: [[f32; 3]; 4], normal: Vec3, material: u32) {
    let vertices: Vec<_> = points.into_iter().zip([[0., 0.], [1., 0.], [1., 1.], [0., 1.]])
        .map(|(pos, uv)| Vertex { pos, normal: normal.to_array(), uv }).collect();
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices, indices: &[0, 1, 2, 0, 2, 3], material }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
}

fn pillar(scene: &mut Scene, c: Vec3, half: f32, height: f32, material: u32) {
    let (x0, x1, z0, z1, y0, y1) = (c.x - half, c.x + half, c.z - half, c.z + half, 0.0, height);
    for (p, n) in [
        ([[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], Vec3::Z),
        ([[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]], Vec3::NEG_Z),
        ([[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]], Vec3::X),
        ([[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]], Vec3::NEG_X),
        ([[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]], Vec3::Y),
    ] { quad(scene, p, n, material); }
}

const SPACING: f32 = 4.0;

fn scene(columns: i32) -> Scene {
    let mut scene = Scene::default();
    scene.graphics.clustered_lights = true;
    scene.sky = Some((16, 8, vec![[0.0, 0.0, 0.0, 1.0]; 128]));
    scene.sky_physical = true;
    let floor = scene.add_material(Material { tint: [0.5; 3], layer: -1, roughness: 0.7, flags: 0, scale: 1.0, metal: 0.0 });
    let stone = scene.add_material(Material { tint: [0.6, 0.55, 0.5], layer: -1, roughness: 0.6, flags: 0, scale: 1.0, metal: 0.0 });
    let e = columns as f32 * SPACING;
    quad(&mut scene, [[-e, 0., -e], [-e, 0., e], [e, 0., e], [e, 0., -e]], Vec3::Y, floor);
    for x in -columns..columns {
        for z in -columns..columns {
            pillar(&mut scene, Vec3::new((x as f32 + 0.5) * SPACING, 0.0, (z as f32 + 0.5) * SPACING), 0.35, 2.5, stone);
        }
    }
    scene
}

fn lamp(position: Vec3, color: Vec3, range: f32) -> LocalLight {
    LocalLight { position, direction: Vec3::NEG_Y, intensity_cd: color, range_m: range, inner_cos: -0.999, outer_cos: -1.0, source_radius_m: 0.05, falloff: 2.0 }
}

fn hue(i: usize) -> Vec3 {
    let h = (i as f32 * 0.618_034).fract() * 6.0;
    let v = Vec3::new((h - 3.0).abs() - 1.0, 2.0 - (h - 2.0).abs(), 2.0 - (h - 4.0).abs());
    v.clamp(Vec3::splat(0.0), Vec3::ONE) * 0.8 + Vec3::splat(0.2)
}

fn mean(rgba: &[u8], w: u32, x: u32, y: u32) -> f64 {
    let mut s = 0.0;
    for yy in y - 2..=y + 2 { for xx in x - 2..=x + 2 { let i = ((yy * w + xx) * 4) as usize; s += (rgba[i] as f64 + rgba[i + 1] as f64 + rgba[i + 2] as f64) / 3.0; } }
    s / 25.0
}

fn main() { if let Err(e) = run() { eprintln!("clustered lights probe: {e}"); std::process::exit(1); } }

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone());
    let out = PathBuf::from(arg("--out").ok_or("--out FRESH_DIRECTORY required")?);
    if out.exists() { return Err("output directory must be fresh".into()); }
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let count: usize = arg("--lights").and_then(|v| v.parse().ok()).unwrap_or(400);
    let shadowed: usize = arg("--shadowed").and_then(|v| v.parse().ok()).unwrap_or(24);
    let frames: usize = arg("--frames").and_then(|v| v.parse().ok()).unwrap_or(12);
    let (w, h) = arg("--size").and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(960), b.parse().unwrap_or(540)))).unwrap_or((960, 540));
    let columns = 6;
    let gpu = Gpu::new()?;
    let light = Lighting { sun_lux: 0.0, sky_lux: 0.0, ev100: 4.0, gi: 0.0, contact_occlusion: 0.0, reflection_occlusion: 0.0, shadow_strength: 0.0,
        near_shadows: 0.0, fog_density: 0.0, fog_start: 1000.0, fog_end: 2000.0, taa: 0.0, sharpen: 0.0, lod_pixels: 0.0, ..Default::default() };
    let renderer = Renderer::new(&gpu, &scene(columns), &light);
    if !renderer.clustered_lights_enabled() { return Err("clustered lights did not turn on".into()); }
    let cam = Camera { pos: Vec3::new(9.0, 9.0, 14.0), target: Vec3::new(0.0, 0.0, 0.0), fov_y: 55f32.to_radians(), near: 0.1 };
    let views = [ViewRect { cam, x: 0, y: 0, w, h }];
    let mut failures = Vec::new();
    let mut rows = Vec::new();
    let mut render = |name: &str, frames: usize, renderer: &Renderer| -> Result<Vec<u8>, String> {
        let mut last = (Vec::new(), None);
        for _ in 0..frames { let (rgba, stats) = renderer.try_render_views(&gpu, &views, &light, w, h, &[])?; last = (rgba, Some(stats)); }
        let stats = last.1.unwrap();
        image::save_buffer(out.join(format!("{name}.png")), &last.0, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
        let c = renderer.clustered_light_stats();
        rows.push(json!({"name": name, "lights": c.lights, "shadowed": c.shadowed, "dropped_shadows": c.dropped_shadows,
            "faces_drawn_last_frame": c.faces_drawn, "tiles_held": c.tiles_held, "cells": c.cells, "gpu_pass_ms": stats.gpu_ms}));
        Ok(last.0)
    };
    // 1. One light: clustered matches local.
    let probe = lamp(Vec3::new(0.0, 1.5, 0.0), Vec3::new(60.0, 50.0, 40.0), 6.0);
    renderer.set_local_lights(&[probe])?;
    let local = render("01-one-local", 2, &renderer)?;
    renderer.set_local_lights(&[])?;
    renderer.set_clustered_lights(&[ClusteredLight::unshadowed(probe)])?;
    let clustered = render("02-one-clustered", 2, &renderer)?;
    let differ = local.chunks_exact(4).zip(clustered.chunks_exact(4)).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 2)).count();
    if differ > (w * h / 1000) as usize { failures.push(format!("one clustered light differs from the same local light in {differ} pixels")); }
    let lit = clustered.chunks_exact(4).filter(|p| p[0] > 20).count();
    if lit < (w * h / 50) as usize { failures.push(format!("one clustered light lit only {lit} pixels")); }
    // 2. Hundreds of lamps over the plaza, the nearest shadowed.
    let e = columns as f32 * SPACING;
    let side = (count as f32).sqrt().ceil() as usize;
    let mut lights: Vec<ClusteredLight> = (0..count).map(|i| {
        let (gx, gz) = (i % side, i / side);
        let p = Vec3::new(-e + (gx as f32 + 0.5) * 2.0 * e / side as f32, 1.2 + (i % 3) as f32 * 0.4, -e + (gz as f32 + 0.5) * 2.0 * e / side as f32);
        ClusteredLight::unshadowed(lamp(p, hue(i) * 18.0, 3.5))
    }).collect();
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|&a, &b| lights[a].light.position.distance_squared(cam.target).total_cmp(&lights[b].light.position.distance_squared(cam.target)));
    for &i in order.iter().take(shadowed) { lights[i].shadow = Some(render::clustered_lights::LightShadow { id: i as u64, exclude: None }); }
    let plain: Vec<ClusteredLight> = lights.iter().map(|l| ClusteredLight { shadow: None, ..*l }).collect();
    renderer.set_clustered_lights(&plain)?;
    let unshadowed = render("03-many-unshadowed", 2, &renderer)?;
    renderer.set_clustered_lights(&lights)?;
    let shadows = render("04-many-shadowed", frames, &renderer)?;
    let lit = unshadowed.chunks_exact(4).filter(|p| p[0].max(p[1]).max(p[2]) > 30).count();
    if lit < (w * h / 4) as usize { failures.push(format!("{count} lights lit only {lit} pixels")); }
    let darker = unshadowed.chunks_exact(4).zip(shadows.chunks_exact(4))
        .filter(|(a, b)| (a[0] as i32 + a[1] as i32 + a[2] as i32) - (b[0] as i32 + b[1] as i32 + b[2] as i32) > 24).count();
    if darker < (w * h / 200) as usize { failures.push(format!("shadowed lamps darkened only {darker} pixels")); }
    let brighter = unshadowed.chunks_exact(4).zip(shadows.chunks_exact(4))
        .filter(|(a, b)| (b[0] as i32 + b[1] as i32 + b[2] as i32) - (a[0] as i32 + a[1] as i32 + a[2] as i32) > 6).count();
    if brighter > (w * h / 500) as usize { failures.push(format!("shadows brightened {brighter} pixels")); }
    // 3. A single shadowed lamp beside a pillar: its far side stays dark.
    let beside = lamp(Vec3::new(SPACING * 0.5 - 1.2, 1.0, SPACING * 0.5), Vec3::splat(80.0), 6.0);
    renderer.set_clustered_lights(&[ClusteredLight::shadowed(beside, 1)])?;
    let one = render("05-one-shadowed", frames, &renderer)?;
    // Pixels of the floor on either side of the pillar at (2, 0, 2).
    let project = |p: Vec3| {
        let vp = glam::Mat4::perspective_infinite_reverse_rh(cam.fov_y, w as f32 / h as f32, cam.near) * glam::Mat4::look_at_rh(cam.pos, cam.target, Vec3::Y);
        let c = vp * p.extend(1.0);
        let n = c.truncate() / c.w;
        (((n.x * 0.5 + 0.5) * w as f32) as u32, ((0.5 - n.y * 0.5) * h as f32) as u32)
    };
    let (sx, sy) = project(Vec3::new(SPACING * 0.5 + 1.2, 0.0, SPACING * 0.5));
    let (lx, ly) = project(Vec3::new(SPACING * 0.5 - 2.4, 0.0, SPACING * 0.5));
    let (shade, lit_side) = (mean(&one, w, sx, sy), mean(&one, w, lx, ly));
    if shade * 3.0 > lit_side { failures.push(format!("pillar shadow too bright: {shade:.1} behind vs {lit_side:.1} in front")); }
    // The same lamp unshadowed: floor pixels with a clear view of the lamp
    // (no pillar on the way to the camera or to the lamp) must not darken.
    // Grazing floor far from the lamp is where receiver bias fails.
    renderer.set_clustered_lights(&[ClusteredLight::unshadowed(beside)])?;
    let open = render("05b-one-unshadowed", 2, &renderer)?;
    let pillars: Vec<Vec3> = (-columns..columns).flat_map(|x| (-columns..columns).map(move |z| Vec3::new((x as f32 + 0.5) * SPACING, 0.0, (z as f32 + 0.5) * SPACING))).collect();
    let blocked = |a: Vec3, b: Vec3| pillars.iter().any(|c| {
        // Segment against the box, slab test, with a 5 cm margin.
        let (lo, hi) = (*c - Vec3::new(0.40, 0.0, 0.40), *c + Vec3::new(0.40, 2.55, 0.40));
        let d = b - a;
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for k in 0..3 {
            if d[k].abs() < 1e-6 { if a[k] < lo[k] || a[k] > hi[k] { return false; } continue; }
            let (u, v) = ((lo[k] - a[k]) / d[k], (hi[k] - a[k]) / d[k]);
            t0 = t0.max(u.min(v)); t1 = t1.min(u.max(v));
        }
        t0 <= t1
    });
    let inv = (glam::Mat4::perspective_infinite_reverse_rh(cam.fov_y, w as f32 / h as f32, cam.near) * glam::Mat4::look_at_rh(cam.pos, cam.target, Vec3::Y)).inverse();
    let mut false_shadow = 0;
    let mut open_floor = 0;
    for y in (0..h).step_by(2) { for x in (0..w).step_by(2) {
        let ndc = glam::Vec2::new((x as f32 + 0.5) / w as f32 * 2.0 - 1.0, 1.0 - (y as f32 + 0.5) / h as f32 * 2.0);
        let far = inv.project_point3(ndc.extend(0.5));
        let dir = (far - cam.pos).normalize();
        if dir.y >= -1e-3 { continue; }
        let p = cam.pos + dir * (-cam.pos.y / dir.y);
        if p.distance(beside.position) > beside.range_m * 0.95 || blocked(cam.pos, p) || blocked(p + Vec3::Y * 0.01, beside.position) { continue; }
        let i = ((y * w + x) * 4) as usize;
        let (a, b) = (open[i] as i32 + open[i + 1] as i32 + open[i + 2] as i32, one[i] as i32 + one[i + 1] as i32 + one[i + 2] as i32);
        if a < 6 { continue; }
        open_floor += 1;
        if b * 2 < a { false_shadow += 1; }
    }}
    if open_floor < 200 || false_shadow * 100 > open_floor { failures.push(format!("open floor darkened by the shadow in {false_shadow} of {open_floor} samples")); }
    renderer.set_clustered_lights(&[])?;
    let off = render("06-off", 1, &renderer)?;
    let still = off.chunks_exact(4).filter(|p| p[0].max(p[1]).max(p[2]) > 8).count();
    if still > 0 { failures.push(format!("{still} pixels stayed lit with no lights")); }
    let report = json!({"passed": failures.is_empty(), "failures": failures, "size": [w, h], "lights": count, "shadowed": shadowed,
        "one_light_differing_pixels": differ, "open_floor_false_shadow": [false_shadow, open_floor], "darkened_by_shadows": darker, "pillar_shadow": {"behind": shade, "front": lit_side}, "cases": rows});
    std::fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report).unwrap()).map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if report["passed"] == true { Ok(()) } else { Err("failed; see report.json".into()) }
}
