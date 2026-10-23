//! `sideways gpu-run`: offscreen, held-pose rendering jobs.
//! `warmup` (default 2) excludes initial frames from timing summaries; raw
//! samples are retained. `taa` defaults to true, with history retained across
//! warmup and measured frames. Wall time includes synchronous pixel readback.

use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

pub fn run(out: &str, jobs_path: &str) -> i32 {
    let out = Path::new(out);
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("{}: {e}", out.display());
        return 2;
    }
    let jobs: toml::Table = match std::fs::read_to_string(jobs_path).map_err(|e| e.to_string()).and_then(|t| toml::from_str(&t).map_err(|e| e.to_string())) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("{jobs_path}: {e}");
            return 2;
        }
    };
    let Some(jobs) = jobs.get("job").and_then(|j| j.as_array()).filter(|j| !j.is_empty()) else {
        eprintln!("{jobs_path}: expected at least one [[job]]");
        return 2;
    };
    let t = render::Stopwatch::start();
    let gpu = match render::Gpu::new() {
        Ok(g) => g,
        Err(e) => {
            if let Err(write_error) = std::fs::write(out.join("error.txt"), &e) {
                eprintln!("writing error.txt: {write_error}");
            }
            eprintln!("{e}");
            return 1;
        }
    };
    let info = gpu.summary();
    let mut results = vec![json!({"job": "init", "ms": t.ms(), "gpu": gpu_metadata(&gpu)})];
    let mut summary = format!("GPU: {} ({}, {})\nHeld-pose offscreen render benchmark; wall times include pixel readback. This is not moving gameplay FPS.\n", info.name, info.backend, info.device_type);
    let mut failed = false;
    let mut output_names = std::collections::BTreeSet::new();
    for job in jobs {
        let kind = job.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        let name = job.get("name").and_then(|v| v.as_str()).unwrap_or(if kind == "stage-frame" { "stage" } else { kind });
        let r = (|| -> Result<Value, String> {
            let size = job_size(job)?;
            if size.0 > gpu.limits.max_texture_dimension_2d || size.1 > gpu.limits.max_texture_dimension_2d {
                return Err("size exceeds the GPU texture dimension limit".into());
            }
            if kind == "stage-frame" || kind == "test-frame" {
                validate_name(name)?;
                if !output_names.insert(name.to_string()) {
                    return Err(format!("duplicate output name: {name}"));
                }
            }
            match kind {
                "info" => Ok(json!({"job": "info", "gpu": gpu_metadata(&gpu), "name": info.name, "backend": info.backend, "driver": info.driver, "type": info.device_type, "timestamps": info.timestamps})),
                "test-frame" => {
                    let t = render::Stopwatch::start();
                    let px = render::test_frame(&gpu, size.0, size.1);
                    let file = format!("{name}.png");
                    render::write_png(&out.join(&file), size.0, size.1, &px).map_err(|e| format!("writing {file}: {e}"))?;
                    Ok(json!({"job": "test-frame", "ok": true, "ms": t.ms(), "file": file}))
                }
                "stage-frame" => stage_frame(&gpu, job, size, out, &mut summary),
                other => Err(format!("unknown job kind: {other}")),
            }
        })();
        let r = match r {
            Ok(r) => r,
            Err(e) => {
                failed = true;
                eprintln!("{name}: {e}");
                json!({"job": name, "kind": kind, "ok": false, "error": e})
            }
        };
        summary.push_str(&format!("{kind}: {}\n", r));
        results.push(r);
    }
    if let Err(e) = write_json(&out.join("results.json"), &json!(results)) {
        eprintln!("{e}");
        failed = true;
    }
    if let Err(e) = std::fs::write(out.join("summary.md"), &summary) {
        eprintln!("writing summary.md: {e}");
        failed = true;
    }
    print!("{summary}");
    if failed { 1 } else { 0 }
}

/// Hold the scene and cameras fixed, but invalidate shadows every frame so
/// timing includes shadow rendering. GPU totals sum instrumented passes; they
/// do not include uninstrumented GPU work or CPU simulation.
fn stage_frame(gpu: &render::Gpu, job: &toml::Value, size: (u32, u32), out: &Path, summary: &mut String) -> Result<Value, String> {
    let get = |k: &str, d: f64| job.get(k).and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64))).unwrap_or(d);
    let name = job.get("name").and_then(|v| v.as_str()).unwrap_or("stage");
    let repeat = count(job, "repeat", 5, 1)?;
    let warmup = count(job, "warmup", 2, 0)?;
    let total_frames = repeat.checked_add(warmup).ok_or("warmup + repeat exceeds u32")?;
    let taa = match job.get("taa") {
        None => true,
        Some(v) => v.as_bool().ok_or("taa must be a boolean")?,
    };
    let args = crate::shot::ShotArgs {
        out: String::new(),
        seed: get("seed", 1.0) as u64,
        km: get("km", 2.0),
        at: get("at", 300.0),
        frames: get("frames", 400.0) as u64,
        size,
        weather: job.get("weather").and_then(|v| v.as_str()).map(String::from),
        lod_pixels: get("lod", 1.0) as f32,
        picks: vec![],
        views: get("views", 1.0) as usize,
        taa_frames: if taa { total_frames } else { 1 },
        ground_cover: job.get("ground_cover").and_then(|v| v.as_bool()).unwrap_or(true),
        sets: job.get("set").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
    };
    let t = render::Stopwatch::start();
    let mut setup = crate::shot::prepare(&args)?;
    if !taa {
        setup.light.taa = 0.0;
    }
    let taa_enabled = setup.light.taa > 0.5;
    // prepare() describes shot convergence. This command instead retains
    // history through its whole warmup + measured sequence, even for 1 frame.
    setup.metadata["taa_enabled"] = json!(taa_enabled);
    setup.metadata["taa_frames"] = json!(if taa_enabled { total_frames } else { 1 });
    let prep_ms = t.ms();
    let t = render::Stopwatch::start();
    let renderer = render::Renderer::new(gpu, &setup.scene, &setup.light);
    let upload_ms = t.ms();
    let views = crate::shot::view_rects(&setup.cams, args.views, size.0, size.1);
    let mut walls = Vec::new();
    let mut totals = Vec::new();
    let mut passes: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    let mut samples = Vec::new();
    let mut last = (Vec::new(), render::FrameStats::default());
    for frame in 0..total_frames {
        let t = render::Stopwatch::start();
        renderer.invalidate_shadow();
        last = renderer.render_views(gpu, &views, &setup.light, size.0, size.1, &[]);
        let wall_ms = t.ms();
        let gpu_total = gpu_total(&last.1.gpu_ms)?;
        let measured = frame >= warmup;
        if measured {
            walls.push(wall_ms);
            if let Some(total) = gpu_total {
                totals.push(total);
            }
            for (n, ms) in &last.1.gpu_ms {
                passes.entry(n).or_default().push(*ms);
            }
        }
        let gpu_passes: BTreeMap<_, _> = last.1.gpu_ms.iter().copied().collect();
        samples.push(json!({
            "render_frame": frame + 1, "phase": if measured { "measured" } else { "warmup" },
            "wall_ms": wall_ms, "gpu_ms_total": gpu_total, "gpu_ms_by_pass": gpu_passes,
            "counts": frame_counts(&last.1)
        }));
    }
    let file = format!("{name}.png");
    render::write_png(&out.join(&file), size.0, size.1, &last.0).map_err(|e| format!("writing {file}: {e}"))?;
    // Preserve the old rounded, upper-median fields for existing consumers,
    // but never present their sum as the measured median frame total.
    let legacy_passes: BTreeMap<_, _> = passes.iter().map(|(n, v)| (*n, (upper_median(v) * 100.0).round() / 100.0)).collect();
    let legacy_total: Option<f64> = (!legacy_passes.is_empty()).then(|| legacy_passes.values().sum());
    let pass_medians: BTreeMap<_, _> = passes.iter().map(|(n, v)| (*n, median(v))).collect();
    let gpu_median = (!totals.is_empty()).then(|| median(&totals));
    let gpu_p95 = (!totals.is_empty()).then(|| p95(&totals));
    let wall_median = median(&walls);
    let wall_p95 = p95(&walls);
    let metadata_file = format!("{file}.json");
    let log_file = format!("{name}.log");
    let benchmark = json!({
        "kind": "held_pose_offscreen", "warmup_frames": warmup, "measured_frames": repeat,
        "rendered_frames": total_frames, "shadow_invalidated_every_frame": true,
        "taa_enabled": taa_enabled, "taa_history": "retained across warmup and measured frames",
        "gpu_total_definition": "sum of instrumented GPU passes per rendered frame; excludes uninstrumented work",
        "wall_definition": "render_views including synchronous pixel readback; excludes simulation, prepare, upload and PNG encoding",
        "p95_definition": "nearest rank ceil(0.95 * sample_count)",
        "median_definition": "middle value, or mean of two middle values for an even count",
        "gpu_timestamp_samples": totals.len(), "moving_gameplay": false
    });
    setup.metadata["gpu"] = gpu_metadata(gpu);
    setup.metadata["benchmark"] = benchmark.clone();
    setup.metadata["job_config"] = json!(job);
    setup.metadata["last_frame_counts"] = frame_counts(&last.1);
    setup.metadata["preparation_log"] = json!(setup.log);
    write_json(&out.join(&metadata_file), &setup.metadata)?;
    let line = format!("{name} {}x{}: instrumented GPU median {} ms / p95 {} ms, wall median {wall_median:.3} ms / p95 {wall_p95:.3} ms, {} tris, {warmup} warmup + {repeat} measured frames\n",
        size.0, size.1, display_ms(gpu_median), display_ms(gpu_p95), last.1.triangles);
    let log = format!("{}\n{line}Metadata: {metadata_file}\n", setup.log.join("\n"));
    std::fs::write(out.join(&log_file), log).map_err(|e| format!("writing {log_file}: {e}"))?;
    summary.push_str(&line);
    Ok(json!({
        "job": name, "kind": "stage-frame", "ok": true, "size": [size.0, size.1], "file": file,
        "metadata_file": metadata_file, "log_file": log_file, "gpu": gpu_metadata(gpu), "benchmark": benchmark,
        "prepare_ms": prep_ms, "upload_ms": upload_ms,
        "wall_ms_median": wall_median, "wall_ms_p95": wall_p95,
        "gpu_ms_total_median": gpu_median, "gpu_ms_total_p95": gpu_p95, "gpu_ms_by_pass_median": pass_medians,
        "gpu_ms_median": legacy_passes, "gpu_ms_total": legacy_total,
        "legacy_fields": {"gpu_ms_median": "rounded upper median per pass", "gpu_ms_total": "sum of rounded per-pass upper medians; NOT median frame total"},
        "samples": samples,
        "clusters": last.1.clusters, "triangles": last.1.triangles,
        "shadow_clusters": last.1.shadow_clusters, "shadow_triangles": last.1.shadow_triangles
    }))
}

fn gpu_metadata(gpu: &render::Gpu) -> Value {
    let info = gpu.summary();
    json!({"name": info.name, "backend": info.backend, "driver": info.driver, "type": info.device_type, "timestamps": info.timestamps})
}

fn frame_counts(s: &render::FrameStats) -> Value {
    json!({"clusters": s.clusters, "triangles": s.triangles, "shadow_clusters": s.shadow_clusters, "shadow_triangles": s.shadow_triangles})
}

fn gpu_total(passes: &[(&str, f64)]) -> Result<Option<f64>, String> {
    if passes.iter().any(|(_, ms)| !ms.is_finite() || *ms < 0.0) {
        return Err("invalid GPU timestamp sample".into());
    }
    Ok((!passes.is_empty()).then(|| passes.iter().map(|(_, ms)| ms).sum()))
}

fn sorted(values: &[f64]) -> Vec<f64> {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values
}

fn median(values: &[f64]) -> f64 {
    let values = sorted(values);
    let mid = values.len() / 2;
    if values.len() % 2 == 0 { (values[mid - 1] + values[mid]) * 0.5 } else { values[mid] }
}

fn upper_median(values: &[f64]) -> f64 {
    sorted(values)[values.len() / 2]
}

fn p95(values: &[f64]) -> f64 {
    sorted(values)[(values.len() as f64 * 0.95).ceil() as usize - 1]
}

fn display_ms(value: Option<f64>) -> String {
    value.map(|v| format!("{v:.3}")).unwrap_or_else(|| "unavailable".into())
}

fn count(job: &toml::Value, key: &str, default: u32, minimum: u32) -> Result<u32, String> {
    match job.get(key) {
        None => Ok(default),
        Some(v) => v.as_integer().and_then(|i| u32::try_from(i).ok()).filter(|n| *n >= minimum).ok_or_else(|| format!("{key} must be an integer in {minimum}..=u32::MAX")),
    }
}

fn job_size(job: &toml::Value) -> Result<(u32, u32), String> {
    let Some(size) = job.get("size") else { return Ok((640, 360)); };
    let Some(parts) = size.as_array().filter(|a| a.len() == 2) else { return Err("size must be [width, height]".into()); };
    let dimension = |v: &toml::Value| v.as_integer().and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0).ok_or("size dimensions must be positive u32 integers".to_string());
    Ok((dimension(&parts[0])?, dimension(&parts[1])?))
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')) {
        return Err("output name must contain only ASCII letters, digits, hyphens or underscores".into());
    }
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| format!("serializing {}: {e}", path.display()))?;
    std::fs::write(path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_total_median_is_not_sum_of_pass_medians() {
        let a = [1.0, 100.0, 100.0];
        let b = [100.0, 1.0, 100.0];
        let totals: Vec<_> = a.iter().zip(b).map(|(a, b)| gpu_total(&[("a", *a), ("b", b)]).unwrap().unwrap()).collect();
        assert_eq!(median(&totals), 101.0);
        assert_eq!(median(&a) + median(&b), 200.0);
        assert_eq!(p95(&totals), 200.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    #[test]
    fn missing_timestamps_are_not_zero_and_invalid_timestamps_fail() {
        assert_eq!(gpu_total(&[]).unwrap(), None);
        assert!(gpu_total(&[("bad", f64::NAN)]).is_err());
        assert!(gpu_total(&[("bad", -1.0)]).is_err());
    }

    #[test]
    fn invalid_job_dimensions_and_counts_are_rejected() {
        let job: toml::Value = toml::from_str("size = [4]\nrepeat = 0\nwarmup = -1").unwrap();
        assert!(job_size(&job).is_err());
        assert!(count(&job, "repeat", 5, 1).is_err());
        assert!(count(&job, "warmup", 2, 0).is_err());
        assert!(validate_name("../outside").is_err());
    }
}
