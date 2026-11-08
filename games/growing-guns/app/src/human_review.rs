//! Two simultaneous viewpoints on one stadium population, sampled at the same presentation time.
use crate::{Args, stadium};
use glam::Vec3;
use render::{
    Camera, Gpu,
    humans::{Constraints, Far, Instance, ReviewRenderer},
};
use std::{path::Path, time::Instant};

pub fn run(args: &Args) -> Result<(), String> {
    let seed = args.num("--seed", 52u64);
    let far = match args.value("--far").unwrap_or("cells") {
        "cells" => Far::Cells,
        "mesh" => Far::Mesh,
        "impostor" => Far::Impostor,
        _ => return Err("--far cells|mesh|impostor".into()),
    };
    let out = Path::new(args.value("--out").unwrap_or("build/human-review"));
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let (w, h) = args.size((960, 540));
    let frames = args.num("--frames", 121usize);
    if !(128..=4096).contains(&w) || !(128..=4096).contains(&h) || !(9..=3600).contains(&frames) {
        return Err("size per view must be 128..4096, frames 9..3600".into());
    }
    let world = growing_guns::World::new(seed, &[true; 4]);
    let seats = stadium::review_seats(&world);
    let target = seats.first().ok_or("empty stadium")?;
    let mut instances = Vec::new();
    let floor = 2.02 * world.arena.stadium.inner_r;
    instances.push(Instance::stand(
        Vec3::new(0., -0.4, 0.),
        0.,
        Vec3::new(floor, 0.8, floor),
        [0.32, 0.42, 0.32],
    ));
    for (i, p) in world.arena.stadium.pieces().into_iter().enumerate() {
        let t = (i % 5) as f32 * 0.008;
        instances.push(Instance::stand(
            p.center,
            p.yaw,
            p.size,
            [0.45 + t, 0.412 + t, 0.36 + t],
        ));
    }
    for (i, s) in seats.iter().enumerate() {
        let rules = Constraints {
            height_m: Some(s.scale),
            shirt: None,
        };
        instances.push(Instance::person(
            seed.wrapping_mul(0x9e3779b97f4a7c15) ^ i as u64,
            s.pos,
            s.yaw,
            &rules,
        ));
    }
    let gpu = Gpu::new()?;
    let mut renderer = ReviewRenderer::new(&gpu, &instances, far)?;
    let initial_bytes = renderer.resident_buffer_bytes;
    let texture = |format, usage| {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
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
    let col = texture(
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let front = Vec3::new(target.yaw.sin(), 0., target.yaw.cos());
    let face = target.pos + Vec3::Y * target.scale * 0.926;
    let radius = world.arena.stadium.wall_r();
    let wide = Camera {
        pos: front * (radius * 1.60) + Vec3::Y * (radius * 1.45),
        target: Vec3::Y * 10.,
        near: 0.04,
        fov_y: 58f32.to_radians(),
    };
    let view_at = |zoom: f32| {
        let distance = (radius * 2.3).powf(1. - zoom) * 0.55f32.powf(zoom);
        let elevation = 0.80 * (1. - zoom);
        let view_dir = (front + Vec3::Y * elevation + Vec3::Y.cross(front) * 0.19).normalize();
        let aim = wide.target.lerp(face, (zoom * 2.5).min(1.0));
        Camera {
            pos: aim + view_dir * distance,
            target: aim,
            ..wide
        }
    };
    let mut samples = Vec::new();
    let mut cold = None;
    let mut request = None;
    for _ in 0..5 {
        renderer.draw(&gpu, &wide, 0., &col, &depth)?;
    }
    for i in 0..frames {
        let t = i as f32 / (frames - 1) as f32;
        let zoom = 0.5 - 0.5 * (std::f32::consts::TAU * t).cos();
        let mut images = Vec::new();
        let mut views = Vec::new();
        let capture =
            args.flag("--sequence") || [0, frames / 4, frames / 2, frames - 1].contains(&i);
        for cam in [view_at(zoom), wide] {
            let s = renderer.draw(&gpu, &cam, t * 5., &col, &depth)?;
            if s.counts[2] > 0 && !renderer.detail_ready && request.is_none() {
                request = Some((i, Instant::now()));
            }
            if renderer.detail_ready && cold.is_none() && s.counts[2] > 0 {
                if let Some((start, clock)) = request {
                    cold = Some(
                        serde_json::json!({"requested_frame":start,"first_detailed_frame":i,"frames":i-start,"wall_ms":clock.elapsed().as_secs_f64()*1000.}),
                    );
                }
            }
            views.push(serde_json::json!({"far":s.counts[0],"medium":s.counts[1],"close":s.counts[2],"gpu_ms":s.gpu_ms}));
            if capture {
                images.push(gpu.read_rgba8(&col));
            }
        }
        samples.push(serde_json::json!({"frame":i,"views":views,"detail_resident":renderer.detail_ready,"buffer_bytes":renderer.resident_buffer_bytes}));
        if request.is_some() && !renderer.detail_ready {
            renderer.load_detail(&gpu);
        }
        if capture {
            let mut pixels = Vec::with_capacity((w * h * 8) as usize);
            for y in 0..h as usize {
                for img in &images {
                    let offset = y * w as usize * 4;
                    pixels.extend_from_slice(&img[offset..offset + w as usize * 4]);
                }
            }
            render::write_png(&out.join(format!("frame-{i:04}.png")), 2 * w, h, &pixels)?;
        }
    }
    let report = serde_json::json!({"seed":seed,"target_seat":0,"population":seats.len(),"adapter":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),
        "view_size":[w,h],"far":format!("{far:?}"),"initial_buffer_bytes":initial_bytes,"resident_buffer_bytes":renderer.resident_buffer_bytes,
        "target_bytes":w as u64*h as u64*8,"cold_detail":cold,"samples":samples});
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
