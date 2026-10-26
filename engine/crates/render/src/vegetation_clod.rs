//! Source-derived CLOD for small closed blade/needle primitives.
//!
//! Generic cluster builders stop once a primitive fits in one cluster. These
//! closed, triangular cross-section sweeps instead remove interior rings, not
//! plants or needles. The original source cluster remains byte-identical in
//! its geometry. Each coarse level keeps the root cap, tip, material and UVs,
//! and supplies geometric face normals. Unknown topology is left unchanged.
//!
//! Parent errors are conservative two-way surface-distance bounds. They include
//! a bounded adaptive triangle-cover residual, not just vertex samples. The
//! existing instance operator bound converts that error into world space.
//! A thirteen-direction projected-area guard rejects large coverage loss; it
//! is an additional guard, not proof of optical/lighting equivalence.

use crate::{cluster::{ClusterMesh, GpuCluster, Part, SimplificationPolicy, Vertex},
    foliage_voxel::PROJECTION_DIRECTIONS};
use glam::{DVec3, Mat4, Vec3};
use std::{cmp::Ordering, collections::{BTreeMap, BTreeSet, BinaryHeap}};

const MAX_RING_COUNT: usize = 8;
const MAX_DISTANCE_SPLITS: usize = 8192;
const MIN_AREA_RATIO: f32 = 0.90;
const MAX_AREA_RATIO: f32 = 1.10;
// Positional error alone can simplify a very thin needle while its face
// shading is still readable. Keep its finest shape above roughly 12.5 pixels
// of longitudinal extent; successive reductions need a smaller footprint.
const MIN_DETAIL_ERROR_FRACTION: f32 = 0.08;

type Triangle = [Vec3; 3];

struct Sweep {
    points: Vec<Vertex>,
    rings: Vec<[u32; 3]>,
    tip: u32,
}

fn position_key(p: [f32; 3]) -> [u32; 3] { p.map(|v| if v == 0.0 { 0 } else { v.to_bits() }) }
fn oriented_key(t: [u32; 3]) -> [u32; 3] {
    let start = if t[0] <= t[1] && t[0] <= t[2] { 0 } else if t[1] <= t[2] { 1 } else { 2 };
    [t[start], t[(start + 1) % 3], t[(start + 2) % 3]]
}
fn side(a: [u32; 3], b: [u32; 3]) -> Vec<[u32; 3]> {
    (0..3).flat_map(|j| { let k=(j+1)%3; [[a[j],b[j],b[k]],[a[j],b[k],a[k]]] }).collect()
}

fn recognize(part: &Part<'_>) -> Option<Sweep> {
    if part.indices.is_empty() || part.indices.len()%3 != 0 || part.indices.len()/3 > 128 { return None; }
    let mut points = Vec::<Vertex>::new();
    let mut map = BTreeMap::new();
    let mut remap = Vec::new();
    for &v in part.vertices {
        if !Vec3::from(v.pos).is_finite() || !Vec3::from(v.normal).is_finite() { return None; }
        let key=position_key(v.pos);
        let id=*map.entry(key).or_insert_with(|| { let id=points.len() as u32; points.push(v); id });
        remap.push(id);
    }
    let mut triangles=BTreeSet::new();
    for tri in part.indices.chunks_exact(3) {
        let t=[*remap.get(tri[0]as usize)?,*remap.get(tri[1]as usize)?,*remap.get(tri[2]as usize)?];
        if t[0]==t[1] || t[0]==t[2] || t[1]==t[2] || !triangles.insert(oriented_key(t)) { return None; }
    }
    let mut heights:Vec<f32>=points.iter().map(|p|p.pos[1]).collect();
    heights.sort_by(f32::total_cmp); heights.dedup();
    if !(4..=MAX_RING_COUNT+1).contains(&heights.len()) { return None; }
    let stations:Vec<Vec<u32>>=heights.iter().map(|&y| points.iter().enumerate().filter(|(_,p)|p.pos[1]==y).map(|(i,_)|i as u32).collect()).collect();
    if stations.last()?.len()!=1 || stations[..stations.len()-1].iter().any(|s|s.len()!=3) { return None; }
    let base=&stations[0];
    let root=*triangles.iter().find(|t|t.iter().all(|v|base.contains(v)))?;
    let mut rings=vec![root];
    for station in &stations[1..stations.len()-1] {
        let mut matched=None;
        for order in [[0,1,2],[0,2,1],[1,0,2],[1,2,0],[2,0,1],[2,1,0]] {
            let candidate=order.map(|i|station[i]);
            if side(*rings.last()?,candidate).iter().all(|&t|triangles.contains(&oriented_key(t))) {
                matched=Some(candidate);break;
            }
        }
        rings.push(matched?);
    }
    let tip=stations.last()?[0];
    let mut expected=BTreeSet::from([oriented_key(root)]);
    for pair in rings.windows(2) { expected.extend(side(pair[0],pair[1]).into_iter().map(oriented_key)); }
    for j in 0..3 { expected.insert(oriented_key([rings.last()?[j],tip,rings.last()?[(j+1)%3]])); }
    if triangles!=expected { return None; }
    Some(Sweep { points,rings,tip })
}

fn reduced(sweep:&Sweep, rings:&[usize]) -> Vec<Vertex> {
    let mut faces=vec![sweep.rings[0]];
    for pair in rings.windows(2) { faces.extend(side(sweep.rings[pair[0]],sweep.rings[pair[1]])); }
    let last=sweep.rings[*rings.last().unwrap()];
    for j in 0..3 { faces.push([last[j],sweep.tip,last[(j+1)%3]]); }
    let mut vertices=Vec::with_capacity(faces.len()*3);
    for face in faces {
        let [a,b,c]=face.map(|i|Vec3::from(sweep.points[i as usize].pos));
        let normal=(b-a).cross(c-a).normalize();
        for id in face { let mut v=sweep.points[id as usize];v.normal=normal.to_array();vertices.push(v); }
    }
    vertices
}
fn triangles(vertices:&[Vertex],indices:&[u32]) -> Vec<Triangle> {
    indices.chunks_exact(3).map(|t|[t[0],t[1],t[2]].map(|i|Vec3::from(vertices[i as usize].pos))).collect()
}
fn area(triangles:&[Triangle],direction:Vec3) -> f32 {
    triangles.iter().map(|[a,b,c]|(b-a).cross(c-a).dot(direction).abs()*0.25).sum()
}
fn coverage_guard(source:&[Triangle],coarse:&[Triangle]) -> bool {
    PROJECTION_DIRECTIONS.iter().all(|&d| {
        let d=Vec3::from(d);let a=area(source,d);let b=area(coarse,d);
        if a<1e-10 { b<1e-10 } else { b>=a*MIN_AREA_RATIO && b<=a*MAX_AREA_RATIO }
    })
}

// Closest point on a nondegenerate triangle, including edges/vertices.
fn point_distance(p:Vec3,[a,b,c]:Triangle) -> f32 {
    let ab=b-a;let ac=c-a;let ap=p-a;let d1=ab.dot(ap);let d2=ac.dot(ap);
    if d1<=0. && d2<=0. { return ap.length(); }
    let bp=p-b;let d3=ab.dot(bp);let d4=ac.dot(bp);
    if d3>=0. && d4<=d3 { return bp.length(); }
    let vc=d1*d4-d3*d2;
    if vc<=0. && d1>=0. && d3<=0. { return (p-(a+ab*(d1/(d1-d3)))).length(); }
    let cp=p-c;let d5=ab.dot(cp);let d6=ac.dot(cp);
    if d6>=0. && d5<=d6 { return cp.length(); }
    let vb=d5*d2-d1*d6;
    if vb<=0. && d2>=0. && d6<=0. { return (p-(a+ac*(d2/(d2-d6)))).length(); }
    let va=d3*d6-d5*d4;
    if va<=0. && d4-d3>=0. && d5-d6>=0. { return (p-(b+(c-b)*((d4-d3)/((d4-d3)+(d5-d6))))).length(); }
    let denominator=1./(va+vb+vc);
    (p-(a+ab*(vb*denominator)+ac*(vc*denominator))).length()
}
fn distance(p:Vec3,surface:&[Triangle]) -> f32 { surface.iter().map(|&t|point_distance(p,t)).fold(f32::INFINITY,f32::min) }

#[derive(Clone)]
struct Cell { triangle:Triangle,upper:f32 }
impl PartialEq for Cell { fn eq(&self,other:&Self)->bool {self.upper==other.upper} }
impl Eq for Cell {}
impl PartialOrd for Cell { fn partial_cmp(&self,other:&Self)->Option<Ordering> {Some(self.cmp(other))} }
impl Ord for Cell { fn cmp(&self,other:&Self)->Ordering {self.upper.total_cmp(&other.upper)} }
fn cell(triangle:Triangle,target:&[Triangle],lower:&mut f32) -> Cell {
    let center=(triangle[0]+triangle[1]+triangle[2])/3.;let d=distance(center,target);
    *lower=lower.max(d);
    // Every point in the triangle is within this radius of its centroid;
    // distance to a surface is 1-Lipschitz. Subdivision tightens the bound.
    let radius=triangle.iter().map(|p|p.distance(center)).fold(0.,f32::max);
    Cell {triangle,upper:d+radius}
}
fn directed_bound(source:&[Triangle],target:&[Triangle],tolerance:f32) -> f32 {
    let mut lower=source.iter().flatten().map(|&p|distance(p,target)).fold(0.,f32::max);
    let mut cells:BinaryHeap<_>=source.iter().map(|&t|cell(t,target,&mut lower)).collect();
    for _ in 0..MAX_DISTANCE_SPLITS {
        if cells.peek().unwrap().upper<=lower+tolerance {break;}
        let old=cells.pop().unwrap();let t=old.triangle;
        let edge=(0..3).max_by(|&a,&b|t[a].distance_squared(t[(a+1)%3]).total_cmp(&t[b].distance_squared(t[(b+1)%3]))).unwrap();
        let (a,b,c)=(t[edge],t[(edge+1)%3],t[(edge+2)%3]);let middle=(a+b)*0.5;
        cells.push(cell([a,middle,c],target,&mut lower));cells.push(cell([middle,b,c],target,&mut lower));
    }
    lower.max(cells.peek().unwrap().upper)
}
fn surface_bound(source:&[Triangle],coarse:&[Triangle],diameter:f32) -> f32 {
    let tolerance=(diameter*0.001).max(1e-6);
    directed_bound(source,coarse,tolerance).max(directed_bound(coarse,source,tolerance))+diameter*1e-5+1e-6
}

/// Build a complete primitive LOD DAG when the exact closed sweep topology is
/// recognized. `None` asks the caller to retain its ordinary mesh cooker.
/// Material/placement/branch identities and every finest source triangle stay.
pub fn build(part:&Part<'_>) -> Option<ClusterMesh> {
    let sweep=recognize(part)?;
    let source=triangles(part.vertices,part.indices);
    if source.iter().any(|[a,b,c]|!(b-a).cross(c-a).length_squared().is_finite() || (b-a).cross(c-a).length_squared()<1e-16) {return None;}
    let mut mesh=ClusterMesh::build_with_policy(&[Part {vertices:part.vertices,indices:part.indices,material:part.material}], |_|SimplificationPolicy::TexturedGeometry);
    if mesh.clusters.len()!=1 || mesh.leaf_triangles()!=source.len() {return None;}
    let diameter=mesh.sphere[3]*2.;
    let height=sweep.points[sweep.tip as usize].pos[1]-sweep.points[sweep.rings[0][0]as usize].pos[1];
    let mut rings:Vec<_>=(0..sweep.rings.len()).collect();
    let mut errors=vec![0.];
    while rings.len()>1 {
        let mut best:Option<(f32,usize,Vec<Vertex>)>=None;
        for remove in 1..rings.len() {
            let mut retained=rings.clone();retained.remove(remove);
            let vertices=reduced(&sweep,&retained);
            if vertices.iter().any(|v|!Vec3::from(v.normal).is_finite()) {continue;}
            let indices:Vec<_>=(0..vertices.len()as u32).collect();let coarse=triangles(&vertices,&indices);
            if !coverage_guard(&source,&coarse) {continue;}
            let error=surface_bound(&source,&coarse,diameter);
            if !error.is_finite() || error<=0. {continue;}
            if best.as_ref().is_none_or(|(e,_,_)|error<*e) {best=Some((error,remove,vertices));}
        }
        let Some((error,remove,vertices))=best else {break;};
        rings.remove(remove);
        let detail_floor=height*MIN_DETAIL_ERROR_FRACTION*errors.len()as f32;
        let error=error.max(detail_floor).max(errors.last().unwrap()*1.001+1e-6);
        let first_vertex=mesh.vertices.len()as u32;let first_vert=mesh.cluster_verts.len()as u32;let first_tri=mesh.tris.len()as u32;
        mesh.cluster_verts.extend(0..vertices.len()as u32); // rebase immediately below
        for id in &mut mesh.cluster_verts[first_vert as usize..] {*id+=first_vertex;}
        for i in (0..vertices.len()as u32).step_by(3) {mesh.tris.push(i|((i+1)<<8)|((i+2)<<16));}
        mesh.clusters.push(GpuCluster {cull:mesh.sphere,parent:mesh.sphere,own:mesh.sphere,parent_error:f32::MAX,own_error:error,
            first_tri,tri_count:(vertices.len()/3)as u32,first_vert,material:part.material,depth:errors.len()as u32,_pad:0});
        mesh.vertices.extend(vertices);errors.push(error);
    }
    if errors.len()==1 {return None;}
    for i in 0..mesh.clusters.len()-1 {mesh.clusters[i].parent=mesh.sphere;mesh.clusters[i].parent_error=errors[i+1];}
    mesh.levels=errors.len()as u32;
    Some(mesh)
}

// Thin, opaque leaf surfaces use an explicit topology contract rather than
// pretending they are closed volumes. Three paired stations and one tip form
// five triangles. Coarser cuts retain both root vertices and the same tip.
struct Ribbon {
    points: Vec<Vertex>,
    pairs: Vec<[u32; 2]>,
    tip: u32,
}

fn ribbon_strip(a: [u32; 2], b: [u32; 2]) -> [[u32; 3]; 2] {
    [[a[0], a[1], b[1]], [a[0], b[1], b[0]]]
}

fn recognize_ribbon(part: &Part<'_>) -> Option<Ribbon> {
    if part.indices.len() != 15 { return None; }
    let mut points = Vec::<Vertex>::new();
    let mut map = BTreeMap::new();
    let mut remap = Vec::with_capacity(part.vertices.len());
    for &v in part.vertices {
        if !Vec3::from(v.pos).is_finite() || !Vec3::from(v.normal).is_finite()
            || v.uv.iter().any(|x| !x.is_finite()) { return None; }
        let id = *map.entry(position_key(v.pos)).or_insert_with(|| {
            let id = points.len() as u32; points.push(v); id
        });
        remap.push(id);
    }
    if points.len() != 7 { return None; }
    let mut faces = BTreeSet::new();
    for t in part.indices.chunks_exact(3) {
        let ids = [*remap.get(t[0] as usize)?, *remap.get(t[1] as usize)?, *remap.get(t[2] as usize)?];
        if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2]
            || !faces.insert(oriented_key(ids)) { return None; }
    }
    let mut heights: Vec<_> = points.iter().map(|p| p.pos[1]).collect();
    heights.sort_by(f32::total_cmp); heights.dedup();
    if heights.len() != 4 || heights[0] != 0.0 { return None; }
    let stations: Vec<Vec<u32>> = heights.iter().map(|&height| points.iter().enumerate()
        .filter(|(_, p)| p.pos[1] == height).map(|(i, _)| i as u32).collect()).collect();
    if stations[..3].iter().any(|s| s.len() != 2) || stations[3].len() != 1 { return None; }
    let root = (Vec3::from(points[stations[0][0] as usize].pos)
        + Vec3::from(points[stations[0][1] as usize].pos)) * 0.5;
    if root.length_squared() > 1e-12 { return None; }
    let tip = stations[3][0];
    // Match orientation as well as adjacency. A missing, reversed or duplicate
    // face must not silently acquire a different silhouette during cooking.
    for order in 0..8 {
        let pairs: Vec<_> = (0..3).map(|station| {
            let swap = (order >> station) & 1;
            [stations[station][swap], stations[station][1 - swap]]
        }).collect();
        let mut expected = BTreeSet::new();
        for pair in pairs.windows(2) {
            expected.extend(ribbon_strip(pair[0], pair[1]).into_iter().map(oriented_key));
        }
        expected.insert(oriented_key([pairs[2][0], pairs[2][1], tip]));
        if faces == expected { return Some(Ribbon { points, pairs, tip }); }
    }
    None
}

fn reduced_ribbon(ribbon: &Ribbon, stations: &[usize]) -> Vec<Vertex> {
    let mut faces = Vec::new();
    for pair in stations.windows(2) {
        faces.extend(ribbon_strip(ribbon.pairs[pair[0]], ribbon.pairs[pair[1]]));
    }
    let last = ribbon.pairs[*stations.last().unwrap()];
    faces.push([last[0], last[1], ribbon.tip]);
    let mut vertices = Vec::with_capacity(faces.len() * 3);
    for face in faces {
        let [a, b, c] = face.map(|i| Vec3::from(ribbon.points[i as usize].pos));
        let normal = (b - a).cross(c - a).normalize();
        for id in face {
            let mut vertex = ribbon.points[id as usize];
            vertex.normal = normal.to_array(); vertices.push(vertex);
        }
    }
    vertices
}

fn ribbon_coverage_guard(source: &[Triangle], coarse: &[Triangle]) -> bool {
    let projected: Vec<_> = PROJECTION_DIRECTIONS.iter().map(|&d| {
        (area(source, Vec3::from(d)), area(coarse, Vec3::from(d)))
    }).collect();
    let maximum = projected.iter().map(|&(a, _)| a).fold(0.0f32, f32::max);
    if !maximum.is_finite() || maximum <= 0.0 { return false; }
    projected.into_iter().all(|(a, b)| {
        // A relative-only rule is ill-conditioned for an almost edge-on thin
        // leaf. Permit at most 2% of its largest projected area there; dominant
        // directions still keep the ordinary 10% coverage bound. Geometry is
        // independently bounded below, so this is not an opacity substitute.
        b.is_finite() && (b - a).abs() <= (a * 0.10).max(maximum * 0.02)
    })
}

/// Source-derived cuts for an opaque, two-sided thin ribbon: three paired
/// constant-Y stations, a root midpoint at zero, and one final tip (5 faces).
/// Unknown topology returns `None`. Recognized geometry always keeps its
/// byte-identical finest cut; rejected 3/1-face reductions leave fewer levels.
/// Every cut retains the same root pair, tip, material, and real face normals.
/// Callers baking these open leaves must use `ProjectionMeasure::SheetCrossings`.
pub fn build_ribbon(part: &Part<'_>) -> Option<ClusterMesh> {
    let ribbon = recognize_ribbon(part)?;
    let source = triangles(part.vertices, part.indices);
    if source.iter().any(|[a, b, c]| {
        let area2 = (b - a).cross(c - a).length_squared();
        !area2.is_finite() || area2 < 1e-16
    }) { return None; }
    let mut mesh = ClusterMesh::build_with_policy(&[Part {
        vertices: part.vertices, indices: part.indices, material: part.material,
    }], |_| SimplificationPolicy::TexturedGeometry);
    if mesh.clusters.len() != 1 || mesh.leaf_triangles() != 5 { return None; }
    let diameter = mesh.sphere[3] * 2.0;
    let mut stations = vec![0usize, 1, 2];
    let mut errors = vec![0.0f32];
    while stations.len() > 1 {
        let mut best: Option<(f32, usize, Vec<Vertex>)> = None;
        for remove in 1..stations.len() {
            let mut retained = stations.clone(); retained.remove(remove);
            let vertices = reduced_ribbon(&ribbon, &retained);
            if vertices.iter().any(|v| !Vec3::from(v.normal).is_finite()) { continue; }
            let indices: Vec<_> = (0..vertices.len() as u32).collect();
            let coarse = triangles(&vertices, &indices);
            if !ribbon_coverage_guard(&source, &coarse) { continue; }
            let error = surface_bound(&source, &coarse, diameter);
            if !error.is_finite() || error <= 0.0 { continue; }
            if best.as_ref().is_none_or(|(old, _, _)| error < *old) {
                best = Some((error, remove, vertices));
            }
        }
        let Some((error, remove, vertices)) = best else { break; };
        stations.remove(remove);
        // Only the measured conservative surface bound and a monotonic epsilon.
        // A height-derived detail floor would retain tiny dense grass forever.
        let error = error.max(errors.last().unwrap() * 1.001 + 1e-6);
        let first_vertex = mesh.vertices.len() as u32;
        let first_vert = mesh.cluster_verts.len() as u32;
        let first_tri = mesh.tris.len() as u32;
        mesh.cluster_verts.extend((0..vertices.len() as u32).map(|id| first_vertex + id));
        for i in (0..vertices.len() as u32).step_by(3) {
            mesh.tris.push(i | ((i + 1) << 8) | ((i + 2) << 16));
        }
        mesh.clusters.push(GpuCluster { cull: mesh.sphere, parent: mesh.sphere,
            own: mesh.sphere, parent_error: f32::MAX, own_error: error,
            first_tri, tri_count: (vertices.len() / 3) as u32, first_vert,
            material: part.material, depth: errors.len() as u32, _pad: 0 });
        mesh.vertices.extend(vertices); errors.push(error);
    }
    for i in 0..mesh.clusters.len() - 1 {
        mesh.clusters[i].parent = mesh.sphere;
        mesh.clusters[i].parent_error = errors[i + 1];
    }
    mesh.levels = errors.len() as u32;
    Some(mesh)
}

/// A source-to-cut correspondence for a ribbon with planar cross-sections.
/// Deviations retain their direction instead of collapsing to a local scalar
/// and subsequently multiplying by the tallest axis of an anisotropic blade.
#[derive(Clone, Debug)]
pub struct RibbonErrorBound {
    deviations: Vec<DVec3>,
    maximum_coordinate: DVec3,
    identical: bool,
}

impl RibbonErrorBound {
    /// Conservative two-way surface bound after the actual affine placement.
    /// This includes a rounding allowance for packing transformed f32 vertices;
    /// subsequent uniform growth/rigid wind/terrain shear apply their ordinary
    /// operator bounds. Translation does not change exact geometric distance,
    /// but its magnitude participates in the vertex-rounding allowance.
    pub fn transformed_error(&self, transform: Mat4) -> Option<f32> {
        if !transform.is_finite() || transform.x_axis.w != 0.0
            || transform.y_axis.w != 0.0 || transform.z_axis.w != 0.0
            || transform.w_axis.w != 1.0 { return None; }
        if self.identical { return Some(0.0); }
        let x = transform.x_axis.truncate().as_dvec3();
        let y = transform.y_axis.truncate().as_dvec3();
        let z = transform.z_axis.truncate().as_dvec3();
        let linear = |p: DVec3| x * p.x + y * p.y + z * p.z;
        let geometric = self.deviations.iter().map(|&d| linear(d).length()).fold(0.0f64, f64::max);
        let magnitude = (x.abs() * self.maximum_coordinate.x
            + y.abs() * self.maximum_coordinate.y + z.abs() * self.maximum_coordinate.z
            + transform.w_axis.truncate().as_dvec3().abs()).length();
        // Each packed vertex performs at most a few f32 products/additions.
        // Convex interpolation cannot amplify their maximum error; 32 epsilon
        // covers both surfaces plus f64 correspondence/norm evaluation slack.
        let padding = magnitude * (32.0 * f32::EPSILON as f64)
            + (geometric + magnitude) * (64.0 * f64::EPSILON) + 1e-12;
        let bound = geometric + padding;
        if !bound.is_finite() || bound > f32::MAX as f64 { return None; }
        let rounded = bound as f32;
        Some(if (rounded as f64) < bound { f32::from_bits(rounded.to_bits() + 1) } else { rounded })
    }
}

fn ribbon_pair_at(ribbon: &Ribbon, stations: &[usize], height: f64) -> Option<[DVec3; 2]> {
    let point = |id: u32| Vec3::from(ribbon.points[id as usize].pos).as_dvec3();
    let tip = point(ribbon.tip);
    for (index, &station) in stations.iter().enumerate() {
        let a = ribbon.pairs[station].map(point);
        let b = stations.get(index + 1).map(|&next| ribbon.pairs[next].map(point)).unwrap_or([tip; 2]);
        if height < a[0].y || height > b[0].y { continue; }
        let t = (height - a[0].y) / (b[0].y - a[0].y);
        return Some(std::array::from_fn(|edge| {
            let mut p = a[edge] + (b[edge] - a[edge]) * t;
            p.y = height; p
        }));
    }
    None
}

/// Derive cached directional error proofs for the already accepted cuts.
/// This never adds a level or changes the projected-area acceptance gate.
///
/// Every source pair must share X and Y, with a consistent Z-side ordering.
/// Its triangulated strips then are exactly planar x(y) surfaces. At any Y,
/// source and cut can be paired by the same relative position across width.
/// Their difference is affine across width, so its norm after any affine
/// placement is maximal on an edge. Between the union of both sets of Y
/// stations, each edge difference is affine again: checking those endpoints
/// bounds every surface point in both directions. Twisted/general ribbons
/// deliberately return None and retain the ordinary scalar operator bound.
pub fn ribbon_error_bounds(part: &Part<'_>, mesh: &ClusterMesh) -> Option<Vec<RibbonErrorBound>> {
    let ribbon = recognize_ribbon(part)?;
    if mesh.clusters.is_empty() || mesh.clusters.len() > 3 { return None; }
    let mut side = 0.0f32;
    for pair in &ribbon.pairs {
        let a = Vec3::from(ribbon.points[pair[0] as usize].pos);
        let b = Vec3::from(ribbon.points[pair[1] as usize].pos);
        if a.x != b.x || a.y != b.y || a.z == b.z { return None; }
        if side == 0.0 { side = (a.z - b.z).signum(); }
        if (a.z - b.z).signum() != side { return None; }
    }
    let point_ids: BTreeMap<_, _> = ribbon.points.iter().enumerate()
        .map(|(i, v)| (position_key(v.pos), i as u32)).collect();
    let maximum_coordinate = ribbon.points.iter().map(|v| Vec3::from(v.pos).as_dvec3().abs())
        .fold(DVec3::ZERO, DVec3::max);
    let source_stations: Vec<_> = (0..ribbon.pairs.len()).collect();
    let mut result = Vec::with_capacity(mesh.clusters.len());
    for cluster in &mesh.clusters {
        let mut faces = BTreeSet::new();
        let mut present = BTreeSet::new();
        for tri in mesh.tris.get(cluster.first_tri as usize..)?.get(..cluster.tri_count as usize)? {
            let mut ids = [0u32; 3];
            for (corner, local) in [tri & 255, (tri >> 8) & 255, (tri >> 16) & 255].into_iter().enumerate() {
                let index = *mesh.cluster_verts.get(cluster.first_vert as usize + local as usize)?;
                let vertex = mesh.vertices.get(index as usize)?;
                ids[corner] = *point_ids.get(&position_key(vertex.pos))?;
                present.insert(ids[corner]);
            }
            if !faces.insert(oriented_key(ids)) { return None; }
        }
        if !present.contains(&ribbon.tip) { return None; }
        let mut stations = Vec::new();
        for (station, pair) in ribbon.pairs.iter().enumerate() {
            let a = present.contains(&pair[0]); let b = present.contains(&pair[1]);
            if a != b { return None; }
            if a { stations.push(station); }
        }
        if stations.first() != Some(&0) { return None; }
        let mut expected = BTreeSet::new();
        for pair in stations.windows(2) {
            expected.extend(ribbon_strip(ribbon.pairs[pair[0]], ribbon.pairs[pair[1]])
                .into_iter().map(oriented_key));
        }
        let last = ribbon.pairs[*stations.last()?];
        expected.insert(oriented_key([last[0], last[1], ribbon.tip]));
        if faces != expected { return None; }
        // Accepted cut vertices must be source stations; keep the union
        // explicit so correspondence endpoints can never skip a cut kink.
        let mut heights: Vec<_> = source_stations.iter().chain(&stations)
            .map(|&station| ribbon.points[ribbon.pairs[station][0] as usize].pos[1] as f64)
            .chain(std::iter::once(ribbon.points[ribbon.tip as usize].pos[1] as f64)).collect();
        heights.sort_by(f64::total_cmp); heights.dedup();
        let mut deviations = Vec::with_capacity(heights.len() * 2);
        for height in heights {
            let source = ribbon_pair_at(&ribbon, &source_stations, height)?;
            let coarse = ribbon_pair_at(&ribbon, &stations, height)?;
            deviations.extend((0..2).map(|edge| source[edge] - coarse[edge]));
        }
        result.push(RibbonErrorBound { deviations, maximum_coordinate, identical: stations == source_stations });
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foliage_assembly::{generate,Limits,Parameters};

    fn selected(mesh:&ClusterMesh,threshold:f32)->Vec<&GpuCluster> {
        mesh.clusters.iter().filter(|c|c.own_error<=threshold && c.parent_error>threshold).collect()
    }
    fn level_triangles(mesh:&ClusterMesh,c:&GpuCluster)->Vec<Triangle> {
        mesh.tris[c.first_tri as usize..][..c.tri_count as usize].iter().map(|&t|[t&255,(t>>8)&255,(t>>16)&255]
            .map(|i|Vec3::from(mesh.vertices[mesh.cluster_verts[(c.first_vert+i)as usize]as usize].pos))).collect()
    }
    #[test]
    fn finest_geometry_is_unchanged_and_every_cut_retains_one_closed_primitive() {
        for parameters in [Parameters::grass(4),Parameters::conifer_twig(9)] {
            let assembly=generate(parameters,Limits::default()).unwrap();
            for geometry in &assembly.parts {
                let part=Part {vertices:&geometry.vertices,indices:&geometry.indices,material:7};
                let Some(mesh)=build(&part) else {continue;};
                let original=ClusterMesh::build_with_policy(&[part], |_|SimplificationPolicy::TexturedGeometry);
                assert_eq!(bytemuck::cast_slice::<_,u8>(&mesh.vertices[..original.vertices.len()]),bytemuck::cast_slice::<_,u8>(&original.vertices));
                assert_eq!(&mesh.tris[..original.tris.len()],original.tris.as_slice());
                assert_eq!(&mesh.cluster_verts[..original.cluster_verts.len()],original.cluster_verts.as_slice());
                assert_eq!(mesh.leaf_triangles(),geometry.indices.len()/3);
                assert!(mesh.clusters.last().unwrap().tri_count<mesh.clusters[0].tri_count);
                for c in &mesh.clusters {
                    assert_eq!(c.material,7);assert_eq!(selected(&mesh,c.own_error).len(),1);
                    let triangles=level_triangles(&mesh,c);let mut edges=BTreeMap::new();
                    for triangle in &triangles {for i in 0..3 {
                        let a=position_key(triangle[i].to_array());let b=position_key(triangle[(i+1)%3].to_array());
                        let (edge,sign)=if a<b{((a,b),1i32)}else{((b,a),-1)};
                        let record=edges.entry(edge).or_insert((0,0));record.0+=1;record.1+=sign;
                    }}
                    assert!(edges.values().all(|&e|e==(2,0)),"LOD is not an outward closed primitive");
                    assert!(coverage_guard(&level_triangles(&mesh,&mesh.clusters[0]),&triangles));
                    for t in triangles {for p in t {
                        assert!(Vec3::from_slice(&mesh.sphere[..3]).distance(p)<=mesh.sphere[3]+1e-5);
                    }}
                }
                assert_eq!(selected(&mesh,0.)[0].tri_count,mesh.clusters[0].tri_count);
            }
        }
    }
    #[test]
    fn conservative_errors_cover_dense_two_way_surface_samples() {
        let assembly=generate(Parameters::conifer_twig(9),Limits::default()).unwrap();let geometry=&assembly.parts[0];
        let mesh=build(&Part {vertices:&geometry.vertices,indices:&geometry.indices,material:0}).expect("needle gains source LODs");
        let source=level_triangles(&mesh,&mesh.clusters[0]);
        for cluster in &mesh.clusters[1..] {
            let coarse=level_triangles(&mesh,cluster);
            for (from,to) in [(&source,&coarse),(&coarse,&source)] {for &[a,b,c] in from {for i in 0..=24 {for j in 0..=24-i {
                let p=a+(b-a)*(i as f32/24.)+(c-a)*(j as f32/24.);
                assert!(distance(p,to)<=cluster.own_error+1e-6);
            }}}}
        }
    }
    #[test]
    fn tiny_needles_reduce_at_mid_distance_without_disappearing() {
        let assembly=generate(Parameters::conifer_twig(9),Limits::default()).unwrap();let geometry=&assembly.parts[0];
        let mesh=build(&Part {vertices:&geometry.vertices,indices:&geometry.indices,material:0}).unwrap();
        let scale=0.135f32;let focal=1080./(2.*(50f32.to_radians()*0.5).tan());
        let near_threshold=(5.-mesh.sphere[3]*scale)/focal/scale;
        assert_eq!(selected(&mesh,near_threshold)[0].tri_count,mesh.clusters[0].tri_count);
        for distance in [50.,100.,150.] {
            let world_error=(distance-mesh.sphere[3]*scale)/focal;
            let selected=selected(&mesh,world_error/scale);
            assert_eq!(selected.len(),1);assert!(selected[0].tri_count>0);
            assert!(selected[0].tri_count<mesh.clusters[0].tri_count,"needle did not simplify at {distance}m");
        }
    }
    #[test]
    fn all_grass_shapes_keep_near_detail_and_reduce_by_one_hundred_fifty_metres() {
        let assembly=generate(Parameters::grass(9),Limits::default()).unwrap();
        let scale=0.6f32;let focal=1080./(2.*(50f32.to_radians()*0.5).tan());
        for geometry in &assembly.parts {
            let mesh=build(&Part {vertices:&geometry.vertices,indices:&geometry.indices,material:0}).expect("grass blade gains source LODs");
            let threshold=|distance:f32|(distance-mesh.sphere[3]*scale)/focal/scale;
            assert_eq!(selected(&mesh,threshold(5.))[0].tri_count,mesh.clusters[0].tri_count);
            let far=selected(&mesh,threshold(150.));assert_eq!(far.len(),1);
            assert!(far[0].tri_count>0 && far[0].tri_count<mesh.clusters[0].tri_count);
        }
    }
    #[test]
    fn unrecognized_or_open_source_keeps_ordinary_cooker() {
        let assembly=generate(Parameters::conifer_twig(9),Limits::default()).unwrap();
        let stem=&assembly.parts[2];assert!(build(&Part {vertices:&stem.vertices,indices:&stem.indices,material:0}).is_none());
        let leaf=&assembly.parts[0];assert!(build(&Part {vertices:&leaf.vertices,indices:&leaf.indices[3..],material:0}).is_none());
    }

    fn ribbon_source(curve: f32, bow: f32) -> (Vec<Vertex>, Vec<u32>) {
        let station = |y: f32| {
            let x = curve * ((1.0 - bow) * y + bow * y * y);
            [Vec3::new(x, y, 0.5 * (1.0 - y)), Vec3::new(x, y, -0.5 * (1.0 - y))]
        };
        let pairs = [0.0, 0.45, 0.78].map(station);
        let mut faces = Vec::new();
        for pair in pairs.windows(2) {
            faces.extend([[pair[0][0], pair[0][1], pair[1][1]],
                [pair[0][0], pair[1][1], pair[1][0]]]);
        }
        faces.push([pairs[2][0], pairs[2][1], Vec3::new(curve, 1.0, 0.0)]);
        let vertices: Vec<_> = faces.into_iter().flat_map(|[a, b, c]| {
            let normal = (b - a).cross(c - a).normalize().to_array();
            [a, b, c].map(|p| Vertex { pos: p.to_array(), normal, uv: [p.z + 0.5, p.y] })
        }).collect();
        let indices = (0..vertices.len() as u32).collect();
        (vertices, indices)
    }

    #[test]
    fn ribbon_clod_keeps_finest_bytes_roots_tip_normals_and_complete_cuts() {
        for curve in [0.0, 2.0, 4.0, 6.0] {
            let (vertices, indices) = ribbon_source(curve, 0.2);
            let part = Part { vertices: &vertices, indices: &indices, material: 17 };
            let mesh = build_ribbon(&part).expect("authored thin ribbon is recognized");
            let fine = ClusterMesh::build_with_policy(&[part], |_| SimplificationPolicy::TexturedGeometry);
            assert_eq!(bytemuck::cast_slice::<_, u8>(&mesh.vertices[..fine.vertices.len()]),
                bytemuck::cast_slice::<_, u8>(&fine.vertices));
            assert_eq!(&mesh.tris[..fine.tris.len()], fine.tris.as_slice());
            assert_eq!(&mesh.cluster_verts[..fine.cluster_verts.len()], fine.cluster_verts.as_slice());
            // Curve 2 correctly fails the directional area gate for one triangle.
            let accepted = if curve == 2.0 { vec![5, 3] } else { vec![5, 3, 1] };
            assert_eq!(mesh.clusters.iter().map(|c| c.tri_count).collect::<Vec<_>>(), accepted);
            let finest = level_triangles(&mesh, &mesh.clusters[0]);
            for level in &mesh.clusters {
                assert_eq!(level.material, 17);
                assert_eq!(selected(&mesh, level.own_error).len(), 1);
                let surface = level_triangles(&mesh, level);
                assert!(ribbon_coverage_guard(&finest, &surface));
                let points: BTreeSet<_> = surface.iter().flatten().map(|p| position_key(p.to_array())).collect();
                for p in [Vec3::new(0.0, 0.0, 0.5), Vec3::new(0.0, 0.0, -0.5), Vec3::new(curve, 1.0, 0.0)] {
                    assert!(points.contains(&position_key(p.to_array())), "source root pair and tip survive");
                }
                for triangle in surface { for point in triangle {
                    assert!(point.distance(Vec3::from_slice(&mesh.sphere[..3])) <= mesh.sphere[3] + 1e-5);
                }}
                for (triangle, ids) in level_triangles(&mesh, level).iter().zip(
                    mesh.tris[level.first_tri as usize..][..level.tri_count as usize].iter()) {
                    let normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]).normalize();
                    for id in [ids & 255, (ids >> 8) & 255, (ids >> 16) & 255] {
                        let actual = Vec3::from(mesh.vertices[mesh.cluster_verts[(level.first_vert + id) as usize] as usize].normal);
                        assert!(normal.dot(actual) > 0.9999, "normals follow the actual reduced surface");
                    }
                }
            }
        }
    }

    #[test]
    fn ribbon_clod_bounds_dense_two_way_samples_without_height_detail_floor() {
        for curve in [0.0, 2.0, 6.0] {
            let (vertices, indices) = ribbon_source(curve, 0.2);
            let mesh = build_ribbon(&Part { vertices: &vertices, indices: &indices, material: 0 }).unwrap();
            let source = level_triangles(&mesh, &mesh.clusters[0]);
            for level in &mesh.clusters[1..] {
                let coarse = level_triangles(&mesh, level);
                for (from, to) in [(&source, &coarse), (&coarse, &source)] {
                    for &[a, b, c] in from { for i in 0..=24 { for j in 0..=24 - i {
                        let p = a + (b - a) * (i as f32 / 24.0) + (c - a) * (j as f32 / 24.0);
                        assert!(distance(p, to) <= level.own_error + 1e-6);
                    }}}
                }
            }
            if curve == 0.0 {
                // Adaptive triangle covering retains a finite conservative residual.
                assert!(mesh.clusters.last().unwrap().own_error < 0.04,
                    "coplanar bound {} must remain below the former height-times-.08 floor",
                    mesh.clusters.last().unwrap().own_error);
            }
        }
    }

    #[test]
    fn ribbon_clod_rejects_dominant_coverage_change_and_malformed_sources() {
        let (vertices, indices) = ribbon_source(6.0, 1.0);
        let part = Part { vertices: &vertices, indices: &indices, material: 0 };
        let mesh = build_ribbon(&part).unwrap();
        assert!(mesh.clusters.iter().all(|c| c.tri_count != 1),
            "strongly bowed source must not get an area-inflating one-triangle cut");
        let ribbon = recognize_ribbon(&part).unwrap();
        let single = reduced_ribbon(&ribbon, &[0]);
        let single_indices: Vec<_> = (0..single.len() as u32).collect();
        assert!(!ribbon_coverage_guard(&triangles(&vertices, &indices), &triangles(&single, &single_indices)));
        let mut reversed = indices.clone(); reversed.swap(0, 1);
        assert!(build_ribbon(&Part { vertices: &vertices, indices: &reversed, material: 0 }).is_none());
        assert!(build_ribbon(&Part { vertices: &vertices, indices: &indices[..12], material: 0 }).is_none());
        let mut off_station = vertices.clone(); off_station[0].pos[1] += 0.01;
        assert!(build_ribbon(&Part { vertices: &off_station, indices: &indices, material: 0 }).is_none());
        let closed = generate(Parameters::grass(1), Limits::default()).unwrap();
        let p = &closed.parts[0];
        assert!(build_ribbon(&Part { vertices: &p.vertices, indices: &p.indices, material: 0 }).is_none());
    }

    // Independent test oracle: project onto the triangle plane and use
    // oriented edge half-spaces, otherwise measure the three line segments.
    // Do not reuse the production f32 dot-product barycentric determinant:
    // its subtraction loses precision for the very thin physical ribbons.
    fn point_distance_f64(p: DVec3, triangle: Triangle) -> f64 {
        let [a, b, c] = triangle.map(|p| p.as_dvec3());
        let n = (b - a).cross(c - a);
        let n2 = n.length_squared();
        if n2 > 0.0 {
            let signed = (p - a).dot(n);
            let q = p - n * (signed / n2);
            if [(a, b), (b, c), (c, a)].iter()
                .all(|&(u, v)| (v - u).cross(q - u).dot(n) >= 0.0) {
                return signed.abs() / n2.sqrt();
            }
        }
        [(a, b), (b, c), (c, a)].iter().map(|&(u, v)| {
            let edge = v - u;
            let length2 = edge.length_squared();
            let t = if length2 > 0.0 { ((p - u).dot(edge) / length2).clamp(0.0, 1.0) } else { 0.0 };
            p.distance(u + edge * t)
        }).fold(f64::INFINITY, f64::min)
    }

    fn surface_distance_f64(p: DVec3, surface: &[Triangle]) -> f64 {
        surface.iter().map(|&t| point_distance_f64(p, t)).fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn independent_distance_oracle_handles_thin_triangle_plane_edges_and_vertices() {
        let triangle = [Vec3::new(0.0, 0.0, 0.003), Vec3::new(0.0, 0.0, -0.003),
            Vec3::new(0.0, 0.4, 0.0)];
        // This point lies strictly inside the long, narrow triangle. A
        // zero plane distance is known analytically, without a LOD proof.
        let inside = DVec3::new(0.0, 0.15187500417232513, 0.0016546875704079866);
        assert_eq!(point_distance_f64(inside, triangle), 0.0);
        assert!((point_distance_f64(inside + DVec3::X * 0.017, triangle) - 0.017).abs() < 1e-14);
        assert!((point_distance_f64(DVec3::new(0.0, -0.01, 0.0), triangle) - 0.01).abs() < 1e-14);
        let tip = triangle[2].as_dvec3();
        assert!((point_distance_f64(tip + DVec3::Y * 0.02, triangle) - 0.02).abs() < 1e-14);
        let reversed = [triangle[0], triangle[2], triangle[1]];
        assert_eq!(point_distance_f64(inside, reversed), 0.0);
    }

    #[test]
    fn ribbon_physical_error_bounds_dense_affine_two_way_samples() {
        use glam::{Quat, Vec4};
        let transforms = [
            Mat4::from_scale(Vec3::new(0.006, 0.40, 0.006)),
            Mat4::from_scale_rotation_translation(Vec3::new(0.014, 0.12, 0.010),
                Quat::from_rotation_y(1.3) * Quat::from_rotation_z(0.27), Vec3::new(0.17, 0.0, -0.09)),
            Mat4::from_cols(Vec4::new(1.0, 0.61, 0.0, 0.0), Vec4::Y,
                Vec4::new(0.0, -0.35, 1.0, 0.0), Vec4::W)
                * Mat4::from_scale_rotation_translation(Vec3::new(0.009, 0.30, 0.014),
                    Quat::from_rotation_x(-0.21) * Quat::from_rotation_y(2.1), Vec3::new(-0.13, 0.01, 0.08)),
        ];
        for curve in [0.0, 2.0, 4.0, 6.0] {
            let (vertices, indices) = ribbon_source(curve, 0.2);
            let part = Part { vertices: &vertices, indices: &indices, material: 0 };
            let mesh = build_ribbon(&part).unwrap();
            let proofs = ribbon_error_bounds(&part, &mesh).unwrap();
            assert_eq!(proofs.len(), mesh.clusters.len());
            for (transform_index, transform) in transforms.into_iter().enumerate() {
                assert_eq!(proofs[0].transformed_error(transform), Some(0.0));
                let apply = |surface: Vec<Triangle>| surface.into_iter()
                    .map(|t| t.map(|p| transform.transform_point3(p))).collect::<Vec<_>>();
                let source = apply(level_triangles(&mesh, &mesh.clusters[0]));
                for (level, proof) in mesh.clusters.iter().zip(&proofs).skip(1) {
                    let bound = proof.transformed_error(transform).unwrap();
                    let coarse = apply(level_triangles(&mesh, level));
                    let mut largest_f32_disagreement = (0.0f64, 0.0f32, 0.0f64, Vec3::ZERO);
                    for (direction, (from, to)) in [(&source, &coarse), (&coarse, &source)].into_iter().enumerate() {
                        for (triangle_index, &[a, b, c]) in from.iter().enumerate() { for i in 0..=32 { for j in 0..=32 - i {
                            // Sample the actual packed f32 triangle in f64,
                            // so barycentric arithmetic cannot move the test
                            // point away from the measured surface.
                            let p = a.as_dvec3() + (b.as_dvec3() - a.as_dvec3()) * (i as f64 / 32.0)
                                + (c.as_dvec3() - a.as_dvec3()) * (j as f64 / 32.0);
                            let actual = surface_distance_f64(p, to);
                            assert!(actual <= f64::from(bound),
                                "affine ribbon {curve}, transform {transform_index}, {} triangles, direction {direction}, triangle {triangle_index}, barycentric {i}/{j}, point {p:?}: {actual} exceeds {bound}; transform {transform:?}", level.tri_count);
                            // Diagnose the original failure at exactly the
                            // same f32 point for both distance routines.
                            let p32 = a + (b - a) * (i as f32 / 32.0) + (c - a) * (j as f32 / 32.0);
                            let old = distance(p32, to);
                            let robust = surface_distance_f64(p32.as_dvec3(), to);
                            let disagreement = f64::from(old) - robust;
                            if disagreement > largest_f32_disagreement.0 {
                                largest_f32_disagreement = (disagreement, old, robust, p32);
                            }
                        }}}
                    }
                    if largest_f32_disagreement.1 > bound {
                        eprintln!("r37 thin-triangle oracle diagnostic: curve {curve}, transform {transform_index}, {} triangles, bound {bound}, f32 versus independent f64 at same point {largest_f32_disagreement:?}", level.tri_count);
                    }
                }
            }
        }
    }

    #[test]
    fn ribbon_physical_error_tracks_width_and_rejects_unproven_twist_or_cut() {
        let (vertices, indices) = ribbon_source(6.0, 0.2);
        let part = Part { vertices: &vertices, indices: &indices, material: 0 };
        let mesh = build_ribbon(&part).unwrap();
        let proofs = ribbon_error_bounds(&part, &mesh).unwrap();
        let last = proofs.last().unwrap();
        let a = last.transformed_error(Mat4::from_scale(Vec3::new(0.006, 0.12, 0.006))).unwrap();
        let tall = last.transformed_error(Mat4::from_scale(Vec3::new(0.006, 0.40, 0.006))).unwrap();
        let broad = last.transformed_error(Mat4::from_scale(Vec3::new(0.012, 0.12, 0.006))).unwrap();
        assert!(tall >= a && tall < a * 1.01, "height adds rounding allowance, not a fictitious bow error");
        assert!(broad > a * 1.98 && broad < a * 2.01, "physical width scales actual displacement");
        // A twist changes X between the two endpoints of one station. Such
        // a strip is no longer the proven planar x(y) graph; preserve fallback.
        let mut twisted = vertices.clone();
        for v in &mut twisted { if v.pos[1] == 0.45 && v.pos[2] > 0.0 { v.pos[0] += 0.03; } }
        let twisted_part = Part { vertices: &twisted, indices: &indices, material: 0 };
        let twisted_mesh = build_ribbon(&twisted_part).unwrap();
        assert!(ribbon_error_bounds(&twisted_part, &twisted_mesh).is_none());
        let mut damaged = mesh;
        damaged.clusters.last_mut().unwrap().tri_count = 0;
        assert!(ribbon_error_bounds(&part, &damaged).is_none());
        assert!(last.transformed_error(Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0)).is_none());
    }

    #[test]
    fn ribbon_physical_projection_gate_diagnostic_keeps_canonical_acceptance() {
        let source = generate(Parameters::meadow_patch(101), Limits::meadow_patch()).unwrap();
        let poses = source.branch_transforms().unwrap();
        let mut passed = [0usize; 3]; let mut tested = [0usize; 3];
        for placement in &source.instances {
            let geometry = &source.parts[placement.part as usize];
            let part = Part { vertices: &geometry.vertices, indices: &geometry.indices, material: 0 };
            let ribbon = recognize_ribbon(&part).unwrap();
            let coarse = reduced_ribbon(&ribbon, &[0]);
            let transform = poses[placement.branch as usize] * placement.rest;
            let apply = |surface: Vec<Triangle>| surface.into_iter()
                .map(|t| t.map(|p| transform.transform_point3(p))).collect::<Vec<_>>();
            let original = apply(triangles(&geometry.vertices, &geometry.indices));
            let coarse_indices: Vec<_> = (0..coarse.len() as u32).collect();
            let proposed = apply(triangles(&coarse, &coarse_indices));
            tested[placement.part as usize] += 1;
            passed[placement.part as usize] += usize::from(ribbon_coverage_guard(&original, &proposed));
        }
        eprintln!("r37 physical projected-area diagnostic ONLY: one-triangle accepted {passed:?} / {tested:?}; canonical geometry/cut acceptance is unchanged");
        let geometry = &source.parts[0];
        let mesh = build_ribbon(&Part { vertices: &geometry.vertices, indices: &geometry.indices, material: 0 }).unwrap();
        assert_eq!(mesh.clusters.iter().map(|c| c.tri_count).collect::<Vec<_>>(), [5, 3],
            "diagnostic must not change the rejected canonical one-triangle cut");
    }
}
