//! CPU-only snapshot of the production refined terrain functions. No runtime changes.
use glam::{Mat4,Vec3};
use render::cluster::{ClusterMesh,Part,Vertex};
use sideways::{builder::{self,Chunk,N},stage::{Stage,GRID,RUNOUT}};
use serde_json::json;
#[path="../src/procedural_rocks.rs"] mod procedural_rocks;
const SNAPSHOT_HASH:&str="11ce4f2dd2541d5eda89eb8134d901a410cdd5b1a3b6d6854419c3433963a0e0";
const SNAPSHOT_TEXT:&str=r####"fn v(p: Vec3, n: Vec3, uv: [f32; 2]) -> Vertex {
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

"####;
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

fn selected(mesh:&ClusterMesh,cam:Vec3,pixels:f32)->Vec<[Vertex;3]>{
    let projected=|error:f32,sphere:[f32;4]|error*pixels/(cam.distance(Vec3::from_slice(&sphere[..3]))-sphere[3]).max(0.1);
    mesh.clusters.iter().filter(|c|projected(c.parent_error,c.parent)>1.0&&projected(c.own_error,c.own)<=1.0).flat_map(|c|
        mesh.tris[c.first_tri as usize..(c.first_tri+c.tri_count)as usize].iter().map(move|t|std::array::from_fn(|k|mesh.vertices[mesh.cluster_verts[(c.first_vert+((t>>(k*8))&255))as usize]as usize]))).collect()
}
fn hit(origin:Vec3,dir:Vec3,tri:[Vertex;3])->Option<(f32,Vec3,Vec3)> {
    let p=tri.map(|v|Vec3::from(v.pos));let e1=p[1]-p[0];let e2=p[2]-p[0];let pv=dir.cross(e2);let det=e1.dot(pv);
    if det.abs()<1e-12{return None;}let tv=origin-p[0];let u=tv.dot(pv)/det;let qv=tv.cross(e1);let v=dir.dot(qv)/det;let t=e2.dot(qv)/det;
    if t<=0.0||u<0.0||v<0.0||u+v>1.0{return None;}
    let n=(Vec3::from(tri[0].normal)*(1.0-u-v)+Vec3::from(tri[1].normal)*u+Vec3::from(tri[2].normal)*v).normalize();
    Some((t,n,e1.cross(e2).normalize()))
}
fn main(){
    assert!(include_str!("../src/visuals.rs").replace("\r\n","\n").contains(SNAPSHOT_TEXT),"regenerate audit from current production source");
    let adaptive_cliff=std::env::var("SIDEWAYS_ADAPTIVE_CLIFF").as_deref()==Ok("1");
    let mut rng=gdcompat::Pcg::new(1);rng.next_u32();let stage=Stage::generate(&mut rng,3000.0+RUNOUT);
    assert_eq!((stage.noise_seed,stage.detail_seed),(683407368,2033395789));
    let cam=Vec3::new(-95.5089569,6.1492615,-328.5393982);let target=Vec3::new(-87.8702164,0.4992616,-322.0857544);
    let fov=75f32.to_radians();let pixels=1080.0/(2.0*(fov*0.5).tan());
    let forward=(target-cam).normalize();let right=forward.cross(Vec3::Y).normalize();let up=right.cross(forward);
    let vp=Mat4::perspective_infinite_reverse_rh(fov,1920.0/1080.0,0.05)*Mat4::look_at_rh(cam,target,Vec3::Y);
    let sun=Vec3::new(0.409576,0.8660254,0.2867882);
    let mut geometry=Vec::new();
    for z in -7..=-5 {for x in -3..=-1 {let c=builder::terrain_chunk(&stage,(x,z));geometry.push(if adaptive_cliff {chunk_geometry_tessellation::<true>(&c,|x,z|stage.height(x,z)as f32)} else {chunk_geometry(&c,|x,z|stage.height(x,z)as f32)});}}
    let timer=std::time::Instant::now();let normal_stats=terrain_surface_normals(&mut geometry);let normal_ms=timer.elapsed().as_secs_f64()*1000.0;
    let mut source=Vec::new();let mut clod=Vec::new();
    for(v,idx)in &geometry {
        source.extend(idx.chunks_exact(3).map(|t|[v[t[0]as usize],v[t[1]as usize],v[t[2]as usize]]));
        let mesh=ClusterMesh::build_with_policy(&[Part{vertices:v,indices:idx,material:0}],|_|render::cluster::SimplificationPolicy::Terrain);
        clod.extend(selected(&mesh,cam,pixels));
    }
    let mut reports=Vec::new();
    for(name,tris)in [("source",&source),("selected_clod",&clod)] {
        let mut num=0usize;let mut folded=0usize;let mut neg=0usize;let mut anyneg=0usize;let mut minimum=1.0f32;let mut worst=Vec::new();let mut nonpositive_xz=Vec::new();
        for (id,tri)in tris.iter().enumerate(){let p=tri.map(|v|Vec3::from(v.pos));let centre=(p[0]+p[1]+p[2])/3.0;
            let road=tri.iter().map(|v|v.uv[0]).sum::<f32>()/3.0;let macro_up=tri.iter().map(|v|v.uv[1]-1.0).sum::<f32>()/3.0;
            if centre.distance(cam)>50.0||road<2.0||road>35.0||macro_up>0.8{continue;}
            let cross=(p[1]-p[0]).cross(p[2]-p[0]);let gn=cross.normalize();let ns=tri.map(|v|Vec3::from(v.normal));let n=(ns[0]+ns[1]+ns[2]).normalize();let dot=gn.dot(n);
            num+=1;folded+=usize::from(cross.y<=0.0);neg+=usize::from(dot<0.0);anyneg+=usize::from(ns.iter().any(|n|n.dot(gn)<0.0));minimum=minimum.min(dot);
            let clip=vp*centre.extend(1.0);let xy=clip.truncate()/clip.w;let screen=[(xy.x+1.0)*960.0,(1.0-xy.y)*540.0];
            if cross.y<=0.0 {
                let projected=p.map(|p|{let c=vp*p.extend(1.0);let q=c.truncate()/c.w;glam::Vec2::new((q.x+1.0)*960.0,(1.0-q.y)*540.0)});
                let area_pixels=(projected[1]-projected[0]).perp_dot(projected[2]-projected[0]).abs()*0.5;
                let edge_pixels=(0..3).map(|i|projected[i].distance(projected[(i+1)%3])).fold(0.0f32,f32::max);
                nonpositive_xz.push(json!({"triangle":id,"positions":p.map(|p|p.to_array()),"normal":gn.to_array(),"cross":cross.to_array(),"area_m2":cross.length()*0.5,"signed_xz_area_m2":cross.y*0.5,"normal_dot_geo":dot,"screen_centroid":screen,"screen_vertices":projected.map(|p|p.to_array()),"projected_area_px2":area_pixels,"longest_edge_px":edge_pixels,"distance_m":cam.distance(centre),"front_facing":gn.dot(cam-centre)>0.0,"centre_in_front":clip.w>0.0}));
            }
            if clip.w>0.0&&screen[0]>=0.0&&screen[0]<1920.0&&screen[1]>=0.0&&screen[1]<500.0&&gn.dot(cam-centre)>0.0 {
                worst.push((dot,json!({"triangle":id,"pixel":screen,"normal_dot_geo":dot,"normal_dot_sun":n.dot(sun),"geo_dot_sun":gn.dot(sun),"positions":p.map(|p|p.to_array()),"normals":ns.map(|n|n.to_array())})));
            }
        }
        worst.sort_by(|a,b|a.0.total_cmp(&b.0));worst.truncate(20);
        let mut samples=Vec::new();for [x,y] in [[323,72],[1616,349],[580,130],[530,165],[900,230],[477,225],[811,311],[1301,70],[1280,235],[450,110],[280,180],[1000,150],[1410,180]] {
            let ray=(forward+right*((2.0*(x as f32+0.5)/1920.0-1.0)*(fov*0.5).tan()*1920.0/1080.0)+up*((1.0-2.0*(y as f32+0.5)/1080.0)*(fov*0.5).tan())).normalize();
            let nearest=tris.iter().enumerate().filter_map(|(id,t)|hit(cam,ray,*t).map(|h|(id,*t,h))).min_by(|a,b|a.2.0.total_cmp(&b.2.0));
            if let Some((id,t,(distance,n,gn)))=nearest {samples.push(json!({"pixel":[x,y],"triangle":id,"distance":distance,"normal":n.to_array(),"geometric_normal":gn.to_array(),"normal_dot_geo":n.dot(gn),"normal_dot_sun":n.dot(sun),"geo_dot_sun":gn.dot(sun),"front_facing":gn.dot(-ray)>0.0,"positions":t.map(|v|v.pos),"normals":t.map(|v|v.normal)}));}
        }
        reports.push(json!({"variant":name,"all_triangles":tris.len(),"local_cliff_triangles":num,"folded_or_degenerate_xz":folded,"nonpositive_xz_faces":nonpositive_xz,"opposite_average_normal":neg,"opposite_any_vertex_normal":anyneg,"minimum_normal_dot_geo":minimum,"worst_front_visible":worst.into_iter().map(|x|x.1).collect::<Vec<_>>(),"samples":samples}));
    }
    let report=json!({"scope":"CPU production source and actual selected CLOD; no shader or shadow-map attribution","visuals_geometry_snapshot_sha256":SNAPSHOT_HASH,"production_snapshot_verified":true,"adaptive_cliff":adaptive_cliff,"noise_seed":stage.noise_seed,"detail_seed":stage.detail_seed,"normal_rebuild_ms":normal_ms,"shared_detail_vertices":normal_stats.0,"normal_table_byte_bound":normal_stats.1,"seed":1,"camera":cam.to_array(),"variants":reports});
    let heights:Vec<_>=(0..111).flat_map(|z|(0..96).map(move|x|(x,z))).map(|(x,z)|stage.height((-160+x*2)as f64,(-420+z*2)as f64)as f32).collect();
    let hf=json!({"origin":[-160,-420],"cell":2,"width":96,"height":111,"heights":heights,"source":"same authoritative native Stage::height as render heightfield"});
    std::fs::write("C:/dev/perf/warp-sideways-20261024/r21/cliff-native-hf.json",serde_json::to_vec(&hf).unwrap()).unwrap();
    let path=std::env::args().nth(1).expect("out.json");std::fs::write(path,serde_json::to_vec_pretty(&report).unwrap()).unwrap();println!("CPU cliff audit complete");
}
