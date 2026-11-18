//! A person's hair, grown from their seed out of their own scalp: where the
//! hairline runs, how long the hair is, where it parts, how it is combed and
//! how it falls. No groom mesh: a strand starts on the head's skin, leaves
//! it at an angle, bends under its weight and rests on the skull. The
//! renderer draws the strands as they are (render::humans::PlayerStrands).
use super::warp_skin::{hash, noise, smooth, unit, FACE, METRES};
use glam::Vec3;
use render::humans::BodyVertex;

/// Points along one strand, root to tip (the strand renderer's nine
/// segments).
pub(super) const POINTS: usize = 10;
/// Strands on a full head of hair (the strand renderer's budget per head).
const STRANDS: usize = 16_384;

/// How thickly hair grows at a point of the head, 0 to 1: `p` in metres
/// from the point between the eyes (x to the person's left, y up, z out of
/// the face). The hairline crosses the forehead, drops at the temples, runs
/// over the ears and down to the nape.
pub(super) fn scalp(p: Vec3) -> f32 {
    let front = 0.062 - 0.034 * smooth(0.030, 0.072, p.x.abs());
    let side = 0.026;
    let nape = -0.072;
    let mut line = front + (side - front) * smooth(-0.020, -0.050, p.z);
    line += (nape - side) * smooth(-0.125, -0.155, p.z);
    smooth(line, line + 0.007, p.y)
}

/// What the seed decides about a head of hair.
#[derive(Clone, Copy, Debug)]
pub(super) struct Cut {
    pub seed: u32,
    /// Metres on top of the head, at the sides and back.
    pub top: f32,
    pub sides: f32,
    /// Where the hair parts across the head, metres from the middle; hair
    /// lies away from it on both sides.
    pub part: f32,
    /// How far the front is swept back (1) or falls forward as a fringe (-1).
    pub sweep: f32,
    /// Height above the eyes at which hair over the face is cut, metres.
    pub fringe: f32,
    /// How steeply hair leaves the scalp, 0 flat to 1 upright.
    pub lift: f32,
    /// Waves: sideways swing in metres, and waves per metre of hair.
    pub wave: f32,
    pub pitch: f32,
}

impl Cut {
    pub fn new(seed: u32) -> Self {
        let roll = |k: u32| unit(hash(seed ^ k.wrapping_mul(0x9e37_79b9)));
        // Cropped, short, collar length and long, with short the most common.
        let (top, sides) = match roll(1) {
            r if r < 0.14 => (0.012 + 0.012 * roll(2), 0.006 + 0.008 * roll(3)),
            r if r < 0.58 => (0.045 + 0.045 * roll(2), 0.018 + 0.030 * roll(3)),
            r if r < 0.80 => (0.10 + 0.06 * roll(2), 0.09 + 0.07 * roll(3)),
            _ => (0.20 + 0.14 * roll(2), 0.20 + 0.14 * roll(3)),
        };
        let wavy = roll(6).powi(2);
        Self {
            seed,
            top,
            sides,
            part: if roll(4) < 0.25 { 0. } else { (0.025 + 0.02 * roll(5)) * if roll(7) < 0.5 { -1. } else { 1. } },
            // Hair long enough to reach the eyes is worn out of the face.
            sweep: if top < 0.03 { 0. } else if top > 0.07 { 0.6 + 0.4 * roll(8) } else { roll(8) * 2. - 0.6 },
            lift: 0.10 + 0.30 * roll(9) * (0.05 / top.max(0.05)),
            fringe: 0.022 + 0.026 * roll(11),
            wave: 0.012 * wavy,
            pitch: 12. + 25. * roll(10),
        }
    }
}

/// Directions round the head's middle in which its skin is measured.
const AROUND: usize = 64;
const UPWARD: usize = 32;

/// The head as the hair rests on it, in metres from the eye midpoint: an
/// ellipsoid through the skin above the ears for the way hair is combed,
/// and how far the skin itself (ears included) reaches from the middle in
/// every direction, for what hair may not pass.
struct Skull {
    centre: Vec3,
    radii: Vec3,
    reach: Vec<f32>,
}

impl Skull {
    /// `triangles`: the head's skin, three corners each.
    fn of(triangles: &[[Vec3; 3]]) -> Self {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for &p in triangles.iter().flatten().filter(|p| p.y > 0.03) {
            (lo, hi) = (lo.min(p), hi.max(p));
        }
        // The crown is the top of the ellipsoid; its middle sits at brow
        // height, where the head is widest.
        let centre = Vec3::new(0., 0.02, (lo.z + hi.z) * 0.5);
        let radii = Vec3::new(hi.x.max(-lo.x), hi.y - centre.y, (hi.z - lo.z) * 0.5);
        let mut reach = vec![0f32; AROUND * UPWARD];
        for t in triangles {
            // Points every few millimetres over the triangle.
            let n = (((t[1] - t[0]).length().max((t[2] - t[0]).length()) / 0.003).ceil() as usize).clamp(1, 12);
            for a in 0..=n {
                for b in 0..=n - a {
                    let (u, v) = (a as f32 / n as f32, b as f32 / n as f32);
                    let d = t[0] * (1. - u - v) + t[1] * u + t[2] * v - centre;
                    let (x, y) = Self::cell(d);
                    let at = &mut reach[x as usize % AROUND + (y as usize).min(UPWARD - 1) * AROUND];
                    *at = at.max(d.length());
                }
            }
        }
        // Where there is no skin (the open neck), the ellipsoid stands in.
        for y in 0..UPWARD {
            for x in 0..AROUND {
                if reach[x + y * AROUND] == 0. {
                    let lon = (x as f32 + 0.5) / AROUND as f32 * std::f32::consts::TAU - std::f32::consts::PI;
                    let lat = (y as f32 + 0.5) / UPWARD as f32 * std::f32::consts::PI - std::f32::consts::FRAC_PI_2;
                    let d = Vec3::new(lon.sin() * lat.cos(), lat.sin(), lon.cos() * lat.cos());
                    reach[x + y * AROUND] = 1. / (d / radii).length();
                }
            }
        }
        Self { centre, radii, reach }
    }

    /// Where direction `d` falls in the map, in cells.
    fn cell(d: Vec3) -> (f32, f32) {
        let lon = d.x.atan2(d.z);
        let lat = (d.y / d.length().max(1e-9)).clamp(-1., 1.).asin();
        (
            (lon + std::f32::consts::PI) / std::f32::consts::TAU * AROUND as f32,
            (lat + std::f32::consts::FRAC_PI_2) / std::f32::consts::PI * UPWARD as f32,
        )
    }

    /// Keep `p` at least `clear` metres outside the skin.
    fn rest(&self, p: Vec3, clear: f32) -> Vec3 {
        let d = p - self.centre;
        let r = d.length();
        if r < 1e-5 {
            return p;
        }
        let (x, y) = Self::cell(d);
        let (x, y) = (x - 0.5, (y - 0.5).clamp(0., UPWARD as f32 - 1.));
        let (x0, y0) = (x.floor(), y.floor());
        let at = |i: f32, j: f32| {
            self.reach[(i.rem_euclid(AROUND as f32)) as usize % AROUND + (j as usize).min(UPWARD - 1) * AROUND]
        };
        let (fx, fy) = (x - x0, y - y0);
        let skin = (at(x0, y0) * (1. - fx) + at(x0 + 1., y0) * fx) * (1. - fy)
            + (at(x0, y0 + 1.) * (1. - fx) + at(x0 + 1., y0 + 1.) * fx) * fy;
        if r >= skin + clear { p } else { self.centre + d / r * (skin + clear) }
    }

    fn outward(&self, p: Vec3) -> Vec3 {
        ((p - self.centre) / (self.radii * self.radii)).normalize_or(Vec3::Y)
    }
}

/// One strand from a root: POINTS positions in metres from the eye midpoint.
fn grow(root: Vec3, normal: Vec3, cut: Cut, skull: &Skull, id: u32) -> [Vec3; POINTS] {
    let roll = |k: u32| unit(hash(id.wrapping_mul(0x85eb_ca6b) ^ k));
    // Hair lies as its lock does: neighbours share length and bend.
    let lock = |k: u32| 0.5 + 0.5 * noise(root, 0.012, cut.seed ^ k);
    let top = smooth(0.35, 0.85, normal.y);
    let length = (cut.sides + (cut.top - cut.sides) * top) * (0.75 + 0.2 * lock(11) + 0.15 * roll(1));
    // The comb: away from the parting on top, back or forward at the front,
    // down everywhere else.
    // Next to the parting hair runs back along it before it turns aside,
    // so the parting is a line and not a bare strip.
    let away = smooth(-0.006, 0.006, root.x - cut.part) * 2. - 1.;
    let fore = smooth(-0.07, -0.01, root.z);
    let comb = Vec3::new(away * (0.35 + 0.65 * (1. - fore * cut.sweep.abs())), -0.15, -cut.sweep * fore - 0.25 * (1. - fore));
    let flat = |v: Vec3, n: Vec3| (v - n * v.dot(n)).normalize_or(Vec3::NEG_Y);
    let lie = flat(comb.lerp(Vec3::NEG_Y, 1. - top), normal);
    // Hair stands up most on top; at the sides it lies down the head.
    let lift = (cut.lift * (0.7 + 0.6 * lock(13)) * (0.3 + 0.7 * top)).clamp(0.03, 0.9);
    let mut dir = (normal * lift + lie * (1. - lift)).normalize();
    let across = dir.cross(normal).normalize_or(Vec3::X);
    let step = length / (POINTS - 1) as f32;
    let phase = std::f32::consts::TAU * lock(17);
    let mut points = [root; POINTS];
    let mut p = root;
    for k in 1..POINTS {
        let along = step * k as f32;
        // Weight bends a strand more the longer it reaches past its root;
        // stray hairs stand off a little.
        let sag = (along / 0.06).min(1.) * 0.55 + 0.05;
        // Hair follows the round of the head it lies on: what points away
        // from the skull is bent back along it, less the more the hair is
        // lifted, and not below the widest part of the head, where it hangs.
        let out = skull.outward(p);
        let hug = (1. - lift) * smooth(-0.03, 0.02, p.y - skull.centre.y);
        dir = (dir - out * dir.dot(out).max(0.) * hug + Vec3::NEG_Y * sag * step / 0.012 * 0.25).normalize();
        let swing = cut.wave * (cut.pitch * along + phase).sin() * (along / 0.04).min(1.);
        let next = p + dir * step + across * (swing - cut.wave * (cut.pitch * (along - step) + phase).sin() * ((along - step) / 0.04).clamp(0., 1.));
        // Rest on the skull, lower layers under upper ones.
        let clear = 0.002 + 0.006 * roll(3) * (along / length.max(1e-4)) + 0.004 * lift;
        let rested = skull.rest(next, clear);
        if rested != next {
            // Slide along the head instead of into it.
            let out = skull.outward(rested);
            dir = flat(dir, out).lerp(dir, 0.2).normalize();
        }
        p = rested;
        // The face stays clear: hair that would come down over it is cut
        // there, as a fringe across the forehead and shorter round the
        // cheeks.
        if p.z > -0.03 - 0.02 * roll(5) && p.y < cut.fringe + 0.006 * (lock(19) - 0.5) {
            let end = points[k - 1];
            points[k..].fill(end);
            break;
        }
        points[k] = p;
    }
    points
}

/// The strands of one person: POINTS points each, as [x, y, z, shade] in
/// the fitted body's normalized units, for a groom whose fit scales x and z
/// by `skull`. `eyes` is the eye midpoint in the same units.
pub(super) fn strands(body: &[BodyVertex], indices: &[u32], eyes: Vec3, skull: f32, cut: Cut) -> Vec<[f32; 4]> {
    let metres = |p: Vec3| (p - eyes) * METRES;
    let triangles: Vec<[&BodyVertex; 3]> = indices
        .chunks_exact(3)
        .map(|t| [0, 1, 2].map(|k| &body[t[k] as usize]))
        .filter(|t| t[0].material == FACE)
        .collect();
    let head: Vec<[Vec3; 3]> = triangles.iter().map(|t| t.map(|v| metres(v.position))).collect();
    let skull_shape = Skull::of(&head);
    // Hair-bearing area, to share the strands out evenly.
    let cover = |t: &[&BodyVertex; 3]| {
        let p = t.map(|v| metres(v.position));
        0.5 * (p[1] - p[0]).cross(p[2] - p[0]).length() * scalp((p[0] + p[1] + p[2]) / 3.)
    };
    let total: f32 = triangles.iter().map(cover).sum();
    if total <= 0. {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(STRANDS * POINTS);
    let mut owed = 0.;
    let mut id = cut.seed;
    for t in &triangles {
        owed += cover(t) / total * STRANDS as f32;
        while owed >= 1. && out.len() < STRANDS * POINTS {
            owed -= 1.;
            id = hash(id.wrapping_add(0x9e37_79b9));
            let (mut u, mut v) = (unit(id), unit(hash(id ^ 0x51)));
            if u + v > 1. {
                (u, v) = (1. - u, 1. - v);
            }
            let w = 1. - u - v;
            let root = metres(t[0].position * w + t[1].position * u + t[2].position * v);
            // Thin out along the hairline rather than end at a triangle edge.
            if scalp(root) < unit(hash(id ^ 0x77)) {
                continue;
            }
            let normal = (t[0].normal * w + t[1].normal * u + t[2].normal * v).normalize_or(Vec3::Y);
            let shade = 0.62 + 0.55 * unit(hash(id ^ 0x99)) * (0.6 + 0.4 * noise(root, 0.01, cut.seed ^ 5).abs());
            let grown = grow(root, normal, cut, &skull_shape, id);
            // A strand cut at its root is no hair.
            if grown[1] == grown[0] {
                continue;
            }
            for p in grown {
                let q = p / METRES + eyes;
                out.push([q.x / skull, q.y, q.z / skull, shade]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(seed: u64) -> (Vec<BodyVertex>, Vec<u32>, Vec3) {
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
    fn hair_grows_from_the_scalp_and_stays_outside_the_head() {
        let (body, indices, eyes) = person(7);
        let cut = Cut { top: 0.08, sides: 0.05, ..Cut::new(3) };
        let hair = strands(&body, &indices, eyes, 1., cut);
        assert!(hair.len() > STRANDS * POINTS * 8 / 10, "a full head: {} points", hair.len());
        assert_eq!(hair.len() % POINTS, 0);
        let mut face = 0;
        for strand in hair.chunks_exact(POINTS) {
            let at = |k: usize| (Vec3::new(strand[k][0], strand[k][1], strand[k][2]) - eyes) * METRES;
            assert!(scalp(at(0)) > 0., "root on the scalp: {}", at(0));
            let length: f32 = (1..POINTS).map(|k| at(k).distance(at(k - 1))).sum();
            assert!(length > 0.002 && length < 0.12, "length {length}");
            // Nothing hangs in front of the eyes.
            face += usize::from((0..POINTS).any(|k| at(k).z > 0.0 && at(k).y.abs() < 0.012 && at(k).x.abs() < 0.05));
        }
        assert!(face < STRANDS / 100, "{face} strands across the eyes");
    }

    #[test]
    fn cuts_differ_by_seed() {
        let lengths: Vec<f32> = (0..40).map(|s| Cut::new(s).top).collect();
        assert!(lengths.iter().any(|&l| l < 0.03) && lengths.iter().any(|&l| l > 0.15), "{lengths:?}");
    }
}
