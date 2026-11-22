//! How the albedo and normal arrays are stored and filled: BC7 block
//! compression when the device has it, and optional streaming.
//!
//! Compression: a layer's processed mips (texture_mips.rs, cached by
//! texture_cache.rs) are BC7 encoded on the CPU (block_compression's
//! fastest presets), a quarter of the RGBA8
//! memory and bandwidth. The blocks are cached beside the mips under a
//! derived key. `KERN_TEXTURE_COMPRESSION=0` keeps RGBA8; so does a device
//! without `TEXTURE_COMPRESSION_BC` (phones: no ASTC path yet) and an albedo
//! array with screen layers (`Scene::screen_layers`), which are drawn into.
//!
//! Streaming (`Scene::texture_streaming`): the renderer starts with each
//! layer's mips from `TAIL` texels down, and a worker thread makes the full
//! chains. `Stream::pump`, at the start of each frame, uploads finished
//! layers within a byte budget and lowers that layer's least resident LOD
//! in the scene tables (material_maps.rs), which every albedo and normal
//! sample clamps to (`albedo_lod_floor`, `normal_lod_floor` in shade.wgsl),
//! so no sample reads a mip that is not there yet.
use crate::Gpu;
use image::RgbaImage;

/// Smallest resident chain while streaming: mips of this size and below.
pub(crate) const TAIL: u32 = 64;
/// Bytes uploaded per frame while streaming (about 1 ms of PCIe copy).
const BUDGET: usize = 24 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoding { Rgba8, Bc7 }

/// Whether this device and environment compress the texture arrays.
pub(crate) fn compression(gpu: &Gpu) -> Encoding {
    // In a browser the BC7 encoder runs on the page's one thread, seconds
    // a layer: the arrays stay RGBA8 there.
    let on = !crate::switches::TEXTURE_COMPRESSION.off() && !cfg!(target_arch = "wasm32");
    if on && gpu.features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) { Encoding::Bc7 } else { Encoding::Rgba8 }
}

/// The GPU format for an RGBA8 array format in this encoding.
pub(crate) fn gpu_format(format: wgpu::TextureFormat, encoding: Encoding) -> wgpu::TextureFormat {
    use wgpu::TextureFormat as F;
    match (format, encoding) {
        (F::Rgba8UnormSrgb, Encoding::Bc7) => F::Bc7RgbaUnormSrgb,
        (F::Rgba8Unorm, Encoding::Bc7) => F::Bc7RgbaUnorm,
        (f, _) => f,
    }
}

/// Bytes of one mip of `size` texels square.
pub(crate) fn mip_bytes(size: u32, encoding: Encoding) -> usize {
    match encoding {
        Encoding::Rgba8 => (size * size * 4) as usize,
        Encoding::Bc7 => { let b = size.div_ceil(4) as usize; b * b * 16 }
    }
}

/// BC7 blocks of one mip; mips under 4 texels repeat their edge to a block.
fn bc7(mip: &RgbaImage, alpha: bool) -> Vec<u8> {
    let (w, h) = mip.dimensions();
    let padded;
    let src = if w % 4 != 0 || h % 4 != 0 {
        let (pw, ph) = (w.div_ceil(4) * 4, h.div_ceil(4) * 4);
        padded = RgbaImage::from_fn(pw, ph, |x, y| *mip.get_pixel(x.min(w - 1), y.min(h - 1)));
        &padded
    } else { mip };
    // The fastest presets: about 0.1 s (opaque) and 0.4 s (alpha) per
    // megatexel on one core at 41-42 dB PSNR, against 2-4 s for `fast`
    // presets that gain 1-2 dB.
    let settings = if alpha { block_compression::BC7Settings::alpha_ultrafast() } else { block_compression::BC7Settings::opaque_ultra_fast() };
    let variant = block_compression::CompressionVariant::BC7(settings);
    let mut blocks = vec![0u8; variant.blocks_byte_size(src.width(), src.height())];
    block_compression::encode::compress_rgba8(variant, src.as_raw(), &mut blocks, src.width(), src.height(), src.width() * 4);
    blocks
}

/// Upload-ready bytes of a processed chain, from its largest mip down.
/// With a cache key, compressed blocks are read from and kept in the cache.
pub(crate) fn encode(chain: Vec<RgbaImage>, encoding: Encoding, key: Option<crate::texture_cache::Key>, cache: &crate::texture_cache::Cache) -> Vec<Vec<u8>> {
    if encoding == Encoding::Rgba8 { return chain.into_iter().map(|m| m.into_raw()).collect(); }
    let sizes: Vec<usize> = chain.iter().map(|m| mip_bytes(m.width(), encoding)).collect();
    let alpha = chain.first().is_some_and(|m| m.pixels().any(|p| p[3] != 255));
    let key = key.map(|k| k.derived(if alpha { b"bc7 alpha_ultrafast v1" } else { b"bc7 opaque_ultra_fast v1" }));
    if let Some(blocks) = key.and_then(|k| cache.load_blocks(k, sizes.iter().sum())) {
        let mut at = 0;
        return sizes.iter().map(|&n| { at += n; blocks[at - n..at].to_vec() }).collect();
    }
    let levels: Vec<Vec<u8>> = chain.iter().map(|m| bc7(m, alpha)).collect();
    if let Some(key) = key { cache.store_blocks(key, &levels.concat()); }
    levels
}

/// Write `levels` (mips from `first`) of array layer `layer`.
pub(crate) fn upload(gpu: &Gpu, tex: &wgpu::Texture, layer: u32, first: u32, levels: &[Vec<u8>], encoding: Encoding) {
    let size = tex.width();
    for (i, data) in levels.iter().enumerate() {
        let mip = first + i as u32;
        let s = (size >> mip).max(1);
        let (bytes_per_row, rows, extent) = match encoding {
            Encoding::Rgba8 => (4 * s, s, s),
            Encoding::Bc7 => (16 * s.div_ceil(4), s.div_ceil(4), s.div_ceil(4) * 4),
        };
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: mip, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: Some(rows) },
            wgpu::Extent3d { width: extent, height: extent, depth_or_array_layers: 1 },
        );
    }
}

/// One array's share of a texture layer's work.
pub(crate) struct Job {
    pub source: Option<RgbaImage>,
    pub array_layer: u32,
    pub settings: crate::texture_cache::Settings,
}

impl Job {
    fn build(self, encoding: Encoding) -> (u32, Vec<Vec<u8>>) {
        let cache = crate::texture_cache::Cache::from_env();
        let mut stats = Default::default();
        let source = self.source.as_ref();
        let (cached, key) = cache.load(source, self.settings, false, &mut stats);
        let chain = cached.unwrap_or_else(|| {
            let (chain, _t) = crate::frame::texture_mips::build(source, self.settings, cfg!(target_arch = "wasm32"), self.array_layer as usize);
            #[cfg(target_arch = "wasm32")]
            DETAIL.with(|d| { use std::fmt::Write; let _ = write!(d.borrow_mut(), " [{}px from {}x{}: resize {:.0}, alpha {:.0}, mips {:.0}, coverage {:.0}, copies {:.0}, linear rgb {:.0} ms]",
                self.settings.size, _t.source_size[0], _t.source_size[1], _t.base_resize_ms, _t.alpha_analysis_ms, _t.mip_resize_ms, _t.coverage_ms, _t.chain_copy_ms, _t.linear_rgb_ms); });
            if let Some(key) = key { cache.store(key, self.settings, &chain, false, &mut stats); }
            chain
        });
        drop(self.source);
        #[cfg(target_arch = "wasm32")]
        let t = web_time::Instant::now();
        let encoded = encode(chain, encoding, key, &cache);
        #[cfg(target_arch = "wasm32")]
        DETAIL.with(|d| { use std::fmt::Write; let _ = write!(d.borrow_mut(), " encode {} ms", t.elapsed().as_millis()); });
        (self.array_layer, encoded)
    }
}

/// Upload the albedo, normal, and optional material map together before
/// lowering the layer's LOD floor.
pub(crate) struct LayerJobs {
    pub layer: u32,
    pub generate: Option<crate::scene::GeneratedTexture>,
    pub albedo: Job,
    pub normal: Job,
    pub material: Option<Job>,
}

impl LayerJobs {
    pub fn build(mut self, encoding: [Encoding; 2]) -> Ready {
        if let Some(generate) = self.generate {
            #[cfg(target_arch = "wasm32")]
            let t = web_time::Instant::now();
            let (albedo, normal) = generate();
            #[cfg(target_arch = "wasm32")]
            DETAIL.with(|d| { use std::fmt::Write; let _ = write!(d.borrow_mut(), " generate {} ms", t.elapsed().as_millis()); });
            self.albedo.source = Some(albedo);
            self.normal.source = Some(normal);
        }
        Ready {
            layer: self.layer,
            albedo: self.albedo.build(encoding[0]),
            normal: self.normal.build(encoding[1]),
            material: self.material.map(|job| job.build(encoding[1])),
        }
    }
}

pub(crate) struct Ready {
    layer: u32,
    albedo: (u32, Vec<Vec<u8>>),
    normal: (u32, Vec<Vec<u8>>),
    material: Option<(u32, Vec<Vec<u8>>)>,
}

impl Ready {
    pub fn upload(&self, gpu: &Gpu, albedo: &wgpu::Texture, normals: &wgpu::Texture, encoding: [Encoding; 2]) {
        upload(gpu, albedo, self.albedo.0, 0, &self.albedo.1, encoding[0]);
        upload(gpu, normals, self.normal.0, 0, &self.normal.1, encoding[1]);
        if let Some((layer, levels)) = &self.material {
            upload(gpu, normals, *layer, 0, levels, encoding[1]);
        }
    }

    fn bytes(&self) -> usize {
        self.albedo.1.iter().chain(&self.normal.1).chain(self.material.iter().flat_map(|(_, levels)| levels)).map(Vec::len).sum()
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static SLOW: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Where the current layer's time went.
    static DETAIL: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}
/// Lines about layers that took 50 ms or more to bake on the page thread.
#[cfg(target_arch = "wasm32")]
pub fn take_slow_lines() -> Vec<String> { SLOW.with(|l| std::mem::take(&mut *l.borrow_mut())) }

/// A browser bakes a generated layer (a Warp Human's face) once per page:
/// a renderer rebuilt for a new look mid-game reuses it.
#[cfg(target_arch = "wasm32")]
fn generated_memo(job: LayerJobs, encoding: [Encoding; 2]) -> std::rc::Rc<Ready> {
    use std::{cell::RefCell, rc::Rc};
    thread_local! {
        static BAKED: RefCell<Vec<(crate::scene::GeneratedTexture, u32, Rc<Ready>)>> = const { RefCell::new(Vec::new()) };
    }
    let Some(recipe) = job.generate.clone() else { return Rc::new(job.build(encoding)) };
    let (layer, albedo_at, normal_at) = (job.layer, job.albedo.array_layer, job.normal.array_layer);
    let found = BAKED.with(|b| b.borrow().iter().find(|(r, l, _)| std::sync::Arc::ptr_eq(r, &recipe) && *l == layer).map(|(_, _, ready)| ready.clone()));
    if let Some(ready) = found && ready.albedo.0 == albedo_at && ready.normal.0 == normal_at { return ready; }
    let ready = Rc::new(job.build(encoding));
    BAKED.with(|b| {
        let mut b = b.borrow_mut();
        b.retain(|(r, l, _)| !(std::sync::Arc::ptr_eq(r, &recipe) && *l == layer));
        // Faces of a few games, not every face of a long session.
        if b.len() >= 48 { b.remove(0); }
        b.push((recipe, layer, ready.clone()));
    });
    ready
}

pub(crate) struct Stream {
    ready: std::sync::Mutex<std::sync::mpsc::Receiver<Ready>>,
    /// A browser has no threads: `pump` builds these one per frame.
    #[cfg(target_arch = "wasm32")]
    todo: std::sync::Mutex<std::collections::VecDeque<LayerJobs>>,
    remaining: std::sync::atomic::AtomicUsize,
    encoding: [Encoding; 2],
    material_maps: Vec<Option<u32>>,
}

impl Stream {
    /// Start a worker that builds every layer's full chains.
    pub fn start(jobs: Vec<LayerJobs>, encoding: [Encoding; 2], material_maps: Vec<Option<u32>>) -> Stream {
        #[cfg(target_arch = "wasm32")]
        {
            let (_, ready) = std::sync::mpsc::sync_channel(1);
            let remaining = std::sync::atomic::AtomicUsize::new(jobs.len());
            // Generated layers (faces) first: their previews are the coarsest.
            let (generated, regular): (Vec<_>, Vec<_>) = jobs.into_iter().partition(|job| job.generate.is_some());
            let mut todo: std::collections::VecDeque<_> = generated.into();
            todo.extend(regular);
            return Stream { ready: std::sync::Mutex::new(ready), todo: std::sync::Mutex::new(todo), remaining, encoding, material_maps };
        }
        #[cfg(not(target_arch = "wasm32"))]
        Self::start_threads(jobs, encoding, material_maps)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start_threads(jobs: Vec<LayerJobs>, encoding: [Encoding; 2], material_maps: Vec<Option<u32>>) -> Stream {
        // A paused renderer must not accumulate full mip chains for every layer.
        let (send, ready) = std::sync::mpsc::sync_channel(1);
        let remaining = std::sync::atomic::AtomicUsize::new(jobs.len());
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).saturating_sub(1).clamp(1, 4);
        let (generated, regular): (Vec<_>, Vec<_>) = jobs.into_iter().partition(|job| job.generate.is_some());
        // Procedural generators may parallelize internally and need scratch
        // space. Only one generates or sends a completed pair at a time.
        let ordinary_threads = threads.saturating_sub(usize::from(!generated.is_empty())).max(1);
        for (jobs, workers) in [(generated, 1), (regular, ordinary_threads)] {
            let count = workers.min(jobs.len());
            let queue = std::sync::Arc::new(std::sync::Mutex::new(jobs.into_iter()));
            for _ in 0..count {
                let (queue, send) = (queue.clone(), send.clone());
                std::thread::Builder::new().name("warp texture stream".into()).spawn(move || loop {
                    let Some(job) = queue.lock().unwrap().next() else { break };
                    if send.send(job.build(encoding)).is_err() { break; }
                }).expect("texture stream thread");
            }
        }
        Stream { ready: std::sync::Mutex::new(ready), remaining, encoding, material_maps }
    }

    /// Texture layers whose full chains are not uploaded yet.
    pub fn remaining(&self) -> usize { self.remaining.load(std::sync::atomic::Ordering::Relaxed) }

    /// Upload finished layers, up to the per-frame budget (`wait`: all of
    /// them, blocking until the worker is done).
    pub fn pump(&self, gpu: &Gpu, albedo: &wgpu::Texture, normals: &wgpu::Texture, tables: &crate::scene_tables::SceneTables, wait: bool) {
        use std::sync::atomic::Ordering::Relaxed;
        if self.remaining() == 0 { return; }
        #[cfg(target_arch = "wasm32")]
        {
            // One layer a frame, so the page keeps drawing; all of them for a capture.
            let Ok(mut todo) = self.todo.try_lock() else { return };
            while let Some(job) = todo.pop_front() {
                let t = web_time::Instant::now();
                let generated = job.generate.is_some();
                let layer = generated_memo(job, self.encoding);
                let ms = t.elapsed().as_millis();
                let detail = DETAIL.with(|d| std::mem::take(&mut *d.borrow_mut()));
                if ms >= 50 { SLOW.with(|l| l.borrow_mut().push(format!("texture layer {} {}baked in {ms} ms ({} left):{detail}", layer.layer, if generated { "(generated) " } else { "" }, todo.len()))); }
                layer.upload(gpu, albedo, normals, self.encoding);
                tables.write_layer_map(gpu, layer.layer as usize, crate::material_maps::layer_word(self.material_maps.get(layer.layer as usize).copied().flatten(), 0.0, 0.0));
                self.remaining.fetch_sub(1, Relaxed);
                gpu.queue.submit([]);
                if !wait { break; }
            }
            return;
        }
        #[allow(unreachable_code)]
        let Ok(ready) = self.ready.try_lock() else { return };
        let mut spent = 0;
        while self.remaining.load(Relaxed) > 0 && (wait || spent < BUDGET) {
            let next = if wait { ready.recv().ok() } else { ready.try_recv().ok() };
            let Some(layer) = next else { break };
            layer.upload(gpu, albedo, normals, self.encoding);
            tables.write_layer_map(gpu, layer.layer as usize, crate::material_maps::layer_word(self.material_maps.get(layer.layer as usize).copied().flatten(), 0.0, 0.0));
            spent += layer.bytes();
            if wait && spent >= BUDGET {
                // Blocking capture drains the queue without frame submissions.
                gpu.queue.submit([]);
                spent = 0;
            }
            self.remaining.fetch_sub(1, Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedural_layer_streams_the_same_maps_as_an_eager_upload() {
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
        let bakes = Arc::new(AtomicUsize::new(0));
        let image = |color| RgbaImage::from_pixel(4, 4, image::Rgba(color));
        let albedo = image([33, 44, 55, 128]);
        let normal = image([128, 140, 245, 98]);
        let material = image([211, 10, 0, 255]);
        let settings = crate::texture_cache::Settings {
            size: 4, format: wgpu::TextureFormat::Rgba8Unorm, fill: [255; 4],
            preserve_alpha: false, linear_cutout_rgb: false,
        };
        let job = |layer, source| Job { array_layer: layer, settings, source };
        let layer = |generate, a, n| LayerJobs {
            layer: 2, generate, albedo: job(2, a), normal: job(2, n),
            material: Some(job(5, Some(material.clone()))),
        };
        let encodings = [Encoding::Rgba8; 2];
        let expected = layer(None, Some(albedo.clone()), Some(normal.clone())).build(encodings);
        let calls = bakes.clone();
        let generated = layer(Some(Arc::new(move || {
            calls.fetch_add(1, Ordering::Relaxed);
            (albedo.clone(), normal.clone())
        })), None, None);
        assert_eq!(bakes.load(Ordering::Relaxed), 0);
        let stream = Stream::start(vec![generated], encodings, vec![]);
        let ready = stream.ready.lock().unwrap().recv().unwrap();
        assert_eq!(bakes.load(Ordering::Relaxed), 1);
        assert_eq!(ready.albedo, expected.albedo);
        assert_eq!(ready.normal, expected.normal);
        assert_eq!(ready.material, expected.material);
    }

    #[test]
    #[ignore = "exclusive headless GPU test"]
    fn gpu_gameplay_streams_only_generated_layers_and_matches_full_textures() {
        use crate::{Camera, Lighting, Renderer, Scene, scene::Material};
        use glam::{Affine3A, Quat, Vec3};
        use std::sync::{Arc, Mutex, mpsc};

        let gpu = Gpu::new().unwrap();
        let mut scene = Scene::default();
        let solid = |color| RgbaImage::from_pixel(64, 64, image::Rgba(color));
        let ordinary = scene.add_image("shirt", solid([15, 220, 20, 255]));
        let face = scene.add_image("face", solid([230, 20, 15, 128]));
        scene.textures[face as usize].normal = Some(solid([128, 128, 255, 255]));
        for (layer, x) in [(ordinary, -0.7), (face, 0.7)] {
            let flags = if layer == face { crate::scene::SKIN | crate::scene::HAIR } else { 0 };
            let mat = scene.add_material(Material { layer, flags, tint: [1.; 3], scale: 1., roughness: 0.5, metal: 0. });
            let mesh = scene.add_mesh(&crate::primitives::quad(mat).mesh());
            scene.add_instance(mesh, Affine3A::from_scale_rotation_translation(
                Vec3::splat(1.2), Quat::IDENTITY, Vec3::new(x, 0., 0.),
            ));
        }
        let maps = move || (solid([20, 35, 225, 128]), solid([200, 110, 235, 210]));
        let (resume, waiting) = mpsc::channel();
        let waiting = Mutex::new(waiting);
        scene.generated_textures.insert(face as u32, Arc::new(move || {
            // Hold the full maps until the preview frame has been read back.
            waiting.lock().unwrap().recv_timeout(std::time::Duration::from_secs(120)).unwrap();
            maps()
        }));
        let cam = Camera { pos: Vec3::new(0., 0., 2.5), target: Vec3::ZERO, near: 0.05, fov_y: 0.75 };
        let light = Lighting { taa: 0., sharpen: 0., ..Default::default() };
        let capture = |renderer: &Renderer, debug| {
            renderer.reset_capture_sampling();
            renderer.render(&gpu, &cam, &Lighting { debug, ..light.clone() }, 128, 64).0
        };
        let pixel = |frame: &[u8], x: usize| -> [u8; 3] {
            let start: usize = 4 * (128 * 32 + x);
            frame[start..start + 3].try_into().unwrap()
        };
        assert!(!scene.texture_streaming);
        let streamed = Renderer::with_generated_streaming(&gpu, &scene, &light, true);
        assert_eq!(streamed.textures_streaming(), 1);
        let preview = capture(&streamed, 2.);
        let [r, g, b] = pixel(&preview, 87);
        assert!(r > g && r > b, "the face shows its red preview: {r} {g} {b}");
        let [r, g, b] = pixel(&preview, 41);
        assert!(g > r && g > b, "the ordinary shirt is green: {r} {g} {b}");
        resume.send(()).unwrap();
        streamed.finish_texture_streaming(&gpu);
        assert_eq!(streamed.textures_streaming(), 0);
        let actual = [capture(&streamed, 2.), capture(&streamed, 11.)];
        let [r, g, b] = pixel(&actual[0], 87);
        assert!(b > r && b > g, "the face shows its blue full map: {r} {g} {b}");
        drop(streamed);

        scene.generated_textures.clear();
        let (color, normal) = maps();
        scene.textures[face as usize].rgba = color;
        scene.textures[face as usize].normal = Some(normal);
        let eager = Renderer::new(&gpu, &scene, &light);
        for (frame, debug) in actual.iter().zip([2., 11.]) {
            let reference = capture(&eager, debug);
            let differences = frame.iter().zip(&reference).filter(|(a, b)| a != b).count();
            assert_eq!(differences, 0, "debug view {debug} differs from the eager upload");
        }
    }

    #[test]
    fn bc7_sizes_cover_every_mip() {
        for (size, bytes) in [(2048, 2048 * 2048), (4, 16), (2, 16), (1, 16)] {
            assert_eq!(mip_bytes(size, Encoding::Bc7), bytes);
        }
        let mip = RgbaImage::from_fn(2, 2, |x, y| image::Rgba([x as u8 * 200, y as u8 * 200, 50, 255]));
        assert_eq!(bc7(&mip, false).len(), 16);
    }

    #[test]
    fn bc7_round_trip_is_close() {
        let mip = RgbaImage::from_fn(64, 64, |x, y| image::Rgba([(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8, if (x / 8 + y / 8) % 2 == 0 { 255 } else { 0 }]));
        let blocks = bc7(&mip, true);
        let mut out = vec![0u8; 64 * 64 * 4];
        block_compression::decode::decompress_blocks_as_rgba8(block_compression::CompressionVariant::BC7(block_compression::BC7Settings::alpha_fast()), 64, 64, &blocks, &mut out);
        let worst = mip.as_raw().iter().zip(&out).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 12, "worst channel error {worst}");
    }
}
