//! Read-only scene tables in one storage buffer, so a shader stage spends
//! one binding on all of them (the engine rule is 8 storage buffers per
//! stage, browser.md). Layout in vec4s: 0 to 2 are the header (where
//! materials, car materials and solid occluders start, solid occluder count;
//! where the aux words start and how many, where cluster vertices and
//! triangles start in the topology buffer, topology.rs; where the copied
//! terrain hierarchy, foliage sun page table, voxel crown replacement
//! words and the material extension start), then each table packed as its
//! GPU struct, then `Scene::aux`, the voxel crown replacement words, the
//! material extension (material_maps.rs: texture layer maps and residency,
//! material weathering layers), then the copies, then the clustered light
//! arena. Read in WGSL through `material(i)`, `car_material(i)`,
//! `solid_occluder(i)` and `aux_word(i)` (shade.wgsl, car_material.wgsl).
use crate::scene::{GpuCarMaterial, GpuSolidOccluder, Material};
use crate::Gpu;
use wgpu::util::DeviceExt;

const HEADER: u32 = 3;

pub(crate) struct SceneTables {
    pub buffer: wgpu::Buffer,
    /// Byte offsets of the materials, the car materials and the aux words.
    materials: u64,
    cars: u64,
    aux: u64,
    aux_words: usize,
    /// Byte offsets of the copied terrain hierarchy and foliage sun page table.
    pub terrain: u64,
    pub sun: u64,
    /// Byte offset of the clustered light arena (clustered_lights.rs),
    /// written each frame by the light upload and the bin pass.
    pub clustered: u64,
    /// Byte offsets of the texture layer and material weathering vec4s.
    layer_maps: u64,
    material_layers: u64,
    layer_count: usize,
    material_count: usize,
}

impl SceneTables {
    pub fn new(gpu: &Gpu, materials: &[Material], cars: &[GpuCarMaterial], solids: &[GpuSolidOccluder], aux: &[u32], replacement: &[u32], extension: &crate::material_maps::Extension, topology: [u32; 2], copies: [u64; 3]) -> Self {
        let vec4s = |bytes: usize| (bytes / 16) as u32;
        let m = vec4s(std::mem::size_of_val(materials)).max(2);
        let c = vec4s(std::mem::size_of_val(cars));
        let s = vec4s(std::mem::size_of_val(solids));
        let aux_at = HEADER + m + c + s;
        let aux_vec4s = (aux.len() as u32).div_ceil(4).max(1);
        let replacement_at = aux_at + aux_vec4s;
        let extension_at = replacement_at + (replacement.len() as u32).div_ceil(4).max(1);
        let extension_words = extension.words(extension_at);
        let terrain_at = extension_at + extension_words.len() as u32;
        let sun_at = terrain_at + (copies[0].div_ceil(16) as u32);
        let clustered_at = sun_at + (copies[1].div_ceil(16) as u32);
        let end = clustered_at + (copies[2].div_ceil(16) as u32);
        let header = [HEADER, HEADER + m, HEADER + m + c, solids.len() as u32, aux_at, aux.len() as u32, topology[0], topology[1], terrain_at, sun_at, replacement_at, extension_at];
        let mut bytes: Vec<u8> = bytemuck::cast_slice(&header).to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(materials));
        bytes.resize(16 * (HEADER + m) as usize, 0);
        bytes.extend_from_slice(bytemuck::cast_slice(cars));
        bytes.extend_from_slice(bytemuck::cast_slice(solids));
        debug_assert_eq!(bytes.len(), 16 * aux_at as usize);
        bytes.extend_from_slice(bytemuck::cast_slice(aux));
        bytes.resize(16 * replacement_at as usize, 0);
        bytes.extend_from_slice(bytemuck::cast_slice(replacement));
        bytes.resize(16 * extension_at as usize, 0);
        bytes.extend_from_slice(bytemuck::cast_slice(&extension_words));
        // At least one aux vec4 so aux_word(0) stays in range; the copied
        // regions start zeroed and are filled by copy_buffer_to_buffer.
        bytes.resize(16 * end as usize, 0);
        let buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("scene tables"), contents: &bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST });
        SceneTables { buffer, materials: 16 * u64::from(HEADER), cars: 16 * u64::from(HEADER + m), aux: 16 * u64::from(aux_at), aux_words: aux.len(),
            terrain: 16 * u64::from(terrain_at), sun: 16 * u64::from(sun_at), clustered: 16 * u64::from(clustered_at),
            layer_maps: 16 * u64::from(extension_at + extension.layers_offset()),
            material_layers: 16 * u64::from(extension_at + extension.materials_offset()),
            layer_count: extension.layers.len(), material_count: extension.materials.len() }
    }

    /// Overwrite one texture layer's map vec4 (material_maps.rs).
    pub fn write_layer_map(&self, gpu: &Gpu, layer: usize, word: [u32; 4]) {
        if layer >= self.layer_count { return; }
        gpu.queue.write_buffer(&self.buffer, self.layer_maps + 16 * layer as u64, bytemuck::cast_slice(&word));
    }

    /// Overwrite material weathering vec4s from material `first`.
    pub fn write_material_layers(&self, gpu: &Gpu, first: usize, words: &[[u32; 4]]) {
        if first + words.len() > self.material_count { return; }
        gpu.queue.write_buffer(&self.buffer, self.material_layers + 16 * first as u64, bytemuck::cast_slice(words));
    }

    pub fn write_materials(&self, gpu: &Gpu, first: usize, materials: &[Material]) {
        gpu.queue.write_buffer(&self.buffer, self.materials + (first * std::mem::size_of::<Material>()) as u64, bytemuck::cast_slice(materials));
    }

    pub fn write_cars(&self, gpu: &Gpu, cars: &[GpuCarMaterial]) {
        gpu.queue.write_buffer(&self.buffer, self.cars, bytemuck::cast_slice(cars));
    }

    /// Copy `source` (whole) into the region reserved at byte `at`.
    pub fn copy_in(&self, enc: &mut wgpu::CommandEncoder, source: &wgpu::Buffer, at: u64) {
        enc.copy_buffer_to_buffer(source, 0, &self.buffer, at, source.size());
    }

    /// Overwrite aux words from `offset` (in words), within `Scene::aux`.
    pub fn write_aux(&self, gpu: &Gpu, offset: usize, words: &[u32]) -> Result<(), String> {
        let end = offset.checked_add(words.len()).ok_or("aux range overflow")?;
        if end > self.aux_words { return Err("aux write outside reserved buffer".into()); }
        gpu.queue.write_buffer(&self.buffer, self.aux + (offset * 4) as u64, bytemuck::cast_slice(words));
        Ok(())
    }
}
