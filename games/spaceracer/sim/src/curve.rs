//! Godot 4.5 Curve3D even-length bake and cubic sample conventions, native Rust.
//! Adapted from scene/resources/curve.cpp; see licenses/Godot-MIT.txt.
use glam::Vec3;
#[derive(Clone, Debug)]
pub struct Curve {
    pub points: Vec<Vec3>,
    pub distances: Vec<f32>,
    pub length: f32,
}
fn bezier(p: [Vec3; 4], t: f32) -> Vec3 {
    let u = 1.0 - t;
    p[0] * (u * u * u) + p[1] * (3.0 * u * u * t) + p[2] * (3.0 * u * t * t) + p[3] * (t * t * t)
}
fn subdivide(p: [Vec3; 4], begin: f32, end: f32, depth: u32, out: &mut Vec<Vec3>) {
    if bezier(p, begin).distance(bezier(p, end)) > 2.0 && depth < 10 {
        let mid = (begin + end) * 0.5;
        subdivide(p, begin, mid, depth + 1, out);
        out.push(bezier(p, mid));
        subdivide(p, mid, end, depth + 1, out);
    }
}
impl Curve {
    pub fn bake(points: &[Vec3], tangents: &[Vec3]) -> Self {
        assert!(points.len() >= 2);
        assert_eq!(points.len(), tangents.len());
        let mut baked = vec![points[0]];
        for i in 0..points.len() - 1 {
            subdivide(
                [
                    points[i],
                    points[i] + tangents[i],
                    points[i + 1] - tangents[i + 1],
                    points[i + 1],
                ],
                0.0,
                1.0,
                0,
                &mut baked,
            );
            baked.push(points[i + 1]);
        }
        let mut distances = vec![0.0];
        for w in baked.windows(2) {
            distances.push(distances.last().unwrap() + w[0].distance(w[1]));
        }
        Self {
            length: *distances.last().unwrap(),
            points: baked,
            distances,
        }
    }
    pub fn sample(&self, offset: f32) -> Vec3 {
        let offset = offset.clamp(0.0, self.length);
        let i = self
            .distances
            .partition_point(|d| *d < offset)
            .saturating_sub(1)
            .min(self.points.len() - 2);
        let span = self.distances[i + 1] - self.distances[i];
        let t = if span < f32::EPSILON {
            0.5
        } else {
            (offset - self.distances[i]) / span
        };
        let a = self.points[i];
        let b = self.points[i + 1];
        let pre = self.points[i.saturating_sub(1)];
        let post = self.points[(i + 2).min(self.points.len() - 1)];
        (a * 2.0
            + (-pre + b) * t
            + (pre * 2.0 - a * 5.0 + b * 4.0 - post) * (t * t)
            + (-pre + a * 3.0 - b * 3.0 + post) * (t * t * t))
            * 0.5
    }
}
