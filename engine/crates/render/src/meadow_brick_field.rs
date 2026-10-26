//! Bounded current-pose meadow optical bricks. Root placement and deformation
//! are shared with source geometry; this is not an atlas or an opaque proxy.
//! Samples are quadrature points, not exact triangle/voxel intersections.
//! A failed brick is never published: its caller must retain reference grass.
use crate::{foliage_volume_data::GpuLevel, meadow_voxel_samples::MeadowVoxelSamples, Gpu};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

pub const MIPS: usize = 4;
pub const MAX_BRICKS: usize = 8;
pub const MAX_OUTPUT_CELLS: u32 = 262_144;
pub const MAX_SAMPLE_JOBS: u32 = 8_388_608;
pub const MAX_GPU_BYTES: u64 = 128 * 1024 * 1024;
/// Error bits: support bound, vertical clipping, list capacity, nonfinite,
/// source range, root-spacing mismatch. Any bit requires whole-brick fallback.
pub const STATUS_WORDS: usize = 4;
pub const PASS_NAMES: [&str; 8] = ["meadow_brick_clear", "meadow_brick_roots", "meadow_brick_bin", "meadow_brick_reduce",
    "meadow_brick_mip1", "meadow_brick_mip2", "meadow_brick_mip3", "meadow_brick_publish"];

#[derive(Clone, Copy, Debug)]
pub struct BrickSpec {
    /// Lower signed world-root cell. Interiors are eight cells wide in X/Z.
    pub root_origin: [i32; 2],
    pub lower_y: f32,
    /// All dimensions must be divisible by eight, so four mips are exact.
    pub cells_y: u32,
    pub cells_xz: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Group { roots: [i32; 4], work: [u32; 4] }
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    control: [u32; 4],
    memory: [u32; 4],
    physical: [f32; 4],
    ranges: [[u32; 4]; 3],
}
#[derive(Clone)]
pub struct BrickPlan {
    pub levels: Vec<GpuLevel>,
    pub specs: Vec<BrickSpec>,
    pub root_spacing: f32,
    pub root_support_radius: f32,
    pub halo_roots: u32,
    pub roots_per_brick: u32,
    pub sample_jobs: u32,
    pub total_cells: u32,
    pub max_variant_samples: u32,
    groups: Vec<Group>,
}
impl BrickPlan {
    pub fn new(specs: Vec<BrickSpec>, root_spacing: f32, root_support_radius: f32,
               max_variant_samples: u32) -> Result<Self, String> {
        if specs.is_empty() || specs.len() > MAX_BRICKS || !root_spacing.is_finite()
            || root_spacing <= 0.0 || !root_support_radius.is_finite()
            || root_support_radius < 0.0 || max_variant_samples == 0 {
            return Err("invalid meadow brick count, spacing, support or samples".into());
        }
        // The nearest omitted root centre is (halo + .46) cells from an
        // interior edge. ceil(radius/spacing) therefore includes every source
        // support even with the existing bounded .04-cell centre jitter.
        let halo = (root_support_radius / root_spacing).ceil() as u32;
        if halo > 8 { return Err("meadow brick root halo exceeds bounded work".into()); }
        let dimension = 8 + 2 * halo;
        let roots_per_brick = dimension * dimension;
        let jobs = u64::from(roots_per_brick) * specs.len() as u64 * u64::from(max_variant_samples);
        if jobs > u64::from(MAX_SAMPLE_JOBS) { return Err("meadow brick sample-job budget exceeded".into()); }
        let mut levels = Vec::new();
        let mut groups = Vec::new();
        let mut total = 0u64;
        for (i, spec) in specs.iter().enumerate() {
            if !spec.lower_y.is_finite() || spec.cells_y == 0 || spec.cells_xz == 0
                || spec.cells_y % 8 != 0 || spec.cells_xz % 8 != 0
                || spec.cells_y > 256 || spec.cells_xz > 256
                || spec.root_origin.iter().any(|v| v.rem_euclid(8) != 0) {
                return Err("brick dimensions must be positive multiples of eight, <=256".into());
            }
            for other in &specs[..i] {
                if (i64::from(spec.root_origin[0]) - i64::from(other.root_origin[0])).abs() < 8
                    && (i64::from(spec.root_origin[1]) - i64::from(other.root_origin[1])).abs() < 8 {
                    return Err("brick interiors overlap: optical mass would be duplicated".into());
                }
            }
            // Very large f32 world coordinates cannot retain this cell grid.
            if spec.root_origin.iter().any(|v| v.unsigned_abs() > 1_000_000) {
                return Err("brick origin exceeds supported world precision".into());
            }
            let cell = root_spacing * 8.0 / spec.cells_xz as f32;
            for mip in 0..MIPS {
                let dims = [spec.cells_xz >> mip, spec.cells_y >> mip, spec.cells_xz >> mip];
                let count = u64::from(dims[0]) * u64::from(dims[1]) * u64::from(dims[2]);
                if total + count > u64::from(MAX_OUTPUT_CELLS) {
                    return Err("meadow brick dense-cell budget exceeded".into());
                }
                levels.push(GpuLevel { origin_cell: [spec.root_origin[0] as f32 * root_spacing,
                    spec.lower_y, spec.root_origin[1] as f32 * root_spacing, cell * (1u32 << mip) as f32],
                    dims_offset: [dims[0], dims[1], dims[2], total as u32] });
                total += count;
            }
            groups.push(Group { roots: [spec.root_origin[0] - halo as i32,
                spec.root_origin[1] - halo as i32, dimension as i32, (i as u32 * roots_per_brick) as i32],
                work: [i as u32 * roots_per_brick * max_variant_samples, max_variant_samples, (i * MIPS) as u32, 0] });
        }
        Ok(Self { levels, specs, root_spacing, root_support_radius, halo_roots: halo,
            roots_per_brick, sample_jobs: jobs as u32, total_cells: total as u32,
            max_variant_samples, groups })
    }
    pub fn root_count(&self) -> u32 { self.roots_per_brick * self.specs.len() as u32 }
    fn scratch_words(&self) -> u64 { u64::from(self.total_cells) * 2 + u64::from(self.sample_jobs) }
    /// Includes source samples, output, retained roots, heads, links and area
    /// moments. Existing terrain/recipe/view buffers are borrowed, not counted.
    pub fn allocation_bytes(&self, sample_bytes: u64) -> u64 {
        sample_bytes + u64::from(self.total_cells) * 176 + self.scratch_words() * 4
            + u64::from(self.root_count()) * 128 + (self.levels.len() * 32) as u64
            + (self.groups.len() * 48) as u64 + std::mem::size_of::<Params>() as u64
    }
}

pub struct MeadowBrickField {
    plan: BrickPlan,
    params_value: Params,
    params: wgpu::Buffer,
    levels: wgpu::Buffer,
    cells: wgpu::Buffer,
    status: wgpu::Buffer,
    bind: wgpu::BindGroup,
    pipelines: [wgpu::ComputePipeline; 8],
    bytes: u64,
}
pub fn shader_source() -> String {
    // The producer needs only the shared storage ABI. Later optics helpers
    // depend on the lighting module, which this compute cook does not use.
    let volume_abi = include_str!("shaders/foliage_volume.wgsl")
        .split("// This specializes only private interpolation state.").next().unwrap();
    format!("{}\n{}\n{}", crate::meadow::generate_source(),
        volume_abi,
        include_str!("shaders/meadow_brick_field.wgsl"))
}
impl MeadowBrickField {
    /// Generation/view layouts and bindings are the same ones used by meadow
    /// source placement. The producer has no camera-dependent root selection.
    pub fn new(gpu: &Gpu, source: &MeadowVoxelSamples, plan: BrickPlan,
        generation_layout: &wgpu::BindGroupLayout, view_layout: &wgpu::BindGroupLayout) -> Result<Self, String> {
        // Public plan diagnostics are editable by callers; reconstruct all
        // derived offsets and caps rather than trusting modified vectors.
        let plan = BrickPlan::new(plan.specs, plan.root_spacing, plan.root_support_radius, plan.max_variant_samples)?;
        if source.ranges.iter().any(|r| r[1] == 0 || r[1] > plan.max_variant_samples
            || u64::from(r[0]) + u64::from(r[1]) > source.samples.len() as u64) {
            return Err("sample ranges do not fit the brick plan".into());
        }
        if source.samples.iter().any(|s| !bytemuck::cast_slice::<_, f32>(std::slice::from_ref(s)).iter().all(|v| v.is_finite())
            || s.position_area[3] <= 0.0 || s.anchor_branch[3].fract() != 0.0
            || !(0.0..4.0).contains(&s.anchor_branch[3])) {
            return Err("invalid source quadrature payload".into());
        }
        let bytes = plan.allocation_bytes((source.samples.len() * 80) as u64);
        if bytes > MAX_GPU_BYTES { return Err("meadow brick GPU allocation exceeds 128 MiB".into()); }
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
        let buffer = |label, size| gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label), size, usage: storage, mapped_at_creation: false });
        let upload = |label, contents: &[u8]| gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label), contents, usage: storage });
        let levels = upload("meadow brick levels", bytemuck::cast_slice(&plan.levels));
        let groups = upload("meadow brick world interiors", bytemuck::cast_slice(&plan.groups));
        let samples = upload("meadow brick source quadrature", bytemuck::cast_slice(&source.samples));
        let cells = buffer("meadow brick dense optical cells", u64::from(plan.total_cells) * 176);
        let scratch = buffer("meadow brick heads links and area", plan.scratch_words() * 4);
        let roots = buffer("meadow brick exact current roots", u64::from(plan.root_count()) * 128);
        let status = buffer("meadow brick generation status", plan.specs.len() as u64 * 16);
        for size in [u64::from(plan.total_cells) * 176, plan.scratch_words() * 4,
                     (source.samples.len() * 80) as u64] {
            if size > u64::from(gpu.limits.max_storage_buffer_binding_size) {
                return Err("meadow brick buffer exceeds device binding limit".into());
            }
        }
        let params_value = Params { control: [0, plan.specs.len() as u32, plan.root_count(), plan.sample_jobs],
            memory: [plan.total_cells, plan.total_cells, plan.total_cells + plan.sample_jobs, 8192],
            physical: [plan.root_spacing, plan.root_support_radius, plan.max_variant_samples as f32, plan.roots_per_brick as f32],
            ranges: source.ranges };
        let params = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meadow brick frame parameters"), contents: bytemuck::bytes_of(&params_value),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let entries: Vec<_> = (0..8).map(|binding| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: if binding == 0 { wgpu::BufferBindingType::Uniform }
                else { wgpu::BufferBindingType::Storage { read_only: matches!(binding, 1 | 2 | 3) } },
                has_dynamic_offset: false, min_binding_size: None }, count: None }).collect();
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meadow brick producer"), entries: &entries });
        let buffers = [&params, &groups, &samples, &levels, &cells, &status, &scratch, &roots];
        let bindings: Vec<_> = buffers.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry {
            binding: i as u32, resource: b.as_entire_binding() }).collect();
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow brick producer"), layout: &layout, entries: &bindings });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meadow brick pipeline"), bind_group_layouts: &[Some(generation_layout), Some(view_layout), Some(&layout)], immediate_size: 0 });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meadow brick source-derived production"), source: wgpu::ShaderSource::Wgsl(shader_source().into()) });
        let pipelines = ["brick_clear", "brick_roots", "brick_bin", "brick_reduce", "brick_mip1", "brick_mip2", "brick_mip3", "brick_publish"]
            .map(|entry| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry), layout: Some(&pipeline_layout), module: &module, entry_point: Some(entry),
                compilation_options: Default::default(), cache: gpu.pipeline_cache() }));
        Ok(Self { plan, params_value, params, levels, cells, status, bind, pipelines, bytes })
    }
    pub fn levels(&self) -> &[GpuLevel] { &self.plan.levels }
    pub fn plan(&self) -> &BrickPlan { &self.plan }
    pub fn levels_buffer(&self) -> &wgpu::Buffer { &self.levels }
    pub fn cells_buffer(&self) -> &wgpu::Buffer { &self.cells }
    pub fn status_buffer(&self) -> &wgpu::Buffer { &self.status }
    pub fn gpu_bytes(&self) -> u64 { self.bytes }
    /// Exactly once per submission/frame. The caller updates the borrowed
    /// meadow settings first; generation zero is reserved for never-ready.
    pub fn encode(&self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder,
        generation: &wgpu::BindGroup, view: &wgpu::BindGroup, generation_id: u32) -> Result<(), String> {
        self.encode_profiled(gpu, encoder, generation, view, generation_id, [None; 8])
    }
    /// Optional beginning/end timestamp pairs in PASS_NAMES order. Each
    /// supplied index consumes two queries; the caller resolves asynchronously.
    pub fn encode_profiled(&self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder,
        generation: &wgpu::BindGroup, view: &wgpu::BindGroup, generation_id: u32,
        times: [Option<(&wgpu::QuerySet, u32)>; 8]) -> Result<(), String> {
        if generation_id == 0 { return Err("brick generation zero is reserved".into()); }
        let mut params = self.params_value; params.control[0] = generation_id;
        gpu.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let max_mip = |mip| self.plan.levels.iter().skip(mip).step_by(MIPS)
            .map(|l| l.dims_offset[0] * l.dims_offset[1] * l.dims_offset[2]).max().unwrap();
        let jobs = [self.plan.total_cells.max(self.plan.specs.len() as u32), self.plan.root_count(),
            self.plan.sample_jobs, max_mip(0), max_mip(1), max_mip(2), max_mip(3), self.plan.specs.len() as u32];
        for (i, pipeline) in self.pipelines.iter().enumerate() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(PASS_NAMES[i]),
                timestamp_writes: times[i].map(|(query_set, index)| wgpu::ComputePassTimestampWrites {
                    query_set, beginning_of_pass_write_index: Some(index), end_of_pass_write_index: Some(index+1) }) });
            pass.set_pipeline(pipeline); pass.set_bind_group(0, generation, &[]);
            pass.set_bind_group(1, view, &[]); pass.set_bind_group(2, &self.bind, &[]);
            if (3..=6).contains(&i) { pass.dispatch_workgroups(jobs[i].div_ceil(64), self.plan.specs.len() as u32, 1); }
            else { pass.dispatch_workgroups(jobs[i].min(262_144).div_ceil(64), jobs[i].div_ceil(262_144), 1); }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec(x:i32,z:i32)->BrickSpec { BrickSpec {root_origin:[x,z],lower_y:-0.5,cells_y:16,cells_xz:32} }
    #[test] fn brick_mips_have_identical_bounds_and_no_duplicate_interiors() {
        let p=BrickPlan::new(vec![spec(-8,0),spec(0,0)],0.45,0.8,2400).unwrap();
        for group in p.levels.chunks_exact(MIPS) { for level in group {
            for axis in 0..3 { assert!((level.dims_offset[axis] as f32*level.origin_cell[3]
                -group[0].dims_offset[axis] as f32*group[0].origin_cell[3]).abs()<1e-6); }
        }}
        assert!(BrickPlan::new(vec![spec(0,0),spec(7,7)],0.45,0.8,2400).is_err());
        assert_eq!(p.levels[4].dims_offset[3],18720);
        assert!(p.allocation_bytes(3*2400*80)<MAX_GPU_BYTES);
    }
    #[test] fn brick_caps_fail_before_truncating_sources_or_cells() {
        assert!(BrickPlan::new(vec![spec(0,0);9],0.45,0.8,2400).is_err());
        assert!(BrickPlan::new(vec![spec(0,0)],0.45,0.8,100_000).is_err());
        let mut large=spec(0,0);large.cells_xz=256;
        assert!(BrickPlan::new(vec![large],0.45,0.8,2400).is_err());
        let mut odd=spec(0,0);odd.cells_y=9;
        assert!(BrickPlan::new(vec![odd],0.45,0.8,2400).is_err());
    }
    #[test] fn brick_halo_contains_support_across_negative_world_boundaries() {
        let p=BrickPlan::new(vec![spec(-8,-8)],0.45,0.8,2400).unwrap();
        let nearest_omitted=(p.halo_roots as f32+0.46)*p.root_spacing;
        assert!(nearest_omitted>p.root_support_radius);
        assert_eq!(p.groups[0].roots[0],-10);
        assert_eq!(p.groups[0].roots[1],-10);
    }
    #[test] fn brick_shader_validates_shared_source_and_optical_layouts() {
        assert_eq!(std::mem::size_of::<GpuLevel>(),32);
        assert_eq!(std::mem::size_of::<crate::foliage_volume_data::GpuCell>(),176);
        assert_eq!(std::mem::size_of::<Group>(),32);
        assert_eq!(std::mem::size_of::<Params>(),96);
        let source=shader_source();
        let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
            .validate(&module).unwrap();
    }
}
