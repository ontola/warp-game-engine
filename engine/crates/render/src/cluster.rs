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
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// One cluster as the GPU sees it (80 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
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
    pub _pad: u32,
}

/// A mesh part with one material.
pub struct Part<'a> {
    pub vertices: &'a [Vertex],
    pub indices: &'a [u32],
    pub material: u32,
}

/// Material-aware simplification is opt-in: opaque geometry keeps the existing
/// spatial policy. Alpha cards must retain their texture mapping and folds;
/// an opaque geometric error alone cannot describe their visible silhouette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimplificationPolicy {
    Geometry,
    AlphaCards,
    /// Preserve authored normals, UV seams and folds without alpha-area credit.
    TexturedGeometry,
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
            SimplificationPolicy::TexturedGeometry, SimplificationPolicy::AlphaCards] {
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

/// Cutout-card coverage is not represented by positional simplification error:
/// a planar card can shrink without moving any surviving point off its plane.
/// Carry original projected-area credit through the DAG and include cumulative
/// area loss in its existing world-space error, with no new GPU data or work.
/// This is summed geometric area, not alpha-union coverage (cards can overlap).
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

/// Cluster data for a mesh, ready to append to the scene buffers. Indices
/// in `clusters` are local to these arrays; the scene rebases them.
#[derive(Default)]
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
            let selected_policy=policy(part);
            let alpha_cards=selected_policy == SimplificationPolicy::AlphaCards;
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
            if selected_policy == SimplificationPolicy::TexturedGeometry {
                protect_surface_orientation(part, &mut groups, &pending);
                #[cfg(test)]
                policy_tests::assert_complete_dag_cuts(&groups, &pending);
            }
            if alpha_cards {
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
                    _pad: 0,
                });
            }
        }
        out.sphere = bounding_sphere(&out.vertices);
        out
    }

    /// Triangles at full detail (the DAG's leaves).
    pub fn leaf_triangles(&self) -> usize {
        self.clusters.iter().filter(|c| c.own_error == 0.0).map(|c| c.tri_count as usize).sum()
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

/// One explicit coarse level for [`ClusterMesh::replace_beyond`]: unindexed
/// geometry is fine; triangles are split into clusters as they come.
pub struct ProxyLevel<'a> {
    /// Object-space error at which this level becomes acceptable.
    pub error: f32,
    pub parts: Vec<Part<'a>>,
}

impl ClusterMesh {
    /// Nanite-style switch: clusters of `replace` materials are kept only
    /// while their error is below `levels[0].error`; beyond it the explicit
    /// `levels` take over, each until the next one's error. All switching
    /// bounds share one sphere enclosing both geometry and the original
    /// error spheres. This makes the switch dominate every finer source
    /// decision, so exactly one of {source cut, level k} passes the GPU's
    /// own <= t < parent test for any view. Source clusters coarser than the
    /// switch are dropped. Other
    /// materials (wood) keep their own DAG untouched.
    pub fn replace_beyond(&mut self, replace: impl Fn(u32) -> bool, levels: &[ProxyLevel<'_>]) -> Result<(), String> {
        let Some(first) = levels.first() else { return Ok(()) };
        let switch = first.error;
        if !switch.is_finite() || switch <= 0.0
            || levels.windows(2).any(|w| !w[1].error.is_finite() || w[1].error <= w[0].error) {
            return Err("proxy level errors must be finite, positive and increasing".into());
        }
        // One sphere over the replaced source and every proxy vertex.
        let mut points: Vec<Vertex> = Vec::new();
        for c in self.clusters.iter().filter(|c| replace(c.material)) {
            for k in 0..c.tri_count {
                let packed = self.tris[(c.first_tri + k) as usize];
                for corner in 0..3 {
                    let local = (packed >> (8 * corner)) & 255;
                    points.push(self.vertices[self.cluster_verts[(c.first_vert + local) as usize] as usize]);
                }
            }
        }
        for level in levels {
            if !level.parts.iter().any(|part| !part.indices.is_empty()) {
                return Err("proxy level must contain replacement triangles".into());
            }
            for part in &level.parts {
                if part.indices.len() % 3 != 0 || part.indices.iter().any(|&i| i as usize >= part.vertices.len()) {
                    return Err("proxy level has invalid indices".into());
                }
                points.extend(part.indices.iter().map(|&i| part.vertices[i as usize]));
            }
        }
        if points.iter().any(|v| v.pos.iter().any(|x| !x.is_finite())) {
            return Err("proxy level has non-finite positions".into());
        }
        let mut sphere = bounding_sphere(&points);
        // A sphere containing the vertices need not contain a local error
        // sphere: that sphere can extend beyond its own geometry. If its
        // nearest point is closer to the camera than our common sphere's,
        // an untouched finer source cut can still pass after the proxy has
        // become acceptable. Enclosing all original error spheres ensures
        // projected(switch, common) >= projected(source error, local) for
        // every source error below the switch, including off-centre views.
        let centre = glam::DVec3::new(sphere[0] as f64, sphere[1] as f64, sphere[2] as f64);
        let mut radius = sphere[3] as f64;
        for c in self.clusters.iter().filter(|c| replace(c.material)) {
            for bound in [c.own, c.parent] {
                if bound.iter().any(|x| !x.is_finite()) || bound[3] < 0.0 {
                    return Err("source cluster has an invalid error sphere".into());
                }
                let local = glam::DVec3::new(bound[0] as f64, bound[1] as f64, bound[2] as f64);
                radius = radius.max(centre.distance(local) + bound[3] as f64);
            }
        }
        if !radius.is_finite() || radius > f32::MAX as f64 {
            return Err("proxy switching sphere exceeds finite bounds".into());
        }
        sphere[3] = radius as f32;
        if (sphere[3] as f64) < radius {
            sphere[3] = f32::from_bits(sphere[3].to_bits() + 1);
        }
        self.clusters.retain(|c| !replace(c.material) || c.own_error < switch);
        for c in self.clusters.iter_mut().filter(|c| replace(c.material) && c.parent_error >= switch) {
            c.parent_error = switch;
            c.parent = sphere;
        }
        for (k, level) in levels.iter().enumerate() {
            let parent_error = levels.get(k + 1).map_or(f32::MAX, |l| l.error);
            for part in &level.parts {
                // Split in index order: at most MAX_TRIS triangles and 255
                // distinct vertices per cluster (u8 local indices).
                let mut start = 0;
                while start < part.indices.len() {
                    let mut local = std::collections::HashMap::<u32, u32>::new();
                    let mut verts = Vec::new();
                    let mut packed = Vec::new();
                    let mut end = start;
                    while end < part.indices.len() && packed.len() < MAX_TRIS {
                        let tri = &part.indices[end..end + 3];
                        let mut fresh = 0;
                        for (n, i) in tri.iter().enumerate() {
                            if !local.contains_key(i) && !tri[..n].contains(i) { fresh += 1; }
                        }
                        if verts.len() + fresh > 255 { break; }
                        let mut p = 0u32;
                        for (corner, &i) in tri.iter().enumerate() {
                            let l = *local.entry(i).or_insert_with(|| { verts.push(i); verts.len() as u32 - 1 });
                            p |= l << (8 * corner);
                        }
                        packed.push(p);
                        end += 3;
                    }
                    let base = self.vertices.len() as u32;
                    let cluster_vertices: Vec<Vertex> = verts.iter().map(|&i| part.vertices[i as usize]).collect();
                    let cull = bounding_sphere(&cluster_vertices);
                    self.vertices.extend_from_slice(&cluster_vertices);
                    let first_vert = self.cluster_verts.len() as u32;
                    self.cluster_verts.extend((0..cluster_vertices.len() as u32).map(|i| base + i));
                    let first_tri = self.tris.len() as u32;
                    self.tris.extend_from_slice(&packed);
                    self.clusters.push(GpuCluster {
                        cull, parent: sphere, own: sphere, parent_error, own_error: level.error,
                        first_tri, tri_count: packed.len() as u32, first_vert,
                        material: part.material, depth: 0, _pad: 0,
                    });
                    start = end;
                }
            }
        }
        self.levels += levels.len() as u32;
        self.sphere = bounding_sphere(&self.vertices);
        Ok(())
    }
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
    /// Triangles drawn when the whole mesh is far away (the DAG's roots).
    pub fn root_triangles(&self) -> usize {
        self.clusters.iter().filter(|c| c.parent_error > 1e30).map(|c| c.tri_count as usize).sum()
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

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

#[cfg(test)]
mod proxy_tests {
    use super::*;

    fn grid(n: usize, z: f32) -> (Vec<Vertex>, Vec<u32>) {
        let mut v = Vec::new();
        let mut i = Vec::new();
        for y in 0..=n { for x in 0..=n {
            v.push(Vertex { pos: [x as f32 / n as f32 * 4.0, y as f32 / n as f32 * 4.0, z], normal: [0.0, 0.0, 1.0], uv: [0.0; 2] });
        } }
        for y in 0..n { for x in 0..n {
            let a = (y * (n + 1) + x) as u32;
            let b = a + 1; let c = a + n as u32 + 1; let d = c + 1;
            i.extend([a, b, c, b, d, c]);
        } }
        (v, i)
    }

    /// The GPU rule from cull.wgsl: own projects <= t < parent.
    fn selected(mesh: &ClusterMesh, eye: glam::Vec3, t: f32) -> Vec<&GpuCluster> {
        let proj = |e: f32, s: [f32; 4]| {
            if e > 1e30 { return 1e30; }
            let d = (glam::Vec3::new(s[0], s[1], s[2]).distance(eye) - s[3]).max(1e-3);
            e / d
        };
        mesh.clusters.iter().filter(|c| proj(c.own_error, c.own) <= t && proj(c.parent_error, c.parent) > t).collect()
    }

    #[test]
    fn proxy_switch_encloses_error_spheres_for_oblique_nonplanar_cuts() {
        // Two independent branches, each with a leaf and a simplified cut.
        // Their error spheres extend past the common vertex sphere. The
        // triangles are deliberately not coplanar and the camera can graze
        // either branch rather than looking straight at the crown centre.
        let vertices: Vec<Vertex> = [
            [-1.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 0.9, 0.1],
            [-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, -0.9, -0.1],
        ].into_iter().map(|pos| Vertex { pos, normal: [0.0, 0.0, 1.0], uv: [0.0; 2] }).collect();
        let indices = [0, 1, 2, 3, 4, 5];
        let mut mesh = ClusterMesh::default();
        mesh.vertices = vertices.clone();
        mesh.cluster_verts = indices.to_vec();
        mesh.tris = vec![0 | (1 << 8) | (2 << 16); 2];
        mesh.sphere = bounding_sphere(&vertices);
        let original_bounds = [[0.0, 1.0, 0.0, 1.0], [0.0, -1.0, 0.0, 1.0]];
        for (branch, own) in original_bounds.into_iter().enumerate() {
            for leaf in [true, false] {
                mesh.clusters.push(GpuCluster {
                    cull: own, own,
                    parent: if leaf { own } else { [0.0, 0.0, 0.0, 2.0] },
                    parent_error: if leaf { 0.07 } else { f32::MAX },
                    own_error: if leaf { 0.0 } else { 0.07 },
                    first_tri: branch as u32, tri_count: 1, first_vert: branch as u32 * 3,
                    material: 0, depth: 0, _pad: 0,
                });
            }
        }
        let levels = [
            ProxyLevel { error: 0.08, parts: vec![Part { vertices: &vertices, indices: &indices, material: 2 }] },
            ProxyLevel { error: 0.2, parts: vec![Part { vertices: &vertices, indices: &indices, material: 3 }] },
        ];
        mesh.replace_beyond(|m| m == 0, &levels).unwrap();
        let common = mesh.clusters.iter().find(|c| c.material == 2).unwrap().own;
        for bound in original_bounds {
            let distance = glam::DVec3::new(common[0] as f64, common[1] as f64, common[2] as f64)
                .distance(glam::DVec3::new(bound[0] as f64, bound[1] as f64, bound[2] as f64));
            assert!(distance + bound[3] as f64 <= common[3] as f64);
        }

        // Reproduce the previous vertex-only bound without changing any
        // triangles or error values: it selects leaf source AND a proxy.
        let mut old = ClusterMesh::default();
        old.clusters = mesh.clusters.clone();
        let vertex_sphere = bounding_sphere(&vertices);
        for c in &mut old.clusters {
            if c.material == 0 && c.parent_error == 0.08 { c.parent = vertex_sphere; }
            if c.material != 0 { c.own = vertex_sphere; c.parent = vertex_sphere; }
        }
        let broken = selected(&old, glam::Vec3::new(0.0, 2.5, 0.1), 0.1);
        assert!(broken.iter().any(|c| c.material == 0) && broken.iter().any(|c| c.material == 2));

        for direction in [
            glam::Vec3::Y, -glam::Vec3::Y, glam::Vec3::Z,
            glam::Vec3::new(0.1, 1.0, 0.04).normalize(),
            glam::Vec3::new(1.0, -0.6, 0.2).normalize(),
        ] {
            for distance in [1.45, 2.5, 4.0, 20.0] {
                let eye = direction * distance;
                let mut thresholds = vec![0.0, 1e-5, 0.1, 1000.0];
                for c in &mesh.clusters {
                    for (error, sphere) in [(c.own_error, c.own), (c.parent_error, c.parent)] {
                        if error <= 0.0 || error > 1e30 { continue; }
                        let projected = error / (glam::Vec3::from_slice(&sphere[..3]).distance(eye) - sphere[3]).max(1e-3);
                        thresholds.extend([projected * 0.9999, projected, projected * 1.0001]);
                    }
                }
                for threshold in thresholds {
                    let mut credit = [0usize; 2];
                    for c in selected(&mesh, eye, threshold) {
                        if c.material == 0 { credit[c.first_tri as usize] += 1; }
                        else { credit[0] += 1; credit[1] += 1; }
                    }
                    assert_eq!(credit, [1, 1], "eye={eye:?}, threshold={threshold}");
                }
            }
        }
    }

    #[test]
    fn empty_proxy_level_is_rejected_before_source_changes() {
        let (vertices, indices) = grid(4, 0.0);
        let mut mesh = ClusterMesh::build(&[Part { vertices: &vertices, indices: &indices, material: 0 }]);
        let before = bytemuck::cast_slice::<GpuCluster, u8>(&mesh.clusters).to_vec();
        let levels = [ProxyLevel { error: 0.08, parts: Vec::new() }];
        assert!(mesh.replace_beyond(|m| m == 0, &levels).unwrap_err().contains("replacement triangles"));
        assert_eq!(before, bytemuck::cast_slice::<GpuCluster, u8>(&mesh.clusters));
    }

    #[test]
    fn proxy_levels_replace_exactly_one_representation_at_every_distance() {
        let (sv, si) = grid(40, 0.0);
        let (wv, wi) = grid(8, -1.0);
        let mut mesh = ClusterMesh::build(&[Part { vertices: &sv, indices: &si, material: 0 }, Part { vertices: &wv, indices: &wi, material: 1 }]);
        let source_area = 16.0;
        let (p1, i1) = grid(4, 0.0);
        let (p2, i2) = grid(2, 0.0);
        let levels = [
            ProxyLevel { error: 0.05, parts: vec![Part { vertices: &p1, indices: &i1, material: 2 }] },
            ProxyLevel { error: 0.2, parts: vec![Part { vertices: &p2, indices: &i2, material: 3 }] },
        ];
        let wood_before: Vec<GpuCluster> = mesh.clusters.iter().filter(|c| c.material == 1).copied().collect();
        mesh.replace_beyond(|m| m == 0, &levels).unwrap();
        assert!(mesh.clusters.iter().filter(|c| c.material == 0).all(|c| c.own_error < 0.05));
        assert_eq!(wood_before.len(), mesh.clusters.iter().filter(|c| c.material == 1).count(), "other materials untouched");
        let area = |c: &GpuCluster| -> f32 {
            (0..c.tri_count).map(|k| {
                let packed = mesh.tris[(c.first_tri + k) as usize];
                let p: [glam::Vec3; 3] = std::array::from_fn(|j| glam::Vec3::from(mesh.vertices[mesh.cluster_verts[(c.first_vert + ((packed >> (8 * j)) & 255)) as usize] as usize].pos));
                (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5
            }).sum()
        };
        let mut seen = [false; 3];
        for step in 0..400 {
            let distance = 0.5 * 1.03f32.powi(step);
            let eye = glam::Vec3::new(2.0, 2.0, distance);
            let sel = selected(&mesh, eye, 1.0 / 900.0);
            let by = |m: u32| sel.iter().filter(|c| c.material == m).map(|c| area(c)).sum::<f32>();
            let (src, a, b) = (by(0), by(2), by(3));
            let reps = [src > 0.0, a > 0.0, b > 0.0];
            assert_eq!(reps.iter().filter(|x| **x).count(), 1, "distance {distance}: {reps:?}");
            for (k, r) in reps.iter().enumerate() { seen[k] |= *r; }
            let total = src + a + b;
            assert!((total - source_area).abs() < 1e-3, "distance {distance}: area {total}");
        }
        assert_eq!(seen, [true; 3], "every representation is used at some distance");
    }
}
