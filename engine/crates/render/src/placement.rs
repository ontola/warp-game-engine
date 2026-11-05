//! Rule-based placement: what stands where, decided by the world itself
//! instead of an artist placing each object. One `Rule` per kind of object
//! (a tree, a boulder, a fallen trunk, a flower patch, later a bin or a bench)
//! says how likely it is at a spot from the spot's context (the terrain's
//! attributes and named fields such as distance to a road or a biome's
//! weight), how it is shaped, and what it keeps clear of. `place` puts the
//! rules on jittered grids, layer by layer in rule order, so a boulder can
//! avoid the trees placed before it.
//!
//! Everything is a function of position and seed: each grid cell rolls its
//! own hashes, so the same spot gives the same object on every machine, in
//! any order and for any area asked, and an object's `id` is stable for
//! overrides, saves and gameplay. The tree rule (`tree_rule`) reproduces
//! `forest_field::trees_in_disc` exactly.
//!
//! ```ignore
//! let world = placement::TerrainWorld { seed, style: &style, constraints: None };
//! let rules = [placement::tree_rule(0.6), placement::boulder_rule(), placement::deadwood_rule()];
//! let objects = placement::place(&rules, &world, centre, 900.0);
//! ```
use crate::{forest_field, world_constraints};
use crate::terrain_field::{self, Sample, Style};
use glam::{Vec2, Vec3};
use std::collections::HashMap;

/// What the rules can ask about a point of the world.
pub trait World: Sync {
    /// The world's seed; each rule mixes its own into it.
    fn seed(&self) -> u32;
    /// The ground at (x, z).
    fn ground(&self, x: f32, z: f32) -> Sample;
    /// A named context field at (x, z): "forest" (share of forest),
    /// "track_side", "zone:industrial" (a zone tag's share), "density"
    /// (the zones' soft intent). 0 when unknown.
    fn field(&self, name: &str, x: f32, z: f32, ground: &Sample) -> f32 {
        let _ = (name, x, z, ground);
        0.0
    }
    /// Metres from (x, z) to the nearest road edge or pad rim (negative
    /// inside); a world without constraints is clear everywhere.
    fn clearance(&self, x: f32, z: f32) -> f32 {
        let _ = (x, z);
        1e6
    }
    /// The share (0..1) of region kind `kind` at (x, z) (crate::regions):
    /// a kind's rules (its flora, later its streets and buildings) scale
    /// their density by it. A world without regions is all kind 0.
    fn share(&self, kind: u32, x: f32, z: f32) -> f32 {
        let _ = (x, z);
        if kind == 0 { 1.0 } else { 0.0 }
    }
}

/// Rules of several region kinds at once: kind k's rules (`per_kind[k]`)
/// with their density scaled by the kind's share at each spot, their ids
/// moved into the kind's own range (`kind_rule`) so objects of different
/// kinds never share an id, and their avoid lists widened to the same rule
/// of every kind. `variant` maps each kind's own variant numbers into one
/// shared list (the world's species, say).
pub fn regional(per_kind: Vec<Vec<Rule>>, variant: impl Fn(u32, u32) -> u32 + Send + Sync + Clone + 'static) -> Vec<Rule> {
    let kinds = per_kind.len() as u32;
    let mut out = Vec::new();
    for (k, rules) in per_kind.into_iter().enumerate() {
        let k = k as u32;
        for r in rules {
            let (density, shape) = (r.density, r.shape);
            let variant = variant.clone();
            out.push(Rule {
                id: kind_rule(k, r.id), name: r.name, cell: r.cell, salt: r.salt ^ k.wrapping_mul(0x9e37_79b9), cap: r.cap,
                density: Box::new(move |s| {
                    let share = s.world.share(k, s.ground.pos.x, s.ground.pos.z);
                    if share <= 0.0 { 0.0 } else { density(s) * share }
                }),
                shape: Box::new(move |s| { let mut sh = shape(s); sh.variant = variant(k, sh.variant); sh }),
                avoid: r.avoid.iter().flat_map(|&(id, d)| (0..kinds).map(move |j| (kind_rule(j, id), d))).collect(),
            });
        }
    }
    out
}

/// Rules kept out of where a field is high: each one's density scaled by
/// 1 - `field` (a zone's share, say "zone:town": no forest in town).
pub fn thinned(rules: Vec<Rule>, field: &'static str) -> Vec<Rule> {
    rules.into_iter().map(|mut r| {
        let density = r.density;
        r.density = Box::new(move |s| {
            let f = s.field(field);
            if f >= 1.0 { 0.0 } else { density(s) * (1.0 - f.max(0.0)) }
        });
        r
    }).collect()
}

/// Rule `rule` (below 256) of region kind `kind` (below 256): kind 0 keeps
/// the id itself, so a world of one kind places exactly what it did
/// without regions. The result fits the 16 bits `instance_id` keeps.
pub fn kind_rule(kind: u32, rule: u32) -> u32 {
    assert!(rule < 256 && kind < 256, "rule {rule} of kind {kind} out of range");
    rule | (kind << 8)
}

/// The kind-independent rule of a `kind_rule` id (TREE for any kind's trees).
pub fn base_rule(id: u32) -> u32 { id & 0xff }

/// The region kind of a `kind_rule` id.
pub fn rule_kind(id: u32) -> u32 { (id >> 8) & 0xff }

/// The procedural terrain and its forest (terrain_field, forest_field),
/// bent to the world's constraints when it has any.
pub struct TerrainWorld<'a> {
    pub seed: u32,
    pub style: &'a Style,
    pub constraints: Option<&'a world_constraints::World>,
}

impl World for TerrainWorld<'_> {
    fn seed(&self) -> u32 { self.seed }
    fn ground(&self, x: f32, z: f32) -> Sample {
        match self.constraints {
            Some(w) => w.eval(x, z, forest_field::CUT, self.seed, self.style),
            None => terrain_field::eval(x, z, forest_field::CUT, self.seed, self.style),
        }
    }
    fn clearance(&self, x: f32, z: f32) -> f32 { self.constraints.map_or(1e6, |w| w.clearance(x, z).min(1e6)) }
    fn share(&self, kind: u32, x: f32, z: f32) -> f32 {
        match self.constraints.and_then(|w| w.regions.as_ref()) {
            Some(r) => r.share(kind, x, z),
            None => if kind == 0 { 1.0 } else { 0.0 },
        }
    }
    fn field(&self, name: &str, x: f32, z: f32, ground: &Sample) -> f32 {
        match name {
            // Share of tree spots that grow a tree here, 0 where none can.
            "forest" => if forest_field::grows(ground, &self.style_at(x, z)) {
                forest_field::share(x, z, self.seed, forest_field::site(ground.up, ground.wet), 0.0)
            } else { 0.0 },
            "grows" => f32::from(u8::from(forest_field::grows(ground, &self.style_at(x, z)))),
            "snow_line" => self.style_at(x, z).snow_line,
            // World::clearance, and where along and beside the road.
            "clearance" => self.clearance(x, z),
            "track_distance" => self.near(x, z).map_or(1e6, |n| n.distance),
            "track_side" => self.near(x, z).map_or(0.0, |n| n.side),
            "track_along" => self.near(x, z).map_or(0.0, |n| n.along),
            // Zones (crate::zones): "zone:TAG" is that tag's share, "zoned"
            // how much is zoned at all, the rest the zones' soft intent.
            "zoned" => self.constraints.map_or(0.0, |w| w.zoning(x, z).covered),
            "density" => self.constraints.map_or(0.0, |w| w.zoning(x, z).density),
            "wealth" => self.constraints.map_or(0.0, |w| w.zoning(x, z).wealth),
            "age" => self.constraints.map_or(0.0, |w| w.zoning(x, z).age),
            _ => match (name.strip_prefix("zone:"), self.constraints) {
                (Some(tag), Some(w)) => w.zone(tag, x, z),
                _ => 0.0,
            },
        }
    }
}

impl TerrainWorld<'_> {
    /// The terrain style at (x, z): its region's, when the constraints
    /// carry regions (crate::regions).
    pub fn style_at(&self, x: f32, z: f32) -> Style { self.constraints.map_or(*self.style, |w| w.style_at(x, z, self.style)) }
    fn near(&self, x: f32, z: f32) -> Option<world_constraints::Near> { self.constraints.and_then(|w| w.near(x, z)) }
}

/// One grid cell of a rule: its own stable random numbers.
#[derive(Clone, Copy, Debug)]
pub struct Cell { pub i: i32, pub j: i32, seed: u32 }

impl Cell {
    pub const fn new(i:i32,j:i32,seed:u32)->Self {Self {i,j,seed}}
    /// A uniform 0..1 number, channel `k`. Channels 1-3 are the placer's
    /// (jitter x, jitter z, roll); rules use 4 and up.
    pub fn hash(&self, k: u32) -> f32 { forest_field::hash(self.i, self.j, k, self.seed) }
}

/// A candidate spot: its cell, position and context.
pub struct Spot<'a> {
    pub cell: Cell,
    pub ground: Sample,
    pub world: &'a dyn World,
}

impl Spot<'_> {
    pub fn field(&self, name: &str) -> f32 { self.world.field(name, self.ground.pos.x, self.ground.pos.z, &self.ground) }
}

/// How a placed object stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub yaw: f32,
    pub scale: Vec3,
    pub variant: u32,
    /// Metres above (or into, negative) the ground.
    pub lift: f32,
}

type DensityFn = dyn Fn(&Spot) -> f32 + Send + Sync;
type ShapeFn = dyn Fn(&Spot) -> Shape + Send + Sync;

/// One kind of object.
pub struct Rule {
    /// Stable kind id: part of every object id, so keep it when editing rules.
    pub id: u32,
    pub name: &'static str,
    /// Metres between candidate spots.
    pub cell: f32,
    /// Mixed into the world seed (0: the world seed itself).
    pub salt: u32,
    /// Upper bound of `density`: spots rolling above it are skipped before
    /// the terrain is evaluated, which keeps sparse rules cheap.
    pub cap: f32,
    /// Probability (0..cap) that a spot carries the object.
    pub density: Box<DensityFn>,
    pub shape: Box<ShapeFn>,
    /// Keep this far (metres, in x/z) from objects of earlier rules.
    pub avoid: Vec<(u32, f32)>,
}

/// A placed object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    pub rule: u32,
    /// Stable over sessions and machines: rule id and grid cell.
    pub id: u64,
    pub pos: Vec3,
    pub yaw: f32,
    pub scale: Vec3,
    pub variant: u32,
}

/// The stable id of rule `rule`'s object in cell (i, j).
pub fn instance_id(rule: u32, i: i32, j: i32) -> u64 {
    (u64::from(rule) << 48) ^ (u64::from(i as u32 & 0xff_ffff) << 24) ^ u64::from(j as u32 & 0xff_ffff)
}

/// Every object of `rules` within `radius` of `centre` (x, z), rules in
/// order (later ones avoid earlier ones), each rule row by row.
pub fn place(rules: &[Rule], world: &dyn World, centre: Vec2, radius: f32) -> Vec<Instance> {
    // An earlier rule is placed a little wider, so an object near the edge
    // still sees what it avoids just outside the disc.
    let margin: Vec<f32> = rules.iter().map(|r| rules.iter().flat_map(|later| &later.avoid)
        .filter(|(id, _)| *id == r.id).map(|(_, d)| *d).fold(0.0, f32::max)).collect();
    let mut placed: Vec<Instance> = Vec::new();
    for (rule, margin) in rules.iter().zip(margin) {
        let reach = rule.avoid.iter().map(|(_, d)| *d).fold(0.0, f32::max);
        let grid = Grid::new(&placed, &rule.avoid, reach);
        let seed = world.seed() ^ rule.salt;
        let r = radius + margin;
        let (i0, i1) = (((centre.x - r) / rule.cell).floor() as i32, ((centre.x + r) / rule.cell).ceil() as i32);
        let (j0, j1) = (((centre.y - r) / rule.cell).floor() as i32, ((centre.y + r) / rule.cell).ceil() as i32);
        let mut mine = Vec::new();
        for j in j0..j1 {
            for i in i0..i1 {
                let cell = Cell { i, j, seed };
                let (x, z) = ((i as f32 + cell.hash(1)) * rule.cell, (j as f32 + cell.hash(2)) * rule.cell);
                if Vec2::new(x - centre.x, z - centre.y).length() > r { continue; }
                let roll = cell.hash(3);
                if roll >= rule.cap { continue; }
                let spot = Spot { cell, ground: world.ground(x, z), world };
                if roll >= (rule.density)(&spot) { continue; }
                if grid.blocks(Vec2::new(x, z)) { continue; }
                let shape = (rule.shape)(&spot);
                mine.push(Instance { rule: rule.id, id: instance_id(rule.id, i, j), pos: Vec3::new(x, spot.ground.pos.y + shape.lift, z),
                    yaw: shape.yaw, scale: shape.scale, variant: shape.variant });
            }
        }
        placed.extend(mine);
    }
    placed.retain(|o| Vec2::new(o.pos.x - centre.x, o.pos.z - centre.y).length() <= radius);
    placed
}

/// Earlier objects a rule avoids, bucketed by its largest avoid distance.
struct Grid<'a> { size: f32, cells: HashMap<(i32, i32), Vec<(Vec2, f32)>>, _p: std::marker::PhantomData<&'a ()> }

impl Grid<'_> {
    fn new(placed: &[Instance], avoid: &[(u32, f32)], reach: f32) -> Self {
        let size = reach.max(1.0);
        let mut cells: HashMap<(i32, i32), Vec<(Vec2, f32)>> = HashMap::new();
        for o in placed {
            if let Some(&(_, d)) = avoid.iter().find(|(id, _)| *id == o.rule) {
                let p = Vec2::new(o.pos.x, o.pos.z);
                cells.entry(((p.x / size).floor() as i32, (p.y / size).floor() as i32)).or_default().push((p, d));
            }
        }
        Self { size, cells, _p: std::marker::PhantomData }
    }

    fn blocks(&self, p: Vec2) -> bool {
        let (ci, cj) = ((p.x / self.size).floor() as i32, (p.y / self.size).floor() as i32);
        (-1..=1).any(|dj| (-1..=1).any(|di| self.cells.get(&(ci + di, cj + dj))
            .is_some_and(|v| v.iter().any(|(q, d)| q.distance(p) < *d))))
    }
}

/// Rule ids of the built-in nature rules.
pub const TREE: u32 = 1;
pub const BOULDER: u32 = 2;
pub const DEADWOOD: u32 = 3;
pub const FLOWERS: u32 = 4;

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Trees, exactly as `forest_field::tree`: forest share times `density`,
/// three variants.
pub fn tree_rule(density: f32) -> Rule {
    Rule {
        id: TREE, name: "tree", cell: forest_field::CELL, salt: 0, cap: density,
        density: Box::new(move |s| s.field("forest") * density * off_road(s, 0.5)),
        shape: Box::new(|s| {
            let width = 0.75 + 0.5 * s.cell.hash(4);
            Shape { yaw: s.cell.hash(6) * std::f32::consts::TAU, scale: Vec3::new(width, width * (0.9 + 0.25 * s.cell.hash(5)), width),
                variant: (s.cell.hash(7) * 3.0) as u32 % 3, lift: -0.2 }
        }),
        avoid: Vec::new(),
    }
}

/// Boulders where bedrock breaks through and on scree, a few in the grass
/// (erratics), half sunk; clear of tree trunks.
pub fn boulder_rule() -> Rule {
    Rule {
        id: BOULDER, name: "boulder", cell: 6.0, salt: 0xb01d_3e55, cap: 0.55,
        density: Box::new(|s| {
            let g = &s.ground;
            if g.water > 0.01 || g.snow > 0.3 { return 0.0; }
            // Most at the edges of rock, where it crumbles, and on scree.
            let edge = smooth(0.05, 0.3, g.rock) * (1.0 - smooth(0.7, 0.95, g.rock));
            (0.5 * edge + 0.35 * smooth(0.2, 0.6, g.scree) + 0.01) * off_road(s, 1.0)
        }),
        shape: Box::new(|s| {
            let size = 0.4 + 2.2 * s.cell.hash(4).powi(3) + 0.6 * s.ground.scree;
            let squash = 0.55 + 0.35 * s.cell.hash(5);
            Shape { yaw: s.cell.hash(6) * std::f32::consts::TAU, scale: Vec3::new(size, size * squash, size * (0.7 + 0.5 * s.cell.hash(7))),
                variant: (s.cell.hash(8) * 4.0) as u32 % 4, lift: -0.25 * size * squash }
        }),
        avoid: vec![(TREE, 1.5)],
    }
}

/// Fallen trunks and stumps in the forest, clear of standing trees.
pub fn deadwood_rule() -> Rule {
    Rule {
        id: DEADWOOD, name: "deadwood", cell: 10.0, salt: 0xdead_0d00, cap: 0.3,
        density: Box::new(|s| 0.3 * smooth(0.4, 0.9, s.field("forest")) * off_road(s, 1.0)),
        shape: Box::new(|s| {
            let length = 3.0 + 7.0 * s.cell.hash(4);
            Shape { yaw: s.cell.hash(6) * std::f32::consts::TAU, scale: Vec3::new(0.2 + 0.25 * s.cell.hash(5), 0.2 + 0.25 * s.cell.hash(5), length),
                // 0 fallen trunk, 1 stump.
                variant: u32::from(s.cell.hash(7) < 0.3), lift: 0.0 }
        }),
        avoid: vec![(TREE, 2.0), (BOULDER, 2.0)],
    }
}

/// Flower patches in open, dry meadow (not in forest, not on moor or rock).
pub fn flower_rule() -> Rule {
    Rule {
        id: FLOWERS, name: "flowers", cell: 4.0, salt: 0xf10e_0055, cap: 0.35,
        density: Box::new(|s| {
            let g = &s.ground;
            if s.field("grows") == 0.0 { return 0.0; }
            0.35 * (1.0 - smooth(0.05, 0.3, s.field("forest"))) * (1.0 - g.moor) * (1.0 - smooth(0.3, 0.8, g.wet)) * (1.0 - smooth(0.05, 0.2, g.rock))
                * off_road(s, 0.5)
        }),
        shape: Box::new(|s| {
            let size = 0.8 + 1.6 * s.cell.hash(4);
            Shape { yaw: s.cell.hash(6) * std::f32::consts::TAU, scale: Vec3::new(size, 0.3 + 0.2 * s.cell.hash(5), size),
                variant: (s.cell.hash(7) * 3.0) as u32 % 3, lift: 0.0 }
        }),
        avoid: vec![(TREE, 2.5), (BOULDER, 1.5), (DEADWOOD, 1.5)],
    }
}

/// 0 on and near a road or pad, 1 from `margin` + 1.5 m past its edge:
/// nature rules multiply their density by it so roads stay clear.
pub fn off_road(s: &Spot, margin: f32) -> f32 { smooth(margin, margin + 1.5, s.world.clearance(s.ground.pos.x, s.ground.pos.z)) }

/// The built-in nature rules, in layer order.
pub fn nature_rules(tree_density: f32) -> Vec<Rule> {
    vec![tree_rule(tree_density), boulder_rule(), deadwood_rule(), flower_rule()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_rule_is_the_forest_exactly() {
        for style in ["default", "alpine", "lowland"] {
            let style = Style::named(style).unwrap();
            for (seed, centre) in [(1, Vec2::ZERO), (7, Vec2::new(3_000.0, -1_200.0))] {
                let world = TerrainWorld { seed, style: &style, constraints: None };
                let forest = forest_field::trees_in_disc(centre, 500.0, seed, &style, 0.6);
                let placed = place(&[tree_rule(0.6)], &world, centre, 500.0);
                assert!(forest.len() > 100);
                assert_eq!(forest.len(), placed.len());
                for (t, p) in forest.iter().zip(&placed) {
                    assert_eq!((t.pos, t.yaw, t.scale, t.variant), (p.pos, p.yaw, p.scale, p.variant));
                }
            }
        }
    }

    #[test]
    fn placement_is_local_and_layers_keep_clear() {
        let style = Style::named("alpine").unwrap();
        let world = TerrainWorld { seed: 3, style: &style, constraints: None };
        let big = place(&nature_rules(0.6), &world, Vec2::ZERO, 600.0);
        let small = place(&nature_rules(0.6), &world, Vec2::new(150.0, 80.0), 200.0);
        // Same objects, same ids, whatever area is asked.
        let by_id: HashMap<u64, &Instance> = big.iter().map(|o| (o.id, o)).collect();
        assert!(!small.is_empty());
        for o in &small { assert_eq!(by_id.get(&o.id), Some(&o), "{} differs between areas", o.id); }
        let count = |r| big.iter().filter(|o| o.rule == r).count();
        for r in [TREE, BOULDER, DEADWOOD, FLOWERS] { assert!(count(r) > 0, "rule {r} placed nothing"); }
        // Later layers keep their distance from earlier ones.
        for rule in nature_rules(0.6) {
            for &(other, d) in &rule.avoid {
                for a in big.iter().filter(|o| o.rule == rule.id) {
                    for b in big.iter().filter(|o| o.rule == other) {
                        assert!(Vec2::new(a.pos.x - b.pos.x, a.pos.z - b.pos.z).length() >= d, "{} too close to {}", rule.name, other);
                    }
                }
            }
        }
        // Ids are unique.
        assert_eq!(by_id.len(), big.len());
        // Boulders gather on rock edges and scree.
        let boulders: Vec<_> = big.iter().filter(|o| o.rule == BOULDER).collect();
        let stony = boulders.iter().filter(|o| { let g = world.ground(o.pos.x, o.pos.z); g.rock > 0.05 || g.scree > 0.2 }).count();
        assert!(stony * 2 > boulders.len(), "{stony} of {} boulders on stone", boulders.len());
    }
    #[test]
    fn nature_keeps_off_the_road_and_reads_the_track() {
        let style = Style::default();
        let (_, constraints) = crate::world_constraints::tests_ring(4, &style);
        let world = TerrainWorld { seed: 4, style: &style, constraints: Some(&constraints) };
        let placed = place(&nature_rules(0.7), &world, Vec2::new(570.0, -60.0), 200.0);
        assert!(placed.len() > 200, "{} placed", placed.len());
        for p in &placed { assert!(constraints.clearance(p.pos.x, p.pos.z) > 0.5, "{p:?} on the road"); }
        let n = constraints.near(570.0, -60.0).expect("a road near the ring's east side");
        let g = world.ground(570.0, -60.0);
        assert_eq!(world.field("track_distance", 570.0, -60.0, &g), n.distance);
        assert_eq!(world.field("track_along", 570.0, -60.0, &g), n.along);
    }

}
