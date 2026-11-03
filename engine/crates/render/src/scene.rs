//! What gets drawn: cluster meshes, instances of them, materials, the
//! textures they sample and the sky. Built on the CPU, uploaded once.

use crate::cluster::{ClusterMesh, GpuCluster, Vertex};
use bytemuck::{Pod, Zeroable};
use glam::{Affine3A, Vec3};
use std::collections::HashMap;

/// Material flags.
pub const ALPHA_TEST: u32 = 1;
pub const TERRAIN: u32 = 2;
pub const ROAD: u32 = 4;
pub const TWO_SIDED: u32 = 8;
/// Albedo projected from world space along the dominant normal axis.
pub const TRIPLANAR: u32 = 16;
/// A glossy clear coat over the base (car paint).
pub const CLEARCOAT: u32 = 32;
/// Photographed low grass cards: upward lighting normal and root occlusion.
pub const GRASS: u32 = 64;
/// Glass/blended surfaces must not become opaque ambient-light occluders.
/// This flag does not change the material's rasterization or albedo.
pub const NO_AMBIENT_OCCLUSION: u32 = 128;
/// Drawn by the sorted forward pass, never the opaque or shadow raster pass.
pub const BLEND: u32 = 256;
/// True leaf/needle geometry: thin-leaf lighting without alpha-tested cards.
pub const THIN_FOLIAGE: u32 = 512;
/// Self-lit surface (lava, lamps, tracers): adds `tint * scale` nits of
/// radiance; `scale` is the emission, not a repeat. With a texture `layer`
/// (uv-mapped) the textured colour glows instead: screens and signs.
pub const EMISSIVE: u32 = 1024;
/// Procedural rock (Growing Guns' rock_surface): world-space pits, cracks
/// and moss on a block's faces, tinted by `tint`; `scale` 0..1 is damage
/// (more and darker cracks). Meant for unit-cube blocks (hewn bricks).
/// With a texture layer, samples a shared mipmapped stone surface instead:
/// RGB albedo, normal XY, normal B signed 32 mm height, and normal alpha
/// roughness multiplier. Height 1 disables parallax for custom rock maps. The repeat
/// is `stone_surface::REPEAT_METRES`; `scale` still carries damage.
pub const ROCK: u32 = 2048;
/// Procedural flowing lava (Growing Guns' lava_flow), unlit: an animated
/// emissive surface in world XZ; `tint * scale` is the radiance in nits of a
/// white pixel. Follows `Lighting::time`. Reads its noise from
/// `Scene::noise` (Godot's 256 px lava texture) when the scene has one,
/// else from a built-in gradient noise of the same statistics.
pub const LAVA: u32 = 4096;

/// Pixel art: sample the albedo's top mip (the sharp source pixels, like
/// Godot's nearest filter without mipmaps) for colour and alpha test. Upload
/// the image nearest-upscaled so linear filtering keeps hard pixel edges.
pub const PIXEL_ART: u32 = 8192;

/// Live display layer: only mip 2 (512 square) and below contain video.
/// Use with EMISSIVE and `Scene::add_live_screen_texture`.
pub const LIVE_SCREEN: u32 = 16384;

/// A game's own surface shader: `render::set_game_surface` supplies
/// `game_surface` (see shaders/game_surface.wgsl), which sets albedo,
/// normal, roughness, metalness and emission. `layer` holds -2 - kind.
pub const SURFACE: u32 = 32768;

/// Material flags from this bit up hold 1 + an index into
/// `Scene::car_materials`: a procedural car surface (car_material.wgsl).
pub const CAR_SHIFT: u32 = 16;

/// One procedural car surface: paint, or another part of a car, with its
/// wear and grime. The meaning of each field is in car_material.wgsl; the
/// CLI fills these from engine/data/car_materials.toml.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuCarMaterial {
    /// rgb paint or part colour (linear), w: kind.
    pub colour: [f32; 4],
    /// rgb pearl shift / candy flake colour, w: paint finish.
    pub second: [f32; 4],
    /// Base roughness, metalness, clear coat, coat roughness.
    pub surface: [f32; 4],
    /// Flake amount, flake size (m), flake spread, orange peel.
    pub flake: [f32; 4],
    /// Chips, scratches, polish swirls, sun fade.
    pub wear: [f32; 4],
    /// Dust, mud, rust, seed.
    pub grime: [f32; 4],
    /// rgb dust colour, w: mud wetness.
    pub dirt: [f32; 4],
    /// Object-space bounds: min y, max y, min z, max z.
    pub body: [f32; 4],
    /// Front axle z, rear axle z, wheel radius, half track (negative: a wheel).
    pub axles: [f32; 4],
}

/// Forward blend data, separate from the fixed GPU material layout.
#[derive(Clone, Copy, Debug)]
pub struct BlendMaterial {
    pub alpha: f32,
    pub double_sided: bool,
    /// Add source radiance without attenuating the destination.
    pub additive: bool,
    /// Sort by the instance origin instead of per triangle (like Godot):
    /// nested shells at one centre draw in instance order.
    pub object_sort: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, serde::Serialize, serde::Deserialize)]
pub struct Material {
    /// Albedo tint (linear), multiplied with the texture.
    pub tint: [f32; 3],
    /// Texture array layer, or -1 for none. LAVA with -2 uses mesh UVs for small molten chambers.
    pub layer: i32,
    pub roughness: f32,
    pub flags: u32,
    /// World-space metres per texture repeat (triplanar/planar materials).
    pub scale: f32,
    /// Metalness 0..1.
    pub metal: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuInstance {
    /// Rows of the 3x4 object-to-world transform.
    pub rows: [[f32; 4]; 3],
    /// World-space bounding sphere.
    pub sphere: [f32; 4],
    pub first_cluster: u32,
    pub cluster_count: u32,
    /// Largest axis scale: errors and radii grow with it.
    pub scale: f32,
    /// Instance flags (FOLIAGE: fills the canopy volume for GI).
    pub flags: u32,
}

pub const FOLIAGE: u32 = 1;
/// An instance whose opaque geometry supplies near-field ambient occluders.
pub const SOLID: u32 = 2;
/// Small ground cover receives shadows but does not fill the sun shadow map.
pub const NO_SHADOW: u32 = 4;
/// Fixed-pool grass with per-view opaque/transparent transition selection.
pub const LIVE_GRASS: u32 = 8;
/// Ground-cover wind response without enabling a live distance transition.
pub const WIND_GRASS: u32 = 16;
/// A rigid vegetation part driven by the shared hierarchical GPU pose pass.
pub const PART_WIND: u32 = 32;
/// Weighted vertices are updated by the shared skinning pass. Dynamic shadow
/// caster; temporal history is rejected until deformation vectors are supplied.
pub const SKINNED: u32 = 64;
/// GPU debris touching lava: use the shared molten surface shading.
pub const MOLTEN_DEBRIS: u32 = 128;

// Bit 7 is reserved for part_wind::HISTORY_EPOCH. The GPU toggles it when a
// sleeping assembly wakes, so motion history rejects the old pose once while
// ordinary foliage/material and per-camera visibility flags remain unchanged.

/// Bits 8..15: hidden from camera view 0..7 (still casts shadows and shows in
/// other views). Set with [`GpuInstance::hide_in_views`].
pub const HIDE_IN_VIEW_SHIFT: u32 = 8;
pub const HIDE_IN_VIEW_MASK: u32 = 0xff << HIDE_IN_VIEW_SHIFT;

/// Explicit source provenance for render diagnostics. These bits are in
/// `GpuInstance::flags`, not material flags, and do not select rendering policy.
pub const INSTANCE_CATEGORY_SHIFT: u32 = 24;
pub const INSTANCE_CATEGORY_MASK: u32 = 7 << INSTANCE_CATEGORY_SHIFT;

/// Bits 16..23: a hit flash, 0..255 (the body glows white for a moment,
/// strongest at its edges). Set with [`GpuInstance::set_hit_flash`].
pub const HIT_FLASH_SHIFT: u32 = 16;
pub const HIT_FLASH_MASK: u32 = 0xff << HIT_FLASH_SHIFT;
/// Bits 27..31: wounds, 0..31 (wet blood stains spreading over the
/// surface). Set with [`GpuInstance::set_wounds`].
pub const WOUNDS_SHIFT: u32 = 27;
pub const WOUNDS_MASK: u32 = 0x1f << WOUNDS_SHIFT;

#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GeometryCategory {
    #[default]
    Other = 0,
    Terrain = 1,
    Trees = 2,
    Grass = 3,
    Cars = 4,
}

impl GeometryCategory {
    pub const ALL: [Self; 5] = [Self::Other, Self::Terrain, Self::Trees, Self::Grass, Self::Cars];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Other => "other", Self::Terrain => "terrain", Self::Trees => "trees",
            Self::Grass => "grass", Self::Cars => "cars",
        }
    }

    pub const fn exclusion_bit(self) -> u32 { 1 << self as u32 }
}

impl GpuInstance {
    pub const fn category(&self) -> GeometryCategory {
        match (self.flags & INSTANCE_CATEGORY_MASK) >> INSTANCE_CATEGORY_SHIFT {
            1 => GeometryCategory::Terrain, 2 => GeometryCategory::Trees,
            3 => GeometryCategory::Grass, 4 => GeometryCategory::Cars,
            _ => GeometryCategory::Other,
        }
    }

    /// Changes diagnostic provenance only; wind, shadows and occlusion flags stay intact.
    /// Hide this instance from the camera views in `mask` (bit i: view i),
    /// e.g. a first-person player's own body. Shadows are unaffected.
    pub fn hide_in_views(&mut self, mask: u8) {
        self.flags = (self.flags & !HIDE_IN_VIEW_MASK) | (u32::from(mask) << HIDE_IN_VIEW_SHIFT);
    }

    /// How white the body flashes this frame, 0 (not) to 1.
    pub fn set_hit_flash(&mut self, k: f32) {
        let v = (k.clamp(0.0, 1.0) * 255.0).round() as u32;
        self.flags = (self.flags & !HIT_FLASH_MASK) | (v << HIT_FLASH_SHIFT);
    }

    /// How bloodied the body is, 0 (clean) to 1 (covered).
    pub fn set_wounds(&mut self, k: f32) {
        let v = (k.clamp(0.0, 1.0) * 31.0).round() as u32;
        self.flags = (self.flags & !WOUNDS_MASK) | (v << WOUNDS_SHIFT);
    }

    pub fn set_category(&mut self, category: GeometryCategory) {
        self.flags = (self.flags & !INSTANCE_CATEGORY_MASK) | ((category as u32) << INSTANCE_CATEGORY_SHIFT);
    }
}

/// CPU source counts, not selected GPU work. Empty reserved pool slots count
/// toward `instances` only; referenced clusters include every available LOD.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GeometryCategoryCounts {
    pub instances: u64,
    pub active_instances: u64,
    pub referenced_clusters: u64,
}

pub const MAX_SOLID_SEGMENTS: usize = 6;

/// Static object-space proxy. Shaders fetch the current transform from the
/// existing instance buffer, so moving/rotating cars need no proxy re-upload.
/// WGSL layout: center:vec4f, half_extent:vec4f, meta:vec4u (48 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuSolidOccluder {
    /// Object-space box center; w is reserved (zero).
    pub center: [f32; 4],
    /// Object-space half-extents; w is their bounding-sphere radius.
    pub half_extent: [f32; 4],
    /// x: owner instance index, y/z/w reserved (zero).
    pub meta: [u32; 4],
}

fn clip_axis(polygon: &[Vec3], axis: usize, plane: f32, keep_greater: bool) -> Vec<Vec3> {
    let mut out=Vec::with_capacity(polygon.len()+1);
    let Some(mut previous)=polygon.last().copied() else{return out;};
    let inside=|point:Vec3|if keep_greater{point[axis]>=plane}else{point[axis]<=plane};
    let mut was_inside=inside(previous);
    for &point in polygon {
        let is_inside=inside(point);
        if is_inside!=was_inside {
            let t=(plane-previous[axis])/(point[axis]-previous[axis]);
            let mut intersection=previous.lerp(point,t.clamp(0.0,1.0));
            intersection[axis]=plane;
            out.push(intersection);
        }
        if is_inside{out.push(point);}
        previous=point;was_inside=is_inside;
    }
    out
}

/// Fit a small number of local boxes from actual opaque triangle coverage.
/// Slicing along the body's longest horizontal axis prevents one tall cabin
/// box from extending over both the bonnet and boot. It is a bounded proxy,
/// not a claim to reconstruct every wheel arch or cavity from the mesh.
fn solid_segments(triangles: &[[Vec3;3]]) -> Vec<(Vec3,Vec3)> {
    let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
    for triangle in triangles {for &point in triangle {lo=lo.min(point);hi=hi.max(point);}}
    if !lo.is_finite() || !hi.is_finite(){return Vec::new();}
    let extent=hi-lo;let axis=if extent.z>=extent.x{2}else{0};
    if extent[axis]<=0.001{return Vec::new();}
    let step=extent[axis]/MAX_SOLID_SEGMENTS as f32;
    let mut boxes=Vec::with_capacity(MAX_SOLID_SEGMENTS);
    for segment in 0..MAX_SOLID_SEGMENTS {
        let lower=lo[axis]+step*segment as f32;
        let upper=if segment+1==MAX_SOLID_SEGMENTS{hi[axis]}else{lo[axis]+step*(segment+1) as f32};
        let mut part_lo=Vec3::splat(f32::INFINITY);let mut part_hi=Vec3::splat(f32::NEG_INFINITY);
        for triangle in triangles {
            let tri_lo=triangle[0][axis].min(triangle[1][axis]).min(triangle[2][axis]);
            let tri_hi=triangle[0][axis].max(triangle[1][axis]).max(triangle[2][axis]);
            if tri_hi<lower || tri_lo>upper{continue;}
            let clipped=clip_axis(&clip_axis(triangle,axis,lower,true),axis,upper,false);
            for point in clipped{part_lo=part_lo.min(point);part_hi=part_hi.max(point);}
        }
        if !part_lo.is_finite() || !part_hi.is_finite(){continue;}
        let half=(part_hi-part_lo)*0.5;
        // Do not inflate isolated open sheets into a fictitious solid volume.
        if half.min_element()<=0.001{continue;}
        let inset=half*0.08;
        let inset=inset.min(Vec3::splat(0.04));
        for dimension in 0..3 {
            if dimension!=axis || segment==0{part_lo[dimension]+=inset[dimension];}
            if dimension!=axis || segment+1==MAX_SOLID_SEGMENTS{part_hi[dimension]-=inset[dimension];}
        }
        // Adjacent longitudinal slices meet exactly: no artificial slit every
        // segment. Only the exterior shell is inset to reduce self-occlusion.
        boxes.push(((part_lo+part_hi)*0.5,(part_hi-part_lo)*0.5));
    }
    boxes
}

/// Terrain heights on a regular grid, for GI occlusion (and later grass).
#[derive(Default, Clone)]
pub struct Heightfield {
    pub origin: [f32; 2],
    pub cell: f32,
    pub w: u32,
    pub h: u32,
    pub heights: Vec<f32>,
}

pub struct MeshRef {
    pub first_cluster: u32,
    pub cluster_count: u32,
    pub sphere: [f32; 4],
}

/// Encoding of the RGB normal components in a source image. This must be
/// supplied by the asset caller, never inferred from colour/profile metadata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NormalEncoding {
    /// Ordinary normal-map data: RGB already holds linear UNORM components.
    #[default]
    LinearUnorm,
    /// The file applied the IEC sRGB transfer function to its UNORM components.
    SrgbTransferred,
}

impl NormalEncoding {
    fn decode(self, image: &mut image::RgbaImage) {
        if self == Self::LinearUnorm { return; }
        // Decode at source resolution, before any resize/mip filtering. The
        // texture array remains linear RGBA8 UNORM; alpha is coverage/data.
        let table: [u8; 256] = std::array::from_fn(|byte| {
            let encoded = byte as f64 / 255.0;
            let linear = if encoded <= 0.04045 { encoded / 12.92 }
                else { ((encoded + 0.055) / 1.055).powf(2.4) };
            (linear * 255.0).round() as u8
        });
        for pixel in image.pixels_mut() {
            for channel in &mut pixel.0[..3] { *channel = table[*channel as usize]; }
        }
    }
}

/// Textures to put in the albedo array (all resized to one size).
pub struct TextureSource {
    pub name: String,
    pub rgba: image::RgbaImage,
    /// Tangent-space normal map in linear UNORM (OpenGL convention), if any.
    /// Source transfer encoding has already been removed before GPU mipmaps.
    pub normal: Option<image::RgbaImage>,
}

/// Startup graphics modifiers, independent of windowing or XR presentation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Graphics {
    /// Coarsest mesh LOD, indexed instancing and vertex-lit colour. Requires
    /// plain CPU-owned meshes; no skinning, GPU debris or procedural animation.
    pub minimal_forward: bool,
    /// Textured diffuse sun/sky lighting, without procedural relief, GI or shadows.
    pub simple_lighting: bool,
    /// One lane per mesh owner. For plain mesh scenes without wind, vegetation
    /// replacement, spatial aggregation, cluster traversal or cached sun pages.
    pub simple_mesh_culling: bool,
}

#[derive(Default)]
pub struct Scene {
    pub graphics: Graphics,
    pub vertices: Vec<Vertex>,
    pub cluster_verts: Vec<u32>,
    pub tris: Vec<u32>,
    pub clusters: Vec<GpuCluster>,
    pub meshes: Vec<MeshRef>,
    pub instances: Vec<GpuInstance>,
    pub materials: Vec<Material>,
    /// Procedural car surfaces, referenced through `CAR_SHIFT` flag bits.
    /// `Renderer::set_car_materials` replaces them without a rebuild.
    pub car_materials: Vec<GpuCarMaterial>,
    pub blended_materials: HashMap<u32, BlendMaterial>,
    pub grass_ranges: Vec<crate::grass::GrassRange>,
    /// GPU-generated ground cover: terrain/settings/recipes once, no CPU tuft stream.
    pub meadow: Option<crate::meadow_data::MeadowField>,
    /// Shared CPU residency / per-view GPU fade policy for source grass.
    pub grass_detail: crate::vegetation_detail::DistanceProfile,
    /// Optional world-fixed terrain-following grass density field.
    pub far_grass: Option<crate::far_grass::FarGrassField>,
    /// Maximum vertical displacement of TERRAIN source geometry from the
    /// native ground field. Zero keeps strict solid-ground receiver tests.
    /// Only ascending escape within one native-cell diagonal of the receiver
    /// is permitted, independent of its position relative to grid boundaries;
    /// subsequent terrain, grass and road occluders remain unchanged.
    pub terrain_visual_offset_m: f32,
    /// Optional bounded validation forest; absent in normal live scenes.
    pub voxel_forest: Option<crate::foliage_volume_data::GpuForest>,
    /// Shared source-derived whole-plant volumes and a world-grid index.
    /// Experimental native renderer; source is retained unless explicitly enabled.
    pub foliage_world: Option<crate::foliage_world::WorldField>,
    pub foliage_world_experimental: bool,
    /// Output-pixel spatial support budget; optics require separate validation.
    pub foliage_world_error_pixels: f32,
    /// Explicit projected voxel LOD experiment; zero keeps finest-level mode.
    pub voxel_lod_pixels: f32,
    /// Far trees (far_trees::attach): past this camera distance every
    /// voxel_forest owner is drawn as rasterized voxels instead of its
    /// mesh, and the shared sun map takes voxels for all of them. Zero
    /// keeps per-group selection.
    pub voxel_far_distance_m: f32,
    /// The last this many voxel_forest trees are whole-plant proxies
    /// (vegetation_asset::SceneVegetation::append_proxy): no source, no
    /// selection, drawn and casting sun at any distance.
    pub voxel_proxies: u32,
    /// A proxy uses the coarsest level whose cell stays under this many pixels.
    pub voxel_proxy_pixels: f32,
    /// Past this camera distance (m) the proxy pixel error grows with
    /// sqrt(distance / voxel_proxy_pixels_from); zero keeps it constant.
    pub voxel_proxy_pixels_from: f32,
    /// Experimental GPU worklist: integrate only tiles with volume candidates.
    /// Defaults to the full-screen reference until image/timing validation.
    pub voxel_active_tiles: bool,
    /// Optional exact source/volume pairs, selected independently for each
    /// camera. Empty preserves static validation-forest replacement.
    pub foliage_representations: Vec<crate::foliage_representation::Binding>,
    /// A branch proxy may own many shared primitive instances. All members
    /// switch with its single source/voxel decision; shadows retain members.
    pub foliage_source_groups: Vec<crate::foliage_representation::SourceGroup>,
    /// Optical evidence is required unless the caller explicitly opts into
    /// the bounded procedural foliage experiment.
    pub foliage_representation_policy: crate::foliage_representation::Policy,
    /// Optional bounded shared branch poses for both source and voxel parts.
    pub part_wind: Vec<crate::part_wind::PartWind>,
    /// Reused primitive meshes following shared branch poses on the GPU.
    /// Empty preserves the original one-source-instance-per-branch path.
    pub part_attachments: Vec<crate::part_wind::PartAttachment>,
    /// Optional GPU source/aggregate error selection; absent preserves legacy planning.
    pub aggregate_lod: Option<crate::aggregate_lod::Selection>,
    /// GPU all-consumer visibility and projected-motion scheduling for rigid branches.
    pub part_wind_lod: bool,
    /// Shared baked rigid-part clips. Optional, with bounded actor/part pools.
    pub baked_animations: Option<crate::baked_animation::BakedAnimations>,
    /// Independent weighted character meshes, with caller-sampled palettes.
    pub skinning: crate::skinning::Skinning,
    /// Indoor irradiance probes in `aux` (`reserve_irradiance_volume`): the
    /// word offset of their header and the grid.
    pub irradiance_volume: Option<(usize, crate::irradiance_volume::Grid)>,
    /// Bullet dents in ROCK surfaces (`reserve_dents`): word offset and capacity.
    pub dents: Option<(usize, usize)>,
    /// 3D unsigned Euclidean distance field for DFAO (`reserve_dfao`): word offset and grid.
    pub dfao: Option<(usize, crate::dfao::Grid)>,
    /// A procedural terrain patch pool (`procedural_terrain::reserve`).
    pub procedural_terrain: Option<crate::procedural_terrain::TerrainSpec>,
    /// A GPU-simulated debris pool (`debris::Debris::reserve`).
    pub debris: Option<crate::debris::DebrisSpec>,
    /// A GPU-posed crowd (`crowd::Crowd::reserve`).
    pub crowd: Option<crate::crowd::CrowdSpec>,
    pub textures: Vec<TextureSource>,
    pub live_screen_feeds: Vec<crate::screen_feed::Feed>,
    pub heightfield: Heightfield,
    /// Explicit declaration that heightfield sentinel holes and outside its
    /// bounds contain no loaded TERRAIN geometry. Does not cover meshes or
    /// future streamed terrain. False keeps strict unknown-outside tracing.
    pub terrain_visibility_complete: bool,
    /// Trace the sun through `heightfield` for terrain shadows past the
    /// shadow maps (as `KERN_WORLD_TERRAIN_VISIBILITY=1`; `=0` still wins).
    pub world_terrain_visibility: bool,
    /// Sun visibility 0..1 baked per `heightfield` sample (same grid). When
    /// present, terrain visibility reads it instead of tracing per pixel.
    pub terrain_sun: Vec<f32>,
    /// Persistent sun-shadow casters, fitted to these static instance bounds.
    /// Camera motion reuses the map; source transforms and sun changes rebake it.
    /// Keep moving/destructible actors out. Empty preserves the default policy.
    pub static_sun_instances: Vec<usize>,
    /// Game data for materials (Sideways: road surface per 2 m row).
    pub aux: Vec<u32>,
    /// A tiling scalar field procedural materials sample (`noise_field`):
    /// LAVA reads it as Godot's lava noise texture when it is there.
    pub noise: Option<crate::noise_field::NoiseField>,
    /// Equirectangular HDR sky, linear RGB.
    pub sky: Option<(u32, u32, Vec<[f32; 4]>)>,
    /// Absolute, sun-disc-free radiance with its solar state supplied by the
    /// atmosphere bake. Bypass HDRI brightest-pixel rotation and lux rescaling.
    pub sky_physical: bool,
}

impl Scene {
    /// Exact immutable mesh buffers and reserved array sizes, for rewinding
    /// runtime state on the same renderer. This does not compare textures or
    /// dynamic instance/material/auxiliary values; those still need restoring.
    pub fn same_reserved_geometry(&self, other: &Self) -> bool {
        self.instances.len() == other.instances.len() && self.materials.len() == other.materials.len()
            && self.aux.len() == other.aux.len() && self.meshes.len() == other.meshes.len()
            && self.skinning.actors.len() == other.skinning.actors.len()
            && self.cluster_verts == other.cluster_verts && self.tris == other.tris
            && bytemuck::cast_slice::<_, u8>(&self.vertices) == bytemuck::cast_slice::<_, u8>(&other.vertices)
            && bytemuck::cast_slice::<_, u8>(&self.clusters) == bytemuck::cast_slice::<_, u8>(&other.clusters)
    }

    /// At most six opaque local boxes per SOLID instance. Build once after
    /// scene assembly; each descriptor's owner points at the live instance
    /// transform. Alpha-tested and explicitly non-occluding materials are
    /// excluded before fitting, so transparent geometry cannot enlarge boxes.
    ///
    /// Shaders should test a nearby owner bound before its boxes, bias the
    /// sample from the real surface, and ignore a containing proxy belonging
    /// to the shaded owner. These are conservative interior approximations,
    /// not replacements for the sun shadow map or a full mesh ray tracer.
    pub fn solid_occluders(&self) -> Vec<GpuSolidOccluder> {
        let mut meshes:HashMap<(u32,u32),Vec<(Vec3,Vec3)>>=HashMap::new();
        let mut result=Vec::new();
        for (owner,instance) in self.instances.iter().enumerate() {
            if instance.flags & SOLID==0{continue;}
            let boxes=meshes.entry((instance.first_cluster,instance.cluster_count)).or_insert_with(||{
                let mut triangles=Vec::new();
                let start=instance.first_cluster as usize;
                let end=start.saturating_add(instance.cluster_count as usize).min(self.clusters.len());
                for cluster in self.clusters.get(start..end).unwrap_or_default() {
                    if cluster.own_error!=0.0{continue;}
                    let Some(material)=self.materials.get(cluster.material as usize) else{continue;};
                    if material.flags & (ALPHA_TEST|NO_AMBIENT_OCCLUSION)!=0{continue;}
                    for triangle in cluster.first_tri..cluster.first_tri.saturating_add(cluster.tri_count) {
                        let Some(&packed)=self.tris.get(triangle as usize) else{continue;};
                        let mut points=[Vec3::ZERO;3];let mut valid=true;
                        for (corner,point) in points.iter_mut().enumerate() {
                            let local=(packed>>(corner*8))&255;
                            let index=self.cluster_verts.get(cluster.first_vert as usize+local as usize).and_then(|i|self.vertices.get(*i as usize));
                            match index {Some(vertex)=>{*point=Vec3::from(vertex.pos);valid&=point.is_finite();},None=>valid=false}
                        }
                        if valid{triangles.push(points);}
                    }
                }
                solid_segments(&triangles)
            });
            for (index,&(center,half)) in boxes.iter().enumerate() {
                result.push(GpuSolidOccluder{center:[center.x,center.y,center.z,0.0],half_extent:[half.x,half.y,half.z,half.length()],meta:[owner as u32,(boxes.len()-index) as u32,0,0]});
            }
        }
        result
    }

    /// Add a mesh; returns its id for instancing.
    pub fn add_mesh(&mut self, m: &ClusterMesh) -> usize {
        let vbase = self.vertices.len() as u32;
        let cvbase = self.cluster_verts.len() as u32;
        let tbase = self.tris.len() as u32;
        let first_cluster = self.clusters.len() as u32;
        self.vertices.extend_from_slice(&m.vertices);
        self.cluster_verts.extend(m.cluster_verts.iter().map(|v| v + vbase));
        self.tris.extend_from_slice(&m.tris);
        for c in &m.clusters {
            let mut c = *c;
            c.first_vert += cvbase;
            c.first_tri += tbase;
            self.clusters.push(c);
        }
        self.meshes.push(MeshRef { first_cluster, cluster_count: m.clusters.len() as u32, sphere: m.sphere });
        self.meshes.len() - 1
    }

    pub fn add_instance(&mut self, mesh: usize, xf: Affine3A) {
        self.instances.push(Self::instance(&self.meshes[mesh], xf, 0));
    }

    /// Label at the source maker, rather than inferring a category from a
    /// material or behavioral flag (a tree contains bark as well as leaves).
    pub fn add_classified_instance(&mut self, mesh: usize, xf: Affine3A, category: GeometryCategory) {
        self.instances.push(Self::instance(&self.meshes[mesh], xf, (category as u32) << INSTANCE_CATEGORY_SHIFT));
    }

    pub fn set_instance_category(&mut self, index: usize, category: GeometryCategory) {
        self.instances[index].set_category(category);
    }

    pub fn set_instance_categories(&mut self, range: std::ops::Range<usize>, category: GeometryCategory) {
        for instance in &mut self.instances[range] { instance.set_category(category); }
    }

    /// Explicit O(source instances) startup/diagnostic scan; never part of rendering.
    pub fn geometry_category_counts(&self) -> [GeometryCategoryCounts; 5] {
        let mut counts = [GeometryCategoryCounts::default(); 5];
        for instance in &self.instances {
            let count = &mut counts[instance.category() as usize];
            count.instances += 1;
            count.active_instances += u64::from(instance.cluster_count > 0);
            count.referenced_clusters += u64::from(instance.cluster_count);
        }
        counts
    }

    pub fn update_instance(&mut self, index: usize, mesh: usize, xf: Affine3A) {
        let flags = self.instances[index].flags;
        self.instances[index] = Self::instance(&self.meshes[mesh], xf, flags);
    }

    fn instance(m: &MeshRef, xf: Affine3A, flags: u32) -> GpuInstance {
        let c = xf.transform_point3(Vec3::from_slice(&m.sphere[..3]));
        let cols = xf.matrix3;
        let scale = cols.x_axis.length().max(cols.y_axis.length()).max(cols.z_axis.length());
        let mat = glam::Mat4::from(xf);
        let r = mat.transpose();
        GpuInstance {
            rows: [r.x_axis.to_array(), r.y_axis.to_array(), r.z_axis.to_array()],
            sphere: [c.x, c.y, c.z, m.sphere[3] * scale],
            first_cluster: m.first_cluster,
            cluster_count: m.cluster_count,
            scale,
            flags,
        }
    }

    /// Mark the last instance as a source of opaque near-field proxies.
    pub fn mark_solid(&mut self) {
        if let Some(i) = self.instances.last_mut() {
            i.flags |= SOLID;
        }
    }

    /// Mark the last instance as foliage (a tree crown for the GI volume).
    pub fn mark_foliage(&mut self) {
        if let Some(i) = self.instances.last_mut() {
            i.flags |= FOLIAGE;
        }
    }

    pub fn mark_no_shadow(&mut self) {
        if let Some(i) = self.instances.last_mut() {
            i.flags |= NO_SHADOW;
        }
    }

    pub fn add_material(&mut self, m: Material) -> u32 {
        self.materials.push(m);
        (self.materials.len() - 1) as u32
    }

    /// Add a procedural car surface; returns the material flag bits that
    /// select it (or them into `Material::flags`).
    pub fn add_car_material(&mut self, c: GpuCarMaterial) -> u32 {
        self.car_materials.push(c);
        (self.car_materials.len() as u32) << CAR_SHIFT
    }

    /// Preserve continuous alpha, including opaque texels of a blended badge.
    /// An EMISSIVE material blends its emission only, unlit (signs, labels).
    /// The same material is excluded from both depth/shadow and solid proxies.
    pub fn set_material_blend(&mut self, material: u32, alpha: f32, double_sided: bool) {
        self.materials[material as usize].flags |= BLEND | NO_AMBIENT_OCCLUSION;
        self.blended_materials.insert(material, BlendMaterial { alpha: alpha.clamp(0.0, 1.0), double_sided, additive: false, object_sort: false });
    }

    /// Sort this blended material's triangles by their instance origin.
    pub fn set_material_object_sort(&mut self, material: u32) {
        if let Some(blend) = self.blended_materials.get_mut(&material) { blend.object_sort = true; }
    }

    /// Add continuous-alpha radiance without obscuring the scene behind it.
    /// EMISSIVE surfaces use only their emission, attenuated by fog transmission.
    /// Like ordinary blending, this excludes opaque depth, shadows and proxies.
    pub fn set_material_additive(&mut self, material: u32, alpha: f32, double_sided: bool) {
        self.set_material_blend(material, alpha, double_sided);
        self.blended_materials.get_mut(&material).unwrap().additive = true;
    }

    /// Load an image file into the texture array; returns its layer.
    pub fn add_texture(&mut self, path: &std::path::Path) -> Result<i32, String> {
        self.add_texture_with_normal_encoding(path, NormalEncoding::LinearUnorm)
    }

    /// Load an albedo and its sibling normal map with explicit source encoding.
    /// Only the normal RGB is decoded; albedo and normal alpha stay untouched.
    pub fn add_texture_with_normal_encoding(&mut self, path: &std::path::Path, encoding: NormalEncoding) -> Result<i32, String> {
        // The normal map sits beside the albedo: x_albedo.jpg, x_normal.jpg (or .png).
        let normal_path = path.file_name().and_then(|f| f.to_str()).filter(|f| f.contains("_albedo.")).and_then(|f| {
            let stem = f.split("_albedo.").next().unwrap_or(f);
            ["jpg", "png"].iter().map(|e| path.with_file_name(format!("{stem}_normal.{e}"))).find(|p| p.exists())
        });
        let (img, normal) = std::thread::scope(|s| {
            let n = s.spawn(|| normal_path.and_then(|p| image::open(p).ok()).map(|i| {
                let mut normal = i.to_rgba8();
                encoding.decode(&mut normal);
                normal
            }));
            (image::open(path), n.join().unwrap())
        });
        let img = img.map_err(|e| format!("{}: {e}", path.display()))?.to_rgba8();
        self.textures.push(TextureSource { name: path.display().to_string(), rgba: img, normal });
        Ok((self.textures.len() - 1) as i32)
    }

    /// Add an already decoded image to the texture array; returns its layer.
    /// Reserve room in `aux` for `capacity` bullet dents (`dents`), none
    /// until `Renderer::set_dents`. Call after the game's own `aux` data.
    pub fn reserve_dents(&mut self, capacity: usize) -> Result<(), String> {
        if self.dents.is_some() { return Err("the scene already has dents".into()); }
        if self.aux.is_empty() { self.aux.push(0); }
        let words = crate::dents::words(capacity);
        if self.aux.len() + words >= 1 << 24 { return Err("aux too large for a dent offset".into()); }
        self.dents = Some((self.aux.len(), capacity));
        self.aux.extend(crate::dents::pack(capacity, &[]));
        Ok(())
    }

    /// Reserve a 3D Euclidean distance field (`dfao`) in `aux`, unoccluded
    /// until `Renderer::set_dfao`. Call after the game's own `aux` data.
    pub fn reserve_dfao(&mut self, grid: crate::dfao::Grid) -> Result<(), String> {
        if self.dfao.is_some() { return Err("the scene already has a dfao field".into()); }
        let words = grid.empty_words()?;
        if self.aux.is_empty() { self.aux.push(0); }
        if self.aux.len() + words.len() >= 1 << 24 { return Err("aux too large for a dfao offset".into()); }
        self.dfao = Some((self.aux.len(), grid));
        self.aux.extend(words);
        Ok(())
    }

    /// Reserve indoor irradiance probes (`irradiance_volume`) in `aux`, dark
    /// until `Renderer::set_irradiance_volume`. Call after the game's own
    /// `aux` data is in place; one volume per scene.
    pub fn reserve_irradiance_volume(&mut self, grid: crate::irradiance_volume::Grid) -> Result<(), String> {
        if self.irradiance_volume.is_some() { return Err("the scene already has an irradiance volume".into()); }
        let words = crate::irradiance_volume::pack(&grid, &vec![Default::default(); grid.count()])?;
        // Offset 0 means "no volume" to the shader.
        if self.aux.is_empty() { self.aux.push(0); }
        if self.aux.len() + words.len() >= 1 << 24 { return Err("aux too large for an irradiance volume offset".into()); }
        self.irradiance_volume = Some((self.aux.len(), grid));
        self.aux.extend(words);
        Ok(())
    }

    /// Reserve an albedo layer that `screen::Screen` draws at runtime (TVs,
    /// signs, covers that arrive later). Black until the first draw. Pair it
    /// with an EMISSIVE material for a glowing screen.
    pub fn add_screen_texture(&mut self, name: &str) -> i32 {
        self.add_image(name, image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255])))
    }

    /// Broadcast a completed scene view to this layer at up to 30 Hz, before
    /// HUD, with one frame of latency and no extra scene render. All screens
    /// can share the returned layer. Pair with EMISSIVE | LIVE_SCREEN.
    pub fn add_live_screen_texture(&mut self, name: &str, source_view: usize, aspect: f32) -> Result<i32, String> {
        if !aspect.is_finite() || aspect <= 0.0 || source_view >= 4 { return Err("live screen requires view 0..3 and a positive finite aspect".into()); }
        let layer = self.add_screen_texture(name);
        self.live_screen_feeds.push(crate::screen_feed::Feed { layer: layer as u32, source_view, aspect });
        Ok(layer)
    }

    pub fn add_image(&mut self, name: &str, rgba: image::RgbaImage) -> i32 {
        self.textures.push(TextureSource { name: name.to_string(), rgba, normal: None });
        (self.textures.len() - 1) as i32
    }

    pub fn load_sky(&mut self, path: &std::path::Path) -> Result<(), String> {
        let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.to_rgba32f();
        let (w, h) = img.dimensions();
        let px = img.pixels().map(|p| [p[0], p[1], p[2], 1.0]).collect();
        self.sky = Some((w, h, px));
        Ok(())
    }

    pub fn stats(&self) -> String {
        let leaf: usize = self.clusters.iter().filter(|c| c.own_error == 0.0).map(|c| c.tri_count as usize).sum();
        format!(
            "{} meshes, {} instances, {} clusters ({} leaf triangles), {} vertices, {} textures",
            self.meshes.len(),
            self.instances.len(),
            self.clusters.len(),
            leaf,
            self.vertices.len(),
            self.textures.len()
        )
    }
}

#[cfg(test)]
mod geometry_category_tests {
    use super::*;

    fn scene() -> Scene {
        let mut scene = Scene::default();
        scene.meshes.push(MeshRef { first_cluster: 12, cluster_count: 7, sphere: [1.0, 2.0, 3.0, 4.0] });
        scene
    }

    #[test]
    fn geometry_category_preserves_abi_geometry_and_behavior_bits() {
        assert_eq!(std::mem::size_of::<GpuInstance>(), 80);
        let behavior = FOLIAGE | SOLID | NO_SHADOW | LIVE_GRASS | WIND_GRASS | PART_WIND;
        assert_eq!(behavior & INSTANCE_CATEGORY_MASK, 0);
        let mut scene = scene();
        scene.add_instance(0, Affine3A::from_translation(Vec3::new(10.0, 5.0, -40.0)));
        scene.instances[0].flags = behavior | (1 << 31);
        let before = scene.instances[0];
        // Being foliage alone never implicitly assigns tree provenance.
        assert_eq!(before.category(), GeometryCategory::Other);
        for category in GeometryCategory::ALL {
            scene.set_instance_category(0, category);
            let after = scene.instances[0];
            assert_eq!(after.category(), category);
            assert_eq!(after.flags & !INSTANCE_CATEGORY_MASK, before.flags);
            assert_eq!(&bytemuck::bytes_of(&after)[..76], &bytemuck::bytes_of(&before)[..76]);
            assert_eq!(category.exclusion_bit(), 1 << category as u32);
        }
    }

    #[test]
    fn geometry_category_survives_transform_and_mesh_updates() {
        let mut scene = scene();
        scene.add_classified_instance(0, Affine3A::IDENTITY, GeometryCategory::Trees);
        scene.mark_foliage();
        scene.meshes.push(MeshRef { first_cluster: 30, cluster_count: 3, sphere: [0.0, 0.0, 0.0, 1.0] });
        scene.update_instance(0, 1, Affine3A::from_translation(Vec3::X * 99.0));
        assert_eq!(scene.instances[0].category(), GeometryCategory::Trees);
        assert_eq!(scene.instances[0].flags & FOLIAGE, FOLIAGE);
        assert_eq!(scene.instances[0].first_cluster, 30);
        assert_eq!(scene.instances[0].sphere[0], 99.0);
    }

    #[test]
    fn geometry_category_counts_distinguish_reserved_slots_and_available_lods() {
        let mut scene = scene();
        for category in [GeometryCategory::Other, GeometryCategory::Grass, GeometryCategory::Grass,
            GeometryCategory::Cars, GeometryCategory::Cars] {
            scene.add_classified_instance(0, Affine3A::IDENTITY, category);
        }
        scene.instances[2].cluster_count = 0;
        scene.set_instance_categories(3..5, GeometryCategory::Trees);
        let counts = scene.geometry_category_counts();
        assert_eq!(counts[GeometryCategory::Other as usize].instances, 1);
        assert_eq!(counts[GeometryCategory::Grass as usize], GeometryCategoryCounts {
            instances: 2, active_instances: 1, referenced_clusters: 7,
        });
        assert_eq!(counts[GeometryCategory::Trees as usize], GeometryCategoryCounts {
            instances: 2, active_instances: 2, referenced_clusters: 14,
        });
        assert_eq!(counts[GeometryCategory::Cars as usize], GeometryCategoryCounts::default());
        assert_eq!(counts.iter().map(|c| c.instances).sum::<u64>(), scene.instances.len() as u64);
    }
}

#[cfg(test)]
mod normal_encoding_tests {
    use super::NormalEncoding;
    use image::{Rgba, RgbaImage};

    #[test]
    fn normal_encoding_srgb_neutral_and_alpha() {
        let mut image = RgbaImage::from_fn(256, 1, |x, _| Rgba([188, 188, 255, x as u8]));
        NormalEncoding::SrgbTransferred.decode(&mut image);
        for (x, pixel) in image.pixels().enumerate() {
            assert_eq!(pixel.0, [128, 128, 255, x as u8]);
        }
    }

    #[test]
    fn normal_encoding_uses_iec_piecewise_transfer() {
        // Independent reference byte values include both sides of the toe.
        let encoded = [0, 10, 11, 64, 128, 188, 255];
        let expected = [0, 1, 1, 13, 55, 128, 255];
        let mut image = RgbaImage::from_fn(7, 1, |x, _| Rgba([encoded[x as usize]; 4]));
        NormalEncoding::SrgbTransferred.decode(&mut image);
        for (i, pixel) in image.pixels().enumerate() {
            assert_eq!(pixel.0, [expected[i], expected[i], expected[i], encoded[i]]);
        }
    }

    #[test]
    fn normal_encoding_tilted_vectors_round_trip() {
        let vectors = [[0.36f64, 0.48, 0.8], [-0.6, 0.0, 0.8],
            [0.0, -0.8, 0.6], [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]];
        let forward = |signed: f64| {
            let linear = signed * 0.5 + 0.5;
            let encoded = if linear <= 0.0031308 { linear * 12.92 }
                else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 };
            (encoded * 255.0).round() as u8
        };
        for vector in vectors {
            let mut image = RgbaImage::from_pixel(1, 1,
                Rgba([forward(vector[0]), forward(vector[1]), forward(vector[2]), 73]));
            NormalEncoding::SrgbTransferred.decode(&mut image);
            for (channel, expected) in vector.into_iter().enumerate() {
                let actual = image.get_pixel(0, 0)[channel] as f64 * (2.0 / 255.0) - 1.0;
                // Source transfer quantization plus linear UNORM quantization.
                assert!((actual - expected).abs() <= 0.013, "{vector:?}: {channel} = {actual}");
            }
            assert_eq!(image.get_pixel(0, 0)[3], 73);
        }
    }

    #[test]
    fn normal_encoding_linear_is_byte_identical_including_fallback() {
        assert_eq!(NormalEncoding::default(), NormalEncoding::LinearUnorm);
        let mut image = RgbaImage::from_fn(256, 1, |x, _|
            Rgba([x as u8, (255 - x) as u8, (x * 37) as u8, (x * 71) as u8]));
        let original = image.clone();
        NormalEncoding::LinearUnorm.decode(&mut image);
        assert_eq!(image, original);
        let mut flat = RgbaImage::from_pixel(1, 1, Rgba([128, 128, 255, 255]));
        NormalEncoding::LinearUnorm.decode(&mut flat);
        assert_eq!(flat.get_pixel(0, 0).0, [128, 128, 255, 255]);
    }
}

#[cfg(test)]
mod solid_tests {
    use super::*;

    fn material(flags:u32)->Material {Material{tint:[1.0;3],layer:-1,roughness:0.5,flags,scale:1.0,metal:0.0}}

    fn boxes(parts:&[(Vec3,Vec3,u32)])->ClusterMesh {
        let mut mesh=ClusterMesh::default();
        let indices=[[0u32,2,1],[1,2,3],[4,5,6],[5,7,6],[0,1,4],[1,5,4],[2,6,3],[3,6,7],[0,4,2],[2,4,6],[1,3,5],[3,7,5]];
        for &(lo,hi,material) in parts {
            let first_vert=mesh.cluster_verts.len() as u32;let first_tri=mesh.tris.len() as u32;
            for corner in 0..8 {
                let p=Vec3::new(if corner&1==0{lo.x}else{hi.x},if corner&2==0{lo.y}else{hi.y},if corner&4==0{lo.z}else{hi.z});
                mesh.cluster_verts.push(mesh.vertices.len() as u32);
                mesh.vertices.push(Vertex{pos:p.to_array(),normal:Vec3::Y.to_array(),uv:[0.0;2]});
            }
            for [a,b,c] in indices{mesh.tris.push(a|(b<<8)|(c<<16));}
            let mut cluster=GpuCluster::zeroed();cluster.first_vert=first_vert;cluster.first_tri=first_tri;cluster.tri_count=12;cluster.material=material;
            mesh.clusters.push(cluster);
        }
        mesh.sphere=[0.0,0.0,0.0,10.0];mesh
    }

    fn scene(extra:Option<u32>)->(Scene,usize) {
        let mut scene=Scene::default();scene.add_material(material(0));
        let mut parts=vec![(Vec3::new(-1.0,0.0,-3.0),Vec3::new(1.0,1.0,3.0),0)];
        if let Some(flags)=extra {
            let glass=scene.add_material(material(flags));
            parts.push((Vec3::new(-4.0,3.0,-8.0),Vec3::new(4.0,6.0,8.0),glass));
        }
        let mesh=scene.add_mesh(&boxes(&parts));scene.add_instance(mesh,Affine3A::IDENTITY);scene.mark_solid();(scene,mesh)
    }

    #[test]
    fn local_opaque_segments_fit_geometry_without_transparent_expansion() {
        let (opaque,_)=scene(None);let expected=opaque.solid_occluders();assert_eq!(expected.len(),6);
        for flag in [ALPHA_TEST,NO_AMBIENT_OCCLUSION] {
            let (mixed,_)=scene(Some(flag));let actual=mixed.solid_occluders();
            assert_eq!(bytemuck::cast_slice::<_,u8>(&actual),bytemuck::cast_slice::<_,u8>(&expected));
        }
        for occluder in &expected {
            let center=Vec3::from_slice(&occluder.center);let half=Vec3::from_slice(&occluder.half_extent);
            assert!(half.min_element()>0.0 && half.is_finite());
            assert!((center-half).cmpge(Vec3::new(-1.0,0.0,-3.0)).all());
            assert!((center+half).cmple(Vec3::new(1.0,1.0,3.0)).all());
        }
        for pair in expected.windows(2) {
            assert!((pair[0].center[2]+pair[0].half_extent[2]-(pair[1].center[2]-pair[1].half_extent[2])).abs()<1e-5);
        }
    }

    #[test]
    fn static_descriptor_uses_live_owner_and_does_not_need_pose_reupload() {
        let (mut scene,mesh)=scene(None);let before=scene.solid_occluders();
        scene.update_instance(0,mesh,Affine3A::from_scale_rotation_translation(Vec3::new(1.1,0.9,1.0),glam::Quat::from_rotation_y(1.2),Vec3::new(140.0,4.0,-720.0)));
        let after=scene.solid_occluders();
        assert_eq!(std::mem::size_of::<GpuSolidOccluder>(),48);
        assert_eq!(bytemuck::cast_slice::<_,u8>(&before),bytemuck::cast_slice::<_,u8>(&after));
        for car in 1..4 {scene.add_instance(mesh,Affine3A::from_translation(Vec3::X*car as f32*5.0));scene.mark_solid();}
        let four=scene.solid_occluders();assert_eq!(four.len(),4*MAX_SOLID_SEGMENTS);
        for (owner,group) in four.chunks_exact(MAX_SOLID_SEGMENTS).enumerate(){assert!(group.iter().all(|b|b.meta[0]==owner as u32));}
    }

    #[test]
    fn non_solids_and_fully_transparent_bodies_emit_no_occluders() {
        let (mut scene,_)=scene(None);scene.instances[0].flags=FOLIAGE;assert!(scene.solid_occluders().is_empty());
        scene.instances[0].flags=SOLID;scene.materials[0].flags=NO_AMBIENT_OCCLUSION;assert!(scene.solid_occluders().is_empty());
        let p=Vec3::ZERO;assert!(solid_segments(&[[p,p,p]]).is_empty());
    }
}
