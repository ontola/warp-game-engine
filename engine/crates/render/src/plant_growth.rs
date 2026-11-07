//! Plants that grow: a species is a set of growth rules, an age in years runs
//! them, and what the plant looks like follows from what grew, died and was
//! shed. Counts come out of the simulation (a seedling has one needle tuft, an
//! old tree thousands), so age is never a scale on a finished model.
//!
//! The rules are architectural, one step per year: an apex extends, sets a
//! whorl of side buds, and carries foliage on its last few years of shoot. A
//! shaded apex slows and dies; a limb with nothing alive on it dies and is shed
//! to a stub. Same species, age and seed give the same plant.
use glam::Vec3;
use std::collections::HashMap;

/// Growth rules of one species. Lengths in metres, times in years.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Species {
    /// Height the leader approaches, and how fast: h(t) = height (1 - e^-(t/pace))^shape.
    pub height: f32,
    pub pace: f32,
    pub shape: f32,
    /// Year the first whorl of side branches appears.
    pub first_whorl: u32,
    /// Side branches per whorl, the lowest and highest count.
    pub whorl: [u32; 2],
    /// Side shoots an order-1 or finer apex sets each year.
    pub laterals: f32,
    /// Finest branch order: trunk 0, limbs 1, then twigs.
    pub max_order: u32,
    /// Yearly length of a limb's shoot when young, and of each finer order over its parent.
    pub limb_step: f32,
    pub side_length: f32,
    /// Years a limb lives at most, the shortest and longest.
    pub limb_life: [u32; 2],
    /// Live shoot tips a 1.5 m cell holds before the crowded ones die.
    pub crowding: u32,
    /// Branch angle from the parent's axis, radians, when young and old.
    pub angle: [f32; 2],
    /// Upward bend per year of a side apex, and droop of its older wood.
    pub rise: f32,
    pub droop: f32,
    /// Years foliage stays on a shoot.
    pub needle_years: u32,
    /// Light is e^-(shade * foliage above); an apex under `min_light` dies.
    pub shade: f32,
    pub min_light: f32,
    /// Radius of a twig tip, the wood laid on per year^0.93 (the trunk's; a
    /// limb takes `limb_wood` of it), and years a dead limb stands before it snaps.
    pub tip_radius: f32,
    pub wood: f32,
    pub limb_wood: f32,
    pub dead_years: u32,
    /// Needle brushes on each young side shoot: 1 at its tip; more sit along the
    /// shoot and splay outward (a fraction is a chance of one more).
    pub plumes: f32,
    /// Share of the tree's height, from the top, that keeps living limbs once
    /// it is `lift_years` old (1 keeps them all): an old pine sheds its lower
    /// limbs and stands on a clear trunk. The crown lifts gradually from year 10.
    pub live_crown: f32,
    pub lift_years: f32,
}

impl Default for Species {
    fn default() -> Self { Self::jeffrey_pine() }
}

impl Species {
    /// Pinus jeffreyi: a straight leader, whorled limbs, long needles in
    /// tufts at the shoot ends; a cone young, a clear trunk and an open crown old.
    pub fn jeffrey_pine() -> Self {
        Self {
            height: 46.0, pace: 45.0, shape: 1.75, first_whorl: 3, whorl: [3, 5], laterals: 1.0, max_order: 4,
            limb_step: 0.22, side_length: 0.6, limb_life: [60, 150], crowding: 8, angle: [1.05, 1.4], rise: 0.05, droop: 0.012,
            needle_years: 5, shade: 0.025, min_light: 0.3, tip_radius: 0.005, wood: 0.0035, limb_wood: 0.62, dead_years: 18,
            plumes: 2.0, live_crown: 0.55, lift_years: 60.0,
        }
    }
}

/// One year's length of shoot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub from: Vec3,
    pub to: Vec3,
    pub radius: [f32; 2],
    pub order: u32,
    pub born: u32,
    pub parent: Option<u32>,
    pub alive: bool,
}

/// A cluster of needles at a shoot end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuft {
    pub at: Vec3,
    pub axis: Vec3,
    /// 0..1: how much of a full tuft is left on this shoot.
    pub fullness: f32,
    pub segment: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Plant {
    pub age: u32,
    pub segments: Vec<Segment>,
    pub tufts: Vec<Tuft>,
}

impl Plant {
    pub fn height(&self) -> f32 {
        self.segments.iter().map(|s| s.to.y).fold(0.0, f32::max)
    }
    /// Height of the lowest living foliage.
    pub fn crown_base(&self) -> f32 {
        self.tufts.iter().map(|t| t.at.y).fold(f32::MAX, f32::min).min(self.height())
    }
    pub fn crown_radius(&self) -> f32 {
        self.tufts.iter().map(|t| (t.at * Vec3::new(1.0, 0.0, 1.0)).length()).fold(0.0, f32::max)
    }
    pub fn trunk_radius(&self) -> f32 {
        self.segments.first().map_or(0.0, |s| s.radius[0])
    }
}

#[derive(Clone, Copy)]
struct Apex {
    at: Vec3,
    axis: Vec3,
    order: u32,
    segment: Option<u32>,
    vigour: f32,
    /// The year this apex's limb started and the years it may live.
    limb: [u32; 2],
}

fn hash(seed: u64, a: u64, b: u64) -> f32 {
    let mut x = seed ^ a.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ b.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((x ^ (x >> 31)) >> 40) as f32 / (1u64 << 24) as f32
}

fn frame(axis: Vec3) -> (Vec3, Vec3) {
    let u = axis.cross(if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X }).normalize();
    (u, axis.cross(u))
}

fn leader_height(s: &Species, t: f32) -> f32 {
    s.height * (1.0 - (-t / s.pace).exp()).powf(s.shape)
}

const SHADE_CELL: f32 = 1.5;

/// Foliage above each cell of a coarse grid: counted per column from the top,
/// so an apex reads the shade it stands under in one lookup.
fn shade_map(tips: &[Apex]) -> HashMap<(i32, i32, i32), f32> {
    let mut columns: HashMap<(i32, i32), Vec<i32>> = HashMap::new();
    for a in tips {
        let k = (a.at / SHADE_CELL).floor();
        for dz in -1..=1 {
            for dx in -1..=1 {
                columns.entry((k.x as i32 + dx, k.z as i32 + dz)).or_default().push(k.y as i32);
            }
        }
    }
    let mut map = HashMap::new();
    for ((x, z), mut ys) in columns {
        ys.sort_unstable_by(|a, b| b.cmp(a));
        let mut above = 0;
        let mut i = 0;
        while i < ys.len() {
            let y = ys[i];
            map.insert((x, y, z), above as f32);
            while i < ys.len() && ys[i] == y {
                i += 1;
                above += 1;
            }
        }
    }
    map
}

fn foliage_above(map: &HashMap<(i32, i32, i32), f32>, at: Vec3) -> f32 {
    let k = (at / SHADE_CELL).floor();
    map.get(&(k.x as i32, k.y as i32, k.z as i32)).copied().unwrap_or(0.0) / 9.0
}

/// Grow `species` for `age` years. Deterministic in its arguments.
pub fn grow(species: &Species, age: u32, seed: u64) -> Plant {
    let s = species;
    let mut segments: Vec<Segment> = Vec::new();
    let mut apices = vec![Apex { at: Vec3::ZERO, axis: Vec3::Y, order: 0, segment: None, vigour: 1.0, limb: [0, u32::MAX] }];
    for year in 1..=age.max(1) {
        let map = shade_map(&apices);
        let mut crowd: HashMap<(i32, i32, i32), u32> = HashMap::new();
        for a in &apices {
            if a.order > 0 {
                let k = (a.at / SHADE_CELL).floor();
                *crowd.entry((k.x as i32, k.y as i32, k.z as i32)).or_default() += 1;
            }
        }
        // The crown's reach in each height band: tips near it see side light.
        let mut reach: HashMap<i32, f32> = HashMap::new();
        for a in &apices {
            let r = reach.entry((a.at.y / SHADE_CELL).floor() as i32).or_default();
            *r = r.max(Vec3::new(a.at.x, 0.0, a.at.z).length());
        }
        let leader_step = leader_height(s, year as f32) - leader_height(s, year as f32 - 1.0);
        let maturity = (year as f32 / 120.0).min(1.0);
        let mut next = Vec::with_capacity(apices.len() * 2);
        for (index, apex) in apices.iter().enumerate() {
            // Light from above, through the foliage over this cell, plus side
            // light for tips near the crown's edge: an open-grown pine stays
            // green low down.
            let top = (-s.shade * foliage_above(&map, apex.at)).exp();
            let edge = reach.get(&((apex.at.y / SHADE_CELL).floor() as i32)).copied().unwrap_or(0.0);
            let side = if edge > 0.5 { (Vec3::new(apex.at.x, 0.0, apex.at.z).length() / edge).clamp(0.0, 1.0).powi(2) } else { 1.0 };
            let light = top + (1.0 - top) * side * 0.5;
            let id = (year as u64) << 32 | index as u64;
            if apex.order > 0 {
                let k = (apex.at / SHADE_CELL).floor();
                let crowd = *crowd.get(&(k.x as i32, k.y as i32, k.z as i32)).unwrap_or(&0);
                // Crowding prunes twigs, never a limb's own leader: a limb
                // whose leader dies stops making wood and is shed.
                let crowded = apex.order > 1 && crowd > s.crowding && hash(seed, id, 70) < 1.0 - s.crowding as f32 / crowd as f32;
                // Past its span a limb only lives on where its shoots still reach good light
                // from above: dominant lower limbs of an old pine keep going, shaded ones do not.
                let spent = year - apex.limb[0] > apex.limb[1] && top < 0.75;
                // A limb leader holds on until it is truly dark.
                let dark = light < if apex.order == 1 { s.min_light * 0.4 } else { s.min_light };
                // Crown lift: a limb set low on the trunk dies once the crown has
                // lifted past where it springs from, the whole limb at once.
                let lift = ((year as f32 - 10.0) / (s.lift_years - 10.0).max(1.0)).clamp(0.0, 1.0);
                let floor = leader_height(s, year as f32) * (1.0 - s.live_crown.clamp(0.0, 1.0)) * lift;
                let base = leader_height(s, apex.limb[0] as f32);
                let lifted = base < floor * (0.85 + 0.3 * hash(seed, apex.limb[0] as u64 * 7919 + apex.limb[1] as u64, 71));
                if dark || crowded || spent || lifted {
                    continue;
                }
            }
            let wander = Vec3::new(hash(seed, id, 1) - 0.5, 0.0, hash(seed, id, 2) - 0.5);
            let length = if apex.order == 0 {
                leader_step.max(0.03)
            } else {
                let slowing = 1.0 / (1.0 + (year - apex.limb[0]) as f32 / 80.0);
                (s.limb_step * s.side_length.powi(apex.order as i32 - 1) * slowing * apex.vigour * light.sqrt() * (0.7 + 0.6 * hash(seed, id, 3))).max(0.02)
            };
            let axis = if apex.order == 0 {
                (apex.axis + wander * 0.05 + Vec3::Y * 0.3).normalize()
            } else {
                let lean = s.rise * (1.0 - maturity) + Vec3::Y.dot(apex.axis).max(0.0) * -s.droop * 2.0;
                (apex.axis + Vec3::Y * lean + wander * 0.22).normalize()
            };
            let to = apex.at + axis * length;
            let segment = segments.len() as u32;
            segments.push(Segment { from: apex.at, to, radius: [0.0; 2], order: apex.order, born: year, parent: apex.segment, alive: true });
            next.push(Apex { at: to, axis, order: apex.order, segment: Some(segment), vigour: apex.vigour * if apex.order == 0 { 1.0 } else { 0.985 }, limb: apex.limb });
            let (u, v) = frame(axis);
            let spread = s.angle[0] + (s.angle[1] - s.angle[0]) * maturity;
            if apex.order == 0 {
                if year >= s.first_whorl {
                    let count = s.whorl[0] + (hash(seed, id, 4) * (s.whorl[1] - s.whorl[0] + 1) as f32) as u32;
                    let turn = hash(seed, id, 5) * std::f32::consts::TAU;
                    for i in 0..count {
                        let a = turn + i as f32 / count as f32 * std::f32::consts::TAU + (hash(seed, id, 6 + i as u64) - 0.5) * 0.5;
                        let out = (u * a.cos() + v * a.sin()) * spread.sin() + axis * spread.cos();
                        let life = s.limb_life[0] + (hash(seed, id, 60 + i as u64) * (s.limb_life[1] - s.limb_life[0]) as f32) as u32;
                        next.push(Apex { at: to, axis: out.normalize(), order: 1, segment: Some(segment), vigour: 0.75 + 0.5 * hash(seed, id, 20 + i as u64), limb: [year, life] });
                    }
                }
            } else if apex.order < s.max_order {
                let mut count = s.laterals * light;
                let mut i = 0;
                while count > 0.0 {
                    if hash(seed, id, 30 + i) < count.min(1.0) {
                        let a = hash(seed, id, 40 + i) * std::f32::consts::TAU;
                        // Flattened sprays: mostly sideways, the crown stays layered.
                        let side = u * a.cos() + v * a.sin() * 0.45;
                        next.push(Apex { at: to, axis: (axis * 0.72 + side.normalize() * 0.7).normalize(), order: apex.order + 1, segment: Some(segment), vigour: apex.vigour * (0.7 + 0.3 * hash(seed, id, 50 + i)), limb: apex.limb });
                    }
                    count -= 1.0;
                    i += 1;
                }
            }
        }
        apices = next;
    }
    finish(s, age.max(1), seed, segments)
}

/// Radii from years of wood, foliage on young shoots, dead limbs shed.
fn finish(s: &Species, age: u32, seed: u64, mut segments: Vec<Segment>) -> Plant {
    let n = segments.len();
    let mut newest = vec![0u32; n];
    for i in (0..n).rev() {
        let seg = segments[i];
        newest[i] = newest[i].max(seg.born);
        if let Some(p) = seg.parent {
            newest[p as usize] = newest[p as usize].max(newest[i]);
        }
    }
    let mut keep = vec![true; n];
    for i in 0..n {
        let dead_for = age.saturating_sub(newest[i]);
        segments[i].alive = dead_for <= s.needle_years;
        if segments[i].order > 0 && dead_for > s.needle_years {
            // Dead wood falls from the tips in, leaving a stub on the parent:
            // each finer order stands half as long (twigs go in a few years).
            let stands = s.dead_years >> segments[i].order.min(31);
            let parent_alive = segments[i].parent.is_some_and(|p| age.saturating_sub(newest[p as usize]) <= s.needle_years);
            keep[i] = dead_for < stands || (parent_alive && segments[i].order == 1 && hash(seed, i as u64, 90) < 0.55);
        }
        if let Some(p) = segments[i].parent {
            keep[i] = keep[i] && keep[p as usize];
        }
    }
    let mut tufts = Vec::new();
    for i in 0..n {
        let seg = segments[i];
        if !keep[i] {
            continue;
        }
        // Wood is laid on every year the part stays alive, so a stem tapers
        // towards its young end by itself.
        let years = age.saturating_sub(seg.born);
        let grew = newest[i].saturating_sub(seg.born) as f32;
        let wood = s.wood * if seg.order == 0 { 1.0 } else { s.limb_wood.powi(seg.order as i32) };
        let r = s.tip_radius + wood * grew.powf(0.93);
        segments[i].radius = [r, r * 0.97];
        if years < s.needle_years {
            let axis = (seg.to - seg.from).normalize_or_zero();
            let fullness = 1.0 - years as f32 / s.needle_years as f32 * 0.65;
            tufts.push(Tuft { at: seg.to, axis, fullness, segment: i as u32 });
            // More brushes along side shoots, each turned outward a little
            // (the leader keeps one: a seedling stays a few tufts).
            let extra = if seg.order == 0 { 0.0 } else { (s.plumes - 1.0).max(0.0) };
            let count = extra.floor() as u32 + u32::from(hash(seed, i as u64, 91) < extra.fract());
            let (u, v) = frame(if axis == Vec3::ZERO { Vec3::Y } else { axis });
            for k in 0..count {
                let f = 1.0 - (k + 1) as f32 / (count + 1) as f32;
                let a = hash(seed, i as u64, 92 + k as u64) * std::f32::consts::TAU;
                let out = (axis + (u * a.cos() + v * a.sin()) * 0.6).normalize_or_zero();
                tufts.push(Tuft { at: seg.from.lerp(seg.to, f), axis: out, fullness: fullness * 0.85, segment: i as u32 });
            }
        }
    }
    let mut map = vec![u32::MAX; n];
    let mut kept = Vec::new();
    for i in 0..n {
        if keep[i] {
            map[i] = kept.len() as u32;
            let mut seg = segments[i];
            seg.parent = seg.parent.map(|p| map[p as usize]);
            kept.push(seg);
        }
    }
    for t in &mut tufts {
        t.segment = map[t.segment as usize];
    }
    Plant { age, segments: kept, tufts }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_changes_counts_not_just_size() {
        let s = Species::jeffrey_pine();
        let (seedling, young, old) = (grow(&s, 2, 1), grow(&s, 15, 1), grow(&s, 150, 1));
        assert!(seedling.tufts.len() <= 3, "{}", seedling.tufts.len());
        assert!(young.tufts.len() > 20 * seedling.tufts.len());
        assert!(old.tufts.len() > 5 * young.tufts.len());
        assert!(old.height() > 5.0 * young.height());
    }

    #[test]
    fn an_old_tree_clears_its_trunk() {
        let s = Species::jeffrey_pine();
        let (young, old) = (grow(&s, 12, 2), grow(&s, 200, 2));
        assert!(young.crown_base() < 0.35 * young.height());
        // Old jeffreyi: a clear lower trunk under big, still living limbs.
        assert!(old.crown_base() > 0.08 * old.height(), "{} of {}", old.crown_base(), old.height());
        // About a fifth: denser crowns (crowding 16, 5 needle years) land at 0.195.
        assert!(old.crown_radius() > 0.18 * old.height(), "radius {} of {}", old.crown_radius(), old.height());
    }

    #[test]
    fn an_old_pine_sheds_its_lower_limbs_even_when_crowding_spares_them() {
        // Joep's tuning: crowding never prunes, many limbs per whorl.
        let s = Species { crowding: 60, whorl: [2, 12], laterals: 2.0, height: 52.0, ..Species::jeffrey_pine() };
        for (age, share) in [(50, 0.25), (120, 0.35)] {
            let tree = grow(&s, age, 1);
            assert!(tree.crown_base() > share * tree.height(), "age {age}: crown base {} of {}", tree.crown_base(), tree.height());
        }
        let open = grow(&Species { live_crown: 1.0, ..s }, 120, 1);
        assert!(open.crown_base() < 0.2 * open.height(), "live_crown 1 keeps low limbs: {}", open.crown_base());
    }

    #[test]
    fn the_same_inputs_grow_the_same_plant() {
        let s = Species::jeffrey_pine();
        assert_eq!(grow(&s, 40, 7).segments, grow(&s, 40, 7).segments);
        assert_ne!(grow(&s, 40, 7).segments, grow(&s, 40, 8).segments);
    }
}
