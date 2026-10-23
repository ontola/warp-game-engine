//! Bounded offscreen regression for the real shared canopy lighting shader.
//! Run only when the GPU is available:
//! cargo run --release -p render --example canopy_probe -- --out FRESH_DIRECTORY
//!
//! These synthetic receivers test missing coverage, origin changes and split
//! views, not the visual fidelity of a particular tree asset. Measurements are
//! displayed RGB after the actual diagnostic shader and tone mapper; they are
//! deliberately not reported as linear transmittance or gameplay performance.

use glam::{Affine3A, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::Material;
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::path::{Path, PathBuf};
use std::time::Instant;

const SIDE: u32 = 129; // An exact centre pixel, including each split panel.
const MODES: [u32; 2] = [1, 3]; // Shared sky visibility and direct sunlight.
const MOTION: [f32; 9] = [136., 144., 148., 150., 152., 154., 156., 160., 168.];
const SPREAD: [f32; 5] = [162., 163., 164., 165., 166.];

#[derive(Clone)]
struct Case {
    name: String,
    owners: Vec<usize>,
    cameras: Vec<Camera>,
}

#[derive(Clone)]
struct Measurement {
    forest: bool,
    name: String,
    debug: u32,
    owners: Vec<usize>,
    rgb: Vec<[u8; 3]>,
    camera_positions: Vec<[f32; 3]>,
    camera_targets: Vec<[f32; 3]>,
    wall_ms: f64,
}

fn sites() -> [Vec3; 6] {
    [Vec3::ZERO, Vec3::new(0., 200., -500.),
        Vec3::new(-500., 0., -500.), Vec3::new(500., 0., -500.),
        Vec3::new(-500., 0., 500.), Vec3::new(500., 0., 500.)]
}

fn camera(site: Vec3, distance: f32) -> Camera {
    Camera { pos: site + Vec3::new(0., 4., distance), target: site,
        fov_y: 42f32.to_radians(), near: 0.1 }
}

fn cases() -> Vec<Case> {
    let s = sites();
    let mut result = [12., 100., 300.].into_iter().map(|d| Case {
        name: format!("distance-{d:.0}"), owners: vec![0], cameras: vec![camera(s[0], d)],
    }).collect::<Vec<_>>();
    let mut high = camera(s[1], 300.);
    high.pos.y = 15.; // Crown Y~220 is outside the former cameraY+42 cap.
    result.push(Case { name: "elevated-y200".into(), owners: vec![1], cameras: vec![high] });
    for owner in 2..6 {
        result.push(Case { name: format!("isolated-{owner}"), owners: vec![owner],
            cameras: vec![camera(s[owner], 12.)] });
    }
    for (name, owners) in [("four-separated", vec![2, 3, 4, 5]),
                           ("four-reordered", vec![5, 4, 3, 2])] {
        let cameras = owners.iter().map(|&i| camera(s[i], 12.)).collect();
        result.push(Case { name: name.into(), owners, cameras });
    }
    result
}

fn material(scene: &mut Scene, tint: [f32; 3]) -> u32 {
    scene.add_material(Material { tint, layer: -1, roughness: 1., flags: 0, scale: 1., metal: 0. })
}

fn scene(forest: bool) -> Scene {
    let mut scene = Scene::default();
    let receiver_material = material(&mut scene, [0.3; 3]);
    let n = Vec3::new(0., 0.6, 1.).normalize();
    let x = Vec3::X * 14.;
    let t = Vec3::new(0., 1., -0.6).normalize() * 14.;
    let vertices = [-x-t, x-t, x+t, -x+t].map(|p| Vertex {
        pos: p.to_array(), normal: n.to_array(), uv: [0.; 2],
    });
    let receiver = scene.add_mesh(&ClusterMesh::build(&[Part {
        vertices: &vertices, indices: &[0, 1, 2, 0, 2, 3], material: receiver_material,
    }]));
    // The first six instance IDs are stable, measurable receiver surfaces.
    for position in sites() { scene.add_instance(receiver, Affine3A::from_translation(position)); }

    if forest {
        let crown_material = material(&mut scene, [0.05, 0.12, 0.025]);
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        // A simple opaque box crown, instanced into a broad synthetic canopy.
        // The visible geometry casts the same sun shadows as the FOLIAGE proxy.
        for (normal, tangent, bitangent, half) in [
            (Vec3::X, Vec3::NEG_Z, Vec3::Y, Vec3::new(6., 6., 8.)),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y, Vec3::new(6., 6., 8.)),
            (Vec3::Y, Vec3::X, Vec3::NEG_Z, Vec3::new(8., 6., 6.)),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z, Vec3::new(8., 6., 6.)),
            (Vec3::Z, Vec3::X, Vec3::Y, Vec3::new(6., 6., 8.)),
            (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y, Vec3::new(6., 6., 8.)),
        ] {
            let centre = normal * half.x;
            let a = tangent * half.y;
            let b = bitangent * half.z;
            let start = vertices.len() as u32;
            for p in [centre-a-b, centre+a-b, centre+a+b, centre-a+b] {
                vertices.push(Vertex { pos: p.to_array(), normal: normal.to_array(), uv: [0.; 2] });
            }
            indices.extend_from_slice(&[start, start+1, start+2, start, start+2, start+3]);
        }
        let crown = scene.add_mesh(&ClusterMesh::build(&[Part {
            vertices: &vertices, indices: &indices, material: crown_material,
        }]));
        for site in sites() {
            for z in -6..=6 { for x in -6..=6 {
                scene.add_instance(crown, Affine3A::from_translation(
                    site + Vec3::new(x as f32 * 8., 20., z as f32 * 8.)));
                scene.mark_foliage();
            }}
        }
    }
    scene
}

fn light(debug: u32) -> Lighting {
    let mut light = Lighting::default();
    light.debug = debug as f32;
    light.sun_dir = Vec3::new(0., 0.6, 0.8);
    light.taa = 0.;
    light.sharpen = 0.;
    light.lod_pixels = 0.1;
    light.contact_occlusion = 0.;
    light.reflection_occlusion = 0.;
    light.fog_start = 1e8;
    light.fog_end = 1e9;
    light.fog_density = 0.;
    // Hero shadow maps around the receiver target could otherwise hide the
    // missing far canopy even while the camera is hundreds of metres away.
    light.near_shadows = 0.;
    light
}

fn capture(gpu: &Gpu, renderer: &Renderer, out: &Path, case: &Case,
           forest: bool, debug: u32) -> Result<Measurement, String> {
    let (w, h) = if case.cameras.len() == 1 { (SIDE, SIDE) } else { (SIDE*2+1, SIDE*2+1) };
    let rects = render::split_rects(case.cameras.len(), w, h);
    let views = case.cameras.iter().zip(&rects).map(|(&cam, &(x,y,w,h))|
        ViewRect { cam, x, y, w, h }).collect::<Vec<_>>();
    if rects.iter().any(|r| r.2 != SIDE || r.3 != SIDE) {
        return Err("fixture expected an odd, identical-size pixel grid in every view".into());
    }
    let picks = rects.iter().map(|r| (r.0+r.2/2, r.1+r.3/2)).collect::<Vec<_>>();
    let started = Instant::now();
    let (pixels, stats) = renderer.render_views(gpu, &views, &light(debug), w, h, &picks);
    let wall_ms = started.elapsed().as_secs_f64()*1000.;
    if pixels.len() != w as usize*h as usize*4 || stats.picks.len() != picks.len() {
        return Err("unexpected actual render/readback dimensions".into());
    }
    let mut rgb = Vec::new();
    for (i, (&(x,y), &expected)) in picks.iter().zip(&case.owners).enumerate() {
        if stats.picks[i].map(|p| p.0) != Some(expected as u32) {
            return Err(format!("{} debug{debug}: view{i} centre hit {:?}, expected receiver {expected}",
                case.name, stats.picks[i]));
        }
        let start = ((y*w+x)*4) as usize;
        rgb.push([pixels[start], pixels[start+1], pixels[start+2]]);
    }
    let prefix = if forest { "forest" } else { "clear" };
    render::write_png(&out.join(format!("{prefix}-{}-debug{debug}.png", case.name)), w, h, &pixels)?;
    Ok(Measurement { forest, name: case.name.clone(), debug, owners: case.owners.clone(), rgb,
        camera_positions: case.cameras.iter().map(|c| c.pos.to_array()).collect(),
        camera_targets: case.cameras.iter().map(|c| c.target.to_array()).collect(), wall_ms })
}

fn luma(rgb: [u8; 3]) -> f64 {
    (rgb[0] as f64*0.2126 + rgb[1] as f64*0.7152 + rgb[2] as f64*0.0722)/255.
}

fn quote(s: &str) -> String {
    let mut result = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""), '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"), '\r' => result.push_str("\\r"), '\t' => result.push_str("\\t"),
            c if c.is_control() => result.push_str(&format!("\\u{:04x}", c as u32)),
            c => result.push(c),
        }
    }
    result.push('"'); result
}

fn record(m: &Measurement) -> String {
    format!("{{\"scene\":{},\"case\":{},\"debug\":{},\"owners\":{:?},\"rgb8\":{:?},\"display_luma\":{:?},\"camera_positions\":{:?},\"camera_targets\":{:?},\"wall_ms\":{:.5}}}",
        quote(if m.forest { "forest" } else { "clear" }), quote(&m.name), m.debug,
        m.owners, m.rgb, m.rgb.iter().map(|&c|luma(c)).collect::<Vec<_>>(),
        m.camera_positions, m.camera_targets, m.wall_ms)
}

fn verify(records: &[Measurement]) -> Vec<String> {
    let get = |forest, name: &str, debug| records.iter().find(|m|
        m.forest == forest && m.name == name && m.debug == debug).unwrap();
    let mut errors = Vec::new();
    for debug in MODES {
        for case in cases() {
            let a = get(true, &case.name, debug);
            let b = get(false, &case.name, debug);
            for (view, (&forest, &clear)) in a.rgb.iter().zip(&b.rgb).enumerate() {
                if luma(clear) < 0.1 || luma(clear)-luma(forest) < 0.025 {
                    errors.push(format!("{} debug{debug} view{view}: canopy did not measurably darken the matched clear receiver ({:.4} -> {:.4})", case.name, luma(clear), luma(forest)));
                }
            }
        }
        let separated = get(true, "four-separated", debug);
        let reversed = get(true, "four-reordered", debug);
        for (i, &owner) in separated.owners.iter().enumerate() {
            let j = reversed.owners.iter().position(|&id|id == owner).unwrap();
            if (luma(separated.rgb[i])-luma(reversed.rgb[j])).abs() > 2./255. {
                errors.push(format!("debug{debug} receiver{owner}: ordering the same cameras changed lighting"));
            }
            let isolated = get(true, &format!("isolated-{owner}"), debug);
            if (luma(separated.rgb[i])-luma(isolated.rgb[0])).abs() > 2./255. {
                errors.push(format!("debug{debug} receiver{owner}: identical camera in single/split differs by >2/255 displayed light"));
            }
        }
    }
    let motion = MOTION.iter().map(|d|get(true, &format!("motion-{d:.0}"), 1)).collect::<Vec<_>>();
    for pair in motion.windows(2) {
        let jump = (luma(pair[1].rgb[0])-luma(pair[0].rgb[0])).abs();
        if jump > 0.08 {
            errors.push(format!("{} -> {}: fixed-receiver displayed sky light jumped {jump:.5} (>0.08)", pair[0].name, pair[1].name));
        }
    }
    if get(true, "motion-168", 1).rgb != get(true, "stationary-repeat", 1).rgb {
        errors.push("unchanged camera/scene did not reproduce exact centre pixels".into());
    }
    let spread = SPREAD.iter().map(|d|get(true, &format!("spread-{d:.0}"), 1)).collect::<Vec<_>>();
    for pair in spread.windows(2) {
        for (view, (&a,&b)) in pair[0].rgb.iter().zip(&pair[1].rgb).enumerate() {
            let jump = (luma(a)-luma(b)).abs();
            if jump > 2./255. {
                errors.push(format!("{} -> {} view{view}: separating players changed fixed-receiver sky light by {jump:.6} (>2/255)", pair[0].name, pair[1].name));
            }
        }
    }
    errors
}

fn run(out: PathBuf) -> Result<(), String> {
    if out.exists() { return Err(format!("use a fresh output directory: {}", out.display())); }
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let started = Instant::now();
    let gpu = Gpu::new()?;
    let mut records = Vec::new();
    for forest in [false, true] {
        let scene = scene(forest);
        for debug in MODES {
            // The shared shadow extent grows monotonically. Reusing the sky
            // run's four-player extent here would let the direct-light test
            // use a huge detailed shadow map even at300m, hiding a broken
            // canopy fallback. Each component starts with its own renderer.
            let renderer = Renderer::new(&gpu, &scene, &light(debug));
            for case in cases() {
                records.push(capture(&gpu, &renderer, &out, &case, forest, debug)?);
            }
            if forest && debug == 1 {
                for distance in MOTION {
                    let case = Case { name: format!("motion-{distance:.0}"), owners: vec![0],
                        cameras: vec![camera(sites()[0], distance)] };
                    records.push(capture(&gpu, &renderer, &out, &case, true, 1)?);
                }
                let repeat = Case { name: "stationary-repeat".into(), owners: vec![0],
                    cameras: vec![camera(sites()[0], *MOTION.last().unwrap())] };
                records.push(capture(&gpu, &renderer, &out, &repeat, true, 1)?);
            }
        }
        if forest {
            // A fresh renderer exposes the former shared24->48m transition.
            // Per-view fields must keep lighting stable as other players move;
            // this is now a gating regression, not an informational warning.
            let renderer = Renderer::new(&gpu, &scene, &light(1));
            for spread in SPREAD {
                let target = sites()[1];
                let cameras = [(-spread,300.), (spread,300.), (-spread,320.), (spread,320.)]
                    .map(|(x,z)| Camera { pos: Vec3::new(target.x+x, 15., target.z+z),
                        target, fov_y: 42f32.to_radians(), near: 0.1 }).to_vec();
                let case = Case { name: format!("spread-{spread:.0}"),
                    owners: vec![1;4], cameras };
                records.push(capture(&gpu, &renderer, &out, &case, true, 1)?);
            }
        }
    }
    let errors = verify(&records);
    let spread_records = records.iter().filter(|m|m.name.starts_with("spread-")).collect::<Vec<_>>();
    let spread_max_jump = spread_records.windows(2).flat_map(|pair|
        pair[0].rgb.iter().zip(&pair[1].rgb).map(|(&a,&b)|(luma(a)-luma(b)).abs()))
        .fold(0.0, f64::max);
    let report = format!("{{\n\"schema\":2,\"adapter\":{},\"passed\":{},\"purpose\":\"synthetic actual-GPU canopy coverage and camera continuity; not asset parity or gameplay FPS\",\"measurement\":\"centre pixel must hit the same named receiver; displayed tone-mapped RGB, not linear transmittance\",\"independent_renderer_per_component\":true,\"near_shadows\":false,\"taa\":false,\"fog\":false,\"source_canopies\":1014,\"sun_direction\":[0.0,0.6,0.8],\"receiver_centres\":{:?},\"limits\":{{\"minimum_display_darkening\":0.025,\"maximum_motion_sky_jump\":0.08,\"maximum_single_split_delta\":0.00784314,\"maximum_order_delta\":0.00784314,\"maximum_spread_jump\":0.00784314}},\"per_view_spread_stability\":{{\"maximum_display_jump\":{:.6},\"included_in_pass\":true,\"note\":\"fresh renderer; camera halfspread162..166m crosses the former shared24-to48m pitch threshold; per-view fields must remain independent of other players\"}},\"total_wall_seconds\":{:.5},\"records\":[\n{}\n],\"failures\":[{}]\n}}\n",
        quote(&gpu.info.name), errors.is_empty(), sites().map(|p|p.to_array()), spread_max_jump, started.elapsed().as_secs_f64(),
        records.iter().map(record).collect::<Vec<_>>().join(",\n"),
        errors.iter().map(|s|quote(s)).collect::<Vec<_>>().join(","));
    std::fs::write(out.join("report.json"), report).map_err(|e|e.to_string())?;
    println!("canopy_probe: {} frames, {} failures; {}", records.len(), errors.len(), out.display());
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = if args.len() == 2 && args[0] == "--out" {
        run(PathBuf::from(&args[1]))
    } else { Err("usage: canopy_probe --out FRESH_DIRECTORY".into()) };
    if let Err(error) = result { eprintln!("canopy_probe: {error}"); std::process::exit(1); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_receiver_pixels_are_exactly_centre_aligned() {
        let rects = render::split_rects(4, SIDE*2+1, SIDE*2+1);
        assert!(rects.iter().all(|r|r.2 == SIDE && r.3 == SIDE));
        assert_eq!(cases().last().unwrap().owners, vec![5,4,3,2]);
    }
    #[test]
    fn receivers_face_every_requested_camera_and_motion_crosses_old_edge() {
        let normal = Vec3::new(0.,0.6,1.).normalize();
        for case in cases() { for cam in case.cameras {
            assert!((cam.pos-cam.target).dot(normal) > 0.);
        }}
        assert!(MOTION[0]-54. < 96. && MOTION.last().unwrap()-54. > 96.);
        assert!(MOTION.windows(2).all(|w|w[1] > w[0]));
    }
    #[test]
    fn report_quotes_do_not_break_json() {
        assert_eq!(quote("GPU \"name\"\\path\n"), "\"GPU \\\"name\\\"\\\\path\\n\"");
    }
}
