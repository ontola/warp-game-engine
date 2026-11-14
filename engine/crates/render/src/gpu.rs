//! A headless GPU: no window, no surface. Everything renders into
//! textures and is read back, so the same code runs on a desktop GPU, in CI
//! and on a software rasteriser (lavapipe) in a container.

use web_time::Instant;

#[derive(Clone)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub features: wgpu::Features,
    pub limits: wgpu::Limits,
    pipeline_cache: std::sync::Arc<crate::pipeline_cache::Cache>,
}

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub driver: String,
    pub device_type: String,
    pub timestamps: bool,
}

/// The wgpu instance every window and tool starts from. On Windows only the
/// primary backends (Vulkan, DX12 for WARP): starting OpenGL as well costs
/// a cold launch seconds and is never picked there. `WGPU_BACKEND` still
/// overrides.
pub fn instance() -> wgpu::Instance { instance_for(wgpu::Backends::PRIMARY) }

/// A game window on Windows tries Vulkan alone first: creating the DX12
/// instance and its adapters as well costs every launch hundreds of ms.
/// `instance()` is the fallback when Vulkan cannot present.
pub fn window_instance() -> Option<wgpu::Instance> {
    let pinned = std::env::var_os("WGPU_BACKEND").is_some() || std::env::var_os("KERN_ADAPTER").is_some();
    if !cfg!(windows) || pinned { return None; }
    // Store overlays (Steam, Epic, GOG) hook every Vulkan app as implicit
    // layers and cost a launch up to 0.7 s; a Warp game uses none of them.
    // An explicit VK_LOADER_LAYERS_DISABLE (even empty) is kept.
    if std::env::var_os("VK_LOADER_LAYERS_DISABLE").is_none() {
        // SAFETY: read only by the Vulkan loader, at instance creation just below.
        unsafe { std::env::set_var("VK_LOADER_LAYERS_DISABLE", "~implicit~") };
    }
    Some(instance_for(wgpu::Backends::VULKAN))
}

fn instance_for(windows: wgpu::Backends) -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) { desc.backends = windows; }
    wgpu::Instance::new(desc.with_env())
}

#[cfg(test)]
thread_local! {
    /// Tests on this thread get devices with this largest texture side.
    pub(crate) static TEST_MAX_TEXTURE: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

impl Gpu {
    /// The best adapter there is. `KERN_ADAPTER=software` forces the
    /// fallback (lavapipe/WARP); any other value picks the first adapter
    /// whose name contains it.
    pub fn new() -> Result<Gpu, String> {
        let instance = instance();
        Self::request(&instance, None).map(|(gpu, _)| gpu)
    }

    /// Only an explicitly opened game window calls this. Headless tools keep
    /// selecting their adapters without creating a surface or window.
    pub fn for_surface(instance: &wgpu::Instance, surface: &wgpu::Surface<'_>, w: u32, h: u32) -> Result<(Gpu, wgpu::SurfaceConfiguration), String> {
        Self::for_surface_with_adapter(instance, surface, w, h).map(|(gpu, config, _)| (gpu, config))
    }

    /// [`Self::for_surface`], also returning the adapter: a window keeps it
    /// to query the surface's HDR support and its display
    /// ([`crate::hdr::HdrDisplay::query`]).
    pub fn for_surface_with_adapter(instance: &wgpu::Instance, surface: &wgpu::Surface<'_>, w: u32, h: u32) -> Result<(Gpu, wgpu::SurfaceConfiguration, wgpu::Adapter), String> {
        let (gpu, adapter) = Self::request(instance, Some(surface))?;
        let mut config = surface.get_default_config(&adapter, w.max(1), h.max(1)).ok_or("adapter cannot present to this window")?;
        // Presentation decodes the already-tonemapped scene into linear light.
        // An sRGB target then blends HUD coverage in linear light before encoding.
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().find(|f| f.is_srgb()) { config.format = *format; }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.desired_maximum_frame_latency = 2;
        Ok((gpu, config, adapter))
    }

    fn request(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<(Gpu, wgpu::Adapter), String> {
        pollster::block_on(Self::request_async(instance, surface))
    }

    /// [`Self::for_surface_with_adapter`] without blocking: a browser
    /// answers adapter and device requests on its own event loop.
    pub async fn for_surface_async(instance: &wgpu::Instance, surface: &wgpu::Surface<'_>, w: u32, h: u32) -> Result<(Gpu, wgpu::SurfaceConfiguration, wgpu::Adapter), String> {
        let (gpu, adapter) = Self::request_async(instance, Some(surface)).await?;
        let mut config = surface.get_default_config(&adapter, w.max(1), h.max(1)).ok_or("adapter cannot present to this canvas")?;
        // A WebGPU canvas has no sRGB format, only an sRGB view of one.
        config.view_formats = vec![config.format.add_srgb_suffix()];
        config.desired_maximum_frame_latency = 2;
        Ok((gpu, config, adapter))
    }

    async fn request_async(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<(Gpu, wgpu::Adapter), String> {
        let wanted = std::env::var("KERN_ADAPTER").ok();
        let adapter = (async {
            if let Some(w) = wanted.as_deref().filter(|w| *w != "software") {
                for a in instance.enumerate_adapters(wgpu::Backends::all()).await {
                    if a.get_info().name.to_lowercase().contains(&w.to_lowercase()) && surface.is_none_or(|s| a.is_surface_supported(s)) {
                        return Ok(a);
                    }
                }
            }
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: wanted.as_deref() == Some("software"),
                    compatible_surface: surface,
                    ..Default::default()
                })
                .await
        })
        .await
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let info = adapter.get_info();
        let optional = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS | wgpu::Features::INDIRECT_FIRST_INSTANCE | wgpu::Features::PIPELINE_CACHE | wgpu::Features::PRIMITIVE_INDEX
            // BC7 albedo, normal and material arrays (texture_stream.rs).
            | wgpu::Features::TEXTURE_COMPRESSION_BC
            | wgpu::Features::SUBGROUP;
        let features = adapter.features() & optional;
        let mut limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let al = adapter.limits();
        limits.max_storage_buffer_binding_size = al.max_storage_buffer_binding_size;
        limits.max_buffer_size = al.max_buffer_size;
        limits.max_storage_buffers_per_shader_stage = al.max_storage_buffers_per_shader_stage.min(16);
        // The shade layout samples 20 textures in a stage (the noise field,
        // voxel depth, the coarse terrain and the voxel surface beyond
        // WebGPU's 16); desktop adapters allow far more.
        limits.max_sampled_textures_per_shader_stage = al.max_sampled_textures_per_shader_stage.min(32);
        limits.max_compute_workgroups_per_dimension = al.max_compute_workgroups_per_dimension;
        #[cfg(test)]
        if let Some(side) = TEST_MAX_TEXTURE.with(|c| c.get()) { limits.max_texture_dimension_2d = side; }
        // KERN_WEBGPU_LIMITS: render natively with what a browser gives, to
        // find what a browser build would hit. Both keep only WebGPU's
        // optional timestamp, first-instance and BC compression features. `spec` is the
        // spec's default limits (every browser); `browser` is what about
        // 80 % of WebGPU devices offer (web3dsurvey.com, 2026-11): 16
        // storage buffers per stage, 32 sampled textures, 1 GiB buffers;
        // `mac` is `browser` with the 10 storage buffers most Macs give.
        let web = std::env::var("KERN_WEBGPU_LIMITS").ok();
        let (features, limits) = match web.as_deref() {
            Some(mode @ ("spec" | "browser" | "mac")) => {
                let mut web = wgpu::Limits::default();
                if mode != "spec" {
                    web.max_storage_buffers_per_shader_stage = if mode == "mac" { 10 } else { 16 };
                    web.max_sampled_textures_per_shader_stage = 32;
                    web.max_storage_buffer_binding_size = 1 << 30;
                    web.max_buffer_size = 1 << 30;
                }
                // Never above what this adapter allows.
                (features & (wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::INDIRECT_FIRST_INSTANCE | wgpu::Features::TEXTURE_COMPRESSION_BC), web.or_worse_values_from(&al))
            }
            _ => (features, limits),
        };
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("kern"),
            required_features: features,
            required_limits: limits.clone(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        };
        #[cfg(all(feature = "dlss", target_os = "windows"))]
        let native = crate::dlss::request_device(&adapter, &descriptor);
        #[cfg(not(all(feature = "dlss", target_os = "windows")))]
        let native: Option<Result<(wgpu::Device, wgpu::Queue), String>> = None;
        let (device, queue) = match native {
            Some(Ok(result)) => result,
            Some(Err(reason)) => {
                // Optional capability discovery must not disable native rendering
                // on an older driver. Explicit DLSS selection still fails in its
                // context's extension/capability validation; it never falls back.
                eprintln!("DLSS capability setup unavailable: {reason}; creating native device");
                adapter.request_device(&descriptor).await.map_err(|e| format!("device: {e}"))?
            }
            None => adapter.request_device(&descriptor).await.map_err(|e| format!("device: {e}"))?,
        };
        // Under KERN_WEBGPU_LIMITS, report each distinct GPU error once and
        // keep going, so one run lists every layout over the limits.
        if web.is_some() {
            let seen = std::sync::Mutex::new(std::collections::HashSet::new());
            device.on_uncaptured_error(std::sync::Arc::new(move |error: wgpu::Error| {
                let text = error.to_string();
                if seen.lock().map(|mut s| s.insert(text.clone())).unwrap_or(true) { eprintln!("KERN_WEBGPU_LIMITS GPU error: {text}"); }
            }));
        }
        let pipeline_cache = std::sync::Arc::new(crate::pipeline_cache::Cache::new(&device, &info));
        Ok((Gpu { device, queue, info, features, limits, pipeline_cache }, adapter))
    }

    /// Adopt a device created for an external compositor (e.g. OpenXR).
    /// The caller selects the runtime's adapter and enables its required Vulkan extensions.
    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue, info: wgpu::AdapterInfo) -> Self {
        let features = device.features();
        let limits = device.limits();
        let pipeline_cache = std::sync::Arc::new(crate::pipeline_cache::Cache::new(&device, &info));
        Self { device, queue, info, features, limits, pipeline_cache }
    }

    /// One cache is shared by all views, stage workers and pipeline variants.
    pub(crate) fn pipeline_cache(&self) -> Option<&wgpu::PipelineCache> { self.pipeline_cache.handle() }

    /// Explicit construction/reload savepoint, never called by a render frame.
    pub(crate) fn persist_pipeline_cache(&self) -> crate::pipeline_cache::Profile { self.pipeline_cache.persist() }

    pub fn summary(&self) -> GpuInfo {
        GpuInfo {
            name: self.info.name.clone(),
            backend: format!("{:?}", self.info.backend),
            driver: format!("{} {}", self.info.driver, self.info.driver_info),
            device_type: format!("{:?}", self.info.device_type),
            timestamps: self.features.contains(wgpu::Features::TIMESTAMP_QUERY),
        }
    }

    pub fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Copy an `Rgba16Float` texture to memory: four half floats per texel
    /// ([`crate::hdr::half_to_f32`]).
    pub fn read_rgba16f(&self, tex: &wgpu::Texture) -> Vec<u16> {
        let bytes = self.read_rows(tex, 8);
        bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect()
    }

    /// Copy an RGBA8 texture to memory.
    pub fn read_rgba8(&self, tex: &wgpu::Texture) -> Vec<u8> {
        let mut out = self.read_rows(tex, 4);
        if matches!(tex.format(), wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
            for pixel in out.chunks_exact_mut(4) { pixel.swap(0, 2); }
        }
        out
    }

    pub(crate) fn read_rows(&self, tex: &wgpu::Texture, texel: u32) -> Vec<u8> {
        let (w, h) = (tex.width(), tex.height());
        let row = (w * texel).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.wait();
        let data = buf.slice(..).get_mapped_range().expect("mapped readback");
        let mut out = Vec::with_capacity((w * h * texel) as usize);
        for y in 0..h {
            let start = (y * row) as usize;
            out.extend_from_slice(&data[start..start + (w * texel) as usize]);
        }
        out
    }
}

pub fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgba).map_err(|e| e.to_string())
}

/// Wall-clock timer for CPU-side stages.
pub struct Stopwatch(Instant);

impl Stopwatch {
    pub fn start() -> Stopwatch {
        Stopwatch(Instant::now())
    }
    pub fn ms(&self) -> f64 {
        self.0.elapsed().as_secs_f64() * 1e3
    }
}
