//! Live reflection probes: one box per room (`ProbeBox`: where to capture
//! and the room's bounds), captured by the renderer itself and kept current
//! a face at a time, so an opened door or a switched lamp shows in the
//! reflections a fraction of a second later with nothing baked.
//!
//! Each frame, after the players' views, the renderer draws the six cube
//! faces (128x128, 90 degrees) of one probe with its own cull, raster and shade
//! (`frame.rs`): the frame's shadow maps and lights, no TAA, exposure,
//! tone mapping or bloom, and nothing of the players' per-view state
//! changes. The faces are linear radiance in absolute units (shade's output
//! before exposure), with light faces dark: their light reaches a glossy
//! surface as the light's own analytic specular. Probes go round robin (4
//! probes: each fresh within 4 frames); all six faces in one frame, so a
//! moving object (a swinging lamp) shows once, not at six moments.
//!
//! Two cube arrays: `raw` holds the captures with box-filtered mips, and
//! `filtered` the GGX prefilter of `raw` per mip (roughness = mip / (levels
//! - 1)), by filtered importance sampling (Krivanek and Colbert 2007).
//! Shade box-projects the reflected ray into the box that holds the point
//! (outside every box, in a doorway or a wall between rooms, it blends the
//! boxes within [`FADE_M`] that the ray heads into, so a doorway has no
//! seam and a door's face does not show the room behind it), reads
//! `filtered` at the roughness's mip, and uses it where screen-space
//! reflections miss. See engine/docs/dynamic-gi.md.
use crate::Gpu;
use glam::{Mat4, Vec3};

/// Face edge in texels.
pub const FACE: u32 = 128;
/// Mips: 128 down to 1.
pub const LEVELS: u32 = 8;
pub const MAX_PROBES: usize = 8;
/// Outside every box (a doorway, a wall between rooms), a box counts in a
/// blend up to this far from its bounds, fading to 0 there.
pub const FADE_M: f32 = 0.5;
/// Near plane of the capture (metres).
pub const NEAR: f32 = 0.05;
/// GGX samples per prefiltered texel.
const SAMPLES: u32 = 64;

/// A probe: where it is captured and the box its reflections are projected
/// into (a room's inside bounds). The boxes are also where the scene's rooms
/// are for the froxel fog (`FogBounds::Rooms`). A room joined to another by
/// a door reaches through the opening to the door's leaf (the middle of the
/// wall), as `architecture` builds them; ending at the wall's face leaves
/// the doorway in no room: no smoke there, and its reveal takes the probe
/// of the room behind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProbeBox {
    pub capture: Vec3,
    pub min: Vec3,
    pub max: Vec3,
}

/// The six faces in cube order (+X, -X, +Y, -Y, +Z, -Z): look direction
/// and up, as a cube map's texel layout wants them.
const FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y), (Vec3::NEG_X, Vec3::Y), (Vec3::Y, Vec3::NEG_Z),
    (Vec3::NEG_Y, Vec3::Z), (Vec3::Z, Vec3::Y), (Vec3::NEG_Z, Vec3::Y),
];

/// View-projection for a face: 90 degrees, reversed infinite Z (as the
/// colour raster), mirrored in x, since a cube map is seen from inside
/// (its +X face has -Z on the right).
pub fn face_view_proj(capture: Vec3, face: usize) -> Mat4 {
    let (dir, up) = FACES[face];
    let proj = Mat4::perspective_infinite_reverse_rh(std::f32::consts::FRAC_PI_2, 1.0, NEAR);
    Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)) * proj * Mat4::look_at_rh(capture, capture + dir, up)
}

const SHADER: &str = "
struct Params { face: u32, layer: u32, levels: u32, samples: u32 };
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var raw: texture_cube_array<f32>;
@group(0) @binding(4) var samp: sampler;

// Level k of a face from level k - 1: the four texels' mean.
@compute @workgroup_size(8, 8)
fn down(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst);
    if any(id.xy >= size) { return; }
    let q = vec2<i32>(id.xy) * 2;
    let c = textureLoad(src, q, 0) + textureLoad(src, q + vec2<i32>(1, 0), 0)
        + textureLoad(src, q + vec2<i32>(0, 1), 0) + textureLoad(src, q + vec2<i32>(1, 1), 0);
    textureStore(dst, id.xy, c * 0.25);
}

// The direction of a face's texel centre (cube map layout).
fn face_dir(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let s = uv.x * 2.0 - 1.0;
    let t = uv.y * 2.0 - 1.0;
    switch face {
        case 0u: { return normalize(vec3<f32>(1.0, -t, -s)); }
        case 1u: { return normalize(vec3<f32>(-1.0, -t, s)); }
        case 2u: { return normalize(vec3<f32>(s, 1.0, t)); }
        case 3u: { return normalize(vec3<f32>(s, -1.0, -t)); }
        case 4u: { return normalize(vec3<f32>(s, -t, 1.0)); }
        default: { return normalize(vec3<f32>(-s, -t, -1.0)); }
    }
}

fn radical_inverse(b: u32) -> f32 { return f32(reverseBits(b)) * 2.3283064365386963e-10; }

// GGX prefilter of the raw cube for this level's roughness, the view along
// the normal (Karis 2013), with filtered importance sampling: each sample
// reads the raw mip whose texel covers its share of the lobe.
@compute @workgroup_size(8, 8)
fn prefilter(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst);
    if any(id.xy >= size) { return; }
    let n = face_dir(params.face, (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size));
    let level = log2(f32(textureDimensions(raw).x) / f32(size.x));
    let rough = level / f32(params.levels - 1u);
    if rough <= 0.0 {
        textureStore(dst, id.xy, textureSampleLevel(raw, samp, n, i32(params.layer / 6u), 0.0));
        return;
    }
    let a = rough * rough;
    let t1 = normalize(select(cross(n, vec3<f32>(0.0, 1.0, 0.0)), cross(n, vec3<f32>(1.0, 0.0, 0.0)), abs(n.y) > 0.9));
    let t2 = cross(n, t1);
    let texel_sa = 4.0 * 3.14159265 / (6.0 * f32(textureDimensions(raw).x * textureDimensions(raw).x));
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0u; i < params.samples; i++) {
        let xi = vec2<f32>(f32(i) / f32(params.samples), radical_inverse(i));
        let phi = 6.2831853 * xi.x;
        let cos_t = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
        let sin_t = sqrt(1.0 - cos_t * cos_t);
        let h = t1 * (sin_t * cos(phi)) + t2 * (sin_t * sin(phi)) + n * cos_t;
        let l = 2.0 * dot(n, h) * h - n;
        let nl = dot(n, l);
        if nl <= 0.0 { continue; }
        // pdf of l (n = v): D(h) / 4.
        let d = a * a / (3.14159265 * pow(cos_t * cos_t * (a * a - 1.0) + 1.0, 2.0));
        let sample_sa = 1.0 / (f32(params.samples) * d * 0.25 + 1e-6);
        let lod = clamp(0.5 * log2(sample_sa / texel_sa) + 1.0, 0.0, f32(params.levels - 1u));
        sum += textureSampleLevel(raw, samp, l, i32(params.layer / 6u), lod).rgb * nl;
        weight += nl;
    }
    textureStore(dst, id.xy, vec4<f32>(sum / max(weight, 1e-6), 1.0));
}
";

struct Capture {
    vis: wgpu::TextureView,
    depth: wgpu::TextureView,
    hdr: wgpu::Texture,
    hdr_view: wgpu::TextureView,
    scratch: wgpu::Buffer,
    indirect: wgpu::Buffer,
}

/// Captures run only on frames where the renderer enables reflections.
struct CaptureSchedule {
    boxes: Vec<ProbeBox>,
    captured: [bool; MAX_PROBES],
    cursor: usize,
    interval: u32,
    elapsed: u32,
}

impl Default for CaptureSchedule {
    fn default() -> Self {
        Self { boxes: Vec::new(), captured: [false; MAX_PROBES], cursor: 0, interval: 1, elapsed: 0 }
    }
}

impl CaptureSchedule {
    fn set(&mut self, boxes: &[ProbeBox]) {
        if self.boxes == boxes { return; }
        for (i, b) in boxes.iter().enumerate() {
            if self.boxes.get(i) != Some(b) { self.captured[i] = false; }
        }
        self.boxes.clear();
        self.boxes.extend_from_slice(boxes);
    }

    fn next(&mut self) -> Option<Job> {
        let count = self.boxes.len();
        if count == 0 { return None; }
        self.elapsed = self.elapsed.saturating_add(1);
        let unfilled = (0..count).map(|i| (self.cursor + i) % count).find(|&i| !self.captured[i]);
        let probe = match unfilled {
            Some(i) => i,
            None if self.elapsed >= self.interval => self.cursor % count,
            None => return None,
        };
        self.cursor = probe + 1;
        self.elapsed = 0;
        Some(Job { probe, bounds: self.boxes[probe] })
    }

    fn finish(&mut self, job: &Job) {
        // The game can move or remove the box before its capture completes.
        self.captured[job.probe] = self.boxes.get(job.probe) == Some(&job.bounds);
    }
}

pub(crate) struct Probes {
    schedule: std::sync::Mutex<CaptureSchedule>,
    raw: wgpu::Texture,
    filtered: wgpu::Texture,
    /// All of `filtered`, for shade (binding 27).
    pub view: wgpu::TextureView,
    raw_view: wgpu::TextureView,
    layout: wgpu::BindGroupLayout,
    down: wgpu::ComputePipeline,
    prefilter: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
    /// Per face: which face and layer the mips and prefilter write.
    params: [wgpu::Buffer; 6],
    capture: std::sync::OnceLock<Capture>,
}

/// The cubemap to draw this frame.
pub(crate) struct Job { pub probe: usize, pub bounds: ProbeBox }

impl Probes {
    pub fn new(gpu: &Gpu) -> Self {
        let cube = |label: &str| gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some(label),
            size: wgpu::Extent3d { width: FACE, height: FACE, depth_or_array_layers: 6 * MAX_PROBES as u32 }, mip_level_count: LEVELS, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        let raw = cube("reflection probes (raw)");
        let filtered = cube("reflection probes");
        let cube_view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::CubeArray), ..Default::default() });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("reflection probes"), entries: &[
            entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
            entry(1, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            entry(2, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba16Float, view_dimension: wgpu::TextureViewDimension::D2 }),
            entry(3, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::CubeArray, multisampled: false }),
            entry(4, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
        ] });
        let pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("reflection probes"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("reflection probes"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pipe = |entry: &str| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&pl), module: &module,
            entry_point: Some(entry), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("reflection probes"),
            mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, ..Default::default() });
        let params = std::array::from_fn(|_| gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("reflection probe face"), size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        Self { schedule: Default::default(), view: cube_view(&filtered), raw_view: cube_view(&raw), raw, filtered,
            down: pipe("down"), prefilter: pipe("prefilter"), layout, sampler, params, capture: std::sync::OnceLock::new() }
    }

    pub fn set(&self, boxes: &[ProbeBox]) -> Result<(), String> {
        if boxes.len() > MAX_PROBES { return Err(format!("at most {MAX_PROBES} reflection probes")); }
        if boxes.iter().any(|b| !(b.capture.is_finite() && b.min.is_finite() && b.max.is_finite()) || b.min.cmpge(b.max).any()) {
            return Err("a reflection probe box needs finite bounds, min below max".into());
        }
        self.schedule.lock().unwrap().set(boxes);
        Ok(())
    }

    pub fn set_interval(&self, frames: u32) {
        self.schedule.lock().unwrap().interval = frames.max(1);
    }

    /// Capture newly placed probes first, then refresh one cubemap per interval.
    pub fn next(&self) -> Option<Job> { self.schedule.lock().unwrap().next() }

    /// Shade's view of the boxes: per probe capture (w 1 once all six faces
    /// are in), min, max; and the count.
    pub fn uniform(&self) -> ([[f32; 4]; 3 * MAX_PROBES], u32) {
        let state = self.schedule.lock().unwrap();
        let mut out = [[0.0; 4]; 3 * MAX_PROBES];
        for (i, b) in state.boxes.iter().enumerate() {
            let done = state.captured[i];
            out[3 * i] = b.capture.extend(if done { 1.0 } else { 0.0 }).to_array();
            out[3 * i + 1] = b.min.extend(0.0).to_array();
            out[3 * i + 2] = b.max.extend(0.0).to_array();
        }
        (out, state.boxes.len() as u32)
    }

    /// The capture's targets: visibility, depth, radiance (shade's
    /// `out_hdr`) and shade's scratch.
    pub fn targets(&self, gpu: &Gpu) -> (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView, &wgpu::Buffer, &wgpu::Buffer) {
        let c = self.capture.get_or_init(|| {
            let tex = |label: &str, format, usage| gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some(label),
                size: wgpu::Extent3d { width: FACE, height: FACE, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
            let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let hdr = tex("reflection probe radiance", wgpu::TextureFormat::Rgba16Float, wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
            let scratch = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("reflection probe shade scratch"),
                size: crate::shade_scratch::bytes((FACE, FACE), false), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let indirect = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("reflection probe shade tiles"), size: 16,
                usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            gpu.queue.write_buffer(&scratch, crate::shade_scratch::TILES, bytemuck::cast_slice(&crate::shade_scratch::tile_header((FACE, FACE), false)));
            Capture { vis: tex("reflection probe visibility", wgpu::TextureFormat::R32Uint, rt).create_view(&Default::default()),
                depth: tex("reflection probe depth", wgpu::TextureFormat::Depth32Float, rt).create_view(&Default::default()),
                hdr_view: hdr.create_view(&Default::default()), hdr, scratch, indirect }
        });
        (&c.vis, &c.depth, &c.hdr_view, &c.scratch, &c.indirect)
    }

    /// After shade drew a face of `probe` into the radiance target: copy it
    /// into the raw cubes.
    pub fn copy_face(&self, enc: &mut wgpu::CommandEncoder, probe: usize, face: usize) {
        let c = self.capture.get().expect("probe targets");
        enc.copy_texture_to_texture(c.hdr.as_image_copy(), wgpu::TexelCopyTextureInfo { texture: &self.raw, mip_level: 0,
            origin: wgpu::Origin3d { x: 0, y: 0, z: (probe * 6 + face) as u32 }, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: FACE, height: FACE, depth_or_array_layers: 1 });
    }

    /// After all six faces of `probe` are in: their mips, then their
    /// prefiltered levels (each face's lobe reads its neighbours, captured
    /// in the same frame).
    pub fn finish(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, job: &Job, timestamps: Option<(&wgpu::QuerySet, u32)>) {
        let probe = job.probe;
        let c = self.capture.get().expect("probe targets");
        for face in 0..6 {
            gpu.queue.write_buffer(&self.params[face], 0, bytemuck::cast_slice(&[face as u32, (probe * 6 + face) as u32, LEVELS, SAMPLES]));
        }
        let level = |t: &wgpu::Texture, mip: u32, layer: u32| t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2),
            base_mip_level: mip, mip_level_count: Some(1), base_array_layer: layer, array_layer_count: Some(1), ..Default::default() });
        let bind = |face: usize, src: &wgpu::TextureView, dst: &wgpu::TextureView, raw: &wgpu::TextureView| gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reflection probe"), layout: &self.layout, entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params[face].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(dst) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(raw) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ] });
        // Bindings a pass does not read: any view of another texture.
        let unused = c.hdr_view.clone();
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("reflection probe mips"),
                timestamp_writes: timestamps.map(|(query_set, i)| wgpu::ComputePassTimestampWrites { query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: None }) });
            pass.set_pipeline(&self.down);
            for face in 0..6 {
                let layer = (probe * 6 + face) as u32;
                for mip in 1..LEVELS {
                    let g = bind(face, &level(&self.raw, mip - 1, layer), &level(&self.raw, mip, layer), &self.view);
                    pass.set_bind_group(0, &g, &[]);
                    let s = (FACE >> mip).max(1);
                    pass.dispatch_workgroups(s.div_ceil(8), s.div_ceil(8), 1);
                }
            }
        }
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("reflection probe prefilter"),
                timestamp_writes: timestamps.map(|(query_set, i)| wgpu::ComputePassTimestampWrites { query_set, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(i + 1) }) });
            pass.set_pipeline(&self.prefilter);
            for face in 0..6 {
                let layer = (probe * 6 + face) as u32;
                for mip in 0..LEVELS {
                    let g = bind(face, &unused, &level(&self.filtered, mip, layer), &self.raw_view);
                    pass.set_bind_group(0, &g, &[]);
                    let s = (FACE >> mip).max(1);
                    pass.dispatch_workgroups(s.div_ceil(8), s.div_ceil(8), 1);
                }
            }
        }
        self.schedule.lock().unwrap().finish(job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_boxes_preempt_periodic_refreshes_and_keep_round_robin_order() {
        let room = |x| ProbeBox { min: Vec3::X * x, max: Vec3::X * x + Vec3::ONE, capture: Vec3::X * x + Vec3::splat(0.5) };
        let tick = |s: &mut CaptureSchedule| {
            let job = s.next()?;
            s.finish(&job);
            Some(job.probe)
        };
        let mut s = CaptureSchedule::default();
        assert!(s.next().is_none());
        s.set(&[room(0.0), room(20.0)]);
        assert_eq!([tick(&mut s), tick(&mut s), tick(&mut s)], [Some(0), Some(1), Some(0)]);
        s.interval = 3;
        assert_eq!([tick(&mut s), tick(&mut s), tick(&mut s)], [None, None, Some(1)]);
        s.set(&[room(0.0), room(20.0), room(40.0)]);
        assert_eq!(tick(&mut s), Some(2));
        s.set(&[room(0.0), room(21.0), room(40.0)]);
        let stale = s.next().unwrap();
        assert_eq!(stale.probe, 1);
        s.set(&[room(0.0), room(22.0), room(40.0)]);
        s.finish(&stale);
        assert_eq!(tick(&mut s), Some(1));
        assert_eq!([tick(&mut s), tick(&mut s), tick(&mut s)], [None, None, Some(2)]);
        s.set(&[]);
        assert!(s.next().is_none());
    }

    #[test]
    fn faces_match_the_cube_layout() {
        // A face's centre texel looks along its axis; its right edge (x ndc
        // +1) and top edge (y ndc +1) go where the cube map's s and -t go.
        let c = Vec3::ZERO;
        let rights = [Vec3::NEG_Z, Vec3::Z, Vec3::X, Vec3::X, Vec3::X, Vec3::NEG_X];
        let ups = [Vec3::Y, Vec3::Y, Vec3::NEG_Z, Vec3::Z, Vec3::Y, Vec3::Y];
        for f in 0..6 {
            let vp = face_view_proj(c, f);
            let ndc = |p: Vec3| { let h = vp * p.extend(1.0); h.truncate() / h.w };
            let (dir, _) = FACES[f];
            let centre = ndc(dir);
            assert!(centre.x.abs() < 1e-5 && centre.y.abs() < 1e-5, "face {f} centre {centre}");
            let r = ndc(dir + rights[f] * 0.5);
            assert!(r.x > 0.4 && r.y.abs() < 1e-4, "face {f} right {r}");
            let u = ndc(dir + ups[f] * 0.5);
            assert!(u.y > 0.4 && u.x.abs() < 1e-4, "face {f} up {u}");
        }
    }
}
