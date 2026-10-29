//! Cached sky visibility and single diffuse sky return from static oriented boxes.
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
    fn contains(&self, p: Vec3) -> bool {
        if p.cmplt(self.lo).any() || p.cmpgt(self.hi).any() {
            return false;
        }
        if let Some((a, b)) = &self.children {
            return a.contains(p) || b.contains(p);
        }
        self.shapes.iter().any(|s| {
            s.inverse
                .transform_point3(p)
                .abs()
                .cmplt(Vec3::splat(0.499))
                .all()
        })
    }
    fn hit(&self, o: Vec3, d: Vec3) -> Option<(f32, Vec3)> {
        if !intersects(o, d, self.lo, self.hi) {
            return None;
        }
        if let Some((a, b)) = &self.children {
            return match (a.hit(o, d), b.hit(o, d)) {
                (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
                (a, b) => a.or(b),
            };
        }
        let mut best = None;
        for s in &self.shapes {
            let p = s.inverse.transform_point3(o);
            let v = s.inverse.transform_vector3(d);
            let (mut enter, mut exit) = (0.02f32, 1000f32);
            let mut normal = Vec3::ZERO;
            for axis in 0..3 {
                if v[axis].abs() < 1e-8 {
                    if p[axis].abs() > 0.5 {
                        exit = -1.;
                    }
                } else {
                    let a = (-0.5 - p[axis]) / v[axis];
                    let b = (0.5 - p[axis]) / v[axis];
                    let near = a.min(b);
                    if near > enter {
                        enter = near;
                        normal = Vec3::ZERO;
                        normal[axis] = -v[axis].signum();
                    }
                    exit = exit.min(a.max(b));
                }
            }
            if exit >= enter && normal.length_squared() > 0.0 && best.is_none_or(|(t, _)| enter < t)
            {
                best = Some((enter, (s.inverse.matrix3.transpose() * normal).normalize()));
            }
        }
        best
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
fn geometry(boxes: &[Affine3A]) -> Result<Vec<Shape>, String> {
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
    Ok(shapes)
}
/// Experimental nearest-surface separation planes, cached with static geometry.
/// Not full DDGI visibility: one plane cannot represent corners or openings.
/// xyz points into the probe's half-space; w is the plane offset. Zero = no wall.
pub fn bake_separation_planes(grid: &Grid, boxes: &[Affine3A]) -> Result<Vec<[f32;4]>, String> {
    grid.validate()?;
    let shapes=geometry(boxes)?;
    if shapes.is_empty() { return Ok(vec![[0.0;4];grid.count()]); }
    let tree=Node::build(shapes);
    let directions:Vec<Vec3>=(-1..=1).flat_map(|x| (-1..=1).flat_map(move |y| (-1..=1).filter_map(move |z| {
        let d=Vec3::new(x as f32,y as f32,z as f32);
        (d.length_squared()>0.0).then(||d.normalize())
    }))).collect();
    Ok((0..grid.count()).map(|i| {
        let p=grid.position(i);
        if tree.contains(p) { return [0.0;4]; }
        let mut nearest=grid.cell*1.75;
        let mut plane=[0.0;4];
        for &d in &directions {
            if let Some((t,n))=tree.hit(p,d) {
                if t<nearest && n.dot(d)<0.0 {
                    nearest=t;
                    plane=[n.x,n.y,n.z,-n.dot(p+d*t)];
                }
            }
        }
        plane
    }).collect())
}

/// Cosine-weighted visibility on six axes. Boxes are transformed unit cubes.
pub fn bake(grid: &Grid, boxes: &[Affine3A], rays: usize) -> Result<Vec<Probe>, String> {
    grid.validate()?;
    if rays < 16 {
        return Err("sky visibility needs at least 16 rays".into());
    }
    let shapes = geometry(boxes)?;
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
/// Unit-environment transport: R=direct visibility, G=one diffuse sky bounce,
/// B=probe validity. Reflectance is a neutral proxy for the supplied material.
/// Invalid probes inside solids are excluded from both CPU and GPU interpolation.
pub fn bake_diffuse(
    grid: &Grid,
    boxes: &[Affine3A],
    rays: usize,
    reflectance: f32,
) -> Result<Vec<Probe>, String> {
    if !reflectance.is_finite() || !(0.0..=1.0).contains(&reflectance) {
        return Err("invalid reflectance".into());
    }
    let mut sky = bake(grid, boxes, rays)?;
    if boxes.is_empty() {
        return Ok(vec![
            Probe {
                axes: [Vec3::new(1., 0., 1.); 6]
            };
            grid.count()
        ]);
    }
    let tree = Node::build(geometry(boxes)?);
    for (i, p) in sky.iter_mut().enumerate() {
        let valid = if tree.contains(grid.position(i)) {
            0.0
        } else {
            1.0
        };
        for a in &mut p.axes {
            a.z = valid;
        }
    }
    let directions: Vec<Vec3> = (0..rays)
        .map(|i| {
            let y = 1. - 2. * (i as f32 + 0.5) / rays as f32;
            let a = i as f32 * 2.3999632;
            let r = (1. - y * y).sqrt();
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
        std::array::from_fn(|a| directions.iter().map(|d| axes[a].dot(*d).max(0.)).sum());
    let mut result = sky.clone();
    let chunk = grid.count().div_ceil(4);
    std::thread::scope(|scope| {
        for (group, out) in result.chunks_mut(chunk).enumerate() {
            let tree = &tree;
            let sky = &sky;
            let directions = &directions;
            scope.spawn(move || {
                for (local, p) in out.iter_mut().enumerate() {
                    if p.axes[0].z == 0. {
                        continue;
                    }
                    let origin = grid.position(group * chunk + local);
                    let mut bounce = [0.; 6];
                    for d in directions {
                        if let Some((t, n)) = tree.hit(origin, *d) {
                            let e = sample_valid(grid, sky, origin + *d * t + n * 0.15, n);
                            for a in 0..6 {
                                bounce[a] += reflectance * e * axes[a].dot(*d).max(0.);
                            }
                        }
                    }
                    for a in 0..6 {
                        p.axes[a].y = (bounce[a] / denom[a]).clamp(0., 1.);
                    }
                }
            });
        }
    });
    Ok(result)
}
/// Add one diffuse sun bounce to a sky cache. Z is sun transfer, or 2 for
/// embedded probes. Recompute on sun direction changes, not colour/intensity.
/// Shadow tests use static scene geometry, independent of camera/view range.
pub fn bake_sun(
    grid: &Grid,
    boxes: &[Affine3A],
    sky: &[Probe],
    rays: usize,
    reflectance: f32,
    sun: Vec3,
) -> Result<Vec<Probe>, String> {
    grid.validate()?;
    if sky.len() != grid.count()
        || rays < 8
        || !sun.is_finite()
        || sun.length_squared() < 0.5
        || !reflectance.is_finite()
        || !(0.0..=1.0).contains(&reflectance)
    {
        return Err("invalid sun transport inputs".into());
    }
    let mut result = sky.to_vec();
    let tree = Node::build(geometry(boxes)?);
    let sun = sun.normalize();
    let axes = [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ];
    let directions: Vec<Vec3> = (0..rays)
        .map(|i| {
            let y = 1. - 2. * (i as f32 + 0.5) / rays as f32;
            let a = i as f32 * 2.3999632;
            let r = (1. - y * y).sqrt();
            Vec3::new(r * a.cos(), y, r * a.sin())
        })
        .collect();
    let denom: [f32; 6] =
        std::array::from_fn(|a| directions.iter().map(|d| axes[a].dot(*d).max(0.)).sum());
    let chunk = grid.count().div_ceil(4);
    std::thread::scope(|scope| {
        for (group, out) in result.chunks_mut(chunk).enumerate() {
            let tree = &tree;
            let directions = &directions;
            scope.spawn(move || {
                for (local, p) in out.iter_mut().enumerate() {
                    if p.axes[0].z == 0. {
                        for a in &mut p.axes {
                            a.z = 2.;
                        }
                        continue;
                    }
                    let origin = grid.position(group * chunk + local);
                    let mut returned = [0.; 6];
                    if sun.y > 0. {
                        for d in directions {
                            if let Some((t, n)) = tree.hit(origin, *d) {
                                let cosine = n.dot(sun).max(0.);
                                if cosine > 0. && !tree.blocked(origin + *d * t + n * 0.03, sun) {
                                    for a in 0..6 {
                                        returned[a] +=
                                            reflectance * cosine * axes[a].dot(*d).max(0.);
                                    }
                                }
                            }
                        }
                    }
                    for a in 0..6 {
                        p.axes[a].z = returned[a] / denom[a];
                    }
                }
            });
        }
    });
    Ok(result)
}
fn sample_valid(grid: &Grid, probes: &[Probe], p: Vec3, n: Vec3) -> f32 {
    let max = Vec3::new(
        grid.dims[0] as f32 - 1.,
        grid.dims[1] as f32 - 1.,
        grid.dims[2] as f32 - 1.,
    );
    let f = ((p - grid.origin) / grid.cell).clamp(Vec3::ZERO, max);
    let base = f.floor().min(max - Vec3::ONE);
    let t = f - base;
    let mut value = 0.;
    let mut weight = 0.;
    for c in 0..8 {
        let o = [c & 1, (c >> 1) & 1, (c >> 2) & 1];
        let w = (if o[0] == 1 { t.x } else { 1. - t.x })
            * (if o[1] == 1 { t.y } else { 1. - t.y })
            * (if o[2] == 1 { t.z } else { 1. - t.z });
        let i = base.x as usize
            + o[0]
            + (base.y as usize + o[1]) * grid.dims[0] as usize
            + (base.z as usize + o[2]) * (grid.dims[0] * grid.dims[1]) as usize;
        let valid = probes[i].axes[0].z;
        value += probes[i].irradiance(n).x * w * valid;
        weight += w * valid;
    }
    if weight > 1e-5 { value / weight } else { 0. }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separation_plane_rejects_other_side_of_thin_wall() {
        let grid=Grid { origin:Vec3::new(-1.0,0.0,0.0),cell:2.0,dims:[2,2,2] };
        let wall=Affine3A::from_scale_rotation_translation(Vec3::new(0.2,20.0,20.0),glam::Quat::from_rotation_y(0.2),Vec3::ZERO);
        let planes=bake_separation_planes(&grid,&[wall]).unwrap();
        for i in 0..2 {
            let a=planes[i]; let n=Vec3::new(a[0],a[1],a[2]);
            assert!(n.dot(grid.position(i))+a[3]>0.5);
            assert!(n.dot(grid.position(1-i))+a[3]< -0.5);
        }
        assert_eq!(bake_separation_planes(&grid,&[]).unwrap()[0],[0.0;4]);
    }
    #[test]
    fn sun_return_is_shadowed_and_night_is_zero() {
        let grid = Grid {
            origin: Vec3::new(0., 1., 0.),
            cell: 1.,
            dims: [2, 2, 2],
        };
        let floor = Affine3A::from_scale_rotation_translation(
            Vec3::new(100., 0.5, 100.),
            glam::Quat::IDENTITY,
            Vec3::ZERO,
        );
        let sky = bake_diffuse(&grid, &[floor], 64, 0.7).unwrap();
        let day = bake_sun(&grid, &[floor], &sky, 64, 0.7, Vec3::Y).unwrap();
        assert!(
            day[0].axes[3].z > 0.6,
            "sunlit floor must return sunlight upward"
        );
        let roof = Affine3A::from_scale_rotation_translation(
            Vec3::new(200., 0.5, 200.),
            glam::Quat::IDENTITY,
            Vec3::Y * 4.,
        );
        let blocked = bake_sun(&grid, &[floor, roof], &sky, 64, 0.7, Vec3::Y).unwrap();
        let night = bake_sun(&grid, &[floor], &sky, 64, 0.7, Vec3::NEG_Y).unwrap();
        assert!(
            blocked[0].axes[3].z < 0.001,
            "roof must block the floor's sun source"
        );
        assert_eq!(night[0].axes[3].z, 0.);
        assert_eq!(day[0].axes[3].x, sky[0].axes[3].x);
        crate::irradiance_volume::pack(&grid, &day).unwrap();
    }
    #[test]
    fn diffuse_return_depends_on_material_and_rejects_embedded_probes() {
        let grid = Grid {
            origin: Vec3::new(0., 1., 0.),
            cell: 2.,
            dims: [3, 3, 3],
        };
        let roof = Affine3A::from_scale_rotation_translation(
            Vec3::new(5., 0.5, 5.),
            glam::Quat::IDENTITY,
            Vec3::new(0., 3., 0.),
        );
        let floor = Affine3A::from_scale_rotation_translation(
            Vec3::new(12., 0.5, 12.),
            glam::Quat::IDENTITY,
            Vec3::new(0., 0., 0.),
        );
        let dark = bake_diffuse(&grid, &[roof, floor], 64, 0.).unwrap();
        let pale = bake_diffuse(&grid, &[roof, floor], 64, 0.7).unwrap();
        assert_eq!(
            pale[3].axes[0].z, 0.,
            "probe in the roof must not contribute"
        );
        assert_eq!(pale[0].axes[0].z, 1.);
        assert!(pale[0].axes[3].y > 0.01, "floor must return some sky light");
        for (a, b) in dark.iter().zip(&pale) {
            for i in 0..6 {
                if a.axes[i].z > 0. {
                    assert_eq!(
                        a.axes[i].x, b.axes[i].x,
                        "material must not change direct visibility"
                    );
                    assert_eq!(a.axes[i].y, 0.);
                    assert!(b.axes[i].y >= 0. && b.axes[i].y <= 0.7);
                }
            }
        }
    }
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
