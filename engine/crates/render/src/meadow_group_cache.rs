//! Bounded world-keyed surface-page residency for the meadow renderer.
//!
//! A page represents actual integer root cells, not a repeated macro recipe.
//! Its producer is supplied by MeadowRenderer and uses the ordinary root and
//! source-atlas shader. This is a surface cache, not voxel transport. Missing,
//! stale and uncertain pages retain the ordinary per-root renderer.

use glam::{Mat4, Vec3, Vec3Swizzles};
use std::collections::{BTreeMap, BTreeSet};

pub const GROUP_SIDE: i32 = 8;
pub const PAGE_SIZE: u32 = 64;
pub const PAGE_CAPACITY: usize = 256;
pub const UPDATE_BUDGET: usize = 4;
pub const LOOKUP_CAPACITY: usize = 2048;
pub const PAGE_WORDS: usize = 16;
pub const MAX_ERROR_PIXELS: f32 = 1.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Key {
    pub x: i32,
    pub z: i32,
    pub azimuth: u16,
    pub elevation: u16,
}
impl Key {
    pub fn root_origin(self) -> [i32; 2] {
        [self.x * GROUP_SIDE, self.z * GROUP_SIDE]
    }
    pub fn direction(self) -> Vec3 {
        let az = self.azimuth as f32 * std::f32::consts::TAU / 512.0;
        let el = self.elevation as f32 * std::f32::consts::FRAC_PI_2 / 256.0;
        Vec3::new(az.sin() * el.cos(), el.sin(), az.cos() * el.cos())
    }
    pub fn from_direction(x: i32, z: i32, direction: Vec3) -> Self {
        let d = direction.normalize_or_zero();
        let az = (d.x.atan2(d.z) / std::f32::consts::TAU).rem_euclid(1.0);
        Self {
            x,
            z,
            azimuth: ((az * 512.0).round() as u32 % 512) as u16,
            elevation: ((d.y.clamp(0.0, 1.0).asin() / std::f32::consts::FRAC_PI_2 * 256.0).round()
                as u32)
                .min(256) as u16,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub lo: Vec3,
    pub hi: Vec3,
}
impl Bounds {
    pub fn center(self) -> Vec3 {
        (self.lo + self.hi) * 0.5
    }
    pub fn radius(self) -> f32 {
        (self.hi - self.lo).length() * 0.5
    }
    pub fn corners(self) -> [Vec3; 8] {
        std::array::from_fn(|k| {
            Vec3::new(
                if k & 1 == 0 { self.lo.x } else { self.hi.x },
                if k & 2 == 0 { self.lo.y } else { self.hi.y },
                if k & 4 == 0 { self.lo.z } else { self.hi.z },
            )
        })
    }
    pub fn projection(self, key: Key) -> (Mat4, Vec3, f32) {
        let direction = key.direction();
        let center = self.center();
        let right = if direction.y.abs() > 0.999 {
            Vec3::X
        } else {
            Vec3::Y.cross(direction).normalize()
        };
        let up = direction.cross(right);
        let mut half = Vec3::splat(0.01);
        for corner in self.corners() {
            let p = corner - center;
            half = half.max(Vec3::new(
                p.dot(right).abs(),
                p.dot(up).abs(),
                p.dot(direction).abs(),
            ));
        }
        // One texel of border retains a genuinely empty outside support.
        half.x *= 1.0 + 2.0 / PAGE_SIZE as f32;
        half.y *= 1.0 + 2.0 / PAGE_SIZE as f32;
        half.z += 0.1;
        let eye = center + direction * (half.z + 0.1);
        let view = Mat4::look_at_rh(eye, center, up);
        let projection =
            Mat4::orthographic_rh(-half.x, half.x, -half.y, half.y, 2.0 * half.z + 0.2, 0.01);
        (projection * view, eye, PAGE_SIZE as f32 / (2.0 * half.y))
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub key: Key,
    pub bounds: Bounds,
    pub priority: f32,
    pub views: u8,
}
#[derive(Clone, Debug)]
pub struct Resident {
    pub key: Key,
    pub bounds: Bounds,
    pub cooked_frame: u64,
    pub used_frame: u64,
    pub phase: f32,
    pub strength: f32,
    pub ready: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub requested: u32,
    pub hits: u32,
    pub misses: u32,
    pub cooked: u32,
    pub resident: u32,
    pub evictions: u32,
    pub deferred: u32,
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub updates: Vec<usize>,
    pub slots: BTreeMap<Key, usize>,
    pub stats: Stats,
}
pub struct Residency {
    pub pages: Vec<Option<Resident>>,
    pub frame: u64,
}
impl Default for Residency {
    fn default() -> Self {
        Self {
            pages: vec![None; PAGE_CAPACITY],
            frame: 0,
        }
    }
}
impl Residency {
    pub fn clear(&mut self) {
        self.pages.fill(None);
    }
    /// All cameras request before choosing victims: a page already used by
    /// another view cannot be evicted midway through this frame's commands.
    pub fn plan(
        &mut self,
        requests: &[Request],
        frame: u64,
        phase: f32,
        strength: f32,
        pose_limit: f32,
    ) -> Plan {
        self.frame = frame;
        let mut merged: BTreeMap<Key, Request> = BTreeMap::new();
        for request in requests {
            merged
                .entry(request.key)
                .and_modify(|r| {
                    r.priority = r.priority.max(request.priority);
                    r.views |= request.views;
                })
                .or_insert_with(|| request.clone());
        }
        let requested: BTreeSet<_> = merged.keys().copied().collect();
        let mut ordered: Vec<_> = merged.into_values().collect();
        ordered.sort_by(|a, b| b.priority.total_cmp(&a.priority).then(a.key.cmp(&b.key)));
        let mut plan = Plan {
            updates: vec![],
            slots: BTreeMap::new(),
            stats: Stats {
                requested: ordered.len() as u32,
                ..Default::default()
            },
        };
        for request in ordered {
            let existing = self
                .pages
                .iter()
                .position(|p| p.as_ref().is_some_and(|p| p.key == request.key));
            let valid = existing.is_some_and(|i| {
                let p = self.pages[i].as_ref().unwrap();
                p.ready && phase_distance(p.phase, phase) <= pose_limit && p.strength == strength
            });
            if valid {
                let i = existing.unwrap();
                self.pages[i].as_mut().unwrap().used_frame = frame;
                plan.slots.insert(request.key, i);
                plan.stats.hits += 1;
                continue;
            }
            plan.stats.misses += 1;
            if plan.updates.len() >= UPDATE_BUDGET {
                plan.stats.deferred += 1;
                continue;
            }
            let victim = existing
                .or_else(|| self.pages.iter().position(Option::is_none))
                .or_else(|| {
                    self.pages
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p.as_ref().is_some_and(|p| !requested.contains(&p.key)))
                        .min_by_key(|(_, p)| p.as_ref().unwrap().used_frame)
                        .map(|(i, _)| i)
                });
            let Some(i) = victim else {
                plan.stats.deferred += 1;
                continue;
            };
            if self.pages[i].as_ref().is_some_and(|p| p.key != request.key) {
                plan.stats.evictions += 1;
            }
            self.pages[i] = Some(Resident {
                key: request.key,
                bounds: request.bounds,
                cooked_frame: frame,
                used_frame: frame,
                phase,
                strength,
                ready: false,
            });
            plan.updates.push(i);
            plan.slots.insert(request.key, i);
        }
        plan.stats.cooked = plan.updates.len() as u32;
        plan.stats.resident = self.pages.iter().filter(|p| p.is_some()).count() as u32;
        plan
    }
    pub fn commit(&mut self, slots: &[usize]) {
        for &slot in slots {
            if let Some(page) = &mut self.pages[slot] {
                page.ready = true;
            }
        }
    }
}

pub fn phase_distance(a: f32, b: f32) -> f32 {
    ((a - b + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs()
}

/// Conservatively bounded field blocks; no CPU root generation. Invalid blocks
/// refuse caching and leave the GPU reference producer authoritative.
pub struct TerrainBounds {
    origin: glam::Vec2,
    cell: f32,
    width: u32,
    height: u32,
    blocks: Vec<[f32; 2]>,
}
impl TerrainBounds {
    pub fn new(field: &crate::meadow_data::MeadowField) -> Self {
        let width = (field.width - 1).div_ceil(4);
        let height = (field.height - 1).div_ceil(4);
        let mut blocks = vec![[f32::INFINITY, f32::NEG_INFINITY]; (width * height) as usize];
        for z in 0..height {
            for x in 0..width {
                let value = &mut blocks[(z * width + x) as usize];
                let mut valid = true;
                for zz in z * 4..=(z * 4 + 4).min(field.height - 1) {
                    for xx in x * 4..=(x * 4 + 4).min(field.width - 1) {
                        let p = field.samples[(zz * field.width + xx) as usize];
                        valid &= p[2] >= 0.5 && p[0].is_finite();
                        value[0] = value[0].min(p[0]);
                        value[1] = value[1].max(p[0]);
                    }
                }
                if !valid {
                    *value = [f32::INFINITY, f32::NEG_INFINITY];
                }
            }
        }
        Self {
            origin: glam::Vec2::from_array(field.origin),
            cell: field.cell * 4.0,
            width,
            height,
            blocks,
        }
    }
    pub fn group(&self, x: i32, z: i32, margin: f32) -> Option<Bounds> {
        let spacing = crate::meadow_data::ROOT_CELL * GROUP_SIDE as f32;
        let lo = glam::Vec2::new(x as f32, z as f32) * spacing - glam::Vec2::splat(margin);
        let hi = lo + glam::Vec2::splat(spacing + 2.0 * margin);
        let a = ((lo - self.origin) / self.cell).floor().as_ivec2();
        let b = ((hi - self.origin) / self.cell).floor().as_ivec2();
        if a.min_element() < 0 || b.x >= self.width as i32 || b.y >= self.height as i32 {
            return None;
        }
        let mut ylo = f32::INFINITY;
        let mut yhi = f32::NEG_INFINITY;
        for zz in a.y..=b.y {
            for xx in a.x..=b.x {
                let v = self.blocks[zz as usize * self.width as usize + xx as usize];
                if v[0] > v[1] {
                    return None;
                }
                ylo = ylo.min(v[0]);
                yhi = yhi.max(v[1]);
            }
        }
        Some(Bounds {
            lo: Vec3::new(lo.x, ylo - 2.0 * margin, lo.y),
            hi: Vec3::new(hi.x, yhi + 2.0 * margin, hi.y),
        })
    }
    pub fn bytes(&self) -> u64 {
        (self.blocks.len() * 8) as u64
    }
}

/// A sufficient geometric bound, not a claim that an orthographic surface
/// cache retains arbitrary disocclusions. It limits every support point's
/// angular/parallax displacement and the page texel footprint; uncertain
/// opaque-depth cuts are additionally rejected by the consumer.
pub fn projection_error(bounds: Bounds, key: Key, camera: crate::Camera, pixels: f32) -> f32 {
    let delta = camera.pos - bounds.center();
    let distance = delta.length();
    let near = distance - bounds.radius();
    if near <= 1.0 || delta.y < 0.0 {
        return f32::INFINITY;
    }
    let direction = key.direction();
    let angle = (delta / distance - direction).length();
    let mut depth = 0.0f32;
    let mut radius = 0.0f32;
    for corner in bounds.corners() {
        let p = corner - bounds.center();
        let z = p.dot(direction);
        depth = depth.max(z.abs());
        radius = radius.max((p - direction * z).length());
    }
    let shift = radius * angle + depth * radius / near;
    (shift * pixels / near).max(2.0 * radius / PAGE_SIZE as f32 * pixels / near)
}

pub fn requests(
    terrain: &TerrainBounds,
    views: &[crate::ViewRect],
    range: [f32; 2],
    margin: f32,
) -> Vec<Request> {
    let spacing = crate::meadow_data::ROOT_CELL * GROUP_SIDE as f32;
    let mut result = Vec::new();
    for (index, view) in views.iter().take(4).enumerate() {
        let focal = view.h as f32 / (2.0 * (view.cam.fov_y * 0.5).tan());
        let forward = (view.cam.target - view.cam.pos).normalize_or_zero();
        let span = (range[1] / spacing).ceil() as i32 + 1;
        let center = (view.cam.pos.xz() / spacing).floor().as_ivec2();
        for z in center.y - span..=center.y + span {
            for x in center.x - span..=center.x + span {
                let Some(bounds) = terrain.group(x, z, margin) else {
                    continue;
                };
                let delta = bounds.center() - view.cam.pos;
                let horizontal = delta.xz().length();
                let radius = bounds.radius();
                if horizontal - radius < range[0]
                    || horizontal + radius >= range[1]
                    || delta.dot(forward) < -radius
                {
                    continue;
                }
                // Broad conservative cone only: the regular GPU producer still
                // performs actual frustum/Hi-Z for all fallback roots.
                let angle =
                    (view.cam.fov_y * 0.5).tan() * (view.w as f32 / view.h.max(1) as f32).max(1.0);
                if delta.cross(forward).length() > delta.dot(forward).max(0.0) * angle + radius {
                    continue;
                }
                let key = Key::from_direction(x, z, -delta);
                if projection_error(bounds, key, view.cam, focal) > MAX_ERROR_PIXELS * 0.8 {
                    continue;
                }
                result.push(Request {
                    key,
                    bounds,
                    priority: focal / (delta.length() + 1.0),
                    views: 1 << index,
                });
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(x: i32, views: u8) -> Request {
        Request {
            key: Key::from_direction(x, -1, Vec3::Z),
            bounds: Bounds {
                lo: Vec3::ZERO,
                hi: Vec3::ONE,
            },
            priority: 1.0,
            views,
        }
    }
    #[test]
    fn world_groups_use_euclidean_cells_and_camera_independent_identity() {
        for cell in [-17i32, -9, -8, -1, 0, 7, 8, 17] {
            let group = cell.div_euclid(GROUP_SIDE);
            let local = cell.rem_euclid(GROUP_SIDE);
            assert!((0..GROUP_SIDE).contains(&local));
            assert_eq!(group * GROUP_SIDE + local, cell);
        }
        let key = Key::from_direction(-2, 3, Vec3::new(1.0, 0.2, 1.0));
        assert_eq!(key.root_origin(), [-16, 24]);
        assert_eq!(
            key,
            Key::from_direction(-2, 3, Vec3::new(1.0, 0.2, 1.0) * 20.0)
        );
    }
    #[test]
    fn shared_requests_deduplicate_and_never_publish_uncommitted_payload() {
        let mut cache = Residency::default();
        let a = cache.plan(&[request(0, 1), request(0, 2)], 1, 0.0, 0.6, 0.01);
        assert_eq!(a.stats.requested, 1);
        assert_eq!(a.updates.len(), 1);
        assert!(!cache.pages[a.updates[0]].as_ref().unwrap().ready);
        cache.commit(&a.updates);
        let b = cache.plan(&[request(0, 4)], 2, 0.0, 0.6, 0.01);
        assert_eq!(b.stats.hits, 1);
        assert!(b.updates.is_empty());
    }
    #[test]
    fn budget_overflow_is_explicit_miss_not_missing_root() {
        let mut cache = Residency::default();
        let requests: Vec<_> = (0..20).map(|x| request(x, 1)).collect();
        let a = cache.plan(&requests, 1, 0.0, 0.6, 0.01);
        assert_eq!(a.updates.len(), UPDATE_BUDGET);
        assert_eq!(a.stats.deferred, 16);
        assert_eq!(a.slots.len(), UPDATE_BUDGET);
        cache.commit(&a.updates);
        let b = cache.plan(&requests, 2, 0.0, 0.6, 0.01);
        assert_eq!(b.stats.hits, 4);
        assert_eq!(b.updates.len(), 4);
        let c = cache.plan(&requests, 3, 1.0, 0.6, 0.01);
        assert_eq!(c.stats.hits, 0);
        assert_eq!(c.stats.deferred, 16);
    }
    #[test]
    fn orthographic_projection_contains_all_bounds_and_keeps_reverse_depth() {
        let bounds = Bounds {
            lo: Vec3::new(-200.0, -4.0, 600.0),
            hi: Vec3::new(-194.0, 5.0, 607.0),
        };
        for direction in [Vec3::Z, Vec3::new(1.0, 0.2, 1.0), Vec3::Y] {
            let key = Key::from_direction(-1, 1, direction);
            let (vp, eye, _) = bounds.projection(key);
            for point in bounds.corners() {
                let p = vp.project_point3(point);
                assert!(
                    p.x.abs() < 1.0 && p.y.abs() < 1.0 && p.z >= 0.0 && p.z <= 1.0,
                    "{p:?}"
                );
            }
            assert!(
                vp.project_point3(bounds.center() + key.direction() * 0.1).z
                    > vp.project_point3(bounds.center()).z
            );
            assert!(eye.is_finite());
        }
    }
    #[test]
    fn all_requested_views_protect_resident_pages_from_eviction() {
        let mut cache = Residency::default();
        let requests: Vec<_> = (0..PAGE_CAPACITY as i32).map(|x| request(x, 1)).collect();
        for frame in 0..PAGE_CAPACITY / UPDATE_BUDGET {
            let p = cache.plan(&requests, frame as u64, 0.0, 0.6, 0.1);
            cache.commit(&p.updates);
        }
        assert!(cache
            .pages
            .iter()
            .all(|p| p.as_ref().is_some_and(|p| p.ready)));
        let mut both = requests.clone();
        both.push(request(999, 2));
        let p = cache.plan(&both, 100, 0.0, 0.6, 0.1);
        assert!(p.updates.is_empty());
        assert_eq!(p.stats.hits, 256);
        assert_eq!(p.stats.deferred, 1);
        assert_eq!(p.stats.evictions, 0);
    }
    #[test]
    fn translation_keeps_world_projection_and_has_nonzero_reuse_width() {
        let bounds = Bounds {
            lo: Vec3::new(-2.5, -1.5, -2.5),
            hi: Vec3::new(2.5, 1.5, 2.5),
        };
        let camera = crate::Camera {
            pos: Vec3::new(0.0, 10.0, 160.0),
            target: Vec3::ZERO,
            fov_y: 64f32.to_radians(),
            near: 0.1,
        };
        let key = Key::from_direction(0, 0, camera.pos);
        let (vp, _, _) = bounds.projection(key);
        for x in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            let moved = crate::Camera {
                pos: camera.pos + Vec3::X * x,
                ..camera
            };
            assert_eq!(
                vp,
                bounds.projection(key).0,
                "camera translation is not in page projection/key"
            );
            assert!(
                projection_error(bounds, key, moved, 864.0) < 1.0,
                "bounded translated reuse"
            );
        }
        let close = crate::Camera {
            pos: Vec3::new(0.0, 1.0, 8.0),
            ..camera
        };
        assert!(projection_error(bounds, key, close, 864.0) > 1.0);
    }
}
