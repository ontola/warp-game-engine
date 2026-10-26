//! Exclusive-GPU transactional material reload regression, not a benchmark.
//! shader_reload_probe --out FRESH_DIRECTORY. Only its own shader copies change.
use glam::{Affine3A, Vec3};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Material, TextureSource, BlendMaterial, Heightfield, TWO_SIDED, BLEND, GRASS, ALPHA_TEST, LIVE_GRASS};
use std::path::{Path, PathBuf};

const W:u32=256;
const H:u32=192;

fn quad(scene:&mut Scene,material:u32,position:Vec3,width:f32,height:f32)->u32 {
    let vertices=[[-width/2.0,0.0,0.0],[width/2.0,0.0,0.0],[-width/2.0,height,0.0],[width/2.0,height,0.0]]
        .map(|pos|Vertex{pos,normal:[0.0,0.0,1.0],uv:[if pos[0]>0.0{1.0}else{0.0},if pos[1]>0.0{0.0}else{1.0}]});
    let mesh=scene.add_mesh(&ClusterMesh::build(&[Part{vertices:&vertices,indices:&[0,1,2,2,1,3],material}]));
    let owner=scene.instances.len() as u32;scene.add_instance(mesh,Affine3A::from_translation(position));owner
}
fn fixture()->Result<Scene,String> {
    let mut scene=Scene::default();
    let base=Material{tint:[0.1,0.3,0.08],layer:-1,roughness:0.8,flags:TWO_SIDED,scale:1.0,metal:0.0};
    let opaque=scene.add_material(base);
    quad(&mut scene,opaque,Vec3::new(8.0,0.0,0.0),2.5,2.0);
    let glass=scene.add_material(Material{flags:TWO_SIDED|BLEND,roughness:0.1,..base});
    scene.blended_materials.insert(glass,BlendMaterial{alpha:0.7,double_sided:true});
    quad(&mut scene,glass,Vec3::new(-8.0,0.0,0.0),2.5,2.0);
    scene.textures.push(TextureSource{name:"reload synthetic grass".into(),rgba:image::RgbaImage::from_pixel(4,4,image::Rgba([70,150,45,255])),normal:None});
    let grass=scene.add_material(Material{layer:0,flags:GRASS|ALPHA_TEST|TWO_SIDED,..base});
    let first=quad(&mut scene,grass,Vec3::ZERO,2.0,0.5);
    scene.instances[first as usize].flags|=LIVE_GRASS;
    scene.grass_ranges.push(render::grass::GrassRange{first,count:1,material:grass,width:2.0,height:0.5,split:false});
    // Its only density patch lies beside, not underneath, the near-grass card.
    scene.far_grass=Some(render::far_grass::FarGrassField{
        origin:[16.0,-8.0],cell:2.0,width:9,height:9,heights:vec![0.0;81],road:vec![8.0;81],coverage:vec![255;64],
        profile:[4.0;4],max_height:0.5,albedo:[0.08,0.22,0.05],
        shading_profile:[[0.08,0.22,0.05,0.125],[0.08,0.22,0.05,0.375],[0.08,0.22,0.05,0.625],[0.08,0.22,0.05,0.875]],
    });
    let vertices=[[-1.0,0.0,0.0],[1.0,0.0,0.0],[-1.0,2.0,0.0],[1.0,2.0,0.0]]
        .map(|pos|Vertex{pos,normal:[0.0,0.0,1.0],uv:[0.0;2]});
    let parts=[Part{vertices:&vertices,indices:&[0,1,2,2,1,3],material:opaque}];
    let asset=render::foliage_voxel::bake(&parts,&scene.materials,&scene.textures,
        render::foliage_voxel::BakeConfig{cell_size:0.25,levels:2,max_texture_texels:0,..Default::default()})?;
    let mesh=scene.add_mesh(&ClusterMesh::build(&parts));
    let owner=scene.instances.len() as u32;
    scene.add_instance(mesh,Affine3A::from_translation(Vec3::new(-16.0,0.0,0.0)));
    scene.voxel_forest=Some(render::foliage_volume_data::pack(&[asset],
        &[render::foliage_volume_data::VolumeBinding{owner,asset:0,crown_material:render::foliage_volume_data::ALL_MATERIALS}],
        &scene.instances,render::foliage_volume_data::PackLimits{levels_per_asset:2,first_level_bounds:false,..Default::default()})?);
    scene.voxel_active_tiles=true;
    scene.heightfield=Heightfield{origin:[-32.0,-16.0],cell:2.0,w:41,h:49,heights:vec![0.0;41*49]};
    Ok(scene)
}
fn camera(x:f32,z:f32)->Camera {
    Camera{pos:Vec3::new(x,2.5,z),target:Vec3::new(x,0.35,0.0),fov_y:8f32.to_radians(),near:0.1}
}
fn views()->Vec<(&'static str,Vec<ViewRect>,u32,u32)> {
    let cameras=[("opaque",camera(8.0,40.0)),("glass",camera(-8.0,40.0)),
        ("near-grass",camera(0.0,50.0)),("far-grass",camera(24.0,70.0)),("voxel",camera(-16.0,40.0))];
    let mut cases:Vec<_>=cameras.into_iter().map(|(name,cam)|(name,vec![ViewRect{cam,x:0,y:0,w:W,h:H}],W,H)).collect();
    cases.push(("split",[camera(0.0,28.0),camera(24.0,70.0),camera(-16.0,40.0),camera(-8.0,40.0)]
        .into_iter().enumerate().map(|(i,cam)|ViewRect{cam,x:(i as u32%2)*W,y:(i as u32/2)*H,w:W,h:H}).collect(),W*2,H*2));
    cases
}
fn captures(renderer:&Renderer,gpu:&Gpu,light:&Lighting,out:&Path,prefix:&str)->Result<Vec<Vec<u8>>,String> {
    let mut result=Vec::new();
    for (name,views,w,h) in views() {
        let (pixels,_)=renderer.try_render_views(gpu,&views,light,w,h,&[])?;
        render::write_png(&out.join(format!("{prefix}-{name}.png")),w,h,&pixels)?;
        if name=="far-grass" && !renderer.far_grass_diagnostics(gpu)?.unwrap().iter().any(|d|d.traced_rays>0){return Err("far-grass probe traced no rays".into());}
        if name=="voxel" && !renderer.voxel_diagnostics(gpu)?.unwrap().iter().any(|d|d.traced_rays>0){return Err("voxel probe traced no rays".into());}
        result.push(pixels);
    }
    Ok(result)
}
fn difference(a:&[u8],b:&[u8])->usize {a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(a,b)|a[..3]!=b[..3]).count()}
fn run()->Result<(),String> {
    let args:Vec<_>=std::env::args().collect();
    let out=PathBuf::from(args.windows(2).find(|a|a[0]=="--out").ok_or("use --out FRESH_DIRECTORY")?[1].clone());
    if out.exists(){return Err("output directory must be fresh".into());}
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let overrides=out.join("shader-overrides");std::fs::create_dir(&overrides).map_err(|e|e.to_string())?;
    // This standalone process has no other threads reading environment yet.
    unsafe {std::env::set_var("KERN_SHADER_DIR",&overrides);}
    let original=include_str!("../src/shaders/shade.wgsl");
    let voxel_original=include_str!("../src/shaders/voxel_forest.wgsl");
    let edited=original.replace("fn apply_distance_fog(","fn previous_apply_distance_fog(")+
        "\nfn apply_distance_fog(radiance:vec3<f32>,distance:f32,direction:vec3<f32>)->vec3<f32>{return vec3<f32>(4000.0,40.0,15.0);}\n";
    let gpu=Gpu::new()?;let scene=fixture()?;
    let light=Lighting{taa:0.0,sharpen:0.0,gi:0.0,near_shadows:0.0,sun_lux:0.0,sky_lux:3000.0,ev100:12.0,
        fog_density:0.0,fog_start:10000.0,fog_end:20000.0,contact_occlusion:0.0,reflection_occlusion:0.0,lod_pixels:0.0,..Default::default()};
    let mut renderer=Renderer::new(&gpu,&scene,&light);
    if renderer.reload_shaders(&gpu)?{return Err("unchanged reload unexpectedly rebuilt".into());}
    let baseline=captures(&renderer,&gpu,&light,&out,"baseline")?;
    // Failure in the LAST material consumer follows successful preparation of
    // opaque, glass and both grass paths. Nothing may commit prematurely.
    std::fs::write(overrides.join("shade.wgsl"),&edited).map_err(|e|e.to_string())?;
    std::fs::write(overrides.join("voxel_forest.wgsl"),format!("{voxel_original}\nBROKEN_VOXEL_EDIT")).map_err(|e|e.to_string())?;
    let rejected=renderer.reload_shaders(&gpu).expect_err("invalid final consumer must reject generation");
    if renderer.shader_generation()!=0{return Err("failed reload advanced generation".into());}
    let retained=captures(&renderer,&gpu,&light,&out,"rejected")?;
    if baseline!=retained{return Err("failed final consumer changed live pixels".into());}
    std::fs::write(overrides.join("voxel_forest.wgsl"),voxel_original).map_err(|e|e.to_string())?;
    if !renderer.reload_shaders(&gpu)? || renderer.shader_generation()!=1{return Err("valid reload did not commit one generation".into());}
    let changed=captures(&renderer,&gpu,&light,&out,"reloaded")?;
    let fresh=Renderer::new(&gpu,&scene,&light);
    let reference=captures(&fresh,&gpu,&light,&out,"fresh-reference")?;
    let changes:Vec<_>=baseline.iter().zip(&changed).map(|(a,b)|difference(a,b)).collect();
    if changes.iter().any(|&n|n<10){return Err(format!("not all consumers visibly exercised: {changes:?}"));}
    if changed!=reference{return Err(format!("reload differs from one coherent fresh generation: {:?}",changed.iter().zip(&reference).map(|(a,b)|difference(a,b)).collect::<Vec<_>>()));}
    // Shared syntax failure must also leave the already-successful generation.
    std::fs::write(overrides.join("shade.wgsl"),"BROKEN_SHARED_EDIT").map_err(|e|e.to_string())?;
    if renderer.reload_shaders(&gpu).is_ok()||renderer.shader_generation()!=1{return Err("invalid shared source committed".into());}
    std::fs::write(overrides.join("shade.wgsl"),original).map_err(|e|e.to_string())?;
    if !renderer.reload_shaders(&gpu)?||renderer.shader_generation()!=2{return Err("restoration failed".into());}
    let restored=captures(&renderer,&gpu,&light,&out,"restored")?;
    if baseline!=restored{return Err("restored generation differs from baseline".into());}
    std::fs::write(out.join("report.json"),format!("{{\"passed\":true,\"cases\":[\"opaque\",\"glass\",\"near-grass\",\"far-grass\",\"voxel\",\"split\"],\"changed_pixels\":{changes:?},\"rejected_generation_pixel_changes\":0,\"fresh_generation_pixel_changes\":0,\"restored_pixel_changes\":0,\"successful_generations\":2,\"last_consumer_error_was_voxel\":{}}}",rejected.contains("voxel reload"))).map_err(|e|e.to_string())?;
    println!("PASS: {}",out.join("report.json").display());Ok(())
}
fn main(){if let Err(error)=run(){eprintln!("shader reload probe: {error}");std::process::exit(1);}}
