//! CPU-only camera/caster CLOD audit of a captured roadside wedge.
//! Usage: roadside_lod_audit capture.json output.json [width height]
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
use serde_json::{Value,json};

#[derive(Clone,Copy)]
struct Triangle { vertices:[Vertex;3], cluster:usize, own_error:f32 }
#[derive(Clone,Copy)]
struct Hit { distance:f32, p:Vec3, n:Vec3, uv:[f32;2], triangle:Triangle }
fn triangles(mesh:&ClusterMesh, pixels:f32, threshold:f32, camera:Option<(Vec3,f32)>)->Vec<Triangle> {
    let projected=|error:f32,bound:[f32;4]|if let Some((position,near))=camera {
        error*pixels/(position.distance(Vec3::from_slice(&bound))-bound[3]).max(near)
    }else{error*pixels};
    mesh.clusters.iter().enumerate().filter(|(_,c)|
        projected(c.parent_error,c.parent)>threshold&&projected(c.own_error,c.own)<=threshold)
        .flat_map(|(ci,c)|mesh.tris[c.first_tri as usize..(c.first_tri+c.tri_count)as usize].iter()
            .map(move |tri|Triangle {cluster:ci,own_error:c.own_error,vertices:std::array::from_fn(|k|
                mesh.vertices[mesh.cluster_verts[(c.first_vert+((tri>>(8*k))&255))as usize]as usize])}))
        .collect()
}
fn ray_hit(o:Vec3,d:Vec3,tri:Triangle,limit:f32)->Option<Hit> {
    let [a,b,c]=tri.vertices.map(|v|Vec3::from(v.pos));let e=b-a;let f=c-a;let p=d.cross(f);let det=e.dot(p);
    if det.abs()<1e-10{return None;}let q=o-a;let u=q.dot(p)/det;let r=q.cross(e);let v=d.dot(r)/det;let t=f.dot(r)/det;
    if !(u>=0.&&v>=0.&&u+v<=1.&&t>1e-4&&t<limit){return None;}
    let weights=[1.-u-v,u,v];let blend=|get:fn(Vertex)->Vec3|tri.vertices.into_iter().zip(weights).map(|(v,w)|get(v)*w).sum::<Vec3>();
    Some(Hit {distance:t,p:o+d*t,n:blend(|v|Vec3::from(v.normal)).normalize_or(Vec3::Y),
        uv:std::array::from_fn(|k|tri.vertices.iter().zip(weights).map(|(v,w)|v.uv[k]*w).sum()),triangle:tri})
}
fn nearest(tris:&[Triangle],o:Vec3,d:Vec3,limit:f32)->Option<Hit> {
    tris.iter().filter_map(|&t|ray_hit(o,d,t,limit)).min_by(|a,b|a.distance.total_cmp(&b.distance))
}
fn describe(hit:Option<Hit>)->Value {match hit {None=>Value::Null,Some(h)=>{
    let p=h.triangle.vertices.map(|v|Vec3::from(v.pos));let geo=(p[1]-p[0]).cross(p[2]-p[0]).normalize_or(Vec3::Y);
    json!({"position":h.p.to_array(),"distance_m":h.distance,"normal":h.n.to_array(),"geometric_normal":geo.to_array(),
        "uv":h.uv,"cluster":h.triangle.cluster,"own_error_m":h.triangle.own_error,
        "vertices":h.triangle.vertices.map(|v|v.pos),"triangle_uvs":h.triangle.vertices.map(|v|v.uv)})
}}}
fn vector(v:&Value)->Result<Vec3,String> {
    let a=v.as_array().ok_or("expected vector")?;if a.len()!=3{return Err("expected three components".into());}
    Ok(Vec3::new(a[0].as_f64().ok_or("number")? as f32,a[1].as_f64().ok_or("number")? as f32,a[2].as_f64().ok_or("number")? as f32))
}
fn native_height(stage:&sideways::stage::Stage,p:Vec3)->f32 {
    let x=p.x as f64/2.;let z=p.z as f64/2.;let i=x.floor();let j=z.floor();let u=x-i;let v=z-j;
    let h=|dx:f64,dz:f64|stage.height((i+dx)*2.,(j+dz)*2.) as f32 as f64;
    if u+v<=1. {(h(0.,0.)*(1.-u-v)+h(1.,0.)*u+h(0.,1.)*v) as f32}
    else{(h(1.,1.)*(u+v-1.)+h(0.,1.)*(1.-u)+h(1.,0.)*(1.-v)) as f32}
}
fn main()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();let path=args.get(1).ok_or("capture.json required")?;
    let out=args.get(2).ok_or("output.json required")?;
    let capture:Value=serde_json::from_slice(&std::fs::read(path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if capture["state"]["race_round"].as_u64()!=Some(1){return Err("this bounded audit supports exact round one only".into());}
    let mut rng=gdcompat::Pcg::new(capture["seed"].as_u64().ok_or("seed")?);rng.next_u32();
    let km=capture["state"]["active_settings"]["stage_km"].as_f64().ok_or("stage length")?;
    let stage=sideways::stage::Stage::generate(&mut rng,km*1000.+sideways::stage::RUNOUT);
    if capture["stage_noise_seed"].as_i64()!=Some(stage.noise_seed as i64)||capture["stage_detail_seed"].as_i64()!=Some(stage.detail_seed as i64) {
        return Err("stage history mismatch".into());
    }
    let c=&capture["views"][0]["camera"];let cam=vector(&c["position"])?;let target=vector(&c["target"])?;
    let fov=c["fov_y"].as_f64().ok_or("fov")? as f32;let near=c["near"].as_f64().ok_or("near")? as f32;
    let width=args.get(3).map(|s|s.parse::<u32>()).transpose().map_err(|e|e.to_string())?.unwrap_or(1920);
    let height=args.get(4).map(|s|s.parse::<u32>()).transpose().map_err(|e|e.to_string())?.unwrap_or(1080);
    let focal=height as f32*0.5/(fov*0.5).tan();let forward=(target-cam).normalize();let right=forward.cross(Vec3::Y).normalize();let up=right.cross(forward);
    let sun=vector(&capture["render"]["sun_direction"])?;
    let mesh=visuals::road_mesh(&stage,0);
    let source=triangles(&mesh,1.,0.,None);let camera=triangles(&mesh,focal,1.,Some((cam,near)));
    let far=triangles(&mesh,4096./192.,2.,None);let close=triangles(&mesh,64.,1.,None);
    let mut pixels=Vec::new();let mut differences=Vec::new();let mut counts=[0u32;4];
    // A bounded dense probe of the reported left shoulder. Coordinates are
    // in the 2048x1152 displayed F12 preview, scaled to the requested render.
    for y in (400..=510).step_by(5) {for x in (840..=990).step_by(5) {
        let pixel=[((x as f32+0.5)/2048.*width as f32).floor()as u32,((y as f32+0.5)/1152.*height as f32).floor()as u32];
        let dir=(forward+right*((pixel[0]as f32+0.5-width as f32*0.5)/focal)
            +up*((height as f32*0.5-pixel[1]as f32-0.5)/focal)).normalize();
        let fine=nearest(&source,cam,dir,200.);let selected=nearest(&camera,cam,dir,200.);
        let Some(receiver)=selected else{continue;};
        if receiver.uv[0].abs()<0.9||receiver.uv[0].abs()>1.251 {continue;}
        counts[0]+=1;
        let bias=|texel:f32|receiver.p+receiver.n*if receiver.n.dot(sun)+1e-4>=0.{texel*1.5}else{-texel*1.5}+sun*texel;
        let far_origin=bias(192./4096.);let near_origin=bias(1./64.);
        let source_hit=nearest(&source,far_origin,sun,10.);let far_hit=nearest(&far,far_origin,sun,10.);
        let close_hit=nearest(&close,near_origin,sun,10.);let fine_near=nearest(&source,near_origin,sun,10.);
        counts[1]+=u32::from(source_hit.is_some());counts[2]+=u32::from(far_hit.is_some());counts[3]+=u32::from(close_hit.is_some());
        let row=json!({"preview_pixel":[x,y],"pixel":pixel,"camera_selected":describe(selected),"camera_source":describe(fine),
            "selected_above_native_terrain_m":receiver.p.y-native_height(&stage,receiver.p),
            "source_height_delta_m":fine.map(|h|receiver.p.y-h.p.y),
            "far_origin":far_origin.to_array(),"source_caster_far_origin":describe(source_hit),"far_caster":describe(far_hit),
            "source_caster_near_origin":describe(fine_near),"near_caster":describe(close_hit)});
        if source_hit.is_some()!=far_hit.is_some(){differences.push(row.clone());}
        if (x==930&&y==440)||(x==900&&y==465)||(x==950&&y==425)||(x==880&&y==480)||(x==960&&y==415)||(x==945&&y==450){pixels.push(row);}
    }}
    let report=json!({"capture":path,"render_size":[width,height],"camera":c,"sun":sun.to_array(),"counts":{
        "road_receiver_samples":counts[0],"source_sun_hits":counts[1],"far_sun_hits":counts[2],"near_sun_hits":counts[3],"false_or_missing_far_hits":differences.len()},
        "triangles":{"source":source.len(),"camera":camera.len(),"far":far.len(),"near":close.len()},"selected_probes":pixels,"different_sun_hits":differences,
        "limits":["Actual production road source and exact camera/orthographic CLOD predicates; frustum rejection irrelevant for rays hitting retained triangles",
            "Continuous CPU rays with production legacy receiver bias; excludes raster depth bias, PCF, texel quantization, tree casters and terrain caster geometry",
            "Native terrain height reports source overlap at each ray hit; roadside terrain is locked at all cuts, no rendered cliff displacement within 2m road guard",
            "Diagnostic receiver sampling only; no renderer, physics or geometry modified"]});
    std::fs::write(out,serde_json::to_vec_pretty(&report).unwrap()).map_err(|e|e.to_string())?;
    println!("roadside audit counts={} triangles={} output={out}",report["counts"],report["triangles"]);Ok(())
}
