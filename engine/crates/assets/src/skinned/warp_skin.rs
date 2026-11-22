//! A person's facial skin, grown from their seed over the shared hm08 face:
//! pores and the fine lines between them, lip creases, freckles, moles and
//! uneven redness, as an albedo and a normal map. The relief is a function
//! of the point on the head (not of the UV), so it crosses UV seams unbroken
//! and keeps its true size wherever the unwrap stretches.
use glam::{Vec2, Vec3};
use image::RgbaImage;
use render::humans::BodyVertex;

/// The face surface's material in the cooked body.
pub(super) const FACE: u32 = 11;
/// Metres per unit of the normalized body, for sizing skin features.
pub(super) const METRES: f32 = 1.75;

pub(super) struct Maps {
    pub albedo: RgbaImage,
    pub normal: RgbaImage,
}

/// What varies between people beyond tone, all drawn from the seed.
#[derive(Clone, Copy, Debug)]
pub(super) struct Traits {
    pub seed: u32,
    /// 0 young adult to 1 old: deeper lines, rougher relief, more spots.
    pub age: f32,
    /// 0 none to 1 heavily freckled.
    pub freckles: f32,
    /// How unevenly red the skin is (cheeks, nose, ears).
    pub flush: f32,
    /// 0 dark to 1 fair: fair skin shows redness and freckles more.
    pub fair: f32,
}

impl Traits {
    pub fn new(seed: u32, fair: f32) -> Self {
        let roll = |k: u32| hash(seed ^ k.wrapping_mul(0x9e37_79b9)) as f32 / u32::MAX as f32;
        // Most people have few freckles; a few have many.
        let freckles = (roll(1).powi(3) * (0.35 + 0.65 * fair)).clamp(0., 1.);
        Self { seed, age: roll(2).powf(1.4), freckles, flush: 0.4 + 0.6 * roll(3), fair }
    }

    /// The age in years these traits stand for, and the traits at an age.
    pub fn years(&self) -> f32 {
        18. + 60. * self.age
    }
    pub fn aged(self, years: f32) -> Self {
        Self { age: ((years - 18.) / 60.).clamp(0., 1.), ..self }
    }
}

pub(super) fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

pub(super) fn hash3(c: [i32; 3], seed: u32) -> u32 {
    hash(
        (c[0] as u32).wrapping_mul(0x8da6_b343)
            ^ (c[1] as u32).wrapping_mul(0xd816_3841)
            ^ (c[2] as u32).wrapping_mul(0xcb1a_b31f)
            ^ seed,
    )
}

pub(super) fn unit(h: u32) -> f32 {
    (h & 0xff_ffff) as f32 / 16_777_216.
}

pub(super) fn smooth(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

/// Distances to the nearest and second nearest feature point, and the
/// nearest point's hash, for cells of `cell` metres.
fn worley(p: Vec3, cell: f32, seed: u32) -> (f32, f32, u32) {
    let q = p / cell;
    let base = q.floor();
    let (mut f1, mut f2, mut id) = (f32::MAX, f32::MAX, 0);
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let c = [base.x as i32 + dx, base.y as i32 + dy, base.z as i32 + dz];
                let h = hash3(c, seed);
                let point = Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32)
                    + Vec3::new(unit(h), unit(hash(h)), unit(hash(h ^ 0x5bd1_e995)));
                let d = point.distance_squared(q);
                if d < f1 {
                    (f2, f1, id) = (f1, d, h);
                } else if d < f2 {
                    f2 = d;
                }
            }
        }
    }
    (f1.sqrt() * cell, f2.sqrt() * cell, id)
}

/// Smooth value noise in -1..1 with features of `cell` metres.
pub(super) fn noise(p: Vec3, cell: f32, seed: u32) -> f32 {
    let q = p / cell;
    let base = q.floor();
    let f = q - base;
    let w = f * f * (Vec3::splat(3.) - 2. * f);
    let corner = |dx: i32, dy: i32, dz: i32| {
        unit(hash3([base.x as i32 + dx, base.y as i32 + dy, base.z as i32 + dz], seed)) * 2. - 1.
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x = |dy, dz| lerp(corner(0, dy, dz), corner(1, dy, dz), w.x);
    let y = |dz| lerp(x(0, dz), x(1, dz), w.y);
    lerp(y(0), y(1), w.z)
}

/// Where on the head a texel lies.
#[derive(Clone, Copy)]
struct Texel {
    /// Metres from the point between the eyes: x to the person's left, y
    /// up, z out of the face.
    p: Vec3,
    n: Vec3,
    /// How far the point travels when the eye closes, metres: most at the
    /// rim of the upper lid.
    lid: f32,
}

/// Rasterize a band in global UV coordinates, retaining triangle order at overlaps.
fn unwrap_rows(body: &[BodyVertex], indices: &[u32], eyes: Vec3, size: usize, first: usize, texels: &mut [Option<Texel>]) {
    texels.fill(None);
    let end = first + texels.len() / size;
    let s = size as f32;
    for tri in indices.chunks_exact(3) {
        let v = [0, 1, 2].map(|k| &body[tri[k] as usize]);
        if v[0].material != FACE {
            continue;
        }
        let uv = v.map(|v| Vec2::new(v.uv[0] * s - 0.5, v.uv[1] * s - 0.5));
        let area = (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]);
        if area.abs() < 1e-9 {
            continue;
        }
        let lo = uv[0].min(uv[1]).min(uv[2]).floor().max(Vec2::ZERO);
        let hi = uv[0].max(uv[1]).max(uv[2]).ceil().min(Vec2::splat(s - 1.));
        for y in (lo.y as usize).max(first)..(hi.y as usize + 1).min(end) {
            for x in lo.x as usize..=hi.x as usize {
                let q = Vec2::new(x as f32, y as f32);
                let w0 = (uv[1] - q).perp_dot(uv[2] - q) / area;
                let w1 = (uv[2] - q).perp_dot(uv[0] - q) / area;
                let w2 = 1. - w0 - w1;
                // A texel of slack, so that neighbours across an edge exist.
                let slack = -1.5 / area.abs().sqrt();
                if w0 < slack || w1 < slack || w2 < slack {
                    continue;
                }
                let inside = w0 >= 0. && w1 >= 0. && w2 >= 0.;
                let slot = &mut texels[(y - first) * size + x];
                if slot.is_some() && !inside {
                    continue;
                }
                let p = v[0].position * w0 + v[1].position * w1 + v[2].position * w2;
                let n = v[0].normal * w0 + v[1].normal * w1 + v[2].normal * w2;
                let lid = v[0].lid_closure.length() * w0 + v[1].lid_closure.length() * w1 + v[2].lid_closure.length() * w2;
                *slot = Some(Texel { p: (p - eyes) * METRES, n: n.normalize_or(Vec3::Z), lid: lid * METRES });
            }
        }
    }
}

/// The lash mask is relative to the greatest *sampled* lid displacement.
/// Only rows covered by a moving lid can contain that maximum. Rasterize
/// all triangles in those rows so a UV seam has the same winner as in the bake.
fn lid_travel(body: &[BodyVertex], indices: &[u32], eyes: Vec3, size: usize, scratch: &mut [Option<Texel>]) -> f32 {
    let (mut first, mut end) = (size, 0);
    for tri in indices.chunks_exact(3) {
        let vertices = [0, 1, 2].map(|k| &body[tri[k] as usize]);
        if vertices[0].material == FACE && vertices.iter().any(|v| v.lid_closure != Vec3::ZERO) {
            for v in vertices {
                let y = v.uv[1] * size as f32 - 0.5;
                first = first.min(y.floor() as usize);
                end = end.max((y.ceil() as usize + 1).min(size));
            }
        }
    }
    let rows = scratch.len() / size;
    let mut longest = 1e-6f32;
    for y in (first..end).step_by(rows) {
        let band = &mut scratch[..rows.min(end - y) * size];
        unwrap_rows(body, indices, eyes, size, y, band);
        longest = band.iter().flatten().map(|t| t.lid).fold(longest, f32::max);
    }
    longest
}

/// Relief in metres (negative into the skin) and what the albedo is
/// multiplied by, at one point of the face.
fn skin(t: &Texel, lips: f32, traits: Traits, build: Build) -> (f32, Vec3) {
    let p = t.p;
    let q = build.reference(p);
    let seed = traits.seed;
    // Regions, from where the point lies on the head.
    let front = smooth(-0.02, 0.03, t.n.z);
    let nose = smooth(0.035, 0.012, Vec2::new(p.x, p.y + 0.035).length()) * front;
    let cheeks = smooth(0.075, 0.03, Vec2::new(p.x.abs() - 0.045, p.y + 0.045).length()) * front;
    let forehead = smooth(0.02, 0.05, p.y) * front;
    let eye = smooth(0.030, 0.016, Vec2::new(p.x.abs() - 0.032, p.y * 1.5).length());
    let skin = 1. - lips;

    // The skin's own relief: polygons a millimetre or so across, bounded by
    // fine furrows, with a pore where furrows meet. Coarser and deeper on
    // the nose and cheeks, finer around the eyes; deeper with age.
    let cell = 0.0011 * (1. + 0.5 * nose.max(cheeks) - 0.3 * eye);
    let (f1, f2, _) = worley(p, cell, 17);
    let furrow = 1. - smooth(0., 0.30 * cell, f2 - f1);
    let (g1, _, pore_id) = worley(p, cell * 1.9, 29);
    let open = 0.35 + 0.65 * unit(pore_id);
    let pore = (1. - smooth(0.10 * cell, 0.42 * cell, g1)) * open * (0.35 + 0.65 * nose.max(cheeks));
    let depth = 1. + 0.9 * traits.age;
    let mut height = -(0.000_045 * furrow + 0.000_11 * pore) * depth * skin;
    // Slow unevenness under the fine relief.
    height += 0.000_05 * noise(p, 0.004, 41) * skin;
    height += 0.000_12 * noise(p, 0.012, 43) * skin * (0.4 + 0.6 * traits.age);

    // Lips: creases run across them, top to bottom.
    let crease = noise(Vec3::new(p.x * 9., p.y * 0.8, p.z), 0.004, 53)
        + 0.5 * noise(Vec3::new(p.x * 22., p.y, p.z), 0.004, 59);
    height -= 0.000_16 * smooth(0.05, 0.75, crease.abs()) * lips;

    // Expression lines that age leaves: across the forehead, fanning from
    // the outer eye corners.
    // A face is lined by fifty; after that the lines only get deeper.
    let lined = smooth(0.10, 0.55, traits.age);
    let late = smooth(0.55, 1.0, traits.age).powf(1.3);
    let deep = lined * (1. + 1.6 * late);
    if lined > 0. {
        let a = deep;
        let wave = p.y + 0.0025 * noise(Vec3::new(p.x, 0., 0.), 0.02, 61);
        let line = 1. - smooth(0., 0.35, ((wave / 0.0085).fract() - 0.5).abs() * 2.);
        height -= 0.000_22 * a * line * forehead * smooth(0.06, 0.03, p.x.abs())
            * (0.5 + 0.5 * noise(p, 0.02, 67));
        let corner = Vec2::new(p.x.abs() - 0.052, p.y + 0.002);
        let fan = smooth(0.028, 0.006, corner.length()) * smooth(0., 0.006, corner.x);
        let angle = corner.y.atan2(corner.x.max(1e-5));
        let ray = 1. - smooth(0., 0.4, ((angle / 0.32).fract().abs() - 0.5).abs() * 2.);
        height -= 0.000_18 * a * fan * ray;
    }

    // What age does to the skin itself: folds where the face has moved for
    // decades and where it has come down, slack creasing on the cheeks,
    // and spots where the sun has been.
    let old = smooth(0.30, 1.0, traits.age);
    let mut fold = 0.;
    if lined > 0. {
        // No fold is a ruled line.
        let at = Vec2::new(q.x.abs() + 0.0018 * noise(Vec3::new(0., p.y, p.x.signum()), 0.011, seed ^ 0xf1), q.y);
        let groove = |line: &[Vec2], width: f32| {
            let near = line.windows(2).map(|w| segment(at, w[0], w[1])).fold(f32::MAX, f32::min);
            1. - smooth(0., width, near)
        };
        // From the wing of the nose round the mouth, and down from its
        // corners; under the eye; between the brows.
        let nasolabial = groove(&[Vec2::new(0.018, -0.037), Vec2::new(0.029, -0.060), Vec2::new(0.035, -0.084)], 0.0045);
        let marionette = groove(&[Vec2::new(0.031, -0.084), Vec2::new(0.034, -0.108)], 0.004);
        let trough = groove(&[Vec2::new(0.013, -0.015), Vec2::new(0.027, -0.025), Vec2::new(0.046, -0.021)], 0.0035);
        let frown = groove(&[Vec2::new(0.006, 0.010), Vec2::new(0.008, 0.028)], 0.0022) * unit(hash(seed ^ 0xf0)).max(0.3);
        // Late in life the bag under the eye has a lower edge of its own.
        let bag = groove(&[Vec2::new(0.013, -0.022), Vec2::new(0.028, -0.034), Vec2::new(0.047, -0.028)], 0.003) * late;
        fold = (nasolabial + 0.4 * marionette + 0.55 * trough + 0.5 * frown + 0.5 * bag).min(1.) * front * skin;
        height -= 0.000_55 * deep * fold;
        // And fine lines run down into the upper lip.
        let above = smooth(0.026, 0.016, p.x.abs()) * smooth(-0.050, -0.056, q.y) * smooth(-0.074, -0.066, q.y) * front * (1. - lips);
        let down = 1. - smooth(0., 0.4, (((p.x + 0.0012 * noise(Vec3::new(0., p.y, 0.), 0.006, seed ^ 0xf3)) / 0.0032).fract().abs() - 0.5).abs() * 2.);
        height -= 0.000_20 * late * above * down * skin;
        // Slack skin creases finely across the pull of its weight.
        let slack = noise(Vec3::new(p.x * 0.45, p.y * 1.6, p.z * 0.45), 0.0022, 97);
        let crepe = (1. - smooth(0., 0.45, slack.abs())) * (cheeks + 0.8 * eye + 0.5 * (1. - front)).min(1.);
        height -= 0.000_13 * lined * (1. + 0.6 * late) * crepe * skin;
        fold = (fold + 0.4 * crepe * lined).min(1.);
    }

    // Colour. Pores and furrows hold shadow; blood shows unevenly, more on
    // the cheeks, nose and ears of fair skin; pigment gathers in freckles
    // and the odd mole.
    let mut tint = Vec3::ONE;
    tint *= 1. - (0.035 * furrow + 0.12 * pore) * skin;
    let red = Vec3::new(1.04, 0.93, 0.90);
    let blotch = 0.5 + 0.5 * noise(p, 0.009, 71) + 0.25 * noise(p, 0.003, 73);
    let ears = smooth(0.062, 0.075, p.x.abs()) * smooth(-0.11, -0.07, p.z);
    let blood = (traits.flush * (0.25 + 0.75 * traits.fair)
        * (0.25 * blotch + 0.55 * cheeks + 0.5 * nose + 0.6 * ears))
        .clamp(0., 1.);
    tint *= Vec3::ONE.lerp(red * red, blood * skin);
    // Melanin mottling: a few percent everywhere.
    tint *= 1. + 0.035 * noise(p, 0.0025, 79) * skin;
    if traits.freckles > 0.02 {
        let (d, _, id) = worley(p, 0.0042, seed ^ 83);
        let sun = (0.25 + 0.75 * nose.max(cheeks)).max(0.5 * forehead) * front;
        let present = unit(id) < traits.freckles * sun * 1.4;
        if present {
            let radius = 0.0006 + 0.0010 * unit(hash(id));
            let spot = 1. - smooth(radius * 0.45, radius, d);
            let strength = (0.20 + 0.25 * unit(hash(id ^ 7))) * (0.5 + 0.5 * traits.fair);
            tint *= Vec3::ONE.lerp(Vec3::new(0.80, 0.62, 0.48), spot * strength * skin);
        }
    }
    let (d, _, id) = worley(p, 0.045, seed ^ 89);
    if unit(id) < 0.22 {
        let radius = 0.0007 + 0.0012 * unit(hash(id));
        let spot = 1. - smooth(radius * 0.6, radius, d);
        tint *= Vec3::ONE.lerp(Vec3::new(0.38, 0.25, 0.20), spot * 0.85 * skin);
        height += 0.000_12 * spot * skin;
    }
    // Folds hold shadow.
    tint *= 1. - 0.10 * deep.min(1.8) * fold;
    if old > 0. {
        // Old lips lose their red.
        tint *= Vec3::ONE.lerp(Vec3::new(0.95, 1.07, 1.07), 0.7 * old * lips);
        // Age spots: flat brown patches a few millimetres across, on the
        // temples, cheeks and forehead.
        let (d, _, id) = worley(p, 0.013, seed ^ 101);
        let sun = (cheeks.max(forehead) * front).max(0.7 * smooth(0.045, 0.065, p.x.abs()) * smooth(-0.02, 0.05, p.y));
        if unit(id) < 0.55 * old * sun {
            let radius = 0.0012 + 0.0026 * unit(hash(id));
            let edge = radius * (0.8 + 0.3 * noise(p, 0.0015, id));
            let spot = 1. - smooth(edge * 0.6, edge, d);
            tint *= Vec3::ONE.lerp(Vec3::new(0.78, 0.66, 0.56), spot * (0.25 + 0.3 * unit(hash(id ^ 3))) * (0.5 + 0.5 * traits.fair) * skin);
        }
    }
    // Lips: darker in the creases.
    tint *= 1. - 0.18 * smooth(0.05, 0.75, crease.abs()) * lips;
    (height, tint)
}

/// Distance from `p` to the segment from `a` to `b`.
fn segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let along = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-12)).clamp(0., 1.);
    p.distance(a + (b - a) * along)
}

/// How this head's proportions differ from the reference head the
/// landmarks below are given on (eyes to chin 118 mm, 65 mm from the middle
/// to the side of the face at eye height).
#[derive(Clone, Copy, Debug)]
pub(super) struct Build {
    pub wide: f32,
    pub long: f32,
}

impl Build {
    /// `eyes`: the eye midpoint, in the body's units.
    pub fn of(body: &[BodyVertex], eyes: Vec3) -> Self {
        let (mut side, mut chin) = (0f32, 0f32);
        for v in body.iter().filter(|v| v.material == FACE) {
            let p = (v.position - eyes) * METRES;
            if p.z > -0.05 && p.y.abs() < 0.012 {
                side = side.max(p.x.abs());
            }
            if p.x.abs() < 0.006 && p.z > -0.02 {
                chin = chin.min(p.y);
            }
        }
        Self { wide: (side / 0.065).clamp(0.7, 1.5), long: (-chin / 0.118).clamp(0.7, 1.5) }
    }

    /// `p` (metres from the eye midpoint) on the reference head.
    fn reference(&self, p: Vec3) -> Vec3 {
        Vec3::new(p.x / self.wide, p.y / self.long, p.z / self.long)
    }
}

/// Where the years move the face, in metres at a point `q` of the
/// reference head, for `old` from 0 (under forty) to 1 (eighty): the cheeks
/// come down and hollow, the lower cheek hangs over the jaw beside the
/// chin, the skin under the chin and the brow sink, bags stand out under
/// the eyes, the temples hollow and the tip of the nose drops.
fn sag(q: Vec3, old: f32) -> Vec3 {
    let side = q.x.signum();
    let at = Vec3::new(q.x.abs(), q.y, q.z);
    let lump = |centre: Vec3, reach: f32| (-(at.distance_squared(centre)) / (reach * reach)).exp();
    let mut d = Vec3::ZERO;
    d += Vec3::new(0.0018, -0.0065, 0.0020) * lump(Vec3::new(0.036, -0.100, 0.005), 0.026);
    d += Vec3::new(-0.0014, -0.0032, -0.0022) * lump(Vec3::new(0.046, -0.052, 0.008), 0.028);
    d += Vec3::new(0., -0.0014, 0.0022) * lump(Vec3::new(0.031, -0.023, 0.012), 0.011);
    // The upper lid hoods, the corners of the mouth turn down.
    d += Vec3::new(0., -0.0009, 0.0004) * lump(Vec3::new(0.040, 0.008, 0.010), 0.008);
    d += Vec3::new(0., -0.0008, -0.0004) * lump(Vec3::new(0.029, -0.080, 0.018), 0.010);
    // Loose skin hangs forward on the throat.
    d += Vec3::new(0., -0.0030, 0.0045) * lump(Vec3::new(0., -0.172, -0.018), 0.030);
    d += Vec3::new(0., -0.0016, 0.) * lump(Vec3::new(0.038, 0.018, 0.012), 0.012);
    d += Vec3::new(-0.0018, 0., 0.) * lump(Vec3::new(0.058, 0.030, -0.020), 0.020);
    d += Vec3::new(0., -0.0022, 0.0008) * lump(Vec3::new(0., -0.043, 0.035), 0.012);
    d += Vec3::new(0., -0.0070, 0.0065) * lump(Vec3::new(0., -0.138, -0.010), 0.034);
    Vec3::new(d.x * side, d.y, d.z) * old
}

/// Move a fitted body's head to its age (0 young adult to 1 old, as
/// `Traits::age`) and turn its normals with the surface.
pub(super) fn settle(body: &mut [BodyVertex], indices: &[u32], eyes: Vec3, age: f32) {
    // A little by fifty, most of it after.
    let old = 0.4 * smooth(0.25, 0.55, age) + 1.8 * smooth(0.55, 1.0, age).powf(1.3);
    if old <= 0. {
        return;
    }
    let build = Build::of(body, eyes);
    // The eyes stay; everything else near the head moves with the skin.
    let moved: Vec<Vec3> = body
        .iter()
        .map(|v| {
            let p = (v.position - eyes) * METRES;
            if v.material == 6 || p.y < -0.26 {
                return v.position;
            }
            v.position + sag(build.reference(p), old) * Vec3::new(build.wide, build.long, build.long) / METRES
        })
        .collect();
    // Smooth normals of the surface before and after, shared by vertices
    // at one place (the mesh is split along UV seams).
    let place = |p: Vec3| (p * 20000.).round().as_ivec3();
    let mut before = std::collections::HashMap::new();
    let mut after = std::collections::HashMap::new();
    for t in indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|k| t[k] as usize);
        if moved[a] == body[a].position && moved[b] == body[b].position && moved[c] == body[c].position {
            continue;
        }
        let was = (body[b].position - body[a].position).cross(body[c].position - body[a].position);
        let is = (moved[b] - moved[a]).cross(moved[c] - moved[a]);
        for k in [a, b, c] {
            *before.entry(place(body[k].position)).or_insert(Vec3::ZERO) += was;
            *after.entry(place(body[k].position)).or_insert(Vec3::ZERO) += is;
        }
    }
    for (v, to) in body.iter_mut().zip(moved) {
        if let (Some(was), Some(is)) = (before.get(&place(v.position)), after.get(&place(v.position))) {
            v.normal = (v.normal + is.normalize_or_zero() - was.normalize_or_zero()).normalize_or(v.normal);
        }
        v.position = to;
    }
}

/// How much of a texel of the painted face is lip, from its colour against
/// the cheek's: lips are redder (less green and blue for their red).
fn lip(color: [u8; 4], cheek: Vec3) -> f32 {
    let c = Vec3::new(color[0] as f32, color[1] as f32, color[2] as f32) / 255.;
    let ratio = (c.y + c.z) / (2. * c.x).max(1e-3);
    let cheek_ratio = (cheek.y + cheek.z) / (2. * cheek.x);
    smooth(0.86, 0.72, ratio / cheek_ratio)
}

fn bilinear(image: &RgbaImage, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (image.width() as f32, image.height() as f32);
    let (x, y) = ((u * w - 0.5).clamp(0., w - 1.), (v * h - 0.5).clamp(0., h - 1.));
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(image.width() - 1), (y0 + 1).min(image.height() - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    std::array::from_fn(|c| {
        let at = |x, y| image.get_pixel(x, y)[c] as f32;
        (at(x0, y0) * (1. - fx) + at(x1, y0) * fx) * (1. - fy)
            + (at(x0, y1) * (1. - fx) + at(x1, y1) * fx) * fy
    })
}

/// Where the eyes are drawn in the face texture: a square the unwrap leaves
/// free in a corner (centre and half size, in UV).
pub(super) const EYE_PATCH: ([f32; 2], f32) = ([0.914, 0.086], 0.066);

/// Where a person's eyebrows grow and what the face texture holds there.
#[derive(Clone, Copy)]
pub(super) struct Brows {
    /// The texture's colour for a brow hair (the face's tint multiplies it).
    pub color: Vec3,
    /// The box of the right and the left brow, metres from the eye midpoint.
    pub spans: [(Vec3, Vec3); 2],
}

/// How much brow hair covers a point of the face, 0 to 1: single hairs a
/// quarter of a millimetre wide, steep at the nose and lying outward along
/// the arch, thinning to the tail.
fn brow(p: Vec3, brows: &Brows, seed: u32) -> f32 {
    let (lo, hi) = brows.spans[usize::from(p.x > 0.)];
    if lo.x > hi.x || p.z < lo.z - 0.015 {
        return 0.;
    }
    let (inner, outer) = (lo.x.abs().min(hi.x.abs()), lo.x.abs().max(hi.x.abs()));
    let u = (p.x.abs() - inner) / (outer - inner).max(1e-4);
    if !(-0.1..=1.1).contains(&u) {
        return 0.;
    }
    let h = hi.y - lo.y;
    let middle = lo.y + h * (0.40 + 0.25 * smooth(0., 0.65, u) - 0.35 * smooth(0.65, 1., u));
    let half = (h * 0.5).max(0.0045) * (1. - 0.5 * u.clamp(0., 1.));
    let body = smooth(1., 0.45, (p.y - middle).abs() / half.max(1e-4)) * smooth(-0.06, 0.06, u) * smooth(1.06, 0.82, u);
    if body <= 0. {
        return 0.;
    }
    let angle = (1.15 - 1.5 * u).clamp(-0.3, 1.15);
    let (along, across) = (
        p.x.abs() * angle.cos() + p.y * angle.sin(),
        -p.x.abs() * angle.sin() + p.y * angle.cos(),
    );
    let side = if p.x > 0. { 7. } else { 0. };
    let hairs = smooth(-0.25, 0.2, noise(Vec3::new(along / 0.004, across / 0.0003, side), 1., seed ^ 0xb20));
    let thick = 0.75 + 0.25 * unit(hash(seed ^ 0xb21));
    (body * (0.5 + 0.5 * hairs) * thick).clamp(0., 1.)
}

/// Iris colours (sRGB): brown is by far the most common, and the only one
/// in dark skin; the lighter ones need fair skin.
fn iris_color(traits: Traits) -> Vec3 {
    let roll = unit(hash(traits.seed ^ 0x1215));
    let shade = 0.8 + 0.4 * unit(hash(traits.seed ^ 0x77));
    let light = smooth(0.35, 0.75, traits.fair);
    let brown = Vec3::new(0.36, 0.22, 0.12);
    let color = if roll < 1. - 0.6 * light {
        brown.lerp(Vec3::new(0.22, 0.13, 0.08), 1. - light)
    } else if roll < 1. - 0.42 * light {
        Vec3::new(0.45, 0.36, 0.17) // hazel
    } else if roll < 1. - 0.30 * light {
        Vec3::new(0.33, 0.42, 0.26) // green
    } else if roll < 1. - 0.10 * light {
        Vec3::new(0.30, 0.46, 0.62) // blue
    } else {
        Vec3::new(0.46, 0.52, 0.56) // grey
    };
    color * shade
}

/// One eye, looking out of the texture: `x`, `y` in -1..1 across the
/// eyeball's front (its outline at 1/0.46, as the eye's UV maps it).
fn eye(x: f32, y: f32, traits: Traits) -> Vec3 {
    let seed = traits.seed;
    // The iris is 0.49 of the eyeball's radius; the eye's UV shows 0.46 of
    // the unit normal per half tile.
    let r = x.hypot(y) / 0.46 / 0.49;
    let angle = y.atan2(x);
    let at = Vec3::new(angle.cos(), angle.sin(), 0.);
    // Fibres run from the pupil outward, in two layers.
    let fibre = noise(at * 11., 1., seed ^ 3) * 0.6
        + noise(at * 29. + Vec3::new(0., 0., r * 1.5), 1., seed ^ 5) * 0.4;
    let color = iris_color(traits);
    let pupil = 0.30;
    let t = smooth(pupil, 1., r);
    // Lighter round the pupil (the collarette), darker towards the rim.
    let mut iris = color * (1.25 - 0.55 * t) * (1. + 0.38 * fibre);
    let collarette = (1. - ((r - 0.48 - 0.04 * fibre) / 0.10).abs()).clamp(0., 1.);
    iris = iris.lerp(color.lerp(Vec3::new(0.62, 0.50, 0.30), 0.5) * 1.3, 0.35 * collarette);
    iris = iris.lerp(Vec3::splat(0.02), smooth(pupil + 0.03, pupil - 0.03, r));
    iris = iris.lerp(color * 0.18, smooth(0.80, 1.0, r));
    // The white: not white. Warmer and pinker towards the corners, with a
    // few fine vessels.
    let p = Vec3::new(x, y, 0.);
    let vessel = 1. - smooth(0.0, 0.10, noise(p * 4., 1., seed ^ 9).abs());
    let edge = smooth(1.2, 2.2, r);
    let mut white = Vec3::new(0.80, 0.77, 0.74).lerp(Vec3::new(0.76, 0.58, 0.54), 0.55 * edge);
    white = white.lerp(Vec3::new(0.72, 0.36, 0.32), 0.22 * vessel * smooth(1.05, 1.6, r));
    // The rim of the iris is soft: the cornea's edge overlaps the white.
    // The upper lid and its lashes shade the top of the eye.
    let lid = 1. - 0.5 * smooth(0.02, 0.30, -y);
    iris.lerp(white, smooth(0.96, 1.10, r)) * lid
}

/// Draw the eye into its patch of the face texture.
fn paint_eye(albedo: &mut RgbaImage, traits: Traits) {
    let size = albedo.width() as f32;
    let ([cu, cv], half) = EYE_PATCH;
    // A margin of the white round the patch, for the smaller mips.
    let reach = half * 1.12;
    let (x0, x1) = (((cu - reach) * size) as u32, ((cu + reach) * size) as u32);
    let (y0, y1) = (((cv - reach) * size) as u32, ((cv + reach) * size) as u32);
    for py in y0..=y1 {
        for px in x0..=x1 {
            let x = ((px as f32 + 0.5) / size - cu) / half;
            let y = ((py as f32 + 0.5) / size - cv) / half;
            let c = eye(x, y, traits);
            let [r, g, b] = c.to_array().map(|c| (c.clamp(0., 1.) * 255.) as u8);
            albedo.put_pixel(px, py, image::Rgba([r, g, b, 255]));
        }
    }
}

/// The face maps of one person at `size` texels square. `base` is the
/// painted face (lips, lids, the fall of colour over the head) with the
/// shading mask in its alpha, `cheek` that painting's cheek colour,
/// `under_hair` what the texture holds where hair grows (the dark between
/// strands), `shade` how much darker that is well under the hair.
pub(super) fn face(
    body: &[BodyVertex],
    indices: &[u32],
    eyes: Vec3,
    base: &RgbaImage,
    cheek: Vec3,
    traits: Traits,
    under_hair: Vec3,
    shade: f32,
    cut: &super::warp_hair::Cut,
    brows: Brows,
    size: usize,
) -> Maps {
    let paint = FacePaint { base, cheek, traits, under_hair, shade, cut, brows, build: Build::of(body, eyes) };
    paint.bake(body, indices, eyes, size, 64)
}

struct FacePaint<'a> {
    base: &'a RgbaImage,
    cheek: Vec3,
    traits: Traits,
    under_hair: Vec3,
    shade: f32,
    cut: &'a super::warp_hair::Cut,
    brows: Brows,
    build: Build,
}

impl FacePaint<'_> {
    fn bake(&self, body: &[BodyVertex], indices: &[u32], eyes: Vec3, size: usize, strip_rows: usize) -> Maps {
        // A one-row halo at either end supplies the central differences for
        // the normals. All workers share these two buffers, even in large pools.
        let capacity = (strip_rows + 2).min(size) * size;
        let mut points = vec![None; capacity];
        let lid_travel = lid_travel(body, indices, eyes, size, &mut points);
        let mut heights = vec![0f32; capacity];
        let mut albedo = RgbaImage::new(size as u32, size as u32);
        let mut normal = RgbaImage::from_pixel(size as u32, size as u32, image::Rgba([128, 128, 255, 255]));
        let workers = render::par::workers();
        for start in (0..size).step_by(strip_rows) {
            let stop = (start + strip_rows).min(size);
            let first = start.saturating_sub(1);
            let end = (stop + 1).min(size);
            let len = (end - first) * size;
            let texels = &mut points[..len];
            unwrap_rows(body, indices, eyes, size, first, texels);
            let colors = &mut albedo.as_mut()[first * size * 4..end * size * 4];
            let per_worker = (end - first).div_ceil(workers) * size;
            render::par::scope(|scope| {
                for (k, ((texels, heights), colors)) in texels.chunks(per_worker)
                    .zip(heights[..len].chunks_mut(per_worker))
                    .zip(colors.chunks_mut(per_worker * 4))
                    .enumerate()
                {
                    scope.spawn(move || {
                        for (i, ((texel, h), rgba)) in texels.iter().zip(heights).zip(colors.chunks_exact_mut(4)).enumerate() {
                            let at = k * per_worker + i;
                            let (color, height) = self.sample(at % size, first + at / size, size, texel.as_ref(), lid_travel);
                            rgba.copy_from_slice(&color);
                            *h = height;
                        }
                    });
                }
            });
            for y in start.max(1)..stop.min(size - 1) {
                for x in 1..size - 1 {
                    let i = (y - first) * size + x;
                    normal.put_pixel(x as u32, y as u32, surface_normal(texels, &heights, i, size, self.cut));
                }
            }
        }
        paint_eye(&mut albedo, self.traits);
        Maps { albedo, normal }
    }

    fn sample(&self, x: usize, y: usize, size: usize, texel: Option<&Texel>, lid_travel: f32) -> ([u8; 4], f32) {
        let Self { base, cheek, traits, under_hair, shade, cut, brows, build } = *self;
        let mut height = 0.;
        let (u, v) = ((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32);
        let mut painted = bilinear(base, u, v);
        let mut color = Vec3::new(painted[0], painted[1], painted[2]);
        if let Some(t) = texel {
            let lips = lip(painted.map(|c| c as u8), cheek);
            let (relief, tint) = skin(t, lips, traits, build);
            height = relief;
            color *= tint;
            // Under the hair the scalp is seen through it, in
            // its shade.
            // Through short hair the skin still shows; under
            // longer hair it does not, and what is seen there
            // is lit as hair: no sheen of skin in the sun.
            let hair = cut.grows(t.p);
            color = color.lerp(under_hair * 255., 0.7 * hair * cut.inside(t.p, 0., 0.016));
            // Deeper in, the hair stands over it: darker, the
            // more the less light the hair lets through.
            let deep = cut.under(t.p) * smooth(0.005, 0.03, cut.top);
            color = color.lerp(under_hair * 255. * (1. - shade), deep);
            painted[3] *= 1. - cut.under(t.p) * smooth(0.002, 0.012, cut.top);
            color = color.lerp(brows.color * 255., brow(t.p, &brows, traits.seed));
            // Lashes: a dark, broken line along the rim of the
            // upper lid, which travels furthest in a blink.
            let rim = smooth(0.80, 0.96, t.lid / lid_travel);
            let lashes = 0.55 + 0.45 * smooth(-0.3, 0.3, noise(Vec3::new(t.p.x / 0.0005, 0., 0.), 1., traits.seed ^ 0x1a5));
            color = color.lerp(Vec3::new(14., 10., 9.), 0.9 * rim * lashes);
        }
        let [r, g, b] = color.to_array().map(|c| c.clamp(0., 255.) as u8);
        ([r, g, b, painted[3] as u8], height)
    }
}

/// Tangent-space normal from relief along u and the image rows, using the
/// true distance between neighbouring texels on the face.
fn surface_normal(texels: &[Option<Texel>], height: &[f32], i: usize, size: usize, cut: &super::warp_hair::Cut) -> image::Rgba<u8> {
    let at = |i: usize| texels[i].as_ref().map(|t| (t.p, height[i]));
    let slope = |a: Option<(Vec3, f32)>, b: Option<(Vec3, f32)>| match (a, b) {
        (Some((pa, ha)), Some((pb, hb))) => {
            let d = pa.distance(pb);
            // Neighbours in the texture that are far apart on the
            // head lie across a seam.
            if d > 1e-6 && d < 0.004 { (hb - ha) / d } else { 0. }
        }
        _ => 0.,
    };
    let du = slope(at(i - 1), at(i + 1));
    let dv = slope(at(i - size), at(i + size));
    // t = dP/du, b = -dP/dv (shade.wgsl `perturb`).
    let n = Vec3::new(-du, dv, 1.).normalize();
    let [r, g, b] = n.to_array().map(|c| ((c * 0.5 + 0.5) * 255.).round() as u8);
    // Alpha: how much of the skin's sheen shows (the shader's
    // `gloss`). None where hair stands over it: that scalp is in
    // the hair's shade and reflects neither sun nor sky. At the
    // hairline the skin is in plain view and still skin.
    let bare = texels[i].as_ref().map_or(1., |t| 1. - cut.under(t.p) * smooth(0.002, 0.012, cut.top));
    image::Rgba([r, g, b, (bare * 255.) as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unwrap(body: &[BodyVertex], indices: &[u32], eyes: Vec3, size: usize) -> Vec<Option<Texel>> {
        let mut texels = vec![None; size * size];
        unwrap_rows(body, indices, eyes, size, 0, &mut texels);
        texels
    }

    fn fixture(seed: u64) -> (Vec<BodyVertex>, Vec<u32>, Vec3) {
        let person = render::humans::Instance::person(seed, Vec3::ZERO, 0., &Default::default());
        let (body, indices) = person.fitted_mesh(false);
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for v in body.iter().filter(|v| v.material == 6) {
            (lo, hi) = (lo.min(v.position), hi.max(v.position));
        }
        (body, indices, (lo + hi) * 0.5)
    }

    #[test]
    fn the_eye_patch_is_free_of_face() {
        let (body, indices, eyes) = fixture(7);
        let size = 512;
        let texels = unwrap(&body, &indices, eyes, size);
        let ([cu, cv], half) = EYE_PATCH;
        let reach = half * 1.2;
        let mut covered = 0;
        for y in ((cv - reach) * size as f32) as usize..=((cv + reach) * size as f32) as usize {
            for x in ((cu - reach) * size as f32) as usize..=((cu + reach) * size as f32) as usize {
                assert!(texels[y * size + x].is_none(), "face texel {x},{y} under the eye patch");
            }
        }
        for t in &texels {
            covered += usize::from(t.is_some());
        }
        assert!(covered > size * size / 2, "the face fills most of its texture: {covered}");
    }

    #[test]
    fn skin_differs_by_seed_and_has_relief() {
        let (body, indices, eyes) = fixture(7);
        let base = super::super::warp_human::tile(0);
        let cheek = Vec3::new(0.855, 0.659, 0.541);
        let cut = super::super::warp_hair::Cut::styled(1, super::super::warp_hair::Style::SidePart);
        let brows = Brows { color: Vec3::splat(0.1), spans: [(Vec3::splat(1.), Vec3::splat(-1.)); 2] };
        let paint = FacePaint {
            base: &base, cheek, traits: Traits::new(1, 0.8), under_hair: Vec3::splat(0.2),
            shade: 0.3, cut: &cut, brows, build: Build::of(&body, eyes),
        };
        let a = paint.bake(&body, &indices, eyes, 256, 41);
        let whole = paint.bake(&body, &indices, eyes, 256, 256);
        assert_eq!(a.albedo.as_raw(), whole.albedo.as_raw());
        assert_eq!(a.normal.as_raw(), whole.normal.as_raw());
        let b = face(&body, &indices, eyes, &base, cheek, Traits::new(2, 0.8), Vec3::splat(0.2), 0.3, &cut, brows, 256);
        assert_ne!(a.albedo.as_raw(), b.albedo.as_raw());
        let bent = a.normal.pixels().filter(|p| p[0].abs_diff(128) > 2 || p[1].abs_diff(128) > 2).count();
        assert!(bent > 256 * 256 / 20, "relief bends the normals: {bent}");
        // The shading mask in the painted alpha survives on the face (128
        // is skin); under the hair it goes to hair (0).
        let (skin, hair) = a.albedo.pixels().fold((0, 0), |(s, h), p| (s + usize::from(p[3] == 128), h + usize::from(p[3] < 16)));
        assert!(skin > 256 * 256 / 20 && hair > 256 * 256 / 20, "{skin} skin texels, {hair} under hair");
    }

    /// `cargo test -p assets --lib skin_maps -- --ignored` writes the maps
    /// of one person to build/human-skin for a look.
    #[test]
    #[ignore]
    fn skin_maps() {
        let person = render::humans::Instance::person(7, Vec3::ZERO, 0., &Default::default());
        let (body, indices) = person.fitted_mesh(false);
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        let (mut face_lo, mut face_hi) = (lo, hi);
        for v in &body {
            if v.material == 6 {
                (lo, hi) = (lo.min(v.position), hi.max(v.position));
            }
            if v.material == FACE {
                (face_lo, face_hi) = (face_lo.min(v.position), face_hi.max(v.position));
            }
        }
        let eyes = (lo + hi) * 0.5;
        eprintln!("eyes {lo} {hi}; face {face_lo} {face_hi}");
        let base = super::super::warp_human::tile(person.face_tile());
        let traits = Traits { seed: 7, age: 0.6, freckles: 0.5, flush: 0.8, fair: 0.8 };
        let start = std::time::Instant::now();
        let maps = face(&body, &indices, eyes, &base, Vec3::new(0.855, 0.659, 0.541), traits, Vec3::splat(0.2), 0.3, &super::super::warp_hair::Cut::styled(1, super::super::warp_hair::Style::SidePart), Brows { color: Vec3::splat(0.1), spans: [(Vec3::splat(1.), Vec3::splat(-1.)); 2] }, 2048);
        eprintln!("{:?}", start.elapsed());
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../build/human-skin");
        std::fs::create_dir_all(&out).unwrap();
        maps.albedo.save(out.join("albedo.png")).unwrap();
        maps.normal.save(out.join("normal.png")).unwrap();
        // Where the head lies in the texture, for placing regions.
        let texels = unwrap(&body, &indices, eyes, 512);
        RgbaImage::from_fn(512, 512, |x, y| match &texels[y as usize * 512 + x as usize] {
            Some(t) => {
                let c = (t.p / 0.12 * 0.5 + Vec3::splat(0.5)) * 255.;
                image::Rgba([c.x as u8, c.y as u8, c.z as u8, 255])
            }
            None => image::Rgba([0, 0, 0, 255]),
        })
        .save(out.join("position.png"))
        .unwrap();
    }
}
