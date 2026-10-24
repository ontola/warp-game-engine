//! Compare live GPU plant deformation with a separately baked reference mesh.
//! This checks visibility reconstruction, normals and sun shadows together.
//! Run only without another GPU workload: wind_probe --out FRESH_DIRECTORY.
use glam::{Affine3A, Vec2, Vec3, Vec3Swizzles};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Material, FOLIAGE, TWO_SIDED};
use render::wind::WindState;
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::path::PathBuf;

const SIZE: u32 = 257;

fn mesh(scene: &mut Scene, points: &[Vec3], normal: Vec3, indices: &[u32], material: u32,
        shear: Vec2, foliage: bool) {
    let n = Vec3::new(normal.x, normal.y - shear.dot(normal.xz()), normal.z).normalize();
    let vertices: Vec<_> = points.iter().enumerate().map(|(i, p)| {
        let mut p = *p;
        p.x += shear.x * p.y;
        p.z += shear.y * p.y;
        Vertex { pos: p.to_array(), normal: n.to_array(), uv: [[0.,1.],[1.,1.],[1.,0.],[0.,0.]][i % 4] }
    }).collect();
    let id = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices, indices, material }]));
    scene.add_instance(id, Affine3A::IDENTITY);
    if foliage { scene.instances.last_mut().unwrap().flags |= FOLIAGE; }
}

fn scene(shear: Vec2) -> Scene {
    let mut s = Scene::default();
    // A coloured UV ramp catches stale triangle reconstruction even when the
    // silhouette itself moves correctly. No alpha noise or temporal resolve.
    let texture = image::RgbaImage::from_fn(32, 32, |x,y|
        image::Rgba([32 + (x * 6) as u8, 40 + (y * 5) as u8, 70, 255]));
    let layer = s.add_image("wind UV reference", texture);
    let material = s.add_material(Material { tint: [1.;3], layer, roughness: 1.,
        flags: TWO_SIDED, scale: 1., metal: 0. });
    mesh(&mut s, &[Vec3::new(-1.5,0.,0.), Vec3::new(1.5,0.,0.),
        Vec3::new(1.5,10.,0.), Vec3::new(-1.5,10.,0.)], Vec3::Z,
        &[0,1,2,0,2,3], material, shear, true);
    let ground = s.add_material(Material { tint: [0.35;3], layer: -1,
        roughness: 1., flags: 0, scale: 1., metal: 0. });
    mesh(&mut s, &[Vec3::new(-18.,-0.02,-18.), Vec3::new(-18.,-0.02,18.),
        Vec3::new(18.,-0.02,18.), Vec3::new(18.,-0.02,-18.)], Vec3::Y,
        &[0,1,2,0,2,3], ground, Vec2::ZERO, false);
    s
}

fn camera() -> Camera {
    Camera { pos: Vec3::new(12.,11.,21.), target: Vec3::new(0.,4.,0.),
        fov_y: 42f32.to_radians(), near: 0.1 }
}

fn error(a: &[u8], b: &[u8]) -> (f64, f64) {
    assert_eq!(a.len(),b.len());
    let mut total = 0u64;
    let mut bad = 0usize;
    for (a,b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let differences = [a[0].abs_diff(b[0]),a[1].abs_diff(b[1]),a[2].abs_diff(b[2])];
        total += differences.iter().map(|&v|v as u64).sum::<u64>();
        if differences.iter().any(|&v|v > 8) { bad += 1; }
    }
    (total as f64 / (a.len()/4*3) as f64 / 255., bad as f64 / (a.len()/4) as f64)
}

fn run() -> Result<(),String> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(args.windows(2).find(|a|a[0]=="--out")
        .ok_or("use --out FRESH_DIRECTORY")?[1].clone());
    if out.exists() { return Err("output directory must be new".into()); }
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let gpu = Gpu::new()?;
    let mut light = Lighting { taa:0., gi:0., contact_occlusion:0., reflection_occlusion:0.,
        fog_start:10000., fog_end:20000., fog_density:0., lod_pixels:0.,
        sun_dir:Vec3::new(0.6,0.8,0.25).normalize(), ..Lighting::default() };
    let rest = scene(Vec2::ZERO);
    let renderer = Renderer::new(&gpu,&rest,&light);
    let mut records = Vec::new();
    let mut failures = Vec::new();
    let mut movement = 0.;
    for debug in [2.,3.] {
        light.debug=debug;
        renderer.set_wind(&gpu,&WindState::default())?;
        let (still,_) = renderer.render(&gpu,&camera(),&light,SIZE,SIZE);
        for time in [0.,2.75,11.5] {
            let state = WindState { time_seconds:time, direction:Vec2::new(0.8,0.6),
                strength:1., ..WindState::default() };
            let shear = state.sample_shear(Vec3::ZERO,false);
            renderer.set_wind(&gpu,&state)?;
            let (actual,_) = renderer.render(&gpu,&camera(),&light,SIZE,SIZE);
            let reference_scene=scene(shear);
            let reference_renderer=Renderer::new(&gpu,&reference_scene,&light);
            let (expected,_) = reference_renderer.render(&gpu,&camera(),&light,SIZE,SIZE);
            let (mean,bad) = error(&actual,&expected);
            movement += error(&actual,&still).0;
            if mean>0.003 || bad>0.003 {failures.push(format!("debug{debug} time{time}: mean{mean} bad{bad}"));}
            render::write_png(&out.join(format!("live-debug{debug}-time{time}.png")),SIZE,SIZE,&actual)?;
            render::write_png(&out.join(format!("reference-debug{debug}-time{time}.png")),SIZE,SIZE,&expected)?;
            records.push(format!("{{\"debug\":{debug},\"time\":{time},\"shear\":{:?},\"mean_rgb_error\":{mean},\"pixels_over_8_fraction\":{bad}}}",shear.to_array()));
        }
        renderer.set_wind(&gpu,&WindState::default())?;
        let (off,_) = renderer.render(&gpu,&camera(),&light,SIZE,SIZE);
        if off!=still { failures.push(format!("debug{debug}: disabling wind did not restore original frame")); }
    }
    if movement<0.0001 { failures.push("wind did not visibly move the test plant".into()); }
    // Identical physical camera/pixel dimensions must use the same wind in
    // one or four panels, independent of camera identity or view order.
    light.debug=2.;
    renderer.set_wind(&gpu,&WindState {time_seconds:2.75,strength:1.,..WindState::default()})?;
    let (single,_) = renderer.render(&gpu,&camera(),&light,SIZE,SIZE);
    let views: Vec<_> = (0..4).map(|i|ViewRect {cam:camera(),x:(i%2)*SIZE,y:(i/2)*SIZE,w:SIZE,h:SIZE}).collect();
    let (split,_) = renderer.render_views(&gpu,&views,&light,SIZE*2,SIZE*2,&[]);
    for view in &views {
        let mut panel=Vec::new();
        for y in view.y..view.y+SIZE {
            let start=((y*SIZE*2+view.x)*4) as usize;
            panel.extend_from_slice(&split[start..start+(SIZE*4) as usize]);
        }
        if panel!=single { failures.push(format!("split view {},{} differs from single camera",view.x,view.y)); }
    }
    render::write_png(&out.join("four-phase.png"),SIZE*2,SIZE*2,&split)?;
    let report=format!("{{\"schema\":1,\"scope\":\"synthetic GPU affine deformation differential, not natural motion acceptance or performance\",\"cases\":[{}],\"movement_error_sum\":{movement},\"failures\":{:?},\"passed\":{}}}",records.join(","),failures,failures.is_empty());
    std::fs::write(out.join("report.json"),report).map_err(|e|e.to_string())?;
    println!("wind differential: {} cases, {} failures",records.len(),failures.len());
    if !failures.is_empty(){return Err(failures.join("\n"));}
    Ok(())
}

fn main() { if let Err(e)=run(){ eprintln!("{e}");std::process::exit(1); } }
