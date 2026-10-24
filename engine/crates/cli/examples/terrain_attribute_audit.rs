//! CPU-only audit of the current terrain inputs and actual cluster LOD cuts.
//! No renderer/GPU is created. Matches the r15c capture's seed/camera.
use glam::{Vec2,Vec3};
use render::cluster::{ClusterMesh,Part,Vertex,SimplificationPolicy};
use sideways::{builder::{self,Chunk,N},stage::{Stage,GRID,RUNOUT}};
use serde_json::{json,Value};
fn vertices(c:&Chunk,s:&Stage,halo:bool)->Vec<Vertex> {
    (0..N).flat_map(|j|(0..N).map(move|i| {
        let height=|x:isize,z:isize|if halo&&(x<0||z<0||x>=N as isize||z>=N as isize) {
            s.height(c.x0+x as f64*GRID,c.z0+z as f64*GRID)as f32
        }else{c.heights[z.clamp(0,N as isize-1)as usize*N+x.clamp(0,N as isize-1)as usize]};
        let dx=height(i as isize+1,j as isize)-height(i as isize-1,j as isize);
        let dz=height(i as isize,j as isize+1)-height(i as isize,j as isize-1);
        let sx=if !halo&&(i==0||i==N-1){2.0}else{4.0};let sz=if !halo&&(j==0||j==N-1){2.0}else{4.0};
        let p=c.vertex(i,j);Vertex{pos:[p.x,p.y,p.z],normal:Vec3::new(-dx/sx,1.0,-dz/sz).normalize().to_array(),uv:[c.road_d[j*N+i],0.0]}
    })).collect()
}
fn source(v:&[Vertex],c:&Chunk,p:Vec3)->(Vec3,f32) {
    let q=Vec2::new((p.x-c.x0 as f32)/2.0,(p.z-c.z0 as f32)/2.0);
    let x=(q.x.floor()as usize).min(N-2);let z=(q.y.floor()as usize).min(N-2);let f=q-Vec2::new(x as f32,z as f32);let a=z*N+x;
    let (idx,w)=if f.x+f.y<=1.0 {([a,a+1,a+N],[1.0-f.x-f.y,f.x,f.y])}else{([a+1,a+N,a+N+1],[1.0-f.y,1.0-f.x,f.x+f.y-1.0])};
    let mut n=Vec3::ZERO;let mut uv=0.0;for k in 0..3 {n+=Vec3::from(v[idx[k]].normal)*w[k];uv+=v[idx[k]].uv[0]*w[k];}(n.normalize(),uv)
}
fn stats(mut values:Vec<f32>)->Value {if values.is_empty(){return json!(null)}values.sort_by(f32::total_cmp);json!({"mean":values.iter().sum::<f32>()/values.len()as f32,"p95":values[values.len()*95/100],"max":values.last(),"n":values.len()})}
fn main() {
    let mut rng=gdcompat::Pcg::new(1);rng.next_u32();let stage=Stage::generate(&mut rng,3000.0+RUNOUT);
    assert_eq!((stage.noise_seed,stage.detail_seed),(683407368,2033395789));
    let camera=Vec3::new(-16.447708,1.2655956,-849.5579);let pixels=720.0/(64f32.to_radians()*0.5).tan();
    let indices:Vec<u32>=(0..N-1).flat_map(|z|(0..N-1).flat_map(move|x|{let a=(z*N+x)as u32;let n=N as u32;[a,a+n,a+1,a+1,a+n,a+n+1]})).collect();
    let mut chunks=Vec::new();for z in -15..=-12 {for x in -3..=0 {chunks.push(builder::terrain_chunk(&stage,(x,z)));}}
    let mut result=Vec::new();
    for (name,halo,policy) in [("geometry_one_sided",false,SimplificationPolicy::Geometry),("geometry_halo",true,SimplificationPolicy::Geometry),("terrain_halo",true,SimplificationPolicy::Terrain)] {
        let mut angles=Vec::new();let mut up_errors=Vec::new();let mut uv_errors=Vec::new();let mut masks=Vec::new();let mut selected=0u32;let mut roots=0u32;let mut boundaries=Vec::new();let mut worst=json!(null);let mut worst_error=0.0;
        let data:Vec<_>=chunks.iter().map(|c|vertices(c,&stage,halo)).collect();
        for (k,c) in chunks.iter().enumerate(){let v=&data[k];let mesh=ClusterMesh::build_with_policy(&[Part{vertices:v,indices:&indices,material:0}],|_|policy);
            roots+=mesh.clusters.iter().filter(|cl|cl.parent_error>1e30).map(|cl|cl.tri_count).sum::<u32>();
            for cl in &mesh.clusters {
                let projected=|error:f32,sphere:[f32;4]|error*pixels/(Vec3::from_slice(&sphere[..3]).distance(camera)-sphere[3]).max(0.1);
                if cl.parent_error<1e30&&projected(cl.parent_error,cl.parent)<=1.0 {continue;}
                if projected(cl.own_error,cl.own)>1.0 {continue;}
                selected+=cl.tri_count;
                for t in &mesh.tris[cl.first_tri as usize..(cl.first_tri+cl.tri_count)as usize] {
                    let ids=[t&255,(t>>8)&255,(t>>16)&255].map(|i|mesh.cluster_verts[(cl.first_vert+i)as usize]as usize);
                    let p=ids.iter().map(|i|Vec3::from(v[*i].pos)).sum::<Vec3>()/3.0;
                    let n=ids.iter().map(|i|Vec3::from(v[*i].normal)).sum::<Vec3>().normalize();
                    let uv=ids.iter().map(|i|v[*i].uv[0]).sum::<f32>()/3.0;let (reference,road)=source(v,c,p);
                    angles.push(n.dot(reference).clamp(-1.0,1.0).acos().to_degrees());up_errors.push((n.y-reference.y).abs());uv_errors.push((uv-road).abs());
                    let verge=|d:f32|{let t=((d-6.0)/18.0).clamp(0.0,1.0);1.0-t*t*(3.0-2.0*t)};
                    let error=(verge(uv)-verge(road)).abs();masks.push(error);
                    if error>worst_error {worst_error=error;worst=json!({"position":p.to_array(),"interpolated_road_m":uv,"reference_road_m":road,"verge_mask_error":error});}
                }
            }
            for (other,b) in chunks.iter().enumerate(){if b.cell==(c.cell.0+1,c.cell.1){for z in 0..N {let a=Vec3::from(v[z*N+N-1].normal);let b=Vec3::from(data[other][z*N].normal);boundaries.push(a.dot(b).clamp(-1.0,1.0).acos().to_degrees());}}}
        }
        result.push(json!({"variant":name,"selected_triangles_no_frustum":selected,"root_triangles":roots,"normal_angle_degrees":stats(angles),"normal_y_error":stats(up_errors),"road_distance_error_m":stats(uv_errors),"verge_mask_error":stats(masks),"worst_verge_mask":worst,"chunk_border_normal_angle_degrees":stats(boundaries)}));
    }
    println!("{}",serde_json::to_string_pretty(&json!({"seed":1,"stage_noise_seed":stage.noise_seed,"camera":camera.to_array(),"input_height":1440,"lod_pixels":1,"chunks":chunks.len(),"source_caveat":"Current corrected terrain builder; not a replay of r15c's old nearest-road lookup. Triangle-centroid samples, no frustum cull.","variants":result})).unwrap());
}
