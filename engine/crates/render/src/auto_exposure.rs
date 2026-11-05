//! Automatic exposure: the camera meters each view and adapts like an eye.
//!
//! Turn it on with [`crate::Lighting::auto_exposure`]. Every view is metered
//! on its own (split screen: one player in a tunnel, one in the sun). The
//! meter and the adaptation run on the GPU after temporal reconstruction and
//! before bloom and the tone pass, so the frame on screen already uses the
//! new exposure and nothing waits for a readback. A copy of each view's
//! exposure reaches the CPU a few frames later
//! ([`crate::Renderer::exposure_ev100`]) for the HUD, the debug menu and the
//! CPU-side effects that follow exposure (emissive compensation, lens flare).
//!
//! The manual `Lighting::ev100` stays meaningful: it is where a view starts
//! before its first measurement and after [`crate::Renderer::reset_history`].
use crate::Gpu;

/// How the automatic exposure meters and adapts. All values are in EV100,
/// the same unit as `Lighting::ev100` (sunny daylight about 15, an overcast
/// day about 12, a lit room about 7, a street at night about 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoExposure {
    /// The darkest scene the camera follows: below this, a scene is shown as
    /// dark as it is. Keeps a night a night.
    pub min_ev100: f32,
    /// The brightest scene the camera follows: above this, it is shown
    /// brighter. Keeps snow and sun glare bright.
    pub max_ev100: f32,
    /// Exposure compensation in stops: +1 shows every scene twice as bright.
    pub compensation: f32,
    /// How fast the eye adapts to a brighter scene, per second (3: most of
    /// the way in about a second). 0 or less adapts at once.
    pub speed_light: f32,
    /// How fast the eye adapts to a darker scene, per second (slower than to
    /// light, as eyes are). 0 or less adapts at once.
    pub speed_dark: f32,
    /// Share of the darkest samples the meter ignores (0..1): deep shadows
    /// and the night sky should not brighten the picture.
    pub low: f32,
    /// The meter averages the samples below this share (0..1): the
    /// brightest few (the sun, specular glints) should not darken it.
    pub high: f32,
    /// 0: every part of the view counts the same; 1: the centre counts up
    /// to 16 times as much as the corners (what the player looks at).
    pub center: f32,
}

impl Default for AutoExposure {
    fn default() -> Self {
        AutoExposure { min_ev100: 2.0, max_ev100: 16.0, compensation: 0.0, speed_light: 3.0, speed_dark: 1.0, low: 0.5, high: 0.95, center: 0.5 }
    }
}

impl AutoExposure {
    /// Set a field by name (`auto_min_ev100=4`), for `Lighting::set`.
    pub fn set(&mut self, key: &str, value: f32) -> Result<(), String> {
        if !value.is_finite() { return Err(format!("{key}: not a finite number")); }
        match key {
            "auto_min_ev100" => self.min_ev100 = value,
            "auto_max_ev100" => self.max_ev100 = value,
            "auto_compensation" => self.compensation = value,
            "auto_speed_light" => self.speed_light = value,
            "auto_speed_dark" => self.speed_dark = value,
            "auto_low" if (0.0..=1.0).contains(&value) => self.low = value,
            "auto_high" if (0.0..=1.0).contains(&value) => self.high = value,
            "auto_center" if (0.0..=1.0).contains(&value) => self.center = value,
            _ => return Err(format!("unknown or out-of-range auto exposure setting {key}={value}")),
        }
        Ok(())
    }

    /// The same settings with the range in order and the shares in 0..1.
    pub fn sanitized(self) -> Self {
        let finite = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let d = Self::default();
        let (a, b) = (finite(self.min_ev100, d.min_ev100), finite(self.max_ev100, d.max_ev100));
        let low = finite(self.low, d.low).clamp(0.0, 1.0);
        AutoExposure {
            min_ev100: a.min(b), max_ev100: a.max(b),
            compensation: finite(self.compensation, 0.0).clamp(-16.0, 16.0),
            speed_light: finite(self.speed_light, d.speed_light), speed_dark: finite(self.speed_dark, d.speed_dark),
            low, high: finite(self.high, d.high).clamp(low, 1.0), center: finite(self.center, d.center).clamp(0.0, 1.0),
        }
    }
}

/// Bins and range of the luminance histogram (as in the shader).
pub const BINS: usize = 128;
pub const MIN_LOG2: f32 = -10.0;
pub const RANGE_LOG2: f32 = 32.0;
/// At most this many samples across a view (and proportionally down).
pub const SAMPLES_WIDE: u32 = 256;

/// The exposure the tone pass multiplies by for an EV100.
pub fn exposure(ev100: f32) -> f32 { 1.0 / (1.2 * 2f32.powf(ev100)) }

/// The sample grid for a view: at most [`SAMPLES_WIDE`] across, the view's
/// aspect, never more samples than pixels.
pub fn sample_grid(width: u32, height: u32) -> [u32; 2] {
    let w = width.clamp(1, SAMPLES_WIDE);
    let h = ((u64::from(height.max(1)) * u64::from(w)) / u64::from(width.max(1))).clamp(1, u64::from(height.max(1))) as u32;
    [w, h]
}

/// The meter's reading (EV100) from histogram counts, as the shader computes
/// it: the mean log luminance between the `low` and `high` shares.
pub fn metered_ev100(counts: &[u32; BINS], low: f32, high: f32) -> Option<f32> {
    let total: f64 = counts.iter().map(|&n| f64::from(n)).sum();
    let lo = total * f64::from(low.clamp(0.0, 1.0));
    let hi = (total * f64::from(high.clamp(0.0, 1.0))).max(lo + 1.0);
    let (mut below, mut sum, mut weight) = (0.0, 0.0, 0.0);
    for (i, &n) in counts.iter().enumerate() {
        let n = f64::from(n);
        let take = ((below + n).min(hi) - below.max(lo)).max(0.0);
        sum += take * f64::from(MIN_LOG2 + (i as f32 + 0.5) / BINS as f32 * RANGE_LOG2);
        weight += take;
        below += n;
    }
    (weight > 0.0).then(|| (sum / weight) as f32 + 3.0)
}

/// One adaptation step, as the shader takes it: from `ev` towards the
/// metered scene over `dt` seconds.
pub fn adapt(settings: &AutoExposure, ev: f32, metered: f32, dt: f32) -> f32 {
    let target = metered.clamp(settings.min_ev100, settings.max_ev100) - settings.compensation;
    if dt <= 0.0 { return target; }
    let speed = if target > ev { settings.speed_light } else { settings.speed_dark };
    if speed <= 0.0 { return target; }
    ev + (target - ev) * (1.0 - (-dt * speed).exp())
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    min_ev: f32, max_ev: f32, compensation: f32, dt: f32,
    speed_light: f32, speed_dark: f32, low: f32, high: f32,
    center: f32, initial_ev: f32, reset: f32, pad: f32,
    samples: [u32; 2], pad2: [u32; 2],
}

struct View {
    bins: wgpu::Buffer,
    state: wgpu::Buffer,
    params: wgpu::Buffer,
    /// Bind groups for the last few input textures (TAA alternates two).
    binds: Vec<(wgpu::Texture, wgpu::BindGroup)>,
    /// Metered on the previous frame: a view that was off starts fresh.
    active: bool,
}

/// A readback slot: every view's state, copied after the frame.
struct Slot { read: wgpu::Buffer, views: usize, pending: Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>> }

const MIRROR_SLOTS: usize = 3;
const MAX_MIRRORED_VIEWS: usize = 8;

pub(crate) struct AutoExposureGpu {
    layout: wgpu::BindGroupLayout,
    histogram: wgpu::ComputePipeline,
    adapt: wgpu::ComputePipeline,
    views: std::sync::Mutex<Vec<View>>,
    slots: std::sync::Mutex<Vec<Slot>>,
    /// The newest exposure (EV100) and meter reading that reached the CPU, per view.
    mirrored: std::sync::Mutex<Vec<Option<[f32; 2]>>>,
    /// Wall-clock time of the last metered frame (adaptation follows real time).
    clock: std::sync::Mutex<Option<web_time::Instant>>,
    reset: std::sync::atomic::AtomicBool,
}

impl AutoExposureGpu {
    pub fn new(gpu: &Gpu) -> Self {
        let storage = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("auto exposure"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            storage(1),
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            storage(3),
        ]});
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("auto exposure"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("auto exposure"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/auto_exposure.wgsl").into()) });
        let pipeline = |entry: &str| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&pipeline_layout),
            module: &module, entry_point: Some(entry), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let slots = (0..MIRROR_SLOTS).map(|_| Slot { read: gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("auto exposure readback"),
            size: 16 * MAX_MIRRORED_VIEWS as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false }), views: 0, pending: None }).collect();
        AutoExposureGpu { histogram: pipeline("histogram"), adapt: pipeline("adapt"), layout, views: Default::default(), slots: std::sync::Mutex::new(slots),
            mirrored: Default::default(), clock: Default::default(), reset: Default::default() }
    }

    /// Snap every view to its next measurement (after a cut or a teleport).
    pub fn reset(&self) { self.reset.store(true, std::sync::atomic::Ordering::Relaxed); }

    /// Seconds since the previous metered frame, at most a tenth of a second
    /// (a hitch must not jump the exposure). Called once per frame.
    pub fn frame_dt(&self) -> f32 {
        let now = web_time::Instant::now();
        let mut clock = self.clock.lock().unwrap();
        let dt = clock.map_or(0.0, |t| (now - t).as_secs_f32().min(0.1));
        *clock = Some(now);
        dt
    }

    /// Views that are not metered this frame start fresh when they are again.
    pub fn idle(&self, view: usize) {
        if let Some(v) = self.views.lock().unwrap().get_mut(view) { v.active = false; }
    }

    pub fn set_view_count(&self, count: usize) {
        self.views.lock().unwrap().truncate(count);
        self.mirrored.lock().unwrap().truncate(count);
    }

    /// Meter `input` (the view's final HDR) and adapt. Returns the state
    /// buffer whose first float is the exposure to copy into the tone,
    /// bloom and TAA uniforms.
    #[allow(clippy::too_many_arguments)]
    pub fn meter(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, view: usize, input: &wgpu::Texture,
        settings: &AutoExposure, initial_ev: f32, dt: f32, timestamps: Option<(&wgpu::QuerySet, u32)>) -> wgpu::Buffer {
        let s = settings.sanitized();
        let initial_ev = if initial_ev.is_finite() { initial_ev } else { 12.0 };
        let mut views = self.views.lock().unwrap();
        while views.len() <= view {
            let state = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("auto exposure state"), size: 16,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            views.push(View {
                bins: gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("auto exposure histogram"), size: 4 * BINS as u64,
                    usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false }),
                state, binds: Vec::new(), active: false,
                params: gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("auto exposure settings"), size: std::mem::size_of::<Params>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            });
        }
        let v = &mut views[view];
        let reset = !v.active || self.reset.load(std::sync::atomic::Ordering::Relaxed);
        if reset {
            // Never a zero exposure, even on the frames that copy the state
            // before this view's first measurement (TAA reads last frame's).
            let ev = initial_ev.clamp(s.min_ev100, s.max_ev100) - s.compensation;
            gpu.queue.write_buffer(&v.state, 0, bytemuck::cast_slice(&[exposure(ev), ev, initial_ev, 0.0f32]));
        }
        v.active = true;
        let samples = sample_grid(input.width(), input.height());
        gpu.queue.write_buffer(&v.params, 0, bytemuck::bytes_of(&Params {
            min_ev: s.min_ev100, max_ev: s.max_ev100, compensation: s.compensation, dt,
            speed_light: s.speed_light, speed_dark: s.speed_dark, low: s.low, high: s.high,
            center: s.center, initial_ev, reset: if reset { 1.0 } else { 0.0 }, pad: 0.0, samples, pad2: [0; 2] }));
        if !v.binds.iter().any(|(t, _)| t == input) {
            let source = input.create_view(&Default::default());
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("auto exposure"), layout: &self.layout, entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source) },
                wgpu::BindGroupEntry { binding: 1, resource: v.bins.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: v.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: v.state.as_entire_binding() },
            ]});
            if v.binds.len() >= 4 { v.binds.remove(0); }
            v.binds.push((input.clone(), bind));
        }
        let bind = &v.binds.iter().find(|(t, _)| t == input).unwrap().1;
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("auto exposure"),
            timestamp_writes: timestamps.map(|(query_set, i)| wgpu::ComputePassTimestampWrites { query_set,
                beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }) });
        pass.set_bind_group(0, bind, &[]);
        pass.set_pipeline(&self.histogram);
        pass.dispatch_workgroups(samples[0].div_ceil(16), samples[1].div_ceil(16), 1);
        pass.set_pipeline(&self.adapt);
        pass.dispatch_workgroups(1, 1, 1);
        drop(pass);
        v.state.clone()
    }

    /// The state buffer of a view metered on an earlier frame (TAA runs
    /// before this frame's meter and reads the previous exposure).
    pub fn previous_state(&self, view: usize) -> Option<wgpu::Buffer> {
        self.views.lock().unwrap().get(view).filter(|v| v.active).map(|v| v.state.clone())
    }

    /// Copy every metered view's state for the CPU mirror. A busy ring skips
    /// the copy instead of waiting. Call [`Self::submitted`] after the submit.
    pub fn copy_for_cpu(&self, enc: &mut wgpu::CommandEncoder, views: usize) -> bool {
        self.reset.store(false, std::sync::atomic::Ordering::Relaxed);
        let all = self.views.lock().unwrap();
        if !all.iter().take(views).any(|v| v.active) { return false; }
        let mut slots = self.slots.lock().unwrap();
        let Some(slot) = slots.iter_mut().find(|s| s.pending.is_none() && s.views == 0) else { return false };
        let n = views.min(MAX_MIRRORED_VIEWS).min(all.len());
        for (i, v) in all.iter().take(n).enumerate() { enc.copy_buffer_to_buffer(&v.state, 0, &slot.read, 16 * i as u64, 16); }
        slot.views = n;
        true
    }

    /// The frame with the copies was submitted: map them.
    pub fn submitted(&self) {
        let mut slots = self.slots.lock().unwrap();
        for slot in slots.iter_mut().filter(|s| s.views > 0 && s.pending.is_none()) {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            slot.read.slice(..).map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
            slot.pending = Some(rx);
        }
    }

    /// Take mirrored values that are ready; never waits for the GPU.
    pub fn drain(&self, gpu: &Gpu) {
        let mut slots = self.slots.lock().unwrap();
        if !slots.iter().any(|s| s.pending.is_some()) { return; }
        let _ = gpu.device.poll(wgpu::PollType::Poll);
        let mut mirrored = self.mirrored.lock().unwrap();
        for slot in slots.iter_mut() {
            let Some(rx) = &slot.pending else { continue };
            let result = match rx.try_recv() {
                Ok(result) => result.is_ok(),
                Err(std::sync::mpsc::TryRecvError::Empty) => continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
            };
            if result {
                if let Ok(bytes) = slot.read.slice(..16 * slot.views as u64).get_mapped_range() {
                    let values: &[f32] = bytemuck::cast_slice(&bytes);
                    if mirrored.len() < slot.views { mirrored.resize(slot.views, None); }
                    for (i, s) in values.chunks_exact(4).enumerate() {
                        mirrored[i] = (s[3] > 0.5 && s[1].is_finite()).then_some([s[1], s[2]]);
                    }
                }
                slot.read.unmap();
            }
            slot.pending = None;
            slot.views = 0;
        }
    }

    /// The newest exposure and meter reading (EV100) for a view that reached the CPU.
    pub fn mirrored(&self, view: usize) -> Option<[f32; 2]> { self.mirrored.lock().unwrap().get(view).copied().flatten() }

    /// Forget the mirror (auto exposure switched off).
    pub fn clear_mirror(&self) {
        self.mirrored.lock().unwrap().clear();
        for v in self.views.lock().unwrap().iter_mut() { v.active = false; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts_at(log2_luminance: f32, n: u32) -> [u32; BINS] {
        let mut c = [0; BINS];
        let bin = (((log2_luminance - MIN_LOG2) / RANGE_LOG2).clamp(0.0, 0.99999) * BINS as f32) as usize;
        c[bin] = n;
        c
    }

    #[test]
    fn shader_validates() {
        let src = include_str!("shaders/auto_exposure.wgsl");
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        assert_eq!(module.entry_points.len(), 2);
        assert_eq!(std::mem::size_of::<Params>(), 64);
    }

    #[test]
    fn a_uniform_scene_meters_to_its_ev100() {
        // Luminance 4096 (2^12): EV100 = log2(4096 * 100 / 12.5) = 15, sunny daylight.
        let ev = metered_ev100(&counts_at(12.0, 1000), 0.5, 0.95).unwrap();
        assert!((ev - 15.0).abs() < 0.13, "{ev}");
        assert_eq!(metered_ev100(&[0; BINS], 0.5, 0.95), None);
    }

    #[test]
    fn percentiles_ignore_the_sun_and_deep_shadow() {
        let mut c = counts_at(8.0, 900);
        c[BINS - 1] = 40; // a few samples of sun
        c[0] = 60; // and some black
        let ev = metered_ev100(&c, 0.1, 0.9).unwrap();
        assert!((ev - 11.0).abs() < 0.13, "the bulk of the scene decides: {ev}");
    }

    #[test]
    fn adaptation_is_bounded_compensated_and_faster_to_light() {
        let s = AutoExposure::default();
        assert_eq!(adapt(&s, 10.0, 30.0, 0.0), s.max_ev100, "the range bounds a snap");
        assert_eq!(adapt(&s, 10.0, -30.0, 0.0), s.min_ev100);
        let c = AutoExposure { compensation: 1.0, ..s };
        assert_eq!(adapt(&c, 10.0, 12.0, 0.0), 11.0, "+1 stop shows the scene brighter: one EV lower");
        let up = adapt(&s, 10.0, 12.0, 0.1) - 10.0;
        let down = 10.0 - adapt(&s, 10.0, 8.0, 0.1);
        assert!(up > down && up > 0.0 && down > 0.0, "{up} {down}");
        // Converges and never overshoots.
        let mut ev = 4.0;
        for _ in 0..600 { let next = adapt(&s, ev, 12.0, 1.0 / 60.0); assert!(next >= ev && next <= 12.0); ev = next; }
        assert!((ev - 12.0).abs() < 1e-3);
        let instant = AutoExposure { speed_dark: 0.0, ..s };
        assert_eq!(adapt(&instant, 10.0, 6.0, 0.1), 6.0);
    }

    #[test]
    fn settings_parse_and_sanitize() {
        let mut s = AutoExposure::default();
        s.set("auto_min_ev100", 9.0).unwrap();
        s.set("auto_max_ev100", 3.0).unwrap();
        assert!(s.set("auto_low", 2.0).is_err());
        assert!(s.set("nope", 1.0).is_err());
        let t = s.sanitized();
        assert_eq!((t.min_ev100, t.max_ev100), (3.0, 9.0));
        assert!(t.low <= t.high);
    }

    #[test]
    fn sample_grid_follows_the_view() {
        assert_eq!(sample_grid(1920, 1080), [256, 144]);
        assert_eq!(sample_grid(100, 50), [100, 50]);
        assert_eq!(sample_grid(960, 1080), [256, 288]);
        assert_eq!(sample_grid(0, 0), [1, 1]);
    }

    #[test]
    fn exposure_matches_the_tone_pass() {
        assert!((exposure(14.0) - 1.0 / (1.2 * 16384.0)).abs() < 1e-12);
    }
}
