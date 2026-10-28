//! Simple, deterministic collision for game prototypes: boxes rotated about
//! the vertical axis (most level geometry), an upright cylinder for
//! characters, rays and spheres, and a `move_and_slide` character mover.
//! A plain slice has no broadphase (fine for hundreds of boxes); [`BoxGrid`]
//! adds a uniform grid with removal for thousands (destructible levels).
//! Movers and rays work on either through [`Boxes`].
//!
//! Yaw convention: positive yaw turns +X towards -Z (right-handed, Y up;
//! the same as `glam::Quat::from_rotation_y`).
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

/// A box rotated about +Y.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct YawBox {
    pub center: Vec3,
    pub half: Vec3,
    pub yaw: f32,
}

impl YawBox {
    pub fn new(center: Vec3, size: Vec3, yaw: f32) -> Self { Self { center, half: size * 0.5, yaw } }

    fn to_local(&self, d: Vec2) -> Vec2 {
        let (s, c) = self.yaw.sin_cos();
        Vec2::new(c * d.x - s * d.y, s * d.x + c * d.y)
    }

    fn to_world(&self, d: Vec2) -> Vec2 {
        let (s, c) = self.yaw.sin_cos();
        Vec2::new(c * d.x + s * d.y, -s * d.x + c * d.y)
    }

    pub fn top(&self) -> f32 { self.center.y + self.half.y }

    /// Radius of the box's footprint circle, for cheap rejection.
    pub fn reach(&self) -> f32 { Vec2::new(self.half.x, self.half.z).length() }

    /// Push an upright cylinder (centre, radius, half height) out of this
    /// box along the shallower direction. None without overlap.
    pub fn push_out(&self, p: Vec3, radius: f32, half_height: f32) -> Option<Vec3> {
        let dy = p.y - self.center.y;
        let overlap_y = self.half.y + half_height - dy.abs();
        if overlap_y <= 0.0 { return None; }
        let local = self.to_local(Vec2::new(p.x - self.center.x, p.z - self.center.z));
        let h = Vec2::new(self.half.x, self.half.z);
        let delta = local - local.clamp(-h, h);
        let dist2 = delta.length_squared();
        let (n, depth) = if dist2 > 1e-12 {
            let dist = dist2.sqrt();
            if dist >= radius { return None; }
            (delta / dist, radius - dist)
        } else {
            let dx = h.x - local.x.abs();
            let dz = h.y - local.y.abs();
            if dx < dz { (Vec2::new(local.x.signum(), 0.0), dx + radius) } else { (Vec2::new(0.0, local.y.signum()), dz + radius) }
        };
        if overlap_y < depth {
            Some(Vec3::new(0.0, overlap_y * dy.signum(), 0.0))
        } else {
            let w = self.to_world(n) * depth;
            Some(Vec3::new(w.x, 0.0, w.y))
        }
    }

    /// Ray (unit `dir`) against the box: distance and outward surface normal.
    pub fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3)> {
        let o2 = self.to_local(Vec2::new(origin.x - self.center.x, origin.z - self.center.z));
        let d2 = self.to_local(Vec2::new(dir.x, dir.z));
        let o = [o2.x, origin.y - self.center.y, o2.y];
        let d = [d2.x, dir.y, d2.y];
        let h = [self.half.x, self.half.y, self.half.z];
        let (mut t0, mut t1, mut axis, mut sign) = (0.0f32, max, usize::MAX, 0.0f32);
        for k in 0..3 {
            if d[k].abs() < 1e-12 {
                if o[k].abs() > h[k] { return None; }
                continue;
            }
            let inv = 1.0 / d[k];
            let (mut a, mut b) = ((-h[k] - o[k]) * inv, (h[k] - o[k]) * inv);
            let mut s = -1.0;
            if a > b { std::mem::swap(&mut a, &mut b); s = 1.0; }
            if a > t0 { t0 = a; axis = k; sign = s; }
            t1 = t1.min(b);
            if t0 > t1 { return None; }
        }
        if axis == usize::MAX { return None; }
        let mut n = [0.0; 3];
        n[axis] = sign;
        let w = self.to_world(Vec2::new(n[0], n[2]));
        Some((t0, Vec3::new(w.x, n[1], w.y)))
    }
}

/// Closest hit among boxes: distance, normal and index.
pub fn raycast<'a>(boxes: impl IntoIterator<Item = &'a YawBox>, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)> {
    let mut best: Option<(f32, Vec3, usize)> = None;
    for (i, b) in boxes.into_iter().enumerate() {
        if let Some((t, n)) = b.ray(origin, dir, best.map_or(max, |h| h.0)) {
            if best.is_none_or(|h| t < h.0) { best = Some((t, n, i)); }
        }
    }
    best
}

/// A set of boxes movers and rays can query. Visits are in increasing index
/// order, so results are deterministic whatever the broadphase.
pub trait Boxes {
    /// Every live box whose footprint may overlap the XZ rectangle.
    fn near(&self, lo: Vec2, hi: Vec2, f: &mut dyn FnMut(usize, &YawBox));
    /// Closest hit: distance, outward normal, box index.
    fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)>;
}

impl Boxes for [YawBox] {
    fn near(&self, lo: Vec2, hi: Vec2, f: &mut dyn FnMut(usize, &YawBox)) {
        for (i, b) in self.iter().enumerate() {
            let r = b.reach();
            if b.center.x + r < lo.x || b.center.x - r > hi.x || b.center.z + r < lo.y || b.center.z - r > hi.y { continue; }
            f(i, b);
        }
    }
    fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)> { raycast(self, origin, dir, max) }
}

impl Boxes for Vec<YawBox> {
    fn near(&self, lo: Vec2, hi: Vec2, f: &mut dyn FnMut(usize, &YawBox)) { self.as_slice().near(lo, hi, f) }
    fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)> { self.as_slice().ray(origin, dir, max) }
}

impl<const N: usize> Boxes for [YawBox; N] {
    fn near(&self, lo: Vec2, hi: Vec2, f: &mut dyn FnMut(usize, &YawBox)) { self.as_slice().near(lo, hi, f) }
    fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)> { self.as_slice().ray(origin, dir, max) }
}

/// Boxes in a uniform XZ grid, for levels with thousands of pieces. Boxes
/// can be removed (destroyed) and restored without a rebuild; indices stay
/// stable. Each box is listed in every cell its footprint touches.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BoxGrid {
    pub boxes: Vec<YawBox>,
    alive: Vec<bool>,
    origin: Vec2,
    cell: f32,
    dims: [i32; 2],
    /// Start of each cell's run in `items` (dims.0 * dims.1 + 1 entries).
    starts: Vec<u32>,
    items: Vec<u32>,
    live: usize,
}

/// Reusable work space for grid queries and rays. It can be reused across
/// grids; its storage grows only when a query needs more capacity.
#[derive(Debug, Default)]
pub struct BoxGridScratch {
    visited: Vec<u32>,
    generation: u32,
    indices: Vec<usize>,
}

impl BoxGridScratch {
    fn begin(&mut self, boxes: usize) {
        if self.visited.len() < boxes { self.visited.resize(boxes, 0); }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.visited.fill(0);
            self.generation = 1;
        }
        self.indices.clear();
    }

    fn first_visit(&mut self, i: usize) -> bool {
        if self.visited[i] == self.generation { return false; }
        self.visited[i] = self.generation;
        true
    }
}

thread_local! {
    // Borrow the pool only to take/return scratch, never during callbacks.
    // Nested queries get another lease, and separate threads never contend.
    static GRID_SCRATCH: RefCell<Vec<BoxGridScratch>> = const { RefCell::new(Vec::new()) };
}

struct GridScratchLease(BoxGridScratch);

impl Drop for GridScratchLease {
    fn drop(&mut self) {
        let scratch = std::mem::take(&mut self.0);
        let _ = GRID_SCRATCH.try_with(|pool| pool.borrow_mut().push(scratch));
    }
}

fn with_grid_scratch<R>(f: impl FnOnce(&mut BoxGridScratch) -> R) -> R {
    let mut lease = GridScratchLease(GRID_SCRATCH.with(|pool| pool.borrow_mut().pop()).unwrap_or_default());
    f(&mut lease.0)
}

impl BoxGrid {
    /// Build with `cell` metres per grid cell (about the size of a typical
    /// box; 4 suits rooms of 1 to 3 metre pieces).
    pub fn new(boxes: Vec<YawBox>, cell: f32) -> Self {
        let cell = cell.max(0.1);
        let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for b in &boxes {
            let r = b.reach();
            lo = lo.min(Vec2::new(b.center.x - r, b.center.z - r));
            hi = hi.max(Vec2::new(b.center.x + r, b.center.z + r));
        }
        if boxes.is_empty() { (lo, hi) = (Vec2::ZERO, Vec2::ONE); }
        let dims = [(((hi.x - lo.x) / cell).ceil() as i32).max(1), (((hi.y - lo.y) / cell).ceil() as i32).max(1)];
        let mut g = BoxGrid { alive: vec![true; boxes.len()], live: boxes.len(), boxes, origin: lo, cell, dims, starts: Vec::new(), items: Vec::new() };
        let n = (dims[0] * dims[1]) as usize;
        let mut counts = vec![0u32; n + 1];
        let each = |g: &BoxGrid, f: &mut dyn FnMut(usize, usize)| for (i, b) in g.boxes.iter().enumerate() {
            let (a, z) = g.footprint(b);
            for cz in a[1]..=z[1] { for cx in a[0]..=z[0] { f((cz * g.dims[0] + cx) as usize, i); } }
        };
        each(&g, &mut |c, _| counts[c + 1] += 1);
        for k in 0..n { counts[k + 1] += counts[k]; }
        let mut fill = counts.clone();
        let mut items = vec![0u32; counts[n] as usize];
        each(&g, &mut |c, i| { items[fill[c] as usize] = i as u32; fill[c] += 1; });
        g.starts = counts;
        g.items = items;
        g
    }

    fn cell_of(&self, p: Vec2) -> [i32; 2] {
        let c = (p - self.origin) / self.cell;
        [(c.x.floor() as i32).clamp(0, self.dims[0] - 1), (c.y.floor() as i32).clamp(0, self.dims[1] - 1)]
    }

    fn footprint(&self, b: &YawBox) -> ([i32; 2], [i32; 2]) {
        let r = b.reach();
        (self.cell_of(Vec2::new(b.center.x - r, b.center.z - r)), self.cell_of(Vec2::new(b.center.x + r, b.center.z + r)))
    }

    fn cell_items(&self, cx: i32, cz: i32) -> &[u32] {
        let c = (cz * self.dims[0] + cx) as usize;
        &self.items[self.starts[c] as usize..self.starts[c + 1] as usize]
    }

    pub fn len(&self) -> usize { self.boxes.len() }
    pub fn is_empty(&self) -> bool { self.boxes.is_empty() }
    /// Boxes not removed.
    pub fn live(&self) -> usize { self.live }
    pub fn is_alive(&self, i: usize) -> bool { self.alive[i] }
    pub fn remove(&mut self, i: usize) { if std::mem::replace(&mut self.alive[i], false) { self.live -= 1; } }
    pub fn restore(&mut self, i: usize) { if !std::mem::replace(&mut self.alive[i], true) { self.live += 1; } }

    /// Live boxes whose footprint may overlap the XZ rectangle, each once,
    /// in index order.
    pub fn query(&self, lo: Vec2, hi: Vec2) -> Vec<usize> {
        with_grid_scratch(|scratch| self.query_with_scratch(lo, hi, scratch).to_vec())
    }

    /// Like [`Self::query`], retaining storage between calls instead of
    /// allocating an owned result. Only unique candidates are sorted.
    pub fn query_with_scratch<'s>(&self, lo: Vec2, hi: Vec2, scratch: &'s mut BoxGridScratch) -> &'s [usize] {
        scratch.begin(self.boxes.len());
        let (a, z) = (self.cell_of(lo), self.cell_of(hi));
        for cz in a[1]..=z[1] { for cx in a[0]..=z[0] {
            for &i in self.cell_items(cx, cz) {
                let i = i as usize;
                if self.alive[i] && scratch.first_visit(i) { scratch.indices.push(i); }
            }
        } }
        scratch.indices.sort_unstable();
        &scratch.indices
    }

    /// Live boxes overlapping a sphere (conservatively, by footprint and height).
    pub fn in_sphere(&self, center: Vec3, radius: f32) -> Vec<usize> {
        let r = Vec2::splat(radius);
        let c = Vec2::new(center.x, center.z);
        self.query(c - r, c + r).into_iter().filter(|&i| {
            let b = &self.boxes[i];
            let d = b.center - center;
            d.y.abs() <= b.half.y + radius && Vec2::new(d.x, d.z).length() <= b.reach() + radius
        }).collect()
    }
}

impl Boxes for BoxGrid {
    fn near(&self, lo: Vec2, hi: Vec2, f: &mut dyn FnMut(usize, &YawBox)) {
        with_grid_scratch(|scratch| {
            for &i in self.query_with_scratch(lo, hi, scratch) { f(i, &self.boxes[i]); }
        });
    }

    fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)> {
        with_grid_scratch(|scratch| self.ray_with_scratch(origin, dir, max, scratch))
    }
}

impl BoxGrid {
    /// Walks the cells along the ray (2D DDA on XZ) and stops once the best
    /// hit is nearer than the next cell. Reuses visited-box storage, with
    /// constant-time duplicate checks even for rays crossing many cells.
    pub fn ray_with_scratch(&self, origin: Vec3, dir: Vec3, max: f32, scratch: &mut BoxGridScratch) -> Option<(f32, Vec3, usize)> {
        scratch.begin(self.boxes.len());
        let mut best: Option<(f32, Vec3, usize)> = None;
        let p = (Vec2::new(origin.x, origin.z) - self.origin) / self.cell;
        let d = Vec2::new(dir.x, dir.z);
        let mut c = [p.x.floor() as i32, p.y.floor() as i32];
        let step = [if d.x >= 0.0 { 1 } else { -1 }, if d.y >= 0.0 { 1 } else { -1 }];
        // Ray distance (along `dir`, metres) to the next cell boundary per axis.
        let next = |k: usize, c: i32| -> f32 {
            let dk = if k == 0 { d.x } else { d.y };
            if dk.abs() < 1e-9 { return f32::INFINITY; }
            let pk = if k == 0 { p.x } else { p.y };
            let edge = if dk > 0.0 { (c + 1) as f32 } else { c as f32 };
            (edge - pk) * self.cell / dk
        };
        let delta = [if d.x.abs() < 1e-9 { f32::INFINITY } else { self.cell / d.x.abs() }, if d.y.abs() < 1e-9 { f32::INFINITY } else { self.cell / d.y.abs() }];
        let mut t_next = [next(0, c[0]), next(1, c[1])];
        loop {
            let inside = c[0] >= 0 && c[1] >= 0 && c[0] < self.dims[0] && c[1] < self.dims[1];
            if inside {
                for &i in self.cell_items(c[0], c[1]) {
                    if !self.alive[i as usize] || !scratch.first_visit(i as usize) { continue; }
                    if let Some((t, n)) = self.boxes[i as usize].ray(origin, dir, best.map_or(max, |h| h.0)) {
                        if best.is_none_or(|h| t < h.0 || (t == h.0 && (i as usize) < h.2)) { best = Some((t, n, i as usize)); }
                    }
                }
            }
            let t_exit = t_next[0].min(t_next[1]);
            if t_exit > max || best.is_some_and(|h| h.0 <= t_exit) { break; }
            // Outside the grid and moving away: nothing more to find.
            let leaving = (c[0] < 0 && step[0] < 0) || (c[0] >= self.dims[0] && step[0] > 0)
                || (c[1] < 0 && step[1] < 0) || (c[1] >= self.dims[1] && step[1] > 0);
            if leaving { break; }
            let k = if t_next[0] < t_next[1] { 0 } else { 1 };
            c[k] += step[k];
            t_next[k] += delta[k];
        }
        best
    }
}

/// Ray against a sphere: entry distance (0 when starting inside).
pub fn ray_sphere(origin: Vec3, dir: Vec3, center: Vec3, radius: f32, max: f32) -> Option<f32> {
    let m = origin - center;
    let b = m.dot(dir);
    let c = m.length_squared() - radius * radius;
    if c > 0.0 && b > 0.0 { return None; }
    let disc = b * b - c;
    if disc < 0.0 { return None; }
    let t = (-b - disc.sqrt()).max(0.0);
    (t <= max).then_some(t)
}

/// An upright cylinder character.
#[derive(Clone, Copy, Debug)]
pub struct Mover { pub radius: f32, pub half_height: f32 }

/// What a move touched.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Contact {
    pub on_floor: bool,
    pub on_ceiling: bool,
    /// Outward normal of the last wall touched while not on the floor.
    pub wall_normal: Option<Vec3>,
}

impl Mover {
    fn overlaps_footprint(&self, b: &YawBox, p: Vec3, skin: f32) -> bool {
        let local = b.to_local(Vec2::new(p.x - b.center.x, p.z - b.center.z));
        let half = Vec2::new(b.half.x, b.half.z);
        (local - local.clamp(-half, half)).length_squared() < (self.radius - skin).max(0.0).powi(2)
    }

    fn support_height<B: Boxes + ?Sized>(&self, boxes: &B, p: Vec3, lo: f32, hi: f32) -> Option<f32> {
        let center = Vec2::new(p.x, p.z);
        let radius = Vec2::splat(self.radius);
        let mut top: Option<f32> = None;
        boxes.near(center - radius, center + radius, &mut |_, b| {
            let y = b.top();
            if y >= lo && y <= hi && self.overlaps_footprint(b, p, 0.0) {
                top = Some(top.map_or(y, |old| old.max(y)));
            }
        });
        top
    }

    // Test the whole vertical sweep, including thin overhead geometry that
    // could lie between the starting and raised positions. Touching is clear.
    fn clear_vertical<B: Boxes + ?Sized>(&self, boxes: &B, p: Vec3, rise: f32) -> bool {
        let center = Vec2::new(p.x, p.z);
        let radius = Vec2::splat(self.radius);
        let (lo, hi) = (p.y - self.half_height, p.y + self.half_height + rise);
        let mut clear = true;
        boxes.near(center - radius, center + radius, &mut |_, b| {
            if clear && b.top() > lo + 1e-4 && b.center.y - b.half.y < hi - 1e-4 && self.overlaps_footprint(b, p, 1e-4) {
                clear = false;
            }
        });
        clear
    }

    /// Slide normally, with an optional grounded step over low obstacles.
    /// The extra attempt rises at most `step_height` for the entire move,
    /// requires headroom and landing support, and is used only when it makes
    /// more forward progress. Jumping, airborne movement, and zero height use
    /// ordinary [`Self::slide`]. Its existing depenetration is unchanged.
    /// Full step-height headroom is required, even for a lower obstacle.
    pub fn slide_with_step<B: Boxes + ?Sized>(&self, boxes: &B, pos: &mut Vec3, vel: &mut Vec3, dt: f32, max_step: f32, step_height: f32, grounded: bool) -> Contact {
        let (start, incoming) = (*pos, *vel);
        let ordinary = self.slide(boxes, pos, vel, dt, max_step);
        if !grounded || incoming.y > 0.0 || !step_height.is_finite() || step_height <= 0.0 { return ordinary; }
        let travel = Vec2::new(incoming.x, incoming.z) * dt;
        let distance = travel.length();
        if distance < 1e-5 { return ordinary; }
        let direction = travel / distance;
        let progress = Vec2::new(pos.x - start.x, pos.z - start.z).dot(direction);
        if progress >= distance - 1e-4 { return ordinary; }
        let feet = start.y - self.half_height;
        // A stale grounded flag (for example after destroying the floor)
        // must not turn this into a midair lift.
        if self.support_height(boxes, start, feet - 0.02, feet + 1e-4).is_none()
            || !self.clear_vertical(boxes, start, step_height) { return ordinary; }

        let mut raised = start + Vec3::Y * step_height;
        let mut horizontal = Vec3::new(incoming.x, 0.0, incoming.z);
        let across = self.slide(boxes, &mut raised, &mut horizontal, dt, max_step);
        // A horizontal push that lifts us again would climb a taller wall.
        // Allow rounding at the step limit; never accept a ceiling contact.
        if across.on_ceiling || raised.y > start.y + step_height + 1e-4 { return ordinary; }
        let stepped_progress = Vec2::new(raised.x - start.x, raised.z - start.z).dot(direction);
        if stepped_progress <= progress + 1e-4 { return ordinary; }
        let Some(top) = self.support_height(boxes, raised, feet - 1e-4, feet + step_height + 1e-4) else { return ordinary; };
        let landed = Vec3::new(raised.x, top + self.half_height, raised.z);
        if !self.clear_vertical(boxes, landed, (raised.y - landed.y).max(0.0)) { return ordinary; }
        *pos = landed;
        *vel = horizontal;
        Contact { on_floor: true, on_ceiling: false, wall_normal: None }
    }

    /// Move by `vel * dt`, sliding along boxes (Godot's move_and_slide for
    /// boxes): sub-steps of at most `max_step` metres, up to four push-out
    /// passes each. Velocity loses its component into walls, floors and
    /// ceilings. Small ledges (shallower than the side push) are stepped up.
    pub fn slide<B: Boxes + ?Sized>(&self, boxes: &B, pos: &mut Vec3, vel: &mut Vec3, dt: f32, max_step: f32) -> Contact {
        let travel = *vel * dt;
        let steps = (travel.length() / max_step).ceil().max(1.0) as usize;
        let mut c = Contact::default();
        for _ in 0..steps {
            *pos += travel / steps as f32;
            for _ in 0..4 {
                let mut any = false;
                let r = Vec2::splat(self.radius + 0.01);
                // Keep the pre-pass bounds fixed while pushes change pos.
                let center = Vec2::new(pos.x, pos.z);
                boxes.near(center - r, center + r, &mut |_, b| {
                    let Some(push) = b.push_out(*pos, self.radius, self.half_height) else { return };
                    any = true;
                    *pos += push;
                    if push.y > 0.0 {
                        c.on_floor = true;
                        vel.y = vel.y.max(0.0);
                    } else if push.y < 0.0 {
                        c.on_ceiling = true;
                        vel.y = vel.y.min(0.0);
                    } else {
                        let n = push.normalize();
                        let into = vel.dot(n);
                        if into < 0.0 { *vel -= n * into; }
                        c.wall_normal = Some(n);
                    }
                });
                if !any { break; }
            }
        }
        if c.on_floor { c.wall_normal = None; }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step_world(height: f32, yaw: f32) -> Vec<YawBox> {
        let direction = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
        vec![
            YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(20.0, 1.0, 20.0), 0.0),
            YawBox::new(direction + Vec3::Y * (height * 0.5), Vec3::new(1.0, height, 2.0), yaw),
        ]
    }

    #[test]
    fn grounded_steps_clear_small_and_rotated_stones_at_the_height_limit() {
        let mover = Mover { radius: 0.4, half_height: 0.9 };
        for height in [0.2, 0.35] {
            for yaw in [0.0f32, 0.6, -0.8] {
                let boxes = step_world(height, yaw);
                let grid = BoxGrid::new(boxes.clone(), 1.0);
                let direction = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
                let (mut p, mut v) = (Vec3::new(0.0, 0.9, 0.0), direction * 6.0 - Vec3::Y * 3.0);
                let (mut ordinary, mut stopped) = (p, v);
                mover.slide(&grid, &mut ordinary, &mut stopped, 1.0 / 30.0, 0.25);
                let (mut q, mut w) = (p, v);
                let contact = mover.slide_with_step(&grid, &mut p, &mut v, 1.0 / 30.0, 0.25, 0.35, true);
                let plain = mover.slide_with_step(&boxes, &mut q, &mut w, 1.0 / 30.0, 0.25, 0.35, true);
                assert_eq!((contact, p, v), (plain, q, w));
                assert!(contact.on_floor && !contact.on_ceiling);
                assert!(p.dot(direction) > ordinary.dot(direction) + 0.05, "height {height}, yaw {yaw}: {p} vs {ordinary}");
                assert!((p.y - (height + 0.9)).abs() < 1e-4, "{p}");
                assert!((v - direction * 6.0).length() < 1e-4, "{v}");
            }
        }
    }

    #[test]
    fn stepping_does_not_climb_tall_walls_or_pass_through_low_ceilings() {
        let mover = Mover { radius: 0.4, half_height: 0.9 };
        let boxes = step_world(0.8, 0.0);
        let mut p = Vec3::new(0.0, 0.9, 0.0);
        for _ in 0..30 {
            let mut v = Vec3::new(6.0, -3.0, 0.0);
            mover.slide_with_step(&boxes, &mut p, &mut v, 1.0 / 30.0, 0.25, 0.35, true);
            assert!(p.x <= 0.1001 && p.y <= 0.9001, "{p}");
        }
        let mut boxes = step_world(0.2, 0.0);
        boxes.push(YawBox::new(Vec3::new(0.0, 1.92, 0.0), Vec3::new(10.0, 0.02, 10.0), 0.0));
        // The larger lift would finish above the thin ceiling: checking only
        // the raised endpoint would incorrectly permit crossing it.
        for step_height in [0.35, 2.5] {
            let (mut p, mut v) = (Vec3::new(0.0, 0.9, 0.0), Vec3::new(6.0, -3.0, 0.0));
            let (mut q, mut w) = (p, v);
            let expected = mover.slide(&boxes, &mut q, &mut w, 1.0 / 30.0, 0.25);
            let actual = mover.slide_with_step(&boxes, &mut p, &mut v, 1.0 / 30.0, 0.25, step_height, true);
            assert_eq!((actual, p, v), (expected, q, w));
        }
    }

    #[test]
    fn stepping_requires_ground_support_and_never_intercepts_jumps() {
        let mover = Mover { radius: 0.4, half_height: 0.9 };
        let mut grid = BoxGrid::new(step_world(0.25, 0.0), 1.0);
        for (grounded, vy, height, remove_floor) in [(false, -3.0, 0.35, false), (true, 4.0, 0.35, false), (true, -3.0, 0.0, false), (true, -3.0, 0.35, true)] {
            if remove_floor { grid.remove(0); }
            let (mut p, mut v) = (Vec3::new(0.0, 0.9, 0.0), Vec3::new(6.0, vy, 0.0));
            let (mut q, mut w) = (p, v);
            let expected = mover.slide(&grid, &mut q, &mut w, 1.0 / 30.0, 0.25);
            let actual = mover.slide_with_step(&grid, &mut p, &mut v, 1.0 / 30.0, 0.25, height, grounded);
            assert_eq!((actual, p, v), (expected, q, w));
            grid.restore(0);
        }
    }

    #[test]
    fn stepping_works_from_an_existing_wall_contact() {
        let mover = Mover { radius: 0.4, half_height: 0.9 };
        let boxes = step_world(0.3, 0.0);
        let (mut p, mut v) = (Vec3::new(0.1, 0.9, 0.0), Vec3::new(6.0, -3.0, 0.0));
        let contact = mover.slide_with_step(&boxes, &mut p, &mut v, 1.0 / 30.0, 0.25, 0.35, true);
        assert!(contact.on_floor && p.x > 0.29 && (p.y - 1.2).abs() < 1e-4, "{p}");
    }

    #[test]
    fn stepping_requires_a_landing_and_limits_a_whole_move_across_multiple_risers() {
        let mover = Mover { radius: 0.4, half_height: 0.9 };
        let edge = [
            YawBox::new(Vec3::new(-5.0, -0.5, 0.0), Vec3::new(10.0, 1.0, 10.0), 0.0),
            YawBox::new(Vec3::new(0.35, 0.15, 0.0), Vec3::new(0.1, 0.3, 2.0), 0.0),
        ];
        let (mut p, mut v) = (Vec3::new(-0.4, 0.9, 0.0), Vec3::new(1.2, -0.1, 0.0));
        let (mut q, mut w) = (p, v);
        let expected = mover.slide(&edge, &mut q, &mut w, 1.0, 0.2);
        let actual = mover.slide_with_step(&edge, &mut p, &mut v, 1.0, 0.2, 0.35, true);
        assert_eq!((actual, p, v), (expected, q, w), "no support at the raised path's endpoint");

        let mut stairs = step_world(0.2, 0.0);
        stairs.push(YawBox::new(Vec3::new(2.0, 0.25, 0.0), Vec3::new(1.0, 0.5, 2.0), 0.0));
        let (mut p, mut v) = (Vec3::new(0.0, 0.9, 0.0), Vec3::new(3.0, -0.1, 0.0));
        mover.slide_with_step(&stairs, &mut p, &mut v, 1.0, 0.1, 0.35, true);
        assert!(p.y <= 1.2501 && p.x <= 1.1001, "the second riser exceeds the original height budget: {p}");
    }

    // Preserve the pre-scratch algorithms as behavioral/performance references.
    fn legacy_query(grid: &BoxGrid, lo: Vec2, hi: Vec2) -> Vec<usize> {
        let (a, z) = (grid.cell_of(lo), grid.cell_of(hi));
        let mut out = Vec::new();
        for cz in a[1]..=z[1] { for cx in a[0]..=z[0] {
            out.extend(grid.cell_items(cx, cz).iter().map(|&i| i as usize).filter(|&i| grid.alive[i]));
        } }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn legacy_ray(grid: &BoxGrid, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)> {
        let mut best: Option<(f32, Vec3, usize)> = None;
        let mut tested: Vec<u32> = Vec::new();
        let p = (Vec2::new(origin.x, origin.z) - grid.origin) / grid.cell;
        let d = Vec2::new(dir.x, dir.z);
        let mut c = [p.x.floor() as i32, p.y.floor() as i32];
        let step = [if d.x >= 0.0 { 1 } else { -1 }, if d.y >= 0.0 { 1 } else { -1 }];
        // Ray distance (along `dir`, metres) to the next cell boundary per axis.
        let next = |k: usize, c: i32| -> f32 {
            let dk = if k == 0 { d.x } else { d.y };
            if dk.abs() < 1e-9 { return f32::INFINITY; }
            let pk = if k == 0 { p.x } else { p.y };
            let edge = if dk > 0.0 { (c + 1) as f32 } else { c as f32 };
            (edge - pk) * grid.cell / dk
        };
        let delta = [if d.x.abs() < 1e-9 { f32::INFINITY } else { grid.cell / d.x.abs() }, if d.y.abs() < 1e-9 { f32::INFINITY } else { grid.cell / d.y.abs() }];
        let mut t_next = [next(0, c[0]), next(1, c[1])];
        loop {
            let inside = c[0] >= 0 && c[1] >= 0 && c[0] < grid.dims[0] && c[1] < grid.dims[1];
            if inside {
                for &i in grid.cell_items(c[0], c[1]) {
                    if !grid.alive[i as usize] || tested.contains(&i) { continue; }
                    tested.push(i);
                    if let Some((t, n)) = grid.boxes[i as usize].ray(origin, dir, best.map_or(max, |h| h.0)) {
                        if best.is_none_or(|h| t < h.0 || (t == h.0 && (i as usize) < h.2)) { best = Some((t, n, i as usize)); }
                    }
                }
            }
            let t_exit = t_next[0].min(t_next[1]);
            if t_exit > max || best.is_some_and(|h| h.0 <= t_exit) { break; }
            // Outside the grid and moving away: nothing more to find.
            let leaving = (c[0] < 0 && step[0] < 0) || (c[0] >= grid.dims[0] && step[0] > 0)
                || (c[1] < 0 && step[1] < 0) || (c[1] >= grid.dims[1] && step[1] > 0);
            if leaving { break; }
            let k = if t_next[0] < t_next[1] { 0 } else { 1 };
            c[k] += step[k];
            t_next[k] += delta[k];
        }
        best
    }

    fn legacy_slide<B: Boxes + ?Sized>(mover: &Mover, boxes: &B, pos: &mut Vec3, vel: &mut Vec3, dt: f32, max_step: f32) -> Contact {
        let travel = *vel * dt;
        let steps = (travel.length() / max_step).ceil().max(1.0) as usize;
        let mut c = Contact::default();
        let mut near: Vec<(usize, YawBox)> = Vec::new();
        for _ in 0..steps {
            *pos += travel / steps as f32;
            for _ in 0..4 {
                let mut any = false;
                near.clear();
                let r = Vec2::splat(mover.radius + 0.01);
                boxes.near(Vec2::new(pos.x, pos.z) - r, Vec2::new(pos.x, pos.z) + r, &mut |i, b| near.push((i, *b)));
                for (_, b) in &near {
                    let Some(push) = b.push_out(*pos, mover.radius, mover.half_height) else { continue };
                    any = true;
                    *pos += push;
                    if push.y > 0.0 {
                        c.on_floor = true;
                        vel.y = vel.y.max(0.0);
                    } else if push.y < 0.0 {
                        c.on_ceiling = true;
                        vel.y = vel.y.min(0.0);
                    } else {
                        let n = push.normalize();
                        let into = vel.dot(n);
                        if into < 0.0 { *vel -= n * into; }
                        c.wall_normal = Some(n);
                    }
                }
                if !any { break; }
            }
        }
        if c.on_floor { c.wall_normal = None; }
        c
    }

    #[test]
    fn ray_hits_rotated_box_on_the_right_face() {
        let b = YawBox::new(Vec3::new(10.0, 1.0, 0.0), Vec3::splat(2.0), std::f32::consts::FRAC_PI_4);
        let (t, n) = b.ray(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 100.0).unwrap();
        assert!((t - (10.0 - 2f32.sqrt())).abs() < 1e-4, "{t}");
        assert!(n.x < -0.5 && n.y.abs() < 1e-6, "{n}");
        assert!(b.ray(Vec3::new(0.0, 3.5, 0.0), Vec3::X, 100.0).is_none());
        assert!(b.ray(Vec3::ZERO, Vec3::NEG_X, 100.0).is_none());
    }

    #[test]
    fn push_out_floor_and_wall() {
        let floor = YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(10.0, 1.0, 10.0), 0.0);
        let up = floor.push_out(Vec3::new(1.0, 0.85, 0.0), 0.4, 0.9).unwrap();
        assert!((up.y - 0.05).abs() < 1e-5 && up.x == 0.0);
        let wall = YawBox::new(Vec3::new(2.0, 2.0, 0.0), Vec3::new(1.0, 4.0, 4.0), 0.0);
        let side = wall.push_out(Vec3::new(1.2, 1.0, 0.0), 0.4, 0.9).unwrap();
        assert!((side.x + 0.1).abs() < 1e-5 && side.y == 0.0, "{side}");
        assert!(wall.push_out(Vec3::new(0.5, 1.0, 0.0), 0.4, 0.9).is_none());
    }

    fn scatter(n: usize, seed: u64) -> Vec<YawBox> {
        let mut s = seed;
        let mut r = move || { s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (s >> 40) as f32 / (1u64 << 24) as f32 };
        (0..n).map(|_| YawBox::new(Vec3::new(r() * 80.0 - 40.0, r() * 6.0, r() * 80.0 - 40.0),
            Vec3::new(0.3 + r() * 3.0, 0.3 + r() * 3.0, 0.3 + r() * 3.0), if r() < 0.5 { 0.0 } else { r() * 6.28 })).collect()
    }

    struct LegacyGrid<'a>(&'a BoxGrid);

    impl Boxes for LegacyGrid<'_> {
        fn near(&self, lo: Vec2, hi: Vec2, f: &mut dyn FnMut(usize, &YawBox)) {
            for i in legacy_query(self.0, lo, hi) { f(i, &self.0.boxes[i]); }
        }
        fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3, usize)> {
            legacy_ray(self.0, origin, dir, max)
        }
    }

    #[test]
    fn scratch_matches_legacy_after_removal_restoration_and_grid_changes() {
        let mut grid = BoxGrid::new(scatter(600, 7), 2.0);
        let other = BoxGrid::new(scatter(17, 93), 4.0);
        let empty = BoxGrid::new(Vec::new(), 1.0);
        let mut scratch = BoxGridScratch::default();
        let mut state = 73u64;
        let mut r = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 40) as f32 / (1u64 << 24) as f32
        };
        let mover = Mover { radius: 0.4, half_height: 0.9 };
        for k in 0..2000 {
            let changed = (r() * grid.len() as f32) as usize;
            if k % 3 == 0 { grid.restore(changed); } else { grid.remove(changed); }
            let origin = Vec3::new(r() * 160.0 - 80.0, r() * 12.0 - 2.0, r() * 160.0 - 80.0);
            let center = Vec2::new(origin.x, origin.z);
            let extent = Vec2::new(r() * 10.0, r() * 10.0);
            let (lo, hi) = (center - extent, center + extent);
            let want = legacy_query(&grid, lo, hi);
            assert_eq!(grid.query_with_scratch(lo, hi, &mut scratch), want, "query {k}");
            let dir = match k % 11 {
                0 => Vec3::NEG_Y,
                1 => Vec3::X,
                2 => Vec3::NEG_Z,
                _ => Vec3::new(r() - 0.5, r() - 0.5, r() - 0.5).normalize(),
            };
            let max = r() * 120.0;
            assert_eq!(grid.ray_with_scratch(origin, dir, max, &mut scratch), legacy_ray(&grid, origin, dir, max), "ray {k}");
            // Alternate grid sizes without clearing or shrinking scratch.
            assert_eq!(other.query_with_scratch(lo, hi, &mut scratch), legacy_query(&other, lo, hi));
            assert!(empty.query_with_scratch(lo, hi, &mut scratch).is_empty());
            if k % 5 == 0 {
                // Start near geometry so the randomized moves include contacts.
                let b = grid.boxes[changed];
                let (mut p, mut v) = (b.center + Vec3::new(r() - 0.5, r() * 3.0, r() - 0.5), dir * 12.0);
                let (mut q, mut w) = (p, v);
                let a = mover.slide(&grid, &mut p, &mut v, 1.0 / 30.0, 0.25);
                let b = legacy_slide(&mover, &LegacyGrid(&grid), &mut q, &mut w, 1.0 / 30.0, 0.25);
                assert_eq!((a, p, v), (b, q, w), "move {k}");
            }
        }
        scratch.generation = u32::MAX;
        scratch.visited.fill(1);
        assert_eq!(grid.query_with_scratch(Vec2::splat(-100.0), Vec2::splat(100.0), &mut scratch), legacy_query(&grid, Vec2::splat(-100.0), Vec2::splat(100.0)));
        assert_eq!(scratch.generation, 1);
    }

    #[test]
    fn grid_ray_ties_and_boundaries_keep_the_original_index() {
        let b = YawBox::new(Vec3::new(2.0, 0.0, 0.0), Vec3::splat(2.0), 0.0);
        let mut grid = BoxGrid::new(vec![b, b, YawBox::new(Vec3::new(5.0, 0.0, 0.0), Vec3::splat(2.0), 0.4)], 1.0);
        assert_eq!(grid.ray(Vec3::ZERO, Vec3::X, 20.0).unwrap().2, 0);
        grid.remove(0);
        assert_eq!(grid.ray(Vec3::ZERO, Vec3::X, 20.0).unwrap().2, 1);
        grid.restore(0);
        for origin in [Vec3::ZERO, Vec3::new(-10.0, 0.0, 0.0), Vec3::new(2.0, 10.0, 0.0), Vec3::new(grid.origin.x, 0.0, grid.origin.y)] {
            for dir in [Vec3::X, Vec3::NEG_X, Vec3::NEG_Y, Vec3::new(1.0, 0.0, 1.0).normalize()] {
                for max in [0.0, 1.0, 20.0] { assert_eq!(grid.ray(origin, dir, max), legacy_ray(&grid, origin, dir, max)); }
            }
        }
    }

    #[test]
    fn near_callbacks_can_reenter_and_share_a_grid_across_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<BoxGrid>();
        let grid = BoxGrid::new(scatter(40, 11), 3.0);
        let (lo, hi) = (Vec2::splat(-100.0), Vec2::splat(100.0));
        let expected = legacy_query(&grid, lo, hi);
        let mut seen = Vec::new();
        grid.near(lo, hi, &mut |i, _| {
            seen.push(i);
            let mut nested = Vec::new();
            grid.near(lo, hi, &mut |j, _| nested.push(j));
            assert_eq!(nested, expected);
            assert_eq!(grid.ray(Vec3::ZERO, Vec3::X, 100.0), legacy_ray(&grid, Vec3::ZERO, Vec3::X, 100.0));
        });
        assert_eq!(seen, expected);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..50 { assert_eq!(grid.query(lo, hi), expected); }
                });
            }
        });
        let panicked = std::panic::catch_unwind(|| grid.near(lo, hi, &mut |_, _| panic!("test callback")));
        assert!(panicked.is_err());
        assert_eq!(grid.query(lo, hi), expected);
    }

    #[test]
    fn warm_queries_rays_and_moves_retain_scratch_allocations() {
        let mut boxes = scatter(600, 7);
        boxes.push(YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(100.0, 1.0, 100.0), 0.0));
        let grid = BoxGrid::new(boxes, 2.0);
        let (lo, hi) = (Vec2::splat(-100.0), Vec2::splat(100.0));
        grid.near(lo, hi, &mut |_, _| {});
        let storage = || GRID_SCRATCH.with(|pool| {
            let pool = pool.borrow();
            let s = pool.last().unwrap();
            (pool.len(), pool.capacity(), s.visited.as_ptr(), s.visited.capacity(), s.indices.as_ptr(), s.indices.capacity())
        });
        let initial = storage();
        let mover = Mover { radius: 0.4, half_height: 0.9 };
        for k in 0..500 {
            let x = k as f32 * 0.1 - 25.0;
            let center = Vec2::new(x, 0.0);
            grid.near(center - Vec2::ONE, center + Vec2::ONE, &mut |_, _| {});
            std::hint::black_box(grid.ray(Vec3::new(x, 10.0, 0.0), Vec3::NEG_Y, 30.0));
            let (mut p, mut v) = (Vec3::new(x, 0.8, 0.0), Vec3::new(1.0, -3.0, 1.0));
            std::hint::black_box(mover.slide(&grid, &mut p, &mut v, 1.0 / 60.0, 0.25));
        }
        assert_eq!(storage(), initial, "all scratch vectors retain their buffers after warmup");
    }

    #[test]
    #[ignore = "manual CPU microbenchmark; run with --ignored --nocapture"]
    fn collision_scratch_benchmark() {
        use std::{hint::black_box, time::Instant};
        let mut boxes = scatter(8000, 7);
        boxes.push(YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(100.0, 1.0, 100.0), 0.0));
        let grid = BoxGrid::new(boxes, 2.0);
        let mover = Mover { radius: 0.4, half_height: 0.9 };
        grid.near(Vec2::splat(-100.0), Vec2::splat(100.0), &mut |_, _| {});
        for round in 0..3 {
        for legacy in if round % 2 == 0 { [true, false] } else { [false, true] } {
            let start = Instant::now();
            for k in 0..10000 {
                let c = Vec2::new((k % 100) as f32 * 0.8 - 40.0, (k / 100) as f32 * 0.8 - 40.0);
                if legacy { black_box(legacy_query(&grid, c - Vec2::splat(3.0), c + Vec2::splat(3.0))); }
                else { grid.near(c - Vec2::splat(3.0), c + Vec2::splat(3.0), &mut |i, _| { black_box(i); }); }
            }
            let query = start.elapsed();
            let start = Instant::now();
            for k in 0..4000 {
                let origin = Vec3::new(-60.0, 20.0, (k % 100) as f32 * 0.8 - 40.0);
                black_box(if legacy { legacy_ray(&grid, origin, Vec3::X, 120.0) } else { grid.ray(origin, Vec3::X, 120.0) });
            }
            let ray = start.elapsed();
            let start = Instant::now();
            for k in 0..4000 {
                let (mut p, mut v) = (Vec3::new((k % 100) as f32 * 0.8 - 40.0, 0.8, 0.0), Vec3::new(1.0, -3.0, 1.0));
                black_box(if legacy { legacy_slide(&mover, &LegacyGrid(&grid), &mut p, &mut v, 1.0 / 60.0, 0.25) } else { mover.slide(&grid, &mut p, &mut v, 1.0 / 60.0, 0.25) });
            }
            eprintln!("collision round={round} legacy={legacy}: query_10000={query:?} ray_4000={ray:?} mover_4000={:?}", start.elapsed());
        }
        }
    }

    #[test]
    fn grid_rays_and_queries_match_the_plain_scan() {
        let boxes = scatter(600, 7);
        let mut grid = BoxGrid::new(boxes.clone(), 4.0);
        let mut s = 99u64;
        let mut r = move || { s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (s >> 40) as f32 / (1u64 << 24) as f32 };
        for k in 0..2000 {
            let o = Vec3::new(r() * 120.0 - 60.0, r() * 8.0 - 1.0, r() * 120.0 - 60.0);
            let d = Vec3::new(r() - 0.5, (r() - 0.5) * if k % 7 == 0 { 0.0 } else { 0.6 }, r() - 0.5).normalize();
            let d = if k % 13 == 0 { Vec3::NEG_Y } else { d };
            let max = r() * 90.0;
            let a = boxes.as_slice().ray(o, d, max);
            let b = grid.ray(o, d, max);
            assert_eq!(a.map(|h| h.2), b.map(|h| h.2), "ray {k}: {o} {d} {max}");
        }
        for i in (0..600).step_by(3) { grid.remove(i); }
        assert_eq!(grid.live(), 400);
        let live: Vec<YawBox> = boxes.iter().enumerate().map(|(i, b)| if i % 3 == 0 { YawBox::new(Vec3::splat(1e6), Vec3::ONE, 0.0) } else { *b }).collect();
        for _ in 0..500 {
            let o = Vec3::new(r() * 100.0 - 50.0, r() * 6.0, r() * 100.0 - 50.0);
            let d = Vec3::new(r() - 0.5, (r() - 0.5) * 0.4, r() - 0.5).normalize();
            assert_eq!(live.as_slice().ray(o, d, 70.0).map(|h| h.2), grid.ray(o, d, 70.0).map(|h| h.2));
            let near = grid.query(Vec2::new(o.x - 2.0, o.z - 2.0), Vec2::new(o.x + 2.0, o.z + 2.0));
            let mut want = Vec::new();
            live.as_slice().near(Vec2::new(o.x - 2.0, o.z - 2.0), Vec2::new(o.x + 2.0, o.z + 2.0), &mut |i, _| want.push(i));
            assert!(want.iter().all(|i| near.contains(i)), "grid query misses a box");
            assert!(near.windows(2).all(|w| w[0] < w[1]));
        }
        grid.restore(0);
        assert_eq!(grid.live(), 401);
    }

    #[test]
    fn mover_on_a_grid_matches_the_slice() {
        let mut boxes = scatter(300, 3);
        boxes.push(YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(100.0, 1.0, 100.0), 0.0));
        let grid = BoxGrid::new(boxes.clone(), 4.0);
        let m = Mover { radius: 0.4, half_height: 0.9 };
        for start in 0..20 {
            let (mut p, mut v) = (Vec3::new(start as f32 * 3.0 - 30.0, 8.0, 5.0), Vec3::new(4.0, -3.0, -6.0));
            let (mut q, mut w) = (p, v);
            for _ in 0..240 {
                v.y -= 0.5; w.y -= 0.5;
                let a = m.slide(&boxes, &mut p, &mut v, 1.0 / 60.0, 0.25);
                let b = m.slide(&grid, &mut q, &mut w, 1.0 / 60.0, 0.25);
                assert_eq!(a, b);
                assert_eq!(p, q);
            }
        }
    }

    #[test]
    fn mover_lands_slides_along_walls_and_stops() {
        let world = [YawBox::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(40.0, 1.0, 40.0), 0.0), YawBox::new(Vec3::new(3.0, 2.0, 0.0), Vec3::new(1.0, 4.0, 10.0), 0.0)];
        let m = Mover { radius: 0.4, half_height: 0.9 };
        let (mut p, mut v) = (Vec3::new(0.0, 3.0, 0.0), Vec3::new(5.0, -10.0, 3.0));
        let mut touched_wall = false;
        for _ in 0..120 {
            v.y -= 30.0 / 60.0;
            let c = m.slide(&world, &mut p, &mut v, 1.0 / 60.0, 0.25);
            touched_wall |= c.wall_normal.is_some() || (p.x - 2.1).abs() < 0.02;
        }
        assert!((p.y - 0.9).abs() < 0.02, "rests on the floor: {p}");
        assert!(p.x <= 2.5 - 0.4 + 1e-3, "kept out of the wall: {p}");
        assert!(p.z > 3.0, "slid along it: {p}");
        assert!(touched_wall);
        assert!(ray_sphere(Vec3::ZERO, Vec3::Z, Vec3::new(0.0, 0.0, 5.0), 1.0, 10.0).is_some());
        assert_eq!(raycast(&world, Vec3::new(0.0, 1.0, 0.0), Vec3::X, 10.0).map(|h| h.2), Some(1));
    }
}
