//! Geometry shared by render meshes and fitted vehicle hit queries.
use glam::Vec3;

/// A vertex on a clipped triangle. Render meshes also interpolate UVs and normals.
pub trait Vertex: Copy {
    fn position(self) -> Vec3;
    fn on_plane(self, other: Self, axis: usize, plane: f32) -> Self;
}

impl Vertex for Vec3 {
    fn position(self) -> Vec3 { self }
    fn on_plane(self, other: Self, axis: usize, plane: f32) -> Self {
        let mut p = self.lerp(other, (plane - self[axis]) / (other[axis] - self[axis]));
        p[axis] = plane;
        p
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid(pub [f32; 4]);

impl Grid {
    pub fn valid(self) -> bool {
        let [l, r, f, b] = self.0;
        self.0.iter().all(|c| c.is_finite()) && l < r && f < b
    }

    pub fn hinge(self, cell: usize) -> Vec3 {
        let [l, r, f, b] = self.0;
        Vec3::new([l, 0., r][cell % 3], 0., [f, 0., b][cell / 3])
    }

    /// Reuse two scratch buffers across triangles. Most triangles belong wholly
    /// to one cell, and that common case does not touch either buffer.
    pub fn split<V: Vertex>(self, tri: [V; 3], poly: &mut Vec<V>, work: &mut Vec<V>, mut emit: impl FnMut(usize, [V; 3])) {
        let [l, r, f, b] = self.0;
        let band = |p: f32, lo, hi| if p < lo { 0 } else if p <= hi { 1 } else { 2 };
        let [a, b0, c] = tri.map(Vertex::position);
        let lo = a.min(b0).min(c);
        let hi = a.max(b0).max(c);
        let (x0,x1) = (band(lo.x,l,r),band(hi.x,l,r));
        let (z0,z1) = (band(lo.z,f,b),band(hi.z,f,b));
        if x0 == x1 && z0 == z1 { emit(z0 * 3 + x0, tri); return; }
        for z in z0..=z1 {
            for x in x0..=x1 {
                poly.clear();
                poly.extend_from_slice(&tri);
                for (axis, plane, high, needed) in [
                    (0,l,x != 0,x != 2), (0,r,x == 2,x != 0),
                    (2,f,z != 0,z != 2), (2,b,z == 2,z != 0),
                ] {
                    if !needed { continue; }
                    let side = |v: V| if high { v.position()[axis] >= plane } else { v.position()[axis] <= plane };
                    if poly.iter().all(|&v| side(v)) { continue; }
                    let Some(mut prev) = poly.last().copied() else { break; };
                    work.clear();
                    for &next in poly.iter() {
                        if side(prev) != side(next) {
                            // Both adjacent faces compute the same shared edge vertex.
                            let (low,high) = if prev.position()[axis] < next.position()[axis] { (prev,next) } else { (next,prev) };
                            work.push(low.on_plane(high,axis,plane));
                        }
                        if side(next) { work.push(next); }
                        prev=next;
                    }
                    std::mem::swap(poly,work);
                }
                for i in 2..poly.len() {
                    let t=[poly[0],poly[i-1],poly[i]];
                    let [p,q,r]=t.map(Vertex::position);
                    if (q-p).cross(r-p).length_squared()>1e-18 { emit(z*3+x,t); }
                }
            }
        }
    }
}

/// Y shear folds a compressed panel upward around the same face as its XZ
/// compression. Points along shared faces therefore stay welded.
pub fn slope(hinge: Vec3, scale: Vec3) -> Vec3 {
    let tilt = |h: f32, s: f32| if h == 0. { 0. } else { h.signum() * (1. - s).max(0.) };
    Vec3::new(0.30 * tilt(hinge.x, scale.x), 0., 0.42 * tilt(hinge.z, scale.z))
}

pub fn deform(p: Vec3, hinge: Vec3, scale: Vec3) -> Vec3 {
    hinge + scale * (p - hinge) + Vec3::Y * slope(hinge,scale).dot(p-hinge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipped_cells_preserve_area_and_deform_to_a_shared_seam() {
        let grid=Grid([-0.45,0.45,-0.8,0.7]);
        let tri=[Vec3::new(-1.,0.4,-2.),Vec3::new(1.,0.4,-2.),Vec3::new(0.,0.4,2.)];
        let mut area=0.;
        let mut cells=std::collections::BTreeSet::new();
        grid.split(tri,&mut Vec::new(),&mut Vec::new(),|cell,[p,q,r]| {
            area+=(q-p).cross(r-p).length()*0.5;
            cells.insert(cell);
        });
        assert!((area-4.).abs()<1e-5 && cells.len()>=7);
        let seam=Vec3::new(-0.45,0.4,-1.7);
        let left=deform(seam,grid.hinge(0),Vec3::new(0.5,1.,0.4));
        let center=deform(seam,grid.hinge(1),Vec3::new(1.,1.,0.4));
        assert!(left.distance(center)<1e-6 && left.y>seam.y && left.z>seam.z);
    }
}

pub const MAX_DENTS: usize = 8;
/// The continuous displacement limit also pads render and LOD culling spheres.
pub const DENT_ENVELOPE: f32 = 0.68;

/// A compact body-space impact. Axis points into the material. The non-spherical
/// footprint and the shallow raised rim give the metal a local fold in silhouette.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dent {
    pub center: Vec3,
    axis: Vec3,
    diagonal: Vec3,
    inv_radius: f32,
    depth: f32,
}

impl Dent {
    pub fn new(center: Vec3, inward: Vec3, radius: f32, depth: f32) -> Option<Self> {
        if !center.is_finite() || !inward.is_finite() || !(0.12..=3.).contains(&radius) || !(0.0..=0.6).contains(&depth) { return None; }
        let axis = inward.try_normalize()?;
        let across = Vec3::Y.cross(axis).try_normalize().unwrap_or(Vec3::X);
        let diagonal = (across * 0.91 + axis.cross(across) * 0.42).normalize();
        Some(Self { center, axis, diagonal, inv_radius: radius.recip(), depth })
    }

    /// Compact polynomial field plus its analytic displacement Jacobian.
    fn sample(self, point: Vec3, d: &mut Vec3, jac: &mut glam::Mat3) {
        let p = (point - self.center) * self.inv_radius;
        // Narrower through the panel and vertically. The far side and wheels stay local.
        let a = self.axis.dot(p);
        let g = p + self.axis * (0.8 * a) + Vec3::Y * (0.40 * p.y);
        let q = p.dot(g);
        if q >= 1. { return; }
        let edge = 1. - q;
        let x = self.diagonal.dot(p);
        let skew = 1. + 0.36 * x;
        let core = edge * edge * (1. - 2.35 * q);
        let dc = -2. * edge * (1. - 2.35 * q) - 2.35 * edge * edge;
        let shell = q * edge * edge;
        let ridge = Vec3::Y * 0.42 + self.diagonal * 0.24;
        let dq = g * (2. * self.inv_radius);
        let depth = self.depth;
        *d += (self.axis * core * skew + ridge * shell) * depth;
        let gc = dq * (dc * skew) + self.diagonal * (0.36 * self.inv_radius * core);
        let gs = dq * (edge * (1. - 3. * q));
        for (col, c, s) in [(&mut jac.x_axis,gc.x,gs.x),(&mut jac.y_axis,gc.y,gs.y),(&mut jac.z_axis,gc.z,gs.z)] {
            *col += (self.axis * c + ridge * s) * depth;
        }
    }
}

/// Apply all dents in rest space. The derivative of the saturating displacement
/// is included in the cofactor normal, preserving the paint's authored shading.
pub fn dent_vertex(point: Vec3, normal: Vec3, dents: &[Dent]) -> (Vec3, Vec3) {
    let mut offset = Vec3::ZERO;
    let mut deriv = glam::Mat3::ZERO;
    for dent in dents { dent.sample(point, &mut offset, &mut deriv); }
    if offset == Vec3::ZERO { return (point,normal); }
    let e2 = DENT_ENVELOPE * DENT_ENVELOPE;
    let k = (1. + offset.length_squared() / e2).sqrt().recip();
    let column = |d: Vec3| d*k-offset*(offset.dot(d)*k*k*k/e2);
    let dx=Vec3::X+column(deriv.x_axis);
    let dy=Vec3::Y+column(deriv.y_axis);
    let dz=Vec3::Z+column(deriv.z_axis);
    let n=dy.cross(dz)*normal.x+dz.cross(dx)*normal.y+dx.cross(dy)*normal.z;
    (point+offset*k,n.normalize_or(normal))
}

#[cfg(test)]
mod dent_tests {
    use super::*;
    #[test]
    fn local_dents_tilt_paint_normals_and_leave_the_other_corner_at_rest() {
        let d=Dent::new(Vec3::new(-0.55,0.45,-1.93),Vec3::new(0.25,-0.12,1.),1.05,0.46).unwrap();
        let p=d.center+Vec3::new(0.31,0.04,0.17);
        let (m,n)=dent_vertex(p,Vec3::Y,&[d]);
        let x=p+Vec3::X*0.0003; let z=p+Vec3::Z*0.0003;
        let tx=dent_vertex(x,Vec3::Y,&[d]).0-m; let tz=dent_vertex(z,Vec3::Y,&[d]).0-m;
        assert!(n.dot(tz.cross(tx).normalize())>0.999 && Vec3::Y.dot(n)<0.999);
        assert!(m.z>p.z+0.1);
        let far=d.center+Vec3::X*1.45;
        assert_eq!(dent_vertex(far,Vec3::Y,&[d]),(far,Vec3::Y));
        assert!(dent_vertex(p,Vec3::Y,&[d;MAX_DENTS]).0.distance(p)<=DENT_ENVELOPE);
    }
}
