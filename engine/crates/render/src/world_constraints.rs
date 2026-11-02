//! Constraints the world is generated around: what an artist or a game
//! fixes, while everything else stays a function of position and seed.
//! A race game's track is the first one: its road is exactly where and how
//! high the game says, and the terrain bends to it with cuttings and
//! embankments, rivers stop at it, and no tree grows on it. Pads are round
//! flat places (a service park, a village square, a landmark's footing).
//!
//! Storage stays tiny: a track is its centre line, a pad four numbers. The
//! constraints are baked once into a coarse grid of the segments and pads
//! near each cell, so a terrain sample reads one cell's short list: the
//! cost is fixed per sample, on the CPU and on the GPU
//! (`shaders/world_constraints.wgsl`, line for line the same), which keeps
//! low-end hardware in reach.
//!
//! ```ignore
//! let mut track = Track::spline(&control_points, 4.5, true);
//! track.fit_heights(|x, z| terrain_field::height(x, z, seed, &style), 0.08, 80.0);
//! let world = Constraints { tracks: vec![track], pads: vec![] }.bake();
//! let s = world.eval(x, z, cut, seed, &style); // terrain_field::eval with the constraints
//! ```
use crate::forest_field;
use crate::terrain_field::{self, Sample, Style};
use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};

pub const WGSL: &str = include_str!("shaders/world_constraints.wgsl");

/// Grid cell of the baked lookup (metres).
pub const CELL: f32 = 32.0;
/// Flat gravel beside the road before the terrain starts to bend (metres).
pub const VERGE: f32 = 1.0;
/// Shortest and longest distance over which the terrain meets the road
/// again (metres): steep enough for a cutting, never a cliff.
pub const MIN_REACH: f32 = 4.0;
pub const MAX_REACH: f32 = 100.0;
/// Loose stone and worn grass beside the verge, drawn by the terrain's
/// pixels (metres).
pub const SHOULDER: f32 = 2.5;
/// Run of a cutting or embankment per metre of rise.
pub const BANK: f32 = 1.8;
/// Height the bank allows at MAX_REACH past the edge: more than any land,
/// so the constraint ends without a seam (very deep cuts turn into walls).
pub const OPEN: f32 = 9000.0;
/// Most arms (stretches of road or pads) one sample blends.
pub const MAX_ARMS: usize = 8;

/// A road through the world: a centre line with heights and half widths.
#[derive(Clone, Debug, Default)]
pub struct Track {
    /// Centre line, in order. y is the road height.
    pub points: Vec<Vec3>,
    /// Half width of the driven surface at each point (metres).
    pub half: Vec<f32>,
    /// Whether the last point joins the first.
    pub closed: bool,
}

impl Track {
    /// A track along a polyline the game already has (Sideways' stage).
    pub fn polyline(points: Vec<Vec3>, half: Vec<f32>, closed: bool) -> Self {
        assert_eq!(points.len(), half.len());
        assert!(points.len() >= 2, "a track needs two points");
        Track { points, half, closed }
    }

    /// A smooth track through `control` points (Catmull-Rom), sampled every
    /// 4 m, `half` metres either side. Heights come from the control
    /// points; call `fit_heights` to lay the road on the land instead.
    pub fn spline(control: &[Vec3], half: f32, closed: bool) -> Self {
        assert!(control.len() >= 2);
        let n = control.len();
        let at = |i: isize| -> Vec3 {
            if closed { control[i.rem_euclid(n as isize) as usize] } else { control[i.clamp(0, n as isize - 1) as usize] }
        };
        let spans = if closed { n } else { n - 1 };
        let mut points = Vec::new();
        for s in 0..spans as isize {
            let (p0, p1, p2, p3) = (at(s - 1), at(s), at(s + 1), at(s + 2));
            let steps = ((p2 - p1).length() / 4.0).ceil().max(1.0) as usize;
            for k in 0..steps {
                let t = k as f32 / steps as f32;
                let (t2, t3) = (t * t, t * t * t);
                points.push(0.5 * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3));
            }
        }
        if !closed { points.push(control[n - 1]); }
        let half = vec![half; points.len()];
        Track { points, half, closed }
    }

    /// A winding closed loop about `radius` metres around `centre`, `half`
    /// metres either side, from `seed`: a test track, or a start for one.
    /// Heights are 0; call `fit_heights`.
    pub fn wander_loop(centre: Vec2, radius: f32, half: f32, seed: u32) -> Self {
        let n = 14;
        let control: Vec<Vec3> = (0..n).map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            let r = radius * (0.7 + 0.6 * forest_field::hash(k, 0, 11, seed));
            let jitter = 0.25 * (forest_field::hash(k, 1, 11, seed) - 0.5) * std::f32::consts::TAU / n as f32;
            Vec3::new(centre.x + r * (a + jitter).cos(), 0.0, centre.y + r * (a + jitter).sin())
        }).collect();
        Track::spline(&control, half, true)
    }

    /// Lay the road on `ground`: its heights follow the land, smoothed over
    /// `window` metres and with grades no steeper than `max_grade`. The
    /// land then bends where the road cannot follow it.
    pub fn fit_heights(&mut self, ground: impl Fn(f32, f32) -> f32, max_grade: f32, window: f32) {
        let n = self.points.len();
        let raw: Vec<f32> = self.points.iter().map(|p| ground(p.x, p.z)).collect();
        let along = self.along();
        let total = *along.last().unwrap() + if self.closed { self.points[n - 1].xz().distance(self.points[0].xz()) } else { 0.0 };
        // Box filter by distance along the road (wrapping on a loop).
        let mut y = vec![0.0f32; n];
        for i in 0..n {
            let (mut sum, mut weight) = (0.0f32, 0.0f32);
            for (j, h) in raw.iter().enumerate() {
                let mut d = (along[j] - along[i]).abs();
                if self.closed { d = d.min(total - d); }
                if d <= window * 0.5 {
                    let w = 1.0 - d / (window * 0.5 + 1e-3);
                    sum += h * w;
                    weight += w;
                }
            }
            y[i] = sum / weight;
        }
        // Grade limit, forward and back (twice round a loop).
        let passes = if self.closed { 2 } else { 1 };
        for _ in 0..passes {
            for i in 1..n + usize::from(self.closed) {
                let (a, b) = ((i - 1) % n, i % n);
                let run = self.points[a].xz().distance(self.points[b].xz()) * max_grade;
                y[b] = y[b].clamp(y[a] - run, y[a] + run);
            }
            for i in (0..n - usize::from(!self.closed)).rev() {
                let (a, b) = ((i + 1) % n, i);
                let run = self.points[a].xz().distance(self.points[b].xz()) * max_grade;
                y[b] = y[b].clamp(y[a] - run, y[a] + run);
            }
        }
        for (p, h) in self.points.iter_mut().zip(y) { p.y = h; }
    }

    /// Distance along the centre line at each point.
    pub fn along(&self) -> Vec<f32> {
        let mut a = vec![0.0f32; self.points.len()];
        for i in 1..self.points.len() { a[i] = a[i - 1] + self.points[i - 1].xz().distance(self.points[i].xz()); }
        a
    }
}

trait Xz { fn xz(&self) -> Vec2; }
impl Xz for Vec3 { fn xz(&self) -> Vec2 { Vec2::new(self.x, self.z) } }

/// A round flat place: inside `radius` the ground is level at `height`,
/// then it meets the land again over at least `feather` metres.
#[derive(Clone, Copy, Debug)]
pub struct Pad {
    pub centre: Vec2,
    pub radius: f32,
    pub feather: f32,
    /// None: the land's own height at the centre.
    pub height: Option<f32>,
}

/// Everything the world is generated around.
#[derive(Clone, Debug, Default)]
pub struct Constraints {
    pub tracks: Vec<Track>,
    pub pads: Vec<Pad>,
}

/// One straight piece of a road, as the GPU reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Segment {
    /// Start (x, y, z) and its distance along the track.
    pub a: [f32; 4],
    /// End (x, y, z) and its distance along the track.
    pub b: [f32; 4],
    /// Half width at a and b, track index, and 0 (1 for a pad: a.xyz its
    /// centre and height, b.x its radius, b.y its feather).
    pub w: [f32; 4],
}

/// The grid's placement, as the GPU reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Header {
    pub origin: [f32; 2],
    pub cell: f32,
    pub arms: u32,
    pub size: [u32; 2],
    pub segments: u32,
    pub _pad: u32,
}

/// The constraints baked for lookup: a grid of cells, each with the list
/// of segments and pads that can reach it.
#[derive(Clone, Debug, Default)]
pub struct World {
    pub header: Header,
    pub segments: Vec<Segment>,
    /// size.x * size.y + 1 offsets into `items`.
    pub cells: Vec<u32>,
    pub items: Vec<u32>,
    tracks: usize,
}

/// Where a point lies relative to the nearest road.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Near {
    /// Metres from the centre line.
    pub distance: f32,
    /// Signed offset: positive on the right of the direction of travel.
    pub side: f32,
    /// Metres along the track.
    pub along: f32,
    pub track: u32,
    /// Half width of the road here.
    pub half: f32,
    /// Road height here.
    pub height: f32,
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One arm's pull on a sample: its road height, the bank there and its
/// steepness, and how much the point is on the road itself (gravel, no
/// water, no trees).
#[derive(Clone, Copy, Debug, Default)]
struct Pull { y: f32, band: f32, slope: f32, core: f32, near: Near }

impl Constraints {
    pub fn is_empty(&self) -> bool { self.tracks.is_empty() && self.pads.is_empty() }

    /// Bake for lookup. Pads without a height take the land's at their
    /// centre from `seed` and `style`.
    pub fn bake(&self, seed: u32, style: &Style) -> World {
        let mut segments = Vec::new();
        for (t, track) in self.tracks.iter().enumerate() {
            let along = track.along();
            let n = track.points.len();
            let count = if track.closed { n } else { n - 1 };
            for i in 0..count {
                let j = (i + 1) % n;
                let (pa, pb) = (track.points[i], track.points[j]);
                let b_along = if j == 0 { along[i] + pa.xz().distance(pb.xz()) } else { along[j] };
                segments.push(Segment { a: [pa.x, pa.y, pa.z, along[i]], b: [pb.x, pb.y, pb.z, b_along], w: [track.half[i], track.half[j], t as f32, 0.0] });
            }
        }
        for pad in &self.pads {
            let h = pad.height.unwrap_or_else(|| terrain_field::height(pad.centre.x, pad.centre.y, seed, style));
            segments.push(Segment { a: [pad.centre.x, h, pad.centre.y, 0.0], b: [pad.radius, pad.feather.max(MIN_REACH), 0.0, 0.0], w: [0.0, 0.0, 0.0, 1.0] });
        }
        World::from_segments(segments, self.tracks.len())
    }
}

impl World {
    /// No constraints: every lookup returns the land unchanged.
    pub fn empty() -> Self { World::from_segments(Vec::new(), 0) }

    fn reach(s: &Segment) -> (Vec2, Vec2) {
        if s.w[3] > 0.5 {
            let c = Vec2::new(s.a[0], s.a[2]);
            let r = s.b[0] + (s.b[1]).max(MAX_REACH);
            return (c - r, c + r);
        }
        let r = s.w[0].max(s.w[1]) + VERGE + MAX_REACH;
        let (a, b) = (Vec2::new(s.a[0], s.a[2]), Vec2::new(s.b[0], s.b[2]));
        (a.min(b) - r, a.max(b) + r)
    }

    fn from_segments(segments: Vec<Segment>, tracks: usize) -> Self {
        if segments.is_empty() {
            return World { header: Header { size: [1, 1], cell: CELL, ..Default::default() }, segments, cells: vec![0, 0], items: Vec::new(), tracks };
        }
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for s in &segments {
            let (a, b) = Self::reach(s);
            lo = lo.min(a);
            hi = hi.max(b);
        }
        let origin = (lo / CELL).floor() * CELL;
        let size = [((hi.x - origin.x) / CELL).ceil() as u32 + 1, ((hi.y - origin.y) / CELL).ceil() as u32 + 1];
        let mut lists = vec![Vec::<u32>::new(); (size[0] * size[1]) as usize];
        for (k, s) in segments.iter().enumerate() {
            let (a, b) = Self::reach(s);
            let (i0, j0) = (((a.x - origin.x) / CELL).floor() as u32, ((a.y - origin.y) / CELL).floor() as u32);
            let (i1, j1) = (((b.x - origin.x) / CELL).floor() as u32, ((b.y - origin.y) / CELL).floor() as u32);
            for j in j0..=j1.min(size[1] - 1) {
                for i in i0..=i1.min(size[0] - 1) {
                    if Self::reaches(s, origin + Vec2::new(i as f32, j as f32) * CELL) { lists[(j * size[0] + i) as usize].push(k as u32); }
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
        let header = Header { origin: origin.into(), cell: CELL, arms: MAX_ARMS as u32, size, segments: segments.len() as u32, _pad: 0 };
        World { header, segments, cells, items, tracks }
    }

    /// Whether segment `s` can pull any point of the cell at `corner`.
    fn reaches(s: &Segment, corner: Vec2) -> bool {
        let centre = corner + Vec2::splat(CELL * 0.5);
        let r = CELL * std::f32::consts::FRAC_1_SQRT_2;
        if s.w[3] > 0.5 {
            return centre.distance(Vec2::new(s.a[0], s.a[2])) <= s.b[0] + s.b[1].max(MAX_REACH) + r;
        }
        let (d, ..) = closest(s, centre);
        d <= s.w[0].max(s.w[1]) + VERGE + MAX_REACH + r
    }

    pub fn is_empty(&self) -> bool { self.segments.is_empty() }

    /// Most items any cell lists: the worst-case cost of one sample.
    pub fn busiest_cell(&self) -> u32 { self.cells.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0) }

    fn list(&self, x: f32, z: f32) -> &[u32] {
        let h = &self.header;
        let fi = ((x - h.origin[0]) / h.cell).floor();
        let fj = ((z - h.origin[1]) / h.cell).floor();
        if self.segments.is_empty() || fi < 0.0 || fj < 0.0 || fi >= h.size[0] as f32 || fj >= h.size[1] as f32 { return &[]; }
        let c = (fj as u32 * h.size[0] + fi as u32) as usize;
        &self.items[self.cells[c] as usize..self.cells[c + 1] as usize]
    }

    /// The nearest road to (x, z) within its reach, if any.
    pub fn near(&self, x: f32, z: f32) -> Option<Near> {
        let p = Vec2::new(x, z);
        let mut best: Option<Near> = None;
        for &k in self.list(x, z) {
            let s = &self.segments[k as usize];
            if s.w[3] > 0.5 { continue; }
            let n = segment_near(s, p);
            if best.is_none_or(|b| n.distance < b.distance) { best = Some(n); }
        }
        best
    }

    /// Metres from (x, z) to the nearest constrained surface (road edge or
    /// pad rim); negative inside; f32::MAX far from all.
    pub fn clearance(&self, x: f32, z: f32) -> f32 {
        let p = Vec2::new(x, z);
        let mut c = f32::MAX;
        for &k in self.list(x, z) {
            let s = &self.segments[k as usize];
            c = c.min(if s.w[3] > 0.5 { p.distance(Vec2::new(s.a[0], s.a[2])) - s.b[0] } else { let n = segment_near(s, p); n.distance - n.half });
        }
        c
    }

    /// The pulls on (x, z): one per arm (a run of consecutive segments of
    /// a track, or a pad), at its nearest point.
    fn pulls(&self, x: f32, z: f32, out: &mut [Pull; MAX_ARMS]) -> usize {
        let p = Vec2::new(x, z);
        let mut n = 0usize;
        let mut last: Option<(u32, f32)> = None; // previous item, its track
        for &k in self.list(x, z) {
            let s = &self.segments[k as usize];
            let pull = if s.w[3] > 0.5 {
                let d = p.distance(Vec2::new(s.a[0], s.a[2]));
                let near = Near { distance: d, side: 0.0, along: 0.0, track: u32::MAX, half: s.b[0], height: s.a[1] };
                arm_pull(s.a[1], d - s.b[0], 0.0, near)
            } else {
                let near = segment_near(s, p);
                arm_pull(near.height, near.distance - near.half - VERGE, smooth(near.half + VERGE, near.half, near.distance), near)
            };
            // Consecutive segments of one track are one arm: keep its nearest.
            let same = s.w[3] < 0.5 && last.is_some_and(|(prev, t)| prev + 1 == k && t == s.w[2]);
            last = Some((k, if s.w[3] > 0.5 { -1.0 } else { s.w[2] }));
            if same && n > 0 {
                if pull.near.distance < out[n - 1].near.distance { out[n - 1] = pull; }
                continue;
            }
            if n == MAX_ARMS { continue; }
            out[n] = pull;
            n += 1;
        }
        n
    }

    /// terrain_field::eval with the constraints: the land, bent to every
    /// road and pad within reach.
    pub fn eval(&self, x: f32, z: f32, cut: f32, seed: u32, style: &Style) -> Sample {
        let s = terrain_field::eval(x, z, cut, seed, style);
        if self.segments.is_empty() { return s; }
        self.apply(x, z, s)
    }

    /// Bend one natural sample at ground coordinates (x, z). Each arm
    /// allows the land only within its bank, a cone around the road that
    /// widens by 1 / BANK per metre out (a cutting or an embankment) and
    /// opens fully by MAX_REACH; the land is clamped into all of them, with
    /// a rounded edge where the clamp starts.
    pub fn apply(&self, x: f32, z: f32, mut s: Sample) -> Sample {
        let mut pulls = [Pull::default(); MAX_ARMS];
        let n = self.pulls(x, z, &mut pulls);
        if n == 0 { return s; }
        let land = s.pos.y - s.water;
        let (mut lo, mut hi) = (f32::MIN, f32::MAX);
        let (mut lo_slope, mut hi_slope, mut core, mut tight) = (0.0f32, 0.0f32, 0.0f32, f32::MAX);
        for p in &pulls[..n] {
            if p.y - p.band > lo { lo = p.y - p.band; lo_slope = p.slope; }
            if p.y + p.band < hi { hi = p.y + p.band; hi_slope = p.slope; }
            core = core.max(p.core);
            tight = tight.min(p.band);
        }
        let k = (0.5 * tight).min(2.0);
        let y = if lo > hi {
            // Two arms that disagree (roads crossing at different heights).
            0.5 * (lo + hi)
        } else {
            smin(smax(land, lo, k), hi, k)
        };
        let moved = (y - land).abs();
        let pull = smooth(0.0, 0.5, moved).max(core);
        if pull <= 0.0 { return s; }
        // Bent ground: the strata's sideways push fades out, and the road
        // itself is bare gravel.
        s.pos.x += (x - s.pos.x) * pull;
        s.pos.z += (z - s.pos.z) * pull;
        // Standing water keeps its level where the bed rises (it meets an
        // embankment flat) but is never deeper than it was (a cutting does
        // not flood), and the road itself is dry.
        let level = s.pos.y;
        s.water = (level - y).clamp(0.0, s.water) * (1.0 - core);
        s.pos.y = y + s.water;
        let slope = if y > land + 1e-3 { lo_slope } else if y < land - 1e-3 { hi_slope } else { 0.0 };
        let bank_up = 1.0 / (1.0 + slope * slope).sqrt();
        s.up = (s.up + (bank_up - s.up) * smooth(0.0, 0.5, moved)).max(s.up * (1.0 - core) + core);
        s.wet *= 1.0 - core;
        s.snow *= 1.0 - 0.7 * core;
        s.moor *= 1.0 - core;
        s.rock *= 1.0 - core;
        s.scree += (1.0 - s.scree) * core;
        s
    }

    /// Ground height under world (x, z) at full detail, for physics.
    pub fn height(&self, x: f32, z: f32, seed: u32, style: &Style) -> f32 {
        let (mut u, mut v) = (x, z);
        let mut s = self.eval(u, v, 0.0, seed, style);
        for _ in 0..4 {
            u += x - s.pos.x;
            v += z - s.pos.z;
            s = self.eval(u, v, 0.0, seed, style);
        }
        s.pos.y
    }

    /// The forest's trees, fitted to the constraints: none on a road, its
    /// verge or a pad (within `margin` metres of their edge) or on a bank
    /// too steep or bare to grow on; the rest stand on the bent ground.
    pub fn trees(&self, trees: Vec<forest_field::Tree>, margin: f32, seed: u32, style: &Style) -> Vec<forest_field::Tree> {
        if self.segments.is_empty() { return trees; }
        trees.into_iter().filter_map(|mut t| {
            if self.list(t.pos.x, t.pos.z).is_empty() { return Some(t); }
            if self.clearance(t.pos.x, t.pos.z) < margin { return None; }
            let s = self.eval(t.pos.x, t.pos.z, forest_field::CUT, seed, style);
            if !forest_field::grows(&s, style) { return None; }
            t.pos.y = s.pos.y - 0.2;
            Some(t)
        }).collect()
    }

    /// The roads for the terrain's pixels (terrain_surface.wgsl road_at):
    /// a fine grid of only the segments whose surface or shoulder reaches
    /// each 16 m cell, so a pixel tests a handful. As f32 words: a header
    /// (origin x, z, cell, size x, z, first cell word, first item word,
    /// segments), 8 words per segment (a.xz, b.xz, half a, b, along a, b),
    /// then size x * z + 1 cell offsets and the items (all exact integers).
    pub fn surface_words(&self) -> Vec<f32> {
        const C: f32 = 16.0;
        let roads: Vec<&Segment> = self.segments.iter().filter(|s| s.w[3] < 0.5).collect();
        if roads.is_empty() { return Vec::new(); }
        let reach = |s: &Segment| s.w[0].max(s.w[1]) + VERGE + SHOULDER;
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for s in &roads {
            let (a, b) = (Vec2::new(s.a[0], s.a[2]), Vec2::new(s.b[0], s.b[2]));
            lo = lo.min(a.min(b) - reach(s));
            hi = hi.max(a.max(b) + reach(s));
        }
        let origin = (lo / C).floor() * C;
        let size = [((hi.x - origin.x) / C).ceil() as usize + 1, ((hi.y - origin.y) / C).ceil() as usize + 1];
        let mut lists = vec![Vec::<u32>::new(); size[0] * size[1]];
        let r = C * std::f32::consts::FRAC_1_SQRT_2;
        for (k, s) in roads.iter().enumerate() {
            let (a, b) = (Vec2::new(s.a[0], s.a[2]), Vec2::new(s.b[0], s.b[2]));
            let (l, h) = (a.min(b) - reach(s) - origin, a.max(b) + reach(s) - origin);
            for j in (l.y / C).floor().max(0.0) as usize..=((h.y / C).floor() as usize).min(size[1] - 1) {
                for i in (l.x / C).floor().max(0.0) as usize..=((h.x / C).floor() as usize).min(size[0] - 1) {
                    let centre = origin + Vec2::new(i as f32 + 0.5, j as f32 + 0.5) * C;
                    if closest(s, centre).0 <= reach(s) + r { lists[j * size[0] + i].push(k as u32); }
                }
            }
        }
        let first_cell = 8 + roads.len() * 8;
        let first_item = first_cell + lists.len() + 1;
        let mut w = vec![origin.x, origin.y, C, size[0] as f32, size[1] as f32, first_cell as f32, first_item as f32, roads.len() as f32];
        for s in &roads { w.extend_from_slice(&[s.a[0], s.a[2], s.b[0], s.b[2], s.w[0], s.w[1], s.a[3], s.b[3]]); }
        let mut at = 0u32;
        for l in &lists { w.push(at as f32); at += l.len() as u32; }
        w.push(at as f32);
        for l in &lists { w.extend(l.iter().map(|&k| k as f32)); }
        assert!(w.len() < 1 << 24, "road surface grid too large");
        w
    }

    /// The number of tracks baked in.
    pub fn tracks(&self) -> usize { self.tracks }
}

/// The WGSL of the constraints, its three storage bindings at `group`,
/// `first`, `first + 1` and `first + 2`. Include after terrain_field.wgsl.
pub fn wgsl(group: u32, first: u32) -> String {
    WGSL.replace("WC_GROUP", &group.to_string()).replace("WC_HEADER", &first.to_string())
        .replace("WC_SEGMENTS", &(first + 1).to_string()).replace("WC_INDEX", &(first + 2).to_string())
}

/// The constraints on the GPU: header, segments, cells and items.
pub struct GpuWorld {
    pub header: wgpu::Buffer,
    pub segments: wgpu::Buffer,
    pub index: wgpu::Buffer,
}

impl GpuWorld {
    pub fn new(gpu: &crate::Gpu, world: &World) -> Self {
        use wgpu::util::DeviceExt;
        let make = |label: &str, bytes: &[u8]| gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label), contents: bytes, usage: wgpu::BufferUsages::STORAGE });
        let segments: Vec<Segment> = if world.segments.is_empty() { vec![Segment::default()] } else { world.segments.clone() };
        let mut index = world.cells.clone();
        index.extend_from_slice(&world.items);
        index.push(0);
        GpuWorld {
            header: make("world constraints header", bytemuck::bytes_of(&world.header)),
            segments: make("world constraints segments", bytemuck::cast_slice(&segments)),
            index: make("world constraints index", bytemuck::cast_slice(&index)),
        }
    }

    /// Bind group layout entries for `wgsl(_, first)`.
    pub fn layout_entries(first: u32, visibility: wgpu::ShaderStages) -> [wgpu::BindGroupLayoutEntry; 3] {
        let e = |binding| wgpu::BindGroupLayoutEntry { binding, visibility, count: None,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None } };
        [e(first), e(first + 1), e(first + 2)]
    }

    pub fn entries(&self, first: u32) -> [wgpu::BindGroupEntry<'_>; 3] {
        [wgpu::BindGroupEntry { binding: first, resource: self.header.as_entire_binding() },
         wgpu::BindGroupEntry { binding: first + 1, resource: self.segments.as_entire_binding() },
         wgpu::BindGroupEntry { binding: first + 2, resource: self.index.as_entire_binding() }]
    }
}

/// An arm's bank `u` metres past its edge: how far the land may stand
/// above or below the road there, and how steep that bank is.
pub fn bank(u: f32) -> (f32, f32) {
    let u = u.max(0.0);
    let r = u / MAX_REACH;
    let r5 = r * r * r * r * r;
    (u / BANK + OPEN * r5 * r, 1.0 / BANK + 6.0 * OPEN * r5 / MAX_REACH)
}

fn arm_pull(y: f32, u: f32, core: f32, near: Near) -> Pull {
    let (band, slope) = bank(u);
    Pull { y, band, slope, core, near }
}

fn smax(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 { return a.max(b); }
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.max(b) + h * h * k * 0.25
}

fn smin(a: f32, b: f32, k: f32) -> f32 { -smax(-a, -b, k) }

/// Distance from p to segment s and the parameter of its nearest point.
fn closest(s: &Segment, p: Vec2) -> (f32, f32) {
    let (a, b) = (Vec2::new(s.a[0], s.a[2]), Vec2::new(s.b[0], s.b[2]));
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    ((a + ab * t).distance(p), t)
}

fn segment_near(s: &Segment, p: Vec2) -> Near {
    let (d, t) = closest(s, p);
    let (a, b) = (Vec2::new(s.a[0], s.a[2]), Vec2::new(s.b[0], s.b[2]));
    let ab = b - a;
    // Right of travel in x-z (x right, z forward): (dz, -dx) ... sign of cross.
    let cross = ab.x * (p.y - a.y) - ab.y * (p.x - a.x);
    Near {
        distance: d,
        side: if cross > 0.0 { -d } else { d },
        along: s.a[3] + (s.b[3] - s.a[3]) * t,
        track: s.w[2] as u32,
        half: s.w[0] + (s.w[1] - s.w[0]) * t,
        height: s.a[1] + (s.b[1] - s.a[1]) * t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(seed: u32, style: &Style) -> (Track, World) { super::tests_ring(seed, style) }
    #[test]
    fn the_road_is_exactly_where_the_track_says() {
        let style = Style::default();
        let (track, world) = ring(4, &style);
        let along = track.along();
        for i in (0..track.points.len()).step_by(7) {
            let p = track.points[i];
            let s = world.eval(p.x, p.z, 0.0, 4, &style);
            assert!((s.pos.y - p.y).abs() < 1e-3, "road at {i}: {} vs {}", s.pos.y, p.y);
            assert_eq!(s.water, 0.0);
            assert!(s.scree > 0.99 && s.rock == 0.0, "road ground {s:?}");
            let n = world.near(p.x, p.z).unwrap();
            assert!(n.distance < 1e-3 && (n.along - along[i]).abs() < 0.05);
        }
        // Grade limit and smoothing held.
        for w in track.points.windows(2) {
            assert!((w[1].y - w[0].y).abs() <= 0.1 * w[0].xz().distance(w[1].xz()) + 1e-3);
        }
    }

    #[test]
    fn the_land_meets_the_road_without_a_step() {
        let style = Style::default();
        let (track, world) = ring(9, &style);
        let mut worst = 0.0f32;
        for i in (0..track.points.len()).step_by(11) {
            let p = track.points[i];
            let q = track.points[(i + 1) % track.points.len()];
            let dir = (q - p).xz().normalize();
            let right = Vec2::new(-dir.y, dir.x);
            let mut prev = world.eval(p.x, p.z, 0.0, 9, &style).pos.y;
            let mut off = 0.25f32;
            while off < 130.0 {
                let o = p.xz() + right * off;
                let y = world.eval(o.x, o.y, 0.0, 9, &style).pos.y;
                // A bank is at most as steep as its run allows, beyond what
                // the land itself does there (alpine land has its own cliffs).
                let o0 = o - right * 0.25;
                let land = (terrain_field::eval(o.x, o.y, 0.0, 9, &style).pos.y - terrain_field::eval(o0.x, o0.y, 0.0, 9, &style).pos.y).abs() / 0.25;
                // No steeper than the bank allows there. Inside a bend the
                // nearest road point slides along the climbing road, which
                // adds up to about a metre per metre; a seam would add far more.
                let step = (y - prev).abs() / 0.25 - land - bank(off - track.half[i] - VERGE).1;
                worst = worst.max(step);
                prev = y;
                off += 0.25;
            }
            // Past the reach the land is untouched.
            let far = p.xz() + right * (track.half[i] + VERGE + MAX_REACH + 0.5);
            if world.near(far.x, far.y).is_none_or(|n| n.distance > track.half[i] + VERGE + MAX_REACH) {
                assert_eq!(world.eval(far.x, far.y, 0.0, 9, &style), terrain_field::eval(far.x, far.y, 0.0, 9, &style));
            }
        }
        assert!(worst < 1.0, "bank {worst} m per m steeper than allowed");
    }

    #[test]
    fn the_grid_finds_what_brute_force_finds() {
        let style = Style::default();
        let (_, world) = ring(4, &style);
        for k in 0..4000 {
            let x = (k as f32 * 37.31) % 1400.0 - 550.0;
            let z = (k as f32 * 91.73) % 1400.0 - 760.0;
            let p = Vec2::new(x, z);
            let brute = world.segments.iter().filter(|s| s.w[3] < 0.5).map(|s| segment_near(s, p)).min_by(|a, b| a.distance.total_cmp(&b.distance)).unwrap();
            let reach = brute.half + VERGE + MAX_REACH;
            match world.near(x, z) {
                Some(n) => assert!(n.distance <= brute.distance + 1e-4 || brute.distance > reach),
                None => assert!(brute.distance > reach, "missed a road {} m away at {p}", brute.distance),
            }
        }
        // Fixed cost per sample, whatever the track's length.
        assert!(world.busiest_cell() < 120, "busiest cell {}", world.busiest_cell());
    }

    #[test]
    fn a_pad_is_level_and_nothing_changes_without_constraints() {
        let style = Style::default();
        let (_, world) = ring(4, &style);
        let h = world.eval(150.0, -60.0, 0.0, 4, &style).pos.y;
        for (dx, dz) in [(10.0, 0.0), (0.0, -18.0), (-12.0, 12.0)] {
            assert!((world.eval(150.0 + dx, -60.0 + dz, 0.0, 4, &style).pos.y - h).abs() < 1e-3);
        }
        let none = World::empty();
        for k in 0..200 {
            let (x, z) = (k as f32 * 13.7, k as f32 * -7.9);
            assert_eq!(none.eval(x, z, 1.0, 4, &style), terrain_field::eval(x, z, 1.0, 4, &style));
        }
    }

    #[test]
    fn no_tree_grows_on_the_road() {
        let style = Style::default();
        let (track, world) = ring(4, &style);
        let all = forest_field::trees_in_disc(Vec2::new(150.0, -60.0), 700.0, 4, &style, 0.8);
        let kept = world.trees(all.clone(), 2.0, 4, &style);
        assert!(kept.len() > 100 && kept.len() < all.len(), "{} of {}", kept.len(), all.len());
        for t in &kept {
            assert!(world.clearance(t.pos.x, t.pos.z) >= 2.0);
            assert!((t.pos.y + 0.2 - world.eval(t.pos.x, t.pos.z, forest_field::CUT, 4, &style).pos.y).abs() < 1e-3);
        }
        let _ = track;
    }

    #[test]
    fn water_keeps_its_level_against_a_bank() {
        let style = Style::default();
        let (_, world) = ring(4, &style);
        let (mut wet, mut checked) = (0, 0);
        for k in 0..40000 {
            let (x, z) = ((k as f32 * 7.31) % 1400.0 - 550.0, (k as f32 * 3.173) % 1400.0 - 760.0);
            let n = terrain_field::eval(x, z, 0.0, 4, &style);
            let b = world.eval(x, z, 0.0, 4, &style);
            assert!(b.water <= n.water, "deeper water at {x} {z}");
            if b.water > 0.0 && b.water < n.water {
                // A raised bed: the water stays at its level.
                checked += 1;
                assert!((b.pos.y - n.pos.y).abs() < 1e-3, "water level moved at {x} {z}: {} vs {}", b.pos.y, n.pos.y);
            }
            if b.water > 0.0 { wet += 1; }
        }
        let _ = (wet, checked);
    }

    /// CPU and GPU bend the land the same way, to float rounding.
    #[test]
    fn gpu_matches_cpu() {
        let Ok(gpu) = crate::Gpu::new() else { return };
        use wgpu::util::DeviceExt;
        let style = Style::default();
        let seed = 4u32;
        let (_, world) = ring(seed, &style);
        let points: Vec<[f32; 2]> = (0..4096).map(|k| [(k as f32 * 37.31) % 1300.0 - 500.0, (k as f32 * 91.73) % 1300.0 - 710.0]).collect();
        let shader = format!("{}\n{}\n{}", terrain_field::WGSL, wgsl(0, 3), r#"
struct Params { seed: u32, pad0: u32, pad1: u32, pad2: u32, style: TerrainStyle };
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> points: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read_write> out: array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= arrayLength(&points) { return; }
    let p = points[gid.x];
    let s = terrain_eval_world(p.x, p.y, 0.5, params.seed, params.style);
    let n = wc_near(p.x, p.y);
    out[gid.x * 2u] = vec4<f32>(s.pos.y, s.scree, s.up, s.water);
    out[gid.x * 2u + 1u] = vec4<f32>(n.distance, n.along, wc_clearance(p.x, p.y), f32(n.track));
}"#);
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("constraints parity"), source: wgpu::ShaderSource::Wgsl(shader.into()) });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: None, module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
        let mut params = bytemuck::bytes_of(&[seed, 0, 0, 0]).to_vec();
        params.extend_from_slice(bytemuck::bytes_of(&style));
        let params = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &params, usage: wgpu::BufferUsages::UNIFORM });
        let input = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&points), usage: wgpu::BufferUsages::STORAGE });
        let size = (points.len() * 32) as u64;
        let output = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let gw = GpuWorld::new(&gpu, &world);
        let mut entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: input.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: output.as_entire_binding() }];
        entries.extend(gw.entries(3));
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(0), entries: &entries });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((points.len() as u32).div_ceil(64), 1, 1);
        }
        enc.copy_buffer_to_buffer(&output, 0, &read, 0, size);
        gpu.queue.submit([enc.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let got: Vec<[f32; 4]> = bytemuck::cast_slice(&read.slice(..).get_mapped_range().unwrap()).to_vec();
        let (mut bent, mut agree) = (0, 0);
        for (k, p) in points.iter().enumerate() {
            let s = world.eval(p[0], p[1], 0.5, seed, &style);
            let (g, n) = (got[k * 2], got[k * 2 + 1]);
            if s != terrain_field::eval(p[0], p[1], 0.5, seed, &style) { bent += 1; }
            // terrain_eval differs from eval in the last bits.
            if (g[0] - s.pos.y).abs() < 0.02 && (g[1] - s.scree).abs() < 1e-3 && (g[2] - s.up).abs() < 1e-3 && (g[3] - s.water).abs() < 0.02 { agree += 1; }
            match world.near(p[0], p[1]) {
                Some(c) => assert!((n[0] - c.distance).abs() < 1e-3 && (n[1] - c.along).abs() < 1e-2 && n[3] == c.track as f32, "near {p:?}: {n:?} vs {c:?}"),
                None => assert!(n[0] > 1e38),
            }
            let c = world.clearance(p[0], p[1]);
            assert!((n[2] - c).abs() < 1e-3 || (c == f32::MAX && n[2] > 1e38));
        }
        assert!(bent > 300, "{bent} bent samples compared");
        assert!(agree * 1000 >= points.len() * 998, "{agree} of {} samples agree", points.len());
    }
}
#[cfg(test)]
pub(crate) fn tests_ring(seed: u32, style: &Style) -> (Track, World) {
    let control: Vec<Vec3> = (0..10).map(|k| {
        let a = k as f32 / 10.0 * std::f32::consts::TAU;
        let r = 420.0 + 90.0 * (3.0 * a).sin();
        Vec3::new(r * a.cos() + 150.0, 0.0, r * a.sin() - 60.0)
    }).collect();
    let mut track = Track::spline(&control, 4.0, true);
    track.fit_heights(|x, z| terrain_field::height(x, z, seed, style), 0.1, 80.0);
    let world = Constraints { tracks: vec![track.clone()], pads: vec![Pad { centre: Vec2::new(150.0, -60.0), radius: 25.0, feather: 15.0, height: None }] }.bake(seed, style);
    (track, world)
}
