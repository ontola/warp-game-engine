//! Material data beyond the fixed 32-byte `Material`: per texture layer
//! maps (where its occlusion/roughness/metalness map sits in the normal
//! array, and how many top mips are resident while textures stream) and per
//! material weathering layers (wet, snow, moss). Lives in the scene tables
//! buffer (scene_tables.rs), so it costs no storage binding. The WGSL side
//! is `layer_map(i)` and `material_weather(i)` in shade.wgsl and raster.wgsl.
//!
//! Layout from the header's vec4 2.w, in vec4s: a header (where the layer
//! maps start, where the material layers start, how many of each), one
//! vec4 per texture layer (x: 1 + material map layer in the normal array or
//! 0, y: least albedo LOD that is resident as f32 bits, z: least normal and
//! material map LOD, w: 0), then one vec4 per material (x: wet, snow, moss
//! and porosity as bytes, y..w: 0).

/// How a material takes the weather (`Lighting::wetness`, `Lighting::snow`)
/// and lasting growth. Each is 0..1. The default takes wet and snow fully
/// and grows no moss. Car paint, roads (which have their own wet look),
/// lava and emissive surfaces never take these layers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialLayers {
    /// How wet the surface gets when it rains (darker, glossier).
    pub wet: f32,
    /// How much snow settles on its upward faces.
    pub snow: f32,
    /// Moss on its upward, sheltered faces, independent of the weather.
    pub moss: f32,
    /// How much water it soaks up: 1 darkens like soil or wood, 0 only
    /// gets a film of water like stone or metal.
    pub porosity: f32,
}

impl Default for MaterialLayers {
    fn default() -> Self { MaterialLayers { wet: 1.0, snow: 1.0, moss: 0.0, porosity: 0.5 } }
}

impl MaterialLayers {
    pub fn word(&self) -> [u32; 4] {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        [byte(self.wet) | byte(self.snow) << 8 | byte(self.moss) << 16 | byte(self.porosity) << 24, 0, 0, 0]
    }
}

/// One texture layer's map word.
pub fn layer_word(material_map: Option<u32>, albedo_min_lod: f32, normal_min_lod: f32) -> [u32; 4] {
    [material_map.map_or(0, |l| l + 1), albedo_min_lod.to_bits(), normal_min_lod.to_bits(), 0]
}

pub(crate) struct Extension {
    pub layers: Vec<[u32; 4]>,
    pub materials: Vec<[u32; 4]>,
}

impl Extension {
    pub fn new(layers: Vec<[u32; 4]>, scene: &crate::scene::Scene) -> Self {
        let materials = (0..scene.materials.len().max(1))
            .map(|i| scene.material_layers.get(i).copied().unwrap_or_default().word()).collect();
        Extension { layers, materials }
    }

    /// Vec4 offsets of the layer maps and the material layers from the start.
    pub fn layers_offset(&self) -> u32 { 1 }
    pub fn materials_offset(&self) -> u32 { 1 + self.layers.len().max(1) as u32 }

    /// The packed region, placed at vec4 `at` of the scene tables.
    pub fn words(&self, at: u32) -> Vec<[u32; 4]> {
        let mut words = vec![[at + self.layers_offset(), at + self.materials_offset(), self.layers.len() as u32, self.materials.len() as u32]];
        words.extend_from_slice(&self.layers);
        if self.layers.is_empty() { words.push(layer_word(None, 0.0, 0.0)); }
        words.extend_from_slice(&self.materials);
        words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_bytes_and_offsets() {
        let w = MaterialLayers { wet: 1.0, snow: 0.0, moss: 0.5, porosity: 2.0 }.word();
        assert_eq!(w[0], 255 | 0 << 8 | 128 << 16 | 255 << 24);
        let mut scene = crate::scene::Scene::default();
        scene.materials.push(bytemuck::Zeroable::zeroed());
        scene.materials.push(bytemuck::Zeroable::zeroed());
        scene.set_material_layers(1, MaterialLayers { moss: 1.0, ..Default::default() });
        let e = Extension::new(vec![layer_word(Some(3), 2.0, 0.0)], &scene);
        let words = e.words(10);
        assert_eq!(words[0], [11, 12, 1, 2]);
        assert_eq!(words[1], [4, 2f32.to_bits(), 0, 0]);
        assert_eq!(words[2], MaterialLayers::default().word());
        assert_eq!(words[3][0] >> 16 & 255, 255);
    }
}
