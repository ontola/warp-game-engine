//! Nanite-style geometry: every mesh becomes clusters of up to 128
//! triangles arranged in a LOD DAG (meshoptimizer's clusterlod). Groups of
//! clusters are simplified together with their borders locked, so any cut
//! through the DAG is crack-free. At run time a cluster is drawn when its
//! own error projects to at most a pixel and its group's coarser
//! replacement would project to more: one cluster per patch of surface,
//! each at the detail the screen needs.

use bytemuck::{Pod, Zeroable};
use meshopt::clusterlod::{ClodMesh, clod_build, clod_default_config, clod_local_indices};

pub const MAX_TRIS: usize = 128;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, serde::Serialize, serde::Deserialize)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// One cluster as the GPU sees it (80 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, serde::Serialize, serde::Deserialize)]
pub struct GpuCluster {
    /// Culling sphere: centre and radius.
    pub cull: [f32; 4],
    /// The group this cluster is in, simplified: if that projects to at most
    /// the threshold, a coarser cluster covers this patch instead.
    pub parent: [f32; 4],
    /// This cluster's own error sphere (the group it was simplified from).
    pub own: [f32; 4],
    pub parent_error: f32,
    pub own_error: f32,
    pub first_tri: u32,
    pub tri_count: u32,
    pub first_vert: u32,
    pub material: u32,
    pub depth: u32,
    /// Bit 0 (CLUSTERS_COARSE_FIRST): this mesh's clusters are sorted by
    /// parent_error, coarsest first (`ClusterMesh::sort_coarse_first`), so the
    /// cull may stop at the first cluster too fine to be needed.
    pub order: u32,
}

/// GpuCluster::order bit 0: see there.
pub const CLUSTERS_COARSE_FIRST: u32 = 1;

/// A mesh part with one material.
pub struct Part<'a> {
    pub vertices: &'a [Vertex],
    pub indices: &'a [u32],
    pub material: u32,
}

/// Material-aware simplification is opt-in: opaque geometry keeps the existing
/// spatial policy. Alpha cards and thin foliage also preserve their projected
/// area; a positional error alone cannot describe disappearing crown coverage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimplificationPolicy {
    Geometry,
    AlphaCards,
    /// Preserve authored normals, UV seams and folds without projected-area credit.
    TexturedGeometry,
    /// Opaque thin leaves/needles: preserve authored orientation and attributes,
    /// and carry projected-area loss through the DAG. No alpha-test assumption.
    ThinFoliage,
    /// ThinFoliage made of many alike separate pieces (needles, leaves):
    /// each level keeps half of them, widened (piece_levels). Parts that are
    /// not made that way fall back to ThinFoliage.
    ThinPieces,
    /// Smooth terrain normals and bounded road-distance material coordinates.
    Terrain,
}

type PendingCluster = (usize, i32, Vec<u32>, [f32; 4]);

/// A purely positional collapse can turn a small bent branch inside out
/// relative to its retained shading normals, despite preserving its outline.
/// Attribute weights reduce this but do not forbid it. Keep such replacement
/// triangles below two projected pixels by assigning half their longest edge
/// as error. Source detail and distant root budgets remain unchanged.
fn protect_surface_orientation(part: &Part<'_>, groups: &mut [[f32; 5]], pending: &[PendingCluster]) {
    let orientation: f64 = part.indices.chunks_exact(3).map(|tri| {
        let p: [glam::Vec3; 3] = std::array::from_fn(|k| glam::Vec3::from(part.vertices[tri[k] as usize].pos));
        let n: glam::Vec3 = tri.iter().map(|i| glam::Vec3::from(part.vertices[*i as usize].normal)).sum();
        (p[1]-p[0]).cross(p[2]-p[0]).dot(n) as f64
    }).sum();
    if !orientation.is_finite() || orientation.abs() < 1e-12 { return; }
    let sign = orientation.signum() as f32;
    // Retained authored triangles can have unusual scanned normals already.
    // They are not a new collapse defect and must not pin unrelated detail.
    let key = |tri: &[u32]| {
        let k = if tri[0] <= tri[1] && tri[0] <= tri[2] {0} else if tri[1] <= tri[2] {1} else {2};
        [tri[k], tri[(k+1)%3], tri[(k+2)%3]]
    };
    let source: std::collections::HashSet<_> = part.indices.chunks_exact(3).map(key).collect();
    for (_, refined, indices, _) in pending {
        if *refined < 0 { continue; }
        let mut error = 0.0f32;
        for tri in indices.chunks_exact(3) {
            if source.contains(&key(tri)) { continue; }
            let p: [glam::Vec3; 3] = std::array::from_fn(|k| glam::Vec3::from(part.vertices[tri[k] as usize].pos));
            let face = (p[1]-p[0]).cross(p[2]-p[0]);
            if face.length_squared() <= 1e-20 { continue; }
            let n: glam::Vec3 = tri.iter().map(|i| glam::Vec3::from(part.vertices[*i as usize].normal)).sum();
            if n.length_squared() <= 1e-12 || face.normalize().dot(n.normalize()) * sign < -0.1 {
                error = error.max((p[1]-p[0]).length().max((p[2]-p[0]).length()).max((p[2]-p[1]).length()) * 0.5);
            }
        }
        groups[*refined as usize][4] = groups[*refined as usize][4].max(error);
    }
    // Child groups precede parents. Every cut still has matching parent/own
    // error and monotonically increasing error, so this creates no LOD gaps.
    for gid in 0..groups.len() {
        for (_, refined, _, _) in pending.iter().filter(|(group,_,_,_)| *group == gid) {
            if *refined >= 0 {
                debug_assert!((*refined as usize) < gid);
                groups[gid][4] = groups[gid][4].max(groups[*refined as usize][4]);
            }
        }
    }
}

#[cfg(test)]
mod terrain_policy_tests {
    use super::*;
    #[test]
    fn road_corridor_keeps_source_triangles_in_coarsest_terrain_cut() {
        let n=33usize;
        let vertices:Vec<_>=(0..n).flat_map(|z|(0..n).map(move|x|Vertex {
            pos:[x as f32*2.0,0.0,z as f32*2.0],normal:[0.0,1.0,0.0],
            uv:[(x as f32*2.0-32.0).abs()-5.0,0.0]
        })).collect();
        let indices:Vec<_>=(0..n-1).flat_map(|z|(0..n-1).flat_map(move|x| {
            let a=(z*n+x)as u32;let stride=n as u32;[a,a+stride,a+1,a+1,a+stride,a+stride+1]
        })).collect();
        let mesh=ClusterMesh::build_with_policy(&[Part {vertices:&vertices,indices:&indices,material:0}],|_|SimplificationPolicy::Terrain);
        let mut retained=std::collections::HashSet::new();
        for cluster in mesh.clusters.iter().filter(|c|c.parent_error>1e30) {
            for packed in &mesh.tris[cluster.first_tri as usize..(cluster.first_tri+cluster.tri_count)as usize] {
                let mut tri=[packed&255,(packed>>8)&255,(packed>>16)&255].map(|i|mesh.cluster_verts[(cluster.first_vert+i)as usize]);tri.sort();retained.insert(tri);
            }
        }
        let mut protected=0;
        for tri in indices.chunks_exact(3).filter(|t|t.iter().all(|i|vertices[*i as usize].uv[0]<=5.0)) {
            let mut key=[tri[0],tri[1],tri[2]];key.sort();assert!(retained.contains(&key),"road-border triangle removed: {key:?}");protected+=1;
        }
        assert!(protected>500);
        assert!(retained.len()<indices.len()/3*3/4,"terrain outside the road corridor must still simplify");
    }
}

fn projected_triangle_area(vertices: &[Vertex], indices: &[u32]) -> f32 {
    indices.chunks_exact(3).map(|tri| {
        let a=glam::Vec3::from_array(vertices[tri[0] as usize].pos);
        let b=glam::Vec3::from_array(vertices[tri[1] as usize].pos);
        let c=glam::Vec3::from_array(vertices[tri[2] as usize].pos);
        (b-a).cross(c-a).abs().element_sum()*0.5
    }).sum()
}

#[cfg(test)]
mod constraint_tests {
    use super::*;
    use std::collections::HashMap;

    const SIDE: usize = 33;

    fn folded_grid() -> (Vec<Vertex>, Vec<u32>) {
        let vertices = (0..SIDE).flat_map(|z| (0..SIDE).map(move |x| {
            let (x, z) = (x as f32, z as f32);
            Vertex {
                pos: [x * 2.0, 0.3 * (z * 0.41).sin() + 0.12 * (x * 0.37 + z * 0.19).cos(), z * 2.0],
                normal: [0.0, 1.0, 0.0],
                uv: [(x * 2.0 - 32.0).abs() - 5.0, 1.5],
            }
        })).collect();
        let indices = (0..SIDE - 1).flat_map(|z| (0..SIDE - 1).flat_map(move |x| {
            let a = (z * SIDE + x) as u32;
            let row = SIDE as u32;
            [a, a + row, a + 1, a + 1, a + row, a + row + 1]
        })).collect();
        (vertices, indices)
    }

    fn key(triangle: [u32; 3]) -> [u32; 3] {
        let first = (0..3).min_by_key(|&i| triangle[i]).unwrap();
        [triangle[first], triangle[(first + 1) % 3], triangle[(first + 2) % 3]]
    }

    fn selected_triangles(mesh: &ClusterMesh, keep: impl Fn(&GpuCluster) -> bool) -> HashMap<[u32; 3], usize> {
        let mut result = HashMap::new();
        for cluster in mesh.clusters.iter().filter(|c| keep(c)) {
            for packed in &mesh.tris[cluster.first_tri as usize..(cluster.first_tri + cluster.tri_count) as usize] {
                let tri = std::array::from_fn(|k| mesh.cluster_verts[cluster.first_vert as usize + ((packed >> (8 * k)) & 255) as usize]);
                *result.entry(key(tri)).or_insert(0) += 1;
            }
        }
        result
    }

    fn assert_retained(mesh: &ClusterMesh, protected: &[[u32; 3]]) {
        assert!(!protected.is_empty());
        let check = |triangles: HashMap<[u32; 3], usize>| {
            for &triangle in protected {
                assert_eq!(triangles.get(&key(triangle)), Some(&1),
                    "constrained source triangle must survive exactly once, with its winding: {triangle:?}");
            }
        };
        let mut events: Vec<_> = mesh.clusters.iter().flat_map(|c| [c.own_error, c.parent_error])
            .filter(|e| *e > 0.0 && *e < 1e30).collect();
        events.sort_by(f32::total_cmp);
        events.dedup();
        let mut thresholds = vec![0.0, 1e29];
        for (i, &event) in events.iter().enumerate() {
            thresholds.push(event);
            let previous = if i == 0 { 0.0 } else { events[i - 1] };
            thresholds.push((previous + event) * 0.5);
        }
        for threshold in thresholds {
            check(selected_triangles(mesh, |c| c.own_error <= threshold && c.parent_error > threshold));
        }
        // Different groups can change levels in the same camera cut. Check
        // those mixed cuts too, using the production perspective error test.
        let centre = glam::Vec3::from_slice(&mesh.sphere[..3]);
        for camera in [centre + glam::Vec3::new(1.0, 10.0, 9.0),
            centre + glam::Vec3::new(70.0, 20.0, 100.0),
            centre + glam::Vec3::new(-400.0, 180.0, 200.0)] {
            for height in [1080.0f32, 2160.0] {
                let scale = height / (2.0 * 32f32.to_radians().tan());
                let projected = |error: f32, sphere: [f32; 4]| {
                    if error > 1e30 { 1e30 } else {
                        error * scale / (glam::Vec3::from_slice(&sphere[..3]).distance(camera) - sphere[3]).max(0.1)
                    }
                };
                check(selected_triangles(mesh, |c| projected(c.own_error, c.own) <= 1.0
                    && projected(c.parent_error, c.parent) > 1.0));
            }
        }
    }

    #[test]
    fn explicit_vertex_constraints_keep_a_nonplanar_strip_through_every_cut() {
        let (vertices, indices) = folded_grid();
        let lock = |index: usize| (14..=18).contains(&(index % SIDE));
        let parts = [Part { vertices: &vertices, indices: &indices, material: 7 }];
        let mesh = ClusterMesh::build_with_constraints(&parts, |_| SimplificationPolicy::Geometry, |_, i| lock(i));
        let protected: Vec<_> = indices.chunks_exact(3).filter(|tri| tri.iter().all(|i| lock(*i as usize)))
            .map(|tri| [tri[0], tri[1], tri[2]]).collect();
        assert_eq!(protected.len(), 256);
        assert!(vertices[SIDE * 4 + 16].pos[1] - vertices[SIDE * 12 + 16].pos[1] > 0.3,
            "fixture must have real noncoplanar relief");
        assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&mesh.vertices), bytemuck::cast_slice::<Vertex, u8>(&vertices));
        assert_retained(&mesh, &protected);
        assert!(mesh.root_triangles() < indices.len() / 3, "unlocked surface must still simplify");
        assert!(mesh.levels > 1);
    }

    #[test]
    fn explicit_vertex_constraints_add_to_terrain_locks_and_use_part_local_indices() {
        let (vertices, indices) = folded_grid();
        let parts = [Part { vertices: &vertices, indices: &indices, material: 3 },
            Part { vertices: &vertices, indices: &indices, material: 8 }];
        let policy = |part: &Part<'_>| if part.material == 8 { SimplificationPolicy::Terrain } else { SimplificationPolicy::Geometry };
        let mesh = ClusterMesh::build_with_constraints(&parts, policy,
            |part, index| part.material == 8 && (27..=29).contains(&(index % SIDE)));
        let expected: Vec<_> = indices.chunks_exact(3).filter(|tri| tri.iter().all(|&i|
            vertices[i as usize].uv[0] <= 5.0 || (27..=29).contains(&(i as usize % SIDE))))
            .map(|tri| [tri[0], tri[1], tri[2]].map(|i| i + vertices.len() as u32)).collect();
        assert!(expected.len() > 500);
        assert_retained(&mesh, &expected);
        // The first part has no locks: a second part's local index must not
        // accidentally pin the corresponding vertices in that first part.
        let reference = ClusterMesh::build(&parts[..1]);
        let first: Vec<_> = mesh.clusters.iter().filter(|c| c.material == 3).copied().collect();
        assert_eq!(bytemuck::cast_slice::<GpuCluster, u8>(&first), bytemuck::cast_slice::<GpuCluster, u8>(&reference.clusters));
    }

    #[test]
    fn all_false_constraints_preserve_every_existing_policy_byte_for_byte() {
        let (vertices, indices) = folded_grid();
        let parts = [Part { vertices: &vertices, indices: &indices, material: 4 }];
        for policy in [SimplificationPolicy::Geometry, SimplificationPolicy::Terrain,
            SimplificationPolicy::TexturedGeometry, SimplificationPolicy::AlphaCards,
            SimplificationPolicy::ThinFoliage] {
            let baseline = ClusterMesh::build_with_policy(&parts, |_| policy);
            let constrained = ClusterMesh::build_with_constraints(&parts, |_| policy, |_, _| false);
            assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&baseline.vertices), bytemuck::cast_slice::<Vertex, u8>(&constrained.vertices));
            assert_eq!(bytemuck::cast_slice::<GpuCluster, u8>(&baseline.clusters), bytemuck::cast_slice::<GpuCluster, u8>(&constrained.clusters));
            assert_eq!(baseline.cluster_verts, constrained.cluster_verts);
            assert_eq!(baseline.tris, constrained.tris);
            assert_eq!(baseline.sphere, constrained.sphere);
            assert_eq!(baseline.levels, constrained.levels);
        }
    }
}

/// Thin-surface coverage is not represented by positional simplification error:
/// a planar card can shrink without moving any surviving point off its plane,
/// and a tiny closed leaf can collapse while its whole crown remains visible.
/// Carry original projected-area credit through the DAG and include cumulative
/// area loss in its existing world-space error, with no new GPU data or work.
/// This is summed geometric area, not alpha/opaque-union coverage (leaves can
/// overlap, and both sides of a closed leaf contribute). That multiplicity is
/// retained in source and replacement credit; no alpha data is read or changed.
fn protect_card_coverage(vertices: &[Vertex], groups: &mut [[f32; 5]], pending: &[PendingCluster]) {
    let areas:Vec<_>=pending.iter().map(|(_,_,indices,_)|projected_triangle_area(vertices,indices)).collect();
    let mut replacement=vec![0.0f32;groups.len()];
    let mut replacement_count=vec![0usize;groups.len()];
    for ((_,refined,_,_),area) in pending.iter().zip(&areas) {
        if *refined>=0 {replacement[*refined as usize]+=area;replacement_count[*refined as usize]+=1;}
    }
    let mut original=vec![0.0f32;groups.len()];
    // Meshopt emits children before their parents. Distribute each refined
    // group's original credit over all its replacement clusters, preserving
    // the total even when those clusters later enter different parent groups.
    for gid in 0..groups.len() {
        let mut inherited_error=0.0f32;
        for ((group,refined,_,_),area) in pending.iter().zip(&areas).filter(|((g,_,_,_),_)|*g==gid) {
            debug_assert_eq!(*group,gid);
            if *refined<0 {original[gid]+=area;} else {
                let child=*refined as usize;
                debug_assert!(child<gid);
                let retained=replacement[child];
                let credit=if retained>1e-12 {area/retained*original[child]} else {original[child]/replacement_count[child].max(1) as f32};
                original[gid]+=credit;
                inherited_error=inherited_error.max(groups[child][4]);
            }
        }
        if groups[gid][4]>1e30 {continue;} // terminal group has no replacement
        let loss=if original[gid]>1e-12 {1.0-(replacement[gid]/original[gid]).clamp(0.0,1.0)} else {0.0};
        // At a 1 px error target this allows <=26.25% loss at 128 px
        // projected group radius, <=32.5% at64px and <=45% at32px.
        // A heavily depleted root is still allowed for tiny distant crowns.
        let coverage_error=groups[gid][3]*(loss-0.20).max(0.0)/8.0;
        groups[gid][4]=groups[gid][4].max(coverage_error).max(inherited_error);
    }
}

/// Thin foliage built from many small separate pieces (needles, leaves) has
/// no edge collapse that keeps its look: merging two needles makes one bent
/// sliver (protect_surface_orientation then charges half its edge), so a
/// spruce brush jumped from all 896 triangles straight to 8 cm of error and
/// stayed at full detail to ~90 m (Joep's near-tree voxel balls, 13 November:
/// the source budget held a handful of trees). These parts thin by pieces
/// instead, as foliage LODs do: each level keeps a spatially even half of
/// the small pieces (Morton order, nested, so pieces never come back) and
/// widens them across their long axis so the projected area stays the same.
/// The error is how far coverage moves: the RMS distance from each piece to
/// the nearest kept one, or how far a kept piece's side grows.
/// The policy for thin plant sources (cook_source and the aggregate cook):
/// ThinPieces, or ThinFoliage's edge collapse with KERN_THIN_PIECES=0.
pub fn thin_source_policy() -> SimplificationPolicy {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var("KERN_THIN_PIECES").as_deref() != Ok("0")) {
        SimplificationPolicy::ThinPieces
    } else {
        SimplificationPolicy::ThinFoliage
    }
}

/// Fewer small pieces than this keep edge collapse.
const MIN_THIN_PIECES: usize = 32;

/// Per level: the triangles (indices into the part's vertices, before the
/// level's widening), the widening and the error. Level 0 is the part itself.
struct PieceLevel { pieces: Vec<usize>, widen: f32, error: f32 }

struct Piece { tris: Vec<u32>, verts: Vec<u32>, centre: glam::Vec3, axis: glam::Vec3, side: f32, length: f32 }

fn pieces_of(part: &Part<'_>) -> Vec<Piece> {
    let n = part.vertices.len();
    let mut parent: Vec<u32> = (0..n as u32).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 { while p[x as usize] != x { p[x as usize] = p[p[x as usize] as usize]; x = p[x as usize]; } x }
    for tri in part.indices.chunks_exact(3) {
        let a = find(&mut parent, tri[0]);
        for &v in &tri[1..] { let b = find(&mut parent, v); parent[b as usize] = a; }
    }
    let mut index = std::collections::HashMap::<u32, usize>::new();
    let mut pieces: Vec<(Vec<u32>, Vec<u32>)> = Vec::new();
    for tri in part.indices.chunks_exact(3) {
        let root = find(&mut parent, tri[0]);
        let k = *index.entry(root).or_insert_with(|| { pieces.push(Default::default()); pieces.len() - 1 });
        pieces[k].0.extend_from_slice(tri);
        for &v in tri { if !pieces[k].1.contains(&v) { pieces[k].1.push(v); } }
    }
    pieces.into_iter().map(|(tris, verts)| {
        let at = |v: u32| glam::Vec3::from(part.vertices[v as usize].pos);
        let centre = verts.iter().map(|&v| at(v)).sum::<glam::Vec3>() / verts.len() as f32;
        let far = |from: glam::Vec3| verts.iter().map(|&v| at(v)).max_by(|a, b| a.distance_squared(from).total_cmp(&b.distance_squared(from))).unwrap();
        let a = far(centre);
        let b = far(a);
        let axis = (b - a).normalize_or(glam::Vec3::Y);
        let side = verts.iter().map(|&v| { let d = at(v) - centre; (d - axis * d.dot(axis)).length() }).fold(0.0f32, f32::max);
        Piece { tris, verts, centre, axis, side, length: a.distance(b) }
    }).collect()
}

/// A piece's surface area with its sides pushed `w` times further from its axis.
fn piece_surface(part: &Part<'_>, piece: &Piece, w: f32) -> f32 {
    let at = |v: u32| {
        let d = glam::Vec3::from(part.vertices[v as usize].pos) - piece.centre;
        let along = piece.axis * d.dot(piece.axis);
        piece.centre + along + (d - along) * w
    };
    piece.tris.chunks_exact(3).map(|t| (at(t[1]) - at(t[0])).cross(at(t[2]) - at(t[0])).length() * 0.5).sum()
}

fn pieces_median(pieces: &[Piece]) -> f32 {
    let mut l: Vec<f32> = pieces.iter().map(|p| p.length).collect();
    l.sort_by(f32::total_cmp);
    l[l.len() / 2]
}

fn piece_levels(part: &Part<'_>) -> Option<(Vec<Piece>, Vec<PieceLevel>)> {
    let pieces = pieces_of(part);
    if pieces.len() < MIN_THIN_PIECES { return None; }
    let median = pieces_median(&pieces);
    let mut tris: Vec<usize> = pieces.iter().map(|p| p.tris.len() / 3).collect();
    tris.sort_unstable();
    // Needles and leaves: many alike pieces of a few triangles each.
    if tris[tris.len() / 2] > 16 || !(median > 0.0) { return None; }
    // Stalks and twigs (much longer than the typical piece) are structure:
    // every level keeps them as they are.
    let small: Vec<usize> = (0..pieces.len()).filter(|&i| pieces[i].length <= 2.0 * median).collect();
    let structure: Vec<usize> = (0..pieces.len()).filter(|&i| pieces[i].length > 2.0 * median).collect();
    if small.len() < MIN_THIN_PIECES { return None; }
    let (lo, hi) = small.iter().fold((glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN)), |(lo, hi), &i| (lo.min(pieces[i].centre), hi.max(pieces[i].centre)));
    let span = (hi - lo).max(glam::Vec3::splat(1e-9));
    let morton = |p: glam::Vec3| -> u64 {
        let q = ((p - lo) / span * 1023.0).round().clamp(glam::Vec3::ZERO, glam::Vec3::splat(1023.0)).as_uvec3();
        (0..10).fold(0u64, |m, b| m | (((q.x >> b) & 1) as u64) << (3 * b) | (((q.y >> b) & 1) as u64) << (3 * b + 1) | (((q.z >> b) & 1) as u64) << (3 * b + 2))
    };
    let mut order = small.clone();
    order.sort_by_key(|&i| morton(pieces[i].centre));
    let mut levels = vec![PieceLevel { pieces: (0..pieces.len()).collect(), widen: 1.0, error: 0.0 }];
    let mut step = 1usize;
    // Halve until a handful of pieces is left: then the root is cheap.
    while order.len() / (step * 2) >= 4 {
        step *= 2;
        let kept: Vec<usize> = order.iter().copied().step_by(step).collect();
        // Widen until the kept pieces' surface (so their mean projected area,
        // Cauchy) matches all of them: about order/kept for slivers, less once
        // the widened ends show.
        let surface = |pieces_in: &[usize], w: f32| pieces_in.iter().map(|&i| piece_surface(part, &pieces[i], w)).sum::<f32>();
        let target = surface(&order, 1.0);
        let (mut lo_w, mut hi_w) = (1.0f32, order.len() as f32 / kept.len() as f32);
        for _ in 0..32 { let mid = 0.5 * (lo_w + hi_w); if surface(&kept, mid) < target { lo_w = mid } else { hi_w = mid } }
        let widen = 0.5 * (lo_w + hi_w);
        let mut error = levels.last().unwrap().error;
        // How far coverage moves, as the RMS over the thinned pieces: a
        // needle is sub-pixel long before its level is chosen, so what shows
        // is where the brush's coverage sits on average. Its maximum (an
        // isolated needle whose neighbour went, ~6x the mean) kept every
        // brush at full detail to ~45 m.
        let moves = order.iter().map(|&i| kept.iter().map(|&k| pieces[k].centre.distance_squared(pieces[i].centre)).fold(f32::MAX, f32::min));
        error = error.max((moves.sum::<f32>() / order.len() as f32).sqrt());
        for &k in &kept { error = error.max((widen - 1.0) * pieces[k].side); }
        // Strictly increasing, so exactly one level matches any threshold.
        let error = error.max(levels.last().unwrap().error * 1.0001 + 1e-9);
        levels.push(PieceLevel { pieces: structure.iter().copied().chain(kept).collect(), widen, error });
    }
    if levels.len() < 2 { return None; }
    Some((pieces, levels))
}

/// Cluster data for a mesh, ready to append to the scene buffers. Indices
/// in `clusters` are local to these arrays; the scene rebases them.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct ClusterMesh {
    pub vertices: Vec<Vertex>,
    /// Per cluster, the mesh vertices it uses (cluster-local index -> vertex).
    pub cluster_verts: Vec<u32>,
    /// Per triangle, three cluster-local indices packed in one u32.
    pub tris: Vec<u32>,
    pub clusters: Vec<GpuCluster>,
    /// Bounding sphere of the whole mesh.
    pub sphere: [f32; 4],
    pub levels: u32,
}

impl ClusterMesh {
    pub fn build(parts: &[Part]) -> ClusterMesh {
        Self::build_with_policy(parts, |_| SimplificationPolicy::Geometry)
    }

    /// Build source-only clusters for deforming meshes without simplification.
    /// Every part's vertices are appended unchanged, including unreferenced
    /// vertices and parts without triangles, so parallel skinning data keeps
    /// the same vertex order. Triangle order, winding and materials also stay
    /// unchanged; only their cluster-local index maps are constructed.
    ///
    /// `sphere` must conservatively enclose all supported deformations in mesh
    /// space. It is used for the mesh and every cluster's culling/LOD bounds.
    /// Indices must form complete triangles and refer to vertices in their part.
    pub fn build_fixed(parts: &[Part], sphere: [f32; 4]) -> ClusterMesh {
        const MAX_VERTS: usize = 256;
        assert!(sphere.iter().all(|v| v.is_finite()) && sphere[3] >= 0.0,
            "fixed mesh bounds must be a finite sphere with nonnegative radius");
        let mut out = ClusterMesh { sphere, ..Default::default() };
        for part in parts {
            assert_eq!(part.indices.len() % 3, 0, "fixed mesh indices must form complete triangles");
            assert!(part.indices.iter().all(|&i| (i as usize) < part.vertices.len()),
                "fixed mesh index is outside its part's vertices");
            let base = out.vertices.len() as u32;
            out.vertices.extend_from_slice(part.vertices);
            let mut at = 0;
            while at < part.indices.len() {
                let first_vert = out.cluster_verts.len() as u32;
                let first_tri = out.tris.len() as u32;
                let mut local = std::collections::HashMap::<u32, u32>::with_capacity(MAX_VERTS);
                while at < part.indices.len() && out.tris.len() - (first_tri as usize) < MAX_TRIS {
                    let triangle = &part.indices[at..at + 3];
                    let added = triangle.iter().enumerate().filter(|&(k, index)|
                        !local.contains_key(index) && !triangle[..k].contains(index)).count();
                    if local.len() + added > MAX_VERTS { break; }
                    let mut packed = 0;
                    for (k, &index) in triangle.iter().enumerate() {
                        let next = local.len() as u32;
                        let cluster_index = *local.entry(index).or_insert_with(|| {
                            out.cluster_verts.push(base + index);
                            next
                        });
                        packed |= cluster_index << (k * 8);
                    }
                    out.tris.push(packed);
                    at += 3;
                }
                out.clusters.push(GpuCluster {
                    cull: sphere,
                    parent: sphere,
                    own: sphere,
                    // A source leaf with no replacement is selected at every
                    // LOD threshold, matching the renderer's terminal sentinel.
                    parent_error: f32::MAX,
                    own_error: 0.0,
                    first_tri,
                    tri_count: out.tris.len() as u32 - first_tri,
                    first_vert,
                    material: part.material,
                    depth: 0,
                    order: 0,
                });
            }
        }
        out.levels = u32::from(!out.clusters.is_empty());
        out
    }

    pub fn build_with_policy(parts: &[Part], policy: impl Fn(&Part<'_>) -> SimplificationPolicy) -> ClusterMesh {
        Self::build_configured(parts, policy, None)
    }

    /// Keep caller-selected source vertices fixed through every LOD cut.
    /// `locked` receives an index local to that part's `vertices` and is
    /// evaluated once per vertex of each nonempty part during cooking only.
    /// Locks add to the selected policy's own constraints; they never remove
    /// them. Lock every vertex of a source triangle to preserve that triangle,
    /// for example where independently simplified surfaces must meet exactly.
    /// Other vertices remain eligible for simplification. No extra runtime
    /// buffers or shader work are introduced.
    pub fn build_with_constraints(
        parts: &[Part],
        policy: impl Fn(&Part<'_>) -> SimplificationPolicy,
        locked: impl Fn(&Part<'_>, usize) -> bool,
    ) -> ClusterMesh {
        Self::build_configured(parts, policy, Some(&locked))
    }

    fn build_configured(
        parts: &[Part],
        policy: impl Fn(&Part<'_>) -> SimplificationPolicy,
        locked: Option<&dyn Fn(&Part<'_>, usize) -> bool>,
    ) -> ClusterMesh {
        let mut out = ClusterMesh::default();
        for part in parts {
            if part.indices.is_empty() {
                continue;
            }
            let mut selected_policy = policy(part);
            if selected_policy == SimplificationPolicy::ThinPieces {
                if let Some(levels) = locked.is_none().then(|| piece_levels(part)).flatten() {
                    out.append_piece_levels(part, &levels);
                    continue;
                }
                selected_policy = SimplificationPolicy::ThinFoliage;
            }
            let base = out.vertices.len() as u32;
            out.vertices.extend_from_slice(part.vertices);
            let positions: &[u8] = bytemuck::cast_slice(part.vertices);
            let adapter = meshopt::VertexDataAdapter::new(positions, std::mem::size_of::<Vertex>(), 0).expect("vertex adapter");
            let mut mesh = ClodMesh::new(part.indices, &adapter);
            let mut config = clod_default_config(MAX_TRIS);
            config.max_triangles = MAX_TRIS;
            // Keep this storage alive for the synchronous clod_build callback.
            let attributes: Vec<f32>;
            let mut vertex_locks = Vec::new();
            let thin_foliage=selected_policy == SimplificationPolicy::ThinFoliage;
            let coverage=selected_policy == SimplificationPolicy::AlphaCards || thin_foliage;
            let terrain=selected_policy == SimplificationPolicy::Terrain;
            if selected_policy != SimplificationPolicy::Geometry {
                attributes = part.vertices.iter().flat_map(|v| [v.normal[0], v.normal[1], v.normal[2],
                    if terrain {v.uv[0].clamp(0.0,32.0)}else{v.uv[0]},if terrain {v.uv[1].clamp(0.0,2.0)}else{v.uv[1]}]).collect();
                mesh.vertex_attributes = &attributes;
                mesh.vertex_attributes_stride = 5 * std::mem::size_of::<f32>();
                // Terrain uv.x stores metres from the road. Only the first
                // 32 m affect material masks; distant values must not keep
                // needless geometry. These weights retain smooth gradients
                // without treating a metre of road distance as a full UV seam.
                // Terrain uv.y carries 1+macro-up (0 means legacy). Preserve
                // the geology mask when detailed rock normals simplify.
                mesh.attribute_weights = if terrain {&[0.1,0.1,0.1,0.02,0.15]}else{&[0.25,0.25,0.25,1.0,1.0]};
                mesh.attribute_protect_mask = if terrain {0}else{(1 << 3) | (1 << 4)};
                config.simplify_preserve_folds = !terrain;
                // The positional-only fallback discards the attributes again
                // and can erase disconnected branch cards even if UVs matter.
                config.simplify_fallback_sloppy = false;
                if terrain {
                    // The separately rendered road's outer shoulder follows
                    // these exact native triangles. Keep a narrow corridor
                    // intact at every cut, including the sun's coarse cut.
                    // Five metres includes the shoulder plus native-cell halo.
                    vertex_locks=part.vertices.iter().map(|v|if v.uv[0]<=5.0 {
                        meshopt::SimplifyVertex::Lock
                    }else{meshopt::SimplifyVertex::None}).collect();
                }
            }
            if let Some(locked) = locked {
                for index in 0..part.vertices.len() {
                    if locked(part, index) {
                        // An all-false callback retains the old policy's exact
                        // configuration and does not allocate a lock stream.
                        if vertex_locks.is_empty() {
                            vertex_locks.resize_with(part.vertices.len(), || meshopt::SimplifyVertex::None);
                        }
                        vertex_locks[index] = meshopt::SimplifyVertex::Lock;
                    }
                }
            }
            if !vertex_locks.is_empty() {
                mesh.vertex_lock = Some(&vertex_locks);
                // Sloppy fallback is positional-only and does not honor the
                // caller's hard topology constraints. Keep the normal locked
                // simplifier even when a constrained group cannot shrink.
                config.simplify_fallback_sloppy = false;
            }
            let mut groups: Vec<[f32; 5]> = Vec::new(); // simplified bounds per group
            let mut pending: Vec<PendingCluster> = Vec::new(); // (group, refined, indices, cull sphere)
            clod_build(config, &mesh, |group, clusters| {
                let gid = groups.len();
                let b = group.simplified;
                groups.push([b.center[0], b.center[1], b.center[2], b.radius, b.error]);
                for c in clusters {
                    let cb = c.bounds;
                    pending.push((gid, c.refined, c.indices.to_vec(), [cb.center[0], cb.center[1], cb.center[2], cb.radius]));
                }
                out.levels = out.levels.max(group.depth as u32 + 1);
                gid as i32
            });
            if selected_policy == SimplificationPolicy::TexturedGeometry || thin_foliage {
                protect_surface_orientation(part, &mut groups, &pending);
                #[cfg(test)]
                policy_tests::assert_complete_dag_cuts(&groups, &pending);
            }
            if coverage {
                protect_card_coverage(part.vertices,&mut groups,&pending);
                #[cfg(test)]
                policy_tests::assert_complete_dag_cuts(&groups,&pending);
            }
            for (gid, refined, indices, cull) in pending {
                let g = groups[gid];
                let (own, own_error) = if refined < 0 {
                    ([cull[0], cull[1], cull[2], cull[3]], 0.0)
                } else {
                    let r = groups[refined as usize];
                    ([r[0], r[1], r[2], r[3]], r[4])
                };
                let (verts, local) = clod_local_indices(&indices);
                let first_vert = out.cluster_verts.len() as u32;
                out.cluster_verts.extend(verts.iter().map(|v| v + base));
                let first_tri = out.tris.len() as u32;
                for t in local.chunks(3) {
                    out.tris.push(t[0] as u32 | (t[1] as u32) << 8 | (t[2] as u32) << 16);
                }
                out.clusters.push(GpuCluster {
                    cull,
                    parent: [g[0], g[1], g[2], g[3]],
                    own,
                    parent_error: g[4],
                    own_error,
                    first_tri,
                    tri_count: (local.len() / 3) as u32,
                    first_vert,
                    material: part.material,
                    depth: 0,
                    order: 0,
                });
            }
        }
        out.sphere = bounding_sphere(&out.vertices);
        out
    }

    /// One discrete level per PieceLevel (piece_levels): every cluster of a
    /// level shares the part's sphere for its LOD test, so a whole level
    /// switches at once, without gaps or double draws.
    fn append_piece_levels(&mut self, part: &Part<'_>, (pieces, levels): &(Vec<Piece>, Vec<PieceLevel>)) {
        const MAX_VERTS: usize = 256;
        let first_cluster = self.clusters.len();
        let first_vertex = self.vertices.len();
        let median = pieces_median(pieces);
        for (k, level) in levels.iter().enumerate() {
            // Each level owns its vertices: widened pieces move.
            let mut remap = std::collections::HashMap::<u32, u32>::new();
            let mut tris = Vec::new();
            for &p in &level.pieces {
                let piece = &pieces[p];
                // Structure pieces (stalks) are not thinned and keep their width.
                let w = if k > 0 && piece.length <= 2.0 * median { level.widen } else { 1.0 };
                for &v in &piece.verts {
                    let mut vertex = part.vertices[v as usize];
                    let d = glam::Vec3::from(vertex.pos) - piece.centre;
                    let along = piece.axis * d.dot(piece.axis);
                    vertex.pos = (piece.centre + along + (d - along) * w).to_array();
                    remap.insert(v, self.vertices.len() as u32);
                    self.vertices.push(vertex);
                }
                tris.extend(piece.tris.iter().map(|v| remap[v]));
            }
            let own_error = level.error;
            let parent_error = levels.get(k + 1).map(|l| l.error).unwrap_or(f32::MAX);
            let mut at = 0;
            while at < tris.len() {
                let first_vert = self.cluster_verts.len() as u32;
                let first_tri = self.tris.len() as u32;
                let mut local = std::collections::HashMap::<u32, u32>::with_capacity(MAX_VERTS);
                while at < tris.len() && self.tris.len() - (first_tri as usize) < MAX_TRIS {
                    let triangle = &tris[at..at + 3];
                    let added = triangle.iter().enumerate().filter(|&(j, index)| !local.contains_key(index) && !triangle[..j].contains(index)).count();
                    if local.len() + added > MAX_VERTS { break; }
                    let mut packed = 0;
                    for (j, &index) in triangle.iter().enumerate() {
                        let next = local.len() as u32;
                        let cluster_index = *local.entry(index).or_insert_with(|| { self.cluster_verts.push(index); next });
                        packed |= cluster_index << (j * 8);
                    }
                    self.tris.push(packed);
                    at += 3;
                }
                let used: Vec<Vertex> = self.cluster_verts[first_vert as usize..].iter().map(|&v| self.vertices[v as usize]).collect();
                let cull = bounding_sphere(&used);
                self.clusters.push(GpuCluster { cull, parent: cull, own: cull, parent_error, own_error, first_tri,
                    tri_count: self.tris.len() as u32 - first_tri, first_vert, material: part.material, depth: k as u32, order: 0 });
            }
        }
        let sphere = bounding_sphere(&self.vertices[first_vertex..]);
        for c in &mut self.clusters[first_cluster..] { c.parent = sphere; c.own = sphere; }
        self.levels = self.levels.max(levels.len() as u32);
    }

    /// Triangles at full detail (the DAG's leaves).
    pub fn leaf_triangles(&self) -> usize {
        self.clusters.iter().filter(|c| c.own_error == 0.0).map(|c| c.tri_count as usize).sum()
    }

    /// Append a coarse whole-mesh replacement level that activates once `error`
    /// (in mesh-space metres) projects to at most the view's LOD threshold.
    /// Fine root clusters and coarse clusters share the combined mesh bounding
    /// sphere at the `error` transition so GPU cluster selection switches
    /// atomically without gaps or double draws.
    pub fn with_coarse(mut self, coarse: ClusterMesh, error: f32) -> ClusterMesh {
        if self.clusters.is_empty() || coarse.clusters.is_empty() || !error.is_finite() || error <= 0.0 {
            return self;
        }
        let vbase = self.vertices.len() as u32;
        let cvbase = self.cluster_verts.len() as u32;
        let tbase = self.tris.len() as u32;
        self.vertices.extend_from_slice(&coarse.vertices);
        self.cluster_verts.extend(coarse.cluster_verts.iter().map(|v| v + vbase));
        self.tris.extend_from_slice(&coarse.tris);
        let sphere = bounding_sphere(&self.vertices);
        let cut_error = self.clusters.iter().map(|c| c.own_error).fold(error, |acc, e| acc.max(e + 1e-4));
        for c in &mut self.clusters {
            c.cull = sphere;
            if c.parent_error > 1e30 {
                c.parent = sphere;
                c.parent_error = cut_error;
            } else {
                c.parent = sphere;
                c.parent_error = c.parent_error.min(cut_error);
            }
            if c.own_error > 0.0 {
                c.own = sphere;
            }
        }
        let base_depth = self.levels;
        for mut c in coarse.clusters {
            if c.own_error > 0.0 { continue; }
            c.first_vert += cvbase;
            c.first_tri += tbase;
            c.cull = sphere;
            c.own = sphere;
            c.parent = sphere;
            c.own_error = cut_error;
            c.parent_error = f32::MAX;
            c.depth = base_depth;
            self.clusters.push(c);
        }
        self.sphere = sphere;
        self.levels = base_depth + 1;
        self
    }
}

#[cfg(test)]
mod fixed_topology_tests {
    use super::*;

    fn vertices(count: usize, offset: f32) -> Vec<Vertex> {
        (0..count).map(|i| Vertex {
            pos: [offset + i as f32, i as f32 * 0.25, -offset],
            normal: [0.0, 0.6, 0.8],
            uv: [i as f32 * 0.125, offset],
        }).collect()
    }

    fn triangles(mesh: &ClusterMesh) -> Vec<(u32, [u32; 3])> {
        mesh.clusters.iter().flat_map(|cluster| {
            mesh.tris[cluster.first_tri as usize..(cluster.first_tri + cluster.tri_count) as usize]
                .iter().map(move |&packed| {
                    assert_eq!(packed >> 24, 0, "only three byte-sized local indices are packed");
                    (cluster.material, std::array::from_fn(|k|
                        mesh.cluster_verts[cluster.first_vert as usize + ((packed >> (k * 8)) & 255) as usize]))
                })
        }).collect()
    }

    #[test]
    fn fixed_preserves_every_vertex_and_authored_triangle_across_parts() {
        let leading = vertices(2, -2.0);
        let first = vertices(5, 0.0);
        let unused = vertices(1, 10.0);
        let second = vertices(4, 20.0);
        let parts = [
            Part { vertices: &leading, indices: &[], material: 1 },
            Part { vertices: &first, indices: &[3, 1, 2, 3, 2, 0], material: 7 },
            Part { vertices: &unused, indices: &[], material: 2 },
            Part { vertices: &second, indices: &[2, 0, 3, 0, 0, 1], material: 19 },
        ];
        let sphere = [4.0, 7.0, -2.0, 90.0];
        let mesh = ClusterMesh::build_fixed(&parts, sphere);
        let authored: Vec<_> = parts.iter().flat_map(|part| part.vertices.iter().copied()).collect();
        assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&mesh.vertices), bytemuck::cast_slice::<Vertex, u8>(&authored));
        assert_eq!(triangles(&mesh), vec![(7, [5, 3, 4]), (7, [5, 4, 2]), (19, [10, 8, 11]), (19, [8, 8, 9])]);
        assert_eq!(mesh.sphere, sphere);
        assert_eq!(mesh.levels, 1);
        assert_eq!(mesh.leaf_triangles(), 4);
        assert_eq!(mesh.root_triangles(), 4);
        for cluster in &mesh.clusters {
            assert_eq!(cluster.cull, sphere);
            assert_eq!(cluster.parent, sphere);
            assert_eq!(cluster.own, sphere);
            assert_eq!(cluster.own_error, 0.0);
            assert_eq!(cluster.parent_error, f32::MAX);
            for threshold in [0.0, 1.0, 1e29] {
                assert!(cluster.own_error <= threshold && cluster.parent_error > threshold);
            }
        }
    }

    #[test]
    fn fixed_splits_at_both_local_vertex_and_triangle_limits() {
        let sparse = vertices(257, 0.0);
        let dense = vertices(3, 300.0);
        let mut sparse_indices: Vec<u32> = (0..255).collect();
        sparse_indices.extend([0, 254, 255, 0, 255, 256]);
        let dense_indices: Vec<u32> = (0..MAX_TRIS + 1).flat_map(|_| [0, 1, 2]).collect();
        let parts = [
            Part { vertices: &sparse, indices: &sparse_indices, material: 11 },
            Part { vertices: &dense, indices: &dense_indices, material: 13 },
        ];
        let mesh = ClusterMesh::build_fixed(&parts, [0.0, 0.0, 0.0, 1000.0]);
        assert_eq!(mesh.clusters.iter().map(|cluster| cluster.tri_count).collect::<Vec<_>>(), vec![86, 1, 128, 1]);
        let mut expected = Vec::new();
        let mut base = 0;
        for part in &parts {
            expected.extend(part.indices.chunks_exact(3).map(|triangle|
                (part.material, [triangle[0] + base, triangle[1] + base, triangle[2] + base])));
            base += part.vertices.len() as u32;
        }
        assert_eq!(triangles(&mesh), expected);
        for (index, cluster) in mesh.clusters.iter().enumerate() {
            let end = mesh.clusters.get(index + 1).map_or(mesh.cluster_verts.len(), |next| next.first_vert as usize);
            let count = end - cluster.first_vert as usize;
            assert!(count <= 256);
            assert!(cluster.tri_count as usize <= MAX_TRIS);
            if index == 0 { assert_eq!(count, 256); }
            for &packed in &mesh.tris[cluster.first_tri as usize..(cluster.first_tri + cluster.tri_count) as usize] {
                for k in 0..3 { assert!((((packed >> (k * 8)) & 255) as usize) < count); }
            }
        }
    }

    #[test]
    fn fixed_empty_mesh_keeps_supplied_bounds_without_clusters() {
        let sphere = [1.0, 2.0, 3.0, 4.0];
        let mesh = ClusterMesh::build_fixed(&[], sphere);
        assert_eq!(mesh.sphere, sphere);
        assert_eq!(mesh.levels, 0);
        assert!(mesh.vertices.is_empty() && mesh.cluster_verts.is_empty() && mesh.tris.is_empty() && mesh.clusters.is_empty());
    }
}

fn bounding_sphere(v: &[Vertex]) -> [f32; 4] {
    if v.is_empty() {
        return [0.0; 4];
    }
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in v {
        for k in 0..3 {
            lo[k] = lo[k].min(p.pos[k]);
            hi[k] = hi[k].max(p.pos[k]);
        }
    }
    let c = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
    let r = v.iter().map(|p| ((p.pos[0] - c[0]).powi(2) + (p.pos[1] - c[1]).powi(2) + (p.pos[2] - c[2]).powi(2)).sqrt()).fold(0.0f32, f32::max);
    [c[0], c[1], c[2], r]
}

/// Read a .kmesh written by tools/export_meshes.gd: (material name, vertices, indices) per surface.
pub fn read_kmesh(bytes: &[u8]) -> Result<Vec<(String, Vec<Vertex>, Vec<u32>)>, String> {
    let mut at = 0usize;
    let u32_ = |at: &mut usize| -> Result<u32, String> {
        let b = bytes.get(*at..*at + 4).ok_or("kmesh truncated")?;
        *at += 4;
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    };
    if bytes.get(0..4) != Some(b"KMSH") {
        return Err("not a kmesh".into());
    }
    at += 4;
    let surfaces = u32_(&mut at)?;
    let mut out = Vec::new();
    for _ in 0..surfaces {
        let n = u32_(&mut at)? as usize;
        let name = String::from_utf8_lossy(&bytes[at..at + n]).to_string();
        at += n;
        let vc = u32_(&mut at)? as usize;
        let ic = u32_(&mut at)? as usize;
        let f = |at: usize, k: usize| f32::from_le_bytes(bytes[at + k * 4..at + k * 4 + 4].try_into().unwrap());
        let mut verts = vec![Vertex { pos: [0.0; 3], normal: [0.0; 3], uv: [0.0; 2] }; vc];
        for (i, v) in verts.iter_mut().enumerate() {
            v.pos = [f(at, i * 3), f(at, i * 3 + 1), f(at, i * 3 + 2)];
        }
        at += vc * 12;
        for (i, v) in verts.iter_mut().enumerate() {
            v.normal = [f(at, i * 3), f(at, i * 3 + 1), f(at, i * 3 + 2)];
        }
        at += vc * 12;
        for (i, v) in verts.iter_mut().enumerate() {
            v.uv = [f(at, i * 2), f(at, i * 2 + 1)];
        }
        at += vc * 8;
        let mut idx = Vec::with_capacity(ic);
        for _ in 0..ic {
            idx.push(u32_(&mut at)?);
        }
        out.push((name, verts, idx));
    }
    Ok(out)
}

impl ClusterMesh {
    /// Sort the clusters coarsest first: by parent_error descending, the
    /// DAG's roots (parent_error f32::MAX) at the front. Any cut is then
    /// within the prefix whose parent_error exceeds the cut's threshold, so
    /// the cull and the budget need only that prefix (a needle spray far away
    /// draws 16 of its 244 clusters but was culled and booked at all 244).
    /// Triangles and vertices are addressed per cluster and stay in place.
    pub fn sort_coarse_first(&mut self) {
        self.clusters.sort_by(|a, b| b.parent_error.total_cmp(&a.parent_error));
        for cluster in &mut self.clusters { cluster.order |= CLUSTERS_COARSE_FIRST; }
    }

    /// Clusters a cut at `threshold` (mesh-local error) must look at when
    /// sorted coarse first: those whose parent_error exceeds it.
    pub fn coarse_prefix(&self, threshold: f32) -> usize {
        self.clusters.partition_point(|c| c.parent_error > threshold)
    }

    /// Triangles drawn when the whole mesh is far away (the DAG's roots).
    pub fn root_triangles(&self) -> usize {
        self.clusters.iter().filter(|c| c.parent_error > 1e30).map(|c| c.tri_count as usize).sum()
    }
}

#[cfg(test)]
mod thin_piece_tests {
    use super::*;

    /// `count` closed needle slivers (three-sided base, one tip) in a 0.4 m
    /// brush, plus two long stalks: a spruce brush's make-up.
    fn brush(count: usize) -> (Vec<Vertex>, Vec<u32>) {
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        let mut seed = 0x9e37_79b9u32;
        let mut unit = move || { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; seed as f32 / u32::MAX as f32 };
        let mut sliver = |vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, root: glam::Vec3, axis: glam::Vec3, length: f32, width: f32| {
            let side = axis.any_orthonormal_vector();
            let up = axis.cross(side);
            let b = vertices.len() as u32;
            for k in 0..3 {
                let a = k as f32 / 3.0 * std::f32::consts::TAU;
                let out = side * a.cos() + up * a.sin();
                vertices.push(Vertex { pos: (root + out * width).to_array(), normal: out.to_array(), uv: [0.0, 0.0] });
            }
            vertices.push(Vertex { pos: (root + axis * length).to_array(), normal: axis.to_array(), uv: [0.5, 1.0] });
            indices.extend_from_slice(&[b, b + 2, b + 1, b, b + 1, b + 3, b + 1, b + 2, b + 3, b + 2, b, b + 3]);
        };
        for _ in 0..count {
            let root = glam::Vec3::new(unit() - 0.5, unit() - 0.5, unit() - 0.5) * 0.4;
            let axis = glam::Vec3::new(unit() - 0.5, unit() - 0.5, unit() - 0.5).normalize_or(glam::Vec3::Y);
            sliver(&mut vertices, &mut indices, root, axis, 0.02 + 0.01 * unit(), 0.0008);
        }
        for x in [-0.1f32, 0.1] { sliver(&mut vertices, &mut indices, glam::Vec3::new(x, -0.2, 0.0), glam::Vec3::Y, 0.4, 0.004); }
        (vertices, indices)
    }

    fn area(mesh: &ClusterMesh, keep: impl Fn(&GpuCluster) -> bool) -> (usize, glam::Vec3) {
        let mut total = (0, glam::Vec3::ZERO);
        for c in mesh.clusters.iter().filter(|c| keep(c)) {
            for packed in &mesh.tris[c.first_tri as usize..(c.first_tri + c.tri_count) as usize] {
                let p: [glam::Vec3; 3] = std::array::from_fn(|k| glam::Vec3::from(mesh.vertices[mesh.cluster_verts[(c.first_vert + ((packed >> (8 * k)) & 255)) as usize] as usize].pos));
                total.0 += 1;
                total.1 += (p[1] - p[0]).cross(p[2] - p[0]).abs() * 0.5;
            }
        }
        total
    }

    #[test]
    fn needle_brushes_thin_by_needles_and_keep_their_area() {
        let (vertices, indices) = brush(222);
        let mesh = ClusterMesh::build_with_policy(&[Part { vertices: &vertices, indices: &indices, material: 3 }], |_| SimplificationPolicy::ThinPieces);
        assert!(mesh.levels >= 5, "levels {}", mesh.levels);
        let full = area(&mesh, |c| c.own_error == 0.0);
        assert_eq!(full.0, indices.len() / 3, "level 0 is the source");
        // Every threshold selects exactly one whole level: one depth, and the
        // stalks in it unchanged.
        let mut errors: Vec<f32> = mesh.clusters.iter().map(|c| c.own_error).collect();
        errors.sort_by(f32::total_cmp);
        errors.dedup();
        assert_eq!(errors.len() as u32, mesh.levels, "one error per level");
        for pair in errors.windows(2) { assert!(pair[1] > pair[0]); }
        let mut previous = usize::MAX;
        for &t in &errors {
            let cut = |c: &GpuCluster| c.own_error <= t && c.parent_error > t;
            let depths: std::collections::BTreeSet<u32> = mesh.clusters.iter().filter(|c| cut(c)).map(|c| c.depth).collect();
            assert_eq!(depths.len(), 1, "threshold {t} mixes levels {depths:?}");
            // All clusters of a level share one LOD sphere: they switch together.
            let spheres: std::collections::BTreeSet<[u32; 4]> = mesh.clusters.iter().filter(|c| cut(c)).map(|c| c.own.map(f32::to_bits)).collect();
            assert_eq!(spheres.len(), 1);
            let (tris, a) = area(&mesh, cut);
            assert!(tris < previous, "each level is cheaper");
            previous = tris;
            assert!(tris >= 2 * 12, "the stalks stay");
            // Projected area per axis within 15% of the source (widened needles).
            for k in 0..3 { assert!((a[k] / full.1[k] - 1.0).abs() < 0.15, "level at {t}: axis {k} area {}", a[k] / full.1[k]); }
        }
    }

    #[test]
    fn spruce_brushes_thin_far_nearer_than_edge_collapse() {
        // The spruce's own needle brushes: edge collapse merged needles into
        // bent slivers and charged ~8 cm at once, so a tree stayed at full
        // detail to ~90 m. Thinning errors follow the needles' spacing.
        let parts = crate::plant_growth_assembly::Parts::for_look(&crate::plant_growth_assembly::Look::norway_spruce());
        for tuft in &parts.tufts {
            let part = [Part { vertices: &tuft.vertices, indices: &tuft.indices, material: 0 }];
            let first = |m: &ClusterMesh| m.clusters.iter().filter(|c| c.own_error > 0.0).map(|c| c.own_error).fold(f32::MAX, f32::min);
            let (pieces, collapse) = (ClusterMesh::build_with_policy(&part, |_| SimplificationPolicy::ThinPieces), ClusterMesh::build_with_policy(&part, |_| SimplificationPolicy::ThinFoliage));
            assert!(first(&pieces) * 4.0 < first(&collapse), "first thinned level at {} m, edge collapse at {} m", first(&pieces), first(&collapse));
            assert!(pieces.levels >= 5);
        }
    }

    #[test]
    fn few_pieces_keep_edge_collapse() {
        let (vertices, indices) = brush(12);
        let part = [Part { vertices: &vertices, indices: &indices, material: 3 }];
        let pieces = ClusterMesh::build_with_policy(&part, |_| SimplificationPolicy::ThinPieces);
        let collapse = ClusterMesh::build_with_policy(&part, |_| SimplificationPolicy::ThinFoliage);
        assert_eq!(bytemuck::cast_slice::<GpuCluster, u8>(&pieces.clusters), bytemuck::cast_slice::<GpuCluster, u8>(&collapse.clusters));
        assert_eq!(pieces.tris, collapse.tris);
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    /// Closed, disconnected leaves have no alpha mask and are not a terrain
    /// sheet. Each lens has six positions and eight outward triangles; face
    /// vertices are split to retain authored folds and local texture coordinates.
    fn disconnected_closed_leaves() -> (Vec<Vertex>, Vec<u32>) {
        use glam::{Quat, Vec3};
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let local = [Vec3::ZERO, Vec3::new(-0.12,0.20,0.0),
            Vec3::new(0.0,0.20,0.008), Vec3::new(0.12,0.20,0.0),
            Vec3::new(0.0,0.20,-0.008), Vec3::new(0.0,0.45,0.016)];
        for row in 0..12 { for col in 0..14 {
            let phase = col as f32 * 1.71 + row as f32 * 0.43;
            let origin = Vec3::new((col as f32 - 6.5)*0.29, row as f32*0.34, phase.sin()*0.65);
            let rotation = Quat::from_rotation_y(phase) * Quat::from_rotation_z(phase.cos()*0.4);
            for side in 0..4 {
                let a = side+1; let b = (side+1)%4+1;
                for triangle in [[0,b,a],[5,a,b]] {
                    let p = triangle.map(|i| local[i]);
                    let normal = rotation * (p[1]-p[0]).cross(p[2]-p[0]).normalize();
                    for p in p {
                        indices.push(vertices.len() as u32);
                        vertices.push(Vertex {pos:(origin+rotation*p).to_array(),normal:normal.to_array(),
                            uv:[p.x/0.24+0.5,p.y/0.45]});
                    }
                }
            }
        }}
        (vertices,indices)
    }

    /// Independent f64 measurement over the actual selected packed triangles.
    /// This is directional geometric area, not union coverage or radiance.
    fn directional_cut_area(mesh: &ClusterMesh, select: impl Fn(&GpuCluster)->bool) -> (usize, glam::DVec3) {
        let mut count = 0usize;
        let mut area = glam::DVec3::ZERO;
        for cluster in mesh.clusters.iter().filter(|c| select(c)) {
            let centre = glam::DVec3::new(cluster.cull[0] as f64,cluster.cull[1] as f64,cluster.cull[2] as f64);
            for packed in &mesh.tris[cluster.first_tri as usize..(cluster.first_tri+cluster.tri_count) as usize] {
                let points:[glam::DVec3;3]=std::array::from_fn(|k| {
                    let id=mesh.cluster_verts[cluster.first_vert as usize+((packed>>(8*k))&255) as usize];
                    let p=mesh.vertices[id as usize].pos;
                    glam::DVec3::new(p[0] as f64,p[1] as f64,p[2] as f64)
                });
                assert!(points.iter().all(|p| p.distance(centre)<=cluster.cull[3] as f64+1e-5),
                    "selected triangle escaped its unchanged culling sphere");
                area+=(points[1]-points[0]).cross(points[2]-points[0]).abs()*0.5;
                count+=1;
            }
        }
        (count,area)
    }

    #[test]
    fn thin_foliage_keeps_disconnected_closed_leaf_area_without_disabling_lod() {
        let (vertices,indices)=disconnected_closed_leaves();
        let parts=[Part {vertices:&vertices,indices:&indices,material:17}];
        let positional=ClusterMesh::build_with_policy(&parts,|_|SimplificationPolicy::TexturedGeometry);
        let foliage=ClusterMesh::build_with_policy(&parts,|_|SimplificationPolicy::ThinFoliage);
        // The extra error credit must not alter source geometry, any actual
        // simplified triangle, material IDs, culling bounds or DAG topology.
        assert_eq!(bytemuck::cast_slice::<Vertex,u8>(&positional.vertices),bytemuck::cast_slice::<Vertex,u8>(&foliage.vertices));
        assert_eq!(bytemuck::cast_slice::<Vertex,u8>(&vertices),bytemuck::cast_slice::<Vertex,u8>(&foliage.vertices));
        assert_eq!(positional.tris,foliage.tris);
        assert_eq!(positional.cluster_verts,foliage.cluster_verts);
        assert_eq!(positional.sphere,foliage.sphere);
        assert_eq!(positional.levels,foliage.levels);
        assert_eq!(positional.clusters.len(),foliage.clusters.len());
        let mut raised=0;
        for (before,after) in positional.clusters.iter().zip(&foliage.clusters) {
            assert!(after.own_error<=after.parent_error,"nonmonotonic DAG transition");
            assert!(after.own_error>=before.own_error&&after.parent_error>=before.parent_error);
            raised+=usize::from(after.own_error>before.own_error||after.parent_error>before.parent_error);
            let mut neutral=*after;neutral.own_error=before.own_error;neutral.parent_error=before.parent_error;
            assert_eq!(bytemuck::bytes_of(before),bytemuck::bytes_of(&neutral));
        }
        assert!(raised>0,"fixture did not exercise projected-area credit");
        let (full,source)=directional_cut_area(&foliage,|c|c.own_error==0.0);
        assert_eq!(full,indices.len()/3);
        assert!(source.min_element()>0.1,"fixture must have coverage in all three axes");
        assert!(foliage.levels>1&&foliage.root_triangles()<full/2,"distant source simplification was disabled");
        // build_with_policy also audits original-leaf DAG credit at every
        // transition and mixed perspective cut via assert_complete_dag_cuts.
        let mut largest_improvement=0.0f64;
        for radius_px in [8.0f32,16.0,32.0,64.0,128.0,256.0] {
            let scale=radius_px/foliage.sphere[3];
            let selected=|c:&GpuCluster|(c.parent_error>1e30||c.parent_error*scale>1.0)&&c.own_error*scale<=1.0;
            let (old_count,old_area)=directional_cut_area(&positional,selected);
            let (count,area)=directional_cut_area(&foliage,selected);
            let before=old_area.element_sum()/source.element_sum();
            let after=area.element_sum()/source.element_sum();
            largest_improvement=largest_improvement.max(after-before);
            eprintln!("thin closed leaves: radius={radius_px}px tris={old_count}->{count}/{full} area={before:.4}->{after:.4} axes={:?}",area/source);
            assert!(count>0&&count<=full,"invalid selected budget at {radius_px}px");
            if radius_px>=128.0 {
                assert!(after>=0.60,"visible crown lost most of its geometric leaf area at {radius_px}px: {after}");
            }
        }
        assert!(largest_improvement>0.10,"closed-leaf fixture must expose a meaningful positional-only coverage regression");
    }

    #[test]
    fn textured_wood_keeps_authored_surface_orientation_until_subpixel() {
        for (name, bytes) in [
            ("fir_a", include_bytes!("../../../assets/fir_a_near.kmesh").as_slice()),
            ("fir_b", include_bytes!("../../../assets/fir_b_near.kmesh").as_slice()),
            ("fir_c", include_bytes!("../../../assets/fir_c_near.kmesh").as_slice()),
        ] {
            let source = read_kmesh(bytes).unwrap();
            for (material, vertices, indices) in source.iter().filter(|(n,_,_)|n.contains("trunk")||n.contains("bark")) {
                let key=|t:&[u32]| {let k=if t[0]<=t[1]&&t[0]<=t[2]{0}else if t[1]<=t[2]{1}else{2};[t[k],t[(k+1)%3],t[(k+2)%3]]};
                let authored:std::collections::HashSet<_>=indices.chunks_exact(3).map(key).collect();
                let mesh=ClusterMesh::build_with_policy(&[Part{vertices,indices,material:0}],|_|SimplificationPolicy::TexturedGeometry);
                assert_eq!(bytemuck::cast_slice::<Vertex,u8>(vertices),bytemuck::cast_slice::<Vertex,u8>(&mesh.vertices),"source normals/UVs must stay exact");
                assert!(mesh.root_triangles()<indices.len()/3,"{name}/{material}: distant LOD was disabled");
                for cut in [0.001f32,0.01,0.1,1.0,1e29] {
                    let mut count=0;
                    for c in mesh.clusters.iter().filter(|c|c.own_error<=cut&&c.parent_error>cut) {
                        for packed in &mesh.tris[c.first_tri as usize..(c.first_tri+c.tri_count)as usize] {
                            let tri:[u32;3]=std::array::from_fn(|k|mesh.cluster_verts[c.first_vert as usize+((packed>>(8*k))&255)as usize]);
                            count+=1;
                            if authored.contains(&key(&tri)){continue;}
                            let p=tri.map(|i|glam::Vec3::from(vertices[i as usize].pos));
                            let face=(p[1]-p[0]).cross(p[2]-p[0]);
                            if face.length_squared()<=1e-20{continue;}
                            let n:glam::Vec3=tri.iter().map(|i|glam::Vec3::from(vertices[*i as usize].normal)).sum();
                            // These retained source assets have clockwise winding.
                            if n.length_squared()<=1e-12||face.normalize().dot(n.normalize())>0.1 {
                                let edge=(p[1]-p[0]).length().max((p[2]-p[0]).length()).max((p[2]-p[1]).length());
                                assert!(edge<=cut*2.0+1e-6,"{name}/{material}: visible reversed face of {edge}m at error {cut}m");
                            }
                        }
                    }
                    assert!(count>0&&count<=indices.len()/3,"invalid wood budget");
                }
            }
        }
    }

    /// Audit exactly the GPU's parent/own selection at every transition and
    /// halfway between transitions. Track each original leaf's DAG credit;
    /// every cut must represent it exactly once, with no gap or double draw.
    pub(super) fn assert_complete_dag_cuts(groups: &[[f32;5]], pending: &[PendingCluster]) {
        let leaves:Vec<_>=pending.iter().enumerate().filter(|(_,(_,refined,_,_))|*refined<0).map(|(i,_)|i).collect();
        let mut credits=vec![vec![0.0f64;leaves.len()];pending.len()];
        let mut refined_count=vec![0usize;groups.len()];
        for (_,refined,_,_) in pending {if *refined>=0 {refined_count[*refined as usize]+=1;}}
        for (i,(_,refined,_,_)) in pending.iter().enumerate() {
            if *refined<0 {credits[i][leaves.iter().position(|leaf|*leaf==i).unwrap()]=1.0;} else {
                let child=*refined as usize;
                for (j,_) in pending.iter().enumerate().filter(|(_,p)|p.0==child) {
                    assert!(j<i,"DAG is not in child-first order");
                    for k in 0..leaves.len() {let credit=credits[j][k]/refined_count[child] as f64;credits[i][k]+=credit;}
                }
            }
        }
        let mut cuts:Vec<_>=groups.iter().map(|g|g[4]).filter(|e|*e>0.0&&*e<1e30).collect();
        cuts.sort_by(f32::total_cmp);cuts.dedup();
        let mut probes=vec![0.0,1e29];
        for (i,&cut) in cuts.iter().enumerate(){probes.push(cut);probes.push(if i==0{cut*0.5}else{(cuts[i-1]+cut)*0.5});}
        for threshold in probes {
            let mut totals=vec![0.0f64;leaves.len()];
            for (i,(gid,refined,_,_)) in pending.iter().enumerate() {
                let own=if *refined<0{0.0}else{groups[*refined as usize][4]};
                let parent=groups[*gid][4];
                assert!(own<=parent,"nonmonotonic crown DAG errors");
                // Equivalent to projected(error)<=1 at a uniform px/metre
                // scale of1/threshold; threshold0 selects the source leaves.
                if (parent>threshold||parent>1e30)&&own<=threshold {
                    for k in 0..leaves.len(){totals[k]+=credits[i][k];}
                }
            }
            for total in totals {assert!((total-1.0).abs()<1e-8,"DAG gap/overlap at {threshold}: original leaf credit {total}");}
        }
        let root=groups.last().unwrap();
        let centre=glam::Vec3::from_slice(&root[..3]);
        for direction in [glam::Vec3::X,glam::Vec3::Z,glam::Vec3::new(1.0,0.3,1.0).normalize()] {
            for distance in [100.0f32,300.0,800.0] {
                let camera=centre+direction*distance;
                for height in [1080.0f32,2160.0] {
                    let px_scale=height/(2.0*32f32.to_radians().tan());
                    let projected=|g:[f32;5]| if g[4]>1e30 {1e30} else {g[4]*px_scale/(glam::Vec3::from_slice(&g[..3]).distance(camera)-g[3]).max(0.1)};
                    let mut totals=vec![0.0f64;leaves.len()];
                    for (i,(gid,refined,_,_)) in pending.iter().enumerate() {
                        let own=if *refined<0{0.0}else{projected(groups[*refined as usize])};
                        if projected(groups[*gid])>1.0&&own<=1.0 {for k in 0..leaves.len(){totals[k]+=credits[i][k];}}
                    }
                    for total in totals {assert!((total-1.0).abs()<1e-8,"perspective DAG gap/overlap at{distance}m/{height}p: leaf credit {total}");}
                }
            }
        }
    }

    /// Sum projected triangle areas in three orthogonal directions. This is a
    /// geometry diagnostic, not a rendered alpha-coverage or overdraw measure.
    fn projected_area(mesh: &ClusterMesh, material: u32, roots: bool) -> f32 {
        let mut sum = 0.0;
        for cluster in mesh.clusters.iter().filter(|c| c.material == material && if roots { c.parent_error > 1e30 } else { c.own_error == 0.0 }) {
            for &packed in &mesh.tris[cluster.first_tri as usize..(cluster.first_tri + cluster.tri_count) as usize] {
                let p: [glam::Vec3; 3] = std::array::from_fn(|k| {
                    let i = mesh.cluster_verts[cluster.first_vert as usize + ((packed >> (k*8)) & 255) as usize];
                    glam::Vec3::from_array(mesh.vertices[i as usize].pos)
                });
                sum += (p[1]-p[0]).cross(p[2]-p[0]).abs().element_sum()*0.5;
            }
        }
        sum
    }

    fn selected_metrics(mesh: &ClusterMesh, material: u32, select: impl Fn(&GpuCluster)->bool) -> (usize,f32) {
        let mut selected=ClusterMesh::default();
        selected.vertices=mesh.vertices.clone();selected.cluster_verts=mesh.cluster_verts.clone();selected.tris=mesh.tris.clone();
        selected.clusters=mesh.clusters.iter().filter(|c|c.material==material&&select(c)).copied().map(|mut c|{c.parent_error=f32::MAX;c}).collect();
        (selected.clusters.iter().map(|c|c.tri_count as usize).sum(),projected_area(&selected,material,true))
    }

    #[test]
    fn authored_crowns_keep_source_and_opaque_policy_with_lod_metrics() {
        for (name, bytes) in [
            ("fir_a", include_bytes!("../../../assets/fir_a_near.kmesh").as_slice()),
            ("fir_b", include_bytes!("../../../assets/fir_b_near.kmesh").as_slice()),
            ("fir_c", include_bytes!("../../../assets/fir_c_near.kmesh").as_slice()),
        ] {
            let source = read_kmesh(bytes).unwrap();
            let parts: Vec<Part> = source.iter().enumerate().map(|(i,(_,vertices,indices))| Part { vertices, indices, material:i as u32 }).collect();
            let crown = source.iter().position(|(name,_,_)|name.is_empty()).unwrap() as u32;
            let before = ClusterMesh::build(&parts);
            let after = ClusterMesh::build_with_policy(&parts, |p| if p.material == crown { SimplificationPolicy::AlphaCards } else { SimplificationPolicy::Geometry });
            assert_eq!(bytemuck::cast_slice::<Vertex,u8>(&before.vertices), bytemuck::cast_slice::<Vertex,u8>(&after.vertices), "{name}: near source vertices changed");
            // Crown is the final material in these fixtures. Its policy cannot
            // change any earlier wood clusters, triangles or local indices.
            let old_wood: Vec<_> = before.clusters.iter().filter(|c|c.material != crown).copied().collect();
            let new_wood: Vec<_> = after.clusters.iter().filter(|c|c.material != crown).copied().collect();
            assert_eq!(bytemuck::cast_slice::<GpuCluster,u8>(&old_wood), bytemuck::cast_slice::<GpuCluster,u8>(&new_wood), "{name}: wood policy changed");
            let root_tris = |mesh: &ClusterMesh| mesh.clusters.iter().filter(|c|c.material == crown && c.parent_error > 1e30).map(|c|c.tri_count as usize).sum::<usize>();
            let full = source[crown as usize].2.len()/3;
            let old_area = projected_area(&before,crown,false).max(1e-6);
            let before_ratio = projected_area(&before,crown,true)/old_area;
            let after_ratio = projected_area(&after,crown,true)/old_area;
            eprintln!("{name}: crown source={full} root_before={} root_after={} whole_tree_before={} whole_tree_after={} projected_area_before={before_ratio:.4} projected_area_after={after_ratio:.4}", root_tris(&before),root_tris(&after),before.root_triangles(),after.root_triangles());
            assert!(root_tris(&after)>0 && root_tris(&after)<=full, "{name}: invalid crown root budget");
            assert!(after.levels>1, "{name}: LOD unexpectedly disabled");
            let crown_bounds=bounding_sphere(parts[crown as usize].vertices);
            let crown_radius=crown_bounds[3];
            for radius_px in [16.0f32,32.0,64.0,128.0] {
                let scale=radius_px/crown_radius;
                let select=|c:&GpuCluster|(c.parent_error>1e30||c.parent_error*scale>1.0)&&c.own_error*scale<=1.0;
                let (old_tris,old_selected_area)=selected_metrics(&before,crown,select);
                let (tris,selected_area)=selected_metrics(&after,crown,select);
                let old_retained=old_selected_area/old_area;
                let retained=selected_area/old_area;
                let minimum=(0.80-8.0/radius_px).max(0.0);
                eprintln!("{name}: ortho crown_radius_px={radius_px} before_tris={old_tris} after_tris={tris} before_area={old_retained:.4} after_area={retained:.4} minimum={minimum:.4}");
                assert!(tris>0&&tris<=full,"{name}: invalid selected crown budget at{radius_px}px");
                assert!(retained+0.025>=minimum,"{name}: excessive geometric coverage loss at{radius_px}px: {retained} < {minimum}");
            }
            let centre=glam::Vec3::from_slice(&crown_bounds[..3]);
            for distance in [100.0f32,300.0,800.0] {
                let camera=centre+glam::Vec3::Z*distance;
                for height in [1080.0f32,2160.0] {
                    let px_scale=height/(2.0*32f32.to_radians().tan());
                    let projected=|error:f32,sphere:[f32;4]|if error>1e30{1e30}else{error*px_scale/(glam::Vec3::from_slice(&sphere[..3]).distance(camera)-sphere[3]).max(0.1)};
                    let select=|c:&GpuCluster|projected(c.parent_error,c.parent)>1.0&&projected(c.own_error,c.own)<=1.0;
                    let (old_tris,old_selected_area)=selected_metrics(&before,crown,select);
                    let (tris,selected_area)=selected_metrics(&after,crown,select);
                    let old_retained=old_selected_area/old_area;let retained=selected_area/old_area;
                    eprintln!("{name}: perspective identity_instance distance_m={distance} output_height={height} before_tris={old_tris} after_tris={tris} before_area={old_retained:.4} after_area={retained:.4}");
                }
            }
        }
    }
}
