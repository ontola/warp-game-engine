//! Exclusive GPU regression; no game may run concurrently.
//! receiver_shadow_demand_gpu --out FRESH_DIRECTORY
//! Same scene/current wind/reference LOD in two renderers. Compare every raw
//! depth texel of every requested tile, plus final native images. Not an FPS test.
use glam::{Affine3A,Mat4,Vec3};
use render::{Camera,Gpu,Lighting,Renderer,Scene,ViewRect};
use render::cluster::{ClusterMesh,Part,Vertex};
use render::scene::{GeometryCategory,Material,TWO_SIDED,ALPHA_TEST,FOLIAGE,NO_SHADOW};
use serde_json::{json,Value};
use sha2::{Digest,Sha256};
use std::path::PathBuf;
const W:u32=480;
const H:u32=288;

fn quad(scene:&mut Scene,p:[[f32;3];4],normal:Vec3,material:u32)->usize {
    let uv=[[0.,1.],[1.,1.],[1.,0.],[0.,0.]];
    let vertices=p.into_iter().enumerate().map(|(i,pos)|Vertex {pos,normal:normal.to_array(),uv:uv[i]}).collect::<Vec<_>>();
    scene.add_mesh(&ClusterMesh::build(&[Part {vertices:&vertices,indices:&[0,1,2,0,2,3],material}]))
}
fn scene()->(Scene,u32) {
    let mut scene=Scene::default();scene.sky_physical=true;
    scene.sky=Some((4,2,vec![[110.,125.,155.,1.];8]));
    let soil=scene.add_material(Material {tint:[0.28,0.24,0.19],layer:-1,roughness:0.9,flags:TWO_SIDED,scale:1.,metal:0.});
    // Independent small owners. A single scene-wide floor sphere would be a
    // valid but saturated no-saving test of this first conservative mechanism.
    let floor=quad(&mut scene,[[-4.,0.,-4.],[-4.,0.,4.],[4.,0.,4.],[4.,0.,-4.]],Vec3::Y,soil);
    for z in -10..=10 {for x in -10..=10 {scene.add_classified_instance(floor,Affine3A::from_translation(Vec3::new(x as f32*8.,0.,z as f32*8.)),GeometryCategory::Terrain);}}
    let alpha=image::RgbaImage::from_fn(16,16,|x,y|image::Rgba([70,145,45,if (x/4+y/4)%2==0 {255}else{0}]));
    let layer=scene.add_image("real cutout blocker",alpha);
    let leaf=scene.add_material(Material {tint:[0.55,0.8,0.4],layer,roughness:0.8,flags:TWO_SIDED|ALPHA_TEST,scale:1.,metal:0.});
    let card=quad(&mut scene,[[0.,0.,-3.],[0.,0.,3.],[0.,8.,3.],[0.,8.,-3.]],Vec3::X,leaf);
    let blocker=scene.instances.len()as u32;
    scene.add_instance(card,Affine3A::from_translation(Vec3::new(18.,0.,0.)));scene.instances.last_mut().unwrap().flags|=FOLIAGE;
    // Behind the camera, but inside the shared sun projection/depth extent.
    // These must remain eligible casters globally, then be omitted only when
    // their projected footprints miss all requested receiver tiles.
    for z in [40.,56.,72.] {for x in -9..=9 {
        scene.add_instance(card,Affine3A::from_translation(Vec3::new(x as f32*8.,0.,z)));
        scene.instances.last_mut().unwrap().flags|=FOLIAGE;
    }}
    // Transparent owners still receive shadows although their clusters are
    // absent from opaque visibility and the shadow caster list.
    let glass=scene.add_material(Material {tint:[0.55,0.75,0.85],layer:-1,roughness:0.25,flags:TWO_SIDED,scale:1.,metal:0.});
    scene.set_material_blend(glass,0.35,true);
    let pane=quad(&mut scene,[[-2.,0.,0.],[2.,0.,0.],[2.,3.,0.],[-2.,3.,0.]],Vec3::Z,glass);
    scene.add_classified_instance(pane,Affine3A::from_translation(Vec3::new(-6.,0.,-5.)),GeometryCategory::Terrain);
    // NO_SHADOW is a caster policy, never a receiver exclusion.
    let patch=quad(&mut scene,[[-2.,0.05,-2.],[-2.,0.05,2.],[2.,0.05,2.],[2.,0.05,-2.]],Vec3::Y,soil);
    scene.add_classified_instance(patch,Affine3A::IDENTITY,GeometryCategory::Terrain);scene.instances.last_mut().unwrap().flags|=NO_SHADOW;
    // One owner spans almost a kilometre, while its actual cluster geometry
    // is sparse. Refine receiver support without changing the mesh or caster
    // LOD. Separate parts guarantee distinct actual bounds, not fake metadata.
    let panels=[-480.,-240.,-60.,0.,60.,240.,480.].map(|x| {
        [[x-2.,0.1,-26.],[x-2.,0.1,-22.],[x+2.,0.1,-22.],[x+2.,0.1,-26.]]
            .map(|pos|Vertex {pos,normal:Vec3::Y.to_array(),uv:[0.,0.]})
    });
    let parts=panels.iter().map(|vertices|Part {vertices,indices:&[0,1,2,0,2,3],material:soil}).collect::<Vec<_>>();
    let wide=scene.add_mesh(&ClusterMesh::build(&parts));scene.add_classified_instance(wide,Affine3A::IDENTITY,GeometryCategory::Terrain);
    (scene,blocker)
}
fn camera(pos:Vec3,target:Vec3)->Camera {Camera {pos,target,fov_y:54f32.to_radians(),near:0.1}}
fn views(cameras:&[Camera])->(Vec<ViewRect>,u32,u32) {
    let cols=if cameras.len()==1 {1}else{2};let rows=(cameras.len()as u32).div_ceil(cols);
    (cameras.iter().enumerate().map(|(i,&cam)|ViewRect {cam,x:(i as u32%cols)*W,y:(i as u32/cols)*H,w:W,h:H}).collect(),W*cols,H*rows)
}
fn read_depth(gpu:&Gpu,texture:&wgpu::Texture)->Result<Vec<u32>,String> {
    let size=texture.width();if size!=texture.height()||texture.format()!=wgpu::TextureFormat::Depth32Float {return Err("depth oracle expects square Depth32Float".into());}
    let pitch=(size*4).div_ceil(256)*256;
    let staging=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("receiver requested-depth oracle"),size:u64::from(pitch)*u64::from(size),
        usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let mut encoder=gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo {texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::DepthOnly},
        wgpu::TexelCopyBufferInfo {buffer:&staging,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(pitch),rows_per_image:Some(size)}},
        wgpu::Extent3d {width:size,height:size,depth_or_array_layers:1});
    gpu.queue.submit([encoder.finish()]);staging.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
    let bytes=staging.slice(..).get_mapped_range().map_err(|e|e.to_string())?;
    let mut result=Vec::with_capacity((size*size)as usize);
    for row in bytes.chunks_exact(pitch as usize) {for b in row[..(size*4)as usize].chunks_exact(4) {result.push(u32::from_le_bytes(b.try_into().unwrap()));}}
    drop(bytes);staging.unmap();Ok(result)
}
fn hash(words:&[u32])->String {let mut hash=Sha256::new();for w in words {hash.update(w.to_le_bytes());}format!("{:x}",hash.finalize())}
fn validate_capture_tags(tags:impl IntoIterator<Item=(u64,u64)>,frame_id:u64,sequence:u64)->Result<(),String> {
    let mut count=0;
    for (actual_frame,actual_sequence) in tags {
        count+=1;
        if actual_frame!=frame_id || actual_sequence!=sequence {
            return Err(format!("stale shadow capture: frame/sequence {actual_frame}/{actual_sequence}, expected {frame_id}/{sequence}"));
        }
    }
    if count==0 {return Err("shadow capture contains no current-frame passes".into());}
    Ok(())
}
fn requested_difference(a:&[u32],b:&[u32],m:&render::receiver_shadow_demand::MapDiagnostics)->Result<Value,String> {
    if a.len()!=b.len() || a.len()!=(m.size*m.size)as usize || m.tile_size!=64 || m.mask.len()!=(m.tiles_per_axis*m.tiles_per_axis)as usize {
        return Err("depth oracle/mask dimensions disagree".into());
    }
    let(mut compared,mut changed,mut occupied)=(0u64,0u64,0u64);let mut first=None;
    for (tile,&requested) in m.mask.iter().enumerate() {if requested==0 {continue;}
        let tx=tile as u32%m.tiles_per_axis;let ty=tile as u32/m.tiles_per_axis;
        for y in ty*64..(ty+1)*64 {for x in tx*64..(tx+1)*64 {
            let i=(y*m.size+x)as usize;compared+=1;occupied+=u64::from(f32::from_bits(a[i])<1.);
            if a[i]!=b[i] {changed+=1;if first.is_none() {first=Some(json!({"pixel":[x,y],"reference_bits":a[i],"candidate_bits":b[i],"reference":f32::from_bits(a[i]),"candidate":f32::from_bits(b[i])}));}}
        }}
    }
    Ok(json!({"map_index":m.map_index,"size":m.size,"tile_size":m.tile_size,"requested_tiles":m.requested_tiles,"total_tiles":m.total_tiles,
        "mask_sha256":hash(&m.mask),"reference_depth_sha256":hash(a),"candidate_depth_sha256":hash(b),"compared_texels":compared,
        "reference_nonclear_texels":occupied,"different_requested_texels":changed,"first_difference":first,
        "caster_clusters_full_demand":m.caster_clusters_full_demand,
        "caster_clusters_tested":m.caster_clusters_tested,"caster_clusters_rejected":m.caster_clusters_rejected}))
}
fn run()->Result<(),String> {
    let args=std::env::args().collect::<Vec<_>>();
    let out=PathBuf::from(&args.windows(2).find(|x|x[0]=="--out").ok_or("use --out FRESH_DIRECTORY")?[1]);
    if out.exists(){return Err("output directory must be fresh".into());}
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    // This standalone process constructs both renderers on the main thread.
    // No production defaults or another process's environment is modified.
    for name in ["KERN_STATIC_SUN_CACHE","KERN_SUN_CACHE_GUARD_METRES","KERN_FAR_SUN_SHADOW","KERN_PAGED_FOLIAGE_SUN",
        "KERN_INSTANCE_GROUPS","KERN_SUN_INSTANCE_GROUPS","KERN_TREE_OCCLUSION","KERN_CLUSTER_TRAVERSAL"] {
        unsafe {std::env::set_var(name,"0");}
    }
    unsafe {std::env::set_var("KERN_DIAGNOSTIC_HIDE","none");std::env::remove_var("KERN_SHADER_DIR");}
    let (scene,blocker)=scene();let sun=Vec3::new(1.,0.22,0.15).normalize();
    let light=Lighting {sun_dir:sun,sun_lux:25000.,sky_lux:1000.,ev100:12.,near_shadows:1.,shadow_strength:1.,
        lod_pixels:0.,gi:0.,contact_occlusion:0.,reflection_occlusion:0.,taa:0.,sharpen:0.,fog_density:0.,fog_start:10000.,fog_end:20000.,..Default::default()};
    let gpu=Gpu::new()?;
    unsafe {std::env::set_var("KERN_RECEIVER_SHADOW_DEMAND","0");}let reference=Renderer::new(&gpu,&scene,&light);
    unsafe {std::env::set_var("KERN_RECEIVER_SHADOW_DEMAND","1");}let candidate=Renderer::new(&gpu,&scene,&light);
    let front=camera(Vec3::new(0.,7.,14.),Vec3::new(0.,0.,-7.));
    let cut=camera(Vec3::new(-20.,8.,-5.),Vec3::new(-10.,0.,-15.));
    let cases=[("first",vec![front],0.),("moving-wind",vec![front],1.9),("camera-cut",vec![cut],2.2),
        ("four-view",vec![front,cut,camera(Vec3::new(25.,6.,12.),Vec3::new(18.,0.,-10.)),camera(Vec3::new(-25.,10.,25.),Vec3::ZERO)],3.1),
        ("full-demand",vec![front],4.2)];
    let mut failures=Vec::new();let mut records=Vec::new();let mut any_reduced=false;let mut any_partial=false;
    for (case_index,(name,cameras,time)) in cases.into_iter().enumerate() {
        // Each fresh renderer starts its render sequence at zero and renders
        // once per case. An independent explicit tag prevents a stale prior
        // case from passing merely because its maps have the expected shape.
        let sequence=case_index as u64;let frame_id=1000+sequence;
        let (views,w,h)=views(&cameras);let focus=cameras.iter().map(|c|c.target).collect::<Vec<_>>();
        let wind=render::wind::WindState {time_seconds:time,strength:0.8,..Default::default()};
        if name=="full-demand" {
            // Identical real geometry in both paths; only receiver precision
            // changes to exercise a valid fully requested map, not fallback.
            let mut wide=*scene.instances.last().unwrap();wide.set_category(GeometryCategory::Other);
            for renderer in [&reference,&candidate] {renderer.update_instances(&gpu,scene.instances.len()-1,&[wide])?;}
        }
        for renderer in [&reference,&candidate] {renderer.set_gpu_profile_frame_id(frame_id);renderer.set_lighting_focus(&focus)?;renderer.set_sun_shadow_focus(&focus)?;renderer.set_wind(&gpu,&wind)?;}
        let (a,sa)=reference.try_render_views(&gpu,&views,&light,w,h,&[])?;
        let ref_passes=reference.debug_sun_culling(&gpu,Some(sequence))?;
        validate_capture_tags(ref_passes.iter().map(|p|(p.frame_id,p.render_sequence)),frame_id,sequence)?;
        let ref_maps=reference.debug_sun_depth_textures(Some(sequence))?.iter().map(|t|read_depth(&gpu,t)).collect::<Result<Vec<_>,_>>()?;
        let (b,sb)=candidate.try_render_views(&gpu,&views,&light,w,h,&[])?;
        let passes=candidate.debug_sun_culling(&gpu,Some(sequence))?;
        validate_capture_tags(passes.iter().map(|p|(p.frame_id,p.render_sequence)),frame_id,sequence)?;
        let demands=candidate.receiver_shadow_demand_diagnostics(&gpu)?;
        let maps=candidate.debug_sun_depth_textures(Some(sequence))?;
        render::write_png(&out.join(format!("{name}-reference.png")),w,h,&a)?;render::write_png(&out.join(format!("{name}-demand.png")),w,h,&b)?;
        if !demands.enabled || demands.requested_generation!=demands.published_generation || demands.requested_generation!=(sequence as u32).wrapping_add(1).max(1) || demands.fallback_reasons!=0 {
            failures.push(format!("{name}: unpublished or fallback demand, {} / {} flags{}",demands.requested_generation,demands.published_generation,demands.fallback_reasons));
        }
        if (name!="full-demand" && (demands.refined_receiver_owners==0 || demands.receiver_cluster_tests==0 || demands.receiver_cluster_bounds==0))
            || demands.coarse_receiver_owners+demands.refined_receiver_owners!=demands.receiver_bounds {
            failures.push(format!("{name}: terrain receiver refinement was not exercised"));
        }
        if maps.len()!=demands.maps.len() || ref_maps.len()!=maps.len() {return Err(format!("{name}: map count mismatch"));}
        let mut depth=Vec::new();
        for (i,m) in demands.maps.iter().enumerate() {
            let pixels=read_depth(&gpu,&maps[i])?;let d=requested_difference(&ref_maps[i],&pixels,m)?;
            if d["different_requested_texels"].as_u64()!=Some(0) {failures.push(format!("{name}: map{i} changed requested depth"));}
            if name=="full-demand" && (m.requested_tiles!=m.total_tiles || m.caster_clusters_full_demand==0
                || m.caster_clusters_tested!=0 || m.caster_clusters_rejected!=0) {
                failures.push(format!("{name}: map{i} did not use only the full-map retain path"));
            }
            any_partial|=m.requested_tiles>0&&m.requested_tiles<m.total_tiles;depth.push(d);
        }
        let reference_triangles=ref_passes.iter().map(|p|u64::from(p.selected_triangles)).sum::<u64>();
        let candidate_triangles=passes.iter().map(|p|u64::from(p.selected_triangles)).sum::<u64>();
        any_reduced|=candidate_triangles<reference_triangles;
        if candidate_triangles>reference_triangles {failures.push(format!("{name}: demand submitted more triangles than reference"));}
        let blocker_reference=ref_passes.iter().any(|p|p.pairs.iter().any(|pair|pair[0]==blocker));
        let blocker_candidate=passes.iter().any(|p|p.pairs.iter().any(|pair|pair[0]==blocker));
        if name=="first"&&(!blocker_reference||!blocker_candidate) {failures.push("offscreen blocker missing from first-frame solar selection".into());}
        let different=a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(a,b)|a!=b).count();
        let max_delta=a.iter().zip(&b).map(|(a,b)|a.abs_diff(*b)).max().unwrap_or(0);
        if different!=0 {failures.push(format!("{name}: final image changed {different} pixels, max{max_delta}"));}
        records.push(json!({"case":name,"frame_id":frame_id,"render_sequence":sequence,"output_size":[w,h],"views":views.len(),"wind_time":time,"wind_strength":wind.strength,
            "cameras":cameras.iter().map(|c|json!({"position":c.pos.to_array(),"target":c.target.to_array()})).collect::<Vec<_>>(),
            "requested_generation":demands.requested_generation,"published_generation":demands.published_generation,"fallback_reasons":demands.fallback_reasons,
            "shared_bytes":demands.shared_bytes,"mask_bytes":demands.mask_bytes,"receiver_owner_tests":demands.receiver_owner_tests,
            "receiver_bounds":demands.receiver_bounds,"extra_bounds":demands.extra_bounds,
            "coarse_receiver_owners":demands.coarse_receiver_owners,"refined_receiver_owners":demands.refined_receiver_owners,
            "receiver_cluster_tests":demands.receiver_cluster_tests,"receiver_cluster_bounds":demands.receiver_cluster_bounds,"maps":depth,
            "reference_triangles":reference_triangles,"candidate_triangles":candidate_triangles,"offscreen_blocker_reference":blocker_reference,
            "offscreen_blocker_candidate":blocker_candidate,"image_different_pixels":different,"image_max_u8_delta":max_delta,
            "reference_gpu_ms":sa.gpu_ms,"candidate_gpu_ms":sb.gpu_ms}));
    }
    if !any_partial {failures.push("fixture produced no partial receiver demand".into());}
    if !any_reduced {failures.push("fixture omitted no actual solar raster triangles".into());}
    let report=json!({"schema":1,"passed":failures.is_empty(),"scope":"native current-pose tiled receivers, real alpha holes/wind/glass/NO_SHADOW receiver; first/cut/four views/full-map retain; requested-tile raw-depth oracle; no performance claim",
        "source_owners":scene.instances.len(),"sun_direction":sun.to_array(),"offscreen_blocker_owner":blocker,"records":records,"failures":failures});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).unwrap()).map_err(|e|e.to_string())?;
    if failures.is_empty() {println!("PASS {}",out.join("report.json").display());Ok(())}else{Err(failures.join("\n"))}
}
fn main(){if let Err(error)=run(){eprintln!("receiver shadow demand: {error}");std::process::exit(1);}}

#[cfg(test)]mod tests {
    use super::*;
    #[test]fn capture_rejects_stale_frame_sequence_or_empty_pass_list() {
        assert!(validate_capture_tags([(1002,2),(1002,2)],1002,2).is_ok());
        assert!(validate_capture_tags([(1001,2)],1002,2).is_err());
        assert!(validate_capture_tags([(1002,1)],1002,2).is_err());
        assert!(validate_capture_tags([],1002,2).is_err());
    }
    #[test]fn fixture_keeps_small_terrain_bounds_and_real_offscreen_cutout() {
        let (scene,blocker)=scene();assert_eq!(scene.instances[blocker as usize].flags&FOLIAGE,FOLIAGE);
        assert!(scene.instances[..441].iter().all(|i|i.sphere[3]<6.));
        assert_eq!(scene.blended_materials.len(),1);assert!(scene.instances.iter().any(|i|i.flags&NO_SHADOW!=0));
        let wide=scene.instances.last().unwrap();assert_eq!(wide.category(),GeometryCategory::Terrain);assert!(wide.sphere[3]>450.);
        assert_eq!(wide.cluster_count,7);assert!(scene.clusters[wide.first_cluster as usize..][..wide.cluster_count as usize].iter().all(|c|c.cull[3]<4.));
        let cam=camera(Vec3::new(0.,7.,14.),Vec3::new(0.,0.,-7.));
        let p=Mat4::perspective_infinite_reverse_rh(cam.fov_y,W as f32/H as f32,cam.near)*Mat4::look_at_rh(cam.pos,cam.target,Vec3::Y);
        let s=scene.instances[blocker as usize].sphere;let q=p.project_point3(Vec3::from_slice(&s[..3]));assert!(q.x>1.);
    }
    #[test]fn requested_depth_oracle_ignores_only_unrequested_tiles() {
        let a=vec![1f32.to_bits();128*128];let mut b=a.clone();b[80]=0;
        let m=render::receiver_shadow_demand::MapDiagnostics {size:128,tile_size:64,tiles_per_axis:2,mask:vec![1,0,0,0],..Default::default()};
        assert_eq!(requested_difference(&a,&b,&m).unwrap()["different_requested_texels"],0);
        b[0]=0;assert_eq!(requested_difference(&a,&b,&m).unwrap()["different_requested_texels"],1);
        let full=render::receiver_shadow_demand::MapDiagnostics {mask:vec![1;4],..m};
        let d=requested_difference(&a,&b,&full).unwrap();assert_eq!(d["compared_texels"],128*128);
        assert_eq!(d["different_requested_texels"],2);
    }
}
