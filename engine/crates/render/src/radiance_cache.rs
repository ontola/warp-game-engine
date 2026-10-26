//! Bounded world-space diffuse transport reference and GPU upload contract.
//!
//! This is NOT integrated into production rendering. Unlike the existing
//! heuristic ground bounce, each incoming ray here evaluates an actual hit's
//! emission and shadowed Lambertian reflection, or explicitly certified sky.
//! Unknown geometry/lighting never silently becomes unoccluded environment.
//! Shared probe residency is independent of camera orientation and view count.
use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Vec3};
use std::collections::{BTreeMap, BTreeSet};

pub const FACE_SIZE: usize = 4;
pub const RAYS_PER_PROBE: usize = 6 * FACE_SIZE * FACE_SIZE;
const PI: f32 = std::f32::consts::PI;
// Keep integer identities exactly representable in f32, with room for the
// resident stencil. Also avoids float-to-i32 saturation before key addition.
const MAX_GRID_COORD: f32 = 16_777_200.0;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub spacing: f32,
    pub radius_cells: [i32; 3],
    pub capacity: usize,
    pub probes_per_update: usize,
    pub trace_distance: f32,
    pub sun_trace_distance: f32,
    pub ray_bias: f32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            spacing: 4.0,
            radius_cells: [4, 2, 4],
            capacity: 2048,
            probes_per_update: 4,
            trace_distance: 64.0,
            sun_trace_distance: 1024.0,
            ray_bias: 0.002,
        }
    }
}

/// Integer world-cell identity, shared by all views. No screen-space index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProbeKey(pub [i32; 3]);
impl ProbeKey {
    pub fn position(self, spacing: f32) -> Vec3 {
        IVec3::from_array(self.0).as_vec3() * spacing
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    /// Unit world direction; distances are world metres.
    pub direction: Vec3,
    pub min_distance: f32,
    pub max_distance: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SurfaceHit {
    pub distance: f32,
    /// Outward world-space geometric normal, not a shading-normal map.
    pub normal: Vec3,
    pub albedo: Vec3,
    pub emission: Vec3,
}

#[derive(Clone, Copy, Debug)]
pub enum Trace {
    Hit(SurfaceHit),
    /// The backend certifies that the ray reaches the environment, not merely
    /// that a finite segment is clear. Missing world coverage is NOT this.
    KnownSky,
    Unknown,
}

/// The same world visibility backend must serve probe rays and sun rays.
/// A finite terrain TraceResult::Clear is insufficient to return KnownSky;
/// its adapter must independently certify escape from the complete scene.
pub trait Transport {
    fn trace(&self, ray: Ray) -> Trace;
    /// Linear environment radiance, without the explicitly sampled sun disc.
    fn sky_radiance(&self, direction: Vec3) -> Vec3;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectLight {
    pub sun_direction: Vec3,
    /// RGB irradiance on a surface perpendicular to the sun direction.
    pub sun_irradiance: Vec3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Revision {
    pub geometry: u64,
    /// Includes environment/sky, material and emission changes.
    pub lighting: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuProbe {
    pub position_spacing: [f32; 4],
    /// Cosine-convolved real SH9. Evaluation returns irradiance, not albedo.
    pub irradiance_sh: [[f32; 4]; 9],
    /// x: valid (all rays resolved and probe outside opaque geometry),
    /// y: unresolved rays, z: first directional moment, w: ray count.
    pub meta: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuMoment {
    /// mean distance, mean squared distance, known(0/1), reserved.
    /// Directional order: six cube faces, row-major 4x4 per face.
    pub data: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct Probe {
    pub key: ProbeKey,
    pub irradiance_sh: [Vec3; 9],
    pub moments: [GpuMoment; RAYS_PER_PROBE],
    pub unresolved_rays: u32,
    pub inside_geometry: bool,
    pub valid: bool,
    last_attempt: u64,
}
impl Probe {
    fn empty(key: ProbeKey) -> Self {
        Self {
            key,
            irradiance_sh: [Vec3::ZERO; 9],
            moments: [GpuMoment::zeroed(); RAYS_PER_PROBE],
            unresolved_rays: RAYS_PER_PROBE as u32,
            inside_geometry: false,
            valid: false,
            last_attempt: 0,
        }
    }
    pub fn irradiance(&self, normal: Vec3) -> Vec3 {
        if !self.valid || !normal.is_finite() || normal.length_squared() < 1e-12 {
            return Vec3::ZERO;
        }
        let y = sh_basis(normal.normalize());
        self.irradiance_sh
            .iter()
            .zip(y)
            .map(|(c, b)| *c * b)
            .sum::<Vec3>()
            .max(Vec3::ZERO)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ResidencyStats {
    pub requested: usize,
    pub resident: usize,
    pub retained: usize,
    pub dropped: usize,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct UpdateStats {
    pub updated_probes: usize,
    pub primary_rays: usize,
    pub sun_rays: usize,
    pub unknown_rays: usize,
    pub valid_probes: usize,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub irradiance: Vec3,
    /// Missing/unresolved corners remain explicit; this is not a promise of
    /// exact visibility. The returned energy is attenuated by this coverage.
    pub known_weight: f32,
    pub visibility_weight: f32,
    pub contributing_probes: u32,
}

pub struct RadianceCache {
    config: Config,
    probes: BTreeMap<ProbeKey, Probe>,
    revision: Option<(Revision, DirectLight)>,
    attempt: u64,
}
impl RadianceCache {
    pub fn config(&self) -> Config {
        self.config
    }

    pub fn new(config: Config) -> Result<Self, String> {
        if !config.spacing.is_finite()
            || !(0.001..=1.0e6).contains(&config.spacing)
            || config.radius_cells.iter().any(|v| !(0..=8).contains(v))
            || !(1..=8192).contains(&config.capacity)
            || !(1..=64).contains(&config.probes_per_update)
            || !config.trace_distance.is_finite()
            || !(0.001..=1.0e6).contains(&config.trace_distance)
            || !config.sun_trace_distance.is_finite()
            || !(0.001..=1.0e6).contains(&config.sun_trace_distance)
            || !config.ray_bias.is_finite()
            || !(0.0..=0.1).contains(&config.ray_bias)
        {
            return Err("invalid bounded radiance-cache configuration".into());
        }
        Ok(Self {
            config,
            probes: BTreeMap::new(),
            revision: None,
            attempt: 0,
        })
    }

    /// One call with the union of up to four player origins. Identical views
    /// request identical keys, not duplicate allocations or update budgets.
    pub fn set_views(&mut self, origins: &[Vec3]) -> Result<ResidencyStats, String> {
        if origins.is_empty()
            || origins.len() > 4
            || origins.iter().any(|p| {
                !p.is_finite() || (p / self.config.spacing).abs().max_element() > MAX_GRID_COORD
            })
        {
            return Err("radiance cache needs 1..4 finite bounded world origins".into());
        }
        let mut keys = BTreeSet::new();
        let [rx, ry, rz] = self.config.radius_cells;
        let centres: Vec<_> = origins
            .iter()
            .map(|origin| (*origin / self.config.spacing).floor().as_ivec3())
            .collect();
        for &cell in &centres {
            for z in -rz..=rz {
                for y in -ry..=ry {
                    for x in -rx..=rx {
                        keys.insert(ProbeKey((cell + IVec3::new(x, y, z)).to_array()));
                    }
                }
            }
        }
        let requested = keys.len();
        let mut keys: Vec<_> = keys.into_iter().collect();
        let distance = |key: &ProbeKey| {
            centres
                .iter()
                .map(|p| {
                    key.position(self.config.spacing)
                        .distance_squared(p.as_vec3() * self.config.spacing)
                })
                .fold(f32::INFINITY, f32::min)
        };
        keys.sort_by(|a, b| distance(a).total_cmp(&distance(b)).then_with(|| a.cmp(b)));
        keys.truncate(self.config.capacity);
        let keep: BTreeSet<_> = keys.into_iter().collect();
        self.probes.retain(|key, _| keep.contains(key));
        let retained = self.probes.len();
        for key in keep {
            self.probes.entry(key).or_insert_with(|| Probe::empty(key));
        }
        Ok(ResidencyStats {
            requested,
            resident: self.probes.len(),
            retained,
            dropped: requested.saturating_sub(self.probes.len()),
        })
    }

    pub fn probes(&self) -> impl Iterator<Item = &Probe> {
        self.probes.values()
    }

    /// Hard maximum: probes_per_update * 96 primary rays and at most the
    /// same number of sun rays. Backend traversal must impose its own bound.
    pub fn update<T: Transport>(
        &mut self,
        world: &T,
        light: DirectLight,
        revision: Revision,
    ) -> Result<UpdateStats, String> {
        if !light.sun_direction.is_finite()
            || (light.sun_direction.length_squared() - 1.0).abs() > 1e-3
            || !valid_rgb(light.sun_irradiance)
        {
            return Err("radiance transport requires a unit sun direction and finite nonnegative irradiance".into());
        }
        if self.revision != Some((revision, light)) {
            for p in self.probes.values_mut() {
                p.valid = false;
                // Keep scheduling age across revisions. A continuously moving
                // sun must not repeatedly update only the first few keys.
            }
            self.revision = Some((revision, light));
        }
        let mut order: Vec<_> = self
            .probes
            .values()
            .map(|p| (p.valid, p.last_attempt, p.key))
            .collect();
        order.sort();
        let mut stats = UpdateStats::default();
        for (_, _, key) in order.into_iter().take(self.config.probes_per_update) {
            self.attempt = self.attempt.saturating_add(1);
            let mut p = gather(key, self.config, world, light, &mut stats);
            p.last_attempt = self.attempt;
            self.probes.insert(key, p);
            stats.updated_probes += 1;
        }
        stats.valid_probes = self.probes.values().filter(|p| p.valid).count();
        Ok(stats)
    }

    /// Visibility-aware interpolation, with explicit unresolved coverage.
    /// Directional moments are a low-resolution approximation, not proof that
    /// every thin wall is resolved. No epsilon light floor is added.
    pub fn sample(&self, position: Vec3, normal: Vec3) -> Sample {
        if !position.is_finite() || !normal.is_finite() || normal.length_squared() < 1e-12 {
            return Sample::default();
        }
        let grid = position / self.config.spacing;
        if grid.abs().max_element() > MAX_GRID_COORD {
            return Sample::default();
        }
        let base = grid.floor().as_ivec3();
        let t = grid - grid.floor();
        let mut result = Sample::default();
        for z in 0..=1 {
            for y in 0..=1 {
                for x in 0..=1 {
                    let weight = (if x == 0 { 1.0 - t.x } else { t.x })
                        * (if y == 0 { 1.0 - t.y } else { t.y })
                        * (if z == 0 { 1.0 - t.z } else { t.z });
                    if weight <= 0.0 {
                        continue;
                    }
                    let key = ProbeKey((base + IVec3::new(x, y, z)).to_array());
                    let Some(probe) = self.probes.get(&key).filter(|p| p.valid) else {
                        continue;
                    };
                    result.known_weight += weight;
                    let delta = position - key.position(self.config.spacing);
                    let distance = delta.length();
                    let visibility = if distance < 1e-5 {
                        1.0
                    } else {
                        let moment = interpolate_moments(&probe.moments, delta / distance);
                        moment_visibility(moment, distance, self.config.ray_bias)
                    };
                    if visibility <= 0.0 {
                        continue;
                    }
                    let w = weight * visibility;
                    result.irradiance += probe.irradiance(normal) * w;
                    result.visibility_weight += w;
                    result.contributing_probes += 1;
                }
            }
        }
        if result.visibility_weight > 1e-8 {
            // Normalize only the visible interpolation, then retain missing
            // data loss explicitly instead of filling it from a bright probe.
            result.irradiance *= result.known_weight / result.visibility_weight;
        } else {
            result.irradiance = Vec3::ZERO;
        }
        result
    }

    /// Packed records for a future shared GPU buffer. No GPU allocation/upload
    /// or production shader access occurs here. Order follows stable keys.
    pub fn packed(&self) -> (Vec<GpuProbe>, Vec<GpuMoment>) {
        let mut probes = Vec::with_capacity(self.probes.len());
        let mut moments = Vec::with_capacity(self.probes.len() * RAYS_PER_PROBE);
        for p in self.probes.values() {
            let position = p.key.position(self.config.spacing);
            let sh = if p.valid {
                p.irradiance_sh.map(|v| [v.x, v.y, v.z, 0.0])
            } else {
                [[0.0; 4]; 9]
            };
            probes.push(GpuProbe {
                position_spacing: [position.x, position.y, position.z, self.config.spacing],
                irradiance_sh: sh,
                meta: [
                    u32::from(p.valid),
                    p.unresolved_rays,
                    moments.len() as u32,
                    RAYS_PER_PROBE as u32,
                ],
            });
            moments.extend_from_slice(&p.moments);
        }
        (probes, moments)
    }
}

fn valid_rgb(v: Vec3) -> bool {
    v.is_finite() && v.min_element() >= 0.0
}

fn gather<T: Transport>(
    key: ProbeKey,
    config: Config,
    world: &T,
    light: DirectLight,
    stats: &mut UpdateStats,
) -> Probe {
    let mut p = Probe::empty(key);
    p.unresolved_rays = 0;
    let origin = key.position(config.spacing);
    for index in 0..RAYS_PER_PROBE {
        let (direction, solid_angle) = ray_sample(index);
        stats.primary_rays += 1;
        let ray = Ray {
            origin,
            direction,
            min_distance: config.ray_bias,
            max_distance: config.trace_distance,
        };
        let mut resolved = true;
        // Certified sky is unobstructed beyond the trace limit too. Its finite
        // moment must at least cover every adjacent-probe interpolation query.
        let mut distance = config.trace_distance.max(2.0 * config.spacing);
        let radiance = match world.trace(ray) {
            Trace::KnownSky => {
                let value = world.sky_radiance(direction);
                if valid_rgb(value) {
                    value
                } else {
                    resolved = false;
                    Vec3::ZERO
                }
            }
            Trace::Unknown => {
                resolved = false;
                Vec3::ZERO
            }
            Trace::Hit(hit) => {
                if !hit.distance.is_finite()
                    || hit.distance < ray.min_distance
                    || hit.distance > ray.max_distance
                    || !hit.normal.is_finite()
                    || hit.normal.length_squared() < 1e-12
                    || !valid_rgb(hit.albedo)
                    || hit.albedo.max_element() > 1.0
                    || !valid_rgb(hit.emission)
                {
                    resolved = false;
                    Vec3::ZERO
                } else {
                    distance = hit.distance;
                    let n = hit.normal.normalize();
                    if n.dot(-direction) <= 0.0 {
                        p.inside_geometry = true;
                        resolved = false;
                        Vec3::ZERO
                    } else {
                        let mut value = hit.emission;
                        let cosine = n.dot(light.sun_direction).max(0.0);
                        if cosine > 0.0 && (hit.albedo * light.sun_irradiance).max_element() > 0.0 {
                            let hit_position = origin + direction * hit.distance;
                            stats.sun_rays += 1;
                            match world.trace(Ray {
                                origin: hit_position + n * config.ray_bias,
                                direction: light.sun_direction,
                                min_distance: config.ray_bias,
                                max_distance: config.sun_trace_distance,
                            }) {
                                Trace::KnownSky => {
                                    value += hit.albedo * light.sun_irradiance * (cosine / PI)
                                }
                                Trace::Hit(blocker) => {
                                    if !blocker.distance.is_finite()
                                        || blocker.distance < config.ray_bias
                                        || blocker.distance > config.sun_trace_distance
                                    {
                                        resolved = false;
                                    }
                                }
                                Trace::Unknown => resolved = false,
                            }
                        }
                        value
                    }
                }
            }
        };
        if !resolved || !valid_rgb(radiance) {
            p.unresolved_rays += 1;
            stats.unknown_rays += 1;
            continue;
        }
        p.moments[index].data = [distance, distance * distance, 1.0, 0.0];
        for (coefficient, basis) in p.irradiance_sh.iter_mut().zip(sh_basis(direction)) {
            *coefficient += radiance * (basis * solid_angle);
        }
    }
    for (i, c) in p.irradiance_sh.iter_mut().enumerate() {
        *c *= if i == 0 {
            PI
        } else if i < 4 {
            2.0 * PI / 3.0
        } else {
            PI / 4.0
        };
    }
    p.valid = p.unresolved_rays == 0
        && !p.inside_geometry
        && p.irradiance_sh.iter().all(|c| c.is_finite());
    p
}

/// Exact cube-pixel solid angles, with cubic symmetry: constant radiance
/// integrates to pi*L at every normal after SH cosine convolution.
pub fn ray_sample(index: usize) -> (Vec3, f32) {
    assert!(index < RAYS_PER_PROBE);
    let face = index / (FACE_SIZE * FACE_SIZE);
    let pixel = index % (FACE_SIZE * FACE_SIZE);
    let x = pixel % FACE_SIZE;
    let y = pixel / FACE_SIZE;
    let u = 2.0 * (x as f32 + 0.5) / FACE_SIZE as f32 - 1.0;
    let v = 2.0 * (y as f32 + 0.5) / FACE_SIZE as f32 - 1.0;
    let d = match face {
        0 => Vec3::new(1.0, v, -u),
        1 => Vec3::new(-1.0, v, u),
        2 => Vec3::new(u, 1.0, -v),
        3 => Vec3::new(u, -1.0, v),
        4 => Vec3::new(u, v, 1.0),
        _ => Vec3::new(-u, v, -1.0),
    }
    .normalize();
    let edge = |i: usize| 2.0 * i as f64 / FACE_SIZE as f64 - 1.0;
    let area = |a: f64, b: f64| (a * b).atan2((a * a + b * b + 1.0).sqrt());
    let (u0, u1, v0, v1) = (edge(x), edge(x + 1), edge(y), edge(y + 1));
    let omega = area(u1, v1) - area(u0, v1) - area(u1, v0) + area(u0, v0);
    (d, omega as f32)
}

fn sh_basis(n: Vec3) -> [f32; 9] {
    [
        0.2820948,
        0.48860252 * n.y,
        0.48860252 * n.z,
        0.48860252 * n.x,
        1.0925485 * n.x * n.y,
        1.0925485 * n.y * n.z,
        0.31539157 * (3.0 * n.z * n.z - 1.0),
        1.0925485 * n.x * n.z,
        0.54627424 * (n.x * n.x - n.y * n.y),
    ]
}

fn interpolate_moments(moments: &[GpuMoment; RAYS_PER_PROBE], d: Vec3) -> [f32; 3] {
    let a = d.abs();
    let (face, u, v) = if a.x >= a.y && a.x >= a.z {
        if d.x >= 0.0 {
            (0, -d.z / a.x, d.y / a.x)
        } else {
            (1, d.z / a.x, d.y / a.x)
        }
    } else if a.y >= a.z {
        if d.y >= 0.0 {
            (2, d.x / a.y, -d.z / a.y)
        } else {
            (3, d.x / a.y, d.z / a.y)
        }
    } else if d.z >= 0.0 {
        (4, d.x / a.z, d.y / a.z)
    } else {
        (5, -d.x / a.z, d.y / a.z)
    };
    let x = ((u + 1.0) * 0.5 * FACE_SIZE as f32 - 0.5).clamp(0.0, (FACE_SIZE - 1) as f32);
    let y = ((v + 1.0) * 0.5 * FACE_SIZE as f32 - 0.5).clamp(0.0, (FACE_SIZE - 1) as f32);
    let (ix, iy) = (x.floor() as usize, y.floor() as usize);
    let (tx, ty) = (x - ix as f32, y - iy as f32);
    let mut result = [0.0; 3];
    for dy in 0..=1 {
        for dx in 0..=1 {
            let w = (if dx == 0 { 1.0 - tx } else { tx }) * (if dy == 0 { 1.0 - ty } else { ty });
            let q = moments[face * FACE_SIZE * FACE_SIZE
                + (iy + dy).min(FACE_SIZE - 1) * FACE_SIZE
                + (ix + dx).min(FACE_SIZE - 1)]
            .data;
            for i in 0..3 {
                result[i] += q[i] * w;
            }
        }
    }
    result
}

fn moment_visibility(moment: [f32; 3], distance: f32, bias: f32) -> f32 {
    if moment[2] < 0.999 {
        return 0.0;
    }
    if distance <= moment[0] + bias {
        return 1.0;
    }
    let squared_mean = moment[0] * moment[0];
    let measured_variance = moment[1] - squared_mean;
    // Bilinear sums of constant moments can leave a few positive ULPs. Without
    // this arithmetic error bound, normalizing tiny visibility weights restores
    // full light behind a perfectly constant-distance occluder.
    let rounding = 8.0 * f32::EPSILON * moment[1].abs().max(squared_mean);
    let variance = if measured_variance > rounding {
        measured_variance
    } else {
        0.0
    };
    let delta = distance - moment[0];
    // One-sided variance bound, with no arbitrary minimum-light floor.
    variance / (variance + delta * delta).max(1e-12)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Sky(Vec3);
    impl Transport for Sky {
        fn trace(&self, _: Ray) -> Trace {
            Trace::KnownSky
        }
        fn sky_radiance(&self, _: Vec3) -> Vec3 {
            self.0
        }
    }
    fn light() -> DirectLight {
        DirectLight {
            sun_direction: Vec3::Y,
            sun_irradiance: Vec3::splat(10.0),
        }
    }
    fn one() -> RadianceCache {
        RadianceCache::new(Config {
            radius_cells: [0; 3],
            ..Default::default()
        })
        .unwrap()
    }
    #[test]
    fn constant_radiance_has_exact_cosine_energy_for_every_normal() {
        let mut c = one();
        c.set_views(&[Vec3::ZERO]).unwrap();
        c.update(
            &Sky(Vec3::new(0.25, 1.0, 3.0)),
            light(),
            Revision::default(),
        )
        .unwrap();
        let p = c.probes().next().unwrap();
        let omega: f32 = (0..RAYS_PER_PROBE).map(|i| ray_sample(i).1).sum();
        assert!((omega - 4.0 * PI).abs() < 1e-5);
        for n in [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::new(1.0, 2.0, 3.0).normalize(),
        ] {
            let expected = Vec3::new(0.25, 1.0, 3.0) * PI;
            assert!((p.irradiance(n) - expected).abs().max_element() < 2e-5);
        }
    }

    struct Floor {
        albedo: Vec3,
        blocked: bool,
        unknown_sun: bool,
    }
    impl Transport for Floor {
        fn trace(&self, ray: Ray) -> Trace {
            if ray.direction.y < 0.0 {
                let t = -ray.origin.y / ray.direction.y;
                if t >= ray.min_distance && t <= ray.max_distance {
                    return Trace::Hit(SurfaceHit {
                        distance: t,
                        normal: Vec3::Y,
                        albedo: self.albedo,
                        emission: Vec3::ZERO,
                    });
                }
            }
            if ray.origin.y < 0.01 {
                if self.unknown_sun {
                    return Trace::Unknown;
                }
                if self.blocked {
                    return Trace::Hit(SurfaceHit {
                        distance: 1.0,
                        normal: Vec3::NEG_Y,
                        albedo: Vec3::ZERO,
                        emission: Vec3::ZERO,
                    });
                }
            }
            Trace::KnownSky
        }
        fn sky_radiance(&self, _: Vec3) -> Vec3 {
            Vec3::ZERO
        }
    }
    #[test]
    fn actual_diffuse_surface_transports_color_and_shared_sun_blocker_stops_it() {
        let mut c = one();
        c.set_views(&[Vec3::Y * 4.0]).unwrap();
        c.update(
            &Floor {
                albedo: Vec3::X,
                blocked: false,
                unknown_sun: false,
            },
            light(),
            Revision::default(),
        )
        .unwrap();
        let e = c.probes().next().unwrap().irradiance(Vec3::NEG_Y);
        // Infinite Lambertian floor: Lo=10/pi; hemisphere integration E=10.
        assert!((e.x - 10.0).abs() < 0.5, "{e}");
        assert_eq!(e.y, 0.0);
        assert_eq!(e.z, 0.0);
        c.update(
            &Floor {
                albedo: Vec3::X,
                blocked: true,
                unknown_sun: false,
            },
            light(),
            Revision {
                geometry: 1,
                lighting: 0,
            },
        )
        .unwrap();
        assert_eq!(
            c.probes().next().unwrap().irradiance(Vec3::NEG_Y),
            Vec3::ZERO
        );
    }

    #[test]
    fn unresolved_primary_or_sun_visibility_never_becomes_environment_light() {
        struct Missing;
        impl Transport for Missing {
            fn trace(&self, _: Ray) -> Trace {
                Trace::Unknown
            }
            fn sky_radiance(&self, _: Vec3) -> Vec3 {
                Vec3::splat(1e8)
            }
        }
        let mut c = one();
        c.set_views(&[Vec3::Y * 4.0]).unwrap();
        let stats = c.update(&Missing, light(), Revision::default()).unwrap();
        assert_eq!(stats.unknown_rays, RAYS_PER_PROBE);
        assert_eq!(c.sample(Vec3::Y * 4.0, Vec3::Y).known_weight, 0.0);
        c.update(
            &Floor {
                albedo: Vec3::ONE,
                blocked: false,
                unknown_sun: true,
            },
            light(),
            Revision {
                geometry: 1,
                lighting: 0,
            },
        )
        .unwrap();
        assert!(!c.probes().next().unwrap().valid);
        let (gpu, _) = c.packed();
        assert_eq!(gpu[0].meta[0], 0);
        assert_eq!(gpu[0].irradiance_sh, [[0.0; 4]; 9]);
    }

    #[test]
    fn four_views_share_residency_update_budget_and_stable_world_identity() {
        let mut c = RadianceCache::new(Config {
            capacity: 16,
            probes_per_update: 2,
            radius_cells: [1; 3],
            ..Default::default()
        })
        .unwrap();
        let a = c.set_views(&[Vec3::ZERO; 4]).unwrap();
        assert_eq!(a.requested, 27);
        assert_eq!(a.resident, 16);
        assert_eq!(a.dropped, 11);
        let keys: Vec<_> = c.probes().map(|p| p.key).collect();
        let b = c.set_views(&[Vec3::splat(0.1)]).unwrap();
        assert_eq!(b.retained, 16);
        assert_eq!(keys, c.probes().map(|p| p.key).collect::<Vec<_>>());
        let u = c
            .update(&Sky(Vec3::ONE), light(), Revision::default())
            .unwrap();
        assert_eq!(u.updated_probes, 2);
        assert_eq!(u.primary_rays, 2 * RAYS_PER_PROBE);
        assert_eq!(u.sun_rays, 0);
        for _ in 0..7 {
            c.update(&Sky(Vec3::ONE), light(), Revision::default())
                .unwrap();
        }
        assert_eq!(c.probes().filter(|p| p.valid).count(), 16);
        // Geometry changes invalidate all previous energy before the budgeted
        // refill. Unresolved old lighting cannot remain silently valid.
        let u = c
            .update(
                &Sky(Vec3::ZERO),
                light(),
                Revision {
                    geometry: 1,
                    lighting: 0,
                },
            )
            .unwrap();
        assert_eq!(u.valid_probes, 2);
        let valid_before: Vec<_> = c.probes().filter(|p| p.valid).map(|p| p.key).collect();
        c.update(
            &Sky(Vec3::ZERO),
            light(),
            Revision {
                geometry: 1,
                lighting: 1,
            },
        )
        .unwrap();
        assert!(
            c.probes()
                .filter(|p| p.valid)
                .all(|p| !valid_before.contains(&p.key)),
            "a new light revision must not starve other world cells"
        );
    }

    #[test]
    fn visibility_moments_do_not_add_a_light_floor_behind_known_blocker() {
        assert_eq!(moment_visibility([2.0, 4.0, 1.0], 1.0, 0.002), 1.0);
        assert_eq!(moment_visibility([2.0, 4.0, 1.0], 3.0, 0.002), 0.0);
        assert_eq!(moment_visibility([2.0, 8.0, 0.0], 1.0, 0.002), 0.0);
        let v = moment_visibility([2.0, 5.0, 1.0], 3.0, 0.002);
        assert!((v - 0.5).abs() < 1e-6);
        assert_eq!(std::mem::size_of::<GpuProbe>(), 176);
        assert_eq!(std::mem::size_of::<GpuMoment>(), 16);
    }

    #[test]
    fn filtering_constant_distance_does_not_invent_visibility_variance() {
        let moments = [GpuMoment {
            data: [2.0, 4.0, 1.0, 0.0],
        }; RAYS_PER_PROBE];
        for y in 0..31 {
            for x in 0..31 {
                let direction =
                    Vec3::new(1.0, -0.99 + y as f32 * 0.063, -0.99 + x as f32 * 0.063).normalize();
                let filtered = interpolate_moments(&moments, direction);
                assert_eq!(moment_visibility(filtered, 3.0, 0.002), 0.0, "{filtered:?}");
            }
        }
    }

    #[test]
    fn probes_inside_closed_geometry_are_invalid_and_never_export_bounce() {
        struct Enclosure;
        impl Transport for Enclosure {
            fn trace(&self, ray: Ray) -> Trace {
                // A sphere centred on this probe. Its outward normals face
                // along outgoing rays, unlike valid front-facing receivers.
                Trace::Hit(SurfaceHit {
                    distance: 2.0,
                    normal: ray.direction,
                    albedo: Vec3::ONE,
                    emission: Vec3::splat(100.0),
                })
            }
            fn sky_radiance(&self, _: Vec3) -> Vec3 {
                Vec3::splat(100.0)
            }
        }
        let mut c = one();
        c.set_views(&[Vec3::ZERO]).unwrap();
        c.update(&Enclosure, light(), Revision::default()).unwrap();
        let p = c.probes().next().unwrap();
        assert!(p.inside_geometry);
        assert!(!p.valid);
        assert_eq!(p.unresolved_rays, RAYS_PER_PROBE as u32);
        assert_eq!(c.sample(Vec3::ZERO, Vec3::Y).irradiance, Vec3::ZERO);
        assert_eq!(c.packed().0[0].meta[0], 0);
    }

    #[test]
    fn malformed_configuration_and_unrepresentable_world_keys_fail_closed() {
        for value in [f32::INFINITY, f32::NAN, -1.0, 1.0e30] {
            assert!(
                RadianceCache::new(Config {
                    trace_distance: value,
                    ..Default::default()
                })
                .is_err()
            );
        }
        let mut c = one();
        assert!(c.set_views(&[Vec3::splat(i32::MAX as f32 * 4.0)]).is_err());
        assert!(c.set_views(&[Vec3::splat(f32::NAN)]).is_err());
        assert!(c.probes().next().is_none());
    }
}
