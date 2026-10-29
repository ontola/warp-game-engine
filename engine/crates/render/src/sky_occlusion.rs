//! Cached ambient visibility from static oriented boxes. Geometry only, not GI.
use crate::irradiance_volume::{Grid, Probe};
use glam::{Affine3A, Vec3};
#[derive(Clone)]
struct Shape {
    inverse: Affine3A,
    lo: Vec3,
    hi: Vec3,
}
struct Node {
    lo: Vec3,
    hi: Vec3,
    children: Option<(Box<Node>, Box<Node>)>,
    shapes: Vec<Shape>,
}
fn intersects(o: Vec3, d: Vec3, lo: Vec3, hi: Vec3) -> bool {
    let (mut enter, mut exit) = (0.02f32, 1000.0f32);
    for a in 0..3 {
        if d[a].abs() < 1e-8 {
            if o[a] < lo[a] || o[a] > hi[a] {
                return false;
            }
        } else {
            let x = (lo[a] - o[a]) / d[a];
            let y = (hi[a] - o[a]) / d[a];
            enter = enter.max(x.min(y));
            exit = exit.min(x.max(y));
        }
    }
    exit >= enter
}
impl Node {
    fn build(mut shapes: Vec<Shape>) -> Self {
        let lo = shapes
            .iter()
            .fold(Vec3::splat(f32::INFINITY), |a, b| a.min(b.lo));
        let hi = shapes
            .iter()
            .fold(Vec3::splat(f32::NEG_INFINITY), |a, b| a.max(b.hi));
        let children = if shapes.len() > 8 {
            let s = hi - lo;
            let axis = if s.x >= s.y && s.x >= s.z {
                0
            } else if s.y >= s.z {
                1
            } else {
                2
            };
            shapes.sort_by(|a, b| (a.lo[axis] + a.hi[axis]).total_cmp(&(b.lo[axis] + b.hi[axis])));
            let right = shapes.split_off(shapes.len() / 2);
            Some((
                Box::new(Self::build(std::mem::take(&mut shapes))),
                Box::new(Self::build(right)),
            ))
        } else {
            None
        };
        Self {
            lo,
            hi,
            children,
            shapes,
        }
    }
    fn blocked(&self, o: Vec3, d: Vec3) -> bool {
        if !intersects(o, d, self.lo, self.hi) {
            return false;
        }
        if let Some((a, b)) = &self.children {
            return a.blocked(o, d) || b.blocked(o, d);
        }
        self.shapes.iter().any(|s| {
            intersects(
                s.inverse.transform_point3(o),
                s.inverse.transform_vector3(d),
                Vec3::splat(-0.5),
                Vec3::splat(0.5),
            )
        })
    }
}
/// Cosine-weighted visibility on six axes. Boxes are transformed unit cubes.
pub fn bake(grid: &Grid, boxes: &[Affine3A], rays: usize) -> Result<Vec<Probe>, String> {
    grid.validate()?;
    if rays < 16 {
        return Err("sky visibility needs at least 16 rays".into());
    }
    let mut shapes = Vec::new();
    for xf in boxes {
        if !xf.is_finite() || xf.matrix3.determinant().abs() < 1e-8 {
            return Err("invalid occluder transform".into());
        }
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = -lo;
        for i in 0..8 {
            let p = xf.transform_point3(Vec3::new(
                (i & 1) as f32 - 0.5,
                ((i >> 1) & 1) as f32 - 0.5,
                ((i >> 2) & 1) as f32 - 0.5,
            ));
            lo = lo.min(p);
            hi = hi.max(p);
        }
        shapes.push(Shape {
            inverse: xf.inverse(),
            lo,
            hi,
        });
    }
    if shapes.is_empty() {
        return Ok(vec![
            Probe {
                axes: [Vec3::ONE; 6]
            };
            grid.count()
        ]);
    }
    let tree = Node::build(shapes);
    let directions: Vec<Vec3> = (0..rays)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / rays as f32;
            let a = i as f32 * 2.3999632;
            let r = (1.0 - y * y).sqrt();
            Vec3::new(r * a.cos(), y, r * a.sin())
        })
        .collect();
    let axes = [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ];
    let denom: [f32; 6] =
        std::array::from_fn(|a| directions.iter().map(|d| axes[a].dot(*d).max(0.0)).sum());
    let mut probes = vec![Probe::default(); grid.count()];
    let chunk = grid.count().div_ceil(4);
    std::thread::scope(|scope| {
        for (group, out) in probes.chunks_mut(chunk).enumerate() {
            let tree = &tree;
            let directions = &directions;
            scope.spawn(move || {
                for (local, p) in out.iter_mut().enumerate() {
                    let origin = grid.position(group * chunk + local);
                    let mut v = [0.0; 6];
                    for d in directions {
                        if !tree.blocked(origin, *d) {
                            for a in 0..6 {
                                v[a] += axes[a].dot(*d).max(0.0);
                            }
                        }
                    }
                    p.axes =
                        std::array::from_fn(|a| Vec3::splat((v[a] / denom[a]).clamp(0.0, 1.0)));
                }
            });
        }
    });
    Ok(probes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roof_blocks_upward_light_and_edge_is_brighter() {
        let grid = Grid {
            origin: Vec3::new(0., 1., 0.),
            cell: 3.,
            dims: [2, 2, 2],
        };
        let roof = Affine3A::from_scale_rotation_translation(
            Vec3::new(5., 0.3, 5.),
            glam::Quat::IDENTITY,
            Vec3::new(0., 3., 0.),
        );
        let p = bake(&grid, &[roof], 256).unwrap();
        assert!(p[0].axes[2].x < p[1].axes[2].x);
        assert!(p[0].axes[2].x < 0.5);
        assert!(p[0].axes[3].x > 0.99);
        assert_eq!(bake(&grid, &[], 32).unwrap()[0].axes, [Vec3::ONE; 6]);
        assert!(intersects(
            Vec3::ZERO,
            Vec3::Y,
            Vec3::splat(-1.),
            Vec3::splat(1.)
        ));
        assert!(!intersects(
            Vec3::X * 2.,
            Vec3::Y,
            Vec3::splat(-1.),
            Vec3::splat(1.)
        ));
    }
}
