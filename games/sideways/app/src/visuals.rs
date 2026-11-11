//! Sideways' look as a Warp scene: terrain chunks, the road ribbon, the
//! forest (retained native meshes in the game's assets/meshes),
//! barriers and cars, with the Poly Haven textures.

use glam::{Affine3A, Quat, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{ALPHA_TEST, GeometryCategory, Material, ROAD, Scene, TERRAIN, TRIPLANAR, TWO_SIDED};
use sideways::builder::{self, Chunk, N};
use sideways::stage::{GRID, Stage};
use std::path::PathBuf;

pub struct Built {
    pub scene: Scene,
    /// What each mesh id is, for picking.
    pub mesh_names: Vec<String>,
    pub log: Vec<String>,
    pub cars: Vec<DynamicCar>,
    /// Every car surface in `scene.car_materials` order, to re-apply edited
    /// car material data (`Renderer::set_car_materials`).
    pub car_slots: Vec<crate::car_look::Slot>,
    /// Explicit experimental patch provenance; None in normal play.
    pub procedural_grass: Option<serde_json::Value>,
    /// Resolved values and source counts recorded during construction, not
    /// a later environment-variable snapshot of what might have been built.
    pub tree_source: serde_json::Value,
    live_ground_cover: Option<crate::live_ground_cover::LiveGroundCover>,
    /// The mixed forest's full trees that follow the cameras.
    pub tree_pool: Option<render::recipe_forest::Pool>,
}

pub struct DynamicCar {
    pub model: String,
    pub part_names: Vec<&'static str>,
    pub first: usize,
    meshes: crate::car_model::CarMeshes,
    fallback: bool,
}

impl Built {
    pub fn update_ground_cover(&mut self, cameras: &[Vec3]) -> Option<std::ops::Range<usize>> {
        self.live_ground_cover.as_mut()?.update(&mut self.scene, cameras)
    }

    /// Move the mixed forest's full trees to the trees nearest the cameras.
    /// Call after Renderer::new and before every frame.
    pub fn update_trees(&mut self, renderer: &render::Renderer, gpu: &render::Gpu, cameras: &[Vec3])
        -> Result<Option<render::recipe_forest::PoolStats>, String> {
        self.tree_pool.as_mut().map(|pool| pool.update(renderer, gpu, cameras)).transpose()
    }

    pub fn ground_cover_stats(&self) -> Option<crate::live_ground_cover::Stats> {
        self.live_ground_cover.as_ref().map(|stream| stream.stats())
    }

    pub fn update_cars(&mut self, poses: &[CarPose]) {
        for (car, pose) in self.cars.iter().zip(poses) {
            let xf = Affine3A::from_rotation_translation(pose.rot, pose.pos);
            if car.fallback {
                self.scene.update_instance(car.first, car.meshes.body, xf);
            } else {
                crate::car_model::update(&mut self.scene, car.first, &car.meshes, xf, pose.ride, &pose.wheels);
            }
        }
    }
}

fn root() -> PathBuf {
    sideways::game_assets::root()
}

fn v(p: Vec3, n: Vec3, uv: [f32; 2]) -> Vertex {
    Vertex { pos: p.to_array(), normal: n.to_array(), uv }
}

#[cfg(test)]
fn chunk_mesh(c: &Chunk, material: u32, sample_halo: impl Fn(f64,f64)->f32) -> ClusterMesh {
    let mut geometry=vec![chunk_geometry(c,sample_halo)];
    terrain_surface_normals(&mut geometry);
    let (verts, idx) = geometry.pop().unwrap();
    ClusterMesh::build_with_policy(&[Part { vertices: &verts, indices: &idx, material }],
        |_|render::cluster::SimplificationPolicy::Terrain)
}

// Surface relief is bounded relative to the unchanged 2 m collision mesh.
// The entire road/shoulder and a 2 m verge are exact; only steep, off-road
// faces receive curved geometry. Flat chunks keep their original topology.
const CLIFF_VISUAL_OFFSET: f32 = 1.10;
const CLIFF_FACE_RELIEF: f32 = 0.24;
const CLIFF_ROAD_GUARD: f32 = 2.0;
const CLIFF_DETAIL_UP: f32 = 0.80;
// Sample the actual native face, not just its XZ projection. This is a target,
// not an unbounded tessellator: very steep faces retain explicit hard caps.
const CLIFF_EDGE_TARGET: f32 = 0.75;
const CLIFF_EDGE_MAX: usize = 12;
const CLIFF_TRIANGLE_BUDGET: usize = 140;

fn cliff_edge_segments(a: Vec3, b: Vec3) -> usize {
    ((a.distance(b) / CLIFF_EDGE_TARGET).ceil() as usize).clamp(4, CLIFF_EDGE_MAX)
}

fn cliff_radial_segments(radius: f32, boundary: usize) -> usize {
    let wanted = ((radius / 1.5).ceil() as usize).clamp(2, 3);
    // n triangles around the centre, then 2n for each annular strip.
    let allowed = ((CLIFF_TRIANGLE_BUDGET / boundary.max(1) + 1) / 2).max(2);
    wanted.min(allowed)
}

fn cubic_height(a: f32, b: f32, c: f32, d: f32, t: f32) -> f32 {
    b + 0.5 * t * (c - a + t * (2.0*a - 5.0*b + 4.0*c - d + t * (3.0*(b-c) + d-a)))
}

/// Derive detail normals from the actual bounded, weighted render surface.
/// All chunks participate before cluster construction, so a shared border
/// includes triangles on BOTH sides and receives one bit-identical normal.
/// Road/shoulder normals and every position/UV remain unchanged.
fn terrain_surface_normals(geometry:&mut [(Vec<Vertex>,Vec<u32>)])->(usize,usize) {
    #[derive(Default)]
    struct Normal {area:Vec3,direction:Vec3,up_mix:f32}
    let key=|v:&Vertex|(v.pos[0].to_bits(),v.pos[2].to_bits());
    let detailed=|v:&Vertex|v.uv[0]>CLIFF_ROAD_GUARD&&v.uv[0]<35.0&&v.uv[1]-1.0<CLIFF_DETAIL_UP;
    let mut shared=std::collections::HashMap::<(u32,u32),Normal>::new();
    for (vertices,_) in geometry.iter() {for v in vertices.iter().filter(|v|detailed(v)) {shared.entry(key(v)).or_default();}}
    if shared.is_empty(){return (0,0);}
    // Area weighting does not overemphasise the tiny fan triangles relative
    // to their neighbours. Accumulation order is the sorted chunk/triangle
    // order, independent of HashMap iteration or parallel mesh preparation.
    for (vertices,indices) in geometry.iter() {for t in indices.chunks_exact(3) {
        let p=[t[0],t[1],t[2]].map(|i|Vec3::from(vertices[i as usize].pos));
        let face=(p[1]-p[0]).cross(p[2]-p[0]);
        for &i in t {if let Some(n)=shared.get_mut(&key(&vertices[i as usize])) {n.area+=face;}}
    }}
    for n in shared.values_mut() {n.direction=n.area.normalize_or(Vec3::Y);}
    // A smooth normal at a sharp non-convex joint must remain outside every
    // incident face. The heightfield's +Y is a feasible common hemisphere.
    // Use only the smallest necessary blend, then normalize once; the bound
    // is shared by all copies of the vertex, including chunk boundaries.
    for (vertices,indices) in geometry.iter() {for t in indices.chunks_exact(3) {
        let p=[t[0],t[1],t[2]].map(|i|Vec3::from(vertices[i as usize].pos));
        let face=(p[1]-p[0]).cross(p[2]-p[0]).normalize_or(Vec3::Y);
        for &i in t {if let Some(n)=shared.get_mut(&key(&vertices[i as usize])) {
            let dot=n.direction.dot(face);let margin=face.y.max(0.0)*0.0001;
            if dot<margin {n.up_mix=n.up_mix.max(((margin-dot)/(face.y-dot).max(1e-8)).clamp(0.0,1.0));}
        }}
    }}
    for n in shared.values_mut() {n.direction=n.direction.lerp(Vec3::Y,n.up_mix).normalize_or(Vec3::Y);}
    for (vertices,_) in geometry.iter_mut() {for v in vertices {if let Some(n)=shared.get(&key(v)) {v.normal=n.direction.to_array();}}}
    // Conservative bucket storage bound, excluding allocator bookkeeping.
    // No per-triangle adjacency lists and no permanent runtime allocation.
    let bucket_bound=(shared.capacity()*8/7+16).next_power_of_two();
    (shared.len(),bucket_bound*(std::mem::size_of::<((u32,u32),Normal)>()+16))
}

fn chunk_geometry(c: &Chunk, sample_halo: impl Fn(f64,f64)->f32) -> (Vec<Vertex>, Vec<u32>) {
    chunk_geometry_tessellation::<false>(c, sample_halo)
}

// Adaptive sampling is an opt-in candidate until matched visual acceptance.
// Keeping the surface evaluator identical isolates topology cost and retains
// the exact protected road. The default also preserves legacy triangle order.
fn chunk_geometry_tessellation<const ADAPTIVE: bool>(c: &Chunk, sample_halo: impl Fn(f64,f64)->f32) -> (Vec<Vertex>, Vec<u32>) {
    // A shared three-cell world halo makes the rounded surface and its
    // finite-difference normals identical at neighbouring chunk borders.
    let halo = 3isize;
    let stride = N + 2 * halo as usize;
    let heights: Vec<f32> = (-halo..N as isize+halo).flat_map(|j| {
        let sample = &sample_halo;
        (-halo..N as isize+halo).map(move |i| if i>=0 && j>=0 && i<N as isize && j<N as isize {
            c.heights[j as usize*N+i as usize]
        } else {sample(c.x0+i as f64*GRID,c.z0+j as f64*GRID)})
    }).collect();
    let h = |i:isize,j:isize| heights[(j+halo) as usize*stride+(i+halo) as usize];
    let native_normal = |i:isize,j:isize| Vec3::new(
        -(h(i+1,j)-h(i-1,j))/(2.0*GRID as f32),1.0,
        -(h(i,j+1)-h(i,j-1))/(2.0*GRID as f32)).normalize();
    let smooth=|t:f32| {let t=t.clamp(0.0,1.0);t*t*(3.0-2.0*t)};
    let weight=|road:f32,n:Vec3| smooth((road-CLIFF_ROAD_GUARD)/1.4)*smooth((CLIFF_DETAIL_UP-n.y)/0.25)
        *(1.0-smooth((road-25.0)/10.0));
    let curved=|x:f32,z:f32,n:Vec3| {
        let gx=(x as f64-c.x0)/GRID;let gz=(z as f64-c.z0)/GRID;
        let i=gx.floor()as isize;let j=gz.floor()as isize;
        let u=(gx-i as f64)as f32;let w=(gz-j as f64)as f32;
        let row=|j| cubic_height(h(i-1,j),h(i,j),h(i+1,j),h(i+2,j),u);
        let rounded=cubic_height(row(j-1),row(j),row(j+1),row(j+2),w);
        // A vertical-only heightfield needs 1/n.y times more vertical offset
        // to express a fixed depth normal to a steep face. The old 14 cm Y
        // cap produced only 3 cm of rock volume on an almost vertical cut.
        let relief=crate::procedural_rocks::terrain_relief(Vec3::new(x,rounded,z),n)
            .clamp(-CLIFF_FACE_RELIEF,CLIFF_FACE_RELIEF)/n.y.max(0.22);
        (rounded,relief)
    };
    let surface = |x:f32,z:f32,road:f32,n:Vec3| {
        let gx=(x as f64-c.x0)/GRID;let gz=(z as f64-c.z0)/GRID;
        let i=gx.floor() as isize;let j=gz.floor() as isize;
        let u=gx.fract() as f32;let w=gz.fract() as f32;
        // Coordinates used here are inside the chunk except the small
        // derivative halo. fract() is negative there, so use floor residual.
        let u=if u<0.0 {u+1.0}else{u};let w=if w<0.0 {w+1.0}else{w};
        let native=if u+w<=1.0 {h(i,j)+(h(i+1,j)-h(i,j))*u+(h(i,j+1)-h(i,j))*w}
            else {h(i+1,j+1)+(h(i,j+1)-h(i+1,j+1))*(1.0-u)+(h(i+1,j)-h(i+1,j+1))*(1.0-w)};
        let weight=weight(road,n);
        if weight==0.0 {return native;}
        let (rounded,relief)=curved(x,z,n);
        let rounding=(rounded-native).clamp(-0.16,0.16);
        native+(rounding+relief).clamp(-CLIFF_VISUAL_OFFSET,CLIFF_VISUAL_OFFSET)*weight
    };
    let detailed_vertex = |p:Vec3,n:Vec3,road:f32| {
        if road<=CLIFF_ROAD_GUARD || road>=35.0 || n.y>=CLIFF_DETAIL_UP {return v(p,n,[road,1.0+n.y]);}
        let y=surface(p.x,p.z,road,n);
        // Final normals are reconstructed from the rendered triangles after
        // every chunk is available, not a derivative of a different field.
        v(Vec3::new(p.x,y,p.z),n,[road,1.0+n.y])
    };
    let mut verts = Vec::with_capacity(N * N);
    let mut native = Vec::with_capacity(N * N);
    for j in 0..N {
        for i in 0..N {
            let n = native_normal(i as isize,j as isize);
            let p = c.vertex(i, j);
            let p=Vec3::new(p.x,p.y,p.z);let road=c.road_d[j*N+i];
            native.push(v(p,n,[road,0.0]));
            verts.push(detailed_vertex(p,n,road));
        }
    }
    if !native.iter().any(|v|v.uv[0]>CLIFF_ROAD_GUARD && v.uv[0]<35.0 && v.normal[1]<CLIFF_DETAIL_UP) {
        // Most chunks are rolling forest, not a rock cut. Avoid the edge
        // cache and per-triangle allocation entirely for that common case.
        let mut indices=Vec::with_capacity((N-1)*(N-1)*6);
        for j in 0..N-1 {for i in 0..N-1 {
            let a=(j*N+i)as u32;let n=N as u32;
            indices.extend_from_slice(&[a,a+n,a+1,a+1,a+n,a+n+1]);
        }}
        return (verts,indices);
    }
    let mut edges=std::collections::HashMap::<(u32,u32),Vec<u32>>::new();
    let mut idx = Vec::new();
    for j in 0..N - 1 {
        for i in 0..N - 1 {
            let a = (j * N + i) as u32;
            let n = N as u32;
            for triangle in [[a,a+n,a+1],[a+1,a+n,a+n+1]] {
                let mut boundary=Vec::new();let mut refined=false;
                for e in 0..3 {
                    let (a,b)=(triangle[e],triangle[(e+1)%3]);
                    let key=(a.min(b),a.max(b));
                    let edge=edges.entry(key).or_insert_with(|| {
                        let va=native[key.0 as usize];let vb=native[key.1 as usize];
                        let split=va.uv[0].max(vb.uv[0])>CLIFF_ROAD_GUARD && va.uv[0].min(vb.uv[0])<35.0
                            && va.normal[1].min(vb.normal[1])<CLIFF_DETAIL_UP;
                        let mut edge=vec![key.0];
                        let segments=if ADAPTIVE {cliff_edge_segments(Vec3::from(va.pos),Vec3::from(vb.pos))}else{4};
                        if split {for k in 1..segments {
                            let t=k as f32/segments as f32;
                            let p=Vec3::from(va.pos).lerp(Vec3::from(vb.pos),t);
                            let normal=Vec3::from(va.normal).lerp(Vec3::from(vb.normal),t).normalize();
                            let road=va.uv[0]+(vb.uv[0]-va.uv[0])*t;
                            edge.push(verts.len()as u32);native.push(v(p,normal,[road,0.0]));
                            verts.push(detailed_vertex(p,normal,road));
                        }}
                        edge.push(key.1);edge
                    });
                    refined|=edge.len()>2;
                    if a==key.0 {boundary.extend(edge[..edge.len()-1].iter().copied());}
                    else {boundary.extend(edge[1..].iter().rev().copied());}
                }
                if !refined {idx.extend_from_slice(&triangle);continue;}
                let source=triangle.map(|k|native[k as usize]);
                let cp=source.iter().map(|v|Vec3::from(v.pos)).sum::<Vec3>()/3.0;
                let cn=source.iter().map(|v|Vec3::from(v.normal)).sum::<Vec3>().normalize();
                let cr=source.iter().map(|v|v.uv[0]).sum::<f32>()/3.0;
                let centre=verts.len()as u32;native.push(v(cp,cn,[cr,0.0]));verts.push(detailed_vertex(cp,cn,cr));
                if !ADAPTIVE {
                    // Preserve the previous arithmetic and index order too:
                    // otherwise meshopt could choose different shadow casters
                    // during an unrelated lighting A/B with this option off.
                    let mut ring=Vec::with_capacity(boundary.len());
                    for &b in &boundary {
                        let vb=native[b as usize];
                        let p=cp.lerp(Vec3::from(vb.pos),0.5);
                        let nn=cn.lerp(Vec3::from(vb.normal),0.5).normalize();
                        let rd=(cr+vb.uv[0])*0.5;
                        ring.push(verts.len()as u32);native.push(v(p,nn,[rd,0.0]));verts.push(detailed_vertex(p,nn,rd));
                    }
                    for k in 0..boundary.len() {
                        let next=(k+1)%boundary.len();
                        idx.extend_from_slice(&[centre,ring[k],ring[next],ring[k],boundary[k],boundary[next],ring[k],boundary[next],ring[next]]);
                    }
                    continue;
                }
                // A tall face also needs interior samples: adding only edge
                // points leaves the same metre-long radial spokes. Shared
                // boundary samples are independent of either face's rings.
                let radius=source.iter().map(|v|Vec3::from(v.pos).distance(cp)).fold(0.0,f32::max);
                let radial=cliff_radial_segments(radius,boundary.len());
                debug_assert!(boundary.len()*(2*radial-1)<=CLIFF_TRIANGLE_BUDGET);
                let mut previous=Vec::new();
                for level in 1..=radial {
                    let mut ring=Vec::with_capacity(boundary.len());
                    if level==radial {ring.extend_from_slice(&boundary);}
                    else {for &b in &boundary {
                        // Sample the unchanged native domain. Feeding displaced
                        // coordinates back into relief would shift the geology.
                        let vb=native[b as usize];let t=level as f32/radial as f32;
                        let p=cp.lerp(Vec3::from(vb.pos),t);
                        let nn=cn.lerp(Vec3::from(vb.normal),t).normalize();
                        let rd=cr+(vb.uv[0]-cr)*t;
                        ring.push(verts.len()as u32);native.push(v(p,nn,[rd,0.0]));verts.push(detailed_vertex(p,nn,rd));
                    }}
                    for k in 0..ring.len() {
                        let next=(k+1)%ring.len();
                        if level==1 {idx.extend_from_slice(&[centre,ring[k],ring[next]]);}
                        else {idx.extend_from_slice(&[previous[k],ring[k],ring[next],previous[k],ring[next],previous[next]]);}
                    }
                    previous=ring;
                }
            }
        }
    }
    (verts,idx)
}

/// All chunks' heights on one 2 m grid (cells outside the corridor are deep).
fn heightfield(chunks: &[Chunk]) -> render::scene::Heightfield {
    let per = N - 1;
    let lo = chunks.iter().fold((i32::MAX, i32::MAX), |a, c| (a.0.min(c.cell.0), a.1.min(c.cell.1)));
    let hi = chunks.iter().fold((i32::MIN, i32::MIN), |a, c| (a.0.max(c.cell.0), a.1.max(c.cell.1)));
    let w = ((hi.0 - lo.0 + 1) as usize * per + 1) as u32;
    let h = ((hi.1 - lo.1 + 1) as usize * per + 1) as u32;
    let mut heights = vec![-1e9f32; (w * h) as usize];
    for c in chunks {
        let (ox, oz) = ((c.cell.0 - lo.0) as usize * per, (c.cell.1 - lo.1) as usize * per);
        for j in 0..N {
            for i in 0..N {
                heights[(oz + j) * w as usize + ox + i] = c.heights[j * N + i];
            }
        }
    }
    render::scene::Heightfield { origin: [lo.0 as f32 * 64.0, lo.1 as f32 * 64.0], cell: GRID as f32, w, h, heights }
}

fn native_terrain_normal(s: &Stage, p: Vec3) -> Vec3 {
    let gx=p.x as f64/GRID;let gz=p.z as f64/GRID;
    let i=gx.floor();let j=gz.floor();let u=(gx-i)as f32;let w=(gz-j)as f32;
    let normal=|dx:f64,dz:f64| {
        let x=(i+dx)*GRID;let z=(j+dz)*GRID;
        Vec3::new((s.height(x-GRID,z)as f32-s.height(x+GRID,z)as f32)/(2.0*GRID as f32),
            1.0,(s.height(x,z-GRID)as f32-s.height(x,z+GRID)as f32)/(2.0*GRID as f32)).normalize()
    };
    if u+w<=1.0 {(normal(0.0,0.0)*(1.0-u-w)+normal(1.0,0.0)*u+normal(0.0,1.0)*w).normalize()}
    else {(normal(1.0,1.0)*(u+w-1.0)+normal(0.0,1.0)*(1.0-u)+normal(1.0,0.0)*(1.0-w)).normalize()}
}

pub(crate) fn road_mesh(s: &Stage, material: u32) -> ClusterMesh {
    let r = builder::road(s);
    let cols = r.cols;
    let rows = r.rows;
    let mut verts = Vec::with_capacity(r.verts.len());
    for i in 0..rows {
        for c in 0..cols {
            let k = i * cols + c;
            let at = |k: usize| Vec3::new(r.verts[k].x, r.verts[k].y, r.verts[k].z);
            let along = at(if i + 1 < rows { k + cols } else { k }) - at(if i > 0 { k - cols } else { k });
            let side = at(if c + 1 < cols { k + 1 } else { k }) - at(if c > 0 { k - 1 } else { k });
            let mut n = side.cross(along).normalize_or(Vec3::Y);
            if n.y < 0.0 {
                n = -n;
            }
            verts.push(v(at(k), n, r.uvs[k]));
        }
    }
    // The shoulder has extra vertices at every terrain-triangle crossing.
    // Keep its road-space UVs and smooth normals continuous with the original
    // ribbon; collision consumes these exact same positions and triangles.
    for k in rows * cols..r.verts.len() {
        let uv = r.uvs[k];
        let c = if uv[0] < -1.1 { 0 } else if uv[0] < 0.0 { 1 }
            else if uv[0] > 1.1 { cols - 1 } else { cols - 2 };
        let row = (uv[1] / sideways::stage::STEP as f32).clamp(0.0, (rows - 1) as f32);
        let i = row.floor() as usize;
        let a = Vec3::from_array(verts[i * cols + c].normal);
        let b = Vec3::from_array(verts[(i + 1).min(rows - 1) * cols + c].normal);
        let p = r.verts[k];
        verts.push(v(Vec3::new(p.x, p.y, p.z), a.lerp(b, row.fract()).normalize_or(Vec3::Y), uv));
    }
    for vertex in &mut verts {
        let t=((vertex.uv[0].abs()-1.0)/0.25).clamp(0.0,1.0);
        if t>0.0 {
            let terrain=native_terrain_normal(s,Vec3::from(vertex.pos));
            vertex.normal=Vec3::from(vertex.normal).lerp(terrain,t*t*(3.0-2.0*t)).normalize().to_array();
        }
    }
    let mut idx = Vec::new();
    for t in &r.tris {
        // Godot's winding faces down; Kern draws both sides of the ground
        // anyway, but keep it consistent with the terrain.
        idx.extend_from_slice(&[t[0], t[2], t[1]]);
    }
    // Terrain already locks the native roadside corridor. Preserve the full
    // joining shoulder here too, in every camera and shadow cut. Position-only
    // simplification could otherwise bridge tens of metres between seam
    // vertices and cast a floating triangular shadow onto the finer road.
    // The road interior remains eligible for its existing CLOD simplification.
    ClusterMesh::build_with_constraints(
        &[Part { vertices: &verts, indices: &idx, material }],
        |_|render::cluster::SimplificationPolicy::Geometry,
        |part,index|part.vertices[index].uv[0].abs()>=1.0,
    )
}

fn box_mesh(half: Vec3, material: u32) -> ClusterMesh {
    let mut verts = Vec::new();
    let mut idx = Vec::new();
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let mut n = Vec3::ZERO;
            n[axis] = sign;
            let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
            let base = verts.len() as u32;
            for (a, b) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let mut p = n * half[axis];
                p[u] = a * half[u];
                p[w] = b * half[w];
                verts.push(v(p, n, [a * 0.5 + 0.5, b * 0.5 + 0.5]));
            }
            let quad = if sign > 0.0 { [0, 1, 2, 0, 2, 3] } else { [0, 2, 1, 0, 3, 2] };
            idx.extend(quad.iter().map(|q| base + q));
        }
    }
    ClusterMesh::build(&[Part { vertices: &verts, indices: &idx, material }])
}

/// A car: two boxes from its collision shape until the glTF bodies load.
fn car_mesh(boxes: &[[f64; 6]], material: u32) -> ClusterMesh {
    let mut verts = Vec::new();
    let mut idx = Vec::new();
    for b in boxes {
        let m = box_mesh(Vec3::new(b[0] as f32, b[1] as f32, b[2] as f32) * 0.5, material);
        // box_mesh built clusters; rebuild from its vertices instead.
        let base = verts.len() as u32;
        for vx in &m.vertices {
            let mut vx = *vx;
            vx.pos = [vx.pos[0] + b[3] as f32, vx.pos[1] + b[4] as f32, vx.pos[2] + b[5] as f32];
            verts.push(vx);
        }
        for c in &m.clusters {
            for t in 0..c.tri_count {
                let p = m.tris[(c.first_tri + t) as usize];
                for k in 0..3 {
                    idx.push(base + m.cluster_verts[(c.first_vert + ((p >> (8 * k)) & 0xff)) as usize]);
                }
            }
        }
    }
    ClusterMesh::build(&[Part { vertices: &verts, indices: &idx, material }])
}

fn side_of(line: &[Vec3], k: usize) -> Vec3 {
    let f = line[(k + 1).min(line.len() - 1)] - line[k.saturating_sub(1)];
    Vec3::new(-f.z, 0.0, f.x).normalize_or(Vec3::X)
}

/// A profile swept along a line (stage_builder.gd _sweep), with normals
/// from the profile's own edges.
fn sweep(line: &[Vec3], profile: &[[f32; 2]], verts: &mut Vec<Vertex>, idx: &mut Vec<u32>) {
    let mut along = 0.0f32;
    for k in 0..line.len() {
        if k > 0 {
            along += line[k].distance(line[k - 1]);
        }
        let right = side_of(line, k);
        for (q, pr) in profile.iter().enumerate() {
            let prev = profile[q.saturating_sub(1)];
            let next = profile[(q + 1).min(profile.len() - 1)];
            let (dx, dy) = (next[0] - prev[0], next[1] - prev[1]);
            let n = (right * dy - Vec3::Y * dx).normalize_or(Vec3::Y);
            verts.push(v(line[k] + right * pr[0] + Vec3::Y * pr[1], n, [along, pr[1]]));
        }
    }
    let pn = profile.len() as u32;
    let base = verts.len() as u32 - pn * line.len() as u32;
    for k in 0..line.len() as u32 - 1 {
        for q in 0..pn - 1 {
            let a = base + k * pn + q;
            idx.extend_from_slice(&[a, a + pn, a + 1, a + 1, a + pn, a + pn + 1]);
        }
    }
}

fn wall_mesh(line: &[Vec3], material: u32) -> ClusterMesh {
    // sweep expects counterclockwise profiles. The previous reversed order
    // pointed both the side normals and triangle winding into the wall, and
    // its top normal down; the sunny side consequently received no direct sun.
    let profile = [[0.28, -0.5], [0.24, 0.62], [0.12, 0.74], [-0.12, 0.74], [-0.24, 0.62], [-0.28, -0.5]];
    let (mut verts, mut idx) = (Vec::new(), Vec::new());
    sweep(line, &profile, &mut verts, &mut idx);
    ClusterMesh::build(&[Part { vertices: &verts, indices: &idx, material }])
}

/// Armco: a W-beam at bumper height on posts every two metres.
fn rail_mesh(line: &[Vec3], material: u32) -> ClusterMesh {
    let beam = [[0.0, 0.42], [0.07, 0.48], [0.0, 0.55], [0.07, 0.62], [0.0, 0.68], [-0.03, 0.68], [-0.03, 0.42]];
    let (mut verts, mut idx) = (Vec::new(), Vec::new());
    sweep(line, &beam, &mut verts, &mut idx);
    for k in 0..line.len() {
        let right = side_of(line, k);
        let f = (line[(k + 1).min(line.len() - 1)] - line[k.saturating_sub(1)]).normalize_or(Vec3::Z);
        let c = line[k] - right * 0.08 + Vec3::Y * 0.1;
        boxed(c, right * 0.05, Vec3::Y * 0.6, f * 0.04, &mut verts, &mut idx);
    }
    ClusterMesh::build(&[Part { vertices: &verts, indices: &idx, material }])
}

/// A box from its centre and three half-axes.
fn boxed(c: Vec3, x: Vec3, y: Vec3, z: Vec3, verts: &mut Vec<Vertex>, idx: &mut Vec<u32>) {
    for (axis, u, w) in [(x, y, z), (y, z, x), (z, x, y)] {
        for s in [-1.0f32, 1.0] {
            let n = (axis * s).normalize_or(Vec3::Y);
            let base = verts.len() as u32;
            for (a, b) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                verts.push(v(c + axis * s + u * a + w * b, n, [a, b]));
            }
            idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
}

pub struct CarPose {
    pub pos: Vec3,
    pub rot: Quat,
    pub color: Vec3,
    pub boxes: Vec<[f64; 6]>,
    /// res://assets/cars/x.glb, or empty for a box car.
    pub model: String,
    pub ride: f32,
    pub wheels: Vec<crate::car_model::Wheel>,
}

pub fn build(stage: &Stage, cars: &[CarPose], cameras: &[Vec3], ground_cover: bool, looks: &crate::car_look::Looks) -> Result<Built, String> {
    build_inner(stage, cars, cameras, ground_cover, false, looks)
}

pub fn build_live(stage: &Stage, cars: &[CarPose], cameras: &[Vec3], looks: &crate::car_look::Looks) -> Result<Built, String> {
    build_inner(stage, cars, cameras, false, std::env::var("SIDEWAYS_LIVE_GRASS").as_deref() != Ok("0"), looks)
}

fn build_inner(stage: &Stage, cars: &[CarPose], cameras: &[Vec3], ground_cover: bool, live_cover: bool, looks: &crate::car_look::Looks) -> Result<Built, String> {
    let mut log = Vec::new();
    let t = std::time::Instant::now();
    let mut scene = Scene::default();
    // Hierarchical instance-group culling: ~268k instances in ~4k groups.
    // Mancave 4K DLSS-P (cc9e99aa): 2 views 11.9 -> 9.9 ms, 4 views 16.6 -> 12.9 ms,
    // same image. KERN_*INSTANCE_GROUPS=0 restores flat culling.
    scene.graphics.instance_groups = true;
    // The receiver escape contract is the SAME bound enforced by surface().
    // Grass and driving ground keep strict native-height occlusion.
    scene.terrain_visual_offset_m=CLIFF_VISUAL_OFFSET;
    let r = root();
    let tex = |scene: &mut Scene, rel: &str| scene.add_texture(&r.join(rel));
    // Terrain layers, in the order shade.wgsl blends them.
    // Decoded on parallel threads: one by one this was ~2.5 s of startup.
    use render::scene::NormalEncoding::{LinearUnorm, SrgbTransferred};
    let files = [
        ("assets/textures/forest_leaves_02_albedo.jpg", LinearUnorm), // moss
        ("assets/textures/forrest_ground_03_albedo.jpg", LinearUnorm), // needles
        ("assets/textures/grass_ground_albedo.jpg", LinearUnorm), // verge
        ("assets/textures/forest_ground_04_albedo.jpg", LinearUnorm), // bare ground
        ("assets/textures/rock_face_03_albedo.jpg", LinearUnorm), // rock
        // This source normal was sRGB-transferred; all other normal assets remain
        // linear data. Decode before the renderer resizes/builds its normal mips.
        ("assets/textures/gravel_ground_01_albedo.jpg", SrgbTransferred),
        ("assets/textures/forest_ground_04_albedo.jpg", LinearUnorm),
        ("assets/trees/trunk_a_albedo.jpg", LinearUnorm),
        ("assets/trees/trunk_b_albedo.jpg", LinearUnorm),
        ("assets/trees/bark_albedo.jpg", LinearUnorm),
        ("assets/trees/authored_pine_albedo.png", LinearUnorm),
    ].map(|(rel, encoding)| (r.join(rel), encoding));
    let layers = scene.add_textures(&files.iter().map(|(path, encoding)| (path.as_path(), *encoding)).collect::<Vec<_>>())?;
    // Terrain layers, in the order shade.wgsl blends them.
    let (t0, gravel, trunk_a, trunk_b, bark, crown) = (layers[0], layers[5], layers[7], layers[8], layers[9], layers[10]);
    scene.load_sky(&r.join("assets/sky/sky_2k.hdr"))?;
    let m = |tint: [f32; 3], layer: i32, flags: u32, scale: f32| Material { tint, layer, roughness: 0.9, flags, scale, metal: 0.0 };
    // A biome's ground colour (--biome), relative to Sideways' own.
    let terrain_mat = scene.add_material(m(stage.vegetation.land_tint, t0, TERRAIN, 4.0));
    let road_mat = scene.add_material(m([1.9, 1.85, 1.8], gravel, ROAD, 3.0));
    let trunk_a_mat = scene.add_material(m([1.0, 1.0, 1.0], trunk_a, 0, 1.0));
    let trunk_b_mat = scene.add_material(m([1.0, 1.0, 1.0], trunk_b, 0, 1.0));
    let bark_mat = scene.add_material(m([1.0, 1.0, 1.0], bark, 0, 1.0));
    // Godot's foliage.gdshader tint.
    let pine_tint = stage.vegetation.pine_tint();
    let crown_mat = scene.add_material(m([0.62 * pine_tint[0], 0.66 * pine_tint[1], 0.55 * pine_tint[2]], crown, ALPHA_TEST | TWO_SIDED, 1.0));
    log.push(format!("vegetation: {:?}, growth {:.3}, phenotype seed {}, stage construction only; evergreen pine / grass tint and stature", stage.vegetation.season, stage.vegetation.growth(), stage.vegetation.seed));
    let barrier_mat = scene.add_material(m([0.55, 0.56, 0.57], -1, 0, 1.0));
    log.push(format!("textures and sky: {:.0} ms", t.elapsed().as_secs_f64() * 1e3));

    // Terrain and trees, chunk by chunk.
    let t = std::time::Instant::now();
    let cells = builder::chunk_cells(stage);
    let chunks: Vec<Chunk> = std::thread::scope(|s| {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let per = cells.len().div_ceil(threads);
        let handles: Vec<_> = cells.chunks(per).map(|part| s.spawn(move || part.iter().map(|c| builder::terrain_chunk(stage, *c)).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    log.push(format!("terrain data: {:.0} ms", t.elapsed().as_secs_f64() * 1e3));
    let t = std::time::Instant::now();
    let adaptive_cliff=std::env::var("SIDEWAYS_ADAPTIVE_CLIFF").as_deref()==Ok("1");
    let mut geometry: Vec<(Vec<Vertex>,Vec<u32>)> = std::thread::scope(|s| {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let per = chunks.len().div_ceil(threads);
        let handles: Vec<_> = chunks.chunks(per).map(|part| s.spawn(move || part.iter().map(|c| {
            if adaptive_cliff {chunk_geometry_tessellation::<true>(c,|x,z|stage.height(x,z)as f32)}
            else {chunk_geometry(c,|x,z|stage.height(x,z)as f32)}
        }).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let normal_started=std::time::Instant::now();
    let source_bytes:usize=geometry.iter().map(|(v,i)|v.capacity()*std::mem::size_of::<Vertex>()+i.capacity()*4).sum();
    let (normal_count,normal_bytes)=terrain_surface_normals(&mut geometry);
    log.push(format!("terrain surface normals: {normal_count} shared detail vertices, {:.1} ms; transient table <={} KiB, retained source buffers {} KiB",normal_started.elapsed().as_secs_f64()*1000.0,normal_bytes.div_ceil(1024),source_bytes.div_ceil(1024)));
    let meshes: Vec<ClusterMesh> = std::thread::scope(|s| {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let per = geometry.len().div_ceil(threads);
        let handles: Vec<_> = geometry.chunks(per).map(|part| s.spawn(move ||part.iter().map(|(v,i)|ClusterMesh::build_with_policy(&[Part {vertices:v,indices:i,material:terrain_mat}],|_|render::cluster::SimplificationPolicy::Terrain)).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h|h.join().unwrap()).collect()
    });
    drop(geometry);
    log.push(format!("unified terrain rock: {} native / {} refined source / {} coarsest cluster triangles; face relief <=0.24 m, total Y offset <=1.10 m, native 2 m road-edge protection; adaptive_cliff={adaptive_cliff}, seeds={}/{}; {}; zero overlay exposures",
        chunks.len()*(N-1)*(N-1)*2,meshes.iter().map(ClusterMesh::leaf_triangles).sum::<usize>(),
        meshes.iter().map(ClusterMesh::root_triangles).sum::<usize>(),stage.noise_seed,stage.detail_seed,
        if adaptive_cliff {"3D edge target 0.75 m, max12 segments and max140 triangles/native face"}
        else {"legacy quarter-edge/two-ring topology"}));
    let mut mesh_names = Vec::new();
    for (k, mesh) in meshes.iter().enumerate() {
        let id = scene.add_mesh(mesh);
        scene.add_classified_instance(id, Affine3A::IDENTITY, GeometryCategory::Terrain);
        mesh_names.push(format!("terrain chunk {:?}", chunks[k].cell));
    }
    scene.aux = stage.road_surfaces.iter().map(|s| *s as u32).collect();
    scene.heightfield = heightfield(&chunks);
    // This assembled field enumerates every loaded native terrain chunk.
    // distant_forest extends both its mesh set and this field together.
    scene.terrain_visibility_complete = true;
    let rm = road_mesh(stage, road_mat);
    let road = scene.add_mesh(&rm);
    mesh_names.push("road".into());
    scene.add_classified_instance(road, Affine3A::IDENTITY, GeometryCategory::Terrain);
    log.push(format!("terrain + road clusters: {:.0} ms", t.elapsed().as_secs_f64() * 1e3));

    // Three shared evergreen archetypes, used unchanged by every distance LOD.
    let t = std::time::Instant::now();
    let tree_fullness = crate::tree_assets::configured_fullness()?;
    let mixed_species = crate::plant_population::configured()?;
    if !mixed_species {
        log.push(format!("tree crown source: fullness={tree_fullness:.3}, stem-anchored frond expansion; shared near/far source, unchanged tree counts, wood/colliders, triangle counts and alpha"));
    }
    let mut tree_meshes = Vec::new();
    let mut tree_source_archetypes = Vec::new();
    let (recipe_forest, tree_pool) = if mixed_species {
        let (log, pool) = crate::plant_population::plant_forest(&mut scene, stage, &chunks, cameras)?;
        (Some(log), Some(pool))
    } else { (None, None) };
    if recipe_forest.is_none() {
    for archetype in sideways::vegetation::TreeArchetype::ALL {
        let surfaces = crate::tree_assets::load_with_fullness(&r, archetype, tree_fullness)?;
        let crown_fronds = crate::tree_assets::crown_frond_count(&surfaces)?;
        let crown_triangles: usize = surfaces.iter().filter(|(name, _, _)|
            !name.contains("trunk") && !name.contains("bark")).map(|(_, _, i)| i.len()/3).sum();
        let source_triangles: usize = surfaces.iter().map(|(_, _, i)| i.len()/3).sum();
        tree_source_archetypes.push(serde_json::json!({"archetype":archetype.label(),
            "fronds":crown_fronds,"crown_triangles":crown_triangles,"source_triangles":source_triangles}));
        let parts: Vec<(Vec<Vertex>, Vec<u32>, u32)> = surfaces
            .into_iter()
            .map(|(name, verts, idx)| {
                let mat = if name.contains("trunk_b") {
                    trunk_b_mat
                } else if name.contains("trunk") {
                    trunk_a_mat
                } else if name.contains("bark") {
                    bark_mat
                } else {
                    crown_mat
                };
                (verts, idx, mat)
            })
            .collect();
        let p: Vec<Part> = parts.iter().map(|(v, i, m)| Part { vertices: v, indices: i, material: *m }).collect();
        let cm = ClusterMesh::build_with_policy(&p, |part| if part.material == crown_mat {
            render::cluster::SimplificationPolicy::AlphaCards
        } else {
            render::cluster::SimplificationPolicy::TexturedGeometry
        });
        log.push(format!("{}: {} leaf tris, {} root tris, {} clusters, {} levels", archetype.label(), cm.leaf_triangles(), cm.root_triangles(), cm.clusters.len(), cm.levels));
        tree_meshes.push(scene.add_mesh(&cm));
        mesh_names.push(format!("evergreen {}", archetype.label()));
    }
    }
    let tree_source = recipe_forest.clone().unwrap_or_else(|| serde_json::json!({"fullness":tree_fullness,
        "recipe":"stem_anchored_frond_expansion_v1","source_archetypes":tree_source_archetypes,
        "shared_near_far":true,"alpha_modified":false,"wood_modified_by_fullness":false}));
    let mut trees = 0;
    let mut families = [0usize; 3]; let mut juveniles = 0usize;
    let mut species_counts = [0usize; 4];
    for c in chunks.iter().filter(|_| !mixed_species) {
        for spot in &c.trees {
            let phenotype = stage.vegetation.tree(stage.noise_seed as u64, spot.pos.0, spot.pos.2);
            let family = phenotype.archetype.index();
            let variant = if mixed_species { crate::plant_population::stage_index(stage, spot.pos.0, spot.pos.2) } else { family };
            let xf = Affine3A::from_scale_rotation_translation(
                crate::tree_assets::native_scale(spot),
                Quat::from_rotation_y(spot.yaw),
                Vec3::new(spot.pos.0, spot.pos.1, spot.pos.2),
            );
            scene.add_classified_instance(tree_meshes[variant], xf, GeometryCategory::Trees);
            scene.mark_foliage();
            trees += 1;
            families[family] += 1; species_counts[variant] += 1; juveniles += usize::from(phenotype.juvenile);
        }
    }
    if let Some(forest) = &recipe_forest {
        log.push(format!("recipe forest: {forest}"));
    } else {
        log.push(format!("trees: {trees} instances, {} canopy / {} tapered conifer / {} stout trunk; {juveniles} juveniles; 3 shared evergreen source archetypes, {:.0} ms", families[0], families[1], families[2], t.elapsed().as_secs_f64() * 1e3));
    }

    // World-fixed trees extend the forested horizon, using the same source
    // models and LOD as the near forest. Native road/physics are retained.
    if !mixed_species && std::env::var("SIDEWAYS_DISTANT_FOREST").as_deref() != Ok("0") {
        match crate::distant_forest::add(&mut scene, stage, &chunks, terrain_mat, &tree_meshes, &mut mesh_names, Default::default(), mixed_species) {
            Ok(summary) => log.push(summary),
            Err(reason) => log.push(format!("distant forest unavailable: {reason}")),
        }
    }
    // Rock detail is part of the native terrain source mesh. The retired
    // twelve-overlay route used a second geometry/normal field and caused
    // visibly different short spans even when it shared the material ID.

    if let Some(summary)=crate::voxel_tree_patch::attach(&mut scene,&tree_meshes,crown_mat,cameras,&r)? {
        log.push(summary);
    }

    // Land without grass (a --biome desert) draws no ground cover at all.
    let bare=stage.land.as_ref().is_some_and(|l|l.grass<0.05);
    let (live_cover,ground_cover)=(live_cover&&!bare,ground_cover&&!bare);
    let meadow_field=crate::grass_field::enabled()&&!bare;
    let procedural_patch=if meadow_field||bare {None} else {
        crate::grass_voxel_asset::attach_procedural_patch(&mut scene,stage,&chunks,cameras,&mut mesh_names)?
    };
    let excluded=procedural_patch.as_ref().map(|patch|patch.excluded);
    let mut procedural_grass=procedural_patch.map(|patch| {log.push(patch.summary);patch.metadata});
    let live_ground_cover = if meadow_field {
        let (field,summary,metadata)=crate::grass_field::build(&scene.heightfield,&chunks,stage.vegetation)?;
        log.push(summary);procedural_grass=Some(metadata);scene.meadow=Some(field);
        None
    } else if live_cover {
        let stream = crate::live_ground_cover::LiveGroundCover::reserve_with_exclusion(&mut scene, &chunks, cameras, &r, &mut mesh_names, stage.vegetation, excluded)?;
        log.push(stream.summary());
        if std::env::var("SIDEWAYS_FAR_GRASS").as_deref() == Ok("1") {
            let photo = crate::ground_cover::photo(&r)?;
            let (field, summary) = crate::far_ground_cover::bake_with_vegetation(&chunks, &photo, stage.vegetation)?;
            log.push(summary);
            scene.far_grass = Some(field);
        }
        Some(stream)
    } else if ground_cover {
        log.push(crate::ground_cover::add_with_exclusion(&mut scene, &chunks, cameras, &r, &mut mesh_names, stage.vegetation, excluded)?);
        None
    } else {
        log.push("ground cover: disabled (matched A/B)".into());
        None
    };

    // Barriers along the ledges: Armco rails or dry-stone walls.
    let stone = tex(&mut scene, "assets/textures/stone_wall_04_albedo.jpg")?;
    let stone_mat = scene.add_material(m([0.78, 0.76, 0.72], stone, TRIPLANAR, 1.0 / 0.6));
    let _ = barrier_mat;
    // stage_builder.gd's steel: Color(0.62, 0.64, 0.66) (sRGB), metallic 0.7, roughness 0.42.
    let steel_mat = scene.add_material(Material { tint: [0.342, 0.368, 0.392], layer: -1, roughness: 0.42, flags: 0, scale: 1.0, metal: 0.7 });
    for line in builder::barrier_lines(stage) {
        let pts: Vec<Vec3> = line.points.iter().map(|p| Vec3::new(p.x, p.y, p.z)).collect();
        let mesh = if line.rail { rail_mesh(&pts, steel_mat) } else { wall_mesh(&pts, stone_mat) };
        let id = scene.add_mesh(&mesh);
        mesh_names.push(if line.rail { "armco".into() } else { "stone wall".into() });
        scene.add_classified_instance(id, Affine3A::IDENTITY, if line.rail { GeometryCategory::Other } else { GeometryCategory::Terrain });
    }
    // Animated world actors precede the contiguous CPU-updated car range.
    if let Some(summary)=crate::roadside_crowds::append(&mut scene,stage,&chunks,&mut mesh_names)? {
        log.push(summary);
    }
    let mut car_textures = std::collections::HashMap::new();
    let mut dynamic_cars = Vec::new();
    let mut car_slots = Vec::new();
    for (index, car) in cars.iter().enumerate() {
        let first = scene.instances.len();
        let pose = Affine3A::from_rotation_translation(car.rot, car.pos);
        if let Some(rel) = car.model.strip_prefix("res://") {
            let t = std::time::Instant::now();
            let before = scene.meshes.len();
            let meshes = crate::car_model::load(&mut scene, &r.join(rel), index, car.color.to_array(), looks, &mut car_slots, &mut car_textures)?;
            for k in before..scene.meshes.len() {
                mesh_names.push(format!("car {rel} part {}", k - before));
            }
            let part_names=crate::car_model::place(&mut scene, &meshes, pose, car.ride, &car.wheels);
            dynamic_cars.push(DynamicCar { first, meshes, model:car.model.clone(), part_names, fallback: false });
            log.push(format!("car {rel}: {:.0} ms", t.elapsed().as_secs_f64() * 1e3));
        } else {
            let mat = scene.add_material(m(car.color.to_array(), -1, 0, 1.0));
            let id = scene.add_mesh(&car_mesh(&car.boxes, mat));
            mesh_names.push("car box".into());
            scene.add_instance(id, pose);
            dynamic_cars.push(DynamicCar { first, model:car.model.clone(), part_names:vec!["body"], meshes: crate::car_model::CarMeshes { body: id, wheels: [None; 4] }, fallback: true });
        }
        // Label every body, glass and wheel instance, including the fallback car.
        scene.set_instance_categories(first..scene.instances.len(), GeometryCategory::Cars);
    }
    log.push(scene.stats());
    Ok(Built { scene, mesh_names, log, cars: dynamic_cars, car_slots, live_ground_cover, procedural_grass, tree_source, tree_pool })
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn stone_wall_top_faces_up_and_sides_face_outward_with_matching_winding() {
        let mesh = wall_mesh(&[Vec3::ZERO, Vec3::Z * 4.0], 0);
        for v in &mesh.vertices {
            let p = Vec3::from(v.pos);
            let n = Vec3::from(v.normal);
            assert!(p.x * n.x > 0.0, "wall side normal points inward: {p:?}, {n:?}");
            if p.y > 0.73 {
                assert!(n.y > 0.8, "top shading normal must point up");
            }
        }
        let mut top_triangles = 0;
        for c in mesh.clusters.iter().filter(|c| c.own_error == 0.0) {
            for packed in &mesh.tris[c.first_tri as usize..(c.first_tri + c.tri_count) as usize] {
                let vertices: [Vertex; 3] = std::array::from_fn(|k| {
                    let local = (packed >> (8 * k)) & 0xff;
                    mesh.vertices[mesh.cluster_verts[(c.first_vert + local) as usize] as usize]
                });
                let p = vertices.map(|v| Vec3::from(v.pos));
                let face = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
                let normal = vertices.iter().map(|v| Vec3::from(v.normal)).sum::<Vec3>().normalize();
                assert!(face.dot(normal) > 0.5, "triangle winding disagrees with shading normals");
                if p.iter().all(|v| v.y > 0.73) {
                    assert!(face.y > 0.99, "top winding faces down");
                    top_triangles += 1;
                } else {
                    let x = p.iter().map(|v| v.x).sum::<f32>();
                    assert!(face.x * x > 0.0, "side winding faces inward");
                }
            }
        }
        assert_eq!(top_triangles, 2);
    }

    #[test]
    fn sloped_terrain_has_the_same_normal_at_all_chunk_edges_and_corners() {
        let heights = (0..N).flat_map(|j| (0..N).map(move |i| {
            (i as f32 * 0.125 - j as f32 * 0.25) * GRID as f32
        })).collect();
        let chunk = Chunk {
            cell: (0, 0), x0: 0.0, z0: 0.0, heights, road_d: vec![20.0; N * N],
            trees: vec![], rocks: vec![], grass: vec![], ferns: vec![], stumps: vec![],
        };
        let mesh = chunk_mesh(&chunk,0,|x,z|(x*0.125-z*0.25)as f32);
        let expected = Vec3::new(-0.125, 1.0, 0.25).normalize();
        assert_eq!(mesh.vertices.len(), N * N);
        for v in &mesh.vertices {
            assert!(Vec3::from(v.normal).abs_diff_eq(expected, 1e-5), "wrong slope normal at {:?}: {:?}", v.pos, v.normal);
        }
    }

    #[test]
    fn curved_terrain_shared_vertices_have_identical_normals_across_chunks() {
        // The old one-sided rule passes a planar-slope test but disagrees
        // across curved seams. Samples on either side must use one world halo.
        let height=|x:f64,z:f64|(x*x*0.003+z*z*0.004+x*z*0.002+(x/8.0).sin()*3.0)as f32;
        let mut shared=std::collections::HashMap::new();let mut matches=0;
        let mut geometry=Vec::new();
        for cell in [(-1,-1),(0,-1),(-1,0),(0,0)] {
            let (x0,z0)=(cell.0 as f64*64.0,cell.1 as f64*64.0);
            let heights=(0..N).flat_map(|j|(0..N).map(move|i|height(x0+i as f64*GRID,z0+j as f64*GRID))).collect();
            let chunk=Chunk {cell,x0,z0,heights,road_d:vec![20.0;N*N],trees:vec![],rocks:vec![],grass:vec![],ferns:vec![],stumps:vec![]};
            geometry.push(chunk_geometry(&chunk,height));
        }
        terrain_surface_normals(&mut geometry);
        for (vertices,_) in geometry {
            for vertex in vertices {
                let key=(vertex.pos[0].to_bits(),vertex.pos[2].to_bits());
                if let Some(previous)=shared.insert(key,vertex.normal.map(f32::to_bits)) {
                    assert_eq!(previous,vertex.normal.map(f32::to_bits),"shared vertex {:?}",vertex.pos);matches+=1;
                }
            }
        }
        assert!(matches>=4*N-1);
    }

    #[test]
    fn cliff_refinement_is_welded_bounded_and_keeps_the_road_exact() {
        let height=|x:f64,z:f64| ((x*0.20).sin()*0.3 + 11.0*((z-10.0)/9.0).clamp(0.0,1.0)
            + (x*0.27).sin()*(z*0.17).cos())as f32;
        let heights=(0..N).flat_map(|j|(0..N).map(move|i|height(i as f64*GRID,j as f64*GRID))).collect();
        let road_d=(0..N).flat_map(|j|(0..N).map(move|_|j as f32*GRID as f32-3.0)).collect();
        let c=Chunk {cell:(0,0),x0:0.0,z0:0.0,heights,road_d,trees:vec![],rocks:vec![],grass:vec![],ferns:vec![],stumps:vec![]};
        let (vertices,indices)=chunk_geometry_tessellation::<true>(&c,height);
        let mut edges=std::collections::HashMap::<(u32,u32),usize>::new();
        let mut detailed=0;
        for vertex in &vertices {
            let p=Vec3::from(vertex.pos);
            let gx=p.x/GRID as f32;let gz=p.z/GRID as f32;
            let i=(gx.floor()as usize).min(N-2);let j=(gz.floor()as usize).min(N-2);
            let u=gx-i as f32;let w=gz-j as f32;
            let h=|i:usize,j:usize|c.heights[j*N+i];
            let native=if u+w<=1.0 {h(i,j)+(h(i+1,j)-h(i,j))*u+(h(i,j+1)-h(i,j))*w}
                else {h(i+1,j+1)+(h(i,j+1)-h(i+1,j+1))*(1.0-u)+(h(i+1,j)-h(i+1,j+1))*(1.0-w)};
            let delta=(p.y-native).abs();
            assert!(delta<=CLIFF_VISUAL_OFFSET+1e-4,"unbounded collider offset at {p:?}: {delta}");
            if vertex.uv[0]<=CLIFF_ROAD_GUARD {assert!(delta<1e-5,"protected road changed at {p:?}");}
            else if delta>0.01 {detailed+=1;}
            assert!((1.0..=2.0).contains(&vertex.uv[1]),"macro slope missing");
            assert!(delta*(vertex.uv[1]-1.0)<=CLIFF_FACE_RELIEF+0.161,"face-relative relief exceeded the geometry bound");
            assert!(p.is_finite()&&Vec3::from(vertex.normal).is_finite());
        }
        for tri in indices.chunks_exact(3) {
            let p=tri.iter().map(|&i|Vec3::from(vertices[i as usize].pos)).collect::<Vec<_>>();
            assert!((p[1]-p[0]).cross(p[2]-p[0]).y>0.0,"folded/degenerate triangle");
            for k in 0..3 {let (a,b)=(tri[k],tri[(k+1)%3]);*edges.entry((a.min(b),a.max(b))).or_default()+=1;}
        }
        for ((a,b),count) in edges {
            assert!(count<=2,"overlapping surface");
            if count==1 {
                let a=Vec3::from(vertices[a as usize].pos);let b=Vec3::from(vertices[b as usize].pos);
                assert!((a.x==0.0&&b.x==0.0)||(a.z==0.0&&b.z==0.0)||(a.x==64.0&&b.x==64.0)||(a.z==64.0&&b.z==64.0),"internal crack: {a:?} -> {b:?}");
            }
        }
        assert!(detailed>500,"fixture must exercise the whole cut, not one local patch");
        assert!(indices.len()/3<(N-1)*(N-1)*2*10,"adaptive cost exceeded 10x even in this cliff fixture");
        eprintln!("cliff fixture: {} native triangles -> {} source triangles; {detailed} detailed vertices; max relief {CLIFF_VISUAL_OFFSET} m",(N-1)*(N-1)*2,indices.len()/3);
    }

    #[test]
    fn road_shoulder_normals_match_the_actual_adjacent_terrain() {
        let stage=Stage::generate(&mut gdcompat::Pcg::new(7),600.0);
        let mesh=road_mesh(&stage,0);
        let mut count=0;
        for vertex in &mesh.vertices {
            if vertex.uv[0].abs()>1.249 {
                let expected=native_terrain_normal(&stage,Vec3::from(vertex.pos));
                assert!(Vec3::from(vertex.normal).abs_diff_eq(expected,1e-5));count+=1;
            }
        }
        assert!(count>500);
    }

    #[test]
    fn reported_road_shoulder_is_source_exact_in_camera_and_shadow_lod_cuts() {
        use std::collections::HashSet;
        let mut rng=gdcompat::Pcg::new(1);rng.next_u32();
        let stage=Stage::generate(&mut rng,3000.0+sideways::stage::RUNOUT);
        assert_eq!((stage.noise_seed,stage.detail_seed),(683407368,2033395789));
        let road=builder::road(&stage);let mesh=road_mesh(&stage,0);
        assert_eq!(mesh.vertices.len(),road.verts.len());
        for (v,p) in mesh.vertices.iter().zip(&road.verts) {
            assert_eq!(v.pos.map(f32::to_bits),[p.x,p.y,p.z].map(f32::to_bits),"cooking must not move the source/collision ribbon");
        }
        let key=|t:[u32;3]| {let k=if t[0]<=t[1]&&t[0]<=t[2]{0}else if t[1]<=t[2]{1}else{2};[t[k],t[(k+1)%3],t[(k+2)%3]]};
        let source:HashSet<_>=road.tris.iter().map(|t|key([t[0],t[2],t[1]])).collect();
        let shoulder:HashSet<_>=source.iter().copied().filter(|t|t.iter().all(|&i|mesh.vertices[i as usize].uv[0].abs()>=1.0)).collect();
        assert!(shoulder.len()>10_000,"exercise both full stage shoulders");
        let mesh_ref=&mesh;
        let triangles=|selected:&dyn Fn(&render::cluster::GpuCluster)->bool|mesh_ref.clusters.iter().filter(|c|selected(c)).flat_map(|c|
            mesh_ref.tris[c.first_tri as usize..(c.first_tri+c.tri_count)as usize].iter().map(move|packed|
                key(std::array::from_fn(|k|mesh_ref.cluster_verts[(c.first_vert+((packed>>(8*k))&255))as usize])))).collect::<HashSet<_>>();
        let check=|cut:HashSet<[u32;3]>| {
            assert!(shoulder.is_subset(&cut),"a locked shoulder source triangle was replaced");
            for tri in cut.iter().filter(|t|t.iter().any(|&i|mesh.vertices[i as usize].uv[0].abs()>1.0)) {
                assert!(source.contains(tri),"a new triangle crosses the protected shoulder: {tri:?}");
            }
        };
        // Source, production 32m/2048 near and 192m/4096 shared sun maps,
        // plus much coarser cuts. This is a geometry invariant, not a bias fix.
        for error in [0.0,1.0/64.0,2.0/(4096.0/192.0),1.0,16.0,1e20] {
            check(triangles(&|c|c.parent_error>error&&c.own_error<=error));
        }
        let cam=Vec3::new(-92.96343,-6.46169,-518.938);
        for height in [1080.0,1440.0,2160.0] {
            let focal=height/(2.0*32f32.to_radians().tan());
            let projected=|e:f32,s:[f32;4]|e*focal/(cam.distance(Vec3::from_slice(&s))-s[3]).max(0.1);
            check(triangles(&|c|projected(c.parent_error,c.parent)>1.0&&projected(c.own_error,c.own)<=1.0));
        }
        let far=triangles(&|c|c.parent_error*(4096.0/192.0)>2.0&&c.own_error*(4096.0/192.0)<=2.0);
        assert!(far.len()<source.len(),"unlocked drivable interior must still simplify");
        eprintln!("reported road: {} source triangles, {} exact shoulder triangles, {} shared-sun triangles",source.len(),shoulder.len(),far.len());
    }

    #[test]
    fn refined_cliff_matches_vertices_normals_and_segments_across_chunk_boundaries() {
        use std::collections::{BTreeMap,BTreeSet,HashMap};
        let height=|x:f64,z:f64| {
            let t=((x+z-53.0)/15.0).clamp(0.0,1.0);
            (15.0*t*t*(3.0-2.0*t)+0.35*(x*0.31).sin()*(z*0.27).cos())as f32
        };
        let mut boundaries=Vec::new();
        let mut geometry=Vec::new();
        for cell in [(0,0),(1,0),(0,1)] {
            let x0=cell.0 as f64*64.0;let z0=cell.1 as f64*64.0;
            let heights=(0..N).flat_map(|j|(0..N).map(move|i|height(x0+i as f64*GRID,z0+j as f64*GRID))).collect();
            let chunk=Chunk {cell,x0,z0,heights,road_d:vec![12.0;N*N],trees:vec![],rocks:vec![],grass:vec![],ferns:vec![],stumps:vec![]};
            geometry.push(chunk_geometry_tessellation::<true>(&chunk,height));
        }
        terrain_surface_normals(&mut geometry);
        for (vertices,indices) in geometry {
            let mut edges=HashMap::<(u32,u32),usize>::new();
            for tri in indices.chunks_exact(3) {for i in 0..3 {let (a,b)=(tri[i],tri[(i+1)%3]);*edges.entry((a.min(b),a.max(b))).or_default()+=1;}}
            let mut points=BTreeMap::new();let mut segments=BTreeSet::new();
            for ((a,b),count) in edges {
                if count!=1 {continue;}
                let va=vertices[a as usize];let vb=vertices[b as usize];
                let p=Vec3::from(va.pos);let q=Vec3::from(vb.pos);
                if (p.x==64.0&&q.x==64.0)||(p.z==64.0&&q.z==64.0) {
                    let pa=(p.x.to_bits(),p.z.to_bits());let pb=(q.x.to_bits(),q.z.to_bits());
                    points.insert(pa,(va.pos[1].to_bits(),va.normal.map(f32::to_bits)));
                    points.insert(pb,(vb.pos[1].to_bits(),vb.normal.map(f32::to_bits)));
                    segments.insert((pa.min(pb),pa.max(pb)));
                }
            }
            boundaries.push((points,segments));
        }
        for (other,axis) in [(1,0),(2,1)] {
            let on_edge=|p:&(u32,u32)|if axis==0 {p.0==64.0f32.to_bits()}else{p.1==64.0f32.to_bits()};
            let a_points:BTreeMap<_,_>=boundaries[0].0.iter().filter(|(p,_)|on_edge(p)).map(|(k,v)|(*k,*v)).collect();
            let b_points:BTreeMap<_,_>=boundaries[other].0.iter().filter(|(p,_)|on_edge(p)).map(|(k,v)|(*k,*v)).collect();
            assert!(a_points.len()>N,"fixture did not subdivide the shared cliff edge");
            assert_eq!(a_points,b_points,"shared cliff positions/normals differ on axis {axis}");
            let a_segments:BTreeSet<_>=boundaries[0].1.iter().filter(|(a,b)|on_edge(a)&&on_edge(b)).copied().collect();
            let b_segments:BTreeSet<_>=boundaries[other].1.iter().filter(|(a,b)|on_edge(a)&&on_edge(b)).copied().collect();
            assert_eq!(a_segments,b_segments,"shared cliff has a T junction on axis {axis}");
        }
    }

    #[test]
    fn refined_surface_preserves_actual_road_shoulder_contacts_across_generated_stages() {
        use std::collections::HashMap;
        let mut checked=0;let mut refined_chunks=0;
        for seed in [1,7,31] {
            let stage=Stage::generate(&mut gdcompat::Pcg::new(seed),800.0);
            let road=builder::road(&stage);
            let mut cache=HashMap::<(i32,i32),(Vec<Vertex>,Vec<u32>)>::new();
            for side in [-1.0f32,1.0] {
                let mut boundary:Vec<_>=road.verts.iter().zip(&road.uvs).filter(|(_,uv)|(uv[0]-side*1.25).abs()<1e-6).collect();
                boundary.sort_by(|a,b|a.1[1].total_cmp(&b.1[1]));
                for edge in boundary.windows(2) {
                    for t in [0.25,0.5,0.75] {
                        let a=edge[0].0;let b=edge[1].0;
                        let p=Vec3::new(a.x,a.y,a.z).lerp(Vec3::new(b.x,b.y,b.z),t);
                        let cell=((p.x/64.0).floor()as i32,(p.z/64.0).floor()as i32);
                        let (vertices,indices)=cache.entry(cell).or_insert_with(|| {
                            let c=builder::terrain_chunk(&stage,cell);
                            let geometry=chunk_geometry_tessellation::<true>(&c,|x,z|stage.height(x,z)as f32);
                            refined_chunks+=usize::from(geometry.0.len()>N*N);geometry
                        });
                        let mut hit=None;
                        for tri in indices.chunks_exact(3) {
                            let a=Vec3::from(vertices[tri[0]as usize].pos);
                            let b=Vec3::from(vertices[tri[1]as usize].pos);
                            let c=Vec3::from(vertices[tri[2]as usize].pos);
                            let d=(b.x-a.x)*(c.z-a.z)-(b.z-a.z)*(c.x-a.x);
                            let u=((p.x-a.x)*(c.z-a.z)-(p.z-a.z)*(c.x-a.x))/d;
                            let w=((b.x-a.x)*(p.z-a.z)-(b.z-a.z)*(p.x-a.x))/d;
                            if u>=-1e-5 && w>=-1e-5 && u+w<=1.00001 {
                                hit=Some(a.y+(b.y-a.y)*u+(c.y-a.y)*w);break;
                            }
                        }
                        let h=hit.expect("rendered shoulder terrain must exist");
                        assert!((h-p.y-0.003).abs()<0.001,"rendered road seam moved at {p:?} in seed {seed}: terrain {h}");
                        checked+=1;
                    }
                }
            }
        }
        assert!(checked>3000&&refined_chunks>3);
        eprintln!("actual rendered road contacts: {checked} PASS across 3 stages; {refined_chunks} refined chunks");
    }

    #[test]
    fn native_cliff_uses_one_material_and_preserves_macro_slope_through_clod() {
        let height=|x:f64,z:f64|(x*1.8+(z*0.4).sin()*0.4)as f32;
        let heights=(0..N).flat_map(|j|(0..N).map(move|i|height(i as f64*GRID,j as f64*GRID))).collect();
        let c=Chunk {cell:(0,0),x0:0.0,z0:0.0,heights,road_d:vec![12.0;N*N],trees:vec![],rocks:vec![],grass:vec![],ferns:vec![],stumps:vec![]};
        let mesh=chunk_mesh(&c,7,height);
        assert!(mesh.clusters.iter().all(|cluster|cluster.material==7));
        assert!(mesh.vertices.iter().all(|v|(1.0..=2.0).contains(&v.uv[1])));
        assert!(mesh.vertices.iter().any(|v|(v.normal[1]-(v.uv[1]-1.0)).abs()>0.1),"macro geology must be independent of detailed normals");
        assert!(mesh.root_triangles()<mesh.leaf_triangles(),"detail must retain distant cluster reduction");
    }

    #[test]
    fn cliff_sampling_follows_real_face_length_with_hard_budget() {
        let flat=cliff_edge_segments(Vec3::ZERO,Vec3::new(2.0,0.0,0.0));
        let steep=cliff_edge_segments(Vec3::ZERO,Vec3::new(2.0,8.0,0.0));
        assert_eq!(flat,4);assert!(steep>flat&&steep<=CLIFF_EDGE_MAX);
        assert!(Vec3::new(2.0,8.0,0.0).length()/(steep as f32)<0.75);
        assert!(Vec3::new(2.0,8.0,0.0).length()/4.0>2.0,"old quarter split undersamples this real face");
        for y in [0.0,2.0,8.0,30.0,200.0] {for n in 3..=CLIFF_EDGE_MAX*3 {
            let segments=cliff_edge_segments(Vec3::ZERO,Vec3::new(2.0,y,0.0));
            assert!(segments<=CLIFF_EDGE_MAX);
            let rings=cliff_radial_segments(y,n);
            assert!((2..=3).contains(&rings));assert!(n*(2*rings-1)<=CLIFF_TRIANGLE_BUDGET);
        }}
    }

    #[test]
    fn captured_round_two_wall_gains_samples_without_changing_the_native_surface() {
        // Exact stage of F12 sideways-1792850230938-134304-2: seed1,
        // round2, unchanged3km, one human/no rivals in both rounds.
        let mut rng=gdcompat::Pcg::new(1);rng.next_u32();
        let _=Stage::generate(&mut rng,3000.0+sideways::stage::RUNOUT);
        rng.skip(sideways::race::WHEEL_FX_PARTICLES);rng.randf_global();rng.next_u32();
        let stage=Stage::generate(&mut rng,3000.0+sideways::stage::RUNOUT);
        assert_eq!((stage.noise_seed,stage.detail_seed),(370671355,-305494282));
        let mut old_count=0;let mut new_count=0;let mut old_faces=Vec::new();let mut new_faces=Vec::new();
        let mut geometry=Vec::new();
        for z in -8..=-7 {for x in -2..=-1 {
            let c=builder::terrain_chunk(&stage,(x,z));
            let old=chunk_geometry_tessellation::<false>(&c,|x,z|stage.height(x,z)as f32);
            let new=chunk_geometry_tessellation::<true>(&c,|x,z|stage.height(x,z)as f32);
            old_count+=old.1.len()/3;new_count+=new.1.len()/3;
            // Same native vertices and surface evaluator, including every
            // road/shoulder vertex: this change adds samples, not bulging walls.
            for (a,b) in old.0[..N*N].iter().zip(&new.0[..N*N]) {
                assert_eq!(a.pos.map(f32::to_bits),b.pos.map(f32::to_bits));
                assert_eq!(a.uv.map(f32::to_bits),b.uv.map(f32::to_bits));
            }
            for (mesh,lengths) in [(&old,&mut old_faces),(&new,&mut new_faces)] {
                for tri in mesh.1.chunks_exact(3) {
                    let v=[mesh.0[tri[0]as usize],mesh.0[tri[1]as usize],mesh.0[tri[2]as usize]];
                    if v.iter().all(|v|v.uv[0]>3.4&&v.uv[0]<25.0&&v.uv[1]<1.35) {
                        let p=v.map(|v|Vec3::from(v.pos));
                        let longest=p[0].distance(p[1]).max(p[1].distance(p[2])).max(p[2].distance(p[0]));
                        lengths.push(longest);
                    }
                }
            }
            assert!(new.1.len()/3<=(N-1)*(N-1)*2*CLIFF_TRIANGLE_BUDGET);
            geometry.push(new);
        }}
        assert!(new_count>old_count,"actual captured wall must receive additional samples");
        assert!(new_count<old_count*4,"bounded actual-region source growth");
        assert!(!old_faces.is_empty()&&!new_faces.is_empty(),"fixture must contain genuinely steep wall faces");
        old_faces.sort_by(f32::total_cmp);new_faces.sort_by(f32::total_cmp);
        let p95=|a:&[f32]|a[(a.len()*95/100).min(a.len()-1)];
        assert!(p95(&new_faces)<p95(&old_faces),"steep-wall world-space sampling must improve");
        terrain_surface_normals(&mut geometry);
        for (vertices,indices) in &geometry {for tri in indices.chunks_exact(3) {
            let v=[vertices[tri[0]as usize],vertices[tri[1]as usize],vertices[tri[2]as usize]];
            let p=v.map(|v|Vec3::from(v.pos));let cross=(p[1]-p[0]).cross(p[2]-p[0]);
            assert!(cross.y>0.0,"heightfield topology folded");
            let face=cross.normalize();
            for vertex in v {if vertex.uv[0]>CLIFF_ROAD_GUARD&&vertex.uv[0]<35.0&&vertex.uv[1]-1.0<CLIFF_DETAIL_UP {
                assert!(Vec3::from(vertex.normal).dot(face)>=-2e-6,"new detail reintroduced inward normals");
            }}
        }}
        eprintln!("r23 captured wall source {old_count}->{new_count} triangles; steep-face p95 edge {:.3}->{:.3} m; {} steep triangles",p95(&old_faces),p95(&new_faces),new_faces.len());
    }

    #[test]
    fn captured_adaptive_cliff_visible_cut_keeps_surface_orientation() {
        let mut rng=gdcompat::Pcg::new(1);rng.next_u32();
        let _=Stage::generate(&mut rng,3000.0+sideways::stage::RUNOUT);
        rng.skip(sideways::race::WHEEL_FX_PARTICLES);rng.randf_global();rng.next_u32();
        let stage=Stage::generate(&mut rng,3000.0+sideways::stage::RUNOUT);
        assert_eq!((stage.noise_seed,stage.detail_seed),(370671355,-305494282));
        let cam=Vec3::new(-24.095289,1.317953,-452.8251);
        let target=Vec3::new(-28.131865,0.167953,-451.87842);
        let forward=(target-cam).normalize();let right=forward.cross(Vec3::Y).normalize();let up=right.cross(forward);
        let tan=(1.1170107f32*0.5).tan();let pixels=1080.0/(2.0*tan);
        let mut geometry=Vec::new();
        for z in -8..=-7 {for x in -2..=-1 {
            let c=builder::terrain_chunk(&stage,(x,z));
            geometry.push(chunk_geometry_tessellation::<true>(&c,|x,z|stage.height(x,z)as f32));
        }}
        terrain_surface_normals(&mut geometry);
        let mut source=Vec::new();let mut selected=Vec::new();
        for (v,idx) in &geometry {
            source.extend(idx.chunks_exact(3).map(|t|[v[t[0]as usize],v[t[1]as usize],v[t[2]as usize]]));
            let mesh=ClusterMesh::build_with_policy(&[Part{vertices:v,indices:idx,material:0}],|_|render::cluster::SimplificationPolicy::Terrain);
            let projected=|error:f32,s:[f32;4]|error*pixels/(cam.distance(Vec3::from_slice(&s[..3]))-s[3]).max(0.1);
            for c in mesh.clusters.iter().filter(|c|projected(c.parent_error,c.parent)>1.0&&projected(c.own_error,c.own)<=1.0) {
                selected.extend(mesh.tris[c.first_tri as usize..(c.first_tri+c.tri_count)as usize].iter().map(|t|std::array::from_fn(|k|mesh.vertices[mesh.cluster_verts[(c.first_vert+((t>>(8*k))&255))as usize]as usize])));
            }
        }
        let mut source_hits=Vec::new();
        for (name,tris) in [("source",source),("selected",selected)] {
            let reversed=tris.iter().filter(|t|{let p=t.map(|v|Vec3::from(v.pos));(p[1]-p[0]).cross(p[2]-p[0]).y<=0.0}).count();
            if name=="source" {assert_eq!(reversed,0,"native-domain subdivision must not fold");}
            eprintln!("{name}: {} triangles, {reversed} nonpositive XZ",tris.len());
            for [x,y] in [[704,177],[752,206],[999,2],[631,328]] {
                let ray=(forward+right*((2.0*(x as f32+0.5)/1920.0-1.0)*tan*1920.0/1080.0)+up*((1.0-2.0*(y as f32+0.5)/1080.0)*tan)).normalize();
                let hit=tris.iter().filter_map(|tri| {
                    let p=tri.map(|v|Vec3::from(v.pos));let a=p[1]-p[0];let b=p[2]-p[0];let pv=ray.cross(b);let d=a.dot(pv);
                    if d.abs()<1e-12{return None;}let tv=cam-p[0];let u=tv.dot(pv)/d;let q=tv.cross(a);let v=ray.dot(q)/d;let t=b.dot(q)/d;
                    if t<=0.0||u<0.0||v<0.0||u+v>1.0{return None;}
                    let n=(Vec3::from(tri[0].normal)*(1.0-u-v)+Vec3::from(tri[1].normal)*u+Vec3::from(tri[2].normal)*v).normalize();
                    Some((t,n,a.cross(b).normalize(),p))
                }).min_by(|a,b|a.0.total_cmp(&b.0));
                if let Some((t,n,face,p))=hit {
                    assert!(n.dot(face)>0.0,"visible smooth normal entered its actual face");
                    if name=="source" {source_hits.push(t);}
                    else {assert!((t-source_hits.remove(0)).abs()<1e-4,"captured slit must not become a CLOD hole");}
                    let point=cam+ray*t;let native=stage.height(point.x as f64,point.z as f64)as f32;
                    let sun=Vec3::new(0.409576,0.8660254,-0.2867882);let origin=point+face*0.005;
                    let gx=(point.x/2.0).floor()*2.0;let gz=(point.z/2.0).floor()*2.0;let u=(point.x-gx)*0.5;let w=(point.z-gz)*0.5;
                    let a=stage.height(gx as f64,gz as f64)as f32;let b=stage.height((gx+2.0)as f64,gz as f64)as f32;let c=stage.height(gx as f64,(gz+2.0)as f64)as f32;let e=stage.height((gx+2.0)as f64,(gz+2.0)as f64)as f32;
                    let (ground,sx,sz)=if u+w<=1.0{(a+(b-a)*u+(c-a)*w,(b-a)*0.5,(c-a)*0.5)}else{(e+(c-e)*(1.0-u)+(b-e)*(1.0-w),(e-c)*0.5,(e-b)*0.5)};
                    let blocker=tris.iter().filter_map(|tri|{let p=tri.map(|v|Vec3::from(v.pos));let a=p[1]-p[0];let b=p[2]-p[0];let pv=sun.cross(b);let d=a.dot(pv);if d.abs()<1e-12{return None;}let tv=origin-p[0];let u=tv.dot(pv)/d;let q=tv.cross(a);let v=sun.dot(q)/d;let t=b.dot(q)/d;if t<=0.0001||u<0.0||v<0.0||u+v>1.0{return None;}Some(t)}).min_by(f32::total_cmp);
                    let ground_at=|q:Vec3|{let gx=(q.x/2.0).floor()*2.0;let gz=(q.z/2.0).floor()*2.0;let u=(q.x-gx)*0.5;let w=(q.z-gz)*0.5;let h=|x:f32,z:f32|stage.height(x as f64,z as f64)as f32;let a=h(gx,gz);let b=h(gx+2.0,gz);let c=h(gx,gz+2.0);let e=h(gx+2.0,gz+2.0);if u+w<=1.0{a+(b-a)*u+(c-a)*w}else{e+(c-e)*(1.0-u)+(b-e)*(1.0-w)}};
                    let t1=if n.x.abs()<0.9 {n.cross(Vec3::X)}else{n.cross(Vec3::Z)}.normalize();let t2=n.cross(t1);
                    let sky_rays=[(n,2.0,60.0),((n+t1*1.2).normalize(),1.0,40.0),((n-t1*1.2).normalize(),1.0,40.0),((n+t2*1.2).normalize(),1.0,40.0),((n-t2*1.2).normalize(),1.0,40.0)];
                    let sky:f32=sky_rays.iter().map(|(dir,weight,max_t)|{let mut t=0.5f32;for _ in 0..12 {let q=point+n*0.15+*dir*t;if q.y<ground_at(q)-0.3{return 0.0;}t+=(t*0.45).max(0.6);if t>*max_t{break;}}*weight}).sum::<f32>()/6.0;
                    eprintln!("{name} [{x},{y}] d={t:.4} n={n:?} geo={face:?} dot={} native_dy={} grid_dy={} sunrise={} sun_blocker={blocker:?} hf_sky={sky} pos={p:?}",n.dot(face),point.y-native,point.y-ground,sun.y-sx*sun.x-sz*sun.z);
                } else {panic!("captured cliff ray [{x},{y}] missed {name}");}
            }
        }
    }

    #[test]
    fn captured_cliff_normals_match_rendered_faces_without_moving_relief_or_road() {
        let mut rng=gdcompat::Pcg::new(1);rng.next_u32();
        let stage=Stage::generate(&mut rng,3000.0+sideways::stage::RUNOUT);
        assert_eq!((stage.noise_seed,stage.detail_seed),(683407368,2033395789));
        let mut geometry=Vec::new();
        for z in -7..=-5 {for x in -3..=-1 {let c=builder::terrain_chunk(&stage,(x,z));geometry.push(chunk_geometry(&c,|x,z|stage.height(x,z)as f32));}}
        let before=geometry.clone();
        let started=std::time::Instant::now();let (count,bytes)=terrain_surface_normals(&mut geometry);
        let mut tested=0usize;let mut changed=0usize;let mut shared=std::collections::HashMap::new();
        for ((vertices,indices),(original,original_indices)) in geometry.iter().zip(&before) {
            assert_eq!(indices,original_indices,"normal repair must not change topology");
            for (v,old) in vertices.iter().zip(original) {
                assert_eq!(v.pos.map(f32::to_bits),old.pos.map(f32::to_bits),"relief position changed");
                assert_eq!(v.uv.map(f32::to_bits),old.uv.map(f32::to_bits),"material/collider attributes changed");
                if v.uv[0]<=CLIFF_ROAD_GUARD {assert_eq!(v.normal,old.normal,"road normal changed");}
                changed+=usize::from(v.normal!=old.normal);
                let key=(v.pos[0].to_bits(),v.pos[2].to_bits());
                if let Some(previous)=shared.insert(key,v.normal.map(f32::to_bits)) {assert_eq!(previous,v.normal.map(f32::to_bits),"chunk seam normal changed");}
            }
            for tri in indices.chunks_exact(3) {
                let v=[vertices[tri[0]as usize],vertices[tri[1]as usize],vertices[tri[2]as usize]];
                let p=v.map(|v|Vec3::from(v.pos));let face=(p[1]-p[0]).cross(p[2]-p[0]).normalize();
                for vertex in v {if vertex.uv[0]>CLIFF_ROAD_GUARD&&vertex.uv[0]<35.0&&vertex.uv[1]-1.0<CLIFF_DETAIL_UP {
                    assert!(Vec3::from(vertex.normal).dot(face)>=-2e-6,"detail normal points into rendered triangle at {:?}",vertex.pos);tested+=1;
                }}
            }
        }
        assert!(count>10000&&changed>10000&&tested>60000);
        eprintln!("captured cliff normals: {count} shared, {tested} face incidences, {:.1} ms including invariants, temporary table <= {bytes} B",started.elapsed().as_secs_f64()*1000.0);
    }
}
