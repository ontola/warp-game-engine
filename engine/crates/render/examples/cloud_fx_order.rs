//! Compare the same sky pixel with an explosion plume in front of and behind cloud.
use glam::Vec3;
use render::fx::Puff;
use render::sky_volume::{Clouds, SkyVolume};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::path::Path;

const W: u32 = 320;
const H: u32 = 200;
fn plume(foot: Vec3, side: f32) -> Puff {
    Puff {
        center_kind: foot.extend(7.).to_array(),
        ax: [side, 0., 0., 0.],
        ay: [0., side, 0., 0.],
        az: [0., 0., side, 0.],
        body: [0.08, 0.07, 0.06, 14.],
        timing: [0., 4.6, 0., 0.],
        ..Default::default()
    }
}
fn run() -> Result<(), String> {
    let dir = std::env::args().nth(1);
    let gpu = Gpu::new()?;
    let mut scene = Scene::default();
    scene.sky = Some((16, 8, vec![[0.16, 0.36, 0.68, 1.]; 128]));
    let material = scene.add_material(render::scene::Material {
        tint: [0.4; 3],
        layer: -1,
        roughness: 1.,
        flags: 0,
        scale: 1.,
        metal: 0.,
    });
    let floor = scene.add_mesh(&render::primitives::cube(material).mesh());
    scene.add_instance(
        floor,
        glam::Affine3A::from_translation(Vec3::new(0., -10., 0.)),
    );
    let eye = Vec3::new(0., 8., 0.);
    let ray = Vec3::new(0., 0.72, -1.).normalize();
    let view = ViewRect {
        cam: Camera {
            pos: eye,
            target: eye + ray,
            fov_y: 0.7,
            near: 0.05,
        },
        x: 0,
        y: 0,
        w: W,
        h: H,
    };
    let cloud = Clouds {
        coverage: 1.,
        base_m: 160.,
        thickness_m: 900.,
        density: 0.6,
        shadow: 0.,
        wind: Default::default(),
        ..Default::default()
    };
    let mut shots = Vec::new();
    for (name, cloudy, distance) in [
        ("clear", false, 0.),
        ("clear_near", false, 18.),
        ("clear_far", false, 3200.),
        ("cloud", true, 0.),
        ("cloud_near", true, 18.),
        ("cloud_far", true, 3200.),
    ] {
        let light = Lighting {
            sky_volume: cloudy.then_some(SkyVolume {
                aerial: false,
                clouds: Some(cloud),
                ..Default::default()
            }),
            taa: 0.,
            sharpen: 0.,
            fog_density: 0.,
            fog_start: 20000.,
            fog_end: 40000.,
            ..Default::default()
        };
        let renderer = Renderer::new(&gpu, &scene, &light);
        renderer.set_debug_options(render::debug_settings::DebugRenderOptions {
            bloom_enabled: false,
            ..Default::default()
        })?;
        let size = distance * 0.7;
        let puffs = if distance > 0. {
            vec![plume(eye + ray * distance - Vec3::Y * size * 0.52, size)]
        } else {
            vec![]
        };
        renderer.set_effects(&gpu, 0.85, Some(&puffs))?;
        let (image, _) = renderer.try_render_views(&gpu, &[view], &light, W, H, &[])?;
        if let Some(dir) = &dir {
            render::write_png(&Path::new(dir).join(format!("{name}.png")), W, H, &image)?;
        }
        shots.push(image);
    }
    // Only count pixels both covered by the plume in clear weather and
    // changed by the cloud. An empty sky or a misplaced plume cannot pass.
    let difference = |a: usize, b: usize, i: usize| -> f32 {
        (0..3)
            .map(|k| f32::from(shots[a][i + k].abs_diff(shots[b][i + k])))
            .sum::<f32>()
            / 3.
    };
    let mut n = 0;
    let mut near = 0.;
    let mut far = 0.;
    for i in (0..shots[0].len()).step_by(4) {
        if difference(0, 1, i) > 12. && difference(0, 2, i) > 12. && difference(0, 3, i) > 20. {
            n += 1;
            near += difference(3, 4, i);
            far += difference(3, 5, i);
        }
    }
    println!(
        "Cloud/plume overlap {n} px; mean cloud-to-near plume {:.2}, cloud-to-far plume {:.2} RGB8 levels",
        near / (n.max(1) as f32),
        far / (n.max(1) as f32)
    );
    if n < 500 || near < 12. * n as f32 || near < far * 3. {
        return Err("clouds did not distinguish foreground and distant smoke".into());
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
