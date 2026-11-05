//! Regions: where the world is which kind of land. A palette of terrain
//! styles (`terrain_field::Style`, usually biomes', up to 256 per world)
//! and a coarse grid saying which of them hold at every point, so one
//! world can run from alpine massifs through lowland and tundra to polder,
//! with the land changing gradually wherever two meet.
//!
//! Each grid node keeps its four strongest styles as (id, weight) pairs,
//! eight bytes a node, 64 m apart by default. How many styles a world has
//! costs nothing per sample; how many meet at one spot does, so that is
//! what is bounded: a sample reads its four surrounding nodes and mixes at
//! most their sixteen pairs, a fixed cost on the CPU and the GPU.
//! `world_constraints` carries the regions to both (`World::eval` on the
//! CPU, `terrain_eval_world` in `shaders/world_constraints.wgsl`, line for
//! line the same as `Regions::style_at`). Every LOD cut reads the same
//! style at the same point, so the change from one land to another has no
//! seams at any distance. Where only one style holds, it holds bit for bit.
//!
//! ```ignore
//! let regions = Regions::split([alpine.terrain, polder.terrain], Vec2::ZERO, 16_000.0, Vec2::X, 1_500.0, 600.0, seed);
//! let world = Constraints { regions: Some(regions), ..Default::default() }.bake(seed, &alpine.terrain);
//! let ground = world.eval(x, z, cut, seed, &alpine.terrain);
//! ```
use crate::terrain_field::{self, Style};
use glam::Vec2;

/// Most styles one world's palette holds (ids are one byte).
pub const MAX_STYLES: usize = 256;
/// Styles one grid node keeps: at most this many meet at any point.
pub const SLOTS: usize = 4;
/// Default spacing of the grid (metres).
pub const CELL: f32 = 64.0;
/// Words of one style as the GPU reads it (Style is 16 floats).
pub const STYLE_WORDS: usize = 16;
/// Words of one grid node: its four style ids, then their weights, a byte each.
pub const NODE_WORDS: usize = 2;

/// A palette of styles and the grid of which hold where.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Regions {
    pub styles: Vec<Style>,
    /// World (x, z) of node (0, 0).
    pub origin: Vec2,
    /// Metres between nodes.
    pub cell: f32,
    /// Nodes across x and z.
    pub size: [u32; 2],
    /// Two words per node, row by row: byte k of the first is slot k's
    /// style id, byte k of the second its weight (0..255, a node's
    /// weights summing to 255). Points past the grid take its edge.
    pub nodes: Vec<[u32; NODE_WORDS]>,
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A node from per-style weights: the strongest four, scaled to sum 255.
fn node(weights: &[f32]) -> [u32; NODE_WORDS] {
    let mut order: Vec<usize> = (0..weights.len()).filter(|&k| weights[k] > 0.0).collect();
    order.sort_by(|&a, &b| weights[b].total_cmp(&weights[a]).then(a.cmp(&b)));
    order.truncate(SLOTS);
    let sum: f32 = order.iter().map(|&k| weights[k]).sum();
    if order.is_empty() || sum <= 0.0 { return [0, 255]; }
    let mut bytes: Vec<u32> = order.iter().map(|&k| (weights[k] / sum * 255.0).round() as u32).collect();
    // Rounding may leave the sum off by a few: the strongest takes it up.
    let total: u32 = bytes.iter().sum();
    bytes[0] = (bytes[0] + 255).saturating_sub(total).min(255);
    let (mut ids, mut ws) = (0u32, 0u32);
    for (slot, (&k, &w)) in order.iter().zip(&bytes).enumerate() {
        ids |= (k as u32) << (8 * slot);
        ws |= w << (8 * slot);
    }
    [ids, ws]
}

impl Regions {
    /// The grid `extent` metres square round `centre`, nodes `cell` metres
    /// apart, from `weight`, which fills in the share of each style (any
    /// scale) at a point; a node keeps its four strongest. A node where
    /// all are zero takes the first style.
    pub fn paint(styles: &[Style], centre: Vec2, extent: f32, cell: f32, weight: impl Fn(Vec2, &mut [f32])) -> Self {
        assert!(!styles.is_empty() && styles.len() <= MAX_STYLES, "1 to {MAX_STYLES} styles");
        assert!(cell > 0.0 && extent > 0.0);
        let n = (extent / cell).ceil() as u32 + 1;
        let origin = centre - Vec2::splat((n - 1) as f32 * cell * 0.5);
        let mut w = vec![0.0f32; styles.len()];
        let mut nodes = Vec::with_capacity((n * n) as usize);
        for j in 0..n {
            for i in 0..n {
                w.iter_mut().for_each(|v| *v = 0.0);
                weight(origin + Vec2::new(i as f32, j as f32) * cell, &mut w);
                nodes.push(node(&w));
            }
        }
        Regions { styles: styles.to_vec(), origin, cell, size: [n, n], nodes }
    }

    /// One style everywhere: the world as if it had no regions.
    pub fn uniform(style: Style) -> Self {
        Regions { styles: vec![style], origin: Vec2::ZERO, cell: CELL, size: [1, 1], nodes: vec![[0, 255]] }
    }

    /// Two lands meeting along a line through `centre` across `dir` (style
    /// `a` behind it, `b` ahead), changing over `blend` metres, the border
    /// wandering up to `wobble` metres either way so it never runs straight.
    /// The grid covers `extent` metres square; past it the edge holds.
    pub fn split(styles: [Style; 2], centre: Vec2, extent: f32, dir: Vec2, blend: f32, wobble: f32, seed: u32) -> Self {
        let dir = dir.normalize_or(Vec2::X);
        Regions::paint(&styles, centre, extent, CELL, |p, w| {
            let t = smooth(-0.5 * blend, 0.5 * blend, (p - centre).dot(dir) + wander(p, wobble, seed));
            w[0] = 1.0 - t;
            w[1] = t;
        })
    }

    /// Any number of lands as cells round `sites` (style k round site k),
    /// each meeting its neighbours over `blend` metres along borders that
    /// wander up to `wobble` metres: a whole map of biomes from a handful
    /// of points.
    pub fn cells(styles: &[Style], sites: &[Vec2], extent: f32, blend: f32, wobble: f32, seed: u32) -> Self {
        assert_eq!(styles.len(), sites.len());
        let lo = sites.iter().fold(Vec2::splat(f32::MAX), |a, &b| a.min(b));
        let hi = sites.iter().fold(Vec2::splat(f32::MIN), |a, &b| a.max(b));
        Regions::paint(styles, (lo + hi) * 0.5, extent, CELL, |p, w| {
            let q = p + Vec2::new(wander(p, wobble, seed), wander(p + Vec2::splat(7919.0), wobble, seed));
            let near = sites.iter().map(|s| s.distance(q)).fold(f32::MAX, f32::min);
            for (k, s) in sites.iter().enumerate() {
                w[k] = 1.0 - smooth(0.0, blend, s.distance(q) - near);
            }
        })
    }

    /// Bytes the regions take as the GPU reads them.
    pub fn bytes(&self) -> usize { 4 * self.words().len() }

    /// The styles then the nodes, as the GPU reads them (u32 words; a
    /// style is its 16 floats' bits).
    pub fn words(&self) -> Vec<u32> {
        let mut w: Vec<u32> = self.styles.iter().flat_map(|s| bytemuck::cast_ref::<Style, [u32; STYLE_WORDS]>(s).to_vec()).collect();
        w.extend(self.nodes.iter().flatten());
        w
    }

    /// How many kinds of land the palette holds (its `styles`). A kind's id
    /// is its index: other systems keep their own tables by the same id
    /// (ground colours, flora, later streets and buildings), so a region
    /// is one id everywhere, whatever reads it.
    pub fn kinds(&self) -> usize { self.styles.len() }

    /// The four grid nodes round (x, z) and their bilinear shares, in the
    /// order the GPU reads them.
    fn corners(&self, x: f32, z: f32) -> [(usize, f32); 4] {
        let (w, h) = (self.size[0], self.size[1]);
        let fx = ((x - self.origin.x) / self.cell).clamp(0.0, (w - 1) as f32);
        let fz = ((z - self.origin.y) / self.cell).clamp(0.0, (h - 1) as f32);
        let i0 = (fx.floor() as u32).min(w.max(2) - 2);
        let j0 = (fz.floor() as u32).min(h.max(2) - 2);
        let (i1, j1) = ((i0 + 1).min(w - 1), (j0 + 1).min(h - 1));
        let (tx, tz) = (fx - i0 as f32, fz - j0 as f32);
        let at = |i: u32, j: u32| (j * w + i) as usize;
        [(at(i0, j0), (1.0 - tx) * (1.0 - tz)), (at(i1, j0), tx * (1.0 - tz)), (at(i0, j1), (1.0 - tx) * tz), (at(i1, j1), tx * tz)]
    }

    /// Each kind's share at (x, z) as (kind, share) pairs, at most 16,
    /// summing to 1; a kind can appear more than once (once per node).
    pub fn mix(&self, x: f32, z: f32) -> Vec<(u32, f32)> {
        let mut out = Vec::with_capacity(4);
        let mut sum = 0.0f32;
        for (c, f) in self.corners(x, z) {
            let n = self.nodes[c];
            for k in 0..SLOTS as u32 {
                let share = ((n[1] >> (8 * k)) & 255) as f32 / 255.0 * f;
                if share > 0.0 { out.push(((n[0] >> (8 * k)) & 255, share)); sum += share; }
            }
        }
        for e in &mut out { e.1 /= sum; }
        out
    }

    /// The share of kind `kind` at (x, z), 0..1.
    pub fn share(&self, kind: u32, x: f32, z: f32) -> f32 {
        self.mix(x, z).iter().filter(|e| e.0 == kind).map(|e| e.1).sum::<f32>().min(1.0)
    }

    /// The style at (x, z): the styles of the four surrounding nodes,
    /// mixed by their weights and bilinear shares. Where one style is all
    /// there is, that style exactly.
    pub fn style_at(&self, x: f32, z: f32) -> Style {
        let corners = self.corners(x, z);
        // One style alone: return it untouched (no rounding from the mix).
        let mut only = u32::MAX;
        let mut mixed = false;
        for &(c, f) in &corners {
            let n = self.nodes[c];
            for k in 0..SLOTS as u32 {
                if f > 0.0 && (n[1] >> (8 * k)) & 255 > 0 {
                    let id = (n[0] >> (8 * k)) & 255;
                    if only == u32::MAX { only = id; } else if id != only { mixed = true; }
                }
            }
        }
        if !mixed { return self.styles[only as usize]; }
        let mut acc = [0.0f32; STYLE_WORDS];
        let mut sum = 0.0f32;
        for &(c, f) in &corners {
            let n = self.nodes[c];
            for k in 0..SLOTS as u32 {
                let share = ((n[1] >> (8 * k)) & 255) as f32 / 255.0 * f;
                if share > 0.0 {
                    let s: &[f32; STYLE_WORDS] = bytemuck::cast_ref(&self.styles[((n[0] >> (8 * k)) & 255) as usize]);
                    for v in 0..STYLE_WORDS { acc[v] += s[v] * share; }
                    sum += share;
                }
            }
        }
        for v in &mut acc { *v /= sum; }
        bytemuck::cast(acc)
    }

    /// A per-kind table (ground colours, say) as the terrain's pixels read
    /// it (`procedural_terrain::region_ground`): a header (origin, cell,
    /// nodes across, kinds), each kind's `per_kind` floats, then each node's
    /// four slots as id * 256 + weight, all exact in f32.
    pub fn surface_words(&self, table: &[f32], per_kind: usize) -> Vec<f32> {
        assert_eq!(table.len(), per_kind * self.kinds(), "one entry per kind");
        let mut w = vec![self.origin.x, self.origin.y, self.cell, self.size[0] as f32, self.size[1] as f32, self.kinds() as f32, per_kind as f32, 0.0];
        w.extend_from_slice(table);
        for n in &self.nodes {
            for k in 0..SLOTS as u32 { w.push((((n[0] >> (8 * k)) & 255) * 256 + ((n[1] >> (8 * k)) & 255)) as f32); }
        }
        w
    }
}

/// A border's sideways wander at p: up to `wobble` metres, over kilometres.
fn wander(p: Vec2, wobble: f32, seed: u32) -> f32 {
    let q = p / 3000.0;
    wobble * (terrain_field::noised(q, seed.wrapping_add(41)).x + 0.5 * terrain_field::noised(q * 2.3, seed.wrapping_add(42)).x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_style_is_that_style_exactly() {
        let alpine = Style::alpine();
        let r = Regions::uniform(alpine);
        let p = Regions::paint(&[alpine, Style::lowland()], Vec2::ZERO, 2000.0, 64.0, |_, w| w[0] = 1.0);
        for k in 0..500 {
            let (x, z) = (k as f32 * 37.1 - 9000.0, k as f32 * -23.3 + 4000.0);
            assert_eq!(r.style_at(x, z), alpine);
            assert_eq!(p.style_at(x, z), alpine);
        }
    }

    #[test]
    fn a_split_changes_gradually_from_one_land_to_the_other() {
        let (a, b) = (Style::alpine(), Style::lowland());
        let r = Regions::split([a, b], Vec2::ZERO, 12_000.0, Vec2::X, 1500.0, 0.0, 3);
        assert_eq!(r.style_at(-3000.0, 50.0), a);
        assert_eq!(r.style_at(3000.0, 50.0), b);
        // Past the grid the edge holds.
        assert_eq!(r.style_at(-50_000.0, 9.0), a);
        // No jumps: a metre further never moves a knob by much.
        let mut prev = r.style_at(-1500.0, 7.0);
        for k in 1..3000 {
            let s = r.style_at(-1500.0 + k as f32, 7.0);
            assert!((s.mountains - prev.mountains).abs() < 2e-3 && (s.peaks - prev.peaks).abs() < 2e-3, "{prev:?} -> {s:?}");
            prev = s;
        }
        let half = r.style_at(0.0, 7.0);
        assert!((half.mountains - 0.5 * (a.mountains + b.mountains)).abs() < 0.05, "{half:?}");
        assert!(r.bytes() < 400_000, "{} bytes", r.bytes());
    }

    /// Far more lands than meet at any one point: twelve biomes' cells
    /// over a 20 km map, each its own land at its centre, and nowhere more
    /// than four mixed.
    #[test]
    fn a_map_of_many_lands() {
        let base = [Style::alpine(), Style::lowland(), Style::default()];
        let styles: Vec<Style> = (0..12).map(|k| Style { snow_line: 500.0 + 100.0 * k as f32, ..base[k % 3] }).collect();
        let sites: Vec<Vec2> = (0..12).map(|k| Vec2::new((k % 4) as f32 * 5000.0 - 7500.0, (k / 4) as f32 * 6000.0 - 6000.0)).collect();
        let r = Regions::cells(&styles, &sites, 22_000.0, 800.0, 400.0, 5);
        for (k, s) in sites.iter().enumerate() {
            assert_eq!(r.style_at(s.x, s.y), styles[k], "site {k}");
        }
        let mut most = 0;
        for n in &r.nodes {
            most = most.max((0..4).filter(|k| (n[1] >> (8 * k)) & 255 > 0).count());
            assert_eq!((0..4).map(|k| (n[1] >> (8 * k)) & 255).sum::<u32>(), 255);
        }
        assert!(most >= 2, "no borders blend");
        // A sample between two lands lies between their snow lines.
        let mid = (sites[0] + sites[1]) * 0.5;
        let s = r.style_at(mid.x, mid.y).snow_line;
        assert!((499.0..=601.0).contains(&s), "{s}");
        // Shares are whole at a site and split on a border.
        assert_eq!(r.share(3, sites[3].x, sites[3].y), 1.0);
        let total: f32 = r.mix(mid.x, mid.y).iter().map(|e| e.1).sum();
        assert!((total - 1.0).abs() < 1e-5);
        assert!(r.share(0, mid.x, mid.y) + r.share(1, mid.x, mid.y) > 0.99);
    }

    #[test]
    fn a_blended_world_has_no_seams_and_each_side_is_its_own_land() {
        let (a, b) = (Style::alpine(), Style::lowland());
        let r = Regions::split([a, b], Vec2::ZERO, 12_000.0, Vec2::X, 1500.0, 500.0, 3);
        for &z in &[0.0f32, 777.0, -2100.0] {
            assert_eq!(terrain_field::eval(-4000.0, z, 0.0, 3, &r.style_at(-4000.0, z)), terrain_field::eval(-4000.0, z, 0.0, 3, &a));
            assert_eq!(terrain_field::eval(4000.0, z, 0.0, 3, &r.style_at(4000.0, z)), terrain_field::eval(4000.0, z, 0.0, 3, &b));
            // Across the border at a coarse cut (no fine relief) the ground
            // is continuous: steps shrink with the distance stepped, which
            // a seam's would not.
            let y = |x: f32| terrain_field::eval(x, z, 64.0, 3, &r.style_at(x, z)).pos.y;
            for k in 0..600 {
                let x = -1500.0 + 5.0 * k as f32 + 0.37;
                let (wide, narrow) = ((y(x + 1.0) - y(x)).abs(), (y(x + 0.01) - y(x)).abs());
                assert!(narrow < 0.05 + 0.03 * wide, "step of {narrow} m over 1 cm at x {x} z {z} ({wide} m over 1 m)");
            }
        }
    }
}
