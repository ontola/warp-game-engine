//! Exclusive-GPU equivalence test; root runs this while no playtest is active.
//! Compares actual selected source IDs, counters and pixels, not draw ordering.
use glam::{Affine3A, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{ALPHA_TEST, FOLIAGE, LIVE_GRASS, NO_SHADOW, Material, TWO_SIDED};
use render::{Camera, FrameStats, Gpu, Lighting, Renderer, Scene, ViewRect};

fn main() { if let Err(e)=run(){eprintln!("{e}");std::process::exit(1);} }

fn add_grid(scene:&mut Scene, material:u32, n:usize)->usize {
    let vertices:Vec<_>=(0..=n).flat_map(|y|(0..=n).map(move|x|Vertex {
        pos:[x as f32/n as f32-0.5,y as f32/n as f32-0.5,0.025*((x*y)as f32).sin()],
        normal:[0.,0.,1.],uv:[x as f32/n as f32,y as f32/n as f32]
    })).collect();
    let indices:Vec<_>=(0..n).flat_map(|y|(0..n).flat_map(move|x| {
        let a=(y*(n+1)+x)as u32;let stride=(n+1)as u32;
        [a,a+1,a+stride,a+1,a+stride+1,a+stride]
    })).collect();
    scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&indices,material}]))
}

fn counters(s:&FrameStats)->[u64;7] {
    [s.clusters as u64,s.triangles as u64,s.submitted_vertices,s.early_grass_rejects,
        s.shadow_clusters as u64,s.shadow_triangles as u64,s.shadow_submitted_vertices]
}

fn compare(gpu:&Gpu, baseline:&Renderer, candidate:&Renderer, views:&[ViewRect], light:&Lighting,
    name:&str,out:&std::path::Path)->Result<String,String> {
    let w=views.iter().map(|v|v.x+v.w).max().unwrap();
    let h=views.iter().map(|v|v.y+v.h).max().unwrap();
    let frame_id=1000+name.bytes().map(u64::from).sum::<u64>();
    baseline.set_gpu_profile_frame_id(frame_id);candidate.set_gpu_profile_frame_id(frame_id);
    let (a,sa)=baseline.try_render_views(gpu,views,light,w,h,&[])?;
    let (b,sb)=candidate.try_render_views(gpu,views,light,w,h,&[])?;
    assert!(!sa.instance_broadphase && sb.instance_broadphase);
    assert_eq!(counters(&sa),counters(&sb),"{name} counters");
    let mut pairs=0;
    let mut sequence=None;
    for view in 0..views.len() {
        let a=baseline.debug_visible_clusters(gpu,view,sequence)?;
        let b=candidate.debug_visible_clusters(gpu,view,sequence)?;
        for diagnostic in [&a,&b] {
            assert_eq!(diagnostic.frame_id,frame_id,"explicit frame identity");
            assert_eq!((diagnostic.view_index,diagnostic.view_count),(view,views.len()as u32));
            let v=views[view];
            assert_eq!(diagnostic.viewport,[v.x,v.y,v.w,v.h]);
            assert_eq!(diagnostic.output_size,[w,h]);
            assert_eq!(diagnostic.input_size,[v.w,v.h]);
            assert_eq!((diagnostic.camera.pos,diagnostic.camera.target),(v.cam.pos,v.cam.target));
        }
        assert_eq!(a.render_sequence,b.render_sequence);
        sequence=Some(a.render_sequence);
        let mut a=a.pairs;let mut b=b.pairs;
        a.sort_unstable();b.sort_unstable();
        assert_eq!(a,b,"{name} source instance/cluster IDs, view {view}");
        assert!(a.windows(2).all(|w|w[0]!=w[1]),"duplicate selected cluster");
        pairs+=a.len();
    }
    assert_eq!(pairs as u32,sa.clusters,"{name}: diagnostic must read all colour views, not a shadow target");
    let max_byte_error=a.iter().zip(&b).map(|(a,b)|a.abs_diff(*b)).max().unwrap_or(0);
    assert!(max_byte_error<=1,"{name}: pixel mismatch {max_byte_error}");
    image::save_buffer(out.join(format!("{name}.png")),&b,w,h,image::ColorType::Rgba8).map_err(|e|e.to_string())?;
    Ok(format!("{{\"name\":\"{name}\",\"views\":{},\"selected_pairs\":{pairs},\"clusters\":{},\"triangles\":{},\"vertices\":{},\"early_grass_rejects\":{},\"max_byte_error\":{max_byte_error}}}",
        views.len(),sb.clusters,sb.triangles,sb.submitted_vertices,sb.early_grass_rejects))
}

fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();
    let out=std::path::PathBuf::from(&args.windows(2).find(|a|a[0]=="--out").ok_or("--out FRESH_DIR required")?[1]);
    if out.exists(){return Err("fresh output directory required".into());}
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let gpu=Gpu::new()?;
    let mut scene=Scene::default();
    scene.sky=Some((16,8,vec![[1.,1.,1.,1.];128]));
    let layer=scene.add_image("broadphase alpha",image::RgbaImage::from_fn(64,64,|x,y|
        image::Rgba([160,220,120,if (x/8+y/8)%2==0{255}else{0}])));
    let opaque=scene.add_material(Material{tint:[0.45,0.3,0.2],layer:-1,roughness:1.,flags:TWO_SIDED,scale:1.,metal:0.});
    let alpha=scene.add_material(Material{tint:[0.3,0.5,0.2],layer,roughness:1.,flags:TWO_SIDED|ALPHA_TEST,scale:1.,metal:0.});
    let simple=add_grid(&mut scene,opaque,1);
    let detailed=add_grid(&mut scene,alpha,12);
    for i in 0..320 {
        let p=Vec3::new((i%20)as f32*1.4-13.3,(i/20)as f32*0.7-5.,-25.-(i%3)as f32);
        scene.add_instance(if i%3==0{detailed}else{simple},Affine3A::from_translation(p));
        if i%3==0{scene.instances.last_mut().unwrap().flags=FOLIAGE;}
    }
    // Empty pool slots and out-of-view instances cross the 65535 dispatch row.
    for i in 320..70_003 {
        scene.add_instance(simple,Affine3A::from_translation(Vec3::new(3000.,0.,20.)));
        if i%2==0{scene.instances.last_mut().unwrap().cluster_count=0;}
    }
    // Grass rejection is counted before frustum rejection by both paths.
    for i in 0..16 {
        scene.update_instance(1000+i,simple,Affine3A::from_translation(Vec3::new(i as f32,0.,-100.)));
        scene.instances[1000+i].flags=LIVE_GRASS|NO_SHADOW;
    }
    let light=Lighting{gi:0.,near_shadows:0.,taa:0.,fog_density:0.,..Default::default()};
    let saved=std::env::var_os("KERN_INSTANCE_BROADPHASE");
    unsafe{std::env::set_var("KERN_INSTANCE_BROADPHASE","0");}
    let baseline=Renderer::new(&gpu,&scene,&light);
    unsafe{std::env::set_var("KERN_INSTANCE_BROADPHASE","1");}
    let candidate=Renderer::new(&gpu,&scene,&light);
    unsafe{match saved{Some(v)=>std::env::set_var("KERN_INSTANCE_BROADPHASE",v),None=>std::env::remove_var("KERN_INSTANCE_BROADPHASE")};}
    assert!(candidate.debug_visible_clusters(&gpu,0,None).is_err(),"unrendered target must not be readable");
    let wind=render::wind::WindState{time_seconds:3.25,strength:1.,..Default::default()};
    baseline.set_wind(&gpu,&wind)?;candidate.set_wind(&gpu,&wind)?;
    let cam=Camera{pos:Vec3::ZERO,target:Vec3::NEG_Z,fov_y:70f32.to_radians(),near:0.1};
    let view=ViewRect{cam,x:0,y:0,w:192,h:128};
    let mut reports=vec![compare(&gpu,&baseline,&candidate,&[view],&light,"mixed-wind",&out)?];
    let four:Vec<_>=(0..4).map(|i|ViewRect{cam:Camera{pos:Vec3::new(i as f32*0.5,0.,0.),
        target:Vec3::new((i as f32-1.5)*5.,0.,-25.),..cam},x:i%2*192,y:i/2*128,w:192,h:128}).collect();
    reports.push(compare(&gpu,&baseline,&candidate,&four,&light,"four-views",&out)?);
    let previous=candidate.debug_visible_clusters(&gpu,3,None)?.render_sequence;
    let fog=Lighting{fog_height_m:0.,fog_start:2.,fog_end:15.,fog_density:1.,fog_culling:true,..light};
    reports.push(compare(&gpu,&baseline,&candidate,&[view],&fog,"fog-hidden",&out)?);
    assert!(candidate.debug_visible_clusters(&gpu,1,None).is_err(),"4-to-1 view change must reject old pooled views");
    assert!(candidate.debug_visible_clusters(&gpu,3,None).is_err(),"old fourth view must be inactive");
    assert!(candidate.debug_visible_clusters(&gpu,0,Some(previous)).is_err(),"stale frame generation must be rejected");
    // More than one indirect dispatch row, after live instance uploads.
    for i in 0..scene.instances.len() {
        scene.update_instance(i,simple,Affine3A::from_scale_rotation_translation(Vec3::splat(0.12),glam::Quat::IDENTITY,
            Vec3::new((i%280)as f32*0.22-30.7,(i/280)as f32*0.12-15.,-100.)));
        scene.instances[i].flags=0;
    }
    baseline.update_instances(&gpu,0,&scene.instances)?;
    candidate.update_instances(&gpu,0,&scene.instances)?;
    reports.push(compare(&gpu,&baseline,&candidate,&[view],&light,"all-visible-70003",&out)?);
    let last=candidate.debug_visible_clusters(&gpu,0,None)?;
    assert_eq!(last.pairs.len(),70_003,"cross-row dispatch must visit every owner");
    for inst in &mut scene.instances{inst.cluster_count=0;}
    baseline.update_instances(&gpu,0,&scene.instances)?;
    candidate.update_instances(&gpu,0,&scene.instances)?;
    reports.push(compare(&gpu,&baseline,&candidate,&[view],&light,"empty",&out)?);
    assert!(candidate.debug_visible_clusters(&gpu,0,None)?.pairs.is_empty());
    std::fs::write(out.join("report.json"),format!("{{\"passed\":true,\"scope\":\"native GPU selection equivalence; not gameplay FPS or DLSS\",\"cases\":[{}]}}",reports.join(","))).map_err(|e|e.to_string())?;
    println!("instance broadphase PASS: source IDs/counters/pixels, wind, fog, updates, 4 views, 70003 owners, empty");
    Ok(())
}
