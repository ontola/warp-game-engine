//! Read-only scene tables in one storage buffer, so a shader stage spends
//! one binding on all of them (the browser target is 8 storage buffers per
//! stage, browser.md). Layout in vec4s: 0 is the header (where materials,
//! car materials and solid occluders start, solid occluder count), then
//! each table packed as its GPU struct. Read in WGSL through `material(i)`,
//! `car_material(i)` and `solid_occluder(i)` (shade.wgsl, car_material.wgsl).
use crate::scene::{GpuCarMaterial, GpuSolidOccluder, Material};
use crate::Gpu;
use wgpu::util::DeviceExt;

pub(crate) struct SceneTables {
    pub buffer: wgpu::Buffer,
    /// Byte offsets of the materials and the car materials.
    materials: u64,
    cars: u64,
}

impl SceneTables {
    pub fn new(gpu: &Gpu, materials: &[Material], cars: &[GpuCarMaterial], solids: &[GpuSolidOccluder]) -> Self {
        let vec4s = |bytes: usize| (bytes / 16) as u32;
        let m = vec4s(std::mem::size_of_val(materials)).max(2);
        let c = vec4s(std::mem::size_of_val(cars));
        let header = [1u32, 1 + m, 1 + m + c, solids.len() as u32];
        let mut bytes: Vec<u8> = bytemuck::cast_slice(&header).to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(materials));
        bytes.resize(16 * (1 + m) as usize, 0);
        bytes.extend_from_slice(bytemuck::cast_slice(cars));
        bytes.extend_from_slice(bytemuck::cast_slice(solids));
        let buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("scene tables"), contents: &bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST });
        SceneTables { buffer, materials: 16, cars: 16 * u64::from(1 + m) }
    }

    pub fn write_materials(&self, gpu: &Gpu, first: usize, materials: &[Material]) {
        gpu.queue.write_buffer(&self.buffer, self.materials + (first * std::mem::size_of::<Material>()) as u64, bytemuck::cast_slice(materials));
    }

    pub fn write_cars(&self, gpu: &Gpu, cars: &[GpuCarMaterial]) {
        gpu.queue.write_buffer(&self.buffer, self.cars, bytemuck::cast_slice(cars));
    }
}
