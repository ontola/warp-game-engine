//! Clustered local lights: up to [`MAX_CLUSTERED_LIGHTS`] point and spot
//! lights per frame, some of them shadowed.
//!
//! Lights are binned on the GPU into a world-space grid fitted around them
//! and the cameras (`clustered_bin.wgsl`, one invocation per cell, at most
//! [`MAX_LIGHTS_PER_CELL`] per cell in light order). Shade reads its pixel's
//! cell and loops over that list only (`clustered_lights.wgsl`).
//!
//! Shadowed lights draw depth into 256-texel tiles of extra layers in the
//! spot shadow texture array (the headlights' `headlight_depth`, so shade
//! binds no new texture): a spot light takes one tile, a point light six (a
//! cube). Tiles stay with their light while it keeps its [`LightShadow::id`],
//! and a tile only redraws when its light moved or, round robin, when it is
//! the oldest; at most [`ShadowBudget::faces_per_frame`] tiles redraw per
//! frame. A tile not drawn yet leaves its face unshadowed.
//!
//! The light arena (lights, shadow faces, grid cells) is a region at the
//! end of the scene tables buffer, which shade already binds, and the grid
//! is a uniform in its own bind group (group 2): shade stays at 8 storage
//! buffers per stage (browser.md). Opt-in: `Graphics::clustered_lights` or
//! `KERN_CLUSTERED_LIGHTS=1`; off, shade compiles none of it.
use crate::headlights::{LocalLight, SHADOW_NEAR, SHADOW_SIZE};
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::collections::HashMap;

pub const MAX_CLUSTERED_LIGHTS: usize = 1024;
/// Light indices per grid cell (u16 pairs after the count in 32 words).
pub const MAX_LIGHTS_PER_CELL: usize = 62;
pub const MAX_CELLS: u32 = 16384;
/// Shadow tile edge in texels; a 1024 layer holds 16.
pub const TILE: u32 = 256;
pub const TILES_PER_LAYER: u32 = (SHADOW_SIZE / TILE) * (SHADOW_SIZE / TILE);
/// Guard texels around each face, so filtering never leaves its tile.
const GUARD: f32 = 2.0;
/// Lights further than this from every camera are dropped (metres beyond
/// their range).
pub const MAX_LIGHT_DISTANCE_M: f32 = 600.0;
/// Vec4s per light: position and range, direction and outer cone,
/// intensity and inner cone, source (radius squared, falloff, shadow faces,
/// shape bits), face (`LightFace::gpu`).
const LIGHT_STRIDE: u32 = 5;
/// The LTC tables (`light_face`) sit ahead of the lights in the arena.
const LTC_VEC4S: u32 = crate::light_face::LTC_VEC4S;
const FACE_STRIDE: u32 = 6;
const CELL_STRIDE: u32 = 8;
/// Reflective shadow map texels (see `Resources::rsm`).
pub const RSM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

/// A clustered light: the same light model as `set_local_lights`, plus an
/// optional shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusteredLight {
    pub light: LocalLight,
    pub shadow: Option<LightShadow>,
    /// Cookie and angular profile (`light_shapes`); none by default.
    pub shape: crate::light_shapes::LightShape,
    /// The light's visible face (`light_face`): its specular highlight is
    /// that face's (an area light), its source radius the face's. None: a
    /// sphere of `source_radius_m`.
    pub face: Option<crate::light_face::LightFace>,
    /// Its first diffuse bounce is gathered live from its shadow tiles
    /// (reflective shadow maps, see `rsm`). For a light whose bounce is not
    /// baked into an irradiance volume; it needs a shadow.
    pub bounce: bool,
}

impl ClusteredLight {
    pub fn unshadowed(light: LocalLight) -> Self { Self { light, shadow: None, shape: Default::default(), face: None, bounce: false } }
    pub fn shadowed(light: LocalLight, id: u64) -> Self { Self { light, shadow: Some(LightShadow { id, exclude: None }), shape: Default::default(), face: None, bounce: false } }
    /// This light with its live first bounce (see `bounce`).
    pub fn with_bounce(self) -> Self { Self { bounce: true, ..self } }
    /// This light with a cookie and/or angular profile.
    pub fn with_shape(self, shape: crate::light_shapes::LightShape) -> Self { Self { shape, ..self } }
    /// This light with a visible face (see `light_face`).
    pub fn with_face(self, face: crate::light_face::LightFace) -> Self { Self { light: face.apply(self.light), face: Some(face), ..self } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightShadow {
    /// Stable per light while it lives: its tiles and their depth are kept
    /// between frames under this id.
    pub id: u64,
    /// Instances (start, count) that do not cast this light's shadow, such
    /// as the lamp's own housing or the torch it sits in.
    pub exclude: Option<(u32, u32)>,
}

/// How much shadow work clustered lights may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowBudget {
    /// Extra 1024-texel layers in the spot shadow array, 16 tiles each
    /// (4 MiB per layer). 0: no clustered shadows.
    pub layers: u32,
    /// Tiles redrawn per frame at most (each is a cull and a raster pass).
    pub faces_per_frame: u32,
}

impl Default for ShadowBudget {
    fn default() -> Self { Self { layers: 4, faces_per_frame: 12 } }
}

impl ShadowBudget {
    /// The defaults, or `KERN_CLUSTERED_SHADOW_LAYERS` /
    /// `KERN_CLUSTERED_SHADOW_FACES` (renderer-switches.md).
    pub fn configured() -> Self {
        let read = |switch: &crate::switches::Switch, default: u32, max: u32| switch.parse::<u32>().unwrap_or(default).min(max);
        let d = Self::default();
        Self { layers: read(&crate::switches::CLUSTERED_SHADOW_LAYERS, d.layers, 16), faces_per_frame: read(&crate::switches::CLUSTERED_SHADOW_FACES, d.faces_per_frame, 96) }
    }
    pub fn tiles(self) -> u32 { self.layers * TILES_PER_LAYER }
}

/// Whether a scene uses clustered lights: `KERN_CLUSTERED_LIGHTS` (1 or 0)
/// over `Graphics::clustered_lights`.
pub fn enabled(scene_wants: bool) -> bool {
    match crate::switches::CLUSTERED_LIGHTS.var().as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => scene_wants,
    }
}

pub fn validate(lights: &[ClusteredLight]) -> Result<Vec<ClusteredLight>, String> {
    if lights.len() > MAX_CLUSTERED_LIGHTS { return Err(format!("at most {MAX_CLUSTERED_LIGHTS} clustered lights are supported")); }
    lights.iter().map(|l| Ok(ClusteredLight { light: l.light.validated()?, ..*l })).collect()
}

/// Six faces for a light that reaches behind itself (or wider than a
/// 120 degree cone), one for a narrower spot.
pub fn face_count(light: &LocalLight) -> u32 { if light.outer_cos < 0.5 { 6 } else { 1 } }

/// One shadow face: where it looks and how it projects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub view_proj: Mat4,
    /// Focal length in tile texels (for LOD in the shadow raster).
    pub focal: f32,
    /// World size of a tile texel per metre of distance from the light.
    pub texel_per_m: f32,
    pub near: f32,
}

/// The faces of a light, in the order shade picks them: +X, -X, +Y, -Y,
/// +Z, -Z for a cube, or the spot's own direction.
pub fn faces(light: &LocalLight) -> Vec<Face> {
    let near = SHADOW_NEAR.max(light.source_radius_m * 0.5).min(light.range_m * 0.25);
    // Widen the projection so the face's own region ends GUARD texels
    // inside the tile.
    let widen = TILE as f32 / (TILE as f32 - 2.0 * GUARD);
    let make = |dir: Vec3, up: Vec3, tan_half: f32| {
        let t = tan_half * widen;
        let view_proj = Mat4::perspective_rh(2.0 * t.atan(), 1.0, near, light.range_m) * Mat4::look_at_rh(light.position, light.position + dir, up);
        Face { view_proj, focal: TILE as f32 / (2.0 * t), texel_per_m: 2.0 * t / TILE as f32, near }
    };
    if face_count(light) == 6 {
        [(Vec3::X, Vec3::Y), (Vec3::NEG_X, Vec3::Y), (Vec3::Y, Vec3::Z), (Vec3::NEG_Y, Vec3::Z), (Vec3::Z, Vec3::Y), (Vec3::NEG_Z, Vec3::Y)]
            .into_iter().map(|(d, u)| make(d, u, 1.0)).collect()
    } else {
        let d = light.direction;
        let up = if d.y.abs() > 0.9 { Vec3::Z } else { Vec3::Y };
        let c = light.outer_cos.clamp(0.5, 0.9999);
        vec![make(d, up, (1.0 - c * c).sqrt() / c)]
    }
}

/// Which face of a cube a direction from the light falls in (shade's
/// choice, `clustered_lights.wgsl`).
pub fn cube_face(d: Vec3) -> usize {
    let a = d.abs();
    if a.x >= a.y && a.x >= a.z { if d.x > 0.0 { 0 } else { 1 } } else if a.y >= a.z { if d.y > 0.0 { 2 } else { 3 } } else if d.z > 0.0 { 4 } else { 5 }
}

/// The world-space grid the lights are binned into.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub origin: Vec3,
    pub cell: Vec3,
    pub dims: [u32; 3],
}

impl Grid {
    /// Fitted around the lights' spheres, clipped to the region near the
    /// cameras, with roughly cubic cells and at most [`MAX_CELLS`].
    pub fn fit(lights: &[LocalLight], cameras: &[Vec3]) -> Option<Grid> {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for l in lights {
            lo = lo.min(l.position - Vec3::splat(l.range_m));
            hi = hi.max(l.position + Vec3::splat(l.range_m));
        }
        if !cameras.is_empty() {
            let mut clo = Vec3::splat(f32::INFINITY);
            let mut chi = Vec3::splat(f32::NEG_INFINITY);
            for c in cameras { clo = clo.min(*c - Vec3::splat(MAX_LIGHT_DISTANCE_M)); chi = chi.max(*c + Vec3::splat(MAX_LIGHT_DISTANCE_M)); }
            lo = lo.max(clo);
            hi = hi.min(chi);
        }
        if !(lo.cmplt(hi).all()) { return None; }
        let extent = (hi - lo).max(Vec3::splat(0.01));
        let mut cell = (extent.x * extent.y * extent.z / MAX_CELLS as f32).cbrt();
        let dims = loop {
            let d = (extent / cell).ceil().clamp(Vec3::ONE, Vec3::splat(256.0));
            if (d.x * d.y * d.z) as u32 <= MAX_CELLS { break d; }
            cell *= 1.05;
        };
        Some(Grid { origin: lo, cell: extent / dims, dims: [dims.x as u32, dims.y as u32, dims.z as u32] })
    }
    pub fn cells(&self) -> u32 { self.dims[0] * self.dims[1] * self.dims[2] }
    /// The CPU copy of the bin shader's cell list (tests, diagnostics).
    pub fn bin(&self, lights: &[LocalLight]) -> Vec<Vec<u16>> {
        (0..self.cells()).map(|c| {
            let (x, y, z) = (c % self.dims[0], (c / self.dims[0]) % self.dims[1], c / (self.dims[0] * self.dims[1]));
            let lo = self.origin + Vec3::new(x as f32, y as f32, z as f32) * self.cell;
            let hi = lo + self.cell;
            lights.iter().enumerate().filter(|(_, l)| {
                let d = (lo - l.position).max(Vec3::ZERO) + (l.position - hi).max(Vec3::ZERO);
                d.length_squared() < l.range_m * l.range_m
            }).map(|(i, _)| i as u16).take(MAX_LIGHTS_PER_CELL).collect()
        }).collect()
    }
}

/// A tile to draw this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceDraw {
    pub tile: u32,
    pub face: Face,
    pub position: Vec3,
    pub exclude: Option<(u32, u32)>,
}

impl FaceDraw {
    pub fn layer(&self) -> u32 { crate::headlights::MAX_LIGHTS as u32 + self.tile / TILES_PER_LAYER }
    /// Its layer in the reflective shadow maps.
    pub fn rsm_layer(&self) -> u32 { self.tile / TILES_PER_LAYER }
    /// Texel rectangle (x, y, size) in its layer.
    pub fn rect(&self) -> [u32; 3] {
        let per_row = SHADOW_SIZE / TILE;
        let t = self.tile % TILES_PER_LAYER;
        [(t % per_row) * TILE, (t / per_row) * TILE, TILE]
    }
}

struct Held {
    tiles: Vec<u32>,
    pose: [f32; 8],
    /// Frame each face was last drawn (None: never).
    drawn: Vec<Option<u64>>,
    seen: u64,
}

/// Which lights hold which shadow tiles, and which tiles redraw.
#[derive(Default)]
pub struct Atlas {
    held: HashMap<u64, Held>,
    free: Vec<u32>,
    tiles: u32,
    frame: u64,
}

/// What `Atlas::plan` decided for one frame.
#[derive(Default, Debug)]
pub struct Plan {
    /// Per input light: (first face record, face count), or None if
    /// unshadowed this frame.
    pub records: Vec<Option<(u32, u32)>>,
    /// Face records in order: projection, tile, and whether it holds depth.
    pub faces: Vec<(Face, u32, bool)>,
    pub draws: Vec<FaceDraw>,
    /// Shadowed lights that got no tiles (over budget).
    pub dropped: u32,
}

fn pose(l: &LocalLight) -> [f32; 8] {
    [l.position.x, l.position.y, l.position.z, l.range_m, l.direction.x, l.direction.y, l.direction.z, l.outer_cos]
}

impl Atlas {
    pub fn new(budget: ShadowBudget) -> Self {
        Self { held: HashMap::new(), free: (0..budget.tiles()).rev().collect(), tiles: budget.tiles(), frame: 0 }
    }

    /// Give tiles to the shadowed lights that matter most (nearest a camera
    /// relative to their range) and choose which tiles to draw.
    pub fn plan(&mut self, lights: &[ClusteredLight], cameras: &[Vec3], faces_per_frame: u32) -> Plan {
        self.frame += 1;
        let frame = self.frame;
        let mut plan = Plan { records: vec![None; lights.len()], ..Default::default() };
        let importance = |l: &LocalLight| {
            let d = cameras.iter().map(|c| c.distance(l.position)).fold(f32::INFINITY, f32::min);
            let d = if d.is_finite() { d } else { 0.0 };
            l.range_m / (d - l.range_m).max(1.0)
        };
        let mut order: Vec<usize> = (0..lights.len()).filter(|&i| lights[i].shadow.is_some()).collect();
        // Stable: equal importance keeps the game's order.
        order.sort_by(|&a, &b| importance(&lights[b].light).total_cmp(&importance(&lights[a].light)));
        // Choose who holds tiles, then release everyone else's.
        let mut chosen = Vec::new();
        let mut used = 0;
        let mut seen_ids = std::collections::HashSet::new();
        for &i in &order {
            let id = lights[i].shadow.unwrap().id;
            if !seen_ids.insert(id) { continue; }
            let need = face_count(&lights[i].light);
            if used + need <= self.tiles { used += need; chosen.push(i); } else { plan.dropped += 1; }
        }
        let keep: std::collections::HashSet<u64> = chosen.iter().map(|&i| lights[i].shadow.unwrap().id).collect();
        let mut release = Vec::new();
        for (id, held) in &self.held {
            let need = chosen.iter().find(|&&i| lights[i].shadow.unwrap().id == *id).map(|&i| face_count(&lights[i].light));
            if !keep.contains(id) || need != Some(held.tiles.len() as u32) { release.push(*id); }
        }
        release.sort_unstable();
        for id in release { let held = self.held.remove(&id).unwrap(); self.free.extend(held.tiles); }
        self.free.sort_unstable_by(|a, b| b.cmp(a));
        // Faces to draw: moved or new first (in importance order), then the
        // longest undrawn.
        let mut dirty = Vec::new();
        let mut stale = Vec::new();
        for &i in &chosen {
            let l = &lights[i].light;
            let id = lights[i].shadow.unwrap().id;
            let need = face_count(l);
            let free = &mut self.free;
            let held = self.held.entry(id).or_insert_with(|| Held {
                tiles: (0..need).map(|_| free.pop().expect("tiles counted above")).collect(),
                pose: [f32::NAN; 8], drawn: vec![None; need as usize], seen: frame,
            });
            held.seen = frame;
            if held.pose != pose(l) { held.pose = pose(l); held.drawn.iter_mut().for_each(|d| *d = None); }
            for f in 0..need as usize {
                match held.drawn[f] { None => dirty.push((i, f)), Some(at) => stale.push((at, i, f)) }
            }
        }
        stale.sort_by_key(|&(at, i, f)| (at, i, f));
        let picks: Vec<(usize, usize)> = dirty.into_iter().chain(stale.into_iter().map(|(_, i, f)| (i, f))).take(faces_per_frame as usize).collect();
        let mut light_faces: HashMap<usize, Vec<Face>> = HashMap::new();
        for &(i, f) in &picks {
            let faces = light_faces.entry(i).or_insert_with(|| faces(&lights[i].light));
            let held = self.held.get_mut(&lights[i].shadow.unwrap().id).unwrap();
            held.drawn[f] = Some(frame);
            plan.draws.push(FaceDraw { tile: held.tiles[f], face: faces[f], position: lights[i].light.position, exclude: lights[i].shadow.unwrap().exclude });
        }
        // Face records for shade, in input order.
        for (i, l) in lights.iter().enumerate() {
            let Some(s) = l.shadow else { continue };
            let Some(held) = self.held.get(&s.id) else { continue };
            if held.seen != frame || !chosen.contains(&i) { continue; }
            let faces = light_faces.remove(&i).unwrap_or_else(|| faces(&l.light));
            plan.records[i] = Some((plan.faces.len() as u32, faces.len() as u32));
            for (f, face) in faces.into_iter().enumerate() { plan.faces.push((face, held.tiles[f], held.drawn[f].is_some())); }
        }
        plan
    }

    pub fn tiles_held(&self) -> usize { self.held.values().map(|h| h.tiles.len()).sum() }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GridUniform {
    origin: [f32; 4],
    inv_cell: [f32; 4],
    /// xyz: cells per axis, w: light count (0: no clustered lights).
    dims: [u32; 4],
    /// In vec4s of the scene tables: x first face record, y first cell
    /// record, z cell count, w first light.
    offsets: [u32; 4],
}

/// Last frame's numbers (`Renderer::clustered_light_stats`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub lights: u32,
    pub shadowed: u32,
    pub dropped_shadows: u32,
    pub faces_drawn: u32,
    pub tiles_held: u32,
    pub cells: u32,
}

pub(crate) struct Resources {
    pub enabled: bool,
    pub budget: ShadowBudget,
    pub layout: wgpu::BindGroupLayout,
    pub bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    /// The scene tables buffer and where the arena starts in it (bytes).
    arena: wgpu::Buffer,
    lights_at: u32,
    bin: Option<(wgpu::ComputePipeline, wgpu::BindGroup)>,
    pub clear_tile: Option<wgpu::RenderPipeline>,
    /// Reflective shadow maps beside the shadow tiles: per texel the
    /// surface's albedo (RGB 565) and normal (octahedral 8:8), one layer
    /// per clustered shadow layer (see `rsm`).
    pub rsm: Option<wgpu::Texture>,
    pub rsm_array: wgpu::TextureView,
    faces_offset: u32,
    cells_offset: u32,
    pub atlas: std::sync::Mutex<Atlas>,
    pub lights: std::sync::Mutex<Vec<ClusteredLight>>,
    pub stats: std::sync::Mutex<Stats>,
}

/// The budget a scene gets: none while clustered lights are off.
pub fn budget(enabled: bool) -> ShadowBudget {
    if enabled { ShadowBudget::configured() } else { ShadowBudget { layers: 0, faces_per_frame: 0 } }
}

/// Bytes the arena takes at the end of the scene tables.
pub fn arena_bytes(enabled: bool, budget: ShadowBudget) -> u64 {
    if !enabled { return 0; }
    (LTC_VEC4S as u64 + MAX_CLUSTERED_LIGHTS as u64 * LIGHT_STRIDE as u64 + budget.tiles() as u64 * FACE_STRIDE as u64 + MAX_CELLS as u64 * CELL_STRIDE as u64) * 16
}

impl Resources {
    /// `tables`: the scene tables buffer, with `arena_bytes` reserved from
    /// byte `base` (SceneTables::clustered).
    pub fn new(gpu: &Gpu, enabled: bool, budget: ShadowBudget, tables: &wgpu::Buffer, base: u64) -> Self {
        // The LTC tables first (written once), then the lights.
        let lights_at = (base / 16) as u32 + LTC_VEC4S;
        if enabled { gpu.queue.write_buffer(tables, base, bytemuck::cast_slice(&crate::light_face::ltc_words())); }
        let faces_offset = lights_at + MAX_CLUSTERED_LIGHTS as u32 * LIGHT_STRIDE;
        let cells_offset = faces_offset + budget.tiles() * FACE_STRIDE;
        let arena = tables.clone();
        let uniform = wgpu::util::DeviceExt::create_buffer_init(&gpu.device, &wgpu::util::BufferInitDescriptor { label: Some("clustered light grid"),
            contents: bytemuck::bytes_of(&GridUniform::zeroed()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
        let uniform_ty = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("clustered lights"), entries: &[entry(48, uniform_ty)] });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("clustered lights"), layout: &layout, entries: &[
            wgpu::BindGroupEntry { binding: 48, resource: uniform.as_entire_binding() },
        ] });
        let bin = enabled.then(|| {
            let bin_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("clustered light bin"), entries: &[
                entry(0, uniform_ty),
                entry(1, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }),
            ] });
            let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("clustered light bin"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/clustered_bin.wgsl").into()) });
            let pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("clustered light bin"), bind_group_layouts: &[Some(&bin_layout)], immediate_size: 0 });
            let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("clustered light bin"), layout: Some(&pl), module: &module,
                entry_point: Some("bin"), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("clustered light bin"), layout: &bin_layout, entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: arena.as_entire_binding() },
            ] });
            (pipeline, bind)
        });
        let clear_tile = (enabled && budget.layers > 0).then(|| {
            let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("clear shadow tile"),
                source: wgpu::ShaderSource::Wgsl(CLEAR_TILE.into()) });
            let pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("clear shadow tile"), bind_group_layouts: &[], immediate_size: 0 });
            gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor { label: Some("clear shadow tile"), layout: Some(&pl),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
                multisample: Default::default(),
                // The reflective shadow map clears to no surface (0).
                fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: RSM_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
                multiview_mask: None, cache: gpu.pipeline_cache() })
        });
        let rsm = (enabled && budget.layers > 0).then(|| gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("reflective shadow maps"),
            size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: budget.layers }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: RSM_FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }));
        let rsm_array = rsm.clone().unwrap_or_else(|| gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("no reflective shadow maps"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: RSM_FORMAT, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] }))
            .create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        Self { enabled, budget, layout, bind, uniform, arena, lights_at, bin, clear_tile, rsm, rsm_array,faces_offset, cells_offset,
            atlas: std::sync::Mutex::new(Atlas::new(budget)), lights: Default::default(), stats: Default::default() }
    }

    /// Upload this frame's lights and shadow faces, bin them, and return
    /// the shadow tiles to draw.
    pub fn prepare(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, cameras: &[Vec3]) -> Vec<FaceDraw> {
        let Some((bin, bin_bind)) = &self.bin else { return Vec::new() };
        let all = self.lights.lock().unwrap().clone();
        // Drop lights far from every camera before anything else.
        let lights: Vec<ClusteredLight> = all.into_iter().filter(|l| cameras.is_empty()
            || cameras.iter().any(|c| c.distance(l.light.position) < l.light.range_m + MAX_LIGHT_DISTANCE_M)).collect();
        let plain: Vec<LocalLight> = lights.iter().map(|l| l.light).collect();
        let grid = Grid::fit(&plain, cameras);
        let mut u = GridUniform::zeroed();
        let mut stats = Stats { lights: lights.len() as u32, shadowed: lights.iter().filter(|l| l.shadow.is_some()).count() as u32, ..Default::default() };
        let plan = if self.budget.layers > 0 { self.atlas.lock().unwrap().plan(&lights, cameras, self.budget.faces_per_frame) } else { Plan { records: vec![None; lights.len()], ..Default::default() } };
        if let Some(grid) = grid {
            let mut words: Vec<[f32; 4]> = Vec::with_capacity(lights.len() * LIGHT_STRIDE as usize + plan.faces.len() * 6);
            for (l, record) in lights.iter().zip(&plan.records) {
                // Bit 16: its bounce is gathered from its shadow tiles (rsm).
                let shape = l.shape.packed() + if l.bounce && record.is_some() { 65536.0 } else { 0.0 };
                let face = l.face.map_or([0.0; 4], |f| f.gpu(&l.light));
                let l = &l.light;
                let (first, count) = record.map_or((0.0, 0.0), |(f, c)| (f as f32 + 1.0, c as f32));
                words.push(l.position.extend(l.range_m).to_array());
                words.push(l.direction.extend(l.outer_cos).to_array());
                words.push(l.intensity_cd.extend(l.inner_cos).to_array());
                // w: shadow face count (0..7) plus the cookie and profile bits.
                words.push([l.source_radius_m * l.source_radius_m, l.falloff, first, count + shape]);
                words.push(face);
            }
            gpu.queue.write_buffer(&self.arena, self.lights_at as u64 * 16, bytemuck::cast_slice(&words));
            words.clear();
            for (face, tile, valid) in &plan.faces {
                let draw = FaceDraw { tile: *tile, face: *face, position: Vec3::ZERO, exclude: None };
                let [x, y, size] = draw.rect();
                let s = SHADOW_SIZE as f32;
                words.extend(face.view_proj.to_cols_array_2d());
                words.push([x as f32 / s, y as f32 / s, size as f32 / s, if *valid { draw.layer() as f32 } else { -1.0 }]);
                words.push([face.texel_per_m, face.near, 0.0, 0.0]);
            }
            if !words.is_empty() { gpu.queue.write_buffer(&self.arena, self.faces_offset as u64 * 16, bytemuck::cast_slice(&words)); }
            u = GridUniform {
                origin: grid.origin.extend(0.0).to_array(),
                inv_cell: grid.cell.recip().extend(0.0).to_array(),
                dims: [grid.dims[0], grid.dims[1], grid.dims[2], lights.len() as u32],
                offsets: [self.faces_offset, self.cells_offset, grid.cells(), self.lights_at],
            };
            stats.cells = grid.cells();
        }
        gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        if u.dims[3] > 0 {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("clustered light bin"), timestamp_writes: None });
            pass.set_pipeline(bin);
            pass.set_bind_group(0, bin_bind, &[]);
            pass.dispatch_workgroups(u.offsets[2].div_ceil(64), 1, 1);
        }
        stats.dropped_shadows = plan.dropped;
        stats.faces_drawn = plan.draws.len() as u32;
        stats.tiles_held = self.atlas.lock().unwrap().tiles_held() as u32;
        *self.stats.lock().unwrap() = stats;
        plan.draws
    }
}

/// A full-tile triangle at the far plane (depth 1), drawn under the tile's
/// viewport before its casters.
const CLEAR_TILE: &str = "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    return vec4<f32>(p, 1.0, 1.0);
}
@fragment fn fs() -> @location(0) u32 { return 0u; }";

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp(x: f32, range: f32) -> LocalLight {
        LocalLight { position: Vec3::new(x, 1.0, 0.0), direction: Vec3::NEG_Y, intensity_cd: Vec3::splat(10.0), range_m: range,
            inner_cos: -0.999, outer_cos: -1.0, source_radius_m: 0.05, falloff: 2.0 }
    }

    #[test]
    fn grid_fits_lights_with_cubic_cells_and_bins_every_reach() {
        let lights: Vec<LocalLight> = (0..300).map(|i| lamp(i as f32 * 0.7, 3.0 + (i % 5) as f32)).collect();
        let grid = Grid::fit(&lights, &[Vec3::ZERO]).unwrap();
        assert!(grid.cells() <= MAX_CELLS);
        let ratio = grid.cell.max_element() / grid.cell.min_element();
        assert!(ratio < 1.6, "cells {:?}", grid.cell);
        let bins = grid.bin(&lights);
        // Every point within a light's range lies in a cell listing it.
        for (i, l) in lights.iter().enumerate().step_by(37) {
            for o in [Vec3::ZERO, Vec3::X * (l.range_m * 0.95), Vec3::new(0.3, -0.5, 0.4) * l.range_m] {
                let p = l.position + o;
                let c = ((p - grid.origin) / grid.cell).floor();
                let index = (c.z as u32 * grid.dims[1] + c.y as u32) * grid.dims[0] + c.x as u32;
                let list = &bins[index as usize];
                assert!(list.len() == MAX_LIGHTS_PER_CELL || list.contains(&(i as u16)), "light {i} missing at {p}");
            }
        }
        // Far lights are clipped to the cameras' region.
        let far = Grid::fit(&[lamp(0.0, 2.0), lamp(5000.0, 2.0)], &[Vec3::ZERO]).unwrap();
        assert!(far.origin.x + far.cell.x * far.dims[0] as f32 <= MAX_LIGHT_DISTANCE_M + 0.01);
        assert!(Grid::fit(&[lamp(5000.0, 2.0)], &[Vec3::ZERO]).is_none());
    }

    #[test]
    fn cube_faces_cover_every_direction_inside_their_tile() {
        let l = lamp(2.0, 8.0);
        let cube = faces(&l);
        assert_eq!(cube.len(), 6);
        for k in 0..2000 {
            let a = k as f32 * 2.399_963;
            let z = 1.0 - 2.0 * (k as f32 + 0.5) / 2000.0;
            let d = Vec3::new((1.0 - z * z).sqrt() * a.cos(), z, (1.0 - z * z).sqrt() * a.sin());
            let p = l.position + d * 5.0;
            let ndc = cube[cube_face(d)].view_proj.project_point3(p);
            // Inside the face's own region, GUARD texels from the edge.
            let edge = 1.0 - 2.0 * GUARD / TILE as f32 + 1e-4;
            assert!(ndc.x.abs() <= edge && ndc.y.abs() <= edge, "{d} -> {ndc}");
            assert!((0.0..=1.0).contains(&ndc.z));
        }
        let spot = LocalLight { direction: Vec3::new(0.3, -1.0, 0.1), outer_cos: 0.8, inner_cos: 0.9, ..l }.validated().unwrap();
        assert_eq!(face_count(&spot), 1);
        let f = faces(&spot)[0];
        let edge = spot.direction * 0.8 + spot.direction.any_orthonormal_vector() * 0.6;
        let ndc = f.view_proj.project_point3(spot.position + edge * 4.0);
        assert!(ndc.x.abs().max(ndc.y.abs()) <= 1.0 - 2.0 * GUARD / TILE as f32 + 1e-3, "{ndc}");
    }

    #[test]
    fn atlas_keeps_tiles_redraws_moved_lights_and_respects_the_budget() {
        let budget = ShadowBudget { layers: 1, faces_per_frame: 6 };
        let mut atlas = Atlas::new(budget);
        let cams = [Vec3::ZERO];
        let mut lights: Vec<ClusteredLight> = (0..4).map(|i| ClusteredLight::shadowed(lamp(i as f32 * 3.0, 4.0), 10 + i)).collect();
        lights.push(ClusteredLight::unshadowed(lamp(1.0, 4.0)));
        // 16 tiles: two cube lights fit, the far two do not.
        let p = atlas.plan(&lights, &cams, budget.faces_per_frame);
        assert_eq!(p.dropped, 2);
        assert_eq!(p.draws.len(), 6);
        assert!(p.records[0].is_some() && p.records[1].is_some() && p.records[4].is_none());
        // Faces not drawn yet are marked empty for shade.
        assert_eq!(p.faces.iter().filter(|f| f.2).count(), 6);
        let tiles0: Vec<u32> = p.faces.iter().map(|f| f.1).collect();
        let p = atlas.plan(&lights, &cams, budget.faces_per_frame);
        assert_eq!(p.faces.iter().filter(|f| f.2).count(), 12);
        assert_eq!(p.faces.iter().map(|f| f.1).collect::<Vec<_>>(), tiles0, "tiles stay with their lights");
        // Now all drawn: the budget refreshes the oldest, light 0 first.
        let p = atlas.plan(&lights, &cams, budget.faces_per_frame);
        assert!(p.draws.iter().all(|d| d.position == lights[0].light.position));
        // A moved light redraws first.
        lights[1].light.position.y += 0.5;
        let p = atlas.plan(&lights, &cams, budget.faces_per_frame);
        assert!(p.draws.iter().all(|d| d.position == lights[1].light.position));
        // Its slot is freed when it goes, and another light takes it.
        lights.remove(1);
        let p = atlas.plan(&lights, &cams, budget.faces_per_frame);
        assert_eq!(p.dropped, 1);
        assert_eq!(atlas.tiles_held(), 12);
        assert!(p.records[1].is_some());
        // Tiles never overlap and stay in range.
        let mut seen: Vec<u32> = p.faces.iter().map(|f| f.1).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), p.faces.len());
        assert!(seen.iter().all(|&t| t < budget.tiles()));
    }

    #[test]
    fn shaders_validate_and_shade_stays_at_eight_storage_buffers() {
        let validate = |src: &str| {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
            let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
            (module, info)
        };
        validate(include_str!("shaders/clustered_bin.wgsl"));
        let src = crate::wind::shade_source();
        let (module, info) = validate(&src);
        let mut constants = naga::back::PipelineConstants::default();
        constants.insert("FEATURE_CLUSTERED_LIGHTS".into(), 1.0);
        let (module, info) = naga::back::pipeline_constants::process_overrides(&module, &info, Some((naga::ShaderStage::Compute, "shade")), &constants).unwrap();
        let entry = module.entry_points.iter().position(|e| e.name == "shade").unwrap();
        let uses = info.get_entry_point(entry);
        let storage: Vec<_> = module.global_variables.iter()
            .filter(|(h, g)| !uses[*h].is_empty() && matches!(g.space, naga::AddressSpace::Storage { .. }))
            .map(|(_, g)| g.name.clone().unwrap_or_default()).collect();
        assert!(storage.len() <= 8, "shade reads {} storage buffers: {storage:?}", storage.len());
        assert!(storage.iter().any(|n| n == "scene_tables"));
    }

    #[test]
    fn lights_are_bounded_and_validated() {
        let l = ClusteredLight::unshadowed(lamp(0.0, 3.0));
        assert!(validate(&vec![l; MAX_CLUSTERED_LIGHTS]).is_ok());
        assert!(validate(&vec![l; MAX_CLUSTERED_LIGHTS + 1]).is_err());
        assert!(validate(&[ClusteredLight::unshadowed(LocalLight { range_m: f32::NAN, ..l.light })]).is_err());
        assert!((MAX_LIGHTS_PER_CELL + 2) / 2 <= CELL_STRIDE as usize * 4);
        assert!(MAX_CLUSTERED_LIGHTS <= u16::MAX as usize);
    }
}
