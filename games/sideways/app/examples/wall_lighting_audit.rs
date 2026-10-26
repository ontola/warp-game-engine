//! CPU-only reconstruction of captured wall triangles; no GPU or scene assets.
use glam::Vec3;
use render::cluster::{ClusterMesh,Part,SimplificationPolicy,Vertex};
use serde_json::json;

fn geometry(line:&[Vec3])->(Vec<Vertex>,Vec<u32>) {
    let profile=[[0.28,-0.5],[0.24,0.62],[0.12,0.74],[-0.12,0.74],[-0.24,0.62],[-0.28,-0.5]];
    let(mut vertices,mut indices,mut along)=(Vec::new(),Vec::new(),0.0);
    for(k,p)in line.iter().enumerate(){
        if k>0{along+=p.distance(line[k-1]);}
        let f=line[(k+1).min(line.len()-1)]-line[k.saturating_sub(1)];
        let right=Vec3::new(-f.z,0.,f.x).normalize_or(Vec3::X);
        for(q,pr)in profile.iter().enumerate(){
            let a=profile[q.saturating_sub(1)];let b=profile[(q+1).min(profile.len()-1)];
            let normal=(right*(b[1]-a[1])-Vec3::Y*(b[0]-a[0])).normalize_or(Vec3::Y);
            vertices.push(Vertex{pos:(*p+right*pr[0]+Vec3::Y*pr[1]).to_array(),normal:normal.to_array(),uv:[along,pr[1]]});
        }
    }
    for k in 0..line.len()-1{for q in 0..5{let a=(k*6+q)as u32;indices.extend_from_slice(&[a,a+6,a+1,a+1,a+6,a+7]);}}
    (vertices,indices)
}
fn selected(mesh:&ClusterMesh,cam:Vec3,pixels:f32,threshold:f32)->Vec<[Vertex;3]>{
    let projected=|error:f32,sphere:[f32;4]|error/(cam.distance(Vec3::from_array([sphere[0],sphere[1],sphere[2]]))-sphere[3]).max(0.1)*pixels;
    mesh.clusters.iter().filter(|c|projected(c.parent_error,c.parent)>threshold&&projected(c.own_error,c.own)<=threshold).flat_map(|c|{
        mesh.tris[c.first_tri as usize..(c.first_tri+c.tri_count)as usize].iter().map(move|p|std::array::from_fn(|k|mesh.vertices[mesh.cluster_verts[(c.first_vert+((p>>(k*8))&255))as usize]as usize]))
    }).collect()
}
fn hit(origin:Vec3,dir:Vec3,tri:[Vertex;3])->Option<(f32,Vec3,Vec3,[f32;3])>{
    let p=tri.map(|v|Vec3::from(v.pos));let e1=p[1]-p[0];let e2=p[2]-p[0];let pv=dir.cross(e2);let det=e1.dot(pv);
    if det.abs()<1e-12{return None;}let tv=origin-p[0];let b1=tv.dot(pv)/det;let qv=tv.cross(e1);let b2=dir.dot(qv)/det;let t=e2.dot(qv)/det;
    if t<=0.||b1<0.||b2<0.||b1+b2>1.{return None;}
    let n=Vec3::from(tri[0].normal)*(1.-b1-b2)+Vec3::from(tri[1].normal)*b1+Vec3::from(tri[2].normal)*b2;
    Some((t,n,e1.cross(e2).normalize_or_zero(),[1.-b1-b2,b1,b2]))
}
fn terrain_height(stage:&sideways::stage::Stage,p:Vec3)->f32{
    let x=(p.x/2.).floor()*2.;let z=(p.z/2.).floor()*2.;let u=(p.x-x)/2.;let v=(p.z-z)/2.;
    let h=|dx:f32,dz:f32|stage.height((x+dx)as f64,(z+dz)as f64)as f32;
    if u+v<=1.{h(0.,0.)*(1.-u-v)+h(2.,0.)*u+h(0.,2.)*v}else{h(2.,2.)*(u+v-1.)+h(0.,2.)*(1.-u)+h(2.,0.)*(1.-v)}
}
fn terrain_clear(stage:&sideways::stage::Stage,p:Vec3,dir:Vec3,far:f32)->bool{
    let mut t=0.5;for _ in 0..12{let q=p+dir*t;if q.y<terrain_height(stage,q)-0.3{return false;}t+=(t*0.45).max(0.6);if t>far{break;}}true
}
fn main(){
    let mut rng=gdcompat::Pcg::new(1);
    for _ in 1..4{rng.next_u32();sideways::stage::Stage::generate(&mut rng,3000.+sideways::stage::RUNOUT);rng.skip(sideways::race::WHEEL_FX_PARTICLES);rng.randf_global();}
    rng.next_u32();let stage=sideways::stage::Stage::generate(&mut rng,3000.+sideways::stage::RUNOUT);
    assert_eq!((stage.noise_seed,stage.detail_seed),(1049722424,-85257117));
    let cam=Vec3::new(-109.74996185302734,13.654882431030273,-1557.899658203125);
    let target=Vec3::new(-113.76341247558594,12.504881858825684,-1558.6282958984375);
    let fov=1.1170107126235962f32;let forward=(target-cam).normalize();let right=forward.cross(Vec3::Y).normalize();let up=right.cross(forward);
    let sun=Vec3::new(0.7094064354896545,0.5,0.4967317581176758);let mut report=Vec::new();
    let lines:Vec<_>=sideways::builder::barrier_lines(&stage).into_iter().filter(|l|!l.rail).collect();
    for policy in [SimplificationPolicy::Geometry,SimplificationPolicy::TexturedGeometry]{
        let mut triangles=Vec::new();let mut stats=Vec::new();
        for (i,line) in lines.iter().enumerate(){
            let pts:Vec<_>=line.points.iter().map(|p|Vec3::new(p.x,p.y,p.z)).collect();let(v,idx)=geometry(&pts);
            let mesh=ClusterMesh::build_with_policy(&[Part{vertices:&v,indices:&idx,material:0}],|_|policy);
            let selected=selected(&mesh,cam,1440./(2.*(fov*0.5).tan()),1.);
            let bad=selected.iter().filter(|t|{let p=t.map(|v|Vec3::from(v.pos));let n=t.iter().map(|v|Vec3::from(v.normal)).sum::<Vec3>();(p[1]-p[0]).cross(p[2]-p[0]).normalize_or_zero().dot(n.normalize_or_zero())<0.}).count();
            stats.push(json!({"wall":i,"leaf":mesh.leaf_triangles(),"selected":selected.len(),"bad_winding_normals":bad}));
            triangles.extend(selected.into_iter().map(|t|(i,t)));
        }
        let mut samples=Vec::new();
        for y in (660..1000).step_by(12){for x in(200..3300).step_by(48){
            let dir=(forward+right*((2.*(x as f32+0.5)/3840.-1.)*(fov*0.5).tan()*3840./2160.)+up*((1.-2.*(y as f32+0.5)/2160.)*(fov*0.5).tan())).normalize();
            if let Some((wall,tri,(t,n,gn,b)))=triangles.iter().filter_map(|(wall,tri)|hit(cam,dir,*tri).map(|h|(*wall,*tri,h))).min_by(|a,b|a.2.0.total_cmp(&b.2.0)){
                let p=cam+dir*t;let nn=n.normalize();let t1=if nn.x.abs()<0.9{nn.cross(Vec3::X).normalize()}else{nn.cross(Vec3::Z).normalize()};let t2=nn.cross(t1);
                let sky_terrain_clear=[nn,(nn+t1*1.2).normalize(),(nn-t1*1.2).normalize(),(nn+t2*1.2).normalize(),(nn-t2*1.2).normalize()].map(|d|terrain_clear(&stage,p+nn*0.15,d,60.));
                samples.push(json!({"pixel":[x,y],"wall":wall,"distance":t,"normal":n.to_array(),"normal_length":n.length(),"normal_dot_sun":n.normalize_or_zero().dot(sun),"normal_dot_geo":n.normalize_or_zero().dot(gn),"bary":b,"positions":tri.map(|v|v.pos),"normals":tri.map(|v|v.normal),"native_ground_delta":p.y-terrain_height(&stage,p),"sky_terrain_clear":sky_terrain_clear,"sun_terrain_clear":terrain_clear(&stage,p+nn*0.2,sun,50.)}));
            }
        }}
        report.push(json!({"policy":format!("{policy:?}"),"stats":stats,"samples":samples}));
    }
    let path=std::env::args().nth(1).expect("output JSON");std::fs::write(&path,serde_json::to_vec_pretty(&report).unwrap()).unwrap();println!("wall audit: {path}");
}
