//! GPU acceptance of shared primitive vegetation against expanded source.
//! Explicit invocation only; run without a simultaneous game.
use glam::{Affine3A,Mat3,Mat4,Quat,Vec2,Vec3,Vec4};
use render::{Camera,Gpu,Lighting,Renderer,Scene,ViewRect};
use render::cluster::{ClusterMesh,Part,SimplificationPolicy,Vertex};
use render::foliage_assembly::{self,Assembly,Limits,Parameters};
use render::foliage_voxel::{BakeConfig,ProjectionMeasure};
use render::foliage_representation::{Policy,Representation,View,ViewPlanner};
use render::vegetation_asset::{CookedPlant,SceneVegetation,AttachLimits,PlantInstance};
use render::scene::{GeometryCategory,Heightfield,Material,PART_WIND};
use render::wind::WindState;
use serde_json::{json,Value};
use std::collections::{BTreeMap,BTreeSet};
use std::time::Instant;

const W:u32=800;const H:u32=450;
const WARMUP_FRAMES:usize=3;
const STEADY_FRAMES:usize=12;
fn base()->Scene {
    let mut s=Scene::default();s.sky=Some((16,8,vec![[0.7,0.8,1.,1.];128]));
    let mat=s.add_material(Material{tint:[0.19,0.17,0.12],layer:-1,roughness:1.,flags:0,scale:1.,metal:0.});
    let verts=[[-100.,-0.01,100.],[100.,-0.01,100.],[-100.,-0.01,-100.],[100.,-0.01,-100.]]
        .map(|pos|Vertex{pos,normal:Vec3::Y.to_array(),uv:[0.;2]});
    let mesh=s.add_mesh(&ClusterMesh::build(&[Part{vertices:&verts,indices:&[0,1,2,1,3,2],material:mat}]));
    s.add_instance(mesh,Affine3A::IDENTITY);
    s.heightfield=Heightfield{origin:[-100.,-100.],cell:2.,w:101,h:101,heights:vec![-0.01;10201]};s
}
fn camera(distance:f32)->Camera {Camera{pos:Vec3::new(0.,0.65+distance*0.07,distance),target:Vec3::new(0.,0.35,0.),fov_y:50f32.to_radians(),near:0.05}}
fn read(gpu:&Gpu,buffer:&wgpu::Buffer)->Result<Vec<u8>,String> {
    let dst=gpu.device.create_buffer(&wgpu::BufferDescriptor{label:Some("unified foliage fixture readback"),size:buffer.size(),
        usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
    let mut encoder=gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer,0,&dst,0,buffer.size());gpu.queue.submit([encoder.finish()]);
    let (tx,rx)=std::sync::mpsc::channel();dst.slice(..).map_async(wgpu::MapMode::Read,move|r|{let _=tx.send(r);});gpu.wait();
    rx.recv().map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
    let result=dst.slice(..).get_mapped_range().map_err(|e|e.to_string())?.to_vec();dst.unmap();Ok(result)
}
fn f32at(b:&[u8],i:usize)->f32 {f32::from_le_bytes(b[i..i+4].try_into().unwrap())}
fn matrix(b:&[u8],i:usize)->Mat4 {
    let r:[[f32;4];3]=std::array::from_fn(|y|std::array::from_fn(|x|f32at(b,i+y*16+x*4)));
    Mat4::from_cols(Vec4::new(r[0][0],r[1][0],r[2][0],0.),Vec4::new(r[0][1],r[1][1],r[2][1],0.),
        Vec4::new(r[0][2],r[1][2],r[2][2],0.),Vec4::new(r[0][3],r[1][3],r[2][3],1.))
}
fn error(a:Mat4,b:Mat4)->f32 {
    if !a.is_finite()||!b.is_finite(){return f32::INFINITY;}
    a.to_cols_array().into_iter().zip(b.to_cols_array()).map(|(a,b)|(a-b).abs()).fold(0.,f32::max)
}
fn attachment_json(r:&render::vegetation_asset::AttachmentReport)->Value {
    json!({"plants":r.plants,"primitive_meshes":r.primitive_meshes,"primitive_instances":r.primitive_instances,
        "branch_proxies":r.branch_proxies,"pose_nodes":r.pose_nodes,"volume_instances":r.volume_instances,"volume_assets":r.volume_assets,
        "source_vertices_shared":r.source_vertices_shared,"resident_vertices_shared":r.resident_vertices_shared,"source_triangles_shared":r.source_triangles_shared,
        "shared_geometry_bytes":r.shared_geometry_bytes,"expanded_source_bytes":r.expanded_source_bytes,"packed_volume_bytes":r.packed_volume_bytes})
}
fn diagnostics(gpu:&Gpu,r:&Renderer)->Result<Value,String> {
    let values=r.voxel_diagnostics(gpu)?.ok_or("missing voxel diagnostics")?;
    if values.len()!=4{return Err(format!("expected four voxel diagnostic views, got {}",values.len()));}
    let mut rows=Vec::new();
    for (view,d) in values.into_iter().enumerate(){
        if d.step_overflow_rays>0{return Err(format!("view {view}: voxel integration exceeded step budget"));}
        rows.push(json!({"view":view,"candidate_overflow_tiles":d.candidate_overflow_tiles,"step_overflow_rays":d.step_overflow_rays,
            "traced_rays":d.traced_rays,"max_steps_used":d.max_steps_used,"max_tile_candidates":d.max_tile_candidates,
            "max_ray_candidates":d.max_ray_candidates,"ray_candidate_overflow_rays":d.ray_candidate_overflow_rays,
            "max_active_volumes":d.max_active_volumes,"active_cache_overflow_rays":d.active_cache_overflow_rays,
            "max_empty_brick_skips":d.max_empty_brick_skips,"max_occupied_lod_x256":d.max_occupied_lod_x256,
            "occupied_coarse_mip_rays":d.occupied_coarse_mip_rays,"active_tiles":d.active_tiles,
            "scheduled_pixels":d.scheduled_pixels,"total_steps":d.total_steps,"volume_samples":d.volume_samples,
            "occupied_cell_samples":d.occupied_cell_samples,"aabb_interval_tests":d.aabb_interval_tests}));
    }
    Ok(json!(rows))
}
fn gpu_passes(stats:&render::FrameStats)->Result<BTreeMap<String,f64>,String> {
    let mut result=BTreeMap::new();
    for &(name,ms) in &stats.gpu_ms {
        if !ms.is_finite()||ms<0.{return Err(format!("invalid GPU timestamp for {name}: {ms}"));}
        *result.entry(name.to_owned()).or_insert(0.)+=ms;
    }
    Ok(result)
}
fn median(values:&[f64])->f64 {
    let mut sorted=values.to_vec();sorted.sort_by(f64::total_cmp);
    let middle=sorted.len()/2;
    if sorted.len()%2==0{(sorted[middle-1]+sorted[middle])*0.5}else{sorted[middle]}
}
fn timing_summary(samples:&[BTreeMap<String,f64>])->Value {
    let names:BTreeSet<_>=samples.iter().flat_map(|s|s.keys().cloned()).collect();
    let mut passes=serde_json::Map::new();
    for name in names {
        let values:Vec<_>=samples.iter().map(|s|s.get(&name).copied().unwrap_or(0.)).collect();
        passes.insert(name.clone(),json!({"median_ms":median(&values),"min_ms":values.iter().copied().fold(f64::INFINITY,f64::min),
            "max_ms":values.iter().copied().fold(0.,f64::max),"samples_present":samples.iter().filter(|s|s.contains_key(&name)).count()}));
    }
    let totals:Vec<f64>=samples.iter().map(|s|s.values().sum()).collect();
    json!({"samples":samples.len(),"timestamps_available":samples.iter().any(|s|!s.is_empty()),"passes":passes,
        "timed_pass_sum_median_ms":median(&totals),"timed_pass_sum_min_ms":totals.iter().copied().fold(f64::INFINITY,f64::min),
        "timed_pass_sum_max_ms":totals.iter().copied().fold(0.,f64::max),
        "absent_pass_policy":"Absent cached passes count as zero; samples_present distinguishes absence from a measured zero. Timed pass sums exclude uninstrumented work."})
}
fn timing_sample(stats:&render::FrameStats,wall_ms:f64,passes:&BTreeMap<String,f64>)->Value {
    json!({"gpu_pass_ms":passes,"timed_pass_sum_ms":passes.values().sum::<f64>(),"synchronous_render_wall_ms":wall_ms,
        "clusters":stats.clusters,"triangles":stats.triangles})
}
fn poses(gpu:&Gpu,r:&Renderer,s:&Scene,wind:&WindState,previous:Option<&[Mat4]>)->Result<(Value,Vec<Mat4>),String> {
    let (p,instances,volumes)=r.part_wind_debug_buffers().ok_or("missing shared poses")?;
    let p=read(gpu,p)?;let data=read(gpu,instances)?;
    let expected=render::part_wind::sample_poses(&s.part_wind,wind)?;
    let prior=previous.unwrap_or(&expected);
    let mut pose_error=0f32;let mut previous_error=0f32;let mut attachment_error=0f32;let mut sphere_margin=f32::INFINITY;
    for (i,m) in expected.iter().enumerate(){pose_error=pose_error.max(error(matrix(&p,i*96),*m));previous_error=previous_error.max(error(matrix(&p,i*96+48),prior[i]));}
    for a in &s.part_attachments {
        let expected=expected[a.pose as usize]*a.rest;let at=a.owner as usize*80;
        attachment_error=attachment_error.max(error(matrix(&data,at),expected));
        let center=Vec3::new(f32at(&data,at+48),f32at(&data,at+52),f32at(&data,at+56));let radius=f32at(&data,at+60);
        if !center.is_finite()||!radius.is_finite()||radius<0.{return Err("invalid animated primitive sphere".into());}
        let instance=&s.instances[a.owner as usize];
        for c in &s.clusters[instance.first_cluster as usize..(instance.first_cluster+instance.cluster_count)as usize] {
            // All actual cluster vertices, including simplified levels, must
            // remain inside the animated primitive's culling sphere.
            for &tri in &s.tris[c.first_tri as usize..(c.first_tri+c.tri_count)as usize] {
                for local in [tri&255,(tri>>8)&255,(tri>>16)&255] {
                    let vi=s.cluster_verts[(c.first_vert+local)as usize];
                    let point=Vec3::from(s.vertices[vi as usize].pos);
                    sphere_margin=sphere_margin.min(radius-expected.transform_point3(point).distance(center));
                }
            }
        }
        // Independent dense directional sphere check, not the implementation's norm formula.
        let rest=Mat4::from_cols(Vec4::new(instance.rows[0][0],instance.rows[1][0],instance.rows[2][0],0.),
            Vec4::new(instance.rows[0][1],instance.rows[1][1],instance.rows[2][1],0.),Vec4::new(instance.rows[0][2],instance.rows[1][2],instance.rows[2][2],0.),
            Vec4::new(instance.rows[0][3],instance.rows[1][3],instance.rows[2][3],1.));
        let local=rest.inverse().transform_point3(Vec3::from_slice(&instance.sphere[..3]));
        let local_radius=instance.sphere[3]/instance.scale;
        for j in 0..96 {let y=1.-2.*(j as f32+0.5)/96.;let angle=j as f32*2.3999631;
            let v=Vec3::new((1.-y*y).sqrt()*angle.cos(),y,(1.-y*y).sqrt()*angle.sin());
            sphere_margin=sphere_margin.min(radius-expected.transform_point3(local+v*local_radius).distance(center));}
        let normal=Mat3::from_mat4(expected).inverse().transpose()*Vec3::new(0.3,0.4,0.5).normalize();
        if !normal.is_finite(){return Err("nonfinite animated inverse-transpose normal".into());}
    }
    let mut volume_inverse_error=0f32;let mut volume_bounds_error=0f32;let mut volume_count=0;
    if s.part_wind.iter().any(|part|part.volume.is_some()) {
        let data=read(gpu,volumes.ok_or("missing animated volume buffer")?)?;
        let forest=s.voxel_forest.as_ref().ok_or("missing source volume forest")?;
        for (pose,part) in s.part_wind.iter().enumerate(){let Some(index)=part.volume else{continue;};
            let volume=&forest.instances[index as usize];let at=index as usize*96;
            volume_inverse_error=volume_inverse_error.max(error(matrix(&data,at),expected[pose].inverse()));
            let mut local_min=Vec3::splat(f32::INFINITY);let mut local_max=Vec3::splat(f32::NEG_INFINITY);
            // Include the filter support of every mip. Derive it from the
            // immutable cooked levels, independently of GPU pose descriptors.
            for level in &forest.levels[volume.meta[1]as usize..][..volume.meta[2]as usize]{
                let origin=Vec3::from_slice(&level.origin_cell[..3]);let cell=level.origin_cell[3];
                let dims=Vec3::new(level.dims_offset[0]as f32,level.dims_offset[1]as f32,level.dims_offset[2]as f32);
                local_min=local_min.min(origin-Vec3::splat(cell*0.5));local_max=local_max.max(origin+(dims+Vec3::splat(0.5))*cell);
            }
            let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
            for x in [local_min.x,local_max.x]{for y in [local_min.y,local_max.y]{for z in [local_min.z,local_max.z]{
                let p=expected[pose].transform_point3(Vec3::new(x,y,z));lo=lo.min(p);hi=hi.max(p);
            }}}
            let actual_lo=Vec3::new(f32at(&data,at+48),f32at(&data,at+52),f32at(&data,at+56));
            let actual_hi=Vec3::new(f32at(&data,at+64),f32at(&data,at+68),f32at(&data,at+72));
            if !lo.is_finite()||!hi.is_finite()||!actual_lo.is_finite()||!actual_hi.is_finite(){return Err("nonfinite animated volume bounds".into());}
            volume_bounds_error=volume_bounds_error.max((actual_lo-lo).abs().max_element()).max((actual_hi-hi).abs().max_element());
            for (i,&meta) in volume.meta.iter().enumerate(){if u32::from_le_bytes(data[at+80+i*4..at+84+i*4].try_into().unwrap())!=meta{return Err("animated volume metadata changed".into());}}
            volume_count+=1;
        }
    }
    let passed=pose_error<3e-4&&previous_error<3e-4&&attachment_error<3e-4&&sphere_margin>=-3e-4&&volume_inverse_error<3e-4&&volume_bounds_error<3e-4;
    if !passed{return Err(format!("GPU pose/attachment/volume mismatch: {pose_error} {previous_error} {attachment_error} {sphere_margin} {volume_inverse_error} {volume_bounds_error}"));}
    Ok((json!({"passed":true,"pose_error":pose_error,"previous_error":previous_error,"attachment_error":attachment_error,"sphere_margin":sphere_margin,
        "volume_count":volume_count,"volume_inverse_error":volume_inverse_error,"volume_bounds_error":volume_bounds_error}),expected))
}

fn run()->Result<(),String>{
    let args:Vec<_>=std::env::args().collect();let out=std::path::PathBuf::from(args.windows(2).find(|a|a[0]=="--out").ok_or("--out FRESH_DIR required")?[1].clone());
    if out.exists(){return Err("output exists".into());}std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let sources:Vec<Assembly>=[Parameters::grass(101),Parameters::conifer_twig(909)].into_iter().map(|p|foliage_assembly::generate(p,Limits::default())).collect::<Result<_,_>>()?;
    let config=BakeConfig{projection_measure:ProjectionMeasure::ClosedSurfaceEntries,cell_size:0.025,levels:4,samples_per_cell:4,max_samples:4_000_000,max_voxels:131072,max_texture_texels:0,..Default::default()};
    let cooked:Vec<_>=sources.iter().map(|s|CookedPlant::cook(s,config)).collect::<Result<_,_>>()?;
    let placements:Vec<_>=(0..6).map(|i|((i%2)as usize,Mat4::from_scale_rotation_translation(
        Vec3::new(0.85+i as f32*0.04,1.05,0.91),Quat::from_rotation_y(i as f32*0.71),Vec3::new((i%3)as f32*0.8-0.8,0.,(i/3)as f32*0.8-0.4)))).collect();
    let mut shared=base();let mut builder=SceneVegetation::new(&mut shared,AttachLimits::default())?;let mut handles=Vec::new();
    for (i,(asset,root)) in placements.iter().enumerate(){handles.push(builder.append(&cooked[*asset],PlantInstance{id:i as u64,generation:1,root:*root,category:if *asset==0{GeometryCategory::Grass}else{GeometryCategory::Trees}})?);}
    let attachment_report=builder.finish(Policy::default())?;
    let mut oracle=base();let mut oracle_owners=Vec::new();
    for (plant,(asset,_)) in placements.iter().enumerate(){let source=&sources[*asset];let base=oracle.materials.len()as u32;
        for &m in &source.materials{oracle.add_material(m);}
        for branch in 0..source.branches.len(){let meshes=source.flatten_branch(branch as u32)?;if meshes.is_empty(){continue;}
            let parts:Vec<_>=meshes.iter().map(|m|Part{vertices:&m.vertices,indices:&m.indices,material:base+m.material}).collect();
            let mesh=oracle.add_mesh(&ClusterMesh::build_with_policy(&parts,|_|SimplificationPolicy::TexturedGeometry));
            let owner=oracle.instances.len();oracle.add_instance(mesh,Affine3A::IDENTITY);oracle.instances[owner].flags|=PART_WIND;
            let pose=handles[plant].pose_range.start+branch;
            if !handles[plant].pose_range.contains(&pose){return Err("oracle branch outside its plant pose range".into());}
            oracle_owners.push((owner,mesh,pose));}}
    let light=Lighting{taa:0.,sharpen:0.,lod_pixels:0.,gi:0.,contact_occlusion:0.,reflection_occlusion:0.,near_shadows:0.,
        fog_density:0.,fog_start:10000.,fog_end:20000.,sun_dir:Vec3::new(0.4,0.65,-0.6).normalize(),..Default::default()};
    let gpu=Gpu::new()?;let renderer=Renderer::new(&gpu,&shared,&light);let mut rows=Vec::new();let mut previous=None;
    for (index,time) in [0.,0.7,1.9].into_iter().enumerate(){
        let wind=WindState{time_seconds:time,strength:if index==0{0.}else{0.8},direction:Vec2::new(0.8,0.6),..Default::default()};
        let cpu=render::part_wind::sample_poses(&shared.part_wind,&wind)?;
        for &(owner,mesh,pose) in &oracle_owners{oracle.update_instance(owner,mesh,Affine3A::from_mat4(cpu[pose]));}
        let reference=Renderer::new(&gpu,&oracle,&light);renderer.set_wind(&gpu,&wind)?;
        let (actual,stats)=renderer.render(&gpu,&camera(3.8),&light,W,H);let (expected,_)=reference.render(&gpu,&camera(3.8),&light,W,H);
        let (pose_report,now)=poses(&gpu,&renderer,&shared,&wind,previous.as_deref())?;previous=Some(now);
        let mean=actual.iter().zip(&expected).map(|(a,b)|a.abs_diff(*b)as f64).sum::<f64>()/actual.len()as f64;
        let large=actual.chunks_exact(4).zip(expected.chunks_exact(4)).filter(|(a,b)|a[..3].iter().zip(&b[..3]).any(|(a,b)|a.abs_diff(*b)>12)).count()as f64/(W*H)as f64;
        if mean>1.5||large>0.0075{return Err(format!("shared/expanded image mismatch {mean} / {large}"));}
        render::write_png(&out.join(format!("shared-{index}.png")),W,H,&actual)?;render::write_png(&out.join(format!("expanded-{index}.png")),W,H,&expected)?;
        rows.push(json!({"time":time,"source_image_mean_u8_error":mean,"large_pixel_fraction":large,"poses":pose_report,"triangles":stats.triangles,"gpu_ms":stats.gpu_ms}));
    }
    // Same immutable scene, one shared pose pass, four independent views/LODs.
    shared.foliage_representation_policy=Policy{allow_unvalidated_optics:true,..Default::default()};
    let runtime=Renderer::new(&gpu,&shared,&light);let distances=[3.8,45.,120.,240.];let mut expected_selection=Vec::new();
    let views:Vec<_>=distances.iter().enumerate().map(|(i,&d)|ViewRect{cam:camera(d),x:(i as u32%2)*W,y:(i as u32/2)*H,w:W,h:H}).collect();
    for v in &views{let plans=ViewPlanner::default().plan(&shared.foliage_representations,View{view_projection:Mat4::perspective_infinite_reverse_rh(v.cam.fov_y,W as f32/H as f32,v.cam.near)*Mat4::look_at_rh(v.cam.pos,v.cam.target,Vec3::Y),width:W,height:H},shared.foliage_representation_policy)?;
        expected_selection.push(plans.iter().filter(|p|matches!(p.representation,Representation::Voxel{..})).count());}
    if expected_selection[0]!=0||expected_selection[3]==0{return Err("fixture did not cover source and voxel representations".into());}
    let wind=WindState{strength:0.8,time_seconds:1.9,direction:Vec2::new(0.8,0.6),..Default::default()};runtime.set_wind(&gpu,&wind)?;
    let start=Instant::now();let (pixels,stats)=runtime.try_render_views(&gpu,&views,&light,W*2,H*2,&[])?;let first_wall_ms=start.elapsed().as_secs_f64()*1000.;
    let first_passes=gpu_passes(&stats)?;
    let (four_pose,four_expected)=poses(&gpu,&runtime,&shared,&wind,None)?;
    let mut selected=Vec::new();for i in 0..4{let capture=runtime.debug_visible_clusters(&gpu,i,None)?;
        let owners:std::collections::BTreeSet<_>=capture.pairs.iter().map(|p|p[0]).collect();
        let plans=ViewPlanner::default().plan(&shared.foliage_representations,View{view_projection:Mat4::perspective_infinite_reverse_rh(views[i].cam.fov_y,W as f32/H as f32,views[i].cam.near)*Mat4::look_at_rh(views[i].cam.pos,views[i].cam.target,Vec3::Y),width:W,height:H},shared.foliage_representation_policy)?;
        for plan in plans{if matches!(plan.representation,Representation::Voxel{..}){let group=shared.foliage_source_groups.iter().find(|g|g.owner==plan.owner).unwrap();if group.members.iter().any(|m|owners.contains(m)){return Err("voxel-selected branch still rasterized its primitives".into());}}}
        selected.push(capture.pairs.len());}
    let first_diagnostics=diagnostics(&gpu,&runtime)?;
    render::write_png(&out.join("four-views.png"),W*2,H*2,&pixels)?;
    // The expanded source is fixed to the identical branch poses and views.
    // Its far geometry remains source, so this is a workload reference, not
    // voxel optical equivalence or an isolated GPU scatter speedup claim.
    for &(owner,mesh,pose) in &oracle_owners{oracle.update_instance(owner,mesh,Affine3A::from_mat4(four_expected[pose]));}
    let reference=Renderer::new(&gpu,&oracle,&light);
    let start=Instant::now();let (source_pixels,source_first)=reference.try_render_views(&gpu,&views,&light,W*2,H*2,&[])?;
    let source_first_wall_ms=start.elapsed().as_secs_f64()*1000.;let source_first_passes=gpu_passes(&source_first)?;
    render::write_png(&out.join("four-views-expanded-source.png"),W*2,H*2,&source_pixels)?;
    let mut warmup=Vec::new();let mut steady=Vec::new();let mut runtime_samples=Vec::new();let mut reference_samples=Vec::new();
    for sample in 0..WARMUP_FRAMES+STEADY_FRAMES {
        let mut pair=serde_json::Map::new();
        // Alternate order, keeping the exact same wind/time and all four
        // camera poses. No debug-buffer mapping enters either timed render.
        let order=if sample%2==0{[true,false]}else{[false,true]};
        for use_runtime in order {
            let r=if use_runtime{&runtime}else{&reference};let start=Instant::now();
            let (_,sample_stats)=r.try_render_views(&gpu,&views,&light,W*2,H*2,&[])?;
            let wall_ms=start.elapsed().as_secs_f64()*1000.;let passes=gpu_passes(&sample_stats)?;
            let name=if use_runtime{"shared_source_and_voxel"}else{"expanded_source_only"};
            pair.insert(name.into(),timing_sample(&sample_stats,wall_ms,&passes));
            if sample>=WARMUP_FRAMES{if use_runtime{runtime_samples.push(passes);}else{reference_samples.push(passes);}}
        }
        let value=json!({"sample":sample,"order":if sample%2==0{"shared_then_expanded"}else{"expanded_then_shared"},"renders":pair});
        if sample<WARMUP_FRAMES{warmup.push(value);}else{steady.push(value);}
    }
    let steady_diagnostics=diagnostics(&gpu,&runtime)?;
    let (steady_pose,_)=poses(&gpu,&runtime,&shared,&wind,Some(&four_expected))?;
    // Reuse the populated per-view targets: a camera cut must clear all old
    // volume radiance before every view switches back to source geometry.
    let close_views:Vec<_>=(0..4).map(|i|ViewRect{cam:camera(3.8),x:(i%2)*W,y:(i/2)*H,w:W,h:H}).collect();
    for view in &close_views {
        let plans=ViewPlanner::default().plan(&shared.foliage_representations,View{
            view_projection:Mat4::perspective_infinite_reverse_rh(view.cam.fov_y,W as f32/H as f32,view.cam.near)
                *Mat4::look_at_rh(view.cam.pos,view.cam.target,Vec3::Y),width:W,height:H},shared.foliage_representation_policy)?;
        if plans.iter().any(|p|matches!(p.representation,Representation::Voxel{..})){return Err("camera cut did not select source-only views".into());}
    }
    let (cut_pixels,cut_stats)=runtime.try_render_views(&gpu,&close_views,&light,W*2,H*2,&[])?;
    let (cut_reference,_)=reference.try_render_views(&gpu,&close_views,&light,W*2,H*2,&[])?;
    render::write_png(&out.join("camera-cut-source.png"),W*2,H*2,&cut_pixels)?;
    render::write_png(&out.join("camera-cut-expanded-source.png"),W*2,H*2,&cut_reference)?;
    if cut_pixels.len()!=cut_reference.len()||cut_pixels.len()!=(W*2*H*2*4)as usize{return Err("camera cut returned incorrect image dimensions".into());}
    let mut cut_images=Vec::new();
    for (view,rect) in close_views.iter().enumerate(){
        let mut total=0u64;let mut large=0u64;
        for y in rect.y..rect.y+rect.h{for x in rect.x..rect.x+rect.w{
            let offset=((y*W*2+x)*4)as usize;let a=&cut_pixels[offset..offset+4];let b=&cut_reference[offset..offset+4];
            total+=a.iter().zip(b).map(|(a,b)|u64::from(a.abs_diff(*b))).sum::<u64>();
            large+=u64::from(a[..3].iter().zip(&b[..3]).any(|(a,b)|a.abs_diff(*b)>12));
        }}
        let mean=total as f64/(W*H*4)as f64;let large=large as f64/(W*H)as f64;
        if mean>1.5||large>0.0075{return Err(format!("camera cut view {view}: source image mismatch {mean} / {large}"));}
        cut_images.push(json!({"view":view,"source_image_mean_u8_error":mean,"large_pixel_fraction":large}));
    }
    let cut_diagnostics=diagnostics(&gpu,&runtime)?;
    for d in cut_diagnostics.as_array().ok_or("invalid camera cut diagnostics")? {
        if d["traced_rays"].as_u64()!=Some(0){return Err(format!("source-only camera cut still traced volume rays: {d}"));}
    }
    let layers=runtime.voxel_layers_rgba16(&gpu)?;
    if layers.len()!=4{return Err("camera cut did not retain all four volume targets".into());}
    let mut cut_layers=Vec::new();
    for (view,(w,h,bytes)) in layers.iter().enumerate(){
        if *w!=W||*h!=H||bytes.len()!=(W*H*8)as usize{return Err(format!("camera cut view {view}: incorrect raw volume layer dimensions"));}
        let nonzero=bytes.chunks_exact(2).filter(|b|u16::from_le_bytes([b[0],b[1]])&0x7fff!=0).count();
        if nonzero!=0{return Err(format!("camera cut view {view}: {nonzero} stale/nonzero volume half components"));}
        cut_layers.push(json!({"view":view,"width":w,"height":h,"nonzero_half_components":nonzero}));
    }
    let (cut_pose,_)=poses(&gpu,&runtime,&shared,&wind,Some(&four_expected))?;
    let report=json!({"passed":true,"adapter":gpu.info.name,"source_checks":rows,"four_view_pose":four_pose,"four_view_voxel_groups":expected_selection,
        "four_view_clusters":selected,"four_view_gpu_ms":stats.gpu_ms,"attachment_report":attachment_json(&attachment_report),
        "four_view_first_frame":{"shared_source_and_voxel":timing_sample(&stats,first_wall_ms,&first_passes),
            "expanded_source_only":timing_sample(&source_first,source_first_wall_ms,&source_first_passes),"voxel_diagnostics":first_diagnostics},
        "four_view_steady":{"warmup_frames_per_renderer":WARMUP_FRAMES,"measured_frames_per_renderer":STEADY_FRAMES,
            "wind_time_seconds":wind.time_seconds,"wind_strength":wind.strength,"view_distances_m":distances,
            "warmup":warmup,"samples":steady,"shared_source_and_voxel":timing_summary(&runtime_samples),
            "expanded_source_only":timing_summary(&reference_samples),"poses":steady_pose,"voxel_diagnostics":steady_diagnostics,
            "scope":"Fixed-pose synchronous four-view diagnostic. Expanded source is CPU-posed once, has no branch compute or voxel integration, and retains source geometry in far views. This is not an isolated optimization comparison or gameplay FPS."},
        "camera_cut_source_only":{"passed":true,"distance_m":3.8,"source_images":cut_images,"poses":cut_pose,
            "voxel_diagnostics":cut_diagnostics,"volume_layers":cut_layers,"gpu_ms":cut_stats.gpu_ms,
            "scope":"First frame after moving all populated views close: source-image equivalence per view, no traced volume rays, and completely clear raw volume layers."},
        "shared_mesh_vertices":shared.vertices.len(),"expanded_mesh_vertices":oracle.vertices.len(),"primitive_instances":shared.part_attachments.len(),"branch_poses":shared.part_wind.len(),
        "scope":"Small native GPU fixture; image comparison tests shared primitive source, not voxel optical equivalence. Synchronous diagnostic timings are not gameplay FPS. DLSS is validated separately by vegetation_temporal_gpu, not by this native fixture."});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).unwrap()).map_err(|e|e.to_string())?;println!("{report}");Ok(())
}
fn main(){if let Err(e)=run(){eprintln!("unified_vegetation_gpu: {e}");std::process::exit(1);}}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summaries_aggregate_repeated_view_passes_and_count_cached_absence() {
        let mut stats=render::FrameStats::default();stats.gpu_ms=vec![("integrate",1.),("integrate",2.),("shadow",4.)];
        let first=gpu_passes(&stats).unwrap();assert_eq!(first["integrate"],3.);
        stats.gpu_ms=vec![("integrate",5.)];let second=gpu_passes(&stats).unwrap();
        let summary=timing_summary(&[first,second]);
        assert_eq!(summary["passes"]["integrate"]["median_ms"],4.);
        assert_eq!(summary["passes"]["shadow"]["median_ms"],2.);
        assert_eq!(summary["passes"]["shadow"]["samples_present"],1);
        assert_eq!(summary["timed_pass_sum_median_ms"],6.);
    }
    #[test]
    fn nonfinite_measurements_cannot_silently_pass() {
        let mut stats=render::FrameStats::default();stats.gpu_ms=vec![("integrate",f64::NAN)];assert!(gpu_passes(&stats).is_err());
        let mut matrix=Mat4::IDENTITY;matrix.x_axis.x=f32::NAN;assert!(error(matrix,Mat4::IDENTITY).is_infinite());
    }
}
