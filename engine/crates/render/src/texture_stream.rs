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
    let on = std::env::var("KERN_TEXTURE_COMPRESSION").as_deref() != Ok("0");
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
#[derive(Clone)]
pub(crate) struct Job {
    pub source: Option<std::sync::Arc<RgbaImage>>,
    pub array_layer: u32,
    pub settings: crate::texture_cache::Settings,
}

/// Everything a texture layer needs before its full detail can be shown:
/// its albedo, its normal map and its material map, if any.
pub(crate) struct LayerJobs { pub layer: u32, pub albedo: Job, pub normal: Vec<Job> }

struct Ready { layer: u32, albedo: (u32, Vec<Vec<u8>>), normal: Vec<(u32, Vec<Vec<u8>>)>, bytes: usize }

pub(crate) struct Stream {
    ready: std::sync::Mutex<std::sync::mpsc::Receiver<Ready>>,
    remaining: std::sync::atomic::AtomicUsize,
    encoding: [Encoding; 2],
    material_maps: Vec<Option<u32>>,
}

impl Stream {
    /// Start a worker that builds every layer's full chains.
    pub fn start(jobs: Vec<LayerJobs>, encoding: [Encoding; 2], material_maps: Vec<Option<u32>>) -> Stream {
        let (send, ready) = std::sync::mpsc::channel();
        let remaining = std::sync::atomic::AtomicUsize::new(jobs.len());
        let build = move |job: &Job, encoding: Encoding| {
            let cache = crate::texture_cache::Cache::from_env();
            let mut stats = Default::default();
            let source = job.source.as_deref();
            let (cached, key) = cache.load(source, job.settings, false, &mut stats);
            let chain = cached.unwrap_or_else(|| {
                let (chain, _) = crate::frame::texture_mips::build(source, job.settings, false, job.array_layer as usize);
                if let Some(key) = key { cache.store(key, job.settings, &chain, false, &mut stats); }
                chain
            });
            (job.array_layer, encode(chain, encoding, key, &cache))
        };
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).saturating_sub(1).clamp(1, 4);
        let queue = std::sync::Arc::new(std::sync::Mutex::new(jobs.into_iter()));
        for _ in 0..threads {
            let (queue, send) = (queue.clone(), send.clone());
            std::thread::Builder::new().name("warp texture stream".into()).spawn(move || loop {
                let Some(job) = queue.lock().unwrap().next() else { break };
                let albedo = build(&job.albedo, encoding[0]);
                let normal: Vec<_> = job.normal.iter().map(|n| build(n, encoding[1])).collect();
                let bytes = albedo.1.iter().chain(normal.iter().flat_map(|n| &n.1)).map(Vec::len).sum();
                if send.send(Ready { layer: job.layer, albedo, normal, bytes }).is_err() { break; }
            }).expect("texture stream thread");
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
        let Ok(ready) = self.ready.try_lock() else { return };
        let mut spent = 0;
        while self.remaining.load(Relaxed) > 0 && (wait || spent < BUDGET) {
            let next = if wait { ready.recv().ok() } else { ready.try_recv().ok() };
            let Some(layer) = next else { break };
            upload(gpu, albedo, layer.albedo.0, 0, &layer.albedo.1, self.encoding[0]);
            for (array_layer, levels) in &layer.normal { upload(gpu, normals, *array_layer, 0, levels, self.encoding[1]); }
            tables.write_layer_map(gpu, layer.layer as usize, crate::material_maps::layer_word(self.material_maps.get(layer.layer as usize).copied().flatten(), 0.0, 0.0));
            spent += layer.bytes;
            self.remaining.fetch_sub(1, Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
