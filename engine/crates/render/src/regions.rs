//! Regions: where the world is which kind of land. Up to four terrain
//! styles (`terrain_field::Style`, usually a biome's) and a coarse grid of
//! weights saying how much of each holds at every point, so one world can
//! run from alpine massifs in the north to polder in the south, with the
//! land changing gradually in between instead of one style everywhere.
//!
//! The grid is tiny (four bytes per node, 64 m apart by default) and
//! cheap to read: a sample takes its four surrounding nodes, blends their
//! weights bilinearly and mixes the styles by them, a fixed cost on the
//! CPU and the GPU. `world_constraints` carries the regions to both (its
//! `World::eval` on the CPU, `terrain_eval_world` in
//! `shaders/world_constraints.wgsl`, line for line the same as
//! `Regions::style_at`). Every LOD cut reads the same style at the same
//! point, so the change from one land to another has no seams at any
//! distance.
//!
//! ```ignore
//! let regions = Regions::split([alpine.terrain, polder.terrain], Vec2::ZERO, 16_000.0, Vec2::X, 1_500.0, 600.0, seed);
//! let world = Constraints { regions: Some(regions), ..Default::default() }.bake(seed, &alpine.terrain);
//! let ground = world.eval(x, z, cut, seed, &alpine.terrain);
//! ```
use crate::terrain_field::{self, Style};
use glam::Vec2;

/// Most styles one world blends.
pub const MAX_STYLES: usize = 4;
/// Default spacing of the weight grid (metres).
pub const CELL: f32 = 64.0;
/// Words of one style as the GPU reads it (Style is 16 floats).
pub const STYLE_WORDS: usize = 16;

/// Up to four styles and the grid of their weights.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Regions {
    pub styles: Vec<Style>,
    /// World (x, z) of node (0, 0).
    pub origin: Vec2,
    /// Metres between nodes.
    pub cell: f32,
    /// Nodes across x and z.
    pub size: [u32; 2],
    /// One word per node, row by row: byte k is style k's weight (0..255).
    /// Points past the grid take its edge.
    pub weights: Vec<u32>,
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Regions {
    /// Weights from `weight` at every node of a grid `extent` metres square
    /// round `centre`, `cell` metres apart. `weight` returns the share of
    /// each style there (any scale; normalised per node); a node where all
    /// are zero takes the first style.
    pub fn paint(styles: &[Style], centre: Vec2, extent: f32, cell: f32, weight: impl Fn(Vec2) -> [f32; MAX_STYLES]) -> Self {
        assert!(!styles.is_empty() && styles.len() <= MAX_STYLES, "1 to {MAX_STYLES} styles");
        assert!(cell > 0.0 && extent > 0.0);
        let n = (extent / cell).ceil() as u32 + 1;
        let origin = centre - Vec2::splat((n - 1) as f32 * cell * 0.5);
        let mut weights = Vec::with_capacity((n * n) as usize);
        for j in 0..n {
            for i in 0..n {
                let w = weight(origin + Vec2::new(i as f32, j as f32) * cell);
                let w: Vec<f32> = (0..styles.len()).map(|k| w[k].max(0.0)).collect();
                let sum: f32 = w.iter().sum();
                let mut word = 0u32;
                if sum > 0.0 {
                    for (k, v) in w.iter().enumerate() { word |= ((v / sum * 255.0).round() as u32).min(255) << (8 * k); }
                }
                if word == 0 { word = 255; }
                weights.push(word);
            }
        }
        Regions { styles: styles.to_vec(), origin, cell, size: [n, n], weights }
    }

    /// One style everywhere: the world as if it had no regions.
    pub fn uniform(style: Style) -> Self {
        Regions { styles: vec![style], origin: Vec2::ZERO, cell: CELL, size: [1, 1], weights: vec![255] }
    }

    /// Two lands meeting along a line through `centre` across `dir` (style
    /// `a` behind it, `b` ahead), changing over `blend` metres, the border
    /// wandering up to `wobble` metres either way so it never runs straight.
    /// The grid covers `extent` metres square; past it the edge holds.
    pub fn split(styles: [Style; 2], centre: Vec2, extent: f32, dir: Vec2, blend: f32, wobble: f32, seed: u32) -> Self {
        let dir = dir.normalize_or(Vec2::X);
        Regions::paint(&styles, centre, extent, CELL, |p| {
            let q = p / 3000.0;
            let wander = wobble * (terrain_field::noised(q, seed.wrapping_add(41)).x + 0.5 * terrain_field::noised(q * 2.3, seed.wrapping_add(42)).x);
            let t = smooth(-0.5 * blend, 0.5 * blend, (p - centre).dot(dir) + wander);
            [1.0 - t, t, 0.0, 0.0]
        })
    }

    /// Bytes the regions take as the GPU reads them.
    pub fn bytes(&self) -> usize { 4 * self.words().len() }

    /// The styles then the weights, as the GPU reads them (u32 words; a
    /// style is its 16 floats' bits).
    pub fn words(&self) -> Vec<u32> {
        let mut w: Vec<u32> = self.styles.iter().flat_map(|s| bytemuck::cast_ref::<Style, [u32; STYLE_WORDS]>(s).to_vec()).collect();
        w.extend_from_slice(&self.weights);
        w
    }

    /// Each style's share at (x, z), summing to 1.
    pub fn shares(&self, x: f32, z: f32) -> [f32; MAX_STYLES] {
        let (w, h) = (self.size[0], self.size[1]);
        let fx = ((x - self.origin.x) / self.cell).clamp(0.0, (w - 1) as f32);
        let fz = ((z - self.origin.y) / self.cell).clamp(0.0, (h - 1) as f32);
        let i0 = (fx.floor() as u32).min(w.max(2) - 2);
        let j0 = (fz.floor() as u32).min(h.max(2) - 2);
        let (i1, j1) = ((i0 + 1).min(w - 1), (j0 + 1).min(h - 1));
        let (tx, tz) = (fx - i0 as f32, fz - j0 as f32);
        let node = |i: u32, j: u32| self.weights[(j * w + i) as usize];
        let (a, b, c, d) = (node(i0, j0), node(i1, j0), node(i0, j1), node(i1, j1));
        let byte = |word: u32, k: u32| ((word >> (8 * k)) & 255) as f32 / 255.0;
        let mut share = [0.0f32; MAX_STYLES];
        let mut sum = 0.0f32;
        for k in 0..MAX_STYLES as u32 {
            let top = byte(a, k) * (1.0 - tx) + byte(b, k) * tx;
            let bottom = byte(c, k) * (1.0 - tx) + byte(d, k) * tx;
            share[k as usize] = top * (1.0 - tz) + bottom * tz;
            sum += share[k as usize];
        }
        for s in &mut share { *s /= sum; }
        share
    }

    /// The style at (x, z): the styles mixed by their shares there.
    pub fn style_at(&self, x: f32, z: f32) -> Style {
        let share = self.shares(x, z);
        let mut acc = [0.0f32; STYLE_WORDS];
        for (k, style) in self.styles.iter().enumerate() {
            let s: &[f32; STYLE_WORDS] = bytemuck::cast_ref(style);
            for f in 0..STYLE_WORDS { acc[f] += s[f] * share[k]; }
        }
        bytemuck::cast(acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_style_is_that_style_exactly() {
        let alpine = Style::alpine();
        let r = Regions::uniform(alpine);
        let p = Regions::paint(&[alpine, Style::lowland()], Vec2::ZERO, 2000.0, 64.0, |_| [1.0, 0.0, 0.0, 0.0]);
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
        assert!(r.bytes() < 200_000, "{} bytes", r.bytes());
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
