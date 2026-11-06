//! Cluster geometry in one storage buffer, so a shader stage spends one
//! binding on vertices, cluster vertex indices and triangles (the engine
//! rule is 8 storage buffers per stage, browser.md). Vertices start at byte
//! 0; the other two regions start 256-byte aligned, so a pass that writes
//! vertices (procedural terrain, skinning) or reads one region with its own
//! typed array can bind that region alone. Shade, raster and transparency
//! read the whole buffer as words through `vertex_f32(i)`, `cluster_vert(i)`
//! and `tri_word(i)`; the region starts (in words) sit in scene table vec4
//! 1.z and 1.w (scene_tables.rs).
use crate::Gpu;

const ALIGN: u64 = 256;

#[derive(Clone)]
pub(crate) struct Topology {
    pub buffer: wgpu::Buffer,
    /// (byte offset, byte size) of vertices, cluster vertices and triangles.
    pub regions: [(u64, u64); 3],
}

impl Topology {
    /// Regions of at least `sizes` bytes each, zeroed. `usage` adds to
    /// STORAGE and COPY_DST.
    pub fn new(gpu: &Gpu, label: &str, sizes: [u64; 3], usage: wgpu::BufferUsages) -> Self {
        let mut regions = [(0, 0); 3];
        let mut end = 0u64;
        for (region, size) in regions.iter_mut().zip(sizes) {
            let size = size.max(16).div_ceil(16) * 16;
            *region = (end.div_ceil(ALIGN) * ALIGN, size);
            end = region.0 + size;
        }
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: end,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | usage, mapped_at_creation: false });
        Topology { buffer, regions }
    }

    /// One buffer holding these three arrays.
    pub fn with(gpu: &Gpu, label: &str, vertices: &[u8], cluster_verts: &[u8], tris: &[u8], usage: wgpu::BufferUsages) -> Self {
        let parts = [vertices, cluster_verts, tris];
        let topology = Self::new(gpu, label, parts.map(|p| p.len() as u64), usage);
        for (i, part) in parts.iter().enumerate() { topology.write(gpu, i, 0, part); }
        topology
    }

    pub fn write(&self, gpu: &Gpu, region: usize, offset: u64, bytes: &[u8]) {
        if !bytes.is_empty() { gpu.queue.write_buffer(&self.buffer, self.regions[region].0 + offset, bytes); }
    }

    /// One region alone, as its own binding.
    pub fn region(&self, region: usize) -> wgpu::BindingResource<'_> {
        let (offset, size) = self.regions[region];
        wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.buffer, offset, size: wgpu::BufferSize::new(size) })
    }

    pub fn vertices(&self) -> wgpu::BindingResource<'_> { self.region(0) }

    /// Where cluster vertices and triangles start, in words.
    pub fn starts(&self) -> [u32; 2] { [(self.regions[1].0 / 4) as u32, (self.regions[2].0 / 4) as u32] }
}
