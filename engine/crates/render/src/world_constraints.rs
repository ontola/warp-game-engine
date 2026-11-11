//! Constraints the world is generated around: what an artist or a game
//! fixes, while everything else stays a function of position and seed.
//! A race game's track is the first one: its road is exactly where and how
//! high the game says, and the terrain bends to it with cuttings and
//! embankments, rivers stop at it, and no tree grows on it. Pads are round
//! flat places (a service park, a village square, a landmark's footing);
//! footprints are flat plots of any outline (a building's footing, a yard).
//! Zones (`crate::zones`) say what each place is for; they bend nothing but
//! are fields generators read.
//! Regions (`crate::regions`) say which kind of land lies where: the style
//! the terrain takes at each point, blended between up to four.
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
//! let world = Constraints { tracks: vec![track], ..Default::default() }.bake(seed, &style);
//! let s = world.eval(x, z, cut, seed, &style); // terrain_field::eval with the constraints
//! ```
use crate::forest_field;
use crate::regions::Regions;
use crate::zones::{Zone, Zones, Zoning};
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
/// How fast a deep cutting or embankment steepens past the first BANK run:
/// at MAX_REACH it stands at about 1.5 (a rock cut) and holds about 100 m
/// of height, so only cuts deeper than that ever reach the OPEN wall.
pub const STEEPEN: f32 = 0.0047;
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

    /// Lay the road on `ground`: as close to the land as grades of at most
    /// `max_grade` allow, then smoothed over `window` metres. The land bends
    /// where the road cannot follow it.
    ///
    /// The road is the midpoint of the highest and the lowest line that
    /// keep to the grade and stay on one side of the land, which halves the
    /// deepest cut or fill a one-sided grade clamp leaves. Smoothing keeps
    /// the grade limit (an average of lines that keep it keeps it too).
    pub fn fit_heights(&mut self, ground: impl Fn(f32, f32) -> f32, max_grade: f32, window: f32) {
        let n = self.points.len();
        if n == 0 { return; }
        let raw: Vec<f32> = self.points.iter().map(|p| ground(p.x, p.z)).collect();
        let along = self.along();
        let total = *along.last().unwrap() + if self.closed { self.points[n - 1].xz().distance(self.points[0].xz()) } else { 0.0 };
        let gap = |i: usize, j: usize| {
            let d = (along[j] - along[i]).abs();
            if self.closed { d.min(total - d) } else { d }
        };
        let mid: Vec<f32> = (0..n).map(|i| {
            let (mut under, mut over) = (f32::MAX, f32::MIN);
            for (j, h) in raw.iter().enumerate() {
                let run = gap(i, j) * max_grade;
                under = under.min(h + run);
                over = over.max(h - run);
            }
            0.5 * (under + over)
        }).collect();
        // Box filter by distance along the road (wrapping on a loop).
        let y: Vec<f32> = (0..n).map(|i| {
            let (mut sum, mut weight) = (0.0f32, 0.0f32);
            for (j, h) in mid.iter().enumerate() {
                let d = gap(i, j);
                if d <= window * 0.5 {
                    let w = 1.0 - d / (window * 0.5 + 1e-3);
                    sum += h * w;
                    weight += w;
                }
            }
            sum / weight
        }).collect();
        // Uneven point spacing lets the filter overshoot the grade by a hair:
        // clamp it, forward and back (twice round a loop).
        let mut y = y;
        for _ in 0..if self.closed { 2 } else { 1 } {
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

/// A flat plot of any outline (a building's footing, a yard, a square):
/// inside the outline the ground is level at `height`, outside it meets
/// the land again like a pad's rim. A pad is the round case.
#[derive(Clone, Debug)]
pub struct Footprint {
    /// Corners in x-z, either winding; at least three, no repeats.
    pub outline: Vec<Vec2>,
    pub feather: f32,
    /// None: the land's own height at the outline's centre.
    pub height: Option<f32>,
    /// 0: the plot keeps the land's surface (grass, sand); 1: bare gravel
    /// like the road (a building's footing, a yard).
    pub bare: f32,
}

impl Footprint {
    /// The plot as bare ground (0..1).
    pub fn bare(mut self, bare: f32) -> Self { self.bare = bare; self }

    /// A rectangle `size` across, centred on `centre`, turned by `yaw`
    /// radians.
    pub fn rect(centre: Vec2, size: Vec2, yaw: f32, feather: f32, height: Option<f32>) -> Self {
        let (s, c) = yaw.sin_cos();
        let h = size * 0.5;
        let outline = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter()
            .map(|&(u, v)| { let q = Vec2::new(u * h.x, v * h.y); centre + Vec2::new(q.x * c - q.y * s, q.x * s + q.y * c) }).collect();
        Footprint { outline, feather, height, bare: 0.0 }
    }
}

/// Everything the world is generated around.
#[derive(Clone, Debug, Default)]
pub struct Constraints {
    pub tracks: Vec<Track>,
    pub pads: Vec<Pad>,
    /// Flat plots the land is levelled to (hard intent).
    pub footprints: Vec<Footprint>,
    /// Which land lies where; None: the one style everywhere.
    pub regions: Option<Regions>,
    /// What each place is for (soft intent: crate::zones).
    pub zones: Vec<Zone>,
}

/// One straight piece of a road, as the GPU reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Segment {
    /// Start (x, y, z) and its distance along the track.
    pub a: [f32; 4],
    /// End (x, y, z) and its distance along the track.
    pub b: [f32; 4],
    /// Half width at a and b, track index, and 0. 1 for a pad: a.xyz its
    /// centre and height, b.x its radius, b.y its feather. 2 for a
    /// footprint: a.xyz its centre and height, b.x its first corner in the
    /// world's outline corners, b.y how many, b.z its feather, b.w the
    /// farthest corner's distance from the centre, w.x how bare it is.
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
    /// The regions' grid (crate::regions): node (0, 0), spacing, where
    /// their words start in the index buffer, nodes across and how many
    /// styles (0: no regions, the style passed in holds everywhere).
    pub region_origin: [f32; 2],
    pub region_cell: f32,
    pub region_at: u32,
    pub region_size: [u32; 2],
    pub region_styles: u32,
    /// Where the footprints' corners start in the index buffer (x, z bits
    /// a corner).
    pub shape_at: u32,
    /// The zones' grid (crate::zones): cell (0, 0), spacing, how many
    /// zones (0: none), cells across and where their words start.
    pub zone_origin: [f32; 2],
    pub zone_cell: f32,
    pub zone_count: u32,
    pub zone_size: [u32; 2],
    pub zone_at: u32,
    pub _pad3: u32,
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
    /// Which land lies where (its words follow `items` on the GPU).
    pub regions: Option<Regions>,
    /// The footprints' corners (after the regions' words on the GPU).
    pub shapes: Vec<Vec2>,
    /// What each place is for (after the corners on the GPU).
    pub zones: Option<Zones>,
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
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.pads.is_empty() && self.footprints.is_empty() && self.regions.is_none() && self.zones.is_empty()
    }

    /// Bake for lookup. Pads without a height take the land's at their
    /// centre from `seed` and `style` (the regions' style there, if any).
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
            let st = self.regions.as_ref().map_or(*style, |r| r.style_at(pad.centre.x, pad.centre.y));
            let h = pad.height.unwrap_or_else(|| terrain_field::height(pad.centre.x, pad.centre.y, seed, &st));
            segments.push(Segment { a: [pad.centre.x, h, pad.centre.y, 0.0], b: [pad.radius, pad.feather.max(MIN_REACH), 0.0, 0.0], w: [0.0, 0.0, 0.0, 1.0] });
        }
        let mut shapes = Vec::new();
        for f in &self.footprints {
            let outline = clean_outline(&f.outline);
            assert!(outline.len() >= 3, "a footprint needs three distinct corners");
            let centre = outline.iter().copied().sum::<Vec2>() / outline.len() as f32;
            let bound = outline.iter().map(|c| c.distance(centre)).fold(0.0, f32::max);
            let st = self.regions.as_ref().map_or(*style, |r| r.style_at(centre.x, centre.y));
            let h = f.height.unwrap_or_else(|| terrain_field::height(centre.x, centre.y, seed, &st));
            segments.push(Segment { a: [centre.x, h, centre.y, 0.0], b: [shapes.len() as f32, outline.len() as f32, f.feather.max(MIN_REACH), bound], w: [f.bare.clamp(0.0, 1.0), 0.0, 0.0, 2.0] });
            shapes.extend(outline);
        }
        assert!(shapes.len() < 1 << 23, "too many footprint corners");
        let mut world = World::from_segments(segments, shapes, self.tracks.len());
        let mut at = world.cells.len() + world.items.len();
        if let Some(r) = &self.regions {
            let h = &mut world.header;
            h.region_origin = r.origin.into();
            h.region_cell = r.cell;
            h.region_at = (world.cells.len() + world.items.len()) as u32;
            h.region_size = r.size;
            h.region_styles = r.styles.len() as u32;
            at += r.words().len();
            world.regions = Some(r.clone());
        }
        world.header.shape_at = at as u32;
        at += 2 * world.shapes.len();
        if !self.zones.is_empty() {
            let z = Zones::bake(&self.zones);
            let h = &mut world.header;
            h.zone_origin = z.origin.into();
            h.zone_cell = z.cell;
            h.zone_count = z.count();
            h.zone_size = z.size;
            h.zone_at = at as u32;
            world.zones = Some(z);
        }
        world
    }
}

impl World {
    /// No constraints: every lookup returns the land unchanged.
    pub fn empty() -> Self { World::from_segments(Vec::new(), Vec::new(), 0) }

    fn reach(s: &Segment) -> (Vec2, Vec2) {
        if s.w[3] > 1.5 {
            let c = Vec2::new(s.a[0], s.a[2]);
            let r = s.b[3] + s.b[2].max(MAX_REACH);
            return (c - r, c + r);
        }
        if s.w[3] > 0.5 {
            let c = Vec2::new(s.a[0], s.a[2]);
            let r = s.b[0] + (s.b[1]).max(MAX_REACH);
            return (c - r, c + r);
        }
        let r = s.w[0].max(s.w[1]) + VERGE + MAX_REACH;
        let (a, b) = (Vec2::new(s.a[0], s.a[2]), Vec2::new(s.b[0], s.b[2]));
        (a.min(b) - r, a.max(b) + r)
    }

    fn from_segments(segments: Vec<Segment>, shapes: Vec<Vec2>, tracks: usize) -> Self {
        if segments.is_empty() {
            return World { header: Header { size: [1, 1], cell: CELL, ..Default::default() }, segments, cells: vec![0, 0], items: Vec::new(), regions: None, shapes, zones: None, tracks };
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
                    if reaches(s, &shapes, origin + Vec2::new(i as f32, j as f32) * CELL) { lists[(j * size[0] + i) as usize].push(k as u32); }
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
        let header = Header { origin: origin.into(), cell: CELL, arms: MAX_ARMS as u32, size, segments: segments.len() as u32, ..Default::default() };
        World { header, segments, cells, items, regions: None, shapes, zones: None, tracks }
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

    /// Metres from (x, z) to the nearest constrained surface (road edge,
    /// pad rim or footprint outline); negative inside; f32::MAX far from all.
    pub fn clearance(&self, x: f32, z: f32) -> f32 {
        let p = Vec2::new(x, z);
        let mut c = f32::MAX;
        for &k in self.list(x, z) {
            let s = &self.segments[k as usize];
            c = c.min(if s.w[3] > 0.5 { rim(s, &self.shapes, p) } else { let n = segment_near(s, p); n.distance - n.half });
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
                let d = rim(s, &self.shapes, p);
                let near = Near { distance: d, side: 0.0, along: 0.0, track: u32::MAX, half: 0.0, height: s.a[1] };
                // A bare footprint is gravel inside, fading out over the verge.
                let core = if s.w[3] > 1.5 { s.w[0] * smooth(VERGE, 0.0, d) } else { 0.0 };
                arm_pull(s.a[1], d, core, near)
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

    /// The share (0..1) of zone tag `tag` at (x, z) (crate::zones); 0
    /// where no zone of that tag lies or the world has none.
    pub fn zone(&self, tag: &str, x: f32, z: f32) -> f32 {
        self.zones.as_ref().and_then(|zs| zs.tag(tag).map(|t| zs.weight(t, x, z))).unwrap_or(0.0)
    }

    /// The zones' soft intent at (x, z) (crate::zones::Zoning).
    pub fn zoning(&self, x: f32, z: f32) -> Zoning { self.zones.as_ref().map_or(Zoning::default(), |zs| zs.zoning(x, z)) }

    /// The terrain style at (x, z): the regions' blend there, or `style`
    /// when the world has no regions.
    pub fn style_at(&self, x: f32, z: f32, style: &Style) -> Style {
        self.regions.as_ref().map_or(*style, |r| r.style_at(x, z))
    }

    /// terrain_field::eval with the constraints: the land of the region
    /// it lies in, bent to every road and pad within reach.
    pub fn eval(&self, x: f32, z: f32, cut: f32, seed: u32, style: &Style) -> Sample {
        let s = terrain_field::eval(x, z, cut, seed, &self.style_at(x, z, style));
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
        s.film *= 1.0 - core;
        s.pos.y = y + s.water;
        let slope = if y > land + 1e-3 { lo_slope } else if y < land - 1e-3 { hi_slope } else { 0.0 };
        let bank_up = 1.0 / (1.0 + slope * slope).sqrt();
        s.up = (s.up + (bank_up - s.up) * smooth(0.0, 0.5, moved)).max(s.up * (1.0 - core) + core);
        s.wet *= 1.0 - core;
        s.snow *= 1.0 - 0.7 * core;
        s.moor *= 1.0 - core;
        s.rock *= 1.0 - core;
        // A steep cutting or embankment is bare rock, not stretched grass.
        s.rock = s.rock.max(smooth(0.9, 1.4, slope) * smooth(0.0, 0.5, moved) * (1.0 - core));
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
        if self.segments.is_empty() && self.regions.is_none() { return trees; }
        trees.into_iter().filter_map(|mut t| {
            if self.regions.is_none() && self.list(t.pos.x, t.pos.z).is_empty() { return Some(t); }
            if self.clearance(t.pos.x, t.pos.z) < margin { return None; }
            let s = self.eval(t.pos.x, t.pos.z, forest_field::CUT, seed, style);
            if !forest_field::grows(&s, &self.style_at(t.pos.x, t.pos.z, style)) { return None; }
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
        if let Some(r) = &world.regions {
            debug_assert_eq!(index.len() as u32, world.header.region_at);
            index.extend(r.words());
        }
        debug_assert_eq!(index.len() as u32, world.header.shape_at);
        for c in &world.shapes { index.extend_from_slice(&[c.x.to_bits(), c.y.to_bits()]); }
        if let Some(z) = &world.zones {
            debug_assert_eq!(index.len() as u32, world.header.zone_at);
            index.extend(z.words());
        }
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
    (u / BANK + STEEPEN * u * u + OPEN * r5 * r, 1.0 / BANK + 2.0 * STEEPEN * u + 6.0 * OPEN * r5 / MAX_REACH)
}

fn arm_pull(y: f32, u: f32, core: f32, near: Near) -> Pull {
    let (band, slope) = bank(u);
    Pull { y, band, slope, core, near }
}

/// Whether segment `s` can pull any point of the cell at `corner`.
fn reaches(s: &Segment, shapes: &[Vec2], corner: Vec2) -> bool {
    let centre = corner + Vec2::splat(CELL * 0.5);
    let r = CELL * std::f32::consts::FRAC_1_SQRT_2;
    if s.w[3] > 1.5 { return rim(s, shapes, centre) <= s.b[2].max(MAX_REACH) + r; }
    if s.w[3] > 0.5 {
        return centre.distance(Vec2::new(s.a[0], s.a[2])) <= s.b[0] + s.b[1].max(MAX_REACH) + r;
    }
    let (d, ..) = closest(s, centre);
    d <= s.w[0].max(s.w[1]) + VERGE + MAX_REACH + r
}

/// Metres from p to a pad's rim or a footprint's outline; negative inside.
fn rim(s: &Segment, shapes: &[Vec2], p: Vec2) -> f32 {
    if s.w[3] > 1.5 {
        let first = s.b[0] as usize;
        outline_distance(&shapes[first..first + s.b[1] as usize], p)
    } else {
        p.distance(Vec2::new(s.a[0], s.a[2])) - s.b[0]
    }
}

/// Signed distance from p to a closed outline: negative inside, exact
/// everywhere, either winding (wc_outline in the WGSL, line for line).
pub fn outline_distance(corners: &[Vec2], p: Vec2) -> f32 {
    let n = corners.len();
    let mut d = (p - corners[0]).length_squared();
    let mut sign = 1.0f32;
    let mut j = n - 1;
    for i in 0..n {
        let (vi, vj) = (corners[i], corners[j]);
        let e = vj - vi;
        let w = p - vi;
        let b = w - e * (w.dot(e) / e.dot(e)).clamp(0.0, 1.0);
        d = d.min(b.dot(b));
        let (c0, c1, c2) = (p.y >= vi.y, p.y < vj.y, e.x * w.y > e.y * w.x);
        if (c0 && c1 && c2) || (!c0 && !c1 && !c2) { sign = -sign; }
        j = i;
    }
    sign * d.sqrt()
}

/// An outline without repeated corners (the distance divides by each
/// edge's length), the closing repeat of the first corner dropped too.
pub(crate) fn clean_outline(corners: &[Vec2]) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = Vec::with_capacity(corners.len());
    for &c in corners {
        if out.last().is_none_or(|l| l.distance_squared(c) > 1e-6) { out.push(c); }
    }
    while out.len() > 1 && out[0].distance_squared(*out.last().unwrap()) <= 1e-6 { out.pop(); }
    out
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

    /// Regions give each point its land; roads still bend it, and a
    /// world of regions alone (no roads) takes the regions' style.
    #[test]
    fn regions_shape_the_land_under_the_roads() {
        let (a, b) = (Style::alpine(), Style::lowland());
        let regions = crate::regions::Regions::split([a, b], Vec2::new(150.0, -60.0), 4000.0, Vec2::X, 600.0, 200.0, 4);
        let only = Constraints { regions: Some(regions.clone()), ..Default::default() }.bake(4, &a);
        assert!(!Constraints { regions: Some(regions.clone()), ..Default::default() }.is_empty());
        for (x, z) in [(-1900.0f32, 10.0f32), (2200.0, -300.0), (150.0, 40.0)] {
            assert_eq!(only.eval(x, z, 0.0, 4, &a), terrain_field::eval(x, z, 0.0, 4, &regions.style_at(x, z)));
        }
        let (track, _) = ring(4, &a);
        let world = Constraints { tracks: vec![track.clone()], regions: Some(regions), ..Default::default() }.bake(4, &a);
        let p = track.points[5];
        assert!((world.eval(p.x, p.z, 0.0, 4, &a).pos.y - p.y).abs() < 1e-3);
        assert_eq!(world.header.region_at as usize, world.cells.len() + world.items.len());
    }

    /// A footprint is level inside its outline, whatever its shape, meets
    /// the land beyond it, and keeps trees off; zones bend nothing.
    #[test]
    fn a_footprint_is_level_inside_its_outline() {
        let style = Style::alpine();
        let l = vec![Vec2::new(0.0, 0.0), Vec2::new(60.0, 0.0), Vec2::new(60.0, 20.0), Vec2::new(20.0, 20.0), Vec2::new(20.0, 50.0), Vec2::new(0.0, 50.0)];
        let plot = Footprint::rect(Vec2::new(300.0, 120.0), Vec2::new(40.0, 24.0), 0.6, 8.0, Some(55.0));
        let zone = Zone::new("residential", vec![Vec2::new(-200.0, -200.0), Vec2::new(500.0, -200.0), Vec2::new(500.0, 400.0)], 50.0);
        let world = Constraints { footprints: vec![Footprint { outline: l, feather: 8.0, height: None, bare: 1.0 }, plot], zones: vec![zone], ..Default::default() }.bake(3, &style);
        let h = world.eval(10.0, 25.0, 0.0, 3, &style).pos.y;
        for (x, z) in [(1.0, 1.0), (59.0, 19.0), (19.0, 49.0), (40.0, 10.0)] {
            let s = world.eval(x, z, 0.0, 3, &style);
            assert!((s.pos.y - h).abs() < 1e-3, "L not level at {x} {z}");
            assert!(s.scree > 0.99 && s.water == 0.0, "the bare L is gravel at {x} {z}");
            assert!(world.clearance(x, z) < 0.0);
        }
        // The notch of the L is outside it: not forced level.
        assert!(world.clearance(40.0, 40.0) > 0.0);
        assert!((world.eval(300.0, 120.0, 0.0, 3, &style).pos.y - 55.0).abs() < 1e-3);
        // Far away the land is its own.
        assert_eq!(world.eval(900.0, 900.0, 0.0, 3, &style), terrain_field::eval(900.0, 900.0, 0.0, 3, &style));
        assert_eq!(world.zone("residential", 250.0, 0.0), 1.0);
        assert_eq!(world.zone("industrial", 250.0, 0.0), 0.0);
        let zoned = Constraints { zones: world.zones.as_ref().map(|_| vec![Zone::new("park", vec![Vec2::ZERO, Vec2::new(90.0, 0.0), Vec2::new(0.0, 90.0)], 10.0)]).unwrap(), ..Default::default() }.bake(3, &style);
        assert_eq!(zoned.eval(20.0, 20.0, 0.0, 3, &style), terrain_field::eval(20.0, 20.0, 0.0, 3, &style));
        assert_eq!(zoned.zone("park", 20.0, 20.0), 1.0);
    }

    /// The constraints' WGSL (regions included) parses and validates, and
    /// the procedural terrain shader that includes it does too: the check
    /// that runs where there is no GPU.
    #[test]
    fn wgsl_validates() {
        let entry = r#"
@group(0) @binding(0) var<storage, read_write> out: array<vec4<f32>>;
@compute @workgroup_size(1) fn main() {
    let s = terrain_eval_world(1.0, 2.0, 0.5, 3u, TerrainStyle(0.5, 0.5, 1500.0, 1.0, 0.5, 0.5, 0.5, 1.0, 0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0));
    out[0] = vec4<f32>(s.pos, wc_clearance(1.0, 2.0) + wc_zone(1.0, 2.0, 0u) + wc_zoning(1.0, 2.0).y);
}"#;
        for source in [format!("{}\n{}\n{}", terrain_field::WGSL, wgsl(0, 3), entry),
            format!("{}\n{}\n{}", terrain_field::WGSL, wgsl(0, 4), include_str!("shaders/procedural_terrain.wgsl"))] {
            let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module)
                .unwrap_or_else(|e| panic!("{e:?}"));
        }
    }

    /// CPU and GPU bend the land the same way, to float rounding.
    #[test]
    fn gpu_matches_cpu() {
        let Ok(gpu) = crate::Gpu::new() else { eprintln!("no GPU: parity skipped"); return };
        for regions in [false, true] { gpu_matches_cpu_with(&gpu, regions); }
    }

    fn gpu_matches_cpu_with(gpu: &crate::Gpu, regions: bool) {
        use wgpu::util::DeviceExt;
        let style = Style::default();
        let seed = 4u32;
        let (track, world) = ring(seed, &style);
        let world = if regions {
            let r = crate::regions::Regions::split([Style::alpine(), Style::lowland()], Vec2::new(150.0, -60.0), 3000.0, Vec2::new(1.0, 0.4), 500.0, 150.0, seed);
            let zones = vec![
                Zone::new("industrial", vec![Vec2::new(-500.0, -700.0), Vec2::new(200.0, -700.0), Vec2::new(100.0, 0.0), Vec2::new(-400.0, 100.0)], 120.0).density(0.9).age(0.8),
                Zone::new("residential", vec![Vec2::new(100.0, 0.0), Vec2::new(800.0, -100.0), Vec2::new(700.0, 600.0), Vec2::new(-100.0, 590.0)], 60.0).wealth(0.7),
                Zone::new("industrial", vec![Vec2::new(300.0, 100.0), Vec2::new(420.0, 100.0), Vec2::new(360.0, 260.0)], 30.0)];
            let footprints = vec![
                Footprint::rect(Vec2::new(-120.0, -200.0), Vec2::new(60.0, 30.0), 0.4, 10.0, None).bare(0.7),
                Footprint { outline: vec![Vec2::new(250.0, 200.0), Vec2::new(330.0, 210.0), Vec2::new(320.0, 260.0), Vec2::new(290.0, 240.0), Vec2::new(260.0, 270.0)], feather: 8.0, height: Some(30.0), bare: 0.0 }];
            Constraints { tracks: vec![track], pads: vec![Pad { centre: Vec2::new(150.0, -60.0), radius: 25.0, feather: 15.0, height: None }], regions: Some(r), footprints, zones }.bake(seed, &style)
        } else { world };
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
    out[gid.x * 4u] = vec4<f32>(s.pos.y, s.scree, s.up, s.water);
    out[gid.x * 4u + 1u] = vec4<f32>(n.distance, n.along, wc_clearance(p.x, p.y), f32(n.track));
    out[gid.x * 4u + 2u] = wc_zoning(p.x, p.y);
    out[gid.x * 4u + 3u] = vec4<f32>(wc_zone(p.x, p.y, 0u), wc_zone(p.x, p.y, 1u), wc_zone(p.x, p.y, 7u), 0.0);
}"#);
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("constraints parity"), source: wgpu::ShaderSource::Wgsl(shader.into()) });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: None, module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
        let mut params = bytemuck::bytes_of(&[seed, 0, 0, 0]).to_vec();
        params.extend_from_slice(bytemuck::bytes_of(&style));
        let params = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &params, usage: wgpu::BufferUsages::UNIFORM });
        let input = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&points), usage: wgpu::BufferUsages::STORAGE });
        let size = (points.len() * 64) as u64;
        let output = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let gw = GpuWorld::new(gpu, &world);
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
        let (mut bent, mut agree, mut zoned) = (0, 0, 0usize);
        for (k, p) in points.iter().enumerate() {
            let s = world.eval(p[0], p[1], 0.5, seed, &style);
            let (g, n, zg, zt) = (got[k * 4], got[k * 4 + 1], got[k * 4 + 2], got[k * 4 + 3]);
            if s != terrain_field::eval(p[0], p[1], 0.5, seed, &world.style_at(p[0], p[1], &style)) { bent += 1; }
            // terrain_eval differs from eval in the last bits.
            if (g[0] - s.pos.y).abs() < 0.02 && (g[1] - s.scree).abs() < 1e-3 && (g[2] - s.up).abs() < 1e-3 && (g[3] - s.water).abs() < 0.02 { agree += 1; }
            // `along` is metres from the track start: allow f32 rounding that grows with it (Metal fma),
            // and a few cm where the nearest point sits at a joint between two polyline segments and
            // the GPU picks the other one (Metal: 254.753 vs 254.701 at [654.1, 213.6]).
            match world.near(p[0], p[1]) {
                Some(c) => assert!((n[0] - c.distance).abs() < 1e-3 && (n[1] - c.along).abs() < 0.1 + 1e-5 * c.along.abs() && n[3] == c.track as f32, "near {p:?}: {n:?} vs {c:?}"),
                None => assert!(n[0] > 1e38),
            }
            let c = world.clearance(p[0], p[1]);
            assert!((n[2] - c).abs() < 1e-3 || (c == f32::MAX && n[2] > 1e38));
            let z = world.zoning(p[0], p[1]);
            assert!((zg[0] - z.covered).abs() < 1e-4 && (zg[1] - z.density).abs() < 1e-4 && (zg[2] - z.wealth).abs() < 1e-4 && (zg[3] - z.age).abs() < 1e-4, "zoning {p:?}: {zg:?} vs {z:?}");
            if let Some(zs) = &world.zones {
                for (t, got) in [(0u32, zt[0]), (1, zt[1])] { assert!((got - zs.weight(t, p[0], p[1])).abs() < 1e-4, "zone {t} at {p:?}"); }
                if regions { zoned += usize::from(z.covered > 0.0 && z.covered < 1.0); }
            }
            assert_eq!(zt[2], 0.0);
        }
        if regions { assert!(zoned > 50, "{zoned} samples on zone edges"); }
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
    let world = Constraints { tracks: vec![track.clone()], pads: vec![Pad { centre: Vec2::new(150.0, -60.0), radius: 25.0, feather: 15.0, height: None }], ..Default::default() }.bake(seed, style);
    (track, world)
}
