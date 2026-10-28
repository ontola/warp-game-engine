//! Flat 2D collision for the level: walls and props are circles and oriented
//! boxes on the ground plane, each with a top height so a lobbed bolt can
//! pass over a low wall. Shapes live in a uniform grid; a removed shape (a
//! broken prop) leaves a hole in the list so indices stay stable.

use glam::{Vec2, Vec3};
use std::collections::HashMap;

const CELL: f32 = 4.0;

#[derive(Clone, Copy, Debug)]
pub enum Shape2 {
    /// An oriented box: centre, half extents along `ax` and its normal, and
    /// the height of its top.
    Box {
        c: Vec2,
        half: Vec2,
        ax: Vec2,
        top: f32,
    },
    Circle {
        c: Vec2,
        r: f32,
        top: f32,
    },
}

impl Shape2 {
    pub fn top(&self) -> f32 {
        match *self {
            Shape2::Box { top, .. } | Shape2::Circle { top, .. } => top,
        }
    }

    fn reach(&self) -> (Vec2, Vec2) {
        match *self {
            Shape2::Box { c, half, ax, .. } => {
                let ax = ax.normalize_or(Vec2::X);
                let ay = ax.perp();
                let e = Vec2::new(
                    (ax.x * half.x).abs() + (ay.x * half.y).abs(),
                    (ax.y * half.x).abs() + (ay.y * half.y).abs(),
                );
                (c - e, c + e)
            }
            Shape2::Circle { c, r, .. } => (c - Vec2::splat(r), c + Vec2::splat(r)),
        }
    }

    /// The point on (or in) the shape nearest to `p` and how deep `p` is
    /// inside: depth > 0 means inside, and the normal points out.
    pub fn nearest(&self, p: Vec2) -> (Vec2, Vec2, f32) {
        match *self {
            Shape2::Circle { c, r, .. } => {
                let d = p - c;
                let len = d.length();
                let n = if len > 1e-6 { d / len } else { Vec2::X };
                (c + n * r, n, r - len)
            }
            Shape2::Box { c, half, ax, .. } => {
                let ax = ax.normalize_or(Vec2::X);
                let ay = ax.perp();
                let d = p - c;
                let l = Vec2::new(d.dot(ax), d.dot(ay));
                let q = l.abs() - half;
                if q.x <= 0.0 && q.y <= 0.0 {
                    // Inside: leave through the nearest face.
                    let (n, depth) = if q.x > q.y {
                        (ax * l.x.signum(), -q.x)
                    } else {
                        (ay * l.y.signum(), -q.y)
                    };
                    (p + n * depth, n, depth)
                } else {
                    let cl = l.clamp(-half, half);
                    let at = c + ax * cl.x + ay * cl.y;
                    let off = p - at;
                    let len = off.length();
                    (at, if len > 1e-6 { off / len } else { ax }, -len)
                }
            }
        }
    }
}

/// Where a ray hit: the fraction along it, the shape and the prop it
/// belongs to (if any), and the ground-plane normal.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub t: f32,
    pub shape: usize,
    pub prop: Option<usize>,
    pub normal: Vec2,
}

#[derive(Default, Clone)]
pub struct Statics {
    shapes: Vec<Option<Shape2>>,
    owners: Vec<Option<usize>>,
    grid: HashMap<(i32, i32), Vec<usize>>,
}

fn cell(p: Vec2) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
}

impl Statics {
    pub fn add(&mut self, s: Shape2) -> usize {
        let i = self.shapes.len();
        self.shapes.push(Some(s));
        self.owners.push(None);
        let (lo, hi) = s.reach();
        let (a, b) = (cell(lo), cell(hi));
        for x in a.0..=b.0 {
            for y in a.1..=b.1 {
                self.grid.entry((x, y)).or_default().push(i);
            }
        }
        i
    }

    pub fn remove(&mut self, i: usize) {
        if let Some(s) = self.shapes.get_mut(i) {
            *s = None;
        }
    }

    /// Mark `shape` as belonging to prop `prop`, so a hit can damage it.
    pub fn owner(&mut self, shape: Option<usize>, prop: usize) {
        if let Some(s) = shape {
            self.owners[s] = Some(prop);
        }
    }

    pub fn finish(&mut self) {
        for v in self.grid.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
    }

    pub fn get(&self, i: usize) -> Option<&Shape2> {
        self.shapes.get(i).and_then(|s| s.as_ref())
    }

    pub fn prop_of(&self, i: usize) -> Option<usize> {
        self.owners.get(i).copied().flatten()
    }

    pub fn len(&self) -> usize {
        self.shapes.iter().filter(|s| s.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The live shapes whose cells touch the box `lo..hi`, in index order.
    pub fn near(&self, lo: Vec2, hi: Vec2) -> Vec<usize> {
        let (a, b) = (cell(lo), cell(hi));
        let mut out = Vec::new();
        for x in a.0..=b.0 {
            for y in a.1..=b.1 {
                if let Some(v) = self.grid.get(&(x, y)) {
                    out.extend(v.iter().copied().filter(|&i| self.shapes[i].is_some()));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The shapes a circle at `p` with radius `r` overlaps.
    pub fn overlaps(&self, p: Vec2, r: f32) -> Vec<usize> {
        self.near(p - Vec2::splat(r), p + Vec2::splat(r))
            .into_iter()
            .filter(|&i| self.shapes[i].unwrap().nearest(p).2 > -r)
            .collect()
    }

    /// Push a circle out of everything it overlaps (a few rounds, so
    /// corners settle). Returns the corrected centre.
    pub fn push_out(&self, mut p: Vec2, r: f32) -> Vec2 {
        for _ in 0..4 {
            let mut moved = false;
            for i in self.near(p - Vec2::splat(r), p + Vec2::splat(r)) {
                let (_, n, depth) = self.shapes[i].unwrap().nearest(p);
                let pen = depth + r;
                if pen > 1e-4 {
                    p += n * pen;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        p
    }

    /// Move a circle by `delta`, sliding along what it meets (sub-stepped
    /// so a fast roll does not tunnel through a thin wall).
    pub fn slide(&self, from: Vec2, delta: Vec2, r: f32) -> Vec2 {
        let steps = ((delta.length() / (r * 0.5).max(0.05)).ceil() as usize).clamp(1, 16);
        let step = delta / steps as f32;
        let mut p = from;
        for _ in 0..steps {
            p = self.push_out(p + step, r);
        }
        p
    }

    /// A ray from `a` to `b` (3D: a shape stops it only below its top),
    /// thickened by `r`. The first hit, if any.
    pub fn ray(&self, a: Vec3, b: Vec3, r: f32) -> Option<Hit> {
        let a2 = Vec2::new(a.x, a.z);
        let b2 = Vec2::new(b.x, b.z);
        let lo = a2.min(b2) - Vec2::splat(r);
        let hi = a2.max(b2) + Vec2::splat(r);
        let mut best: Option<Hit> = None;
        for i in self.near(lo, hi) {
            let s = self.shapes[i].unwrap();
            if let Some((t, n)) = sweep(&s, a2, b2, r) {
                let y = a.y + (b.y - a.y) * t;
                if y > s.top() {
                    continue;
                }
                if best.is_none_or(|h| t < h.t) {
                    best = Some(Hit {
                        t,
                        shape: i,
                        prop: self.owners[i],
                        normal: n,
                    });
                }
            }
        }
        best
    }
}

/// When a circle of radius `r` moving from `a` to `b` first touches `s`.
fn sweep(s: &Shape2, a: Vec2, b: Vec2, r: f32) -> Option<(f32, Vec2)> {
    match *s {
        Shape2::Circle { c, r: cr, .. } => circle_hit(a, b, c, cr + r),
        Shape2::Box { c, half, ax, .. } => {
            let ax = ax.normalize_or(Vec2::X);
            let ay = ax.perp();
            let to = |p: Vec2| Vec2::new((p - c).dot(ax), (p - c).dot(ay));
            let (la, lb) = (to(a), to(b));
            let h = half + Vec2::splat(r);
            if la.x.abs() <= h.x && la.y.abs() <= h.y {
                let (_, n, _) = s.nearest(a);
                return Some((0.0, n));
            }
            // Slab test on the box grown by r (corners are square; close
            // enough for bolts).
            let d = lb - la;
            let mut t0 = 0.0f32;
            let mut t1 = 1.0f32;
            let mut n = Vec2::ZERO;
            for k in 0..2 {
                let (o, dd, e) = if k == 0 {
                    (la.x, d.x, h.x)
                } else {
                    (la.y, d.y, h.y)
                };
                if dd.abs() < 1e-9 {
                    if o.abs() > e {
                        return None;
                    }
                    continue;
                }
                let mut ta = (-e - o) / dd;
                let mut tb = (e - o) / dd;
                let mut side = -1.0;
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                    side = 1.0;
                }
                if ta > t0 {
                    t0 = ta;
                    n = if k == 0 { ax * side } else { ay * side };
                }
                t1 = t1.min(tb);
                if t0 > t1 {
                    return None;
                }
            }
            Some((t0, n))
        }
    }
}

fn circle_hit(a: Vec2, b: Vec2, c: Vec2, r: f32) -> Option<(f32, Vec2)> {
    let d = b - a;
    let f = a - c;
    let cc = f.length_squared() - r * r;
    if cc <= 0.0 {
        return Some((0.0, f.normalize_or(Vec2::X)));
    }
    let aa = d.length_squared();
    if aa < 1e-12 {
        return None;
    }
    let bb = f.dot(d);
    let disc = bb * bb - aa * cc;
    if disc < 0.0 {
        return None;
    }
    let t = (-bb - disc.sqrt()) / aa;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some((t, (a + d * t - c).normalize_or(Vec2::X)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_and_circles() {
        let mut s = Statics::default();
        let wall = s.add(Shape2::Box {
            c: Vec2::ZERO,
            half: Vec2::new(2.0, 0.5),
            ax: Vec2::X,
            top: 4.0,
        });
        let rock = s.add(Shape2::Circle {
            c: Vec2::new(6.0, 0.0),
            r: 1.0,
            top: 1.0,
        });
        s.owner(Some(rock), 7);
        s.finish();
        let p = s.push_out(Vec2::new(0.0, 0.6), 0.45);
        assert!((p.y - 0.95).abs() < 1e-4, "{p}");
        let h = s
            .ray(Vec3::new(0.0, 1.0, 5.0), Vec3::new(0.0, 1.0, -5.0), 0.0)
            .unwrap();
        assert_eq!(h.shape, wall);
        assert!((h.t - 0.45).abs() < 1e-4);
        assert!(h.normal.y > 0.9);
        let h = s
            .ray(Vec3::new(3.0, 0.5, 0.0), Vec3::new(9.0, 0.5, 0.0), 0.0)
            .unwrap();
        assert_eq!(h.prop, Some(7));
        assert!(
            s.ray(Vec3::new(3.0, 2.0, 0.0), Vec3::new(9.0, 2.0, 0.0), 0.0)
                .is_none()
        );
        s.remove(rock);
        assert!(
            s.ray(Vec3::new(3.0, 0.5, 0.0), Vec3::new(9.0, 0.5, 0.0), 0.0)
                .is_none()
        );
        let q = s.slide(Vec2::new(0.0, 3.0), Vec2::new(0.0, -6.0), 0.45);
        assert!(q.y > 0.94);
    }
}
