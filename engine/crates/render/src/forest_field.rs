//! The forest as a function, like `terrain_field`: where trees grow and what
//! the forest looks like from far away, evaluated the same way on the CPU
//! (placing tree instances, gameplay) and on the GPU (`shaders/forest_field.wgsl`,
//! line for line the same, after `terrain_field.wgsl`), so a distant forest
//! drawn into the terrain stands exactly where the trees are.
//!
//! Trees sit on a jittered 8 m grid. Stands follow the ground (`site`):
//! forest on slopes and ridges, meadows on flat and damp ground, with noise
//! from 300 m stands down to 12 m ragged edges, loose trees and hedgerows in
//! the clearings. Nothing grows on rock, scree, water or snow, on slopes
//! steeper than ~35 degrees, or within 250 m of the snow line. `density`
//! (0..1) is the share of forest ground that grows a tree.
//!
//! ```ignore
//! let trees = forest_field::trees_in_disc(Vec2::ZERO, 900.0, seed, &style, 0.6);
//! let far = forest_field::forest(x, z, seed, &style, 0.6, &fs); // colour, cover, crown height
//! ```
use crate::terrain_field::{self, Sample, Style};
use glam::{Vec2, Vec3};

pub const WGSL: &str = include_str!("shaders/forest_field.wgsl");

/// Metres between tree spots.
pub const CELL: f32 = 8.0;
/// How a forest looks, chosen by the game's artist: the palette and the
/// crown shape. Passed to the GPU as is (`ForestStyle` in forest_field.wgsl,
/// four vec4s). The default follows aerial photos of Vosges conifer forest:
/// dark blue-green with lit crown tips, nearly black between crowns, olive
/// and brown patches; clearly darker than the meadows.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ForestStyle {
    /// Linear albedo of sunlit needles; w: per-tree tone variation (0.4: each
    /// tree 0.8 to 1.2 times as bright).
    pub needles: [f32; 4],
    /// Linear albedo of the forest floor seen in the gaps; w: the gaps'
    /// occlusion under a closed canopy (sparser stands leave their gaps lit).
    pub floor: [f32; 4],
    /// Linear albedo of a second canopy in patches (broadleaf, young or
    /// dying trees); w: the height of a full-size tree's crown top above
    /// its foot (metres).
    pub far: [f32; 4],
    /// x: crown radius seen from above per unit of tree width (metres);
    /// y: crown area per tree spot over CELL^2 (cover from straight above is
    /// 1 - exp(-chance y), as overlapping crowns hide ground); z: how far a crown's normal leans out from the trunk (cone
    /// steepness); w: share of the forest in the second canopy's patches.
    pub crown: [f32; 4],
}

impl ForestStyle {
    /// This style with its crowns measured from the plants the forest is
    /// drawn with (their whole-plant voxel proxies, at unit scale, each as
    /// common as the others): sunlit albedo, crown top height and the crown
    /// area seen from straight above. The painted far forest then has the
    /// near trees' cover and colour instead of a generic closed canopy
    /// (Joep, 8 November: far grown pines were flat green patches).
    pub fn measured(self, proxies: &[&crate::foliage_voxel::VoxelAsset]) -> Self {
        let (mut area, mut height, mut albedo, mut weight, mut n) = (0.0f64, 0.0f64, [0.0f64; 3], 0.0f64, 0usize);
        for asset in proxies {
            let Some(level) = asset.levels.first() else { continue };
            let c = f64::from(level.cell_size);
            let mut columns: std::collections::HashMap<(u32, u32), f64> = std::collections::HashMap::new();
            for v in &level.voxels {
                // Optical depth straight down through the cell (axis_image).
                let tau = f64::from(v.directional_area[1]) / (c * c);
                *columns.entry((v.coord[0], v.coord[2])).or_insert(0.0) += tau;
                let w = 1.0 - (-tau).exp();
                for k in 0..3 { albedo[k] += w * f64::from(v.albedo[k]); }
                weight += w;
            }
            area += columns.values().map(|&tau| (1.0 - (-tau).exp()) * c * c).sum::<f64>();
            height += f64::from(asset.source_bounds[1][1] - asset.source_bounds[0][1]);
            n += 1;
        }
        if n == 0 || weight <= 0.0 { return self; }
        let (area, height) = (area / n as f64, height / n as f64);
        let a = albedo.map(|x| (x / weight) as f32);
        let mut style = self;
        style.needles = [a[0], a[1], a[2], self.needles[3]];
        style.far[3] = height as f32;
        style.crown[0] = (area / std::f64::consts::PI).sqrt() as f32;
        style.crown[1] = (area / f64::from(CELL * CELL)) as f32;
        style
    }
}

impl Default for ForestStyle {
    fn default() -> Self {
        ForestStyle {
            needles: [0.052, 0.097, 0.027, 0.5],
            floor: [0.02, 0.029, 0.009, 0.15],
            far: [0.085, 0.09, 0.035, 15.0],
            crown: [3.2, 0.375, 1.6, 0.2],
        }
    }
}
/// The terrain detail cut the forest samples at: within a few centimetres of
/// cut 0 on the CPU, but defined on the GPU (terrain_field's fade divides by
/// the cut, and x / 0 is not portable WGSL: Metal gave other ground).
pub const CUT: f32 = 0.05;
/// A `site` where no forest grows.
pub const NO_FOREST: f32 = -10.0;

/// The forest at a point, for drawing it where single trees are too small.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Forest {
    /// Share of the ground covered by tree spots that grow a tree, 0..1.
    pub density: f32,
    /// Linear crown albedo here (stands vary slightly).
    pub colour: Vec3,
    /// Height of the crowns above the ground (metres), 0 without forest.
    pub crown_height: f32,
}

/// One tree: foot position, rotation about Y, scale (x = z width, y height)
/// and which of the game's variants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tree {
    pub pos: Vec3,
    pub yaw: f32,
    pub scale: Vec3,
    pub variant: u32,
}

/// A uniform 0..1 number for grid cell (i, j), channel k.
pub(crate) fn hash(i: i32, j: i32, k: u32, seed: u32) -> f32 {
    let mut h = (i as u32).wrapping_mul(0x8da6b343) ^ (j as u32).wrapping_mul(0xd8163841) ^ seed.wrapping_mul(0xcb1ab31f) ^ k.wrapping_mul(0x165667b1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b3c6d);
    h ^= h >> 12;
    (h & 0xffffff) as f32 / 16777216.0
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of a pattern `scale` metres across a pixel `footprint` metres
/// across shows: all of it over four pixels, none under two (its mean, 0).
fn band(scale: f32, footprint: f32) -> f32 {
    (scale / footprint.max(1e-3) * 0.5 - 1.0).clamp(0.0, 1.0)
}

/// The ground's own leaning towards forest, from the terrain's `up` and
/// `wet` (Sample, or the terrain's vertex attributes): trees follow water,
/// thin out on steep slopes and stop in standing wet. The ground's colour
/// follows the same stands (terrain_surface.wgsl), so forest grows on green
/// ground and the dry, sandy ground stays open (Joep, 9 November: it was
/// the other way round, forest on dry slopes and meadow in damp hollows).
/// `slopes` (terrain_field::Style::forest_slopes): 1 puts the forest on the
/// steep valley sides and leaves gentle ground to pasture (swiss).
pub fn site(up: f32, wet: f32, slopes: f32) -> f32 {
    let steep = 1.0 - smooth(0.86, 0.97, up);
    let slope = -0.7 * steep + (0.1 * steep - 0.5 * smooth(0.96, 0.995, up) + 0.7 * steep) * slopes;
    0.05 + 0.6 * wet - 1.6 * (wet - 0.75).max(0.0) + slope
}

/// Hedgerows: stretches of the edges of 110 by 80 m fields (on a grid
/// turned by the seed), about a tree wide, 0..1. The edges meander up to
/// ~12 m and fewer carry a hedge: ruler-straight rows of trees read as
/// planted (Joep, 8 November).
fn hedgerow(x: f32, z: f32, seed: u32) -> f32 {
    let a = hash(0, 0, 10, seed) * 1.5;
    let p = Vec2::new(x, z);
    let wu = 12.0 * terrain_field::noised(p / 90.0 + Vec2::new(5.3, 2.9), seed.wrapping_add(83)).x;
    let wv = 12.0 * terrain_field::noised(p / 90.0 + Vec2::new(1.7, 9.1), seed.wrapping_add(84)).x;
    let (u, v) = (x * a.cos() + z * a.sin() + wu, z * a.cos() - x * a.sin() + wv);
    let (fu, fv) = (u / 110.0, v / 80.0);
    let (iu, iv) = (fu.round(), fv.round());
    // Distance to the nearest field edge across u and across v (metres), and
    // whether that stretch of edge carries a hedge.
    let du = if hash(iu as i32, fv.floor() as i32, 11, seed) < 0.3 { (fu - iu).abs() * 110.0 } else { 1e3 };
    let dv = if hash(fu.floor() as i32, iv as i32, 12, seed) < 0.3 { (fv - iv).abs() * 80.0 } else { 1e3 };
    (1.0 - du.min(dv) / 4.0).clamp(0.0, 1.0)
}

/// Stands and clearings: 300 m stands with ragged edges down to 12 m, on
/// the ground's `site`. Octaves under two `footprint`s are left out.
fn stand(x: f32, z: f32, seed: u32, site: f32, footprint: f32) -> f32 {
    let p = Vec2::new(x, z);
    let mut s = terrain_field::noised(p / 300.0, seed.wrapping_add(77)).x + site;
    let b = band(95.0, footprint);
    if b > 0.0 { s += 0.45 * b * terrain_field::noised(p / 95.0 + Vec2::new(3.7, 1.3), seed.wrapping_add(78)).x; }
    let b = band(33.0, footprint);
    if b > 0.0 { s += 0.25 * b * terrain_field::noised(p / 33.0 + Vec2::new(8.1, 4.9), seed.wrapping_add(79)).x; }
    let b = band(12.0, footprint);
    if b > 0.0 { s += 0.14 * b * terrain_field::noised(p / 12.0 + Vec2::new(2.3, 6.2), seed.wrapping_add(80)).x; }
    s
}

/// The share of tree spots at (x, z) that grow a tree, before `density`:
/// 1 in a stand, falling to 0 over a few metres at its edge, with loose
/// trees and hedgerows in the clearings. 0 on a `NO_FOREST` site. For
/// drawing stands, `footprint` is the pixel's size in metres; 0 for trees.
pub fn share(x: f32, z: f32, seed: u32, site: f32, footprint: f32) -> f32 {
    if site <= NO_FOREST { return 0.0; }
    let inside = ((stand(x, z, seed, site, footprint) + 0.05) * 5.0).clamp(0.0, 1.0);
    let rows = 0.8 * band(16.0, footprint) * hedgerow(x, z, seed);
    inside.max(0.04).max(rows)
}

/// Whether ground can carry a tree.
pub(crate) fn grows(s: &Sample, style: &Style) -> bool {
    !(s.rock > 0.3 || s.scree > 0.3 || s.water > 0.01 || s.snow > 0.05 || s.up < 0.82 || s.pos.y > style.snow_line - 250.0)
}

/// Where the second canopy grows instead, 0..1.
fn patch(x: f32, z: f32, seed: u32, amount: f32) -> f32 {
    if amount <= 0.0 { return 0.0; }
    let n = terrain_field::noised(Vec2::new(x, z) / 70.0 + Vec2::new(1.9, 7.3), seed.wrapping_add(82)).x;
    let t = 0.3 - 0.6 * amount;
    smooth(t - 0.04, t + 0.04, n)
}

/// The forest at (x, z).
pub fn forest(x: f32, z: f32, seed: u32, style: &Style, density: f32, fs: &ForestStyle) -> Forest {
    let s = terrain_field::eval(x, z, CUT, seed, style);
    let site = if grows(&s, style) { site(s.up, s.wet, style.forest_slopes) } else { NO_FOREST };
    let cover = share(x, z, seed, site, 0.0) * density;
    let tone = (stand(x, z, seed, site.max(-1.0), 0.0) * 0.5 + 0.5).clamp(0.0, 1.0);
    let colour = Vec3::from_slice(&fs.needles[..3]).lerp(Vec3::from_slice(&fs.far[..3]), patch(x, z, seed, fs.crown[3]));
    Forest {
        density: cover,
        colour: colour * (0.8 + 0.4 * tone),
        crown_height: if cover > 0.0 { fs.far[3] * (0.85 + 0.3 * tone) } else { 0.0 },
    }
}

/// The tree of grid cell (i, j), if it grows one.
pub fn tree(i: i32, j: i32, seed: u32, style: &Style, density: f32) -> Option<Tree> {
    let roll = hash(i, j, 3, seed);
    if roll >= density { return None; }
    let (x, z) = ((i as f32 + hash(i, j, 1, seed)) * CELL, (j as f32 + hash(i, j, 2, seed)) * CELL);
    let s = terrain_field::eval(x, z, CUT, seed, style);
    if !grows(&s, style) { return None; }
    if roll >= share(x, z, seed, site(s.up, s.wet, style.forest_slopes), 0.0) * density { return None; }
    let width = 0.75 + 0.5 * hash(i, j, 4, seed);
    let height = width * (0.9 + 0.25 * hash(i, j, 5, seed));
    Some(Tree {
        pos: Vec3::new(x, s.pos.y - 0.2, z),
        yaw: hash(i, j, 6, seed) * std::f32::consts::TAU,
        scale: Vec3::new(width, height, width),
        variant: (hash(i, j, 7, seed) * 3.0) as u32 % 3,
    })
}

/// The canopy seen from above at one point, for drawing a forest whose trees
/// are too small to model: albedo, the crown surface's normal (so the sun
/// side lights and the far side falls dark), occlusion (1 on a crown tip,
/// low in the gaps between crowns) and how much of the point is crown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canopy {
    pub albedo: Vec3,
    pub normal: Vec3,
    pub occlusion: f32,
    pub cover: f32,
}

/// Mean occlusion over a crown: the cone's mean tip height is 1/3, and its
/// flanks face the sun half of the time.
const CROWN_OCCLUSION: f32 = 0.6 + 0.4 * (1.0 / 3.0) * 0.8;

/// Cell (i, j)'s tree in a stand where `chance` of the spots grow one,
/// without the terrain: centre, width, tint and crown phase.
fn spot(i: i32, j: i32, seed: u32, chance: f32, variation: f32) -> Option<(Vec2, f32, f32, f32)> {
    if hash(i, j, 3, seed) >= chance { return None; }
    let c = Vec2::new((i as f32 + hash(i, j, 1, seed)) * CELL, (j as f32 + hash(i, j, 2, seed)) * CELL);
    let width = 0.75 + 0.5 * hash(i, j, 4, seed);
    Some((c, width, 1.0 + variation * (hash(i, j, 8, seed) - 0.5), hash(i, j, 9, seed) * std::f32::consts::TAU))
}

/// How much wider a crown looks seen from `eye` (unit, towards the viewer)
/// than from straight above: its side, crown length x cot(elevation), adds
/// to its top's area (capped where the 3x3 spots end).
fn crown_growth(eye: Vec3, fs: &ForestStyle) -> f32 {
    let cot = (Vec2::new(eye.x, eye.z).length() / eye.y.max(1e-3)).min(4.0);
    (1.0 + 0.8 * fs.far[3] * cot / (std::f32::consts::PI * fs.crown[0])).sqrt().min(7.0 / fs.crown[0])
}

/// The canopy at (x, z) for a pixel `footprint` metres across, lit from
/// `sun` and seen from `eye` (units, towards each), on ground of `site`
/// (`site()`, or `NO_FOREST`). Up close the crowns of the 3x3 nearest spots
/// are cones, as wide as their side looks from `eye`; as the footprint grows
/// past a crown they fade to their average, textured by clumps and gaps for
/// as long as those span a few pixels.
#[allow(clippy::too_many_arguments)]
pub fn canopy(x: f32, z: f32, footprint: f32, sun: Vec3, eye: Vec3, seed: u32, density: f32, site: f32, fs: &ForestStyle) -> Canopy {
    let floor = Vec3::from_slice(&fs.floor[..3]);
    let needles = Vec3::from_slice(&fs.needles[..3]).lerp(Vec3::from_slice(&fs.far[..3]), patch(x, z, seed, fs.crown[3]));
    let crowd = share(x, z, seed, site, footprint) * density;
    let grow = crown_growth(eye, fs);
    // Crowns overlap at random: the share of ground they hide.
    let mean_cover = 1.0 - (-crowd * fs.crown[1] * grow * grow).exp();
    // Gaps sit in their neighbours' shade only where the stand is dense.
    let gap = 1.0 - (1.0 - fs.floor[3]) * (crowd * 1.25).min(1.0);
    // The same light as the crowns and gaps it stands for (albedo x occlusion).
    let albedo = floor.lerp(needles, mean_cover);
    let light = needles.y * CROWN_OCCLUSION * mean_cover + floor.y * gap * (1.0 - mean_cover);
    // Clumps of crowns and the gaps between them, a few metres high.
    let p = Vec2::new(x, z);
    let (wa, wb) = (band(23.0, footprint), band(9.0, footprint));
    let a = if wa > 0.0 { terrain_field::noised(p / 23.0 + Vec2::new(4.4, 0.7), seed.wrapping_add(83)) * wa } else { Vec3::ZERO };
    let b = if wb > 0.0 { terrain_field::noised(p / 9.0 + Vec2::new(0.3, 5.8), seed.wrapping_add(84)) * wb } else { Vec3::ZERO };
    let rise = crowd.min(1.0);
    let slope = (Vec2::new(a.y, a.z) * (3.0 / 23.0) + Vec2::new(b.y, b.z) * (1.5 / 9.0)) * rise;
    let clump = (1.0 + 0.6 * (a.x + 0.5 * b.x) * rise).max(0.2);
    let mean = Canopy { albedo, normal: Vec3::new(-slope.x, 1.0, -slope.y).normalize(), occlusion: light / albedo.y * clump, cover: mean_cover };
    // Average once a pixel spans a good part of a crown, before it aliases.
    let far = ((footprint - CELL * 0.08) / (CELL * 0.25)).clamp(0.0, 1.0);
    if far >= 1.0 || crowd <= 0.0 { return mean; }
    let (ci, cj) = ((x / CELL).floor() as i32, (z / CELL).floor() as i32);
    let mut best = (0.0f32, Vec2::ZERO, 0.0f32, 1.0f32);
    for dj in -1..=1 {
        for di in -1..=1 {
            let Some((c, width, tint, phase)) = spot(ci + di, cj + dj, seed, crowd, fs.needles[3]) else { continue };
            let d = p - c;
            // A spruce from above is a star of branch tiers, not a disc.
            let radius = fs.crown[0] * grow * width * (0.82 + 0.18 * (7.0 * d.y.atan2(d.x) + phase).sin());
            let h = (1.0 - d.length() / radius) * width * grow;
            if h > best.0 { best = (h, d, width, tint); }
        }
    }
    let (h, d, width, tint) = best;
    let near = if h > 0.0 {
        // A steep cone: its normal leans out from the trunk; sun-facing flanks light.
        let out = d / d.length().max(1e-4);
        let normal = Vec3::new(out.x * fs.crown[2], 1.0, out.y * fs.crown[2]).normalize();
        let tip = (h / (width * grow)).clamp(0.0, 1.0);
        let lit = normal.dot(sun).max(0.0);
        Canopy { albedo: needles * tint, normal, occlusion: 0.6 + 0.4 * tip * (0.6 + 0.4 * lit), cover: 1.0 }
    } else {
        // A gap: forest floor under the neighbours' shade.
        Canopy { albedo: floor, normal: Vec3::Y, occlusion: gap, cover: 0.0 }
    };
    Canopy {
        albedo: near.albedo.lerp(mean.albedo, far),
        normal: near.normal.lerp(mean.normal, far).normalize(),
        occlusion: near.occlusion + (mean.occlusion - near.occlusion) * far,
        cover: near.cover + (mean.cover - near.cover) * far,
    }
}

/// The share of trees drawn as objects `distance` metres from where they
/// were placed, in a handover band `start..end` to the far forest layer
/// (terrain_surface.wgsl draws the rest: smoothstep(start, end, distance)).
/// Drawn and layer forest then sum to the same share at every distance.
pub fn handover(distance: f32, start: f32, end: f32) -> f32 {
    1.0 - smooth(start, end.max(start + 1e-3), distance)
}

/// Whether a tree at `pos` stays an object in the handover band: kept with
/// probability `handover`, by a hash of its position (stable while the
/// camera moves).
pub fn handover_keep(pos: Vec3, centre: Vec2, start: f32, end: f32) -> bool {
    let h = hash(pos.x.to_bits() as i32, pos.z.to_bits() as i32, 11, 0x5eed);
    h < handover(Vec2::new(pos.x, pos.z).distance(centre), start, end)
}

/// Every tree within `radius` of `centre` (x, z), row by row.
pub fn trees_in_disc(centre: Vec2, radius: f32, seed: u32, style: &Style, density: f32) -> Vec<Tree> {
    let (i0, i1) = (((centre.x - radius) / CELL).floor() as i32, ((centre.x + radius) / CELL).ceil() as i32);
    let (j0, j1) = (((centre.y - radius) / CELL).floor() as i32, ((centre.y + radius) / CELL).ceil() as i32);
    let mut trees = Vec::new();
    for j in j0..j1 {
        for i in i0..i1 {
            // Cheap distance test on the jittered spot before the terrain.
            let (x, z) = ((i as f32 + hash(i, j, 1, seed)) * CELL, (j as f32 + hash(i, j, 2, seed)) * CELL);
            if Vec2::new(x - centre.x, z - centre.y).length() > radius { continue; }
            if let Some(t) = tree(i, j, seed, style, density) { trees.push(t); }
        }
    }
    trees
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::{Pod, Zeroable};
    use wgpu::util::DeviceExt;

    #[test]
    fn measured_style_takes_the_plants_crowns() {
        let parts = crate::plant_growth_assembly::Parts::for_look(&crate::plant_growth_assembly::Look::jeffrey_pine());
        let (plant, _) = crate::recipe_forest::cook_grown_stages(crate::plant_growth::Species::jeffrey_pine(), 15, 1, &parts,
            crate::plant_growth_assembly::Look::jeffrey_pine()).unwrap();
        let proxy = plant.proxy().unwrap();
        let base = ForestStyle::default();
        let m = base.measured(&[proxy]);
        let tall = proxy.source_bounds[1][1] - proxy.source_bounds[0][1];
        assert!((m.far[3] - tall).abs() < 1e-3, "height {} vs {tall}", m.far[3]);
        // A young pine's crown seen from above: a few square metres, under its
        // bounding box's footprint.
        let box_area = (proxy.source_bounds[1][0] - proxy.source_bounds[0][0]) * (proxy.source_bounds[1][2] - proxy.source_bounds[0][2]);
        let area = m.crown[1] * CELL * CELL;
        assert!(area > 0.5 && area < box_area, "crown area {area}, box {box_area}");
        assert!((m.crown[0] - (area / std::f32::consts::PI).sqrt()).abs() < 1e-3);
        assert!(m.needles[..3].iter().all(|&c| c > 0.0 && c < 1.0), "{:?}", m.needles);
        assert_eq!(m.needles[3], base.needles[3]);
        assert_eq!(base.measured(&[]), base);
    }

    #[test]
    fn trees_shun_rock_and_follow_stands_and_density() {
        let style = Style::named("alpine").unwrap();
        let trees = trees_in_disc(Vec2::ZERO, 600.0, 1, &style, 0.6);
        assert!(trees.len() > 200, "{} trees", trees.len());
        for t in &trees {
            let s = terrain_field::eval(t.pos.x, t.pos.z, CUT, 1, &style);
            assert!(grows(&s, &style) && (t.pos.y - (s.pos.y - 0.2)).abs() < 1e-3);
            assert!(t.variant < 3 && (0.75..=1.25).contains(&t.scale.x));
        }
        let fewer = trees_in_disc(Vec2::ZERO, 600.0, 1, &style, 0.2).len();
        assert!(fewer < trees.len() / 2, "density 0.2: {fewer} of {}", trees.len());
        // The far field agrees with the trees: cover where they grow.
        let covered = trees.iter().filter(|t| forest(t.pos.x, t.pos.z, 1, &style, 0.6, &ForestStyle::default()).density > 0.0).count();
        assert!(covered * 10 >= trees.len() * 9, "{covered} of {} trees on forest cover", trees.len());
    }

    /// For tuning a ForestStyle: `FF_PREVIEW=out.png cargo test -p render
    /// --lib preview_canopy` draws 80 m of canopy from above, near (left)
    /// and as its far average (right), in simple sunlight.
    #[test]
    fn preview_canopy() {
        let Ok(path) = std::env::var("FF_PREVIEW") else { return };
        let (w, h) = (960u32, 540u32);
        let fs = ForestStyle::default();
        let sun = Vec3::new(0.5, 0.75, 0.43).normalize();
        let mut rgba = Vec::new();
        for y in 0..h { for x in 0..w {
            let (foot, scale) = if x < w / 2 { (0.0, 0.17) } else { (6.0, 6.0) };
            let (wx, wz) = (3000.0 + (x % (w / 2)) as f32 * scale, -2000.0 + y as f32 * scale);
            let c = canopy(wx, wz, foot, sun, Vec3::Y, 5, 0.9, 0.3, &fs);
            let lit = c.albedo * c.occlusion * (0.35 + 1.6 * c.normal.dot(sun).max(0.0));
            for v in lit.to_array() { let v = (v * 6.0).min(1.0); rgba.push((if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 } * 255.0) as u8); }
            rgba.push(255);
        } }
        crate::write_png(std::path::Path::new(&path), w, h, &rgba).unwrap();
    }

    /// Far away the canopy is the average of what it is up close: crown
    /// cover and the light it returns (albedo x occlusion) over a stand.
    #[test]
    fn canopy_mean_matches_its_detail() {
        let sun = Vec3::new(0.4, 0.8, 0.3).normalize();
        let (mut n, mut cover, mut light, mut far_cover, mut far_light) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for k in 0..40_000 {
            let (x, z) = ((k % 200) as f32 * 1.37 + 3000.0, (k / 200) as f32 * 1.41 - 2000.0);
            let fs = ForestStyle::default();
            let c = canopy(x, z, 0.0, sun, Vec3::Y, 5, 0.8, 0.3, &fs);
            // Past the crowns, but with the stands' octaves all but whole
            // (dropping them is its own averaging, nonlinear at stand edges).
            let f = canopy(x, z, 4.9, sun, Vec3::Y, 5, 0.8, 0.3, &fs);
            n += 1.0;
            cover += c.cover as f64;
            light += (c.albedo.y * c.occlusion) as f64;
            far_cover += f.cover as f64;
            far_light += (f.albedo.y * f.occlusion) as f64;
        }
        let (cover, light, far_cover, far_light) = (cover / n, light / n, far_cover / n, far_light / n);
        eprintln!("CANOPY near cover {cover:.3} light {light:.4} | far cover {far_cover:.3} light {far_light:.4}");
        assert!((cover - far_cover).abs() < 0.015, "cover near {cover} far {far_cover}");
        assert!((light / far_light - 1.0).abs() < 0.15, "light near {light} far {far_light}");
    }

    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable)]
    struct Out { forest: [f32; 4], crown: [f32; 4], tree: [f32; 4], shape: [f32; 4], canopy: [f32; 4], canopy_normal: [f32; 4] }

    /// The canopy test point for cell (i, j): off-centre, and a footprint
    /// that cycles through near (cones), blending and far.
    fn canopy_point(i: i32, j: i32) -> (f32, f32, f32, f32) {
        ((i as f32 + 0.3) * CELL, (j as f32 + 0.7) * CELL, [0.0f32, 2.0, 5.0, 30.0][(i + j).rem_euclid(4) as usize],
            [0.3f32, -0.2, 0.0][(i - j).rem_euclid(3) as usize])
    }
    const SUN: [f32; 3] = [0.4, 0.8, 0.3];

    /// CPU and GPU evaluate the same function. terrain_eval differs from eval
    /// in the last bits, so a spot right on a threshold may flip; nearly all
    /// must agree, and agreeing ones exactly in place.
    #[test]
    fn gpu_matches_cpu() {
        let Ok(gpu) = crate::Gpu::new() else { return };
        let style = Style::named("alpine").unwrap();
        let (seed, density, n) = (3u32, 0.6f32, 64i32);
        let cells: Vec<[i32; 2]> = (0..n * n).map(|k| [k % n - n / 2, k / n - n / 2]).collect();
        let shader = format!("{}\n{}\n{}", terrain_field::WGSL, WGSL, r#"
struct Params { seed: u32, density: f32, pad0: u32, pad1: u32, style: TerrainStyle, fs: ForestStyle };
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> cells: array<vec2<i32>>;
@group(0) @binding(2) var<storage, read_write> out: array<array<vec4<f32>, 6>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= arrayLength(&cells) { return; }
    let c = cells[gid.x];
    let f = forest_at((f32(c.x) + 0.5) * FF_CELL, (f32(c.y) + 0.5) * FF_CELL, params.seed, params.style, params.density, params.fs);
    let t = forest_tree(c.x, c.y, params.seed, params.style, params.density);
    let foot = array<f32, 4>(0.0, 2.0, 5.0, 30.0);
    let sites = array<f32, 3>(0.3, -0.2, 0.0);
    let k = ((c.x + c.y) % 4 + 4) % 4;
    let ks = ((c.x - c.y) % 3 + 3) % 3;
    let eye = select(vec3<f32>(0.0, 1.0, 0.0), normalize(vec3<f32>(-0.7, 0.35, 0.2)), ks == 1);
    let cp = forest_canopy((f32(c.x) + 0.3) * FF_CELL, (f32(c.y) + 0.7) * FF_CELL, foot[k], normalize(vec3<f32>(0.4, 0.8, 0.3)), eye,
        params.seed, params.density, sites[ks], params.fs);
    out[gid.x] = array<vec4<f32>, 6>(vec4<f32>(f.density, f.colour), vec4<f32>(f.crown_height, 0.0, 0.0, 0.0),
        vec4<f32>(t.pos, t.yaw), vec4<f32>(t.scale.xy, f32(t.variant), f32(t.present)),
        vec4<f32>(cp.albedo, cp.occlusion), vec4<f32>(cp.normal, cp.cover));
}"#);
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("forest parity"), source: wgpu::ShaderSource::Wgsl(shader.into()) });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: None, module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
        let mut params = bytemuck::bytes_of(&[seed, density.to_bits(), 0, 0]).to_vec();
        params.extend_from_slice(bytemuck::bytes_of(&style));
        let fs = ForestStyle::default();
        params.extend_from_slice(bytemuck::bytes_of(&fs));
        let params = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &params, usage: wgpu::BufferUsages::UNIFORM });
        let input = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&cells), usage: wgpu::BufferUsages::STORAGE });
        let size = (cells.len() * std::mem::size_of::<Out>()) as u64;
        let output = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: input.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: output.as_entire_binding() }] });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((cells.len() as u32).div_ceil(64), 1, 1);
        }
        enc.copy_buffer_to_buffer(&output, 0, &read, 0, size);
        gpu.queue.submit([enc.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let gpu_out: Vec<Out> = bytemuck::cast_slice(&read.slice(..).get_mapped_range().unwrap()).to_vec();
        let (mut agree, mut trees, mut canopy_agree) = (0, 0, 0);
        for (c, g) in cells.iter().zip(&gpu_out) {
            let f = forest((c[0] as f32 + 0.5) * CELL, (c[1] as f32 + 0.5) * CELL, seed, &style, density, &fs);
            let (px, pz, foot, site) = canopy_point(c[0], c[1]);
            let eye = if (c[0] - c[1]).rem_euclid(3) == 1 { Vec3::new(-0.7, 0.35, 0.2).normalize() } else { Vec3::Y };
            let cp = canopy(px, pz, foot, Vec3::from(SUN).normalize(), eye, seed, density, site, &fs);
            let got = [g.canopy[0], g.canopy[1], g.canopy[2], g.canopy[3], g.canopy_normal[0], g.canopy_normal[1], g.canopy_normal[2], g.canopy_normal[3]];
            let want = [cp.albedo.x, cp.albedo.y, cp.albedo.z, cp.occlusion, cp.normal.x, cp.normal.y, cp.normal.z, cp.cover];
            if got.iter().zip(&want).all(|(a, b)| (a - b).abs() < 2e-3) { canopy_agree += 1; }
            let cpu_tree = tree(c[0], c[1], seed, &style, density);
            let gpu_has = g.shape[3] > 0.5;
            if cpu_tree.is_some() == gpu_has && (f.density > 0.0) == (g.forest[0] > 0.0) { agree += 1; }
            if let (Some(t), true) = (cpu_tree, gpu_has) {
                trees += 1;
                assert!((Vec3::from_slice(&g.tree[..3]) - t.pos).abs().max_element() < 0.05, "tree {c:?}: cpu {:?} gpu {:?}", t.pos, &g.tree);
                assert_eq!(g.tree[3], t.yaw);
                assert_eq!([g.shape[0], g.shape[1], g.shape[2]], [t.scale.x, t.scale.y, t.variant as f32]);
            }
            if f.density > 0.0 && g.forest[0] > 0.0 {
                // The share ramp (x5) on the site's wetness (x0.8) magnifies
                // terrain_eval's last bits.
                assert!((g.forest[0] - f.density).abs() < 5e-3, "density {c:?}: cpu {} gpu {}", f.density, g.forest[0]);
                assert!((Vec3::new(g.forest[1], g.forest[2], g.forest[3]) - f.colour).abs().max_element() < 1e-3);
                assert!((g.crown[0] - f.crown_height).abs() < 0.05);
            }
        }
        assert!(trees > 300, "{trees} trees compared");
        assert!(agree * 100 >= cells.len() * 99, "{agree} of {} cells agree", cells.len());
        // A crown edge or a tie between two crowns may flip on float bits.
        assert!(canopy_agree * 100 >= cells.len() * 99, "canopy: {canopy_agree} of {} points agree", cells.len());
    }
}
