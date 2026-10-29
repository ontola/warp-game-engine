//! Exact static/dynamic partition of the shared source-geometry sun shadow.
//!
//! Static depth is rendered with the SAME light projection, source LOD,
//! alpha test and raster bias as the ordinary sun pass. A texture copy seeds
//! the final depth target; dynamic casters then render with Load and Less.
//! This is min(static, dynamic), not a blurred or rest-pose shadow proxy.
//! No camera visibility list may be used to construct either caster list.
//!
//! One additional Depth32Float map costs 64 MiB at 4096 square. There is no
//! second dynamic map or fullscreen merge. The caller times the GPU copy and
//! both caster passes separately, and commits the cache only after submit.

use crate::{Gpu, scene::{Scene, GpuInstance, GeometryCategory, FOLIAGE, NO_SHADOW,
    LIVE_GRASS, WIND_GRASS, PART_WIND}};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

pub const SKIPPED: u32 = 0;
pub const STATIC: u32 = 1;
pub const DYNAMIC: u32 = 2;

/// Optional shared-map guard, independent of the split-cache switch. Invalid
/// settings retain the existing projection. Near shadows never use this.
pub fn configured_guard(value: Option<&str>) -> f32 {
    value.and_then(|v|v.parse::<f32>().ok()).filter(|v|v.is_finite() && (0.0..=128.0).contains(v)).unwrap_or(0.0)
}

pub const FULL_COVERAGE_NDC: f32 = 0.75;

/// Stateful light-space anchor only; scene caster Z bounds remain the caller's
/// responsibility and may expand independently without losing offscreen depth.
#[derive(Default)]
pub struct GuardedAnchor { previous: Option<(Vec3, [u32;3], u32)> }

impl GuardedAnchor {
    /// Keep each receiver's spherical support inside the fully weighted XY
    /// portion of the map. This includes a texel and floating-point allowance.
    /// A sun/extent change or teleport recentres immediately, never gradually.
    pub fn update(&mut self, receivers: &[Vec3], sun: Vec3, extent: f32,
        support_m: f32, resolution: u32) -> Result<(Vec3,bool),String> {
        if receivers.is_empty() || receivers.iter().any(|v|!v.is_finite()) || !sun.is_finite()
            || sun.length_squared()<1e-12 || !extent.is_finite() || extent<=0.0
            || !support_m.is_finite() || support_m<0.0 || resolution==0 {
            return Err("invalid guarded sun receiver/projection state".into());
        }
        let z=sun.normalize(); let up=if z.y.abs()>0.99{Vec3::Z}else{Vec3::Y};
        let x=up.cross(z).normalize(); let y=z.cross(x);
        let sun_key=sun.to_array().map(f32::to_bits);
        let texel=2.0*extent/resolution as f32;
        let covers=|anchor:Vec3| {
            let cx=(x.dot(anchor)/texel).round()*texel;
            let cy=(y.dot(anchor)/texel).round()*texel;
            let roundoff=16.0*f32::EPSILON*(cx.abs()+cy.abs()+extent+1.0);
            receivers.iter().all(|p| (x.dot(*p)-cx).abs().max((y.dot(*p)-cy).abs())
                +support_m+texel+roundoff <= FULL_COVERAGE_NDC*extent)
        };
        if let Some((anchor,previous_sun,previous_extent))=self.previous {
            if sun_key==previous_sun && extent.to_bits()==previous_extent && covers(anchor) {
                return Ok((anchor,false));
            }
        }
        // Average in f64 so large coordinates do not overflow during summation.
        let centre=receivers.iter().fold(glam::DVec3::ZERO,|sum,p|sum+p.as_dvec3())/receivers.len() as f64;
        let anchor=centre.as_vec3();
        if !anchor.is_finite() || !covers(anchor) {
            return Err("guarded sun extent does not conservatively cover every receiver".into());
        }
        self.previous=Some((anchor,sun_key,extent.to_bits()));
        Ok((anchor,true))
    }
}

/// Counts describe source owners, not selected clusters or raster triangles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CasterCounts {
    pub static_casters: u32,
    pub dynamic_casters: u32,
    pub skipped: u32,
}

fn class(instance: &GpuInstance, animated: bool, frozen: bool) -> u32 {
    // Procedural terrain slots fill at runtime; an empty one is still dynamic.
    if instance.flags & crate::procedural_terrain::PROCEDURAL != 0 && instance.flags & NO_SHADOW == 0 { return DYNAMIC; }
    if instance.cluster_count == 0 || instance.flags & NO_SHADOW != 0 { return SKIPPED; }
    // A tree past the wind freeze radius stands exactly at rest (wind.wgsl),
    // so its depth is as static as a rock until the cameras are re-pinned.
    if frozen && !animated && instance.flags & FOLIAGE != 0
        && instance.flags & (LIVE_GRASS | WIND_GRASS | PART_WIND | crate::scene::SKINNED) == 0
        && instance.category() == GeometryCategory::Trees {
        return STATIC;
    }
    // Include provenance as a conservative exclusion even if an asset omitted
    // its wind flag. SOLID alone is not dynamic: buildings/rocks use it too.
    if animated || instance.flags & (FOLIAGE | LIVE_GRASS | WIND_GRASS | PART_WIND | crate::scene::SKINNED | crate::procedural_terrain::PROCEDURAL) != 0
        || matches!(instance.category(), GeometryCategory::Trees | GeometryCategory::Grass | GeometryCategory::Cars) {
        DYNAMIC
    } else { STATIC }
}

/// CPU source ownership plus a fixed-layout GPU list. Original owner IDs never
/// change, so the same live instance/wind buffers serve both shadow passes.
struct Partition {
    sources: Vec<GpuInstance>,
    animated: Vec<bool>,
    /// Trees beyond the freeze radius of every pinned camera.
    frozen: Vec<bool>,
    /// Header [static count, dynamic count, static offset, dynamic offset],
    /// N class words, N static-owner slots, N dynamic-owner slots. Offsets are
    /// u32 WORD offsets. The unused tail of each list is zero and never read.
    words: Vec<u32>,
    counts: CasterCounts,
}

impl Partition {
    fn new(scene: &Scene) -> Result<Self, String> {
        let n = scene.instances.len();
        let word_count = n.checked_mul(3).and_then(|v| v.checked_add(4))
            .filter(|&v| v <= u32::MAX as usize).ok_or("sun caster list exceeds u32 addressing")?;
        let mut animated = vec![false; n];
        let mut mark = |owner: u32| -> Result<(), String> {
            let slot = animated.get_mut(owner as usize).ok_or("animated sun caster owner is outside scene")?;
            *slot = true;
            Ok(())
        };
        for part in &scene.part_wind { if let Some(owner) = part.owner { mark(owner)?; } }
        for attachment in &scene.part_attachments { mark(attachment.owner)?; }
        if let Some(animation) = &scene.baked_animations {
            for part in &animation.parts { mark(part.owner)?; }
        }
        let mut result = Self { sources: scene.instances.clone(), frozen: vec![false; n], animated,
            words: vec![0; word_count], counts: CasterCounts::default() };
        result.rebuild();
        Ok(result)
    }

    fn rebuild(&mut self) {
        self.words.fill(0);
        self.counts = CasterCounts::default();
        let n = self.sources.len();
        let static_start = 4 + n;
        let dynamic_start = static_start + n;
        self.words[2] = static_start as u32;
        self.words[3] = dynamic_start as u32;
        for (owner, source) in self.sources.iter().enumerate() {
            let kind = class(source, self.animated[owner], self.frozen[owner]);
            self.words[4 + owner] = kind;
            match kind {
                STATIC => {
                    self.words[static_start + self.counts.static_casters as usize] = owner as u32;
                    self.counts.static_casters += 1;
                }
                DYNAMIC => {
                    self.words[dynamic_start + self.counts.dynamic_casters as usize] = owner as u32;
                    self.counts.dynamic_casters += 1;
                }
                _ => self.counts.skipped += 1,
            }
        }
        self.words[0] = self.counts.static_casters;
        self.words[1] = self.counts.dynamic_casters;
    }

    /// Mark trees whose root is at least `radius` (horizontal) from every
    /// camera. Returns whether any tree changed class.
    fn pin_frozen(&mut self, cameras: &[Vec3], radius: f32) -> bool {
        let mut changed = false;
        for (owner, source) in self.sources.iter().enumerate() {
            let root = glam::Vec2::new(source.rows[0][3], source.rows[2][3]);
            let frozen = !cameras.is_empty() && cameras.iter()
                .all(|c| root.distance(glam::Vec2::new(c.x, c.z)) >= radius);
            if frozen != self.frozen[owner] {
                let before = class(source, self.animated[owner], self.frozen[owner]);
                self.frozen[owner] = frozen;
                changed |= before != class(source, self.animated[owner], frozen);
            }
        }
        if changed { self.rebuild(); }
        changed
    }

    /// Validate the complete range before mutating. An instance may cease to
    /// be static (or start casting again), which also invalidates old depth.
    fn update(&mut self, first: usize, instances: &[GpuInstance]) -> Result<(bool, bool), String> {
        let end = first.checked_add(instances.len()).filter(|&v| v <= self.sources.len())
            .ok_or("sun caster update exceeds uploaded scene")?;
        let mut dirty = false;
        let mut membership = false;
        for (offset, (&before, &after)) in self.sources[first..end].iter().zip(instances).enumerate() {
            if bytemuck::bytes_of(&before) == bytemuck::bytes_of(&after) { continue; }
            let owner = first + offset;
            let old = class(&before, self.animated[owner], self.frozen[owner]);
            // A changed transform invalidates the proof that this root stays
            // outside every pinned camera's wind radius. Keep it dynamic until
            // the next explicit pin certifies its new position.
            if before.rows != after.rows { self.frozen[owner] = false; }
            let new = class(&after, self.animated[owner], self.frozen[owner]);
            dirty |= old == STATIC || new == STATIC;
            membership |= old != new;
        }
        self.sources[first..end].copy_from_slice(instances);
        if membership { self.rebuild(); }
        Ok((dirty, membership))
    }
}

/// Exact raster key. No epsilon comparison: a reused depth image must have
/// exactly the same projection. `raster_revision` covers any additional caster
/// policy, shader, material/texture, bias or geometry change outside this API.
/// Sun strength and sky radiance do not change depth and are deliberately absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SunRasterKey {
    projection: [u32; 16],
    alpha_cut: u32,
    lod_pixels: u32,
    raster_revision: u64,
}

impl SunRasterKey {
    pub fn new(view_proj: Mat4, alpha_cut: f32, lod_pixels: f32, raster_revision: u64) -> Result<Self, String> {
        if !view_proj.is_finite() || !alpha_cut.is_finite() || !(0.0..=1.0).contains(&alpha_cut)
            || !lod_pixels.is_finite() || lod_pixels < 0.0 {
            return Err("static sun key requires a finite projection, alpha 0..1 and nonnegative LOD".into());
        }
        Ok(Self { projection: view_proj.to_cols_array().map(f32::to_bits),
            alpha_cut: alpha_cut.to_bits(), lod_pixels: lod_pixels.to_bits(), raster_revision })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StaticSunPlan {
    pub redraw_static: bool,
    key: SunRasterKey,
    revision: u64,
}

#[derive(Default)]
struct Content {
    revision: u64,
    submitted: Option<SunRasterKey>,
}

impl Content {
    fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.submitted = None;
    }
    fn prepare(&self, key: SunRasterKey) -> StaticSunPlan {
        StaticSunPlan { redraw_static: self.submitted != Some(key), key, revision: self.revision }
    }
    fn submitted(&mut self, plan: StaticSunPlan) -> bool {
        if plan.revision != self.revision { return false; }
        self.submitted = Some(plan.key);
        true
    }
}

pub struct StaticSun {
    partition: Partition,
    owners: wgpu::Buffer,
    depth: wgpu::Texture,
    content: Content,
}

impl StaticSun {
    /// Construct only when the experimental split-cache path is enabled.
    /// Allocation is retained across invalidations; no depth is valid initially.
    pub fn new(gpu: &Gpu, scene: &Scene, size: u32) -> Result<Self, String> {
        if size == 0 || size > gpu.limits.max_texture_dimension_2d {
            return Err("static sun size exceeds the device's 2D texture limit".into());
        }
        let partition = Partition::new(scene)?;
        let bytes = partition.words.len() as u64 * 4;
        if bytes > u64::from(gpu.limits.max_storage_buffer_binding_size) || bytes > gpu.limits.max_buffer_size {
            return Err("static sun owner list exceeds device buffer limits".into());
        }
        // The shared culler flattens a 2D dispatch as x + y * 65535.
        if gpu.limits.max_compute_workgroups_per_dimension < 65535
            || partition.sources.len().div_ceil(65535) > gpu.limits.max_compute_workgroups_per_dimension as usize {
            return Err("static sun owner dispatch exceeds device workgroup limits".into());
        }
        let owners = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("static/dynamic sun owner lists"), contents: bytemuck::cast_slice(&partition.words),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cached static sun depth"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Ok(Self { partition, owners, depth, content: Content::default() })
    }

    pub fn owner_buffer(&self) -> &wgpu::Buffer { &self.owners }
    pub fn counts(&self) -> CasterCounts { self.partition.counts }
    pub fn texture(&self) -> &wgpu::Texture { &self.depth }
    /// Additional GPU memory only: cached depth and partition/list buffer.
    pub fn gpu_bytes(&self) -> u64 { u64::from(self.depth.width()) * u64::from(self.depth.height()) * 4 + self.owners.size() }

    pub fn update_instances(&mut self, gpu: &Gpu, first: usize, instances: &[GpuInstance]) -> Result<bool, String> {
        let (dirty, membership) = self.partition.update(first, instances)?;
        if dirty { self.content.invalidate(); }
        if membership { gpu.queue.write_buffer(&self.owners, 0, bytemuck::cast_slice(&self.partition.words)); }
        Ok(dirty)
    }

    /// Re-pin wind-frozen trees around `cameras`: trees at least `radius`
    /// from every camera become static casters. Callers choose radius =
    /// freeze distance + guard and re-pin before any camera moves more than
    /// the guard from its pinned position, so every static tree stays at
    /// rest. Invalidates cached depth when membership changes.
    pub fn pin_frozen_trees(&mut self, gpu: &Gpu, cameras: &[Vec3], radius: f32) -> bool {
        let changed = self.partition.pin_frozen(cameras, radius);
        if changed {
            self.content.invalidate();
            gpu.queue.write_buffer(&self.owners, 0, bytemuck::cast_slice(&self.partition.words));
        }
        changed
    }

    /// Call for geometry/material/shader changes and scene-visibility changes
    /// not expressed by update_instances. Keeping depth while merely changing
    /// sunlight strength or disabling sun rendering is safe; dawn's key and
    /// current source revision must still be checked before reuse.
    pub fn invalidate(&mut self) { self.content.invalidate(); }
    pub fn prepare(&self, key: SunRasterKey) -> StaticSunPlan { self.content.prepare(key) }

    /// If plan.redraw_static, the caller must FIRST encode the static-only
    /// raster pass with Clear(1). Then this copy replaces ALL old dynamic depth.
    /// Raster the dynamic list afterward with Load + Less, even when empty.
    /// Call only on the same device that constructed this object.
    pub fn encode_seed(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Texture, plan: &StaticSunPlan) -> Result<(), String> {
        if plan.revision != self.content.revision { return Err("stale static sun plan after source invalidation".into()); }
        if !plan.redraw_static && self.content.submitted != Some(plan.key) {
            return Err("static sun reuse plan no longer matches cached depth".into());
        }
        if target == &self.depth || target.format() != wgpu::TextureFormat::Depth32Float
            || target.size() != self.depth.size() || target.sample_count() != 1
            || target.mip_level_count() != 1 || target.dimension() != wgpu::TextureDimension::D2
            || !target.usage().contains(wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT) {
            return Err("sun merge target must be a distinct matching single-level Depth32Float render/copy target".into());
        }
        let copy = |texture| wgpu::TexelCopyTextureInfo { texture, mip_level: 0,
            origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::DepthOnly };
        encoder.copy_texture_to_texture(copy(&self.depth), copy(target), self.depth.size());
        Ok(())
    }

    /// Queue submission, not command encoding, establishes valid cached depth.
    /// False means an intervening mutation invalidated this frame's plan.
    pub fn submitted(&mut self, plan: StaticSunPlan) -> bool { self.content.submitted(plan) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    fn source(category: GeometryCategory, flags: u32) -> GpuInstance {
        let mut i = GpuInstance::zeroed();
        i.cluster_count = 3; i.scale = 1.0; i.flags = flags;
        i.rows = [[1.,0.,0.,0.], [0.,1.,0.,0.], [0.,0.,1.,0.]];
        i.sphere = [0.,0.,0.,1.]; i.set_category(category); i
    }

    fn key() -> SunRasterKey { SunRasterKey::new(Mat4::IDENTITY, 0.5, 2.0, 0).unwrap() }

    fn ids(p: &Partition, kind: u32) -> &[u32] {
        let index = (kind - 1) as usize;
        let offset = p.words[index + 2] as usize;
        &p.words[offset..offset + p.words[index] as usize]
    }

    #[test]
    fn partition_keeps_all_static_blockers_and_every_animation_dynamic() {
        let mut scene = Scene::default();
        scene.instances = vec![source(GeometryCategory::Terrain, 0), source(GeometryCategory::Other, crate::scene::SOLID),
            source(GeometryCategory::Trees, 0), source(GeometryCategory::Other, FOLIAGE),
            source(GeometryCategory::Cars, 0), source(GeometryCategory::Other, 0),
            source(GeometryCategory::Other, 0), source(GeometryCategory::Other, 0),
            source(GeometryCategory::Other, NO_SHADOW)];
        // Far outside any driving camera: classification cannot remove it.
        scene.instances[1].sphere[0] = 100_000.;
        scene.part_wind.push(crate::part_wind::PartWind { owner: Some(5), ..Default::default() });
        scene.part_attachments.push(crate::part_wind::PartAttachment { owner: 6, pose: 0, rest: Mat4::IDENTITY });
        scene.baked_animations = Some(crate::baked_animation::BakedAnimations { parts: vec![crate::baked_animation::Part {
            owner: 7, actor: 0, bone: 0, local_sphere: [0.,0.,0.,1.] }], ..Default::default() });
        let p = Partition::new(&scene).unwrap();
        assert_eq!(ids(&p, STATIC), &[0,1]);
        assert_eq!(ids(&p, DYNAMIC), &[2,3,4,5,6,7]);
        assert_eq!(p.counts.skipped, 1);
        for flag in [LIVE_GRASS, WIND_GRASS, PART_WIND, crate::scene::SKINNED] {
            assert_eq!(class(&source(GeometryCategory::Other, flag), false, false), DYNAMIC);
            // Freezing never makes grass, part wind or non-trees static.
            assert_eq!(class(&source(GeometryCategory::Trees, flag | FOLIAGE), false, true), DYNAMIC);
        }
        assert_eq!(class(&source(GeometryCategory::Cars, FOLIAGE), false, true), DYNAMIC);
        assert_eq!(class(&source(GeometryCategory::Trees, FOLIAGE), true, true), DYNAMIC);
    }

    #[test]
    fn frozen_trees_become_static_only_beyond_the_pinned_radius() {
        let mut scene = Scene::default();
        let tree = |x: f32| { let mut t = source(GeometryCategory::Trees, FOLIAGE); t.rows[0][3] = x; t };
        scene.instances = vec![source(GeometryCategory::Terrain, 0), tree(10.0), tree(90.0), tree(-200.0)];
        let mut p = Partition::new(&scene).unwrap();
        assert_eq!(ids(&p, DYNAMIC), &[1,2,3]);
        assert!(p.pin_frozen(&[Vec3::ZERO], 80.0));
        assert_eq!(ids(&p, STATIC), &[0,2,3]);
        assert_eq!(ids(&p, DYNAMIC), &[1]);
        assert!(!p.pin_frozen(&[Vec3::ZERO], 80.0), "same pin: no membership change");
        // A second camera near the far tree pulls it back into the wind.
        assert!(p.pin_frozen(&[Vec3::ZERO, Vec3::new(-190.0, 0.0, 0.0)], 80.0));
        assert_eq!(ids(&p, DYNAMIC), &[1,3]);
        // The guarantee the caller relies on: re-pinning before a camera
        // moves more than `guard` keeps every static tree past `freeze`.
        let freeze = crate::wind::WindFreeze { fade_start: 50.0, frozen_beyond: 70.0 };
        let guard = 20.0;
        p.pin_frozen(&[Vec3::ZERO], freeze.frozen_beyond + guard);
        for step in 0..=20 {
            let camera = Vec3::new(step as f32, 0.0, 0.0);
            for owner in ids(&p, STATIC) {
                let i = &p.sources[*owner as usize];
                if i.category() != GeometryCategory::Trees { continue; }
                assert_eq!(freeze.factor(Vec3::new(i.rows[0][3], 0.0, i.rows[2][3]), &[camera]), 0.0);
            }
        }
    }

    #[test]
    fn moving_dynamic_instances_keep_cache_but_static_changes_and_membership_invalidate() {
        let mut scene = Scene::default();
        scene.instances = vec![source(GeometryCategory::Terrain, 0), source(GeometryCategory::Cars, 0)];
        let mut p = Partition::new(&scene).unwrap();
        assert_eq!(p.update(0, &scene.instances).unwrap(), (false, false));
        let words = p.words.clone();
        let mut car = scene.instances[1]; car.rows[0][3] += 1.;
        assert_eq!(p.update(1, &[car]).unwrap(), (false, false));
        assert_eq!(p.words, words);
        let mut terrain = scene.instances[0]; terrain.rows[1][3] += 1.;
        assert_eq!(p.update(0, &[terrain]).unwrap(), (true, false));
        terrain.flags |= NO_SHADOW;
        assert_eq!(p.update(0, &[terrain]).unwrap(), (true, true));
        assert!(ids(&p, STATIC).is_empty());
        terrain.flags &= !NO_SHADOW;
        assert_eq!(p.update(0, &[terrain]).unwrap(), (true, true));
        terrain.flags |= FOLIAGE;
        assert_eq!(p.update(0, &[terrain]).unwrap(), (true, true));
        assert_eq!(ids(&p, DYNAMIC), &[0,1]);
    }

    #[test]
    fn moved_frozen_tree_is_demoted_until_its_new_position_is_certified() {
        let mut scene = Scene::default();
        let camera = Vec3::new(150.0, 12.0, -500.0);
        let mut tree = source(GeometryCategory::Trees, FOLIAGE);
        tree.rows[0][3] = camera.x;
        tree.rows[2][3] = camera.z + 150.0;
        scene.instances.push(tree);
        let mut p = Partition::new(&scene).unwrap();
        assert!(p.pin_frozen(&[camera], 90.0));
        assert_eq!(ids(&p, STATIC), &[0]);
        assert_eq!(p.update(0, &[tree]).unwrap(), (false, false));
        tree.rows[2][3] = camera.z + 10.0;
        assert_eq!(p.update(0, &[tree]).unwrap(), (true, true));
        assert_eq!(ids(&p, DYNAMIC), &[0]);
        assert!(!p.frozen[0]);
        assert!(!p.pin_frozen(&[camera], 90.0));
        tree.rows[2][3] = camera.z + 150.0;
        assert_eq!(p.update(0, &[tree]).unwrap(), (false, false));
        assert_eq!(ids(&p, DYNAMIC), &[0], "moving away is not a fresh pin certificate");
        assert!(p.pin_frozen(&[camera], 90.0));
        assert_eq!(ids(&p, STATIC), &[0]);
        assert!(p.pin_frozen(&[], 0.0), "disabling freeze must clear old static certificates");
        assert_eq!(ids(&p, DYNAMIC), &[0]);
    }

    #[test]
    fn invalid_updates_are_atomic_and_empty_scene_has_valid_header() {
        let mut scene = Scene::default();
        let empty = Partition::new(&scene).unwrap();
        assert_eq!(empty.words, vec![0,0,4,4]);
        scene.instances.push(source(GeometryCategory::Terrain, 0));
        let mut p = Partition::new(&scene).unwrap();
        let words = p.words.clone();
        assert!(p.update(usize::MAX, &scene.instances).is_err());
        assert!(p.update(1, &scene.instances).is_err());
        assert_eq!(p.words, words);
        scene.part_wind.push(crate::part_wind::PartWind {owner: Some(1), ..Default::default()});
        assert!(Partition::new(&scene).is_err());
    }

    #[test]
    fn cache_requires_submit_and_rejects_stale_tickets_and_changed_raster_state() {
        let mut c = Content::default();
        let first = c.prepare(key()); assert!(first.redraw_static);
        assert!(c.prepare(key()).redraw_static); // Encoding/cancelled frame is not submission.
        assert!(c.submitted(first)); assert!(!c.prepare(key()).redraw_static);
        let before_mutation = c.prepare(key()); c.invalidate();
        assert!(!c.submitted(before_mutation)); assert!(c.prepare(key()).redraw_static);
        assert!(c.submitted(c.prepare(key())));
        let mut projection = Mat4::IDENTITY;
        projection.w_axis.x = f32::EPSILON;
        for other in [SunRasterKey::new(projection,0.5,2.0,0).unwrap(),
            SunRasterKey::new(Mat4::IDENTITY,0.6,2.0,0).unwrap(),
            SunRasterKey::new(Mat4::IDENTITY,0.5,1.0,0).unwrap(),
            SunRasterKey::new(Mat4::IDENTITY,0.5,2.0,1).unwrap()] {
            assert!(c.prepare(other).redraw_static);
        }
        // No sun work at night does not mutate correct depth. Dawn at the
        // same projection can reuse it; changed projection above cannot.
        assert!(!c.prepare(key()).redraw_static);
        assert!(SunRasterKey::new(Mat4::IDENTITY,f32::NAN,1.,0).is_err());
        assert!(SunRasterKey::new(Mat4::IDENTITY,0.5,-1.,0).is_err());
    }

    #[test]
    fn copied_static_depth_and_less_dynamic_draw_equal_unsplit_depth() {
        let static_fragments = [[0.7f32,1.,0.4,1.], [0.8,0.2,1.,1.]];
        let moving_a = [0.1f32,0.6,1.,0.3];
        let moving_b = [1.0f32,1.,0.9,1.];
        let mut fixed = [1.0f32;4];
        for fragment in static_fragments { for i in 0..4 { fixed[i] = fixed[i].min(fragment[i]); } }
        for dynamic in [moving_a, moving_b] {
            let mut seeded = fixed; // Copy replaces last frame's car shadow.
            for i in 0..4 { if dynamic[i] < seeded[i] { seeded[i] = dynamic[i]; } }
            let mut unsplit = [1.0f32;4];
            for fragment in [static_fragments[0],dynamic,static_fragments[1]] {
                for i in 0..4 { unsplit[i] = unsplit[i].min(fragment[i]); }
            }
            assert_eq!(seeded, unsplit);
        }
        assert_eq!(fixed[3], 1.0); // Alpha hole stays empty after the car moves.
    }

    #[test]
    fn guarded_projection_holds_translation_but_recovers_cuts_sun_and_spread() {
        for sun in [Vec3::new(0.7,0.02,0.4).normalize(),Vec3::new(0.4,0.8,-0.2).normalize(),Vec3::Y] {
            let mut guard=GuardedAnchor::default();
            let start=Vec3::new(2000.,120.,-3000.);
            let extent=crate::stability::quantized_shadow_extent((90.+32.)/FULL_COVERAGE_NDC);
            let (anchor,fresh)=guard.update(&[start],sun,extent,90.,4096).unwrap(); assert!(fresh);
            let baseline=crate::stability::stable_shadow_depth(anchor,sun,extent,4096,[-5000.,5000.]);
            for shift in [Vec3::X*20.,Vec3::Y*15.,Vec3::Z*(-25.)] {
                let (held,rebuilt)=guard.update(&[start+shift],sun,extent,90.,4096).unwrap();
                assert!(!rebuilt);assert_eq!(held,anchor);
                assert_eq!(crate::stability::stable_shadow_depth(held,sun,extent,4096,[-5000.,5000.]),baseline);
                for axis in [Vec3::X,Vec3::Y,Vec3::Z] {for sign in [-1.,1.] {
                    let clip=baseline.project_point3(start+shift+axis*sign*90.);
                    assert!(clip.x.abs().max(clip.y.abs())<FULL_COVERAGE_NDC);
                }}
            }
            let views=[start+Vec3::X*200.,start+Vec3::Z*200.,start-Vec3::X*200.,start-Vec3::Z*200.];
            let larger=crate::stability::quantized_shadow_extent((90.+200.+32.)/FULL_COVERAGE_NDC);
            assert!(guard.update(&views,sun,larger,90.,4096).unwrap().1);
            let moved=views.map(|v|v+Vec3::new(1000.,70.,-800.));
            assert!(guard.update(&moved,sun,larger,90.,4096).unwrap().1);
            assert!(guard.update(&moved,-sun,larger,90.,4096).unwrap().1);
            assert!(guard.update(&moved,-sun,64.,90.,4096).is_err());
        }
        assert_eq!(configured_guard(Some("32")),32.);
        assert_eq!(configured_guard(Some("NaN")),0.);
        assert_eq!(configured_guard(Some("-1")),0.);
    }
}
