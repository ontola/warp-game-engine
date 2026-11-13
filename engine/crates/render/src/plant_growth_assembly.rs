//! A grown plant as an Assembly: a few reusable parts (stems of three sizes,
//! needle tufts) instanced where growth put them, on a bone per main limb. Age
//! only changes counts and placement, so better part meshes drop in through
//! `Parts` and every age picks them up. Wind moves the bones and parts follow
//! rigidly; virtual members expand the same parts on the GPU.
use crate::{
    cluster::Vertex,
    foliage_assembly::{Archetype, Assembly, BranchGroup, Limits, Parameters, PartGeometry, PartInstance},
    plant_growth::Plant,
    scene::{Material, THIN_FOLIAGE, TWO_SIDED},
};
use glam::{Mat4, Quat, Vec3};
use std::f32::consts::TAU;

/// The parts a plant is assembled from. Stems are unit cylinders along +Y
/// (radius 1, height 1); a tuft grows from its origin along +Y.
#[derive(Clone, Debug)]
pub struct Parts {
    /// Stems that narrow to 1.0, 0.94, 0.86 and 0.72 of their base radius,
    /// six-sided then ten-sided: a run picks the taper nearest its own,
    /// so consecutive runs meet at nearly the same ring.
    pub stems: [PartGeometry; 8],
    /// A unit ball for joints and branch collars.
    pub collar: PartGeometry,
    pub tufts: Vec<PartGeometry>,
}

const TAPERS: [f32; 4] = [1.0, 0.94, 0.86, 0.72];

fn stem(id: u64, sides: u32, taper: f32) -> PartGeometry {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    // Closed (capped) and with the seam on exactly the first ring's positions:
    // foliage optics need every edge shared by two faces.
    let ring = |i: u32| {
        let a = (i % sides) as f32 / sides as f32 * TAU;
        (a.cos(), a.sin())
    };
    for i in 0..=sides {
        let (c, s) = ring(i);
        for y in [0.0, 1.0] {
            let r = 1.0 + (taper - 1.0) * y;
            let slope = 1.0 - taper;
            let n = Vec3::new(c, slope, s).normalize();
            vertices.push(Vertex { pos: [c * r, y, s * r], normal: n.to_array(), uv: [i as f32 / sides as f32, y] });
        }
    }
    for i in 0..sides {
        let b = i * 2;
        indices.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3]);
    }
    for (y, normal) in [(0.0f32, -1.0f32), (1.0, 1.0)] {
        let centre = vertices.len() as u32;
        vertices.push(Vertex { pos: [0.0, y, 0.0], normal: [0.0, normal, 0.0], uv: [0.5, y] });
        let first = vertices.len() as u32;
        for i in 0..sides {
            let (c, s) = ring(i);
            let r = 1.0 + (taper - 1.0) * y;
            vertices.push(Vertex { pos: [c * r, y, s * r], normal: [0.0, normal, 0.0], uv: [0.5 + 0.5 * c, 0.5 + 0.5 * s] });
        }
        for i in 0..sides {
            let (a, b) = (first + i, first + (i + 1) % sides);
            indices.extend_from_slice(&if normal < 0.0 { [centre, a, b] } else { [centre, b, a] });
        }
    }
    PartGeometry { id, vertices, indices }
}

/// A closed unit ball: knuckles where stem runs meet and, stretched, the\n/// collar where a branch leaves its parent.
fn collar(id: u64) -> PartGeometry {
    let (rings, sides) = (4u32, 8u32);
    let mut vertices = vec![Vertex { pos: [0.0, -1.0, 0.0], normal: [0.0, -1.0, 0.0], uv: [0.5, 0.0] }];
    for r in 1..rings {
        let a = r as f32 / rings as f32 * std::f32::consts::PI;
        for i in 0..sides {
            let b = i as f32 / sides as f32 * TAU;
            let n = Vec3::new(a.sin() * b.cos(), -a.cos(), a.sin() * b.sin());
            vertices.push(Vertex { pos: n.to_array(), normal: n.to_array(), uv: [i as f32 / sides as f32, r as f32 / rings as f32] });
        }
    }
    vertices.push(Vertex { pos: [0.0, 1.0, 0.0], normal: [0.0, 1.0, 0.0], uv: [0.5, 1.0] });
    let top = vertices.len() as u32 - 1;
    let at = |r: u32, i: u32| 1 + (r - 1) * sides + i % sides;
    let mut indices = Vec::new();
    for i in 0..sides {
        indices.extend_from_slice(&[0, at(1, i), at(1, i + 1)]);
        indices.extend_from_slice(&[top, at(rings - 1, i + 1), at(rings - 1, i)]);
        for r in 1..rings - 1 {
            indices.extend_from_slice(&[at(r, i), at(r + 1, i), at(r + 1, i + 1), at(r, i), at(r + 1, i + 1), at(r, i + 1)]);
        }
    }
    PartGeometry { id, vertices, indices }
}

fn unit(variant: u64, index: u64, tag: u64) -> f32 {
    let mut x = variant.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ index.wrapping_mul(0xbf58_476d_1ce4_e5b9) ^ tag.wrapping_mul(0x94d0_49bb_1331_11eb);
    x = (x ^ (x >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    ((x ^ (x >> 32)) >> 40) as f32 / (1u64 << 24) as f32
}

/// A needle base vertex's uv: its outward direction octahedron-encoded,
/// with v shifted by -4 to mark it (raster.wgsl needle_widen widens the
/// needle to a minimum on-screen width along it). Needles have no texture.
pub fn needle_base_uv(out: Vec3) -> [f32; 2] {
    let p = out / (out.x.abs() + out.y.abs() + out.z.abs()).max(1e-12);
    let xy = if p.z >= 0.0 { [p.x, p.y] } else { [(1.0 - p.y.abs()) * p.x.signum(), (1.0 - p.x.abs()) * p.y.signum()] };
    [xy[0], xy[1] - 4.0]
}

/// A bottlebrush of long needles along the last 30 cm of a shoot (+Y, ending
/// at the origin), splaying out and forward; a stand-in until a proper mesh
/// replaces it.
fn tuft(id: u64, variant: u64, b: &Look) -> PartGeometry {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let (needles, length, reach) = (b.brush_needles.clamp(1, 2000), b.needle_length.clamp(0.01, 1.0), b.brush_reach.clamp(0.01, 1.0));
    let spread = b.brush_spread.clamp(0.2, 3.0);
    let (width, round) = (b.needle_width.clamp(0.0005, 0.02), b.brush_round.clamp(0.0, 1.0));
    // The shoot (+Y, ending at the origin) and `twigs` side twigs leaving it
    // forward and outward: the needles on a spruce spray then fan away from
    // one another instead of meeting at the shoot's tip (Joep, 9 November:
    // the plumes read convergent). Without twigs: the shoot alone (pine).
    let twigs = b.twigs.min(8) as usize;
    let mut shoots = vec![(Vec3::Y * -reach, Vec3::Y, reach)];
    for k in 0..twigs {
        let from = -reach * (0.2 + 0.45 * (k as f32 + 0.5) / twigs as f32);
        let around = (k as f32 / twigs as f32 + 0.15 * unit(variant, k as u64, 6)) * TAU;
        let lean = 0.55 + 0.3 * unit(variant, k as u64, 7);
        let dir = Vec3::new(lean.sin() * around.cos(), lean.cos(), lean.sin() * around.sin());
        shoots.push((Vec3::Y * from, dir, -from * 0.75));
    }
    // Side twigs carry their balls on a thin stalk (Joep, 9 November: the
    // plumes hang on twigs off a larger branch): a closed three-sided prism
    // in the needle material, so it needs no second material per brush.
    for &(start, dir, len) in shoots.iter().skip(1) {
        let r = width * 2.5;
        let s0 = dir.cross(if dir.y.abs() < 0.9 { Vec3::Y } else { Vec3::X }).normalize();
        let u0 = dir.cross(s0).normalize();
        let base = vertices.len() as u32;
        for end in [start, start + dir * len] {
            for k in 0..3 {
                let a = k as f32 / 3.0 * TAU;
                let out = s0 * a.cos() + u0 * a.sin();
                vertices.push(Vertex { pos: (end + out * r * if end == start { 1.0 } else { 0.6 }).to_array(), normal: out.to_array(), uv: needle_base_uv(out) });
            }
        }
        for k in 0..3u32 {
            let (a, b) = (base + k, base + (k + 1) % 3);
            indices.extend_from_slice(&[a, b, b + 3, a, b + 3, a + 3]);
        }
        indices.extend_from_slice(&[base, base + 2, base + 1, base + 3, base + 4, base + 5]);
    }
    let total: f32 = shoots.iter().map(|s| s.2).sum();
    for i in 0..needles {
        let v = i as u64;
        // The shoot a needle sits on, by length.
        let mut pick = unit(variant, v, 8) * total;
        let (start, dir, len) = *shoots.iter().find(|s| { pick -= s.2; pick <= 0.0 }).unwrap_or(&shoots[0]);
        // Fascicles along the shoot, denser near its tip:
        // `tip_spread` from the shoot at the tip, splaying out to `spread`
        // further back, a starburst round the bud as on a Jeffrey pine.
        let back = len * unit(variant, v, 5).powf(b.ball.clamp(0.5, 8.0));
        let t = back / len;
        let along = -back;
        let tip_spread = b.tip_spread.clamp(0.05, spread);
        let tilt = (tip_spread + (spread - tip_spread) * t.sqrt() + 0.3 * (unit(variant, v, 1) - 0.5)).max(0.05);
        let around = unit(variant, v, 2) * TAU;
        // Relative to the needle's own shoot.
        let (ds, du) = { let s = dir.cross(if dir.y.abs() < 0.9 { Vec3::Y } else { Vec3::X }).normalize(); (s, dir.cross(s).normalize()) };
        let axis = (dir * tilt.cos() + (ds * around.cos() + du * around.sin()) * tilt.sin()).normalize();
        let side = axis.cross(if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X }).normalize();
        let up = axis.cross(side).normalize();
        let root = start + dir * (len + along) + axis * 0.006;
        let long = length * (0.7 + 0.5 * unit(variant, v, 3));
        let tip = root + axis * long - Vec3::Y * 0.12 * long * unit(variant, v, 4);
        // A closed sliver (three-sided base, one tip): foliage optics need
        // every edge shared by two faces.
        // Normals lean toward the brush's outward direction (a ball round the
        // shoot's tip), so a brush shades as one mass instead of per needle.
        let ball = (tip - Vec3::Y * 0.5 * reach).normalize();
        let bend = |n: Vec3| (n * (1.0 - round) + ball * round).normalize_or(ball);
        let b = vertices.len() as u32;
        for k in 0..3 {
            let a = k as f32 / 3.0 * TAU;
            let out = side * a.cos() + up * a.sin();
            // The uv carries the widening direction for the raster's
            // minimum needle width (raster.wgsl needle_widen).
            vertices.push(Vertex { pos: (root + out * width).to_array(), normal: bend(out).to_array(), uv: needle_base_uv(out) });
        }
        vertices.push(Vertex { pos: tip.to_array(), normal: bend((tip - root).normalize()).to_array(), uv: [0.5, 1.0] });
        indices.extend_from_slice(&[b, b + 2, b + 1, b, b + 1, b + 3, b + 1, b + 2, b + 3, b + 2, b, b + 3]);
    }
    PartGeometry { id, vertices, indices }
}

impl Parts {
    /// Placeholder parts for a long-needled pine.
    pub fn long_needle_pine() -> Self { Self::for_look(&Look::jeffrey_pine()) }

    /// Pine parts with the look's needle brushes (brush_needles,
    /// needle_length, brush_spread, brush_reach).
    pub fn for_look(look: &Look) -> Self {
        const STEM: u64 = 0x5047_5354_0000_0000;
        const TUFT: u64 = 0x5047_5446_0000_0000;
        // The brush's shape is part of its identity (and so of every cook key).
        // 2: needle base uvs carry the widening direction.
        let mut h = TUFT ^ 2;
        for x in [u64::from(look.brush_needles), u64::from(look.needle_length.to_bits()), u64::from(look.brush_spread.to_bits()), u64::from(look.brush_reach.to_bits()), u64::from(look.needle_width.to_bits()), u64::from(look.brush_round.to_bits()), u64::from(look.tip_spread.to_bits()), u64::from(look.twigs), u64::from(look.ball.to_bits())] {
            h = (h ^ x).wrapping_mul(0x100_0000_01b3).rotate_left(17);
        }
        let tuft_id = (h & !0xff) | (TUFT & 0xff);
        Self {
            stems: std::array::from_fn(|k| stem(STEM + 1 + k as u64, if k < 4 { 6 } else { 10 }, TAPERS[k % 4])),
            collar: collar(STEM + 16),
            tufts: (0..3).map(|v| tuft(tuft_id + v, v, look)).collect(),
        }
    }
}

/// Members one branch may carry: virtual member ids hold 12 bits
/// (vegetation_asset), with a little headroom.
const MAX_BRANCH_MEMBERS: usize = 4000;

/// Limits for a grown plant: the recipe limits with room for an old pine's
/// ~5k stems and ~10k needle brushes.
pub fn limits() -> Limits {
    Limits { max_instances: 24576, ..crate::plant_recipe::recipe_limits() }
}

/// Colours of the wood and the needles, and the needle brushes
/// (Parts::needle_brushes takes the brush fields).
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Look {
    pub wood: [f32; 3],
    pub needles: [f32; 3],
    /// Needles in one bottlebrush.
    pub brush_needles: u32,
    /// Typical needle length, metres.
    pub needle_length: f32,
    /// How far needles splay from the shoot at the back of a brush, radians
    /// (forward at the tip; 1.6 is sideways, more points back).
    pub brush_spread: f32,
    /// Length of shoot the needles sit on behind its tip, metres.
    pub brush_reach: f32,
    /// Needle radius at its base, metres.
    pub needle_width: f32,
    /// How far needle normals bend toward the brush's outward direction, 0 to
    /// 1: 0 shades each needle on its own (speckles from afar), 1 shades the
    /// brush as one soft ball.
    pub brush_round: f32,
    /// How far needles splay from the shoot at the brush's tip, radians: 0.3
    /// closes the brush to a forward point (pine), ~1 radiates it (spruce;
    /// Joep, 9 November: the plumes read convergent).
    pub tip_spread: f32,
    /// Side twigs in each brush, leaving the shoot forward and outward with
    /// needles of their own (spruce sprays); 0 for a single shoot (pine).
    pub twigs: u32,
    /// How tightly needles crowd toward each shoot's tip: the exponent on
    /// their spread along it. 1.4 spreads them along the shoot; 3 and up
    /// gathers them into a ball at the tip, a pompom per twig (Joep, 9
    /// November: 'deze bomen zijn vooral bolletjes-gebaseerd').
    pub ball: f32,
}

impl Default for Look {
    fn default() -> Self { Self::jeffrey_pine() }
}

impl Look {
    pub fn jeffrey_pine() -> Self {
        Self { wood: [0.11, 0.065, 0.045], needles: [0.11, 0.21, 0.09], brush_needles: 50, needle_length: 0.22, brush_spread: 1.7, brush_reach: 0.2, needle_width: 0.0025, brush_round: 0.7, tip_spread: 0.3, twigs: 0, ball: 1.4 }
    }
    /// The swiss conifer: each brush a shoot forking into two short twigs, all
    /// three ending in a dense ball of needles radiating from the tip, the
    /// way Joep's references show pine branch ends (9 November). Needles are
    /// drawn long: true spruce length left the limbs and trunk showing.
    pub fn norway_spruce() -> Self {
        Self { wood: [0.09, 0.06, 0.045], needles: [0.05, 0.1, 0.045], brush_needles: 220, needle_length: 0.15, brush_spread: 1.9, brush_reach: 0.3, needle_width: 0.0012, brush_round: 0.8, tip_spread: 0.45, twigs: 2, ball: 3.5 }
    }
}

/// What an assembled plant came to.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub bones: usize,
    pub stems: usize,
    pub tufts: usize,
    /// Tufts left out to stay inside the placement limit.
    pub tufts_dropped: usize,
}

/// A product of affine matrices with its last row exact again.
fn affine(mut m: Mat4) -> Mat4 {
    m.x_axis.w = 0.0;
    m.y_axis.w = 0.0;
    m.z_axis.w = 0.0;
    m.w_axis.w = 1.0;
    m
}

fn along(from: Vec3, to: Vec3) -> Quat {
    Quat::from_rotation_arc(Vec3::Y, (to - from).normalize_or(Vec3::Y))
}

/// Long stem runs from year segments: consecutive shoots of one axis merge
/// until `reach` metres, so a tree is tens of trunk pieces, not one a year.
fn runs(plant: &Plant, reach: f32) -> Vec<(u32, Vec3, Vec3, f32, f32, bool)> {
    let n = plant.segments.len();
    let mut next = vec![None; n];
    for (i, s) in plant.segments.iter().enumerate() {
        if let Some(p) = s.parent {
            if plant.segments[p as usize].order == s.order {
                next[p as usize] = Some(i);
            }
        }
    }
    let mut out = Vec::new();
    for (i, s) in plant.segments.iter().enumerate() {
        // Start at axis starts only; the walk covers the rest of the axis.
        if s.parent.is_some_and(|p| plant.segments[p as usize].order == s.order) {
            continue;
        }
        let (mut start, mut radius, mut last, mut at) = (s.from, s.radius[0], i, i);
        while let Some(n) = next[at] {
            at = n;
            if plant.segments[at].to.distance(start) >= reach {
                out.push((last as u32, start, plant.segments[at].to, radius, plant.segments[at].radius[1], last == i));
                start = plant.segments[at].to;
                radius = plant.segments[at].radius[0];
                last = at;
            }
        }
        let end = plant.segments[at].to;
        if end.distance(start) > 1e-3 {
            out.push((last as u32, start, end, radius, plant.segments[at].radius[1], last == i));
        }
    }
    out
}

/// Build `plant` from `parts`. Main limbs with the most foliage get bones (up
/// to `limits.max_branches`); everything else rides the trunk's. Twig runs grow
/// and tufts thin only as far as `limits.max_instances` demands.
pub fn assemble(plant: &Plant, parts: &Parts, look: Look, seed: u64, limits: Limits) -> Result<(Assembly, Counts), String> {
    assemble_shoots(plant, parts, look, seed, limits, None)
}

/// Optional coloured source components at the same attachment as each leaf
/// mesh. Part indices are relative to Parts::tufts. The first variants.len()
/// meshes are the randomly selectable base foliage, the rest are companions.
pub(crate) struct ShootLayers<'a> {
    pub variants: &'a [&'a [(usize, usize)]],
    pub materials: &'a [Material],
}

pub(crate) fn assemble_shoots(plant: &Plant, parts: &Parts, look: Look, seed: u64, limits: Limits,
    layers: Option<ShootLayers<'_>>) -> Result<(Assembly, Counts), String> {
    let variants = layers.as_ref().map_or(parts.tufts.len().max(1), |l| l.variants.len());
    let members = 1 + layers.as_ref().and_then(|l| l.variants.iter().map(|m| m.len()).max()).unwrap_or(0);
    if layers.is_some() && (variants == 0 || variants > parts.tufts.len()) {
        return Err("invalid grown shoot library".into());
    }
    let n = plant.segments.len();
    // The limb each segment belongs to: the first order-1 segment on its way up.
    let mut limb = vec![u32::MAX; n];
    for (i, s) in plant.segments.iter().enumerate() {
        limb[i] = match s.parent {
            _ if s.order == 0 => u32::MAX,
            Some(p) if plant.segments[p as usize].order == 0 => i as u32,
            Some(p) => limb[p as usize],
            None => u32::MAX,
        };
    }
    let mut foliage = std::collections::HashMap::<u32, u32>::new();
    for t in &plant.tufts {
        let l = limb[t.segment as usize];
        if l != u32::MAX {
            *foliage.entry(l).or_default() += 1;
        }
    }
    let mut ranked: Vec<(u32, u32)> = foliage.into_iter().map(|(l, c)| (c, l)).collect();
    ranked.sort_unstable_by(|a, b| b.cmp(a));
    ranked.truncate(limits.max_branches.saturating_sub(1));
    ranked.sort_unstable_by_key(|r| r.1);
    let mut branches = vec![BranchGroup { id: seed ^ 0x4252_414e_4348, parent: None, rest: Mat4::IDENTITY, stiffness: 1.0 }];
    let mut bone = std::collections::HashMap::new();
    for (_, l) in &ranked {
        let s = plant.segments[*l as usize];
        bone.insert(*l, branches.len() as u32);
        branches.push(BranchGroup {
            id: seed ^ 0x4252_414e_4348 ^ ((*l as u64 + 1) << 20),
            parent: Some(0),
            rest: Mat4::from_rotation_translation(along(s.from, s.to), s.from),
            // Wind sway bounds every voxel level's error by angle x limb length:
            // at 0.6 a 10 m limb added ~0.9 m, so the camera never left the
            // source mesh and its splats filled the budget. Stiff, long limbs.
            stiffness: 0.92,
        });
    }
    let inverse: Vec<Mat4> = branches.iter().map(|b| b.rest.inverse()).collect();
    let bone_of = |segment: u32| limb.get(segment as usize).and_then(|l| bone.get(l)).copied().unwrap_or(0);

    let mut reach = 0.6;
    let stems = loop {
        let r = runs(plant, reach);
        if r.len() * 3 <= limits.max_instances || reach > 64.0 {
            break r;
        }
        reach *= 1.5;
    };
    // Rounded joints: a ball where one run of an axis meets the next and at
    // the end of a thick stem, a stretched ball where a branch leaves its
    // parent. Twigs under 2.5 cm get none: their seams are too small to see,
    // and they are most of the stems.
    const JOINT_RADIUS: f32 = 0.025;
    // One bark for every stem from 3 cm up (bands where a trunk crossed a
    // size class showed as seams), smooth young twigs below (materials 0 and
    // 2; 1 is the needles).
    let wood = |radius: f32| if radius >= 0.03 { 0 } else { 2 };

    let taper_of = |radius: f32, end: f32| {
        let wanted = (end / radius.max(1e-6)).clamp(0.6, 1.0);
        (0..4).min_by(|&a, &c| (TAPERS[a] - wanted).abs().total_cmp(&(TAPERS[c] - wanted).abs())).unwrap_or(0)
    };
    let continues = |i: usize| stems.get(i + 1).is_some_and(|r| !r.5);
    let collars: usize = (0..stems.len()).map(|i| {
        let r = stems[i];
        usize::from(r.3 > JOINT_RADIUS && (r.5 && plant.segments[r.0 as usize].order > 0 || !r.5)) + usize::from(!continues(i) && r.4 > JOINT_RADIUS)
            + usize::from(r.5 && plant.segments[r.0 as usize].order == 0 && r.3 > JOINT_RADIUS)
    }).sum();
    let room = limits.max_instances.saturating_sub(stems.len() + collars) / members;
    let keep = (room as f32 / plant.tufts.len().max(1) as f32).min(1.0);
    let mut instances = Vec::with_capacity(limits.max_instances);
    for (i, &(segment, from, to, radius, end, first)) in stems.iter().enumerate() {
        let b = bone_of(segment);
        // Overlap the next run a little so the rings meet without a seam.
        let length = from.distance(to) + radius.min(0.05);
        let world = Mat4::from_scale_rotation_translation(Vec3::new(radius, length, radius), along(from, to), from);
        let taper = taper_of(radius, end);
        let part = taper as u32 + if radius > 0.03 { 4 } else { 0 };
        let bark = wood(radius);
        instances.push(PartInstance { id: seed ^ (0x5354 << 40) ^ i as u64, part, branch: b, rest: affine(inverse[b as usize] * world), material: bark });
        let mut joint = |tag: u64, scale: Vec3, centre: Vec3| {
            let world = Mat4::from_scale_rotation_translation(scale, along(from, to), centre);
            instances.push(PartInstance { id: seed ^ (tag << 40) ^ i as u64, part: 8, branch: b, rest: affine(inverse[b as usize] * world), material: bark });
        };
        let axis = (to - from).normalize_or(Vec3::Y);
        if first && plant.segments[segment as usize].order == 0 && radius > JOINT_RADIUS {
            // Root flare: the trunk swells into the ground.
            joint(0x5246, Vec3::new(radius * 1.7, radius * 2.2, radius * 1.7), from);
        }
        if radius <= JOINT_RADIUS {
            // A twig: no joints.
        } else if first && plant.segments[segment as usize].order > 0 {
            // A branch collar: a swelling around the base, embedded in the
            // parent and easing into the branch over a few of its radii.
            joint(0x434c, Vec3::new(radius * 1.45, radius * 2.6, radius * 1.45), from + axis * radius * 0.8);
        } else if !first {
            // A knuckle as wide as the wider of the two meeting rings hides
            // the step and the crack where the axis bends.
            let before = stems[i - 1];
            let ring = (before.3 * TAPERS[taper_of(before.3, before.4)]).max(radius) * 1.03;
            joint(0x4b4e, Vec3::splat(ring), from);
        }
        if !continues(i) && end > JOINT_RADIUS {
            // A rounded end instead of a flat cap.
            joint(0x454e, Vec3::splat(radius * TAPERS[taper]), from + axis * length);
        }
    }
    // A branch draws its members as virtual instances with 12-bit member ids
    // (vegetation_asset): crowded rules (many limbs riding the trunk's bone,
    // many plumes) thin that branch's brushes to fit, the kept ones larger.
    let mut on_branch = vec![0usize; branches.len()];
    for inst in &instances { on_branch[inst.branch as usize] += 1; }
    let mut wanted = vec![0usize; branches.len()];
    for t in &plant.tufts { wanted[bone_of(t.segment) as usize] += 1; }
    let branch_keep: Vec<f32> = (0..branches.len()).map(|b| {
        let room = (MAX_BRANCH_MEMBERS.saturating_sub(on_branch[b]) / members) as f32;
        (room / (wanted[b] as f32 * keep).max(1.0)).min(1.0)
    }).collect();
    let mut placed = 0;
    for (i, t) in plant.tufts.iter().enumerate() {
        let b = bone_of(t.segment);
        let kept = keep * branch_keep[b as usize];
        // Kept at random to spread the thinning; the draw can overshoot the
        // room by chance, so stop at the limits instead of failing the plant.
        if unit(seed, i as u64, 9) >= kept || instances.len() + members > limits.max_instances || on_branch[b as usize] + members > MAX_BRANCH_MEMBERS {
            continue;
        }
        // Kept tufts grow to cover for the ones the limits leave out.
        // Grow a little for the left-out ones, not into balls.
        let size = (0.75 + 0.45 * t.fullness) * (1.0 / kept.sqrt()).min(1.15);
        let twist = Quat::from_axis_angle(Vec3::Y, unit(seed, i as u64, 10) * TAU);
        let world = Mat4::from_scale_rotation_translation(Vec3::splat(size), along(Vec3::ZERO, t.axis) * twist, t.at);
        let variant = (unit(seed, i as u64, 11) * variants as f32) as usize % variants;
        let first = (parts.stems.len() + 1) as u32;
        let rest = affine(inverse[b as usize] * world);
        let id = seed ^ (0x5446 << 40) ^ i as u64;
        instances.push(PartInstance { id, part: first + variant as u32, branch: b, rest, material: 1 });
        on_branch[b as usize] += 1;
        if let Some(layers) = &layers {
            for (index, &(part, material)) in layers.variants[variant].iter().enumerate() {
                instances.push(PartInstance { id: id ^ ((index as u64 + 1) << 56), part: first + part as u32,
                    branch: b, rest, material: material as u32 });
                on_branch[b as usize] += 1;
            }
        }
        placed += 1;
    }
    let counts = Counts { bones: branches.len(), stems: stems.len() + collars, tufts: placed, tufts_dropped: plant.tufts.len() - placed };
    // Wood: procedural bark (shade.wgsl bark_surface) with ridges `plate`
    // metres wide; the tint stays the plates' and furrows' mean, for voxels.
    // Twigs (plate 0) are plain smooth grey-brown wood.
    let bark = |plate: f32| Material {
        tint: if plate > 0.0 { look.wood } else { [look.wood[0] * 1.1, look.wood[1] * 1.05, look.wood[2] * 1.05] },
        layer: if plate > 0.0 { crate::scene::BARK_LAYER } else { -1 },
        roughness: 0.96,
        flags: 0,
        scale: if plate > 0.0 { plate } else { 1.0 },
        metal: 0.0,
    };
    let needles = Material { tint: look.needles, layer: -1, roughness: 0.8, flags: THIN_FOLIAGE | TWO_SIDED, scale: 1.0, metal: 0.0 };
    let geometry = parts.stems.iter().chain(std::iter::once(&parts.collar)).chain(&parts.tufts).cloned().collect();
    let parameters = Parameters { seed, archetype: Archetype::ConiferTwig, growth: 1.0, season: 0.5, density: placed.max(1) as u32 };
    let mut materials = vec![bark(0.10), needles, bark(0.0)];
    if let Some(layers) = layers {
        materials.extend_from_slice(layers.materials);
    }
    let assembly = Assembly::from_parts(parameters, geometry, branches, instances, materials, limits)?;
    Ok((assembly, counts))
}

#[cfg(test)]
mod tests {
    #[test]
    fn needle_base_uvs_decode_to_their_direction() {
        // The decode in raster.wgsl needle_widen.
        for out in [Vec3::X, -Vec3::Y, Vec3::new(0.3, -0.5, -0.8).normalize(), Vec3::new(-0.6, 0.2, 0.77).normalize()] {
            let [x, y] = needle_base_uv(out);
            assert!(y < -2.5);
            let e = (x, y + 4.0);
            let mut d = Vec3::new(e.0, e.1, 1.0 - e.0.abs() - e.1.abs());
            if d.z < 0.0 { d = Vec3::new((1.0 - d.y.abs()) * d.x.signum(), (1.0 - d.x.abs()) * d.y.signum(), d.z); }
            assert!(d.normalize().dot(out) > 0.999, "{out} -> {d}");
        }
    }

    use super::*;
    use crate::plant_growth::{grow, Species};

    #[test]
    fn a_crowded_crown_thins_its_brushes_instead_of_failing() {
        // Pine-lab tuning that once failed at 31 years: more brushes than room,
        // and the random thinning drew a few too many.
        let species = Species { crowding: 60, height: 52.0, laterals: 2.0, whorl: [2, 12], needle_years: 7, plumes: 6.0, ..Species::jeffrey_pine() };
        let parts = Parts::long_needle_pine();
        for age in [31, 55] {
            for seed in 1..6 {
                let (assembly, counts) = assemble(&grow(&species, age, seed), &parts, Look::jeffrey_pine(), seed, limits()).unwrap();
                assert!(assembly.instances.len() <= limits().max_instances);
                assert!(counts.tufts_dropped > 0);
                // Virtual member ids: every branch fits 12 bits.
                let mut per = vec![0usize; assembly.branches.len()];
                for inst in &assembly.instances { per[inst.branch as usize] += 1; }
                assert!(per.iter().all(|&n| n <= MAX_BRANCH_MEMBERS), "{per:?}");
            }
        }
    }

    #[test]
    fn every_life_stage_fits_the_grown_limits() {
        let (species, parts) = (Species::jeffrey_pine(), Parts::long_needle_pine());
        for age in [1, 3, 6, 12, 25, 50, 80, 120, 200, 300] {
            let plant = grow(&species, age, 1);
            let (assembly, counts) = assemble(&plant, &parts, Look::jeffrey_pine(), 1, limits()).unwrap();
            assert_eq!(assembly.instances.len(), counts.stems + counts.tufts);
            assert!(counts.bones <= 64);
        }
    }

    #[test]
    fn parts_are_closed_with_outward_volume() {
        let parts = Parts::long_needle_pine();
        for part in parts.stems.iter().chain(std::iter::once(&parts.collar)).chain(&parts.tufts) {
            let mut edges = std::collections::BTreeMap::new();
            let mut volume = 0.0f64;
            for tri in part.indices.chunks_exact(3) {
                let p: [Vec3; 3] = std::array::from_fn(|i| Vec3::from(part.vertices[tri[i] as usize].pos));
                volume += p[0].as_dvec3().dot(p[1].as_dvec3().cross(p[2].as_dvec3())) / 6.0;
                for i in 0..3 {
                    let (a, b) = (p[i].to_array().map(f32::to_bits), p[(i + 1) % 3].to_array().map(f32::to_bits));
                    let (key, dir) = if a < b { ((a, b), 1i32) } else { ((b, a), -1) };
                    let e = edges.entry(key).or_insert((0u32, 0i32));
                    e.0 += 1;
                    e.1 += dir;
                }
            }
            assert!(volume > 0.0, "part {:x}: volume {volume}", part.id);
            assert!(edges.values().all(|&e| e == (2, 0)), "part {:x} is not closed", part.id);
        }
    }

    #[test]
    fn an_older_tree_is_more_parts_not_bigger_parts() {
        let (species, parts) = (Species::jeffrey_pine(), Parts::long_needle_pine());
        let limits = limits();
        let (young, _) = assemble(&grow(&species, 6, 1), &parts, Look::jeffrey_pine(), 1, limits).unwrap();
        let (old, _) = assemble(&grow(&species, 80, 1), &parts, Look::jeffrey_pine(), 1, limits).unwrap();
        assert_eq!(young.parts.len(), old.parts.len());
        assert!(old.instances.len() > 10 * young.instances.len());
    }
}
