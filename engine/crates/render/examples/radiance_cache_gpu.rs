//! CPU one-bounce transport + actual GPU SH/moment sampling proof.
//! Exclusive GPU only: radiance_cache_gpu --out FRESH_DIRECTORY.
//! PNGs visualize a white diffuse receiver from GPU-computed irradiance. This
//! is neither production scene GI nor a GPU transport/performance benchmark.
use glam::Vec3;
use render::{
    Gpu,
    radiance_cache::{
        Config, DirectLight, RadianceCache, Ray, Revision, SurfaceHit, Trace, Transport,
    },
    radiance_cache_gpu::{GpuRadianceCache, Query, Readback},
};
use std::{path::PathBuf, time::Instant};
const W: u32 = 129;
const H: u32 = 97;
const CENTRE: Vec3 = Vec3::new(0.0, 4.0, 0.0);

#[derive(Clone, Copy)]
enum World {
    Floor,
    Roof,
    Colored,
    Sky,
    Unknown,
    Buried,
    Cavity,
}
impl Transport for World {
    fn trace(&self, ray: Ray) -> Trace {
        if matches!(self, Self::Unknown) {
            return Trace::Unknown;
        }
        if matches!(self, Self::Sky) {
            return Trace::KnownSky;
        }
        if matches!(self, Self::Buried | Self::Cavity) {
            // Actual sphere boundary, with an occupied interior (outward
            // normal) or an empty emissive cavity (inward normal).
            let q = ray.origin - CENTRE;
            let b = q.dot(ray.direction);
            let disc = b * b - q.length_squared() + 4.0;
            if disc < 0.0 {
                return Trace::KnownSky;
            }
            let roots = [-b - disc.sqrt(), -b + disc.sqrt()];
            if let Some(t) = roots
                .into_iter()
                .filter(|t| *t >= ray.min_distance)
                .min_by(f32::total_cmp)
            {
                if t > ray.max_distance {
                    return Trace::Unknown;
                }
                let n = (ray.origin + ray.direction * t - CENTRE).normalize();
                return Trace::Hit(SurfaceHit {
                    distance: t,
                    normal: if matches!(self, Self::Cavity) { -n } else { n },
                    albedo: Vec3::ZERO,
                    emission: if matches!(self, Self::Cavity) {
                        Vec3::new(1.0, 0.2, 0.05)
                    } else {
                        Vec3::splat(100.0)
                    },
                });
            }
            return Trace::KnownSky;
        }
        let plane = if matches!(self, Self::Roof) { 2.0 } else { 0.0 };
        if ray.direction.y.abs() > 1e-8 {
            let t = (plane - ray.origin.y) / ray.direction.y;
            if t >= ray.min_distance {
                if t > ray.max_distance {
                    return Trace::Unknown;
                }
                let p = ray.origin + ray.direction * t;
                let albedo = match self {
                    Self::Roof => Vec3::ZERO,
                    Self::Colored => {
                        if p.x < 0.0 {
                            Vec3::new(0.8, 0.02, 0.01)
                        } else {
                            Vec3::new(0.03, 0.6, 0.02)
                        }
                    }
                    _ => Vec3::X,
                };
                return Trace::Hit(SurfaceHit {
                    distance: t,
                    normal: if ray.direction.y < 0.0 {
                        Vec3::Y
                    } else {
                        Vec3::NEG_Y
                    },
                    albedo,
                    emission: Vec3::ZERO,
                });
            }
        }
        // These analytic scenes contain no unrepresented objects. A ray with
        // no forward intersection is certified sky, not just finite Clear.
        Trace::KnownSky
    }
    fn sky_radiance(&self, _: Vec3) -> Vec3 {
        if matches!(self, Self::Sky) {
            Vec3::new(0.2, 0.4, 0.8)
        } else {
            Vec3::ZERO
        }
    }
}
fn light() -> DirectLight {
    DirectLight {
        sun_direction: Vec3::Y,
        sun_irradiance: Vec3::splat(10.0),
    }
}
fn bake(world: World, single: bool, centre: Vec3) -> Result<RadianceCache, String> {
    let mut cache = RadianceCache::new(Config {
        radius_cells: if single { [0; 3] } else { [1, 0, 1] },
        ..Default::default()
    })?;
    cache.set_views(&[centre; 4])?;
    let updates = cache
        .probes()
        .count()
        .div_ceil(cache.config().probes_per_update);
    for _ in 0..updates {
        cache.update(&world, light(), Revision::default())?;
    }
    Ok(cache)
}
fn queries(centre: Vec3) -> Vec<Query> {
    (0..H)
        .flat_map(|y| {
            (0..W).map(move |x| {
                let dx = (x as f32 / (W - 1) as f32 * 2.0 - 1.0) * 3.75;
                let dz = (y as f32 / (H - 1) as f32 * 2.0 - 1.0) * 3.75;
                Query::new(
                    centre + Vec3::new(dx, 0.0, dz),
                    Vec3::new(dx * 0.1, -1.0, dz * 0.1).normalize(),
                )
            })
        })
        .collect()
}
fn dispatch(
    gpu: &Gpu,
    cache: &GpuRadianceCache,
    q: &[Query],
    size: [u32; 2],
) -> Result<Readback, String> {
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let batch = cache.encode(gpu, &mut encoder, q, size, 1.0)?;
    gpu.queue.submit([encoder.finish()]);
    batch.read(gpu)
}
fn max_rgb(values: &[u8]) -> u8 {
    values
        .chunks_exact(4)
        .flat_map(|p| p[..3].iter().copied())
        .max()
        .unwrap_or(0)
}
fn number(x: f32) -> String {
    if x.is_finite() {
        x.to_string()
    } else {
        "null".into()
    }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(
        &args
            .windows(2)
            .find(|v| v[0] == "--out")
            .ok_or("use --out FRESH_DIRECTORY")?[1],
    );
    if out.exists() {
        return Err("output directory must be fresh".into());
    }
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let gpu = Gpu::new()?;
    let mut records = Vec::new();
    let mut failures = Vec::new();
    let cases = [
        ("red-floor", World::Floor, false, CENTRE),
        ("red-floor-occluded", World::Roof, false, CENTRE),
        ("colored-bounce", World::Colored, false, CENTRE),
        ("constant-sky", World::Sky, false, CENTRE),
        ("missing-neighbors", World::Floor, true, CENTRE),
        ("unknown-transport", World::Unknown, true, CENTRE),
        ("buried-probe", World::Buried, true, CENTRE),
        ("moment-occlusion", World::Cavity, true, CENTRE),
        (
            "negative-world-keys",
            World::Floor,
            false,
            Vec3::new(-8.0, 4.0, -12.0),
        ),
    ];
    for (name, world, single, centre) in cases {
        let started = Instant::now();
        let cache = bake(world, single, centre)?;
        let bake_ms = started.elapsed().as_secs_f64() * 1000.0;
        let sampled = GpuRadianceCache::new(&gpu, &cache)?;
        let q = queries(centre);
        let started = Instant::now();
        let image = dispatch(&gpu, &sampled, &q, [W, H])?;
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        render::write_png(&out.join(format!("{name}.png")), W, H, &image.rgba)?;
        render::write_png(
            &out.join(format!("{name}-validity.png")),
            W,
            H,
            &image.validity_rgba,
        )?;
        let mut maxima = [0f32; 3];
        let mut mismatched = 0;
        let mut first_bad = None;
        let mut partial = 0;
        let mut missing = 0;
        let mut moment_blocked = 0;
        for (i, (query, actual)) in q.iter().zip(&image.samples).enumerate() {
            let expected = cache.sample(
                Vec3::from_slice(&query.position),
                Vec3::from_slice(&query.normal),
            );
            let errors = [
                (actual.irradiance - expected.irradiance)
                    .abs()
                    .max_element(),
                (actual.known_weight - expected.known_weight).abs(),
                (actual.visibility_weight - expected.visibility_weight).abs(),
            ];
            let finite = actual.irradiance.is_finite()
                && actual.known_weight.is_finite()
                && actual.visibility_weight.is_finite();
            for j in 0..3 {
                if errors[j].is_finite() {
                    maxima[j] = maxima[j].max(errors[j]);
                }
            }
            if !finite
                || errors[0] > 1e-4 + 1e-4 * expected.irradiance.max_element()
                || errors[1] > 2e-5
                || errors[2] > 3e-4
                || actual.contributing_probes != expected.contributing_probes
            {
                mismatched += 1;
                first_bad.get_or_insert(i);
            }
            partial += usize::from(actual.known_weight > 0.0 && actual.known_weight < 0.999);
            missing += usize::from(actual.known_weight == 0.0);
            moment_blocked +=
                usize::from(actual.known_weight > 0.0 && actual.visibility_weight == 0.0);
        }
        if mismatched != 0 {
            failures.push(format!(
                "{name}: {mismatched} GPU/CPU mismatches, first{first_bad:?}, maxima{maxima:?}"
            ));
        }
        let centre_sample = &image.samples[((H / 2) * W + W / 2) as usize];
        match world {
            World::Floor => {
                if (centre_sample.irradiance.x - 10.0).abs() > 0.5
                    || centre_sample.irradiance.y.abs() > 1e-5
                    || centre_sample.irradiance.z.abs() > 1e-5
                {
                    failures.push(format!(
                        "{name}: analytic Lambert red-floor energy incorrect"
                    ));
                }
            }
            World::Roof | World::Unknown | World::Buried => {
                if max_rgb(&image.rgba) != 0 {
                    failures.push(format!(
                        "{name}: blocked/unknown/invalid radiance is not zero"
                    ));
                }
            }
            World::Sky => {
                if (centre_sample.irradiance - Vec3::new(0.2, 0.4, 0.8) * std::f32::consts::PI)
                    .abs()
                    .max_element()
                    > 2e-4
                {
                    failures.push("constant-sky analytic cosine energy mismatch".into());
                }
            }
            World::Cavity => {
                if moment_blocked < 1000 {
                    failures.push(format!(
                        "moment occlusion not exercised: only{moment_blocked} blocked samples"
                    ));
                }
            }
            World::Colored => {
                if image.samples.iter().all(|s| s.irradiance.y < 0.1) {
                    failures.push("green floor did not contribute green bounced light".into());
                }
            }
        }
        if name == "missing-neighbors" && partial < 1000 {
            failures.push("missing-neighbor coverage was not exercised".into());
        }
        if matches!(world, World::Unknown | World::Buried) && missing != q.len() {
            failures.push(format!("{name}: invalid probes became known"));
        }
        records.push(format!("{{\"name\":{name:?},\"transport\":\"CPU analytic geometry, actual surface albedo/emission and shared sun rays\",\"sampling\":\"GPU SH9 plus directional moments\",\"probes\":{},\"valid_probes\":{},\"moments\":{},\"cache_gpu_bytes\":{},\"queries\":{},\"cpu_bake_ms\":{bake_ms},\"wall_ms_including_gpu_and_readback\":{wall_ms},\"mismatches\":{mismatched},\"max_irradiance_error\":{},\"max_known_error\":{},\"max_visibility_error\":{},\"partial_samples\":{partial},\"missing_samples\":{missing},\"moment_blocked_samples\":{moment_blocked}}}",
            sampled.stats.probes,sampled.stats.valid_probes,sampled.stats.moments,sampled.stats.bytes,q.len(),number(maxima[0]),number(maxima[1]),number(maxima[2])));
        if name == "red-floor" {
            // Four simultaneous query panels reuse this single uploaded cache.
            let mut four = Vec::with_capacity(q.len() * 4);
            for y in 0..H * 2 {
                for x in 0..W * 2 {
                    four.push(q[((y % H) * W + x % W) as usize]);
                }
            }
            let shared = dispatch(&gpu, &sampled, &four, [W * 2, H * 2])?;
            render::write_png(
                &out.join("four-views-one-cache.png"),
                W * 2,
                H * 2,
                &shared.rgba,
            )?;
            let mut different = 0;
            for y in 0..H * 2 {
                for x in 0..W * 2 {
                    let source = (((y % H) * W + x % W) * 4) as usize;
                    let target = ((y * W * 2 + x) * 4) as usize;
                    different += usize::from(
                        image.rgba[source..source + 4] != shared.rgba[target..target + 4],
                    );
                }
            }
            if different != 0 {
                failures.push(format!(
                    "four shared query views differ at{different} pixels"
                ));
            }
            records.push(format!("{{\"name\":\"four-views-one-cache\",\"cache_uploads\":1,\"cache_gpu_bytes\":{},\"pixel_differences\":{different}}}",sampled.stats.bytes));
        }
    }
    let empty = RadianceCache::new(Config::default())?;
    let empty_gpu = GpuRadianceCache::new(&gpu, &empty)?;
    let empty_read = dispatch(&gpu, &empty_gpu, &queries(CENTRE), [W, H])?;
    if max_rgb(&empty_read.rgba) != 0 || empty_read.samples.iter().any(|s| s.known_weight != 0.0) {
        failures.push("empty cache manufactured light".into());
    }
    render::write_png(&out.join("empty-cache.png"), W, H, &empty_read.rgba)?;
    records.push(format!(
        "{{\"name\":\"empty-cache\",\"probes\":0,\"gpu_bytes\":{}}}",
        empty_gpu.stats.bytes
    ));
    let report = format!(
        "{{\"scope\":\"CPU transport, GPU cache sampling only; no gameplay GI or GPU transport claim\",\"width\":{W},\"height\":{H},\"max_probes\":2048,\"rays_per_probe\":96,\"validity_image_channels\":\"red known, green visibility, blue missing\",\"adapter\":{:?},\"passed\":{},\"cases\":[{}],\"failures\":[{}]}}",
        gpu.info.name,
        failures.is_empty(),
        records.join(","),
        failures
            .iter()
            .map(|s| format!("{s:?}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    std::fs::write(out.join("report.json"), report).map_err(|e| e.to_string())?;
    if !failures.is_empty() {
        return Err(failures.join("; "));
    }
    println!(
        "PASS: CPU transport/GPU radiance cache sampling, nine numeric cases, shared four-view upload and empty cache. {}",
        out.display()
    );
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("radiance cache GPU proof: {e}");
        std::process::exit(1);
    }
}
