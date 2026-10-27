//! Exclusive-GPU local-light regression. Native held poses; not game FPS.
//! Run: local_lights_probe --out FRESH_DIRECTORY
use glam::{Affine3A, Mat4, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::headlights::{LocalLight, SpotLight};
use render::scene::{Material, TWO_SIDED};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const W: u32 = 640;
const H: u32 = 384;
// Close enough to resolve the intentionally weak 2.5 cd tail above 8-bit
// quantization at EV6, while remaining in every single-light inner cone.
const RECEIVER: Vec3 = Vec3::new(0.0, 0.0, 0.7);
const OUTSIDE_CONE: Vec3 = Vec3::new(2.8, 0.0, -0.3);
const OUTSIDE_RANGE: Vec3 = Vec3::new(0.0, 0.0, 7.0);
// Representative game tuning, not calibrated physical photometry.
const TAIL_INTENSITY_CD: f32 = 2.5;
const BRAKE_INTENSITY_CD: f32 = 32.0;
const TAIL_RANGE_M: f32 = 2.2;
const BRAKE_RANGE_M: f32 = 3.2;
const RED_INNER_DEGREES: f32 = 30.0;
const RED_OUTER_DEGREES: f32 = 78.0;
const REAR_DOWN_DEGREES: f32 = 10.0;

fn camera() -> Camera {
    Camera { pos: Vec3::new(6.0, 7.0, 10.0), target: Vec3::new(0.0, 0.0, 1.0),
        fov_y: 60f32.to_radians(), near: 0.1 }
}
fn quad(scene: &mut Scene, points: [[f32; 3]; 4], normal: Vec3, material: u32) {
    let vertices: Vec<_> = points.into_iter().zip([[0., 0.], [1., 0.], [1., 1.], [0., 1.]])
        .map(|(pos, uv)| Vertex { pos, normal: normal.to_array(), uv }).collect();
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices,
        indices: &[0, 1, 2, 0, 2, 3], material }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
}
fn scene() -> Scene {
    let mut scene = Scene::default();
    scene.sky = Some((16, 8, vec![[0.0, 0.0, 0.0, 1.0]; 128]));
    scene.sky_physical = true;
    let material = scene.add_material(Material { tint: [0.45; 3], layer: -1, roughness: 0.8,
        flags: TWO_SIDED, scale: 1.0, metal: 0.0 });
    quad(&mut scene, [[-10., 0., -10.], [-10., 0., 10.], [10., 0., 10.], [10., 0., -10.]], Vec3::Y, material);
    // A closed car-sized body provides real raster/depth geometry. Local
    // spill is deliberately unshadowed; this fixture does not claim otherwise.
    let (x0, x1, y0, y1, z0, z1) = (-0.8, 0.8, 0.1, 1.1, -2.0, -0.15);
    for (p, n) in [
        ([[x0,y0,z1],[x1,y0,z1],[x1,y1,z1],[x0,y1,z1]],Vec3::Z),
        ([[x1,y0,z0],[x0,y0,z0],[x0,y1,z0],[x1,y1,z0]],Vec3::NEG_Z),
        ([[x1,y0,z1],[x1,y0,z0],[x1,y1,z0],[x1,y1,z1]],Vec3::X),
        ([[x0,y0,z0],[x0,y0,z1],[x0,y1,z1],[x0,y1,z0]],Vec3::NEG_X),
        ([[x0,y1,z1],[x1,y1,z1],[x1,y1,z0],[x0,y1,z0]],Vec3::Y),
        ([[x0,y0,z0],[x1,y0,z0],[x1,y0,z1],[x0,y0,z1]],Vec3::NEG_Y),
    ] { quad(&mut scene, p, n, material); }
    scene
}
fn spill(intensity_cd: Vec3, range_m: f32) -> LocalLight {
    LocalLight { position: Vec3::new(0.0, 0.45, 0.08),
        direction: Vec3::new(0.0, -REAR_DOWN_DEGREES.to_radians().sin(), REAR_DOWN_DEGREES.to_radians().cos()),
        intensity_cd, range_m, inner_cos: 40f32.to_radians().cos(),
        outer_cos: 70f32.to_radians().cos(), source_radius_m: 0.06, falloff: 2.0 }
}
fn red_spill(brake: f32) -> LocalLight {
    let intensity = TAIL_INTENSITY_CD + (BRAKE_INTENSITY_CD - TAIL_INTENSITY_CD) * brake;
    let range = TAIL_RANGE_M + (BRAKE_RANGE_M - TAIL_RANGE_M) * brake;
    LocalLight { inner_cos: RED_INNER_DEGREES.to_radians().cos(),
        outer_cos: RED_OUTER_DEGREES.to_radians().cos(),
        ..spill(Vec3::new(intensity, intensity * 0.008, intensity * 0.003), range) }
}
fn cases() -> Vec<(&'static str, Vec<LocalLight>)> {
    let tail = red_spill(0.0);
    let brake = red_spill(1.0);
    let reverse = spill(Vec3::new(100.0, 94.0, 82.0), 5.0);
    let flash = LocalLight { direction: Vec3::new(0.0, -0.12, 1.0).normalize(),
        inner_cos: 45f32.to_radians().cos(), source_radius_m: 0.05,
        ..spill(Vec3::new(160.0, 36.0, 3.0), 2.0) };
    let mut max16 = Vec::new();
    for car in 0..4 {
        let x = (car as f32 - 1.5)*0.3;
        for (lamp, offset) in [(brake,-0.12),(brake,0.12),(reverse,0.0),(flash,0.0)] {
            max16.push(LocalLight { position: lamp.position + Vec3::X*(x+offset), ..lamp });
        }
    }
    let mut last_only=max16.clone();
    for lamp in last_only.iter_mut().take(15) {lamp.intensity_cd=Vec3::ZERO;}
    vec![("off-before",vec![]),("tail",vec![tail]),("brake",vec![brake]),
        ("reverse",vec![reverse]),("flash",vec![flash]),("max16",max16),
        ("last-slot-only",last_only),("off-after",vec![])]
}
fn pixel(p: Vec3) -> Result<[u32; 2], String> {
    let cam = camera();
    let vp = Mat4::perspective_infinite_reverse_rh(cam.fov_y, W as f32/H as f32, cam.near)
        * Mat4::look_at_rh(cam.pos,cam.target,Vec3::Y);
    let clip = vp*p.extend(1.0); let ndc = clip.truncate()/clip.w;
    let p = [(ndc.x*0.5+0.5)*W as f32,(0.5-ndc.y*0.5)*H as f32];
    if clip.w<=0.0 || p[0]<3.0 || p[1]<3.0 || p[0]>=W as f32-3.0 || p[1]>=H as f32-3.0 {
        return Err("fixture probe outside image".into());
    }
    Ok([p[0] as u32,p[1] as u32])
}
fn region(image: &[u8], p: [u32;2]) -> [f64;3] {
    let mut sum=[0.0;3];
    for y in p[1]-2..=p[1]+2 { for x in p[0]-2..=p[0]+2 {
        let at=((y*W+x)*4) as usize;
        for c in 0..3 {sum[c]+=f64::from(image[at+c])/25.0;}
    }} sum
}
fn region_changes(a: &[u8], b: &[u8], p: [u32;2]) -> usize {
    (p[1]-2..=p[1]+2).flat_map(|y|(p[0]-2..=p[0]+2).map(move |x|((y*W+x)*4)as usize))
        .filter(|&i|a[i..i+4]!=b[i..i+4]).count()
}
fn jpeg(path: &Path, rgba: &[u8]) -> Result<(),String> {
    let rgb:Vec<u8>=rgba.chunks_exact(4).flat_map(|p|p[..3].iter().copied()).collect();
    let file=std::fs::File::create(path).map_err(|e|e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(file,90)
        .encode(&rgb,W,H,image::ExtendedColorType::Rgb8).map_err(|e|e.to_string())
}
fn main() {if let Err(e)=run(){eprintln!("local lights probe: {e}");std::process::exit(1);}}
fn run() -> Result<(),String> {
    let args:Vec<_>=std::env::args().collect();
    let out=PathBuf::from(&args.windows(2).find(|w|w[0]=="--out").ok_or("--out FRESH_DIRECTORY required")?[1]);
    if out.exists(){return Err("output directory must be fresh".into());}
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let probes=[pixel(RECEIVER)?,pixel(OUTSIDE_CONE)?,pixel(OUTSIDE_RANGE)?];
    let light=Lighting {sun_lux:0.0,sky_lux:0.0,ev100:6.0,gi:0.0,contact_occlusion:0.0,
        reflection_occlusion:0.0,shadow_strength:0.0,near_shadows:0.0,fog_density:0.0,
        fog_start:1000.0,fog_end:2000.0,taa:0.0,sharpen:0.0,lod_pixels:0.0,..Default::default()};
    let headlight=SpotLight {position:Vec3::new(0.0,0.55,-2.05),direction:Vec3::new(0.0,-0.12,-1.0).normalize(),
        up:Vec3::Y,intensity_cd:Vec3::splat(8000.0),range_m:20.0,
        inner_angle_radians:17f32.to_radians(),outer_angle_radians:32f32.to_radians()};
    let gpu=Gpu::new()?;let renderer=Renderer::new(&gpu,&scene(),&light);
    renderer.set_headlights(&[headlight])?;
    let views=[ViewRect {cam:camera(),x:0,y:0,w:W,h:H}];
    let mut baseline=Vec::new();let mut reference_hashes=Vec::new();let mut reference_pairs=Vec::new();
    let mut reference_shadow_counts=None;let mut failures=Vec::new();let mut rows=Vec::new();
    let mut tail_red=0.0;let mut final_changed=0;
    for (sequence,(name,locals)) in cases().into_iter().enumerate() {
        renderer.set_gpu_profile_frame_id(100+sequence as u64);
        renderer.set_local_lights(&locals)?;
        let (rgba,stats)=renderer.try_render_views(&gpu,&views,&light,W,H,&[])?;
        let hashes=renderer.debug_headlight_depth_hashes(&gpu,Some(sequence as u64))?;
        let shadow=renderer.debug_headlight_culling(&gpu,Some(sequence as u64))?;
        if shadow.len()!=1 || hashes.len()!=1 {failures.push(format!("{name}: existing headlight must remain exactly one shadow pass"));}
        let mut pairs=shadow.first().map_or_else(Vec::new,|s|s.pairs.clone());pairs.sort_unstable();
        let counts=shadow.first().map(|s|(s.selected_triangles,s.submitted_vertices));
        for pass in &shadow {
            if pass.frame_id!=100+sequence as u64 || pass.render_sequence!=sequence as u64 {
                failures.push(format!("{name}: stale shadow diagnostic"));
            }
        }
        let receiver=region(&rgba,probes[0]);
        if sequence==0 {
            baseline=rgba.clone();reference_hashes=hashes.clone();reference_pairs=pairs.clone();reference_shadow_counts=counts;
            if pairs.is_empty(){failures.push("constant headlight did not draw any real shadow geometry".into());}
        }
        let changed=rgba.chunks_exact(4).zip(baseline.chunks_exact(4)).filter(|(a,b)|a!=b).count();
        let cone_changes=region_changes(&rgba,&baseline,probes[1]);
        let range_changes=region_changes(&rgba,&baseline,probes[2]);
        if cone_changes!=0 || range_changes!=0 {failures.push(format!("{name}: out-of-support pixels changed: cone={cone_changes}, range={range_changes}"));}
        if hashes!=reference_hashes || pairs!=reference_pairs || counts!=reference_shadow_counts {
            failures.push(format!("{name}: local lights changed the existing headlight depth or caster selection"));
        }
        let base=region(&baseline,probes[0]);
        let delta=std::array::from_fn::<_,3,_>(|c|receiver[c]-base[c]);
        match name {
            "tail"=>{tail_red=receiver[0];if delta[0]<2.0 || delta[0]<=delta[1]+1.0 || delta[0]<=delta[2]+1.0 {failures.push("tail did not produce red receiver illumination".into());}},
            "brake"=>if receiver[0]<=tail_red+2.0 {failures.push("braking did not brighten the red tail illumination".into());},
            "reverse"=>if delta.iter().any(|x|*x<2.0) || delta[2]<delta[0]*0.5 {failures.push("reverse light did not produce white receiver illumination".into());},
            "flash"=>if delta[0]<2.0 || delta[0]<=delta[1] || delta[1]<=delta[2] {failures.push("flash did not produce orange receiver illumination".into());},
            "max16"=>if locals.len()!=16 || changed==0 {failures.push("maximum16-light upload did not illuminate scene".into());},
            "last-slot-only"=>if locals.len()!=16 || delta[0]<2.0 || changed==0 {failures.push("local-light slot15 did not illuminate scene".into());},
            "off-after"=>{final_changed=changed;if changed!=0 {failures.push(format!("off-after retained illumination in {changed} pixels"));}},
            _=>{}
        }
        jpeg(&out.join(format!("{sequence:02}-{name}.jpg")),&rgba)?;
        rows.push(json!({"name":name,"frame_id":100+sequence,"render_sequence":sequence,"local_light_count":locals.len(),
            "lights":locals.iter().map(|l|json!({"position":l.position.to_array(),"direction":l.direction.to_array(),
                "intensity_cd":l.intensity_cd.to_array(),"range_m":l.range_m,"inner_cos":l.inner_cos,
                "outer_cos":l.outer_cos,"source_radius_m":l.source_radius_m})).collect::<Vec<_>>(),
            "receiver_mean_rgb_u8":receiver,"receiver_delta_rgb_u8":delta,"changed_pixels":changed,
            "outside_cone_changed_pixels":cone_changes,"outside_range_changed_pixels":range_changes,
            "headlight_shadow_passes":shadow.len(),"headlight_depth_sha256":hashes,"headlight_selected_pairs":pairs.len(),
            "headlight_selected_triangles_vertices":counts,"raw_rgba_sha256":format!("{:x}",Sha256::digest(&rgba)),
            "gpu_pass_ms":stats.gpu_ms}));
    }
    let report:Value=json!({"schema":1,"passed":failures.is_empty(),"failures":failures,"size":[W,H],
        "scope":"Eight held native frames; real local lighting and unchanged active headlight shadow. Not performance, moving gameplay or local-shadow validation.",
        "limits":["Local lights are unshadowed and can leak through occluders; this test does not claim occlusion.",
            "JPEGs are previews; assertions and hashes use original renderer RGBA pixels.",
            "No TAA/DLSS, grass, glass, fog scattering or dynamic event detection is exercised."],
        "rear_spill_tuning":{"tail_intensity_cd":TAIL_INTENSITY_CD,"brake_intensity_cd":BRAKE_INTENSITY_CD,
            "tail_range_m":TAIL_RANGE_M,"brake_range_m":BRAKE_RANGE_M,"range_interpolation":"linear brake 0..1",
            "red_inner_degrees":RED_INNER_DEGREES,"red_outer_degrees":RED_OUTER_DEGREES,
            "down_degrees":REAR_DOWN_DEGREES,"reverse_inner_degrees":40,"reverse_outer_degrees":70,
            "scope":"Representative game settings, not calibrated photometry; flash has separate cone/direction tuning."},
        "receiver_world":RECEIVER.to_array(),"outside_cone_world":OUTSIDE_CONE.to_array(),"outside_range_world":OUTSIDE_RANGE.to_array(),
        "probe_pixels":probes,"probe_neighborhood":"5x5 pixels","off_after_changed_pixels":final_changed,"cases":rows});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if failures.is_empty(){println!("local lights probe PASS: eight frames, maximum16 locals, unchanged headlight depth");Ok(())}
    else{Err(failures.join("\n"))}
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn red_spill_preserves_interpolated_range_and_no_forward_support() {
        let tail = red_spill(0.0);
        let half = red_spill(0.5);
        let brake = red_spill(1.0);
        assert_eq!(tail.intensity_cd.x, TAIL_INTENSITY_CD);
        assert_eq!(brake.intensity_cd.x, BRAKE_INTENSITY_CD);
        assert_eq!(tail.range_m, TAIL_RANGE_M);
        assert_eq!(brake.range_m, BRAKE_RANGE_M);
        assert!((half.range_m - 2.7).abs() < 1e-6);
        assert!((half.intensity_cd.x - 17.25).abs() < 1e-6);
        assert!(RED_OUTER_DEGREES + REAR_DOWN_DEGREES < 90.0);
        // The greatest dot product on the forward half-space z<=0 is
        // sin(down): even its boundary remains outside the red outer cone.
        assert!(tail.direction.y.abs() < tail.outer_cos);
        let flash = cases()[4].1[0];
        assert_eq!(flash.outer_cos, 70f32.to_radians().cos());
        assert_eq!(flash.direction, Vec3::new(0.0, -0.12, 1.0).normalize());
    }
    #[test] fn probe_geometry_and_maximum_light_configuration_are_well_inside_contract() {
        for p in [RECEIVER,OUTSIDE_CONE,OUTSIDE_RANGE] {pixel(p).unwrap();}
        let cases=cases();assert_eq!(cases.len(),8);assert_eq!(cases[5].1.len(),16);
        assert!(cases[6].1[..15].iter().all(|l|l.intensity_cd==Vec3::ZERO));
        assert!(cases[6].1[15].intensity_cd.max_element()>0.0);
        for (_,lights) in cases {
            render::headlights::validate_local(&lights).unwrap();
            let single_light = lights.len() == 1;
            for l in lights {
                let from=l.direction.normalize();let to=RECEIVER-l.position;
                assert!(to.length()<l.range_m*0.8 && from.dot(to.normalize())>l.outer_cos);
                // Single-light colour probes lie in the core. The outermost
                // lamps in the 16-light layout can lie in the cone penumbra.
                if single_light {assert!(from.dot(to.normalize())>l.inner_cos);}
                assert!(from.dot((OUTSIDE_CONE-l.position).normalize())<l.outer_cos-0.1);
                assert!(OUTSIDE_RANGE.distance(l.position)>l.range_m+1.0);
            }
        }
    }
}
