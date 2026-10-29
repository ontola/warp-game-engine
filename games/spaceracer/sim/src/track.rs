//! Continuous ribbon geometry; seed generation is still being ported separately.
use crate::handling::{Surface, Zone};
use glam::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Geometry {
    pub p: Vec3,
    pub frame: [[f32; 3]; 3],
    pub width: f64,
    #[serde(default)]
    pub shape_angle: f64,
}
impl Geometry {
    pub fn basis(&self) -> Mat3 {
        Mat3::from_cols_array_2d(&self.frame)
    }
    pub fn position(&self, x: f64) -> Vec3 {
        let f = self.basis();
        if self.shape_angle < 0.0001 {
            return self.p - f.x_axis * x as f32;
        }
        let r = self.width / self.shape_angle;
        let a = x / r;
        self.p - f.x_axis * (r * a.sin()) as f32 + f.y_axis * (r * (1.0 - a.cos())) as f32
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    #[serde(flatten)]
    pub geometry: Geometry,
    pub heading: f64,
    pub slope: f64,
    pub crest: f64,
    pub curve: f64,
    pub bank: f64,
    pub split_gap: f64,
    pub dead_side: f64,
    pub dead_gap: bool,
    pub preferred_route: f64,
    pub air_gap: bool,
    pub rails: bool,
    pub feature: String,
    pub zone: String,
    pub tunnel: bool,
    #[serde(rename = "loop")]
    pub is_loop: bool,
    pub section: String,
    #[serde(default)]
    pub before: Option<Geometry>,
    #[serde(default)]
    pub after: Option<Geometry>,
}
impl Node {
    pub fn surface(&self) -> Surface {
        Surface {
            width: self.geometry.width,
            curve: self.curve,
            crest: self.crest,
            slope: self.slope,
            rails: self.rails,
            shape_angle: self.geometry.shape_angle,
            split_gap: self.split_gap,
            zone: match self.zone.as_str() {
                "boost" => Zone::Boost,
                "repair" => Zone::Repair,
                _ => Zone::None,
            },
        }
    }
    pub fn frame(&self, x: f64) -> Mat3 {
        let angle = self.geometry.shape_angle;
        let mut frame = self.geometry.basis();
        if angle < 0.0001 {
            return frame;
        }
        frame *= Mat3::from_rotation_z((-x / self.geometry.width * angle) as f32);
        if let (Some(a), Some(b)) = (&self.before, &self.after) {
            let along = b.position(x) - a.position(x);
            let forward = (along - frame.x_axis * along.dot(frame.x_axis)).normalize_or_zero();
            if forward.length_squared() > 0.9 {
                frame = Mat3::from_cols(
                    frame.x_axis,
                    forward.cross(frame.x_axis).normalize(),
                    forward,
                );
            }
        }
        frame
    }
    pub fn point(&self, x: f64, height: f64) -> Vec3 {
        self.geometry.position(x) + self.frame(x).y_axis * height as f32
    }
    pub fn lateral(&self, p: Vec3) -> f64 {
        let d = p - self.geometry.p;
        let f = self.geometry.basis();
        if self.geometry.shape_angle < 0.0001 {
            return -d.dot(f.x_axis) as f64;
        }
        let r = self.geometry.width / self.geometry.shape_angle;
        (-d.dot(f.x_axis) as f64).atan2(r - d.dot(f.y_axis) as f64) * r
    }
    pub fn supported(&self, x: f64, margin: f64) -> bool {
        !self.air_gap
            && !(self.dead_gap && x * self.dead_side > 0.0)
            && (self.geometry.shape_angle > std::f64::consts::PI - 0.001
                || x.abs() <= self.geometry.width - margin)
            && (self.split_gap < 0.01 || x.abs() > self.split_gap + margin)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub seed: i64,
    pub difficulty: String,
    pub length: f64,
    pub step: f64,
    pub nodes: Vec<Node>,
}
impl Track {
    pub fn sample(&self, d: f64) -> Node {
        let f = d.rem_euclid(self.length) / self.step;
        let i = f as usize;
        let t = f - i as f64;
        let a = &self.nodes[i];
        let b = &self.nodes[(i + 1) % self.nodes.len()];
        let mut out = a.clone();
        out.geometry.p = a.geometry.p.lerp(b.geometry.p, t as f32);
        out.geometry.frame = Mat3::from_quat(
            Quat::from_mat3(&a.geometry.basis())
                .slerp(Quat::from_mat3(&b.geometry.basis()), t as f32),
        )
        .to_cols_array_2d();
        macro_rules! blend {($($field:ident),*)=>{$(out.$field=a.$field+(b.$field-a.$field)*t;)*};}
        blend!(slope, crest, curve, bank, split_gap);
        out.geometry.width = a.geometry.width + (b.geometry.width - a.geometry.width) * t;
        out.geometry.shape_angle =
            a.geometry.shape_angle + (b.geometry.shape_angle - a.geometry.shape_angle) * t;
        let delta = (b.heading - a.heading + std::f64::consts::PI)
            .rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        out.heading = a.heading + delta * t;
        out.before = Some(a.geometry.clone());
        out.after = Some(b.geometry.clone());
        out
    }
    pub fn project(&self, p: Vec3, reference: f64, reach: f64) -> (f64, Node, f64) {
        let arc = |d: f64| {
            reference + (d - reference + self.length * 0.5).rem_euclid(self.length)
                - self.length * 0.5
        };
        let mut closest = 0;
        let mut best = f32::INFINITY;
        for i in (0..self.nodes.len()).step_by(8) {
            if (arc(i as f64 * self.step) - reference).abs() > reach {
                continue;
            }
            let d = p.distance_squared(self.nodes[i].geometry.p);
            if d < best {
                best = d;
                closest = i;
            }
        }
        let mut distance = reference;
        best = f32::INFINITY;
        for offset in -10..=10 {
            let i = (closest as isize + offset).rem_euclid(self.nodes.len() as isize) as usize;
            let a = self.nodes[i].geometry.p;
            let delta = self.nodes[(i + 1) % self.nodes.len()].geometry.p - a;
            let along = ((p - a).dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
            let d = arc((i as f64 + along as f64) * self.step);
            if (d - reference).abs() > reach {
                continue;
            }
            let sq = p.distance_squared(a + delta * along);
            if sq < best {
                best = sq;
                distance = d;
            }
        }
        let mut n = self.sample(distance);
        if n.geometry.shape_angle > 0.0001 {
            for _ in 0..6 {
                let x = n.lateral(p);
                let surface = n.geometry.position(x);
                let tangent = self.sample(distance + 0.5).geometry.position(x)
                    - self.sample(distance - 0.5).geometry.position(x);
                let correction = (((p - surface).dot(tangent) / tangent.length_squared().max(0.01))
                    as f64)
                    .clamp(-self.step * 2.0, self.step * 2.0);
                distance = (distance + correction).clamp(reference - reach, reference + reach);
                n = self.sample(distance);
                if correction.abs() < 0.001 {
                    break;
                }
            }
        }
        let x = n.lateral(p);
        (distance, n, x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_track_samples_all_six_layouts_and_three_difficulties() {
        let cases: serde_json::Value = serde_json::from_str(&crate::oracle("tracks")).unwrap();
        for case in cases.as_array().unwrap() {
            let track: Track = serde_json::from_value(case.clone()).unwrap();
            for sample in case["samples"].as_array().unwrap() {
                let d = sample["distance"].as_f64().unwrap();
                let n = track.sample(d);
                let expected: Node = serde_json::from_value(sample["node"].clone()).unwrap();
                assert!(n.geometry.p.distance(expected.geometry.p) < 0.003);
                for (a, b) in [
                    (n.curve, expected.curve),
                    (n.geometry.width, expected.geometry.width),
                    (n.slope, expected.slope),
                ] {
                    assert!((a - b).abs() < 0.00001);
                }
                let point: Vec3 = serde_json::from_value(sample["point"].clone()).unwrap();
                assert!(
                    n.point(8.0, 1.3).distance(point) < 0.003,
                    "seed {} difficulty {} distance {}",
                    track.seed,
                    track.difficulty,
                    d
                );
                let basis: [[f32; 3]; 3] =
                    serde_json::from_value(sample["surface"].clone()).unwrap();
                assert!(
                    (n.frame(8.0).x_axis - Mat3::from_cols_array_2d(&basis).x_axis).length()
                        < 0.0001
                );
            }
        }
    }
}
