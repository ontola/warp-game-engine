//! Reflective shadow maps (Dachsbacher and Stamminger 2005): the live first
//! diffuse bounce of clustered lights flagged `ClusteredLight::bounce` (a
//! flashlight, a swinging pendant: lights whose bounce cannot be baked).
//!
//! Their shadow tiles also store each texel's albedo and normal
//! (`clustered_lights::Resources::rsm`, raster.wgsl `fs_rsm`). Before shade,
//! `rsm_gather` (rsm.wgsl) gathers at a quarter of the view's resolution:
//! per pixel and light, [`TEXELS`] texels each frame, mixed between the
//! whole tile and a disc round the pixel's own projection. A texel's flux
//! follows from the light model shade uses directly (albedo, irradiance at
//! the patch, the texel's solid angle), so the bounce has no gain of its
//! own. Patch and pixel must see each other through [`Voxels`], a bit grid
//! of the scene's surfaces (`Scene::reserve_bounce_voxels`,
//! `Renderer::set_bounce_voxels`); without one the bounce reaches through
//! walls. The gather blends with last frame's (reprojected, rejected where
//! the depth changed), and shade adds it to the diffuse ambient with a
//! depth-aware upsample. See engine/docs/interior-light.md.
use glam::Vec3;

/// Texels per light, pixel and frame.
pub const TEXELS: u32 = 16;
/// The gather's resolution divisor for a view `height` pixels tall (rsm.wgsl
/// `rsm_scale`): a quarter, an eighth from 1600 lines, so the gather stays
/// near 480x270 to 640x360 (the bounce has no fine detail).
pub fn scale(height: u32) -> u32 { if height > 1600 { 8 } else { 4 } }
/// Header words ahead of the voxel bits (`Voxels::pack`).
pub const HEADER_WORDS: usize = 8;
/// Voxels per brick edge (`Voxels::pack`).
pub const BRICK: u32 = 4;

/// One bit per voxel: whether a surface passes through it.
#[derive(Clone, Debug, PartialEq)]
pub struct Voxels {
    pub origin: Vec3,
    pub voxel: f32,
    pub dims: [u32; 3],
    pub bits: Vec<u32>,
}

impl Voxels {
    /// An empty grid over `min..max`.
    pub fn new(min: Vec3, max: Vec3, voxel: f32) -> Self {
        let dims = ((max - min) / voxel).ceil().max(Vec3::ONE).as_uvec3().to_array();
        let count = dims[0] as usize * dims[1] as usize * dims[2] as usize;
        Self { origin: min, voxel, dims, bits: vec![0; count.div_ceil(32)] }
    }

    pub fn count(&self) -> usize { self.dims[0] as usize * self.dims[1] as usize * self.dims[2] as usize }

    /// Words in `aux` for a grid over `min..max` (what
    /// `Scene::reserve_bounce_voxels` reserves).
    pub fn words_for(min: Vec3, max: Vec3, voxel: f32) -> usize { Self::new(min, max, voxel).pack().len() }

    pub fn set(&mut self, x: u32, y: u32, z: u32) {
        let i = x as usize + self.dims[0] as usize * (y as usize + self.dims[1] as usize * z as usize);
        self.bits[i >> 5] |= 1 << (i & 31);
    }

    pub fn get(&self, x: u32, y: u32, z: u32) -> bool {
        let i = x as usize + self.dims[0] as usize * (y as usize + self.dims[1] as usize * z as usize);
        self.bits[i >> 5] & (1 << (i & 31)) != 0
    }

    /// Mark every voxel a triangle touches (conservative: a closed surface
    /// leaves no gap a voxel walk could pass).
    pub fn add_triangle(&mut self, t: [Vec3; 3]) {
        let mut hit = Vec::new();
        self.touched(t, |i| hit.push(i));
        for i in hit { self.bits[i >> 5] |= 1 << (i & 31); }
    }

    /// Every voxel (linear index) the triangle touches.
    pub fn touched(&self, t: [Vec3; 3], mut f: impl FnMut(usize)) {
        let lo = (t[0].min(t[1]).min(t[2]) - self.origin) / self.voxel;
        let hi = (t[0].max(t[1]).max(t[2]) - self.origin) / self.voxel;
        let d = glam::IVec3::from_array(self.dims.map(|d| d as i32));
        let a = (lo - 1e-3).floor().as_ivec3().max(glam::IVec3::ZERO);
        let b = (hi + 1e-3).floor().as_ivec3().min(d - 1);
        // A hair over half a voxel: a surface exactly on a voxel boundary (a
        // wall face on the grid) must not round out of both voxels beside it.
        let half = Vec3::splat(self.voxel * 0.5 * (1.0 + 1e-4));
        for z in a.z..=b.z { for y in a.y..=b.y { for x in a.x..=b.x {
            let centre = self.origin + (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * self.voxel;
            if triangle_box(centre, half, t) { f(x as usize + self.dims[0] as usize * (y as usize + self.dims[1] as usize * z as usize)); }
        } } }
    }

    /// Bricks of [`BRICK`]^3 voxels per axis.
    pub fn bricks(&self) -> [u32; 3] { self.dims.map(|d| d.div_ceil(BRICK)) }

    /// The header (origin, voxel size, dims, and in word 7 where the brick
    /// bits start), the voxel bits, then one bit per brick (any voxel in it
    /// solid), which the walk in rsm.wgsl crosses in one step where empty.
    pub fn pack(&self) -> Vec<u32> {
        let mut w = vec![self.origin.x.to_bits(), self.origin.y.to_bits(), self.origin.z.to_bits(), self.voxel.to_bits(),
            self.dims[0], self.dims[1], self.dims[2], (HEADER_WORDS + self.bits.len()) as u32];
        w.extend(&self.bits);
        let b = self.bricks().map(|v| v as usize);
        let mut bricks = vec![0u32; (b[0] * b[1] * b[2]).div_ceil(32)];
        let (dx, dy) = (self.dims[0] as usize, self.dims[1] as usize);
        let edge = BRICK as usize;
        for (k, &word) in self.bits.iter().enumerate() {
            let mut m = word;
            while m != 0 {
                let i = k * 32 + m.trailing_zeros() as usize;
                m &= m - 1;
                let (x, y, z) = (i % dx, (i / dx) % dy, i / (dx * dy));
                let j = x / edge + b[0] * (y / edge + b[1] * (z / edge));
                bricks[j >> 5] |= 1 << (j & 31);
            }
        }
        w.extend(bricks);
        w
    }
}

/// Where a scene's surface voxels go (`Scene::reserve_bounce_voxels`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelBounds { pub min: Vec3, pub max: Vec3, pub voxel: f32 }

/// The scene's surfaces as voxels, kept current as instances move: per
/// voxel the number of triangles touching it, so an instance that moved or
/// changed its mesh (a door swinging, a pendant swaying) takes its old
/// triangles out and puts its new ones in without a rebuild.
pub struct SceneVoxels {
    pub grid: Voxels,
    counts: Vec<u16>,
    seen: Vec<crate::scene::GpuInstance>,
}

fn same(a: &crate::scene::GpuInstance, b: &crate::scene::GpuInstance) -> bool {
    a.rows == b.rows && a.first_cluster == b.first_cluster && a.cluster_count == b.cluster_count && a.flags == b.flags
}

impl SceneVoxels {
    pub fn new(bounds: VoxelBounds) -> Self {
        let grid = Voxels::new(bounds.min, bounds.max, bounds.voxel);
        let counts = vec![0; grid.count()];
        Self { grid, counts, seen: Vec::new() }
    }

    fn apply(&mut self, scene: &crate::scene::Scene, inst: &crate::scene::GpuInstance, add: bool, tris: &mut Vec<[Vec3; 3]>) -> bool {
        tris.clear();
        crate::room_bake::instance_triangles(scene, inst, tris);
        let mut changed = false;
        let mut hit = Vec::new();
        for t in tris.iter() { self.grid.touched(*t, |i| hit.push(i)); }
        for i in hit {
            let c = &mut self.counts[i];
            let before = *c != 0;
            *c = if add { c.saturating_add(1) } else { c.saturating_sub(1) };
            if before != (*c != 0) {
                self.grid.bits[i >> 5] ^= 1 << (i & 31);
                changed = true;
            }
        }
        changed
    }

    /// Bring the voxels up to `scene`'s instances; whether any voxel changed.
    pub fn update(&mut self, scene: &crate::scene::Scene) -> bool {
        let mut changed = false;
        let mut tris = Vec::new();
        for (i, inst) in scene.instances.iter().enumerate() {
            if let Some(old) = self.seen.get(i).copied() {
                if same(&old, inst) { continue; }
                changed |= self.apply(scene, &old, false, &mut tris);
                self.seen[i] = *inst;
            } else { self.seen.push(*inst); }
            changed |= self.apply(scene, inst, true, &mut tris);
        }
        while self.seen.len() > scene.instances.len() {
            let old = self.seen.pop().unwrap();
            changed |= self.apply(scene, &old, false, &mut tris);
        }
        changed
    }
}

/// Whether a triangle overlaps an axis-aligned box (Akenine-Moeller 2001,
/// the separating axis test: the box's axes, the triangle's normal and the
/// nine edge cross products).
pub fn triangle_box(centre: Vec3, half: Vec3, t: [Vec3; 3]) -> bool {
    let v = t.map(|p| p - centre);
    let e = [v[1] - v[0], v[2] - v[1], v[0] - v[2]];
    let separated = |axis: Vec3| {
        if axis.length_squared() < 1e-20 { return false; }
        let p = v.map(|q| q.dot(axis));
        let r = half.x * axis.x.abs() + half.y * axis.y.abs() + half.z * axis.z.abs();
        p.iter().copied().fold(f32::INFINITY, f32::min) > r || p.iter().copied().fold(f32::NEG_INFINITY, f32::max) < -r
    };
    for edge in e { for axis in [Vec3::X, Vec3::Y, Vec3::Z] { if separated(axis.cross(edge)) { return false; } } }
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] { if separated(axis) { return false; } }
    !separated(e[0].cross(e[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_box_overlap() {
        let t = [Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, -1.0), Vec3::new(0.0, 0.0, 1.0)];
        assert!(triangle_box(Vec3::ZERO, Vec3::splat(0.1), t));
        assert!(triangle_box(Vec3::new(0.0, 0.09, 0.0), Vec3::splat(0.1), t));
        assert!(!triangle_box(Vec3::new(0.0, 0.11, 0.0), Vec3::splat(0.1), t));
        // Beside the triangle's slanted edge, in its plane.
        assert!(!triangle_box(Vec3::new(0.9, 0.0, 0.9), Vec3::splat(0.1), t));
    }

    #[test]
    fn a_wall_is_closed_to_a_voxel_walk() {
        // A 0.3 m wall at x 0..0.3, voxelised at 0.1 m: every column across it
        // holds a solid voxel.
        let mut v = Voxels::new(Vec3::splat(-1.0), Vec3::splat(1.0), 0.1);
        let (a, b) = (Vec3::new(0.0, -1.0, -1.0), Vec3::new(0.0, 1.0, 1.0));
        for x in [0.0f32, 0.3] {
            let q = [Vec3::new(x, a.y, a.z), Vec3::new(x, b.y, a.z), Vec3::new(x, b.y, b.z), Vec3::new(x, a.y, b.z)];
            v.add_triangle([q[0], q[1], q[2]]);
            v.add_triangle([q[0], q[2], q[3]]);
        }
        for z in 0..20 { for y in 0..20 { assert!((0..20).any(|x| v.get(x, y, z))); } }
        assert!(!v.get(5, 10, 10));
        let packed = v.pack();
        assert_eq!(packed.len(), HEADER_WORDS + 8000usize.div_ceil(32) + 125usize.div_ceil(32));
    }
}

/// Per view: two gather targets at 1/[`scale`] resolution, written and
/// read in turn, and the view-projection of the frame each holds.
pub(crate) struct Targets {
    views: Vec<Option<Target>>,
    /// Bound where shade (or the gather) has no target of that kind.
    pub fallback_storage: wgpu::TextureView,
    pub fallback: wgpu::TextureView,
    frame: u32,
}

struct Target {
    size: (u32, u32),
    views: [wgpu::TextureView; 2],
    filter: [wgpu::TextureView; 2],
    cur: usize,
    prev_view_proj: Option<glam::Mat4>,
}

/// This frame's gather for one view.
pub(crate) struct Frame {
    /// Written by the gather, then read by shade.
    pub out: wgpu::TextureView,
    /// Last frame's gather.
    pub prev: wgpu::TextureView,
    /// The spatial filter's passes (rsm_filter): out, a, b, a; shade reads a.
    pub filter: [wgpu::TextureView; 2],
    /// Last frame's view-projection, when `prev` holds a gather.
    pub prev_view_proj: Option<glam::Mat4>,
    pub frame: u32,
}

fn texture(gpu: &crate::Gpu, label: &str, (w, h): (u32, u32), usage: wgpu::TextureUsages) -> wgpu::TextureView {
    gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Float, usage, view_formats: &[] })
        .create_view(&Default::default())
}

impl Targets {
    pub fn new(gpu: &crate::Gpu) -> Self {
        Self { views: Vec::new(), frame: 0,
            fallback_storage: texture(gpu, "no rsm gather out", (1, 1), wgpu::TextureUsages::STORAGE_BINDING),
            fallback: texture(gpu, "no rsm gather", (1, 1), wgpu::TextureUsages::TEXTURE_BINDING) }
    }

    /// Swap view `index`'s targets for a frame seen through `view_proj`
    /// (`size`: the view's full resolution).
    pub fn begin(&mut self, gpu: &crate::Gpu, index: usize, size: (u32, u32), view_proj: glam::Mat4) -> Frame {
        let small = (size.0.div_ceil(scale(size.1)).max(1), size.1.div_ceil(scale(size.1)).max(1));
        if self.views.len() <= index { self.views.resize_with(index + 1, || None); }
        self.frame = self.frame.wrapping_add(1);
        let slot = &mut self.views[index];
        if slot.as_ref().is_none_or(|t| t.size != small) {
            let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
            *slot = Some(Target { size: small, views: [texture(gpu, "rsm gather a", small, usage), texture(gpu, "rsm gather b", small, usage)],
                filter: [texture(gpu, "rsm filter a", small, usage), texture(gpu, "rsm filter b", small, usage)], cur: 0, prev_view_proj: None });
        }
        let t = slot.as_mut().unwrap();
        t.cur = 1 - t.cur;
        let prev_view_proj = t.prev_view_proj.replace(view_proj);
        Frame { out: t.views[t.cur].clone(), prev: t.views[1 - t.cur].clone(), filter: t.filter.clone(), prev_view_proj, frame: self.frame }
    }

    /// Forget every view's history (a cut, or the bounce turned off).
    pub fn reset(&mut self) { for t in self.views.iter_mut().flatten() { t.prev_view_proj = None; } }
}

/// The gather and the filter's three passes (rsm.wgsl), compiled with shade.
pub(crate) struct Pipelines {
    pub gather: wgpu::ComputePipeline,
    pub filters: [wgpu::ComputePipeline; 3],
}
