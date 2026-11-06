//! Read-only scene tables in one storage buffer, so a shader stage spends
//! one binding on all of them (the engine rule is 8 storage buffers per
//! stage, browser.md). Layout in vec4s: 0 and 1 are the header (where
//! materials, car materials and solid occluders start, solid occluder count;
//! where the aux words start and how many), then each table packed as its GPU struct, then
//! `Scene::aux`. Read in WGSL through `material(i)`, `car_material(i)`,
//! `solid_occluder(i)` and `aux_word(i)` (shade.wgsl, car_material.wgsl).
use crate::scene::{GpuCarMaterial, GpuSolidOccluder, Material};
use crate::Gpu;
use wgpu::util::DeviceExt;

const HEADER: u32 = 2;

pub(crate) struct SceneTables {
    pub buffer: wgpu::Buffer,
    /// Byte offsets of the materials, the car materials and the aux words.
    materials: u64,
    cars: u64,
    aux: u64,
    aux_words: usize,
}

impl SceneTables {
    pub fn new(gpu: &Gpu, materials: &[Material], cars: &[GpuCarMaterial], solids: &[GpuSolidOccluder], aux: &[u32]) -> Self {
        let vec4s = |bytes: usize| (bytes / 16) as u32;
        let m = vec4s(std::mem::size_of_val(materials)).max(2);
        let c = vec4s(std::mem::size_of_val(cars));
        let s = vec4s(std::mem::size_of_val(solids));
        let aux_at = HEADER + m + c + s;
        let header = [HEADER, HEADER + m, HEADER + m + c, solids.len() as u32, aux_at, aux.len() as u32, 0, 0];
        let mut bytes: Vec<u8> = bytemuck::cast_slice(&header).to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(materials));
        bytes.resize(16 * (HEADER + m) as usize, 0);
        bytes.extend_from_slice(bytemuck::cast_slice(cars));
        bytes.extend_from_slice(bytemuck::cast_slice(solids));
        debug_assert_eq!(bytes.len(), 16 * aux_at as usize);
        bytes.extend_from_slice(bytemuck::cast_slice(aux));
        // Whole vec4s, and at least one aux vec4 so aux_word(0) stays in range.
        bytes.resize((bytes.len() + 16).div_ceil(16) * 16, 0);
        let buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("scene tables"), contents: &bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST });
        SceneTables { buffer, materials: 16 * u64::from(HEADER), cars: 16 * u64::from(HEADER + m), aux: 16 * u64::from(aux_at), aux_words: aux.len() }
    }

    pub fn write_materials(&self, gpu: &Gpu, first: usize, materials: &[Material]) {
        gpu.queue.write_buffer(&self.buffer, self.materials + (first * std::mem::size_of::<Material>()) as u64, bytemuck::cast_slice(materials));
    }

    pub fn write_cars(&self, gpu: &Gpu, cars: &[GpuCarMaterial]) {
        gpu.queue.write_buffer(&self.buffer, self.cars, bytemuck::cast_slice(cars));
    }

    /// Overwrite aux words from `offset` (in words), within `Scene::aux`.
    pub fn write_aux(&self, gpu: &Gpu, offset: usize, words: &[u32]) -> Result<(), String> {
        let end = offset.checked_add(words.len()).ok_or("aux range overflow")?;
        if end > self.aux_words { return Err("aux write outside reserved buffer".into()); }
        gpu.queue.write_buffer(&self.buffer, self.aux + (offset * 4) as u64, bytemuck::cast_slice(words));
        Ok(())
    }
}
