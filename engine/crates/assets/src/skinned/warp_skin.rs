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
}

pub(super) fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

fn hash3(c: [i32; 3], seed: u32) -> u32 {
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
struct Texel {
    /// Metres from the point between the eyes: x to the person's left, y
    /// up, z out of the face.
    p: Vec3,
    n: Vec3,
    /// How far the point travels when the eye closes, metres: most at the
    /// rim of the upper lid.
    lid: f32,
}

/// Rasterize the face triangles into the texture: the head position and
/// normal under every texel.
fn unwrap(body: &[BodyVertex], indices: &[u32], eyes: Vec3, size: usize) -> Vec<Option<Texel>> {
    let mut texels: Vec<Option<Texel>> = (0..size * size).map(|_| None).collect();
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
        for y in lo.y as usize..=hi.y as usize {
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
                let slot = &mut texels[y * size + x];
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
    texels
}

/// Relief in metres (negative into the skin) and what the albedo is
/// multiplied by, at one point of the face.
fn skin(t: &Texel, lips: f32, traits: Traits) -> (f32, Vec3) {
    let p = t.p;
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
    if traits.age > 0.25 {
        let a = smooth(0.25, 0.9, traits.age);
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
    // Lips: darker in the creases.
    tint *= 1. - 0.18 * smooth(0.05, 0.75, crease.abs()) * lips;
    (height, tint)
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
/// strands).
pub(super) fn face(
    body: &[BodyVertex],
    indices: &[u32],
    eyes: Vec3,
    base: &RgbaImage,
    cheek: Vec3,
    traits: Traits,
    under_hair: Vec3,
    brows: Brows,
    size: usize,
) -> Maps {
    let texels = unwrap(body, indices, eyes, size);
    let lid_travel = texels.iter().flatten().map(|t| t.lid).fold(1e-6f32, f32::max);
    let mut height = vec![0f32; size * size];
    let mut albedo = vec![[0u8; 4]; size * size];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = size.div_ceil(threads);
    std::thread::scope(|scope| {
        for ((k, heights), colors) in height
            .chunks_mut(rows * size)
            .enumerate()
            .zip(albedo.chunks_mut(rows * size))
        {
            let texels = &texels;
            scope.spawn(move || {
                for (i, (h, out)) in heights.iter_mut().zip(colors).enumerate() {
                    let (x, y) = (i % size, k * rows + i / size);
                    let (u, v) = ((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32);
                    let painted = bilinear(base, u, v);
                    let mut color = Vec3::new(painted[0], painted[1], painted[2]);
                    if let Some(t) = &texels[y * size + x] {
                        let lips = lip(painted.map(|c| c as u8), cheek);
                        let (relief, tint) = skin(t, lips, traits);
                        *h = relief;
                        color *= tint;
                        // Under the hair the scalp is seen through it, in
                        // its shade.
                        let hair = super::warp_hair::scalp(t.p);
                        color = color.lerp(under_hair * 255., 0.7 * hair * hair);
                        color = color.lerp(brows.color * 255., brow(t.p, &brows, traits.seed));
                        // Lashes: a dark, broken line along the rim of the
                        // upper lid, which travels furthest in a blink.
                        let rim = smooth(0.80, 0.96, t.lid / lid_travel);
                        let lashes = 0.55 + 0.45 * smooth(-0.3, 0.3, noise(Vec3::new(t.p.x / 0.0005, 0., 0.), 1., traits.seed ^ 0x1a5));
                        color = color.lerp(Vec3::new(14., 10., 9.), 0.9 * rim * lashes);
                    }
                    let [r, g, b] = color.to_array().map(|c| c.clamp(0., 255.) as u8);
                    *out = [r, g, b, painted[3] as u8];
                }
            });
        }
    });
    // Tangent-space normals from the relief: the slope along u and along
    // the image's rows, over the true distance between neighbouring texels.
    let mut normal = RgbaImage::from_pixel(size as u32, size as u32, image::Rgba([128, 128, 255, 255]));
    for y in 1..size - 1 {
        for x in 1..size - 1 {
            let at = |x: usize, y: usize| texels[y * size + x].as_ref().map(|t| (t.p, height[y * size + x]));
            let slope = |a: Option<(Vec3, f32)>, b: Option<(Vec3, f32)>| match (a, b) {
                (Some((pa, ha)), Some((pb, hb))) => {
                    let d = pa.distance(pb);
                    // Neighbours in the texture that are far apart on the
                    // head lie across a seam.
                    if d > 1e-6 && d < 0.004 { (hb - ha) / d } else { 0. }
                }
                _ => 0.,
            };
            let du = slope(at(x - 1, y), at(x + 1, y));
            let dv = slope(at(x, y - 1), at(x, y + 1));
            // t = dP/du, b = -dP/dv (shade.wgsl `perturb`).
            let n = Vec3::new(-du, dv, 1.).normalize();
            let [r, g, b] = n.to_array().map(|c| ((c * 0.5 + 0.5) * 255.).round() as u8);
            normal.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, 255]));
        }
    }
    let mut albedo = RgbaImage::from_fn(size as u32, size as u32, |x, y| {
        image::Rgba(albedo[y as usize * size + x as usize])
    });
    paint_eye(&mut albedo, traits);
    Maps { albedo, normal }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let a = face(&body, &indices, eyes, &base, cheek, Traits::new(1, 0.8), Vec3::splat(0.2), Brows { color: Vec3::splat(0.1), spans: [(Vec3::splat(1.), Vec3::splat(-1.)); 2] }, 256);
        let b = face(&body, &indices, eyes, &base, cheek, Traits::new(2, 0.8), Vec3::splat(0.2), Brows { color: Vec3::splat(0.1), spans: [(Vec3::splat(1.), Vec3::splat(-1.)); 2] }, 256);
        assert_ne!(a.albedo.as_raw(), b.albedo.as_raw());
        assert_eq!(a.albedo.as_raw(), face(&body, &indices, eyes, &base, cheek, Traits::new(1, 0.8), Vec3::splat(0.2), Brows { color: Vec3::splat(0.1), spans: [(Vec3::splat(1.), Vec3::splat(-1.)); 2] }, 256).albedo.as_raw());
        let bent = a.normal.pixels().filter(|p| p[0].abs_diff(128) > 2 || p[1].abs_diff(128) > 2).count();
        assert!(bent > 256 * 256 / 20, "relief bends the normals: {bent}");
        // The shading mask in the painted alpha survives (128 is skin).
        assert!(a.albedo.pixels().all(|p| p[3] >= 128));
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
        let maps = face(&body, &indices, eyes, &base, Vec3::new(0.855, 0.659, 0.541), traits, Vec3::splat(0.2), Brows { color: Vec3::splat(0.1), spans: [(Vec3::splat(1.), Vec3::splat(-1.)); 2] }, 2048);
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
