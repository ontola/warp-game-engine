//! Cull's read-only tables in one storage buffer, so the cull pipelines spend
//! one binding on them (the engine rule is 8 storage buffers per stage,
//! browser.md): the cluster BVH (cluster_traversal.rs), the spatial group
//! members, the static/dynamic sun owner lists (static_sun.rs) and the
//! refitted spatial groups (cull_groups.rs). Each is a copy of its owner's
//! buffer: the first two once, sun owners when they change, groups after
//! each refit. Vec4 0 holds where each starts, in words (groups in vec4s);
//! vec4 1.x the member word count.
//! Read in cull.wgsl through cluster_tree_at(i), spatial_member(i),
//! sun_owner(i), spatial_header() and spatial_group(i).
use crate::Gpu;

pub(crate) const TREE: usize = 0;
pub(crate) const MEMBERS: usize = 1;
pub(crate) const SUN_OWNERS: usize = 2;
pub(crate) const GROUPS: usize = 3;

pub(crate) struct CullTables {
    pub buffer: wgpu::Buffer,
    regions: [u64; 4],
}

impl CullTables {
    pub fn new(gpu: &Gpu, sources: [&wgpu::Buffer; 4]) -> Self {
        let mut regions = [0u64; 4];
        let mut end = 256u64;
        for (region, source) in regions.iter_mut().zip(sources) {
            *region = end;
            end = (end + source.size()).div_ceil(256) * 256;
        }
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("cull tables"), size: end,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let header = [(regions[0] / 4) as u32, (regions[1] / 4) as u32, (regions[2] / 4) as u32, (regions[3] / 16) as u32,
            (sources[MEMBERS].size() / 4) as u32, 0, 0, 0];
        gpu.queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&header));
        let tables = CullTables { buffer, regions };
        let mut enc = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cull table copies") });
        for (i, source) in sources.iter().enumerate() { tables.copy(&mut enc, i, source); }
        gpu.queue.submit([enc.finish()]);
        tables
    }

    /// Copy `source` (whole, the size given at creation) into region `region`.
    pub fn copy(&self, enc: &mut wgpu::CommandEncoder, region: usize, source: &wgpu::Buffer) {
        enc.copy_buffer_to_buffer(source, 0, &self.buffer, self.regions[region], source.size());
    }
}
