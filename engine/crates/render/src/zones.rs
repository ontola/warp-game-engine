//! Zones: what each place is for. A zone is an outline with a tag
//! ("industrial", "residential", "park", or anything a game names) and soft
//! intent (how dense, how wealthy, how old), feathered at its edge so
//! neighbouring zones hand over gradually. Zones bend nothing; they are
//! fields that generators read: a rule's density, later a street pattern
//! or a building style. Like a region's kind, a zone's tag is an id, and
//! each system keeps its own table by tag.
//!
//! Storage is the outlines (a few corners each). For lookup they are baked
//! into a coarse grid listing the zones that reach each cell, with cells
//! wholly inside a zone marked so they need no distance at all: a sample
//! reads one cell's short list, the same on the CPU and the GPU
//! (`wc_zone` and `wc_zoning` in world_constraints.wgsl, line for line).
//!
//! ```ignore
//! let zones = vec![Zone::new("industrial", outline, 60.0).density(0.8).age(0.7)];
//! let world = Constraints { zones, ..Default::default() }.bake(seed, &style);
//! let share = world.zone("industrial", x, z);
//! let soft = world.zoning(x, z); // covered, density, wealth, age
//! ```
use crate::world_constraints::{clean_outline, outline_distance};
use glam::Vec2;

/// Grid spacing in metres.
pub const CELL: f32 = 64.0;
/// Words per zone in `Zones::words`.
pub const RECORD_WORDS: u32 = 8;
/// Marks a cell item as wholly inside its zone.
pub const INSIDE: u32 = 1 << 31;

/// One zone: an outline, a tag and its soft intent.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub tag: String,
    /// Corners in x-z, either winding; at least three.
    pub outline: Vec<Vec2>,
    /// Metres over which the zone fades out, centred on its outline (half
    /// inside, half outside), so two zones sharing an edge sum to one.
    pub feather: f32,
    /// 0..1: how built up, how wealthy, how old.
    pub density: f32,
    pub wealth: f32,
    pub age: f32,
}

impl Zone {
    pub fn new(tag: &str, outline: Vec<Vec2>, feather: f32) -> Self {
        Zone { tag: tag.to_string(), outline, feather, density: 0.5, wealth: 0.5, age: 0.5 }
    }
    pub fn density(mut self, v: f32) -> Self { self.density = v; self }
    pub fn wealth(mut self, v: f32) -> Self { self.wealth = v; self }
    pub fn age(mut self, v: f32) -> Self { self.age = v; self }
}

/// The zones' soft intent at a point: how much of it is zoned at all, and
/// the zones' values there, weighted by their shares (0 where unzoned).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Zoning {
    pub covered: f32,
    pub density: f32,
    pub wealth: f32,
    pub age: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct Baked {
    first: u32,
    count: u32,
    tag: u32,
    feather: f32,
    values: [f32; 3],
}

/// The zones baked for lookup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Zones {
    /// Tag names; a zone's tag id is its index here.
    pub tags: Vec<String>,
    pub origin: Vec2,
    pub cell: f32,
    pub size: [u32; 2],
    zones: Vec<Baked>,
    corners: Vec<Vec2>,
    /// size.x * size.y + 1 offsets into `items`.
    cells: Vec<u32>,
    /// Zone index, | INSIDE when the cell lies wholly inside it.
    items: Vec<u32>,
}

/// How much of a zone with this feather a point `d` metres outside its
/// outline (negative inside) still is.
fn fade(d: f32, feather: f32) -> f32 {
    let h = 0.5 * feather.max(1e-3);
    let t = ((d + h) / (2.0 * h)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

impl Zones {
    pub fn bake(zones: &[Zone]) -> Self {
        let mut tags: Vec<String> = Vec::new();
        let mut baked = Vec::new();
        let mut corners = Vec::new();
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for z in zones {
            let outline = clean_outline(&z.outline);
            assert!(outline.len() >= 3, "zone {} needs three distinct corners", z.tag);
            let tag = match tags.iter().position(|t| *t == z.tag) {
                Some(t) => t,
                None => { tags.push(z.tag.clone()); tags.len() - 1 }
            };
            assert!(tags.len() <= 256, "more than 256 zone tags");
            let h = 0.5 * z.feather.max(0.0);
            for c in &outline { lo = lo.min(*c - h); hi = hi.max(*c + h); }
            baked.push(Baked { first: corners.len() as u32, count: outline.len() as u32, tag: tag as u32, feather: z.feather.max(0.0), values: [z.density, z.wealth, z.age] });
            corners.extend(outline);
        }
        assert!(baked.len() < (INSIDE as usize), "too many zones");
        if baked.is_empty() {
            return Zones { tags, origin: Vec2::ZERO, cell: CELL, size: [1, 1], zones: baked, corners, cells: vec![0, 0], items: Vec::new() };
        }
        let origin = (lo / CELL).floor() * CELL;
        let size = [((hi.x - origin.x) / CELL).floor() as u32 + 1, ((hi.y - origin.y) / CELL).floor() as u32 + 1];
        let r = CELL * std::f32::consts::FRAC_1_SQRT_2;
        let mut lists = vec![Vec::<u32>::new(); (size[0] * size[1]) as usize];
        for (k, z) in baked.iter().enumerate() {
            let outline = &corners[z.first as usize..(z.first + z.count) as usize];
            let h = 0.5 * z.feather;
            let (mut a, mut b) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
            for c in outline { a = a.min(*c - h); b = b.max(*c + h); }
            let (i0, j0) = (((a.x - origin.x) / CELL).floor().max(0.0) as u32, ((a.y - origin.y) / CELL).floor().max(0.0) as u32);
            let (i1, j1) = ((((b.x - origin.x) / CELL).floor() as u32).min(size[0] - 1), (((b.y - origin.y) / CELL).floor() as u32).min(size[1] - 1));
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let centre = origin + (Vec2::new(i as f32, j as f32) + 0.5) * CELL;
                    let d = outline_distance(outline, centre);
                    // Wholly inside, past the fade: the zone's full share.
                    if d <= -(h + r) { lists[(j * size[0] + i) as usize].push(k as u32 | INSIDE); }
                    else if d < h + r { lists[(j * size[0] + i) as usize].push(k as u32); }
                }
            }
        }
        let mut cells = Vec::with_capacity(lists.len() + 1);
        let mut items = Vec::new();
        for l in &lists {
            cells.push(items.len() as u32);
            items.extend_from_slice(l);
        }
        cells.push(items.len() as u32);
        Zones { tags, origin, cell: CELL, size, zones: baked, corners, cells, items }
    }

    /// The number of zones.
    pub fn count(&self) -> u32 { self.zones.len() as u32 }

    /// The id of tag `name`, if any zone has it.
    pub fn tag(&self, name: &str) -> Option<u32> { self.tags.iter().position(|t| t == name).map(|t| t as u32) }

    /// Most zones any cell lists: the worst-case cost of one sample.
    pub fn busiest_cell(&self) -> u32 { self.cells.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0) }

    fn list(&self, x: f32, z: f32) -> &[u32] {
        let fi = ((x - self.origin.x) / self.cell).floor();
        let fj = ((z - self.origin.y) / self.cell).floor();
        if self.zones.is_empty() || fi < 0.0 || fj < 0.0 || fi >= self.size[0] as f32 || fj >= self.size[1] as f32 { return &[]; }
        let c = (fj as u32 * self.size[0] + fi as u32) as usize;
        &self.items[self.cells[c] as usize..self.cells[c + 1] as usize]
    }

    fn share(&self, item: u32, p: Vec2) -> f32 {
        if item & INSIDE != 0 { return 1.0; }
        let z = &self.zones[item as usize];
        fade(outline_distance(&self.corners[z.first as usize..(z.first + z.count) as usize], p), z.feather)
    }

    /// The share (0..1) of tag `tag` at (x, z): its strongest zone there.
    pub fn weight(&self, tag: u32, x: f32, z: f32) -> f32 {
        let p = Vec2::new(x, z);
        let mut w = 0.0f32;
        for &item in self.list(x, z) {
            if self.zones[(item & !INSIDE) as usize].tag == tag { w = w.max(self.share(item, p)); }
        }
        w
    }

    /// The soft intent at (x, z).
    pub fn zoning(&self, x: f32, z: f32) -> Zoning {
        let p = Vec2::new(x, z);
        let (mut sum, mut acc) = (0.0f32, [0.0f32; 3]);
        for &item in self.list(x, z) {
            let w = self.share(item, p);
            if w <= 0.0 { continue; }
            let v = self.zones[(item & !INSIDE) as usize].values;
            for k in 0..3 { acc[k] += v[k] * w; }
            sum += w;
        }
        if sum <= 0.0 { return Zoning::default(); }
        Zoning { covered: sum.min(1.0), density: acc[0] / sum, wealth: acc[1] / sum, age: acc[2] / sum }
    }

    /// The zones for the GPU, as u32 words relative to their start: per
    /// zone RECORD_WORDS (first corner word, corners, tag, feather,
    /// density, wealth, age bits, 0), then size.x * size.y + 1 cell offsets,
    /// the items, and the corners (x, z bits).
    pub fn words(&self) -> Vec<u32> {
        let n = self.zones.len() as u32;
        let corners_at = n * RECORD_WORDS + self.cells.len() as u32 + self.items.len() as u32;
        let mut w = Vec::with_capacity((corners_at as usize) + 2 * self.corners.len());
        for z in &self.zones {
            w.extend_from_slice(&[corners_at + 2 * z.first, z.count, z.tag, z.feather.to_bits(), z.values[0].to_bits(), z.values[1].to_bits(), z.values[2].to_bits(), 0]);
        }
        w.extend_from_slice(&self.cells);
        w.extend_from_slice(&self.items);
        for c in &self.corners { w.extend_from_slice(&[c.x.to_bits(), c.y.to_bits()]); }
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(c: Vec2, h: f32) -> Vec<Vec2> {
        vec![c + Vec2::new(-h, -h), c + Vec2::new(h, -h), c + Vec2::new(h, h), c + Vec2::new(-h, h)]
    }

    #[test]
    fn a_zone_is_whole_inside_and_fades_at_its_edge() {
        let zones = Zones::bake(&[Zone::new("industrial", square(Vec2::ZERO, 500.0), 80.0).density(0.8).age(0.7)]);
        let t = zones.tag("industrial").unwrap();
        assert_eq!(zones.weight(t, 0.0, 0.0), 1.0);
        assert_eq!(zones.weight(t, 459.0, 0.0), 1.0);
        assert!((zones.weight(t, 500.0, 0.0) - 0.5).abs() < 1e-4);
        assert_eq!(zones.weight(t, 541.0, 0.0), 0.0);
        assert_eq!(zones.weight(t, 3000.0, 0.0), 0.0);
        let z = zones.zoning(10.0, 20.0);
        assert_eq!(z, Zoning { covered: 1.0, density: 0.8, wealth: 0.5, age: 0.7 });
        assert_eq!(zones.zoning(-900.0, 0.0), Zoning::default());
        // Most of a big zone's cells are wholly inside: no distance per sample.
        let big = Zones::bake(&[Zone::new("city", square(Vec2::ZERO, 2000.0), 80.0)]);
        let inside = big.items.iter().filter(|&&i| i & INSIDE != 0).count();
        assert!(inside * 10 > big.items.len() * 8, "{inside} of {}", big.items.len());
    }

    /// Two zones sharing an edge hand over: their shares sum to one across
    /// it, and the soft values change smoothly from one to the other.
    #[test]
    fn neighbours_hand_over_along_their_shared_edge() {
        let west = vec![Vec2::new(-600.0, -300.0), Vec2::new(0.0, -300.0), Vec2::new(0.0, 300.0), Vec2::new(-600.0, 300.0)];
        let east = vec![Vec2::new(0.0, -300.0), Vec2::new(600.0, -300.0), Vec2::new(600.0, 300.0), Vec2::new(0.0, 300.0)];
        let zones = Zones::bake(&[Zone::new("industrial", west, 100.0).density(0.9), Zone::new("residential", east, 100.0).density(0.3)]);
        let (a, b) = (zones.tag("industrial").unwrap(), zones.tag("residential").unwrap());
        let mut last = zones.zoning(-80.0, 0.0).density;
        for k in -79..80 {
            let x = k as f32;
            assert!((zones.weight(a, x, 0.0) + zones.weight(b, x, 0.0) - 1.0).abs() < 1e-4, "shares at {x}");
            let d = zones.zoning(x, 0.0);
            assert!((d.covered - 1.0).abs() < 1e-4);
            assert!(d.density <= last + 1e-6 && last - d.density < 0.03, "density step at {x}");
            last = d.density;
        }
        assert!((zones.zoning(-200.0, 0.0).density - 0.9).abs() < 1e-6 && (zones.zoning(200.0, 0.0).density - 0.3).abs() < 1e-6);
    }

    /// The grid finds exactly what testing every zone finds, for any
    /// outline (a concave L here).
    #[test]
    fn the_grid_finds_what_brute_force_finds() {
        let l = vec![Vec2::new(0.0, 0.0), Vec2::new(900.0, 0.0), Vec2::new(900.0, 250.0), Vec2::new(300.0, 250.0), Vec2::new(300.0, 800.0), Vec2::new(0.0, 800.0)];
        let list = [Zone::new("park", l, 50.0), Zone::new("park", square(Vec2::new(700.0, 600.0), 150.0), 200.0), Zone::new("harbour", square(Vec2::new(-100.0, 400.0), 220.0), 30.0)];
        let zones = Zones::bake(&list);
        for k in 0..6000 {
            let (x, z) = ((k as f32 * 37.31) % 1500.0 - 400.0, (k as f32 * 91.73) % 1400.0 - 300.0);
            for name in ["park", "harbour"] {
                let brute = list.iter().filter(|zn| zn.tag == name).map(|zn| fade(outline_distance(&zn.outline, Vec2::new(x, z)), zn.feather)).fold(0.0, f32::max);
                let got = zones.weight(zones.tag(name).unwrap(), x, z);
                assert!((got - brute).abs() < 1e-5, "{name} at {x} {z}: {got} vs {brute}");
            }
        }
        assert!(zones.busiest_cell() <= 3);
    }

    #[test]
    fn the_outline_distance_is_signed_and_exact() {
        let sq = square(Vec2::ZERO, 10.0);
        assert!((outline_distance(&sq, Vec2::ZERO) + 10.0).abs() < 1e-5);
        assert!((outline_distance(&sq, Vec2::new(13.0, 0.0)) - 3.0).abs() < 1e-5);
        assert!((outline_distance(&sq, Vec2::new(13.0, 14.0)) - 5.0).abs() < 1e-5);
        let mut rev = sq.clone();
        rev.reverse();
        assert_eq!(outline_distance(&rev, Vec2::new(2.0, 3.0)), outline_distance(&sq, Vec2::new(2.0, 3.0)));
    }
}
