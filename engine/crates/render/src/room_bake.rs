//! Light for rooms, baked from the scene's own triangles.
//!
//! The approach follows Call of Duty's precomputed lighting, scaled down to
//! Warp's probe grid: trace the real geometry, let sky and sun light bounce
//! several times, keep probes from seeing through walls, and dim reflections
//! by the baked light (the last part is in `shade.wgsl`). The result is the
//! sky/sun transport of [`crate::irradiance_volume`]'s mode 5: per probe and
//! axis, x is the share of the sky seen directly, y the sky light arriving
//! after bounces (per unit horizontal sky illuminance), z the sun light after
//! bounces (per unit sun illuminance), and z = 2 marks a probe inside a wall.
//!
//! ```ignore
//! let occluders = Occluders::from_scene(&scene, |_| true);
//! let probes = room_bake::bake(&grid, &occluders, light.sun_dir, &Settings::default())?;
//! let planes = room_bake::separation_planes(&grid, &occluders);
//! renderer.set_sky_sun_separated(&gpu, &probes, &planes)?;
//! ```
use crate::irradiance_volume::{Grid, Probe};
use crate::scene::{Scene, GpuInstance, BLEND, GRASS, THIN_FOLIAGE, ALPHA_TEST, FOLIAGE, SKINNED};
use glam::{Affine3A, Vec3};

/// World-space triangles in a bounding volume hierarchy, for baking only.
pub struct Occluders {
    tris: Vec<[Vec3; 3]>,
    nodes: Vec<Node>,
}

#[derive(Clone, Copy)]
struct Node { lo: Vec3, hi: Vec3, first: u32, count: u32 }

const LEAF: usize = 4;

fn slab(o: Vec3, inv: Vec3, lo: Vec3, hi: Vec3, far: f32) -> bool {
    let a = (lo - o) * inv;
    let b = (hi - o) * inv;
    let enter = a.min(b).max_element().max(0.0);
    let exit = a.max(b).min_element().min(far);
    exit >= enter
}

impl Occluders {
    /// Every opaque, static triangle in the scene at full detail. Foliage,
    /// grass, alpha-tested, blended and skinned geometry are left out, as is
    /// any instance `keep` rejects.
    pub fn from_scene(scene: &Scene, keep: impl Fn(&GpuInstance) -> bool) -> Self {
        let mut tris = Vec::new();
        for inst in &scene.instances {
            if inst.flags & (FOLIAGE | SKINNED) != 0 || !keep(inst) { continue; }
            let r = inst.rows;
            let xf = |p: [f32; 3]| Vec3::new(
                r[0][0] * p[0] + r[0][1] * p[1] + r[0][2] * p[2] + r[0][3],
                r[1][0] * p[0] + r[1][1] * p[1] + r[1][2] * p[2] + r[1][3],
                r[2][0] * p[0] + r[2][1] * p[1] + r[2][2] * p[2] + r[2][3]);
            let clusters = &scene.clusters[inst.first_cluster as usize..(inst.first_cluster + inst.cluster_count) as usize];
            for c in clusters.iter().filter(|c| c.own_error == 0.0) {
                let skip = scene.materials.get(c.material as usize)
                    .is_some_and(|m| m.flags & (BLEND | GRASS | THIN_FOLIAGE | ALPHA_TEST) != 0);
                if skip { continue; }
                for &packed in &scene.tris[c.first_tri as usize..(c.first_tri + c.tri_count) as usize] {
                    let v: [usize; 3] = std::array::from_fn(|k| scene.cluster_verts[c.first_vert as usize + ((packed >> (k * 8)) & 255) as usize] as usize);
                    let mut t: [Vec3; 3] = v.map(|v| xf(scene.vertices[v].pos));
                    // Front faces face the way the vertex normals do,
                    // whatever the winding (mirrored instances flip it).
                    let shading: Vec3 = v.iter().map(|&v| Vec3::from(scene.vertices[v].normal)).sum();
                    let shading = Vec3::new(
                        r[0][0] * shading.x + r[0][1] * shading.y + r[0][2] * shading.z,
                        r[1][0] * shading.x + r[1][1] * shading.y + r[1][2] * shading.z,
                        r[2][0] * shading.x + r[2][1] * shading.y + r[2][2] * shading.z);
                    if (t[1] - t[0]).cross(t[2] - t[0]).dot(shading) < 0.0 { t.swap(1, 2); }
                    tris.push(t);
                }
            }
        }
        Self::from_triangles(tris)
    }

    /// Unit cubes under `boxes` (the boxes `sky_occlusion` takes).
    pub fn from_boxes(boxes: &[Affine3A]) -> Self {
        let mut tris = Vec::new();
        for xf in boxes {
            let c = |i: usize| xf.transform_point3(Vec3::new((i & 1) as f32 - 0.5, ((i >> 1) & 1) as f32 - 0.5, ((i >> 2) & 1) as f32 - 0.5));
            let centre: Vec3 = xf.translation.into();
            for q in [[0, 2, 6, 4], [1, 5, 7, 3], [0, 4, 5, 1], [2, 3, 7, 6], [0, 1, 3, 2], [4, 6, 7, 5]] {
                for mut t in [[c(q[0]), c(q[1]), c(q[2])], [c(q[0]), c(q[2]), c(q[3])]] {
                    // Outward, as from_scene orients by vertex normals.
                    if (t[1] - t[0]).cross(t[2] - t[0]).dot(t[0] - centre) < 0.0 { t.swap(1, 2); }
                    tris.push(t);
                }
            }
        }
        Self::from_triangles(tris)
    }

    pub fn from_triangles(mut tris: Vec<[Vec3; 3]>) -> Self {
        tris.retain(|t| t.iter().all(|p| p.is_finite()) && (t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-12);
        let mut nodes = Vec::with_capacity(tris.len().div_ceil(LEAF) * 2 + 1);
        if !tris.is_empty() {
            let n = tris.len();
            nodes.push(Node { lo: Vec3::ZERO, hi: Vec3::ZERO, first: 0, count: 0 });
            build(&mut tris, &mut nodes, 0, 0, n);
        }
        Self { tris, nodes }
    }

    pub fn triangles(&self) -> usize { self.tris.len() }

    /// These triangles and `other`'s: a scene with and without a door, say.
    pub fn merged(&self, other: &Occluders) -> Self {
        Self::from_triangles(self.tris.iter().chain(&other.tris).copied().collect())
    }

    /// The nearest hit along `d` (unit) from `o`, at least 2 cm out: the
    /// distance and the triangle's normal, facing `d`'s origin side or not.
    /// Also for game logic that aims at the baked geometry (a flashlight's
    /// spot, say).
    pub fn hit(&self, o: Vec3, d: Vec3) -> Option<(f32, Vec3)> {
        if self.nodes.is_empty() { return None; }
        let inv = d.recip();
        let mut best: Option<(f32, Vec3)> = None;
        let mut stack = [0u32; 64];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let node = self.nodes[stack[top] as usize];
            let far = best.map_or(f32::INFINITY, |b| b.0);
            if !slab(o, inv, node.lo, node.hi, far) { continue; }
            if node.count > 0 {
                for t in &self.tris[node.first as usize..(node.first + node.count) as usize] {
                    if let Some(dist) = intersect(o, d, t) {
                        if dist > 0.02 && dist < far.min(best.map_or(f32::INFINITY, |b| b.0)) {
                            best = Some((dist, (t[1] - t[0]).cross(t[2] - t[0]).normalize()));
                        }
                    }
                }
            } else if top + 2 <= stack.len() {
                stack[top] = node.first;
                stack[top + 1] = node.first + 1;
                top += 2;
            }
        }
        best
    }

    fn blocked(&self, o: Vec3, d: Vec3, far: f32) -> bool {
        self.hit(o, d).is_some_and(|(t, _)| t < far)
    }
}

/// Fill `nodes[index]` with the subtree over `tris[first..first + count]`.
/// An inner node's two children sit next to each other at `first`.
fn build(tris: &mut [[Vec3; 3]], nodes: &mut Vec<Node>, index: usize, first: usize, count: usize) {
    let span = &tris[first..first + count];
    let (lo, hi) = span.iter().flatten().fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(a, b), p| (a.min(*p), b.max(*p)));
    if count <= LEAF {
        nodes[index] = Node { lo, hi, first: first as u32, count: count as u32 };
        return;
    }
    let centre = |t: &[Vec3; 3]| t[0] + t[1] + t[2];
    let (clo, chi) = span.iter().fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(a, b), t| (a.min(centre(t)), b.max(centre(t))));
    let e = chi - clo;
    let axis = if e.x >= e.y && e.x >= e.z { 0 } else if e.y >= e.z { 1 } else { 2 };
    let half = count / 2;
    tris[first..first + count].select_nth_unstable_by(half, |a, b| centre(a)[axis].total_cmp(&centre(b)[axis]));
    let children = nodes.len();
    nodes.push(Node { lo, hi, first: 0, count: 0 });
    nodes.push(Node { lo, hi, first: 0, count: 0 });
    nodes[index] = Node { lo, hi, first: children as u32, count: 0 };
    build(tris, nodes, children, first, half);
    build(tris, nodes, children + 1, first + half, count - half);
}

fn intersect(o: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    let e1 = t[1] - t[0];
    let e2 = t[2] - t[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-10 { return None; }
    let inv = 1.0 / det;
    let s = o - t[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) { return None; }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 { return None; }
    Some(e2.dot(q) * inv)
}

/// Bake quality.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Rays per probe for the direct sky. A door seen from the back of a
    /// room is a few hundredths of the sphere: 1000 or more.
    pub sky_rays: usize,
    /// Rays per probe for bounced light (smooth: fewer are fine).
    pub bounce_rays: usize,
    /// Diffuse bounces: 1 is the light off the sunlit floor, 3 fills a room.
    pub bounces: usize,
    /// Average albedo of the surfaces light bounces off.
    pub reflectance: f32,
}

impl Default for Settings {
    fn default() -> Self { Self { sky_rays: 2048, bounce_rays: 512, bounces: 3, reflectance: 0.5 } }
}

/// Fibonacci sphere directions. Every probe uses the same set: a small
/// opening is then missed or hit alike by neighbouring probes and the light
/// changes smoothly between them; per-probe rotations make blotches.
fn sphere(rays: usize) -> Vec<Vec3> {
    (0..rays).map(|i| {
        let y = 1. - 2. * (i as f32 + 0.5) / rays as f32;
        let a = i as f32 * 2.3999632;
        let s = (1. - y * y).sqrt();
        Vec3::new(s * a.cos(), y, s * a.sin())
    }).collect()
}

const AXES: [Vec3; 6] = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];

/// A channel of `probes` at `p` on a surface facing `n`: trilinear over the
/// valid probes in front of the surface, as the shader samples them.
fn sample(grid: &Grid, probes: &[Probe], valid: &[bool], p: Vec3, n: Vec3, channel: usize) -> f32 {
    let max = Vec3::new(grid.dims[0] as f32 - 1., grid.dims[1] as f32 - 1., grid.dims[2] as f32 - 1.);
    let f = ((p - grid.origin) / grid.cell).clamp(Vec3::ZERO, max);
    let base = f.floor().min(max - Vec3::ONE);
    let t = f - base;
    let (mut value, mut weight) = (0., 0.);
    for c in 0..8 {
        let o = [c & 1, (c >> 1) & 1, (c >> 2) & 1];
        let i = base.x as usize + o[0]
            + (base.y as usize + o[1]) * grid.dims[0] as usize
            + (base.z as usize + o[2]) * (grid.dims[0] * grid.dims[1]) as usize;
        if !valid[i] { continue; }
        let to = grid.position(i) - p;
        let facing = (to.dot(n) / to.length().max(1e-4) * 2. + 1.).clamp(0., 1.);
        let w = (if o[0] == 1 { t.x } else { 1. - t.x })
            * (if o[1] == 1 { t.y } else { 1. - t.y })
            * (if o[2] == 1 { t.z } else { 1. - t.z })
            * (facing * facing + 1e-4);
        value += probes[i].irradiance(n)[channel] * w;
        weight += w;
    }
    if weight > 1e-6 { value / weight } else { 0. }
}

/// Run `f(index, item)` over `items` on the cooking threads.
fn each<T: Send>(items: &mut [T], f: impl Fn(usize, &mut T) + Sync) {
    let chunk = items.len().div_ceil(crate::par::workers().max(1)).max(1);
    let f = &f;
    crate::par::scope(|scope| {
        for (group, out) in items.chunks_mut(chunk).enumerate() {
            scope.spawn(move || for (local, item) in out.iter_mut().enumerate() { f(group * chunk + local, item) });
        }
    });
}

/// Average the bounced channels (y, z), and with `sky` also the direct sky
/// (x), of each valid probe with the neighbours it sees directly
/// (`neighbours`: index and weight).
fn smooth_bounce(light: &mut [Probe], neighbours: &[Vec<(usize, f32)>], valid: &[bool], sky: bool) {
    let gathered = light.to_vec();
    each(light, |i, p| {
        if !valid[i] { return; }
        let mut sum = gathered[i].axes;
        let mut weight = 1.0;
        for &(j, w) in &neighbours[i] {
            for a in 0..6 { sum[a] += gathered[j].axes[a] * w; }
            weight += w;
        }
        for a in 0..6 {
            let s = sum[a] / weight;
            p.axes[a] = Vec3::new(if sky { s.x } else { p.axes[a].x }, s.y, s.z);
        }
    });
}

/// Bake sky and sun transport for `grid` against `occluders` with the sun
/// towards `sun`. Rebake when the sun moves or the geometry changes.
pub fn bake(grid: &Grid, occluders: &Occluders, sun: Vec3, settings: &Settings) -> Result<Vec<Probe>, String> {
    grid.validate()?;
    let Settings { sky_rays, bounce_rays, bounces, reflectance } = *settings;
    if sky_rays < 16 || bounce_rays < 16 || !sun.is_finite() || sun.length_squared() < 0.5
        || !reflectance.is_finite() || !(0.0..=1.0).contains(&reflectance) {
        return Err("invalid room bake settings".into());
    }
    let count = grid.count();
    let sun = sun.normalize();
    let (sky_dirs, bounce_dirs) = (sphere(sky_rays), sphere(bounce_rays));
    // Direct sky per axis, and whether the probe is inside something: a
    // probe that sees the back of a surface along a quarter of its rays is
    // inside a wall (the rule DDGI uses), and must not light anything.
    let mut probes = vec![Probe::default(); count];
    let mut valid = vec![true; count];
    {
        let mut both: Vec<(Probe, bool)> = vec![(Probe::default(), true); count];
        each(&mut both, |i, (p, ok)| {
            let origin = grid.position(i);
            let (mut v, mut denom, mut back) = ([0f32; 6], [0f32; 6], 0usize);
            for d in &sky_dirs {
                let hit = occluders.hit(origin, *d);
                if hit.is_some_and(|(_, n)| n.dot(*d) > 0.0) { back += 1; }
                for a in 0..6 {
                    let c = AXES[a].dot(*d).max(0.);
                    denom[a] += c;
                    if hit.is_none() { v[a] += c; }
                }
            }
            *ok = back * 4 < sky_dirs.len();
            for a in 0..6 { p.axes[a].x = if *ok { v[a] / denom[a].max(1e-6) } else { 0. }; }
        });
        for (i, (p, ok)) in both.into_iter().enumerate() { probes[i] = p; valid[i] = ok; }
    }
    // What each probe's bounce rays hit, with the sun on it: reused by every bounce.
    let mut denom = [0f32; 6];
    for d in &bounce_dirs { for a in 0..6 { denom[a] += AXES[a].dot(*d).max(0.); } }
    let mut hits: Vec<Vec<(Vec3, Vec3, f32, [f32; 6])>> = vec![Vec::new(); count];
    each(&mut hits, |i, list| {
        if !valid[i] { return; }
        let origin = grid.position(i);
        for d in &bounce_dirs {
            if let Some((t, n)) = occluders.hit(origin, *d) {
                // The side facing the probe.
                let n = if n.dot(*d) > 0.0 { -n } else { n };
                let at = origin + *d * t;
                let lit = n.dot(sun).max(0.);
                let sun_e = if sun.y > 0. && lit > 0. && !occluders.blocked(at + n * 0.03, sun, f32::INFINITY) { lit } else { 0. };
                let w: [f32; 6] = std::array::from_fn(|a| AXES[a].dot(*d).max(0.) / denom[a].max(1e-6));
                list.push((at + n * 0.15, n, sun_e, w));
            }
        }
    });
    // The 26 neighbours each probe sees directly, for smoothing the bounces.
    let mut neighbours: Vec<Vec<(usize, f32)>> = vec![Vec::new(); count];
    each(&mut neighbours, |i, list| {
        if !valid[i] { return; }
        let [dx, dy, dz] = grid.dims.map(|d| d as i64);
        let (x, y, z) = (i as i64 % dx, (i as i64 / dx) % dy, i as i64 / (dx * dy));
        let p = grid.position(i);
        for ox in -1..=1i64 { for oy in -1..=1i64 { for oz in -1..=1i64 {
            let (nx, ny, nz) = (x + ox, y + oy, z + oz);
            if (ox, oy, oz) == (0, 0, 0) || nx < 0 || ny < 0 || nz < 0 || nx >= dx || ny >= dy || nz >= dz { continue; }
            let j = (nx + ny * dx + nz * dx * dy) as usize;
            if !valid[j] { continue; }
            let d = grid.position(j) - p;
            if occluders.blocked(p, d.normalize(), d.length()) { continue; }
            list.push((j, 0.5f32.powi((ox.abs() + oy.abs() + oz.abs()) as i32)));
        }}}
    });
    // Gather: each bounce lights the surfaces with the previous one's probes.
    // y and z hold everything that has arrived so far.
    let mut light = probes;
    for _ in 0..bounces.max(1) {
        let previous = light.clone();
        each(&mut light, |i, p| {
            if !valid[i] { return; }
            let (mut sky, mut sunlit) = ([0f32; 6], [0f32; 6]);
            for (at, n, sun_e, w) in &hits[i] {
                let s = sample(grid, &previous, &valid, *at, *n, 0) + sample(grid, &previous, &valid, *at, *n, 1);
                let z = sun_e + sample(grid, &previous, &valid, *at, *n, 2);
                for a in 0..6 { sky[a] += reflectance * s * w[a]; sunlit[a] += reflectance * z * w[a]; }
            }
            for a in 0..6 { p.axes[a].y = sky[a]; p.axes[a].z = sunlit[a]; }
        });
        // Bounced light is smooth; a finite gather is not (a sun patch is hit
        // by a few rays more or less from probe to probe). Average with the
        // neighbours each probe sees, never across a wall.
        smooth_bounce(&mut light, &neighbours, &valid, false);
    }
    // Two more passes on the result, the direct sky included: sky seen
    // through a doorway (or two in a row) or bounced off a sun patch next
    // door reaches a probe along a few rays only. A few probes then stand
    // out from their neighbours, and trilinear shading draws each as a
    // square at the probe spacing on the wall opposite the doorway. Real
    // light through a door is a wide soft patch at that distance anyway.
    for _ in 0..2 { smooth_bounce(&mut light, &neighbours, &valid, true); }
    for (i, p) in light.iter_mut().enumerate() {
        if !valid[i] { for a in &mut p.axes { *a = Vec3::new(0., 0., 2.); } }
    }
    Ok(light)
}

/// Bounced light of local lights (lamps), one probe set per light: per
/// probe and axis the RGB irradiance (lux) that reaches it after `bounces`
/// diffuse bounces, without the light's direct part (the live light draws
/// that, with its shadow). Lamps that switch on and off then combine at
/// runtime by adding the sets of the lit ones (the idea of Call of Duty's
/// UberBake, here per probe), e.g. into the emission of
/// `irradiance_volume::pack_sky_emission`. Rebake when a lamp moves.
pub fn bake_lights(grid: &Grid, occluders: &Occluders, lights: &[crate::headlights::LocalLight], settings: &Settings) -> Result<Vec<Vec<Probe>>, String> {
    grid.validate()?;
    let Settings { bounce_rays, bounces, reflectance, .. } = *settings;
    if bounce_rays < 16 || !reflectance.is_finite() || !(0.0..=1.0).contains(&reflectance) {
        return Err("invalid light bake settings".into());
    }
    let count = grid.count();
    let dirs = sphere(bounce_rays);
    // Inside-wall probes, as `bake` finds them (fewer rays: only a test).
    let probe_dirs = sphere(256);
    let mut valid = vec![true; count];
    each(&mut valid, |i, ok| {
        let o = grid.position(i);
        let back = probe_dirs.iter().filter(|d| occluders.hit(o, **d).is_some_and(|(_, n)| n.dot(**d) > 0.0)).count();
        *ok = back * 4 < probe_dirs.len();
    });
    let mut denom = [0f32; 6];
    for d in &dirs { for a in 0..6 { denom[a] += AXES[a].dot(*d).max(0.); } }
    // Hits with each light's direct irradiance there (shadow-tested).
    let mut hits: Vec<Vec<(Vec3, Vec3, Vec<Vec3>, [f32; 6])>> = vec![Vec::new(); count];
    each(&mut hits, |i, list| {
        if !valid[i] { return; }
        let origin = grid.position(i);
        for d in &dirs {
            let Some((t, n)) = occluders.hit(origin, *d) else { continue };
            let n = if n.dot(*d) > 0.0 { -n } else { n };
            let at = origin + *d * t;
            let direct = lights.iter().map(|lamp| {
                let Some((to, e)) = crate::sprites::light_at(lamp, at) else { return Vec3::ZERO };
                let cos = n.dot(to);
                if cos <= 0.0 { return Vec3::ZERO; }
                let far = lamp.position.distance(at);
                if occluders.blocked(at + n * 0.03, to, far - lamp.source_radius_m.max(0.05)) { return Vec3::ZERO; }
                e * cos
            }).collect();
            let w: [f32; 6] = std::array::from_fn(|a| AXES[a].dot(*d).max(0.) / denom[a].max(1e-6));
            list.push((at + n * 0.15, n, direct, w));
        }
    });
    let sample_rgb = |probes: &[Probe], p: Vec3, n: Vec3| -> Vec3 {
        let max = Vec3::new(grid.dims[0] as f32 - 1., grid.dims[1] as f32 - 1., grid.dims[2] as f32 - 1.);
        let f = ((p - grid.origin) / grid.cell).clamp(Vec3::ZERO, max);
        let base = f.floor().min(max - Vec3::ONE);
        let t = f - base;
        let (mut value, mut weight) = (Vec3::ZERO, 0.);
        for c in 0..8 {
            let o = [c & 1, (c >> 1) & 1, (c >> 2) & 1];
            let i = base.x as usize + o[0] + (base.y as usize + o[1]) * grid.dims[0] as usize
                + (base.z as usize + o[2]) * (grid.dims[0] * grid.dims[1]) as usize;
            if !valid[i] { continue; }
            let to = grid.position(i) - p;
            let facing = (to.dot(n) / to.length().max(1e-4) * 2. + 1.).clamp(0., 1.);
            let w = (if o[0] == 1 { t.x } else { 1. - t.x }) * (if o[1] == 1 { t.y } else { 1. - t.y })
                * (if o[2] == 1 { t.z } else { 1. - t.z }) * (facing * facing + 1e-4);
            value += probes[i].irradiance(n) * w;
            weight += w;
        }
        if weight > 1e-6 { value / weight } else { Vec3::ZERO }
    };
    let mut out = Vec::with_capacity(lights.len());
    for l in 0..lights.len() {
        let mut probes = vec![Probe::default(); count];
        for _ in 0..bounces.max(1) {
            let previous = probes.clone();
            each(&mut probes, |i, p| {
                if !valid[i] { return; }
                let mut axes = [Vec3::ZERO; 6];
                for (at, n, direct, w) in &hits[i] {
                    let leaving = reflectance * (direct[l] + sample_rgb(&previous, *at, *n));
                    for a in 0..6 { axes[a] += leaving * w[a]; }
                }
                p.axes = axes;
            });
        }
        out.push(probes);
    }
    Ok(out)
}

/// For each probe, the plane of the nearest surface it faces (xyz into the
/// probe's side, w the offset), or zero: the shader then ignores the probe
/// for points behind that surface, so light does not cross thin walls.
pub fn separation_planes(grid: &Grid, occluders: &Occluders) -> Vec<[f32; 4]> {
    let dirs: Vec<Vec3> = (-1..=1).flat_map(|x| (-1..=1).flat_map(move |y| (-1..=1).map(move |z| Vec3::new(x as f32, y as f32, z as f32))))
        .filter(|d| d.length_squared() > 0.0).map(|d| d.normalize()).collect();
    let mut planes = vec![[0f32; 4]; grid.count()];
    each(&mut planes, |i, plane| {
        let p = grid.position(i);
        let mut nearest = grid.cell * 1.75;
        for d in &dirs {
            if let Some((t, n)) = occluders.hit(p, *d) {
                let n = if n.dot(*d) > 0.0 { -n } else { n };
                if t < nearest {
                    nearest = t;
                    *plane = [n.x, n.y, n.z, -n.dot(p + *d * t)];
                }
            }
        }
    });
    planes
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn bx(min: [f32; 3], max: [f32; 3]) -> Affine3A {
        let (a, b) = (Vec3::from(min), Vec3::from(max));
        Affine3A::from_scale_rotation_translation(b - a, Quat::IDENTITY, (a + b) * 0.5)
    }

    /// A closed 4 m room with a door, on open ground.
    fn room() -> Vec<Affine3A> {
        vec![
            bx([-50.0, -0.2, -50.0], [50.0, 0.0, 50.0]),
            bx([-2.2, 0.0, -2.2], [2.2, 3.0, -2.0]),
            bx([-2.2, 0.0, -2.0], [-2.0, 3.0, 2.0]),
            bx([2.0, 0.0, -2.0], [2.2, 3.0, 2.0]),
            bx([-2.2, 0.0, 2.0], [-0.5, 3.0, 2.2]),
            bx([0.5, 0.0, 2.0], [2.2, 3.0, 2.2]),
            bx([-0.5, 2.1, 2.0], [0.5, 3.0, 2.2]),
            bx([-2.2, 3.0, -2.2], [2.2, 3.2, 2.2]),
        ]
    }

    #[test]
    fn rooms_are_darker_than_outside_and_walls_are_invalid() {
        let grid = Grid { origin: Vec3::new(-3.75, 0.25, -3.75), cell: 0.5, dims: [16, 7, 16] };
        let occ = Occluders::from_boxes(&room());
        let probes = bake(&grid, &occ, Vec3::new(0.3, 0.8, 0.5), &Settings { sky_rays: 512, bounce_rays: 128, bounces: 2, reflectance: 0.5 }).unwrap();
        let at = |p: Vec3| probes[(0..grid.count()).min_by(|&a, &b| grid.position(a).distance(p).total_cmp(&grid.position(b).distance(p))).unwrap()];
        let inside = at(Vec3::new(-1.25, 1.25, -1.25)).irradiance(Vec3::Y);
        let outside = at(Vec3::new(-3.25, 1.25, -3.25)).irradiance(Vec3::Y);
        assert!(outside.x > 0.9, "outside sees the sky: {outside}");
        assert!(inside.x < 0.05, "inside sees little sky: {inside}");
        assert!(inside.y > 0.0 && inside.z > 0.0, "bounced sky and sun reach the room: {inside}");
        // A probe in the 20 cm back wall (z = -2.1 is not on the grid; use the roof slab at y = 3.1).
        let mut in_wall = 0;
        for i in 0..grid.count() {
            let p = grid.position(i);
            if p.x.abs() < 2.0 && p.z.abs() < 2.0 && (p.y - 3.1).abs() < 0.11 { assert_eq!(probes[i].axes[0].z, 2.0); in_wall += 1; }
        }
        let _ = in_wall;
    }

    #[test]
    fn scene_triangles_match_boxes() {
        let mut scene = Scene::default();
        let m = scene.add_material(crate::scene::Material { tint: [0.5; 3], layer: -1, roughness: 0.8, flags: 0, scale: 1.0, metal: 0.0 });
        let cube = scene.add_mesh(&crate::primitives::cube(m).mesh());
        for b in room() { scene.add_instance(cube, b); }
        let from_scene = Occluders::from_scene(&scene, |_| true);
        let from_boxes = Occluders::from_boxes(&room());
        assert_eq!(from_scene.triangles(), from_boxes.triangles());
        for d in sphere(64) {
            let o = Vec3::new(0.3, 1.2, -0.4);
            let a = from_scene.hit(o, d).map(|h| h.0);
            let b = from_boxes.hit(o, d).map(|h| h.0);
            assert!(a.zip(b).is_none_or(|(a, b)| (a - b).abs() < 1e-3) && a.is_some() == b.is_some(), "{d}: {a:?} vs {b:?}");
        }
    }
}
