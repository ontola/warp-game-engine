//! Repeatable native-resolution diagnostics shared by all games.
//! No pixel readback, adaptation, or window presentation. CPU and GPU overlap;
//! the serialized completion wait is reported separately, never called FPS.
use crate::{Controls, Game, Headless};
use render::Gpu;
use serde::Serialize;
use std::{collections::BTreeMap, time::Instant};

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct DrawTimings {
    pub game_ms: f64,
    pub render_ms: f64,
    pub hud_ms: f64,
    pub present_ms: f64,
    /// HUD vertices and text runs drawn this frame.
    pub hud_vertices: usize,
    pub hud_text_runs: usize,
}

/// Nearest-rank percentiles, with all individual samples retained in the report.
pub fn distribution(values: &[f64]) -> serde_json::Value {
    if values.is_empty() {
        return serde_json::Value::Null;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        sorted[((p * sorted.len() as f64).ceil() as usize)
            .saturating_sub(1)
            .min(sorted.len() - 1)]
    };
    serde_json::json!({"samples": sorted.len(), "mean": sorted.iter().sum::<f64>() / sorted.len() as f64,
        "p50": percentile(0.5), "p95": percentile(0.95), "p99": percentile(0.99), "max": sorted.last()})
}

fn require_native(status: &render::upscale::UpscaleStatus) -> Result<(), String> {
    if status.mode != render::upscale::UpscaleMode::Native
        || status.views.iter().any(|v| v.input != v.output)
    {
        return Err("native profiling cannot include reconstructed or scaled views".into());
    }
    Ok(())
}

/// One simulation tick per rendered frame. The caller supplies initial state
/// and controls through Game (e.g. demo AI); no screenshot enters the sample set.
pub fn bench(
    game: &mut dyn Game,
    gpu: &Gpu,
    size: [u32; 2],
    warmup: u32,
    frames: u32,
    tick_hz: f32,
) -> Result<serde_json::Value, String> {
    if frames == 0 || warmup == 0 || size.contains(&0) || !tick_hz.is_finite() || tick_hz <= 0. {
        return Err(
            "profiling needs nonzero dimensions, warmup, samples and a positive tick rate".into(),
        );
    }
    let total = warmup
        .checked_add(frames)
        .ok_or("profile frame count overflow")?;
    let mut headless = Headless::new(gpu, size, tick_hz)?;
    let controls = vec![Controls::default(); game.local_players()];
    let mut cpu = Vec::new();
    let mut gpu_frames = Vec::new();
    let mut columns: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut drops = 0;
    let mut errors = 0;
    let mut pending = 0;
    for id in 0..total {
        if let Some(renderer) = headless.renderer() {
            renderer.set_gpu_profile_frame_id(u64::from(id));
        }
        let started = Instant::now();
        game.tick(&controls);
        let tick_ms = started.elapsed().as_secs_f64() * 1000.;
        let draw = headless.render_at(game, 1.)?;
        let cpu_ms = started.elapsed().as_secs_f64() * 1000.;
        let waiting = Instant::now();
        gpu.wait();
        let wait_ms = waiting.elapsed().as_secs_f64() * 1000.;
        let renderer = headless.renderer().unwrap();
        require_native(&renderer.upscale_status())?;
        if let Some((spans, _)) = renderer.cpu_spans() {
            if id >= warmup {
                for (name, ms) in spans {
                    columns
                        .entry(format!("renderer_cpu.{name}_ms"))
                        .or_default()
                        .push(ms);
                }
            }
        }
        if id == 0 {
            renderer.set_async_gpu_profiling(gpu, true)?;
        }
        let drain = renderer.drain_gpu_profiles(gpu);
        drops = drain.dropped;
        errors = drain.errors;
        pending = drain.pending;
        for sample in drain
            .samples
            .into_iter()
            .filter(|s| s.frame_id >= u64::from(warmup))
        {
            if sample.output_size != size {
                return Err(format!(
                    "native resolution changed: {:?} != {:?}",
                    sample.output_size, size
                ));
            }
            columns
                .entry("gpu.instrumented_ms".into())
                .or_default()
                .push(sample.instrumented_gpu_ms);
            columns
                .entry("gpu.covered_interval_ms".into())
                .or_default()
                .push(sample.covered_interval_ms);
            for (name, ms) in &sample.gpu_ms {
                columns
                    .entry(format!("gpu.{name}_ms"))
                    .or_default()
                    .push(*ms);
            }
            let intervals: Vec<_> = sample.intervals.iter().map(|s| serde_json::json!({"name":s.name,"start_ms":s.start_ms,"end_ms":s.end_ms,"parent":s.parent,"view":s.view_index})).collect();
            gpu_frames.push(serde_json::json!({"frame": sample.frame_id, "size": sample.output_size, "views":sample.views,"intervals":intervals}));
        }
        if id >= warmup {
            for (name, value) in [
                ("tick", tick_ms),
                ("game_draw", draw.game_ms),
                ("render_encode", draw.render_ms),
                ("hud", draw.hud_ms),
                ("present_encode", draw.present_ms),
                ("submit_total", cpu_ms),
                ("completion_wait", wait_ms),
            ] {
                columns
                    .entry(format!("cpu.{name}_ms"))
                    .or_default()
                    .push(value);
            }
            cpu.push(serde_json::json!({"frame":id,"tick_ms":tick_ms,"draw":draw,"submit_total_ms":cpu_ms,"completion_wait_ms":wait_ms}));
        }
    }
    let summary: BTreeMap<_, _> = columns.iter().map(|(k, v)| (k, distribution(v))).collect();
    Ok(
        serde_json::json!({"schema":"warp.native-profile.v1","size":size,"reconstruction":"native","tick_hz":tick_hz,"warmup":warmup,"frames":frames,
        "gpu_samples":gpu_frames.len(),"dropped":drops,"errors":errors,"pending":pending,"summary":summary,"cpu_frames":cpu,"gpu_frames":gpu_frames,
        "note":"Native output; no dynamic resolution or pixel readback. CPU wall time includes scheduling/driver work. GPU timestamps exclude HUD/presentation. Serialized completion wait is not gameplay FPS; CPU and GPU costs are not additive."}),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn reconstructed_views_cannot_be_reported_as_native() {
        use render::upscale::{UpscaleMode, UpscaleStatus, UpscaleViewStatus};
        let mut s = UpscaleStatus {
            mode: UpscaleMode::Native,
            views: vec![UpscaleViewStatus {
                input: [3840, 2160],
                output: [3840, 2160],
            }],
        };
        assert!(super::require_native(&s).is_ok());
        s.mode = UpscaleMode::DlssQuality;
        assert!(super::require_native(&s).is_err());
        s.mode = UpscaleMode::Native;
        s.views[0].input = [1920, 1080];
        assert!(super::require_native(&s).is_err());
    }
    #[test]
    fn percentiles_keep_outliers_and_handle_empty() {
        assert!(super::distribution(&[]).is_null());
        let d = super::distribution(&[4., 1., 100., 2.]);
        assert_eq!(d["p50"], 2.);
        assert_eq!(d["p95"], 100.);
        assert_eq!(d["mean"], 26.75);
    }
}
