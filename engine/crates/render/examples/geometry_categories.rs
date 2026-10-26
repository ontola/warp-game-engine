//! Explicit GPU diagnostic, never an automatic test or gameplay benchmark.
//! Category exclusion preserves casters/GI; measured pass deltas are marginal.
use glam::{Affine3A,Mat4,Vec3};
use render::{Camera,FrameStats,Gpu,Lighting,Renderer,Scene,ViewRect};
use render::cluster::{ClusterMesh,Part,Vertex};
use render::scene::{GeometryCategory as Category,Material,TWO_SIDED};

fn quad(scene:&mut Scene,material:u32,points:[[f32;3];4],normal:[f32;3])->usize {
    let vertices:Vec<_>=points.into_iter().zip([[0.,1.],[1.,1.],[1.,0.],[0.,0.]])
        .map(|(pos,uv)|Vertex{pos,normal,uv}).collect();
    scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&[0,1,2,0,2,3],material}]))
}
fn grass_clump(scene:&mut Scene,material:u32)->usize {
    // Match the dedicated renderer's fixed template: three crossed cards,
    // first card at local Z=0, all rooted at Y=0 with physical height 0.5 m.
    let mut vertices=Vec::new();let mut indices=Vec::new();
    for card in 0..3 {
        let angle=std::f32::consts::PI*card as f32/3.;
        let side=Vec3::new(angle.cos(),0.,angle.sin())*0.5;
        let base=vertices.len()as u32;
        for (p,uv) in [(-side,[0.,1.]),(side,[1.,1.]),(-side+Vec3::Y*0.5,[0.,0.]),(side+Vec3::Y*0.5,[1.,0.])] {
            vertices.push(Vertex{pos:p.to_array(),normal:Vec3::Y.to_array(),uv});
        }
        indices.extend([0,2,1,1,2,3].map(|i|base+i));
    }
    scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&indices,material}]))
}
fn owner(scene:&mut Scene,mesh:usize,category:Category,position:Vec3)->usize {
    let index=scene.instances.len();scene.add_instance(mesh,Affine3A::from_translation(position));scene.set_instance_category(index,category);index
}
fn shadows(s:&FrameStats)->[u64;3]{[s.shadow_clusters as u64,s.shadow_triangles as u64,s.shadow_submitted_vertices]}
fn build(gpu:&Gpu,scene:&Scene,light:&Lighting,hide:&str)->Renderer {
    // This standalone tool builds all renderers on one thread before any worker.
    unsafe {std::env::set_var("KERN_DIAGNOSTIC_HIDE",hide);}
    Renderer::new(gpu,scene,light)
}
fn pixel(camera:Camera,p:Vec3,w:u32,h:u32)->(u32,u32) {
    let matrix=Mat4::perspective_infinite_reverse_rh(camera.fov_y,w as f32/h as f32,camera.near)
        *Mat4::look_at_rh(camera.pos,camera.target,Vec3::Y);
    let q=matrix*p.extend(1.);((w as f32*(q.x/q.w*0.5+0.5))as u32,(h as f32*(0.5-q.y/q.w*0.5))as u32)
}
fn main(){if let Err(e)=run(){eprintln!("{e}");std::process::exit(1);}}
fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();
    let out=std::path::PathBuf::from(&args.windows(2).find(|v|v[0]=="--out").ok_or("--out FRESH_DIR required")?[1]);
    if out.exists(){return Err("fresh output directory required".into());}std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    unsafe {for key in ["KERN_INSTANCE_BROADPHASE","KERN_WORLD_TERRAIN_VISIBILITY","KERN_PAGED_FOLIAGE_SUN","KERN_FAR_SUN_SHADOW"] {std::env::set_var(key,"0");}}
    let gpu=Gpu::new()?;let mut scene=Scene::default();scene.sky=Some((16,8,vec![[1.,1.,1.,1.];128]));
    let floor=scene.add_material(Material{tint:[0.45,0.42,0.35],layer:-1,roughness:1.,flags:TWO_SIDED,scale:1.,metal:0.});
    let floor=quad(&mut scene,floor,[[-8.,0.,5.],[8.,0.,5.],[8.,0.,-5.],[-8.,0.,-5.]],[0.,1.,0.]);owner(&mut scene,floor,Category::Terrain,Vec3::ZERO);
    let photo=scene.add_image("diagnostic opaque grass",image::RgbaImage::from_pixel(16,16,image::Rgba([255;4])));
    for (i,category) in [Category::Trees,Category::Grass,Category::Cars,Category::Other].into_iter().enumerate() {
        let grass=category==Category::Grass;
        let material=scene.add_material(Material{tint:[0.15+i as f32*0.1,0.6-i as f32*0.1,0.22],layer:if grass{photo}else{-1},
            roughness:1.,flags:TWO_SIDED|if grass{render::scene::GRASS|render::scene::ALPHA_TEST}else{0},scale:1.,metal:0.});
        let mesh=if grass {grass_clump(&mut scene,material)} else {
            quad(&mut scene,material,[[-0.5,0.,0.],[0.5,0.,0.],[0.5,2.,0.],[-0.5,2.,0.]],[0.,0.,1.])};
        let id=owner(&mut scene,mesh,category,Vec3::new(-3.+i as f32*2.,0.,0.));
        if grass {scene.instances[id].flags|=render::scene::LIVE_GRASS|render::scene::NO_SHADOW;
            scene.grass_ranges.push(render::grass::GrassRange{first:id as u32,count:1,material,width:1.,height:0.5,split:false});}
        if matches!(category,Category::Cars|Category::Other) {
            let material=scene.add_material(Material{tint:[0.1,0.2,0.7],layer:-1,roughness:0.4,flags:TWO_SIDED,scale:1.,metal:0.});
            scene.set_material_blend(material,0.35,true);
            let glass=quad(&mut scene,material,[[-0.4,0.4,0.02],[0.4,0.4,0.02],[0.4,1.8,0.02],[-0.4,1.8,0.02]],[0.,0.,1.]);
            owner(&mut scene,glass,category,Vec3::new(-3.+i as f32*2.,0.,0.));
        }
    }
    scene.far_grass=Some(render::far_grass::FarGrassField {origin:[-8.,-8.],cell:2.,width:9,height:9,heights:vec![0.;81],road:vec![2.;81],
        coverage:vec![255;64],profile:[1.;4],max_height:0.5,albedo:[0.2,0.5,0.1],shading_profile:[[0.2,0.5,0.1,0.5];4]});
    let light=Lighting{debug:2.,gi:0.,contact_occlusion:0.,reflection_occlusion:0.,taa:0.,near_shadows:0.,
        fog_density:0.,lod_pixels:0.,sun_dir:Vec3::new(0.2,0.8,0.5).normalize(),..Default::default()};
    let camera=Camera{pos:Vec3::new(0.,5.,14.),target:Vec3::new(0.,0.7,0.),fov_y:60f32.to_radians(),near:0.1};
    let view=ViewRect{cam:camera,x:0,y:0,w:640,h:360};
    let baseline=build(&gpu,&scene,&light,"none");let (base,base_stats)=baseline.try_render_views(&gpu,&[view],&light,640,360,&[])?;
    let base_selected=baseline.debug_visible_clusters(&gpu,0,None)?;let base_pixels=baseline.debug_geometry_pixels(&gpu,0,Some(base_selected.render_sequence))?;
    assert_eq!(base_pixels.pixels.iter().sum::<u64>()+base_pixels.uncovered,640*360);
    for category in Category::ALL {assert!(base_selected.categories.as_ref().unwrap()[category as usize].instances>0,"baseline category {} absent",category.name());
        assert!(base_pixels.pixels[category as usize]>0,"baseline category {} has no pixels",category.name());}
    image::save_buffer(out.join("baseline.png"),&base,640,360,image::ColorType::Rgba8).map_err(|e|e.to_string())?;
    let mut reports=Vec::new();
    for category in Category::ALL {
        let candidate=build(&gpu,&scene,&light,category.name());
        let (image,stats)=candidate.try_render_views(&gpu,&[view],&light,640,360,&[])?;
        let selected=candidate.debug_visible_clusters(&gpu,0,None)?;let pixels=candidate.debug_geometry_pixels(&gpu,0,Some(selected.render_sequence))?;
        let mut expected:Vec<_>=base_selected.pairs.iter().copied().filter(|p|scene.instances[p[0]as usize].category()!=category).collect();
        let mut actual=selected.pairs.clone();expected.sort_unstable();actual.sort_unstable();assert_eq!(actual,expected,"{} selected geometry",category.name());
        assert_eq!(shadows(&stats),shadows(&base_stats),"{} must retain shadow casters",category.name());
        assert_eq!(pixels.pixels[category as usize],0);assert_eq!(selected.categories.as_ref().unwrap()[category as usize].clusters,0);
        if category==Category::Grass {for (name,_) in &stats.gpu_ms {assert!(!name.starts_with("grass ") && *name!="far grass volume","dedicated grass pass remained");}}
        image::save_buffer(out.join(format!("without-{}.png",category.name())),&image,640,360,image::ColorType::Rgba8).map_err(|e|e.to_string())?;
        let mut glass_check="null".to_string();
        if matches!(category,Category::Cars|Category::Other) {
            // Main visibility cannot see blended triangles. Independently
            // check their screen footprint: once both category surfaces are
            // hidden, this ray must see the same uniform floor as the control.
            let x=if category==Category::Cars {1.} else {3.};
            let pane=pixel(camera,Vec3::new(x,1.,0.02),640,360);
            let floor=pixel(camera,Vec3::new(x,0.,3.),640,360);
            let rgb=|bytes:&[u8],p:(u32,u32)|->Vec<u8>{let at=((p.1*640+p.0)*4)as usize;bytes[at..at+3].to_vec()};
            let actual=rgb(&image,pane);let expected=rgb(&image,floor);
            let base_pane=rgb(&base,pane);let base_floor=rgb(&base,floor);
            assert!(base_pane.iter().zip(&base_floor).any(|(a,b)|a.abs_diff(*b)>8),"glass test must cover a visible object");
            assert!(actual.iter().zip(&expected).all(|(a,b)|a.abs_diff(*b)<=1),"{} blended glass remained: {:?} versus floor {:?}",category.name(),actual,expected);
            glass_check=format!("{{\"pane_pixel\":[{},{}],\"floor_control_pixel\":[{},{}],\"excluded_rgb\":{:?},\"floor_rgb\":{:?},\"baseline_rgb\":{:?},\"passed\":true}}",pane.0,pane.1,floor.0,floor.1,actual,expected,base_pane);
        }
        if category==Category::Trees {
            // A known floor receiver in the tree's sun shadow must keep the
            // same received light after removing the tree's colour geometry.
            let shadow_light=Lighting{debug:9.,..light};
            let shadow_point=Vec3::new(-3.25,0.,-0.625);
            let shadow_camera=Camera{pos:Vec3::new(-3.25,3.,-6.),target:shadow_point,..camera};
            let shadow_view=ViewRect{cam:shadow_camera,..view};
            let (a,_)=baseline.try_render_views(&gpu,&[shadow_view],&shadow_light,640,360,&[])?;
            let (b,_)=candidate.try_render_views(&gpu,&[shadow_view],&shadow_light,640,360,&[])?;
            let (x,y)=pixel(shadow_camera,shadow_point,640,360);let at=((y*640+x)*4)as usize;
            assert_eq!(&a[at..at+3],&b[at..at+3],"received tree shadow changed");
            assert!(a[at]<128,"fixture receiver must actually be shadowed");
        }
        let four:Vec<_>=(0..4).map(|i|ViewRect{cam:Camera{pos:camera.pos+Vec3::X*(i as f32*0.1),..camera},
            x:i%2*320,y:i/2*180,w:320,h:180}).collect();
        candidate.try_render_views(&gpu,&four,&light,640,360,&[])?;
        for i in 0..4 {let d=candidate.debug_visible_clusters(&gpu,i,None)?;
            assert_eq!(d.categories.unwrap()[category as usize].clusters,0);assert_eq!(d.view_count,4);}
        reports.push(format!("{{\"excluded\":\"{}\",\"selected_clusters\":{},\"excluded_pixels\":0,\"shadow_counts_equal\":true,\"four_views\":true,\"glass_screen_check\":{}}}",category.name(),selected.pairs.len(),glass_check));
    }
    // A category relabel after submission must not rewrite the old diagnostic.
    let before=baseline.debug_visible_clusters(&gpu,0,None)?;
    let mut changed=scene.instances[1];changed.set_category(Category::Cars);baseline.update_instances(&gpu,1,&[changed])?;
    let after=baseline.debug_visible_clusters(&gpu,0,Some(before.render_sequence))?;
    assert_eq!(before.categories.unwrap().map(|r|r.clusters),after.categories.unwrap().map(|r|r.clusters));
    std::fs::write(out.join("report.json"),format!("{{\"passed\":true,\"scope\":\"native colour geometry diagnostic; input visibility counts include opaque grass; not additive category timings\",\"cases\":[{}]}}",reports.join(","))).map_err(|e|e.to_string())?;
    println!("geometry category GPU diagnostic PASS");Ok(())
}
