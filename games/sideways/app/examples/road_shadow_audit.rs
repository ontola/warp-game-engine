//! CPU-only, actual production road ribbon versus its orthographic shadow cuts.
#![allow(dead_code)]
#[path="../src/car_look.rs"] mod car_look;
#[path="../src/car_model.rs"] mod car_model;
#[path="../src/distant_forest.rs"] mod distant_forest;
#[path="../src/ground_cover.rs"] mod ground_cover;
#[path="../src/grass_voxel_asset.rs"] mod grass_voxel_asset;
#[path="../src/grass_field.rs"] mod grass_field;
#[path="../src/live_ground_cover.rs"] mod live_ground_cover;
#[path="../src/far_ground_cover.rs"] mod far_ground_cover;
#[path="../src/procedural_rocks.rs"] mod procedural_rocks;
#[path="../src/roadside_crowds.rs"] mod roadside_crowds;
#[path="../src/tree_assets.rs"] mod tree_assets;
#[path="../src/voxel_tree_patch.rs"] mod voxel_tree_patch;
#[path="../src/visuals.rs"] mod visuals;
use glam::Vec3;
use render::cluster::{ClusterMesh,Vertex};
use serde_json::json;

fn triangles(mesh:&ClusterMesh,pixels:f32,threshold:f32)->Vec<[Vertex;3]> {
    mesh.clusters.iter().filter(|c|c.parent_error*pixels>threshold&&c.own_error*pixels<=threshold)
        .flat_map(|c|mesh.tris[c.first_tri as usize..(c.first_tri+c.tri_count)as usize].iter()
            .map(move |tri|std::array::from_fn(|k|mesh.vertices[mesh.cluster_verts[(c.first_vert+((tri>>(8*k))&255))as usize]as usize])))
        .collect()
}
fn hit(o:Vec3,d:Vec3,tri:[Vertex;3])->Option<f32> {
    let [a,b,c]=tri.map(|v|Vec3::from(v.pos));let e=b-a;let f=c-a;let p=d.cross(f);let det=e.dot(p);
    if det.abs()<1e-10{return None;}let q=o-a;let u=q.dot(p)/det;let r=q.cross(e);let v=d.dot(r)/det;let t=f.dot(r)/det;
    (u>=0.&&v>=0.&&u+v<=1.&&t>1e-4&&t<10.).then_some(t)
}
fn main()->Result<(),String> {
    let out=std::env::args().nth(1).ok_or("output JSON required")?;
    let mut rng=gdcompat::Pcg::new(1);rng.next_u32();
    let stage=sideways::stage::Stage::generate(&mut rng,3000.+sideways::stage::RUNOUT);
    assert_eq!((stage.noise_seed,stage.detail_seed),(683407368,2033395789));
    let mesh=visuals::road_mesh(&stage,0);let source=triangles(&mesh,1.,0.);
    let car=Vec3::new(-9.260852813720703,11.925579071044922,-1025.5147705078125);
    let sun=Vec3::new(-0.7232683300971985,0.4694715738296509,0.5064379572868347).normalize();
    let row=stage.points.iter().enumerate().min_by(|(_,a),(_,b)| {
        let d=|p:&gdcompat::math::V3|(Vec3::new(p.x,p.y,p.z)-car).length_squared();d(a).total_cmp(&d(b))
    }).unwrap().0;
    let along=row as f32*sideways::stage::STEP as f32;
    let local=|t:&[Vertex;3]|t.iter().map(|v|v.uv[1]).fold(f32::INFINITY,f32::min)<along+60.
        &&t.iter().map(|v|v.uv[1]).fold(f32::NEG_INFINITY,f32::max)>along-60.;
    let source_local:Vec<_>=source.iter().copied().filter(local).collect();
    let receivers:Vec<_>=source.iter().filter(|t|t.iter().all(|v|(v.uv[1]-along).abs()<35.&&v.uv[0].abs()<=1.))
        .flat_map(|tri|[[0.34,0.33,0.33],[0.7,0.15,0.15],[0.15,0.7,0.15],[0.15,0.15,0.7]].map(|w| {
            let p=(0..3).map(|i|Vec3::from(tri[i].pos)*w[i]).sum::<Vec3>();
            let n=(0..3).map(|i|Vec3::from(tri[i].normal)*w[i]).sum::<Vec3>().normalize();
            let uv:[f32;2]=std::array::from_fn(|k|(0..3).map(|i|tri[i].uv[k]*w[i]).sum());(p,n,uv)
        })).collect();
    let mut cuts=Vec::new();
    // Actual frame.rs uses threshold1 near, threshold2 far. Also measure
    // threshold2 near as the requested falsification, labelled explicitly.
    for (name,pixels,threshold) in [("near_production",64.,1.),("near_threshold2_diagnostic",64.,2.),("far_production",4096./192.,2.)] {
        let cut=triangles(&mesh,pixels,threshold);let local_cut:Vec<_>=cut.iter().copied().filter(local).collect();
        let mut changed=Vec::new();let(mut source_hits,mut selected_hits)=(0,0);
        for &(p,n,uv) in &receivers {
            let texel=1./pixels;let signed=n*if n.dot(sun)+1e-4>=0.{1.}else{-1.};
            let o=p+signed*texel*1.5+sun*texel;
            let nearest=|set:&[[Vertex;3]]|set.iter().filter_map(|&t|hit(o,sun,t)).min_by(f32::total_cmp);
            let a=nearest(&source_local);let b=nearest(&local_cut);
            source_hits+=usize::from(a.is_some());selected_hits+=usize::from(b.is_some());
            if a.is_some()!=b.is_some(){changed.push(json!({"position":p.to_array(),"normal":n.to_array(),"uv":uv,"source_hit_m":a,"selected_hit_m":b}));}
        }
        cuts.push(json!({"name":name,"pixels_per_metre":pixels,"threshold_pixels":threshold,"all_triangles":cut.len(),
            "local_triangles":local_cut.len(),"receiver_samples":receivers.len(),"source_hits":source_hits,"selected_hits":selected_hits,"different":changed}));
    }
    let report=json!({"capture":"sideways-1792831215412-137988-0","noise_seed":stage.noise_seed,"detail_seed":stage.detail_seed,
        "road_row":row,"along_m":along,"source_triangles":source.len(),"local_source_triangles":source_local.len(),"sun":sun.to_array(),"cuts":cuts,
        "limits":["CPU continuous rays, not depth-map raster/PCF/texel quantization","Road-only casters within10m; terrain/tree casters and GI are excluded","Actual production road mesh, normals and cluster DAG; no geometry or lighting changed"]});
    std::fs::write(&out,serde_json::to_vec_pretty(&report).unwrap()).map_err(|e|e.to_string())?;println!("road shadow audit: {out}");Ok(())
}
