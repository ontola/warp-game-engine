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
