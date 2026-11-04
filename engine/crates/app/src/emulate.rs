//! Feel slower hardware on a fast machine, for every game on the shell:
//! `WARP_EMULATE=m1pro` or `--emulate m1pro` (`x2.5` for a free GPU factor),
//! or the F10 Render menu's "Emulate hardware".
//!
//! GPU: a calibrated compute pass after each scene burns (factor - 1) times
//! the frame's measured GPU time, so lowering resolution or content also
//! lowers the burn, as on the real machine. It scales time only: bandwidth,
//! tile memory and driver differences are not modelled, so treat results as
//! a good guess, not a measurement of that machine. CPU: the shell spins for
//! (factor - 1) times its frame work. Display: the refresh rate replaces the
//! real one as adaptive quality's target and the height caps the 3D picture
//! (the window itself keeps its size). Memory is reported only, nothing
//! enforces it yet. A start probe reports emulated GPU times, and
//! [`chip_name`] lets a game keep its caches apart.
use render::Gpu;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    /// As the system reports the chip, for a game's chip checks.
    pub chip: &'static str,
    /// FP32 TFLOPS: public figures for the full GPU configuration.
    pub gpu: f32,
    /// Geekbench 6 single core, rounded public figures.
    pub cpu: f32,
    pub display: [u32; 2],
    pub refresh_hz: f64,
    pub memory_mb: u32,
}

pub const PRESETS: [Preset; 7] = [
    Preset { id: "m1", label: "M1", chip: "Apple M1", gpu: 2.6, cpu: 2350.0, display: [2560, 1600], refresh_hz: 60.0, memory_mb: 8192 },
    Preset { id: "m1pro", label: "M1 Pro", chip: "Apple M1 Pro", gpu: 5.2, cpu: 2380.0, display: [3024, 1964], refresh_hz: 120.0, memory_mb: 16384 },
    Preset { id: "m1max", label: "M1 Max", chip: "Apple M1 Max", gpu: 10.4, cpu: 2380.0, display: [3024, 1964], refresh_hz: 120.0, memory_mb: 32768 },
    Preset { id: "m2", label: "M2", chip: "Apple M2", gpu: 3.6, cpu: 2650.0, display: [2560, 1664], refresh_hz: 60.0, memory_mb: 8192 },
    Preset { id: "m3", label: "M3", chip: "Apple M3", gpu: 4.1, cpu: 3050.0, display: [2560, 1664], refresh_hz: 60.0, memory_mb: 8192 },
    Preset { id: "m4", label: "M4", chip: "Apple M4", gpu: 4.3, cpu: 3700.0, display: [2560, 1664], refresh_hz: 60.0, memory_mb: 16384 },
    Preset { id: "steamdeck", label: "Steam Deck", chip: "AMD Custom APU 0405", gpu: 1.6, cpu: 1300.0, display: [1280, 800], refresh_hz: 60.0, memory_mb: 16384 },
];

/// (name fragment, TFLOPS, Geekbench) of machines this runs on, most specific
/// first. The M5 Pro's GPU figure is an estimate; an unknown machine counts
/// as this one too, so factors there are only roughly right.
const HOSTS: [(&str, f32, f32); 8] = [
    ("M5 Pro", 14.5, 4250.0), ("M5", 5.0, 4250.0), ("M4 Pro", 9.2, 3800.0), ("M4", 4.3, 3700.0),
    ("M3", 4.1, 3050.0), ("M2", 3.6, 2650.0), ("M1 Pro", 5.2, 2380.0), ("M1", 2.6, 2350.0),
];
fn host_scores(adapter: &str) -> (f32, f32) {
    HOSTS.iter().find(|h| adapter.contains(h.0)).map_or((HOSTS[0].1, HOSTS[0].2), |h| (h.1, h.2))
}

/// What is being emulated right now.
#[derive(Clone, Debug, PartialEq)]
pub struct Emulation {
    pub label: String,
    pub preset: Option<Preset>,
    pub gpu_factor: f32,
    pub cpu_factor: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
struct State { preset: Option<usize>, gpu: Option<f32> }
static STATE: Mutex<Option<State>> = Mutex::new(None);
static HOST: Mutex<Option<(f32, f32)>> = Mutex::new(None);

fn parse(s: &str) -> Result<State, String> {
    let s = s.trim().to_lowercase();
    if s.is_empty() || s == "off" { return Ok(State::default()); }
    if let Some(v) = s.strip_prefix('x') {
        let gpu = v.parse::<f32>().ok().filter(|f| f.is_finite() && *f >= 1.0).ok_or(format!("emulate {s}: a factor of 1 or more"))?;
        return Ok(State { preset: None, gpu: Some(gpu) });
    }
    let k = PRESETS.iter().position(|p| p.id == s).ok_or(format!("emulate {s}: use xN or {}", PRESETS.map(|p| p.id).join(", ")))?;
    Ok(State { preset: Some(k), gpu: None })
}

fn state() -> State {
    *STATE.lock().unwrap().get_or_insert_with(|| {
        let args: Vec<String> = std::env::args().collect();
        let asked = args.windows(2).find(|a| a[0] == "--emulate").map(|a| a[1].clone()).or_else(|| std::env::var("WARP_EMULATE").ok());
        asked.map_or_else(State::default, |a| parse(&a).unwrap_or_else(|e| { eprintln!("{e}"); State::default() }))
    })
}

/// Tell the emulation which GPU this really is (the shell does, on start).
pub fn set_host(adapter: &str) { *HOST.lock().unwrap() = Some(host_scores(adapter)); }

fn resolve(s: State, host: (f32, f32)) -> Option<Emulation> {
    let preset = s.preset.map(|k| PRESETS[k]);
    if preset.is_none() && s.gpu.is_none() { return None; }
    let gpu_factor = s.gpu.or(preset.map(|p| host.0 / p.gpu)).unwrap_or(1.0).max(1.0);
    let cpu_factor = preset.map_or(1.0, |p| (host.1 / p.cpu).max(1.0));
    Some(Emulation { label: preset.map_or_else(|| "custom GPU".to_string(), |p| p.label.to_string()), preset, gpu_factor, cpu_factor })
}

/// The active emulation, if any.
pub fn current() -> Option<Emulation> {
    let host = HOST.lock().unwrap().unwrap_or((HOSTS[0].1, HOSTS[0].2));
    resolve(state(), host)
}
pub fn active() -> bool { state() != State::default() }
/// The emulated chip's name, for chip checks and per-machine caches; a free
/// factor gets a name of its own so its results stay apart.
pub fn chip_name() -> Option<String> {
    let e = current()?;
    Some(e.preset.map_or_else(|| format!("emulated GPU x{:.1}", e.gpu_factor), |p| format!("{} (emulated)", p.chip)))
}
pub fn refresh_hz() -> Option<f64> { current()?.preset.map(|p| p.refresh_hz) }
/// The emulated display's height, a cap on the 3D picture.
pub fn display_height() -> Option<u32> { current()?.preset.map(|p| p.display[1]) }
pub fn memory_mb() -> Option<u32> { current()?.preset.map(|p| p.memory_mb) }
/// "EMULATING M1 PRO (GPU x2.8)".
pub fn badge() -> Option<String> {
    let e = current()?;
    Some(format!("EMULATING {} (GPU x{:.1}{})", e.label.to_uppercase(), e.gpu_factor, if e.cpu_factor > 1.01 { format!(", CPU x{:.1}", e.cpu_factor) } else { String::new() }))
}

/// For the debug menu: "Off", then the presets.
pub fn choices() -> Vec<String> { std::iter::once("Off".to_string()).chain(PRESETS.iter().map(|p| p.label.to_string())).collect() }
pub fn choice() -> usize { state().preset.map_or(0, |k| k + 1) }
pub fn choose(index: usize) { *STATE.lock().unwrap() = Some(State { preset: index.checked_sub(1).filter(|k| *k < PRESETS.len()), gpu: None }); }
pub fn gpu_factor() -> f32 { current().map_or(1.0, |e| e.gpu_factor) }
/// The free GPU factor: 1 turns a factor without a preset off.
pub fn set_gpu_factor(factor: f32) {
    let mut s = state();
    s.gpu = (factor > 1.0).then_some(factor);
    *STATE.lock().unwrap() = Some(s);
}

/// Busy work on the GPU, calibrated once.
const BURN: &str = "
@group(0) @binding(0) var<storage, read_write> sink: array<f32>;
@group(0) @binding(1) var<uniform> work: vec4<u32>;
@compute @workgroup_size(64)
fn burn(@builtin(global_invocation_id) id: vec3<u32>) {
    var x = f32(id.x) * 1e-6 + 1.0;
    var y = 0.5;
    for (var i = 0u; i < work.x; i++) { x = fma(x, 0.9999, y); y = fma(y, 0.9998, x * 1e-6); }
    if x == 12345.678 { sink[id.x % 1024u] = y; }
}";
const GROUPS: u32 = 4096;
/// One frame's burn at most: a stalled reading must not hang the GPU.
const MAX_BURN_MS: f64 = 250.0;

struct GpuBurn { pipeline: wgpu::ComputePipeline, bind: wgpu::BindGroup, work: wgpu::Buffer, loops_per_ms: f64 }

impl GpuBurn {
    fn new(gpu: &Gpu) -> Self {
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("emulate burn"), source: wgpu::ShaderSource::Wgsl(BURN.into()) });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("emulate burn"), layout: None, module: &module, entry_point: Some("burn"), compilation_options: Default::default(), cache: None });
        let sink = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("emulate sink"), size: 4096, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let work = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("emulate work"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("emulate burn"), layout: &pipeline.get_bind_group_layout(0), entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: sink.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: work.as_entire_binding() }] });
        let mut burn = Self { pipeline, bind, work, loops_per_ms: 1.0 };
        // Grow the trial until it takes a few ms, then take the fastest of three.
        gpu.wait();
        let mut loops = 2000u32;
        for _ in 0..12 {
            let best = (0..3).map(|_| { let t = Instant::now(); burn.dispatch(gpu, loops); gpu.wait(); t.elapsed().as_secs_f64() * 1000.0 }).fold(f64::MAX, f64::min);
            burn.loops_per_ms = f64::from(loops) / best.max(0.01);
            if best >= 4.0 { break; }
            loops = loops.saturating_mul(2);
        }
        burn
    }
    fn dispatch(&self, gpu: &Gpu, loops: u32) {
        gpu.queue.write_buffer(&self.work, 0, &loops.to_ne_bytes());
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.dispatch_workgroups(GROUPS, 1, 1);
        }
        gpu.queue.submit([enc.finish()]);
    }
}

const CLEAN_EVERY: u32 = 60;
const CLEAN_FRAMES: u32 = 4;
const SAMPLES: usize = 64;
/// The second fastest sample: clean frames, without a lone outlier.
fn real_ms(samples: &std::collections::VecDeque<f64>) -> f64 {
    let mut v: Vec<f64> = samples.iter().copied().collect();
    v.sort_by(f64::total_cmp);
    v.get(1).or(v.first()).copied().unwrap_or(0.0)
}

/// Runs the emulation for a window or a headless target.
#[derive(Default)]
pub struct Emulator { burn: Option<GpuBurn>, samples: std::collections::VecDeque<f64>, frame: u32 }

impl Emulator {
    /// After a scene frame: `gpu_ms` is a measured GPU time when one has
    /// arrived (profiles trail by a few frames). The burn overlaps the scene
    /// on the GPU and inflates what is measured, which would feed back into
    /// the burn, so a few frames in every [`CLEAN_EVERY`] run without it and
    /// the real time is the fastest of the recent samples.
    pub fn scene(&mut self, gpu: &Gpu, gpu_ms: Option<f64>) {
        let Some(e) = current() else { return };
        if let Some(ms) = gpu_ms.filter(|m| m.is_finite() && *m > 0.0) {
            if self.samples.len() == SAMPLES { self.samples.pop_front(); }
            self.samples.push_back(ms);
        }
        self.frame = self.frame.wrapping_add(1);
        if self.frame % CLEAN_EVERY < CLEAN_FRAMES { return; }
        let extra = (real_ms(&self.samples) * f64::from(e.gpu_factor - 1.0)).min(MAX_BURN_MS);
        if extra < 0.05 { return; }
        let burn = self.burn.get_or_insert_with(|| GpuBurn::new(gpu));
        let loops = (extra * burn.loops_per_ms).clamp(1.0, f64::from(u32::MAX)) as u32;
        burn.dispatch(gpu, loops);
    }
    /// At the end of a frame: spin so the CPU work took `cpu_factor` as long.
    pub fn cpu(&self, frame_started: Instant) {
        let Some(e) = current().filter(|e| e.cpu_factor > 1.01) else { return };
        let until = frame_started.elapsed().mul_f32(e.cpu_factor - 1.0).min(std::time::Duration::from_millis(100));
        let t = Instant::now();
        while t.elapsed() < until { std::hint::spin_loop(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_scale_from_the_host_and_never_speed_it_up() {
        let m5 = host_scores("Apple M5 Pro");
        let m1pro = resolve(parse("m1pro").unwrap(), m5).unwrap();
        assert!((m1pro.gpu_factor - 2.79).abs() < 0.01, "{}", m1pro.gpu_factor);
        assert!(m1pro.cpu_factor > 1.5);
        assert_eq!(resolve(parse("x2.5").unwrap(), m5).unwrap().gpu_factor, 2.5);
        // An M1 emulating an M1 Max stays itself.
        assert_eq!(resolve(parse("m1max").unwrap(), host_scores("Apple M1")).unwrap().gpu_factor, 1.0);
        assert_eq!(resolve(parse("off").unwrap(), m5), None);
        assert!(parse("m9").is_err() && parse("x0.5").is_err());
    }

    #[test]
    fn the_real_gpu_time_comes_from_the_clean_frames() {
        let samples: std::collections::VecDeque<f64> = [30.0, 31.0, 13.0, 12.9, 13.1, 29.0, 2.0].into_iter().collect();
        assert_eq!(real_ms(&samples), 12.9, "the second fastest, past one outlier");
        assert_eq!(real_ms(&Default::default()), 0.0);
    }

    #[test]
    fn hosts_match_their_most_specific_name() {
        assert_eq!(host_scores("Apple M1 Pro").0, 5.2);
        assert_eq!(host_scores("Apple M1").0, 2.6);
        assert_eq!(host_scores("Apple M5 Pro").0, 14.5);
    }
}
