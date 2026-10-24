//! Explicit, bounded native GPU proof of shared procedural foliage and poses.
//! No game window, live-world replacement, frame generation or DLSS fallback.
//! cargo run --release -p cli --example procedural_foliage_gpu -- --out FRESH_DIR
use glam::{Affine3A, Mat4, Quat, Vec2, Vec3, Vec4};
use render::{Camera, Gpu, Lighting, Renderer, Scene};
use render::cluster::{ClusterMesh, Part, SimplificationPolicy, Vertex};
use render::foliage_assembly::{self, Assembly, BakedBranch, Limits, Parameters};
use render::foliage_volume_data::{self, ALL_MATERIALS, PackLimits, VolumeBinding};
use render::foliage_voxel::{BakeConfig, ProjectionMeasure, VoxelAsset};
use render::foliage_representation::{self,Group,GroupKey,OpticalEvidence,Policy,Representation,View};
use render::part_wind::{PartWind, sample_poses};
use render::scene::{Heightfield, Material, PART_WIND};
use render::wind::WindState;
use serde_json::{Value, json};
use std::{collections::BTreeMap,path::PathBuf, time::Instant};

const WIDTH:u32=960;
const HEIGHT:u32=540;

struct Asset {source:Assembly,branches:Vec<BakedBranch>}
struct Fixture {scene:Scene,identities:Vec<Value>,assets:usize,plant_count:usize,selection:Vec<Value>}
#[derive(Clone,Copy)]
enum Mode {Source,Volume,Automatic(Camera),Runtime}

fn assets()->Result<Vec<Asset>,String> {
    [Parameters::grass(101),Parameters::grass(303),Parameters::grass(707),Parameters::conifer_twig(909)]
        .into_iter().map(|parameters| {
            let source=foliage_assembly::generate(parameters,Limits::default())?;
            let branches=source.bake_branches(BakeConfig {projection_measure:ProjectionMeasure::ClosedSurfaceEntries,
                cell_size:0.025,levels:4,
                samples_per_cell:4,max_samples:4_000_000,max_voxels:131_072,
                max_texture_texels:0,..Default::default()})?;
            Ok(Asset {source,branches})
        }).collect()
}

fn placements()->Vec<(usize,Mat4)> {
    let mut result=Vec::new();
    for row in 0..3 {for col in 0..4 {
        let i=row*4+col;
        // Deliberate nonuniform root scale exercises normal transforms and
        // conservative bounds after child rotation. Pivots themselves rigid.
        let scale=Vec3::new(0.9+(i%3) as f32*0.07,0.85+(i%4) as f32*0.08,1.02);
        let root=Mat4::from_scale_rotation_translation(scale,Quat::from_rotation_y(i as f32*0.71),
            Vec3::new((col as f32-1.5)*0.65,0.0,(row as f32-1.0)*0.65));
        result.push((i%3,root));
    }}
    for sign in [-1.0,1.0] {result.push((3,Mat4::from_rotation_translation(
        Quat::from_rotation_y(sign*0.4),Vec3::new(sign*1.65,0.0,-0.25))));}
    result
}

fn build(assets:&[Asset],voxel:bool)->Result<Fixture,String> {
    build_mode(assets,if voxel{Mode::Volume}else{Mode::Source})
}

fn build_mode(assets:&[Asset],mode:Mode)->Result<Fixture,String> {
    let mut scene=Scene::default();
    let sky=PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap().join("assets/sky/sky_2k.hdr");
    scene.load_sky(&sky)?;
    let mut meshes=Vec::new();let mut volumes:Vec<VoxelAsset>=Vec::new();
    for asset in assets {
        let material_base=scene.materials.len() as u32;
        for &material in &asset.source.materials {scene.add_material(material);}
        let mut branch_meshes=Vec::new();
        for baked in &asset.branches {
            let sources=asset.source.flatten_branch(baked.branch)?;
            let parts:Vec<_>=sources.iter().map(|source|Part {vertices:&source.vertices,
                indices:&source.indices,material:material_base+source.material}).collect();
            let mesh=ClusterMesh::build_with_policy(&parts,|_|SimplificationPolicy::TexturedGeometry);
            let id=scene.add_mesh(&mesh);let volume=volumes.len() as u32;
            volumes.push(baked.asset.clone());branch_meshes.push((baked.branch,id,volume));
        }
        meshes.push(branch_meshes);
    }
    let mut bindings=Vec::new();let mut identities=Vec::new();let mut selection=Vec::new();
    let placements=placements();
    let decisions=if let Mode::Automatic(camera)=mode {
        let mut groups=Vec::new();
        for (plant,(variant,root)) in placements.iter().enumerate() {
            for branch in &assets[*variant].branches {
                groups.push(Group::from_voxels(GroupKey {instance:plant as u64,branch:branch.branch_id,generation:0},
                    &branch.asset,*root*branch.rest_to_assembly,0.75,OpticalEvidence::Unvalidated)?);
            }
        }
        let view=View {view_projection:Mat4::perspective_infinite_reverse_rh(camera.fov_y,WIDTH as f32/HEIGHT as f32,camera.near)
            *Mat4::look_at_rh(camera.pos,camera.target,Vec3::Y),width:WIDTH,height:HEIGHT};
        foliage_representation::select(&groups,view,Policy {allow_unvalidated_optics:true,..Default::default()},&[])?
            .into_iter().map(|d| {
                selection.push(json!({"plant":d.key.instance,"branch_id":d.key.branch.to_string(),
                    "representation":format!("{:?}",d.representation),"projected_error_pixels":d.projected_error_pixels,
                    "optics_validated":d.optics_validated}));
                (d.key,d.representation)
            }).collect::<BTreeMap<_,_>>()
    }else{BTreeMap::new()};
    let mut packed_assets=Vec::new();let mut packed_keys=BTreeMap::new();
    for (plant,(variant,root)) in placements.iter().enumerate() {
        let asset=&assets[*variant];let base=scene.part_wind.len() as u32;
        let rest=asset.source.branch_transforms()?;
        for (index,branch) in asset.source.branches.iter().enumerate() {
            let attached=meshes[*variant].iter().find(|(b,_,_)|*b==index as u32);
            let (owner,volume)=if let Some(&(_,mesh,volume_asset))=attached {
                let owner=scene.instances.len() as u32;
                scene.add_instance(mesh,Affine3A::from_mat4(*root*rest[index]));
                scene.instances.last_mut().unwrap().flags|=PART_WIND;
                let selected=match mode {
                    Mode::Source=>None,
                    Mode::Volume|Mode::Runtime=>Some(None),
                    Mode::Automatic(_)=>match decisions[&GroupKey {instance:plant as u64,branch:branch.id,generation:0}] {
                        Representation::Source=>None,Representation::Voxel {level}=>Some(Some(level)),
                    },
                };
                let volume=if let Some(level)=selected {
                    let key=(volume_asset,level);
                    let packed_asset=if let Some(&index)=packed_keys.get(&key) {index}else{
                        let mut asset=volumes[volume_asset as usize].clone();
                        if let Some(level)=level {asset.levels=vec![asset.levels[level as usize].clone()];}
                        let index=packed_assets.len() as u32;packed_assets.push(asset);packed_keys.insert(key,index);index
                    };
                    let volume=bindings.len() as u32;
                    bindings.push(VolumeBinding {owner,asset:packed_asset,crown_material:ALL_MATERIALS});
                    if matches!(mode,Mode::Runtime) {
                        scene.foliage_representations.push(foliage_representation::Binding {owner,volume,
                            group:Group::from_voxels(GroupKey {instance:plant as u64,branch:branch.id,generation:0},
                                &volumes[volume_asset as usize],*root*rest[index],0.75,OpticalEvidence::Unvalidated)?});
                    }
                    Some(volume)
                }else{None};
                (Some(owner),volume)
            }else{(None,None)};
            let phase=((branch.id>>40) as f32/16_777_216.0)*std::f32::consts::TAU;
            scene.part_wind.push(PartWind {parent:branch.parent.map(|p|base+p),owner,volume,
                rest:if branch.parent.is_none(){*root*branch.rest}else{branch.rest},
                max_angle:if branch.parent.is_none(){0.0}else{0.22*(1.0-branch.stiffness).clamp(0.15,1.0)},
                harmonic:7+(branch.id%13) as u32,phase});
            identities.push(json!({"plant":plant,"variant":variant,"branch_index":index,
                "branch_id":branch.id.to_string(),"pose":base+index as u32,"owner":owner,"volume":volume}));
        }
    }
    if scene.part_wind.len()>128 || bindings.len()>=64 {return Err("fixture exceeds its explicit node/overlap bound".into());}
    if !bindings.is_empty() {
        scene.voxel_forest=Some(foliage_volume_data::pack(&packed_assets,&bindings,&scene.instances,
            PackLimits {max_assets:64,levels_per_asset:4,first_level_bounds:false,
                max_dense_indices:8_388_608,max_sparse_cells:1_048_576,..Default::default()})?);
        scene.voxel_lod_pixels=if matches!(mode,Mode::Automatic(_)){0.0}else{1.25};
    }
    if matches!(mode,Mode::Runtime) {
        scene.foliage_representation_policy=Policy {allow_unvalidated_optics:true,..Default::default()};
        foliage_representation::validate_bindings(&scene)?;
    }
    let ground=scene.add_material(Material {tint:[0.11,0.12,0.08],layer:-1,roughness:0.9,flags:0,scale:1.0,metal:0.0});
    let vertices=[[-80.0,-0.01,-80.0],[-80.0,-0.01,80.0],[80.0,-0.01,-80.0],[80.0,-0.01,80.0]]
        .map(|pos|Vertex {pos,normal:Vec3::Y.to_array(),uv:[0.0;2]});
    let mesh=scene.add_mesh(&ClusterMesh::build(&[Part {vertices:&vertices,indices:&[0,1,2,2,1,3],material:ground}]));
    scene.add_instance(mesh,Affine3A::IDENTITY);
    scene.heightfield=Heightfield {origin:[-80.0,-80.0],cell:2.0,w:81,h:81,heights:vec![-0.01;81*81]};
    Ok(Fixture {scene,identities,assets:volumes.len(),plant_count:placements.len(),selection})
}

fn read_buffer(gpu:&Gpu,source:&wgpu::Buffer)->Result<Vec<u8>,String> {
    let target=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("procedural fixture readback"),
        size:source.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    let mut encoder=gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source,0,&target,0,source.size());gpu.queue.submit([encoder.finish()]);
    let (tx,rx)=std::sync::mpsc::channel();
    target.slice(..).map_async(wgpu::MapMode::Read,move |r|{let _=tx.send(r);});gpu.wait();
    rx.recv().map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
    let data=target.slice(..).get_mapped_range().map_err(|e|e.to_string())?.to_vec();target.unmap();Ok(data)
}
fn f32_at(bytes:&[u8],offset:usize)->f32 {f32::from_le_bytes(bytes[offset..offset+4].try_into().unwrap())}
fn vec3_at(bytes:&[u8],offset:usize)->Vec3 {Vec3::new(f32_at(bytes,offset),f32_at(bytes,offset+4),f32_at(bytes,offset+8))}
fn matrix_at(bytes:&[u8],offset:usize)->Mat4 {
    let r: [[f32;4];3]=std::array::from_fn(|i|std::array::from_fn(|j|f32_at(bytes,offset+i*16+j*4)));
    Mat4::from_cols(Vec4::new(r[0][0],r[1][0],r[2][0],0.0),Vec4::new(r[0][1],r[1][1],r[2][1],0.0),
        Vec4::new(r[0][2],r[1][2],r[2][2],0.0),Vec4::new(r[0][3],r[1][3],r[2][3],1.0))
}
fn matrix_error(a:Mat4,b:Mat4)->f32 {
    a.to_cols_array().into_iter().zip(b.to_cols_array()).map(|(a,b)|(a-b).abs()).fold(0.0,f32::max)
}

/// Read the buffers actually used by the completed renderer, independently
/// compare with quaternion/glam CPU math, including every support-box corner.
fn verify_poses(gpu:&Gpu,renderer:&Renderer,fixture:&Fixture,wind:&WindState,
    previous:Option<&[Mat4]>)->Result<(Value,Vec<Mat4>),String> {
    let (pose_buffer,source_buffer,volume_buffer)=renderer.part_wind_debug_buffers().ok_or("pose buffers absent")?;
    let poses=read_buffer(gpu,pose_buffer)?;let sources=read_buffer(gpu,source_buffer)?;
    let volumes=volume_buffer.map(|b|read_buffer(gpu,b)).transpose()?;
    if poses.chunks_exact(4).any(|v|!f32::from_le_bytes(v.try_into().unwrap()).is_finite()) {
        return Err("non-finite current/previous GPU part pose".into());
    }
    let expected=sample_poses(&fixture.scene.part_wind,wind)?;
    let rest=sample_poses(&fixture.scene.part_wind,&WindState::default())?;
    let prior=previous.unwrap_or(&expected);let mut max_pose=0.0f32;let mut max_previous=0.0f32;
    let mut max_source=0.0f32;let mut max_inverse=0.0f32;let mut max_bounds=0.0f32;
    let mut worst_sphere_margin=f32::INFINITY;
    for (i,part) in fixture.scene.part_wind.iter().enumerate() {
        max_pose=max_pose.max(matrix_error(matrix_at(&poses,i*96),expected[i]));
        max_previous=max_previous.max(matrix_error(matrix_at(&poses,i*96+48),prior[i]));
        if let Some(owner)=part.owner {
            let offset=owner as usize*80;
            max_source=max_source.max(matrix_error(matrix_at(&sources,offset),expected[i]));
            let original=&fixture.scene.instances[owner as usize];
            let local_centre=rest[i].inverse().transform_point3(Vec3::from_slice(&original.sphere[..3]));
            let expected_centre=expected[i].transform_point3(local_centre);
            max_bounds=max_bounds.max(expected_centre.distance(vec3_at(&sources,offset+48)));
            let radius=f32_at(&sources,offset+60);let local_radius=original.sphere[3]/original.scale;
            // A dense fixed set of unit-sphere directions is independent of
            // the implementation's largest-root-axis bound.
            for j in 0..128 {
                let y=1.0-2.0*(j as f32+0.5)/128.0;let angle=j as f32*2.3999631;
                let r=(1.0-y*y).sqrt();let v=Vec3::new(r*angle.cos(),y,r*angle.sin());
                worst_sphere_margin=worst_sphere_margin.min(radius-expected[i].transform_vector3(v*local_radius).length());
            }
        }
        if let Some(volume)=part.volume {
            let bytes=volumes.as_ref().ok_or("volume readback absent")?;let offset=volume as usize*96;
            max_inverse=max_inverse.max(matrix_error(matrix_at(bytes,offset),expected[i].inverse()));
            let forest=fixture.scene.voxel_forest.as_ref().unwrap();let tree=&forest.instances[volume as usize];
            let mut local_min=Vec3::splat(f32::INFINITY);let mut local_max=Vec3::splat(f32::NEG_INFINITY);
            for l in tree.meta[1]..tree.meta[1]+tree.meta[2] {
                let level=&forest.levels[l as usize];let origin=Vec3::from_slice(&level.origin_cell[..3]);let h=level.origin_cell[3];
                let dims=Vec3::new(level.dims_offset[0] as f32,level.dims_offset[1] as f32,level.dims_offset[2] as f32);
                local_min=local_min.min(origin-Vec3::splat(h*0.5));local_max=local_max.max(origin+(dims+Vec3::splat(0.5))*h);
            }
            let mut lo=Vec3::splat(f32::INFINITY);let mut hi=Vec3::splat(f32::NEG_INFINITY);
            for corner in 0..8 {let p=Vec3::new(if corner&1==0{local_min.x}else{local_max.x},
                if corner&2==0{local_min.y}else{local_max.y},if corner&4==0{local_min.z}else{local_max.z});
                let p=expected[i].transform_point3(p);lo=lo.min(p);hi=hi.max(p);}
            max_bounds=max_bounds.max((lo-vec3_at(bytes,offset+48)).abs().max_element()).max((hi-vec3_at(bytes,offset+64)).abs().max_element());
        }
    }
    let finite=[max_pose,max_previous,max_source,max_inverse,max_bounds,worst_sphere_margin].iter().all(|x|x.is_finite());
    let passed=finite && [max_pose,max_previous,max_source,max_inverse,max_bounds].iter().all(|e|*e<2e-4) && worst_sphere_margin>=-2e-5;
    let report=json!({"passed":passed,"max_pose_error":max_pose,"max_previous_error":max_previous,
        "max_source_error":max_source,"max_inverse_error":max_inverse,"max_bounds_error":max_bounds,
        "minimum_sampled_sphere_margin":worst_sphere_margin,"branches":expected.len()});
    Ok((report,expected))
}

fn half(bits:u16)->f32 {
    let exponent=(bits>>10)&31;let fraction=bits&1023;
    if exponent==0 {let x=fraction as f32/16_777_216.0;return if bits&0x8000!=0{-x}else{x};}
    f32::from_bits(((bits as u32&0x8000)<<16)|((if exponent==31{255}else{exponent as u32+112})<<23)|((fraction as u32)<<13))
}
fn rgba_mask(mask:&[f32])->Vec<u8> {mask.iter().flat_map(|v|{let c=(v.clamp(0.0,1.0)*255.0).round() as u8;[c,c,c,255]}).collect()}

fn run(out:PathBuf)->Result<(),String> {
    if out.as_os_str().is_empty() || out.exists(){return Err("--out must name a fresh directory".into());}
    let started=Instant::now();let assets=assets()?;let bake_ms=started.elapsed().as_secs_f64()*1000.0;
    let source=build(&assets,false)?;let volume=build(&assets,true)?;
    std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let gpu=Gpu::new()?;
    let mut light=Lighting {taa:0.0,sharpen:0.0,lod_pixels:0.0,gi:0.0,contact_occlusion:0.0,
        reflection_occlusion:0.0,fog_density:0.0,fog_start:10000.0,fog_end:20000.0,..Default::default()};
    let mut cases=Vec::new();let mut failures=Vec::new();
    let picks:Vec<_>=(0..HEIGHT).flat_map(|y|(0..WIDTH).map(move|x|(x,y))).collect();
    let cameras=[("close",Vec3::new(0.0,1.3,4.5)),("middle",Vec3::new(0.0,2.0,12.0)),("far",Vec3::new(0.0,4.0,40.0))];
    for (light_label,sun,sky,sun_lux) in [("backlight",Vec3::new(0.2,0.25,-1.0).normalize(),9000.0,70000.0),
        ("lowlight",Vec3::new(-0.7,0.6,0.3).normalize(),1500.0,3000.0),("dark",Vec3::Y,0.0,0.0)] {
        // Sky SH/radiance normalization is immutable per Renderer, so a new
        // renderer is required for a genuinely different sky illuminance.
        light.sun_dir=sun;light.sun_lux=sun_lux;light.sky_lux=sky;
        let source_renderer=Renderer::new(&gpu,&source.scene,&light);let volume_renderer=Renderer::new(&gpu,&volume.scene,&light);
        let mut previous_source=None;let mut previous_volume=None;
        for (camera_label,pos) in cameras {
            let cam=Camera {pos,target:Vec3::new(0.0,0.3,0.0),fov_y:50.0f32.to_radians(),near:0.05};
            for (wind_label,strength,time) in [("still",0.0,0.0),("wind-t0",0.75,0.0),("wind-t1",0.75,1.0)] {
                if light_label=="dark" && (camera_label!="close" || wind_label!="still") {continue;}
                let wind=WindState {time_seconds:time,strength,direction:Vec2::new(0.8,0.6),..Default::default()};
                let prefix=format!("{camera_label}-{light_label}-{wind_label}");let mut source_mask=Vec::new();
                for (mode,fixture,renderer,previous) in [("source",&source,&source_renderer,&mut previous_source),
                    ("voxel",&volume,&volume_renderer,&mut previous_volume)] {
                    renderer.set_wind(&gpu,&wind)?;
                    let now=Instant::now();let (pixels,stats)=renderer.render_picking(&gpu,&cam,&light,WIDTH,HEIGHT,if mode=="source"{&picks}else{&[]});
                    let wall_ms=now.elapsed().as_secs_f64()*1000.0;
                    let (pose_report,expected)=verify_poses(&gpu,renderer,fixture,&wind,previous.as_deref())?;*previous=Some(expected);
                    if pose_report["passed"]!=true {failures.push(format!("{prefix}-{mode}: GPU pose/bounds oracle"));}
                    let max_rgb=pixels.chunks_exact(4).flat_map(|p|p[..3].iter()).copied().max().unwrap_or(0);
                    if light_label=="dark" && max_rgb>2 {failures.push(format!("{prefix}-{mode}: nonzero output with no sun or sky ({max_rgb})"));}
                    render::write_png(&out.join(format!("{prefix}-{mode}.png")),WIDTH,HEIGHT,&pixels)?;
                    let coverage=if mode=="source" {
                        source_mask=stats.picks.iter().map(|p|if p.is_some_and(|(owner,_,_)|fixture.scene.instances[owner as usize].flags&PART_WIND!=0){1.0}else{0.0}).collect();
                        render::write_png(&out.join(format!("{prefix}-source-alpha.png")),WIDTH,HEIGHT,&rgba_mask(&source_mask))?;
                        json!({"area_pixels":source_mask.iter().sum::<f32>()})
                    }else {
                        let layers=renderer.voxel_layers_rgba16(&gpu)?;let (w,h,bytes)=layers.first().ok_or("voxel layer absent")?;
                        if (*w,*h)!=(WIDTH,HEIGHT) || bytes.len()!=WIDTH as usize*HEIGHT as usize*8{return Err("wrong raw voxel layer dimensions".into());}
                        let mut alpha=Vec::with_capacity(source_mask.len());
                        for pixel in bytes.chunks_exact(8) {
                            let rgba:[f32;4]=std::array::from_fn(|i|half(u16::from_le_bytes([pixel[2*i],pixel[2*i+1]])));
                            if !rgba.iter().all(|v|v.is_finite()) || !(0.0..=1.001).contains(&rgba[3]) {return Err("invalid voxel HDR/alpha sample".into());}
                            alpha.push(rgba[3]);
                        }
                        render::write_png(&out.join(format!("{prefix}-voxel-alpha.png")),WIDTH,HEIGHT,&rgba_mask(&alpha))?;
                        let area=alpha.iter().map(|v|*v as f64).sum::<f64>();let reference=source_mask.iter().map(|v|*v as f64).sum::<f64>();
                        let intersection=alpha.iter().zip(&source_mask).map(|(a,b)|a.min(*b) as f64).sum::<f64>();
                        let union=alpha.iter().zip(&source_mask).map(|(a,b)|a.max(*b) as f64).sum::<f64>();
                        json!({"area_pixels":area,"source_area_pixels":reference,"volume_to_source_area":area/reference.max(1.0),
                            "soft_iou":intersection/union.max(1.0),"reference":"actual binary native source visibility; no supersampling or temporal resolve"})
                    };
                    let diagnostics:Vec<_>=renderer.voxel_diagnostics(&gpu)?.unwrap_or_default().into_iter().map(|d| {
                        if d.step_overflow_rays>0 {failures.push(format!("{prefix}: {} incomplete rays",d.step_overflow_rays));}
                        json!({"step_overflow_rays":d.step_overflow_rays,"candidate_overflow_tiles":d.candidate_overflow_tiles,
                            "ray_candidate_overflow_rays":d.ray_candidate_overflow_rays,"active_cache_overflow_rays":d.active_cache_overflow_rays,
                            "max_tile_candidates":d.max_tile_candidates,"max_ray_candidates":d.max_ray_candidates,"max_active_volumes":d.max_active_volumes,
                            "max_occupied_lod_x256":d.max_occupied_lod_x256,"occupied_coarse_mip_rays":d.occupied_coarse_mip_rays,
                            "max_steps_used":d.max_steps_used,"traced_rays":d.traced_rays})
                    }).collect();
                    let row=json!({"case":prefix,"mode":mode,"camera":{"position":pos.to_array(),"target":cam.target.to_array(),"fov_y":cam.fov_y},
                        "wind":{"strength":strength,"seconds":time},"light":{"sun_lux":sun_lux,"sky_lux":sky},"max_rgb_u8":max_rgb,
                        "pose_oracle":pose_report,"coverage":coverage,"diagnostics":diagnostics,
                        "wall_ms":wall_ms,"gpu_ms":stats.gpu_ms,"triangles":stats.triangles,"shadow_triangles":stats.shadow_triangles});
                    println!("{row}");cases.push(row);
                }
            }
        }
    }
    let report=json!({"schema":1,"purpose":"bounded native source/derived-volume GPU proof, not gameplay or production acceptance",
        "adapter":gpu.info.name,"size":[WIDTH,HEIGHT],"plants":source.plant_count,"shared_archetype_assets":assets.len(),
        "shared_branch_volume_assets":source.assets,"rig_nodes":source.scene.part_wind.len(),"volume_instances":volume.scene.voxel_forest.as_ref().unwrap().instances.len(),
        "volume_bytes":volume.scene.voxel_forest.as_ref().unwrap().byte_len(),"identities":volume.identities,
        "bake_ms":bake_ms,"total_seconds":started.elapsed().as_secs_f64(),"voxel_lod_pixels":volume.scene.voxel_lod_pixels,
        "projection_measure":"validated closed-surface entry area; source material and normal moments unchanged",
        "closed_parts_checked":assets.iter().flat_map(|a|&a.branches).map(|b|b.asset.stats.closed_parts_checked).sum::<u64>(),
        "closed_components_checked":assets.iter().flat_map(|a|&a.branches).map(|b|b.asset.stats.closed_components_checked).sum::<u64>(),
        "source_geometry":"opaque procedural blades/needles/stems; no photo alpha cards","wind":"one shared GPU hierarchical rigid pose per branch",
        "limitations":["bounded patch, no production world placement","continuous source-to-volume crossfade is not implemented",
            "fractional-volume DLSS motion remains explicitly unsupported","normal lobes and homogeneous extinction approximate correlated opaque leaf coverage",
            "sun shadows retain source geometry, mean canopy density is not rebuilt for branch animation","single-frame GPU times include a cold fixture and are not gameplay FPS"],
        "passed":failures.is_empty(),"failures":failures,"runs":cases});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if !failures.is_empty(){return Err(failures.join("\n"));}Ok(())
}

/// Six-frame opt-in proof that selected groups really change representation
/// while retaining the same branch rig. Rebuilding a fixture per view is a
/// diagnostic convenience, not a proposed production streaming implementation.
fn run_automatic(out:PathBuf)->Result<(),String> {
    if out.as_os_str().is_empty()||out.exists(){return Err("--out must name a fresh directory".into());}
    let assets=assets()?;std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let gpu=Gpu::new()?;
    let light=Lighting {taa:0.0,sharpen:0.0,lod_pixels:0.0,gi:0.0,contact_occlusion:0.0,
        reflection_occlusion:0.0,fog_density:0.0,fog_start:10000.0,fog_end:20000.0,..Default::default()};
    let wind=WindState {time_seconds:0.75,strength:0.75,direction:Vec2::new(0.8,0.6),..Default::default()};
    let mut runs=Vec::new();let mut failures=Vec::new();
    for (name,pos) in [("near",Vec3::new(0.0,1.3,4.5)),("far",Vec3::new(0.0,2.0,80.0)),("distant",Vec3::new(0.0,4.0,140.0))] {
        let camera=Camera {pos,target:Vec3::new(0.0,0.3,0.0),fov_y:50.0f32.to_radians(),near:0.05};
        for (mode,build_mode_value) in [("source",Mode::Source),("automatic",Mode::Automatic(camera))] {
            let fixture=build_mode(&assets,build_mode_value)?;let renderer=Renderer::new(&gpu,&fixture.scene,&light);
            renderer.set_wind(&gpu,&wind)?;
            let (pixels,stats)=renderer.render_picking(&gpu,&camera,&light,WIDTH,HEIGHT,&[]);
            let (poses,_)=verify_poses(&gpu,&renderer,&fixture,&wind,None)?;
            if poses["passed"]!=true{failures.push(format!("{name}-{mode}: pose mismatch"));}
            for d in renderer.voxel_diagnostics(&gpu)?.unwrap_or_default() {
                if d.step_overflow_rays>0{failures.push(format!("{name}-{mode}: incomplete volume rays"));}
            }
            render::write_png(&out.join(format!("{name}-{mode}.png")),WIDTH,HEIGHT,&pixels)?;
            let volume_count=fixture.scene.voxel_forest.as_ref().map_or(0,|f|f.instances.len());
            let row=json!({"view":name,"mode":mode,"camera":pos.to_array(),"volume_groups":volume_count,
                "source_groups":fixture.scene.part_wind.iter().filter(|p|p.owner.is_some()&&p.volume.is_none()).count(),
                "selection":fixture.selection,"pose_oracle":poses,"gpu_ms":stats.gpu_ms,
                "triangles":stats.triangles,"shadow_triangles":stats.shadow_triangles});
            println!("{row}");runs.push(row);
        }
    }
    let report=json!({"schema":1,"purpose":"bounded automatic representation and shared-pose diagnostic",
        "adapter":gpu.info.name,"size":[WIDTH,HEIGHT],"runs":runs,"mechanical_checks_passed":failures.is_empty(),
        "failures":failures,"optical_evidence_override":true,"production_default":"source until optical evidence is accepted",
        "limitations":["geometric projected support is not an opacity equivalence bound","experimental per-view fixture rebuild, not runtime world streaming",
            "no continuous source-volume opacity crossfade","same source geometry still casts sun shadows","DLSS is not enabled"]});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if !failures.is_empty(){return Err(failures.join("\n"));}Ok(())
}
/// The runtime path uploads one immutable scene, then moves cameras without
/// rebuilding assets/owners/poses. Exact source/volume endpoint comparisons
/// intentionally hold the shader at its finest mip, isolating ownership from
/// the independent filtered-mip optical approximation.
fn run_runtime(out:PathBuf,active_tiles:bool)->Result<(),String> {
    if out.as_os_str().is_empty()||out.exists(){return Err("--out must name a fresh directory".into());}
    let assets=assets()?;let source=build(&assets,false)?;
    let mut volume=build(&assets,true)?;let mut dynamic=build_mode(&assets,Mode::Runtime)?;
    volume.scene.voxel_lod_pixels=0.0;dynamic.scene.voxel_lod_pixels=0.0;
    volume.scene.voxel_active_tiles=active_tiles;dynamic.scene.voxel_active_tiles=active_tiles;
    let gpu=Gpu::new()?;std::fs::create_dir_all(&out).map_err(|e|e.to_string())?;
    let light=Lighting {taa:0.0,sharpen:0.0,lod_pixels:0.0,gi:0.0,contact_occlusion:0.0,
        reflection_occlusion:0.0,fog_density:0.0,fog_start:10000.0,fog_end:20000.0,..Default::default()};
    let source_renderer=Renderer::new(&gpu,&source.scene,&light);
    let volume_renderer=Renderer::new(&gpu,&volume.scene,&light);
    let dynamic_renderer=Renderer::new(&gpu,&dynamic.scene,&light);
    let wind=WindState {time_seconds:0.75,strength:0.75,direction:Vec2::new(0.8,0.6),..Default::default()};
    for renderer in [&source_renderer,&volume_renderer,&dynamic_renderer] {renderer.set_wind(&gpu,&wind)?;}
    let camera=|z|Camera {pos:Vec3::new(0.0,1.3,z),target:Vec3::new(0.0,0.3,0.0),fov_y:50.0f32.to_radians(),near:0.05};
    let mut runs=Vec::new();let mut failures=Vec::new();let mut planners:Vec<foliage_representation::ViewPlanner>=Vec::new();
    for (name,distances) in [("near",vec![4.5]),("far",vec![240.0]),("near-return",vec![4.5]),
        ("four-views",vec![4.5,240.0,9.0,180.0])] {
        let four=distances.len()==4;let size=if four {(WIDTH*2,HEIGHT*2)}else{(WIDTH,HEIGHT)};
        let views:Vec<_>=distances.iter().enumerate().map(|(i,&distance)|render::ViewRect {
            cam:camera(distance),x:(i as u32%2)*WIDTH,y:(i as u32/2)*HEIGHT,w:WIDTH,h:HEIGHT}).collect();
        if planners.len()!=views.len() {planners.clear();planners.resize_with(views.len(),Default::default);}
        let mut planned=Vec::new();
        for (i,view) in views.iter().enumerate() {
            let matrix=Mat4::perspective_infinite_reverse_rh(view.cam.fov_y,view.w as f32/view.h as f32,view.cam.near)
                *Mat4::look_at_rh(view.cam.pos,view.cam.target,Vec3::Y);
            planned.push(planners[i].plan(&dynamic.scene.foliage_representations,
                View {view_projection:matrix,width:view.w,height:view.h},dynamic.scene.foliage_representation_policy)?);
        }
        let mut images=BTreeMap::new();let mut metrics=Vec::new();
        for (mode,fixture,renderer) in [("source",&source,&source_renderer),("volume",&volume,&volume_renderer),
            ("runtime",&dynamic,&dynamic_renderer)] {
            let (pixels,stats)=renderer.try_render_views(&gpu,&views,&light,size.0,size.1,&[])?;
            let (pose_report,_)=verify_poses(&gpu,renderer,fixture,&wind,None)?;
            if pose_report["passed"]!=true {failures.push(format!("{name}-{mode}: source/volume pose mismatch"));}
            let diagnostics=renderer.voxel_diagnostics(&gpu)?.unwrap_or_default();
            for diagnostic in &diagnostics {
                if diagnostic.step_overflow_rays>0 {failures.push(format!("{name}-{mode}: incomplete volume rays"));}
            }
            metrics.push(json!({"mode":mode,"gpu_ms":stats.gpu_ms,"triangles":stats.triangles,
                "shadow_triangles":stats.shadow_triangles,"pose_oracle":pose_report,
                "traced_rays":diagnostics.iter().map(|d|d.traced_rays).collect::<Vec<_>>()}));
            render::write_png(&out.join(format!("{name}-{mode}.png")),size.0,size.1,&pixels)?;
            images.insert(mode,pixels);
        }
        let mut comparisons=Vec::new();
        for (i,view) in views.iter().enumerate() {
            let volumes=planned[i].iter().filter(|p|p.volume_weight==1.0).count();
            let expected=if volumes==0 {"source"}else if volumes==planned[i].len(){"volume"}else{"mixed"};
            if expected=="mixed" {failures.push(format!("{name} view{i}: fixture endpoint did not isolate all-source/all-volume"));continue;}
            let mut changed=0u64;let mut max_error=0u8;
            for y in view.y..view.y+view.h {for x in view.x..view.x+view.w {
                let offset=((y*size.0+x)*4) as usize;let mut pixel_error=0;
                for c in 0..3 {pixel_error=pixel_error.max(images["runtime"][offset+c].abs_diff(images[expected][offset+c]));}
                max_error=max_error.max(pixel_error);if pixel_error>1 {changed+=1;}
            }}
            if changed>0 {failures.push(format!("{name} view{i}: {changed} pixels differ from {expected} endpoint by >1 code value"));}
            comparisons.push(json!({"view":i,"camera":view.cam.pos.to_array(),"expected":expected,
                "volume_groups":volumes,"source_groups":planned[i].len()-volumes,
                "changed_pixels_gt1":changed,"max_rgb_error_u8":max_error,
                "selected_mips":planned[i].iter().filter_map(|p|match p.representation {Representation::Voxel {level}=>Some(level),_=>None}).collect::<Vec<_>>()}));
        }
        let row=json!({"name":name,"size":[size.0,size.1],"comparisons":comparisons,"metrics":metrics});
        println!("{row}");runs.push(row);
    }
    let report=json!({"schema":1,"purpose":"actual per-view runtime source/volume ownership on one immutable scene",
        "voxel_active_tiles":active_tiles,
        "adapter":gpu.info.name,"plants":dynamic.plant_count,"rig_nodes":dynamic.scene.part_wind.len(),
        "dynamic_groups":dynamic.scene.foliage_representations.len(),"runs":runs,"passed":failures.is_empty(),"failures":failures,
        "limitations":["finest mip fixed to isolate ownership; not mip optical-parity acceptance",
            "subpixel branch switching, no continuous opacity crossfade","unvalidated optical evidence explicitly enabled",
            "native rendering only; fractional-volume motion for DLSS remains unsupported","fixture timing is not gameplay FPS"]});
    std::fs::write(out.join("report.json"),serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if !failures.is_empty(){return Err(failures.join("\n"));}Ok(())
}

fn main() {
    let args:Vec<_>=std::env::args().skip(1).collect();
    let result=if args.len()==2 && args[0]=="--out" {run(PathBuf::from(&args[1]))}
        else if args.len()==3&&args[0]=="--out"&&args[2]=="--automatic" {run_automatic(PathBuf::from(&args[1]))}
        else if args.len()==3&&args[0]=="--out"&&args[2]=="--runtime" {run_runtime(PathBuf::from(&args[1]),false)}
        else if args.len()==4&&args[0]=="--out"&&args[2]=="--runtime"&&args[3]=="--active-tiles" {
            run_runtime(PathBuf::from(&args[1]),true)
        }
        else{Err("use --out FRESH_DIRECTORY [--automatic|--runtime [--active-tiles]]".into())};
    if let Err(error)=result {eprintln!("procedural_foliage_gpu: {error}");std::process::exit(1);}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_fixture_keeps_near_source_and_reuses_same_far_branch_owners() {
        let assets=assets().unwrap();
        let camera=|z|Camera {pos:Vec3::new(0.0,2.0,z),target:Vec3::new(0.0,0.3,0.0),fov_y:50.0f32.to_radians(),near:0.05};
        let near=build_mode(&assets,Mode::Automatic(camera(4.5))).unwrap();
        let far=build_mode(&assets,Mode::Automatic(camera(140.0))).unwrap();
        assert!(near.scene.voxel_forest.is_none());
        assert!(far.scene.voxel_forest.as_ref().is_some_and(|f|!f.instances.is_empty()));
        assert_eq!(near.scene.instances.len(),far.scene.instances.len());
        assert_eq!(near.scene.part_wind.len(),far.scene.part_wind.len());
        for (a,b) in near.scene.part_wind.iter().zip(&far.scene.part_wind) {
            assert_eq!(a.owner,b.owner);assert_eq!(a.parent,b.parent);assert_eq!(a.rest,b.rest);
            assert_eq!(a.phase,b.phase);assert_eq!(a.max_angle,b.max_angle);
        }
        let runtime=build_mode(&assets,Mode::Runtime).unwrap();
        foliage_representation::validate_bindings(&runtime.scene).unwrap();
        assert_eq!(runtime.scene.instances.len(),near.scene.instances.len());
        assert_eq!(runtime.scene.foliage_representations.len(),runtime.scene.voxel_forest.as_ref().unwrap().instances.len());
        let mut planner=foliage_representation::ViewPlanner::default();
        for (distance,expected_volume) in [(4.5,false),(240.0,true),(4.5,false),(9.0,false),(180.0,true)] {
            let camera=camera(distance);
            let view=View {view_projection:Mat4::perspective_infinite_reverse_rh(camera.fov_y,WIDTH as f32/HEIGHT as f32,camera.near)
                *Mat4::look_at_rh(camera.pos,camera.target,Vec3::Y),width:WIDTH,height:HEIGHT};
            let plans=planner.plan(&runtime.scene.foliage_representations,view,runtime.scene.foliage_representation_policy).unwrap();
            assert!(plans.iter().all(|p|(p.volume_weight==1.0)==expected_volume),"runtime endpoint at {distance}m was mixed");
        }
    }
}
