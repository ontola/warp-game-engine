//! Frame-wide GPU forest residency. CPU supplies recipes and permanent roots
//! through stable population transactions; every consumer's demand is combined
//! on GPU before assigning branches.
use crate::{Gpu,Scene,foliage_select::{Node,Instance,Selector,NONE},foliage_residency::Residency,
    scene::{GpuInstance,PART_WIND},foliage_volume_data::GpuTree};
use glam::{Mat4,Vec3,Vec4};
use bytemuck::{Pod,Zeroable};
use wgpu::util::DeviceExt;

/// World placement independent of any camera, renderer tier or CPU near pool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootPlacement {
    pub pos:Vec3,pub yaw:f32,pub scale:f32,pub variant:usize,
    /// Unit world-space up vector. Trees normally use Y; cells can follow terrain.
    pub up:Vec3,
}

#[repr(C)]
#[derive(Clone,Copy,Debug,Pod,Zeroable)]
pub struct Branch {
    rest:[[f32;4];3], source:GpuInstance, volume:GpuTree,
    low:[f32;4], high:[f32;4], animation:[f32;4], info:[u32;4],
}
#[derive(Clone,Debug)]
pub struct Spec {
    pub page_budget:crate::foliage_pages::cache::Budget,
    pub source_budget:crate::foliage_pages::source::Budget,
    pub nodes:Vec<Node>,
    /// Per node: source clusters the cull visits per error bin (source_work_table).
    pub source_tables:Vec<crate::foliage_pages::source::PackedSourceTable>,
    pub roots:Vec<Instance>,branches:Vec<Branch>,representations:Vec<[u32;4]>,
    proxies:Vec<GpuTree>,templates:Vec<RootTemplate>,population:crate::foliage_population::Population,pub first_owner:u32,pub slots:u32,pub error:f32,pub raster_budget:u32,pub source_cluster_budget:u32,
}
impl Spec {
    pub(crate) fn source_page_instances(&self)->Vec<Option<GpuInstance>> {
        self.representations.iter().map(|r|(r[1]==0).then(||self.branches[r[0]as usize].source)).collect()
    }
    /// The levels finer than a branch voxel selection, nearest first. The sun
    /// pass picks its level per texel and uses these when they are resident,
    /// so the page cache loads them with whatever budget the selections that
    /// must draw leave over; no selection waits for them.
    pub(crate) fn voxel_page_finer(&self)->Vec<Vec<usize>> {
        self.representations.iter().map(|rep|{let branch=&self.branches[if rep[3]==0 {rep[0]}else{rep[3]-1}as usize];
            if rep[1]!=1 || (rep[3]==0 && branch.info[0]==NONE) {return Vec::new();}
            let start=branch.volume.meta[1]as usize;(start..start+(rep[2].min(branch.volume.meta[2]))as usize).rev().collect()
        }).collect()
    }
    pub(crate) fn voxel_page_dependencies(&self,levels:usize)->Result<(Vec<Vec<usize>>,Vec<bool>),String> {
        let mut pinned=vec![false;levels];
        let range=|v:GpuTree|->Result<Vec<usize>,String>{let start=v.meta[1]as usize;let end=start.checked_add(v.meta[2]as usize).ok_or("voxel dependency overflow")?;if end>levels{return Err("voxel dependency outside catalog".into());}Ok((start..end).collect())};
        for proxy in &self.proxies {for id in range(*proxy)?{pinned[id]=true;}}
        let deps=self.representations.iter().map(|rep|{let branch=&self.branches[if rep[3]==0 {rep[0]}else{rep[3]-1}as usize];
            if rep[1]==1 && rep[3]==0 && branch.info[0]==NONE {
                // Direct whole-root rendering publishes exactly the chosen mip
                // (materialize identity.z=1), including its shadow consumer.
                let levels=range(branch.volume)?;
                Ok(vec![*levels.get(rep[2]as usize).ok_or("root voxel mip outside catalog")?])
            } else if rep[1]==0 {Ok(Vec::new())}
            // Only voxel representations need voxel bytes. A source selection
            // renders source geometry for either consumer; it must not wait
            // for an unrelated voxel shadow bundle. A voxel slot waits for its
            // selected mip and the coarser ones, never for a finer level the
            // camera does not draw: its shadow uses a finer one when the cache
            // had room for it (voxel_page_finer, volume_sun_splats_impl).
            else{let levels=range(branch.volume)?;Ok(levels.get(rep[2]as usize..).filter(|l|!l.is_empty()).ok_or("voxel mip outside catalog")?.to_vec())}
        }).collect::<Result<Vec<_>,String>>()?;
        for template in &self.templates {
            let node=&self.nodes[template.node as usize];
            for &id in &deps[node.links[2]as usize]{pinned[id]=true;}
        }
        Ok((deps,pinned))
    }
}
#[derive(Clone,Copy,Debug)]
struct RootTemplate {proxy:GpuTree,source:GpuInstance,node:u32,branch:u32,shadow_sphere:[f32;4],proxy_work:f32}
fn transform(p:RootPlacement)->Mat4 {Mat4::from_scale_rotation_translation(Vec3::splat(p.scale),glam::Quat::from_rotation_arc(Vec3::Y,p.up)*glam::Quat::from_rotation_y(p.yaw),p.pos)}
fn rows(m:Mat4)->[[f32;4];3] {[m.row(0).to_array(),m.row(1).to_array(),m.row(2).to_array()]}
fn matrix(r:[[f32;4];3])->Mat4 {Mat4::from_cols(
    Vec4::new(r[0][0],r[1][0],r[2][0],0.),Vec4::new(r[0][1],r[1][1],r[2][1],0.),
    Vec4::new(r[0][2],r[1][2],r[2][2],0.),Vec4::new(r[0][3],r[1][3],r[2][3],1.))}
// The existing GPU ABI reserves two floats on nodes and roots. Integer work
// bounds are exact up to 2^24; reject larger assets instead of rounding down.
fn exact_work(value:u32)->Result<f32,String> {
    if value>1<<24 {return Err("single representation raster work exceeds exact GPU metadata range".into());}
    Ok(value as f32)
}
fn representation_raster_work(rep:[u32;4],branch:&Branch,counts:&[u32])->Result<[u32;2],String> {
    if rep[1]==0 {return Ok([0,0]);}
    let start=branch.volume.meta[1]as usize;
    let end=start.checked_add(branch.volume.meta[2]as usize).ok_or("raster work mip overflow")?;
    let mips=counts.get(start..end).filter(|m|!m.is_empty()).ok_or("raster work missing volume mips")?;
    if rep[1]==1 && rep[3]==0 && branch.info[0]==NONE {
        let work=*mips.get(rep[2]as usize).ok_or("raster work root mip outside catalog")?;
        return Ok([work,work]);
    }
    let shadow=*mips.iter().max().unwrap();
    // The camera raster draws exactly the selected mip (selection bit 12 from
    // foliage_materialize); only shadows still choose a level per texel.
    let camera=if rep[1]==0 {0} else {*mips.get(rep[2]as usize).ok_or("raster work selected mip outside catalog")?};
    Ok([camera,shadow])
}
/// The emergency stand-in must not pin its entire high-resolution mip chain.
/// Ordinary root detail is requested separately through the selection hierarchy.
fn coarse_proxy(mut v:GpuTree)->Result<GpuTree,String> {
    v.meta[1]=v.meta[1].checked_add(v.meta[2].checked_sub(1).ok_or("empty root proxy")?).ok_or("root proxy mip overflow")?;
    v.meta[2]=1;Ok(v)
}
fn local_bounds(v:GpuTree)->([f32;4],[f32;4]) {
    let inverse=matrix(v.inverse);let mut low=Vec3::splat(f32::INFINITY);let mut high=-low;
    for i in 0..8 {let p=Vec3::new(if i&1==0{v.bounds_min[0]}else{v.bounds_max[0]},
        if i&2==0{v.bounds_min[1]}else{v.bounds_max[1]},if i&4==0{v.bounds_min[2]}else{v.bounds_max[2]});
        let p=inverse.transform_point3(p);low=low.min(p);high=high.max(p);}
    (low.extend(0.).to_array(),high.extend(0.).to_array())
}

/// Shared articulated deformation, independent of material/debug category.
#[derive(Clone,Copy,Debug,Default,PartialEq,Eq)]
#[repr(u32)]
pub enum Deformation {
    #[default]
    Rigid=0,
    /// Shear along each branch's local height, preserving its basal plane.
    /// Currently requires a fixed root with direct child groups.
    BasalShear=1,
}

/// One species in the shared population catalog. Roots select its index via
/// RootPlacement::variant; category never selects a separate renderer/cache.
#[derive(Clone,Copy)]
pub struct Species<'a> {
    pub plant:&'a crate::vegetation_asset::CookedPlant,
    pub category:crate::scene::GeometryCategory,
    pub deformation:Deformation,
}

/// Attach shared recipe geometry once, plus one lightweight root per tree and
/// a bounded number of branch OUTPUT slots. Never constructs recipe_forest::Pool.
pub fn plant(scene:&mut Scene,plants:&[crate::vegetation_asset::CookedPlant],trees:&[RootPlacement],slots:u32,policy:crate::aggregate_lod::Policy)->Result<(),String> {
    let roots=trees.iter().enumerate().map(|(id,&placement)|crate::foliage_population::WorldRoot {id:id as u64,placement}).collect::<Vec<_>>();
    plant_population(scene,plants,&roots,trees.len(),slots,policy)
}
/// Reserve world-root capacity separately from transient branch residency.
/// Loaded roots carry stable world IDs; unused capacity is explicitly inactive.
pub fn plant_population(scene:&mut Scene,plants:&[crate::vegetation_asset::CookedPlant],world:&[crate::foliage_population::WorldRoot],capacity:usize,slots:u32,policy:crate::aggregate_lod::Policy)->Result<(),String> {
    let species=plants.iter().map(|plant|Species {plant,category:crate::scene::GeometryCategory::Trees,deformation:crate::foliage_runtime::Deformation::Rigid}).collect::<Vec<_>>();
    plant_species_population(scene,&species,world,capacity,slots,policy)
}
/// Mixed grass, shrub and tree species share one selection, page cache, pose
/// schedule and shadow path. Category controls diagnostics/material consumers.
pub fn plant_species_population(scene:&mut Scene,species:&[Species<'_>],world:&[crate::foliage_population::WorldRoot],capacity:usize,slots:u32,policy:crate::aggregate_lod::Policy)->Result<(),String> {
    let plants=species.iter().map(|s|s.plant).collect::<Vec<_>>();
    use crate::vegetation_asset::{SceneVegetation,AttachLimits,PlantInstance,SelectedRepresentation};
    if scene.gpu_forest.is_some() || !scene.part_wind.is_empty() || scene.voxel_forest.is_some() {
        return Err("attach all GPU forest species together before other articulated vegetation".into());
    }
    if species.iter().any(|s|s.deformation==Deformation::BasalShear && !s.plant.supports_basal_shear()) {
        return Err("basal shear requires direct child groups under a fixed root".into());
    }
    let population=crate::foliage_population::Population::new(world,capacity)?;
    let parked=RootPlacement {up:Vec3::Y,pos:Vec3::new(0.,-100000.,0.),yaw:0.,scale:1.,variant:0};
    let trees=population.slots.iter().map(|r|r.map(|r|r.placement).unwrap_or(parked)).collect::<Vec<_>>();
    if slots==0 || slots>=1<<19 || trees.is_empty() || plants.is_empty()
        || trees.iter().any(|t|t.variant>=plants.len()||!t.scale.is_finite()||t.scale<=0.) {return Err("invalid GPU forest population/budget".into());}
    let poses=plants.iter().map(|p|p.stats().branches).sum::<usize>();
    let mut limits=AttachLimits {virtual_members:true,max_poses:poses.max(64),max_primitive_instances:1<<19,..Default::default()};
    limits.pack.max_assets=poses+plants.len();// The packed branch catalog is bounded independently of world roots.
    // append_proxy uses the asset-only catalog and is not a branch group.
    limits.pack.max_instances=poses;
    limits.fit_voxels(plants.iter().copied());
    let mut builder=SceneVegetation::new(scene,limits)?;
    let mut handles=Vec::new();
    for (i,p) in plants.iter().enumerate() {handles.push(builder.append(p,PlantInstance {id:i as u64+1,generation:0,root:Mat4::IDENTITY,category:species[i].category})?);}
    for (i,p) in plants.iter().enumerate() {
        builder.append_proxy(p,PlantInstance {id:(plants.len()+capacity+i+1)as u64,generation:0,root:Mat4::IDENTITY,category:species[i].category})?;

    }
    for (i,t) in trees.iter().enumerate() {
        let root=transform(*t);
        builder.append_proxy(&plants[t.variant],PlantInstance {id:(plants.len()+i+1)as u64,generation:0,root,category:species[t.variant].category})?;
    }
    builder.finish_aggregate(policy)?;
    let mut forest=scene.voxel_forest.take().ok_or("GPU forest has no volumes")?;
    let proxy_start=forest.instances.len()-trees.len();
    let proxies=forest.instances[proxy_start..].iter().copied().map(coarse_proxy).collect::<Result<Vec<_>,_>>()?;
    let template_volumes=forest.instances[proxy_start-plants.len()..proxy_start].iter().copied().map(coarse_proxy).collect::<Result<Vec<_>,_>>()?;
    let templates=template_volumes.iter().map(|v|RootTemplate {proxy:*v,source:scene.instances[v.meta[0]as usize],node:0,branch:0,shadow_sphere:[0.;4],proxy_work:0.}).collect::<Vec<_>>();
    for v in &template_volumes {let owner=&mut scene.instances[v.meta[0]as usize];owner.sphere[3]=-1.;owner.flags=0;}

    let mut nodes=Vec::new();let mut representations=Vec::new();let mut root_nodes=Vec::new();let mut group_fixups=Vec::new();
    for (variant,p) in plants.iter().enumerate() {
        let recipe=p.shared_selection_recipe(policy)?;let node_base=nodes.len()as u32;let rep_base=representations.len()as u32;
        root_nodes.push(node_base);
        for mut n in recipe.nodes {if matches!(recipe.representations[n.links[2]as usize],SelectedRepresentation::Voxel {branch:0,..}) {n.links[3]|=2;}if n.links[0]!=NONE {n.links[0]+=node_base;}n.links[1]+=node_base;n.links[2]+=rep_base;nodes.push(n);}
        for r in recipe.representations {let rep_index=representations.len();representations.push(match r {
            SelectedRepresentation::SourceBranch {branch,..}=>[handles[variant].pose_range.start as u32+branch,0,0,0],
            SelectedRepresentation::Voxel {branch,level,..}=>[handles[variant].pose_range.start as u32+branch,1,level,0],
            SelectedRepresentation::GroupVoxel {group,frame_branch,level}=>{
                group_fixups.push((rep_index,variant,group));
                [handles[variant].pose_range.start as u32+frame_branch,1,level,0]
            },
        });}
    }
    let mut branches=Vec::new();
    for p in &scene.part_wind {
        let source=p.owner.map(|o|scene.instances[o as usize]).unwrap_or_else(GpuInstance::zeroed);
        let volume=p.volume.map(|v|forest.instances[v as usize]).unwrap_or(forest.instances[0]);
        let (low,high)=local_bounds(volume);
        branches.push(Branch {rest:rows(p.rest),source,volume,low,high,
            animation:[p.max_angle,p.harmonic as f32,p.phase,0.],info:[p.parent.unwrap_or(NONE),u32::from(p.volume.is_some() && p.parent.is_some()),0,0]});
    }
    for (index,handle) in handles.iter().enumerate() {
        let mode=species[index].deformation;
        for at in handle.pose_range.clone() {
            let branch=&mut branches[at];
            branch.info[3]=mode as u32;
        }
    }
    // Geometry descriptors share this buffer, but are NOT articulated joints.
    // Representation.x remains the original pose; .w selects volume data only.
    let mut group_records=std::collections::BTreeMap::new();
    for (variant,plant) in plants.iter().enumerate() {
        let groups=plant.geometry_volumes().filter(|g|group_fixups.iter().any(|(_,v,id)|*v==variant && *id==g.0)).collect::<Vec<_>>();
        if groups.is_empty(){continue;}
        let assets=groups.iter().map(|g|g.2.clone()).collect::<Vec<_>>();
        let cells=assets.iter().flat_map(|a|&a.levels).map(|l|l.voxels.len()).sum();
        let packed=crate::foliage_volume_data::pack_assets(&assets,crate::foliage_volume_data::AssetPackLimits {
            max_assets:assets.len(),levels_per_asset:crate::foliage_representation::MAX_LEVELS,
            first_level_bounds:false,max_dense_indices:1<<28,max_sparse_cells:cells})?;
        let base=forest.levels.len()as u32;
        for (asset,(group,frame,_)) in groups.iter().enumerate() {
            let pose=handles[variant].pose_range.start+*frame as usize;
            let mut source=branches[pose].source;
            source.rows=rows(Mat4::IDENTITY);source.first_cluster=0;source.cluster_count=0;
            let mut volume=packed.instance(crate::foliage_volume_data::VolumeBinding {owner:0,asset:asset as u32,crown_material:crate::foliage_volume_data::ALL_MATERIALS},&source)?;
            volume.meta[1]+=base;
            let (low,high)=local_bounds(volume);
            group_records.insert((variant,*group),branches.len()as u32);
            branches.push(Branch {rest:rows(Mat4::IDENTITY),source,volume,low,high,animation:[0.;4],info:[NONE,1,0,0]});
        }
        forest.append(packed.forest)?;
    }
    for (rep,variant,group) in group_fixups {representations[rep][3]=group_records[&(variant,group)]+1;}
    let work=forest.raster_work_bounds()?;
    let mut source_tables=Vec::with_capacity(nodes.len());
    for node in &mut nodes {
        let rep=representations[node.links[2]as usize];
        let branch=&branches[if rep[3]==0 {rep[0]} else {rep[3]-1}as usize];
        let source_work=if rep[1]==0 {crate::foliage_pages::source::source_work(scene,branch.source)?.clusters}else{0};
        source_tables.push(if rep[1]==0 {crate::foliage_pages::source::source_work_table(scene,branch.source)?}else{[0;12]});
        if source_work>0x00ff_ffff {return Err("source cluster work exceeds node metadata range".into());}
        node.links[3]=(node.links[3]&3)|if rep[1]==0 {4}else{0}|(source_work<<8);
        let bound=representation_raster_work(rep,branch,&work)?;
        // Near-pixel voxels: a voxel node's spatial error is at least its cell,
        // so cells project under the pixel target. The optical footprint (second
        // error) is 0 when optics validate at any size, which let 37 px cells
        // stand 12 m away.
        if rep[1]==1 {
            let level=forest.levels.get((branch.volume.meta[1]+rep[2])as usize).ok_or("voxel node mip outside catalog")?;
            node.error[0]=node.error[0].max(level.origin_cell[3]);
            // Bits 3-7: k with cell <= error.x/2^k, so the error cut can book
            // the raster's sub-pixel thinning from the real cell, not the error.
            let k=(node.error[0]/level.origin_cell[3].max(1e-12)).log2().floor().clamp(0.,31.)as u32;
            node.links[3]=(node.links[3]&!(31<<3))|(k<<3);
        }
        node.error[2]=exact_work(bound[0])?;
        node.error[3]=exact_work(bound[1])?;
    }
    // Reference recipe carriers are not world instances. Keep the shared
    // negative-radius primitive templates; retire only their reference owners.
    for p in &scene.part_wind {if let Some(o)=p.owner {scene.instances[o as usize].sphere[3]=-1.;scene.instances[o as usize].first_cluster=0;}}
    scene.part_wind.clear();scene.part_attachments.clear();scene.aggregate_lod=None;
    scene.foliage_representations.clear();scene.foliage_source_groups.clear();
    let first_owner=scene.instances.len()as u32;
    if first_owner.checked_add(slots).is_none_or(|end|end>=1<<19) {return Err("GPU branch owners exceed virtual ID encoding".into());}
    let template=branches.iter().find(|b|b.source.first_cluster&4095!=0).ok_or("GPU forest has no source templates")?.source;
    let mut parked=template;parked.rows=rows(Mat4::from_translation(Vec3::new(0.,-100000.,0.)));parked.sphere=[0.,-100000.,0.,1.];
    scene.instances.extend(std::iter::repeat_n(parked,slots as usize));
    let mut placeholder=forest.instances[0];placeholder.inverse=rows(matrix(parked.rows).inverse());
    placeholder.bounds_min=[0.,-100001.,0.,0.];placeholder.bounds_max=[1.,-100000.,1.,0.];
    forest.instances=(0..slots).map(|slot|GpuTree {meta:[first_owner+slot,placeholder.meta[1],placeholder.meta[2],u32::MAX],..placeholder}).collect();
    forest.instances.extend_from_slice(&proxies);
    forest.instances.extend_from_slice(&proxies); // Independent shadow roots share asset and pose ownership.
    let mut templates=templates;
    for (i,t) in templates.iter_mut().enumerate(){t.proxy_work=exact_work(work[t.proxy.meta[1]as usize])?;t.node=root_nodes[i];t.shadow_sphere=nodes[t.node as usize].sphere;t.branch=handles[i].pose_range.start as u32;}
    let roots=trees.iter().enumerate().map(|(i,t)| {
        let m=transform(*t);
        // sqrt(norm1*normInf) conservatively bounds rotated scales.
        let row=(0..3).map(|r|m.row(r).truncate().abs().element_sum()).fold(0.,f32::max);
        let col=(0..3).map(|c|m.col(c).truncate().abs().element_sum()).fold(0.,f32::max);
        scene.instances[proxies[i].meta[0]as usize].flags|=PART_WIND;
        if population.slots[i].is_none(){let owner=&mut scene.instances[proxies[i].meta[0]as usize];owner.flags=PART_WIND;owner.sphere[3]=-1.;}
        Instance {rows:rows(m),scale:[(row*col).sqrt(),0.,templates[t.variant].proxy_work,templates[t.variant].proxy_work],info:[root_nodes[t.variant],population.generations[i],handles[t.variant].pose_range.start as u32,u32::from(population.slots[i].is_none())]}
    }).collect();
    // Every tree at its coarsest proxy, plus room for nearer trees to refine:
    // without it the far forest alone fills the splats and the shared camera
    // cut coarsens the near trees too (Joep's 13 px voxels in the forest).
    let env=|switch:&crate::switches::Switch,default:u64|switch.parse::<u64>().unwrap_or(default);
    let raster_budget=forest.instances.iter().map(|v|work[v.meta[1]as usize]as u64).sum::<u64>()
        .max(env(&crate::switches::RASTER_FLOOR,8<<20)).clamp(1,16<<20)as u32;
    // Source clusters bound the camera cut as well: unbounded, a forest
    // floor reserved 6.7M clusters and culling them took most of a 106 ms frame.
    let source_cluster_budget=env(&crate::switches::SOURCE_CLUSTERS,2<<20).clamp(1,crate::foliage_select::DEFAULT_SOURCE_CLUSTER_BUDGET as u64)as u32;
    scene.voxel_forest=Some(forest);scene.voxel_proxies=trees.len()as u32*2;
    scene.gpu_forest=Some(Spec {page_budget:Default::default(),source_budget:Default::default(),nodes,source_tables,roots,branches,representations,proxies,templates,population,first_owner,slots,raster_budget,source_cluster_budget,error:policy.max_error_pixels.max(0.25)});
    Ok(())
}

pub struct Runtime {
    selector:Selector,residency:Residency,uniform:wgpu::Buffer,bind:wgpu::BindGroup,
    pose_dispatch:wgpu::ComputePipeline,pose_dispatch_bind:wgpu::BindGroup,pose_args:wgpu::Buffer,
    pipeline:wgpu::ComputePipeline,pose_pipelines:[wgpu::ComputePipeline;3],pose_cache:wgpu::Buffer,pose_work:wgpu::Buffer,pose_depth:u32,branch_count:u32,empty_depth:wgpu::TextureView,
    pub choices:wgpu::Buffer,slots:u32,trees:u32,error:f32,raster_budget:u32,source_cluster_budget:u32,
    wind:std::sync::Mutex<crate::wind::WindState>,
    population:std::sync::Mutex<std::sync::Arc<crate::foliage_population::Population>>,templates:std::sync::Arc<Vec<RootTemplate>>,publish_pipeline:wgpu::ComputePipeline,publish_layout:wgpu::BindGroupLayout,source_buffer:wgpu::Buffer,proxy_buffer:wgpu::Buffer,proxy_owners:std::sync::Arc<Vec<u32>>,
}
#[derive(Debug,serde::Serialize)]
pub struct ConsumerCounts {pub outside:u32,pub camera_only:u32,pub shadow_only:u32,pub both:u32}
impl ConsumerCounts {
    fn from_words(v:&[u32])->Self {Self {outside:v[0],camera_only:v[1],shadow_only:v[2],both:v[3]}}
}
#[derive(Debug,serde::Serialize)]
pub struct CameraRefinementLimits {
    pub over_target:u32,pub branch_slots:u32,pub camera_splats:u32,pub shadow_splats:u32,
    pub missing_pages:u32,pub no_children:u32,pub affordable_resident:u32,
}
#[derive(Debug,serde::Serialize)]
pub struct Diagnostics {
    pub trees:u32,pub active_roots:usize,pub branch_capacity:u32,pub selected_records:u32,pub tested_nodes:u32,
    pub source_slots:u32,pub voxel_slots:u32,pub shadow_voxel_slots:u32,pub direct_roots:u32,pub fallback_trees:u32,pub retained:u32,pub assigned:u32,
    pub slot_failures:u32,pub hash_failures:u32,
    pub frontier_overflow_requests:u32,pub peak_frontier_requests:u32,
    pub reserved_source_clusters:u32,pub source_cluster_budget_stops:u32,pub reserved_raster_splats:[u32;2],pub refinement_budget_stops:u32,pub refinement_faults:u32,
    /// Error-driven cut: target multipliers (1 = exact pixel target) and highest cap fill, camera then shadow; page-blocked refinements.
    pub error_cut_scale:[f32;2],pub source_cut_scale:f32,pub error_cut_fill:[f32;2],pub error_cut_page_blocked:u32,pub reused_refinement_proposals:u32,
    pub dormant_records:u32,pub dormant_wakes:u32,pub dormant_wake_passes:u32,pub dormant_park_events:u32,pub dormant_overflows:u32,
    /// Selected-cut maxima: [all consumers, camera consumers], excluding unknown proxies.
    pub selected_spatial_error_pixels:[f32;2],pub selected_error_target_ratio:[f32;2],
    pub selected_over_target:[u32;2],pub error_unknown_proxies:u32,pub error_unmatched_direct:u32,pub error_forced_nodes:u32,
    pub pose_requests:u32,pub evaluated_shared_poses:u32,pub pose_hash_failures:u32,
    pub camera_roots:u32,pub shadow_only_roots:u32,pub camera_fallback_roots:u32,pub shadow_only_fallback_roots:u32,
    /// Selected-node frustum demand, before downstream occlusion/materialization.
    pub camera_refinement_limits:CameraRefinementLimits,
    pub selected_source_consumers:ConsumerCounts,pub selected_voxel_consumers:ConsumerCounts,pub selected_direct_consumers:ConsumerCounts,
    /// Root ownership counts, not branch-level consumer visibility.
    pub camera_published_branches:u64,pub shadow_only_published_branches:u64,
}
#[derive(Debug,serde::Serialize)]
pub struct PopulationStats {pub active:usize,pub capacity:usize,pub changed:usize,pub retained:usize,pub upload_bytes:usize,pub prepare_cpu_ms:f64,pub proxy_cpu_ms:f64,pub root_upload_cpu_ms:f64}
/// Immutable cheap-to-clone world metadata; safe to prepare on a worker thread.
pub struct PopulationSeed {base:std::sync::Arc<crate::foliage_population::Population>,templates:std::sync::Arc<Vec<RootTemplate>>,owners:std::sync::Arc<Vec<u32>>}
pub struct PreparedPopulation {base:std::sync::Arc<crate::foliage_population::Population>,update:crate::foliage_population::Update,changes:Vec<RootChange>,prepare_cpu_ms:f64}
impl PopulationSeed {
    pub fn prepare(self,roots:&[crate::foliage_population::WorldRoot])->Result<PreparedPopulation,String> {
        let started=web_time::Instant::now();
        if roots.iter().any(|r|r.placement.variant>=self.templates.len()){return Err("unknown forest species".into());}
        let update=self.base.plan(roots)?;
        let parked=RootPlacement {up:Vec3::Y,pos:Vec3::new(0.,-100000.,0.),yaw:0.,scale:1.,variant:0};
        let changes=update.changed.iter().map(|&slot| {
            let p=update.next.slots[slot].map(|r|r.placement).unwrap_or(parked);
            let mut change=relocated(self.templates[p.variant],p,self.owners[slot],update.next.generations[slot],update.next.slots[slot].is_none());change.slot=slot as u32;change
        }).collect();
        Ok(PreparedPopulation {base:self.base,update,changes,prepare_cpu_ms:started.elapsed().as_secs_f64()*1000.})
    }
}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
pub(crate) struct RootChange {pub slot:u32,pad:[u32;3],pub root:Instance,pub proxy:GpuTree,pub source:GpuInstance}
fn relocated(template:RootTemplate,placement:RootPlacement,owner:u32,generation:u32,inactive:bool)->RootChange {
    let m=transform(placement);let mut proxy=template.proxy;let mut source=template.source;
    proxy.inverse=rows(matrix(proxy.inverse)*m.inverse());
    let mut low=Vec3::splat(f32::INFINITY);let mut high=-low;
    for i in 0..8 {let p=m.transform_point3(Vec3::new(
        if i&1==0{template.proxy.bounds_min[0]}else{template.proxy.bounds_max[0]},
        if i&2==0{template.proxy.bounds_min[1]}else{template.proxy.bounds_max[1]},
        if i&4==0{template.proxy.bounds_min[2]}else{template.proxy.bounds_max[2]}));low=low.min(p);high=high.max(p);}
    proxy.bounds_min=low.extend(if inactive{1.}else{0.}).to_array();proxy.bounds_max=high.extend(0.).to_array();proxy.meta[0]=owner;
    source.rows=rows(m*matrix(source.rows));source.sphere=m.transform_point3(Vec3::from_slice(&source.sphere)).extend(source.sphere[3]*placement.scale).to_array();
    source.scale*=placement.scale;source.flags|=PART_WIND;
    if inactive {source.flags=PART_WIND;source.sphere[3]=-1.;}
    let row=(0..3).map(|r|m.row(r).truncate().abs().element_sum()).fold(0.,f32::max);
    let col=(0..3).map(|c|m.col(c).truncate().abs().element_sum()).fold(0.,f32::max);
    RootChange {slot:0,pad:[0;3],proxy,source,root:Instance {rows:rows(m),scale:[(row*col).sqrt(),0.,template.proxy_work,template.proxy_work],info:[template.node,generation,template.branch,u32::from(inactive)]}}
}
/// Same conservative animated root envelope used by GPU hierarchy selection.
/// Used only when the world population changes, never for per-frame selection.
pub(crate) fn root_shadow_sphere(root:Instance,local:[f32;4])->Option<[f32;4]> {
    if root.info[3]!=0 {return None;}
    let centre=matrix(root.rows).transform_point3(Vec3::from_slice(&local));
    Some(centre.extend(local[3]*root.scale[0]+root.scale[1]).to_array())
}
fn profile_root_volume(volume:usize,slots:u32,trees:u32)->Option<(u32,u32)> {
    let index=volume.checked_sub(slots as usize)?;
    if trees==0 || index>=2*trees as usize {return None;}
    Some(((index%trees as usize)as u32,256u32<<((index/trees as usize)as u32)))
}
impl Runtime {
    /// The refiner's state buffer and the word that takes last frame's stored
    /// camera splat count (foliage_refine.wgsl fast_adapt).
    pub(crate) fn splat_feedback(&self)->(&wgpu::Buffer,u64) {(&self.selector.refiner.state,crate::foliage_refine::STATE_OFFSET+82*4)}
    pub(crate) fn root_shadow_bound(&self,root:Instance)->Option<[f32;4]> {
        self.templates.iter().find(|t|t.node==root.info[0]).and_then(|t|root_shadow_sphere(root,t.shadow_sphere))
    }
    pub(crate) fn asset_selector(&self)->&Selector {&self.selector}
    pub fn new(gpu:&Gpu,spec:&Spec,instances:&wgpu::Buffer,volumes:&wgpu::Buffer)->Result<Self,String> {
        Self::with_pose_capacity(gpu,spec,instances,volumes,None)
    }
    fn with_pose_capacity(gpu:&Gpu,spec:&Spec,instances:&wgpu::Buffer,volumes:&wgpu::Buffer,capacity_override:Option<u32>)->Result<Self,String> {
        if gpu.device.limits().max_storage_buffers_per_shader_stage<16 {return Err("GPU forest materialization requires 16 storage bindings".into());}
        let limits=gpu.device.limits();
        let raster_budget=crate::voxel_forest::bounded_splat_capacity(u64::from(spec.raster_budget),volumes.size()/96,
            u64::from(limits.max_storage_buffer_binding_size).min(limits.max_buffer_size))as u32;
        if raster_budget==0 {return Err("GPU forest has no splat capacity after per-tree headers".into());}
        let mut cooked_branches=spec.branches.clone();let mut max_depth=0u32;
        for i in 0..spec.branches.len() {
            let mut n=i;let mut depth=1;
            while spec.branches[n].info[0]!=NONE {
                let p=spec.branches[n].info[0]as usize;
                if p>=n || depth>=64 {return Err("GPU forest branch ancestry is not bounded parent-first".into());}
                n=p;depth+=1;
            }
            cooked_branches[i].info[2]=depth-1;max_depth=max_depth.max(depth-1);
        }
        if (spec.roots.len()as u64)*(spec.branches.len()as u64)>=u32::MAX as u64 {return Err("shared pose identity exceeds32 bits".into());}
        let pose_capacity=spec.slots.checked_mul(max_depth.max(1)).and_then(|n|n.checked_mul(2)).and_then(u32::checked_next_power_of_two).ok_or("pose cache size overflow")?;
        let pose_capacity=capacity_override.unwrap_or(pose_capacity);
        if !pose_capacity.is_power_of_two() {return Err("pose cache capacity must be a power of two".into());}
        for size in [pose_capacity as u64*64,(128+spec.slots as u64*(max_depth as u64+1))*4,spec.roots.len()as u64*48] {if size>gpu.device.limits().max_storage_buffer_binding_size as u64 {return Err("shared pose cache exceeds device storage limit".into());}}
        if (spec.slots as u64+spec.roots.len()as u64).div_ceil(64)>gpu.device.limits().max_compute_workgroups_per_dimension as u64 {
            return Err("GPU forest materialization exceeds dispatch capacity".into());
        }
        let records=spec.slots.saturating_mul(4).max(spec.roots.len()as u32).max(1024);
        let selector=Selector::with_source_tables(gpu,&spec.nodes,&spec.source_tables,&spec.roots,records,records)?;
        selector.set_source_cluster_budget(spec.source_cluster_budget)?;
        let residency=Residency::new(gpu,&selector,spec.slots)?;
        crate::shaders::checked(gpu,|| {
            let upload=|label,data:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some(label),contents:data,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST});
            let uniform=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("GPU forest pose state"),size:32,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            let choices=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("GPU forest shared representation choices"),size:u64::from(spec.slots)*8,
                usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
            let root_history=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("GPU forest root representation history"),size:spec.roots.len()as u64*32,usage:wgpu::BufferUsages::STORAGE,mapped_at_creation:false});
            let branches=upload("shared forest branch rigs",bytemuck::cast_slice(&cooked_branches));
            let reps=upload("shared forest representations",bytemuck::cast_slice(&spec.representations));
            let proxies=upload("resident forest roots",bytemuck::cast_slice(&spec.proxies));
            let scratch=|label,size|gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
            let pose_cache=scratch("shared branch pose cache",pose_capacity as u64*64);
            let pose_work=scratch("shared pose depth queues",(128+spec.slots as u64*(max_depth as u64+1))*4);
            let pose_args=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("GPU pose indirect dispatches"),size:64*12,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::INDIRECT|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
            let dispatch_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("pose dispatch preparation"),entries:&(0..3).map(|binding|wgpu::BindGroupLayoutEntry {
                binding,visibility:wgpu::ShaderStages::COMPUTE,count:None,ty:wgpu::BindingType::Buffer {ty:if binding==2 {wgpu::BufferBindingType::Uniform}else{wgpu::BufferBindingType::Storage {read_only:binding==0}},has_dynamic_offset:false,min_binding_size:None},
            }).collect::<Vec<_>>()});
            let pose_dispatch_bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("pose dispatch preparation"),layout:&dispatch_layout,entries:&[&pose_work,&pose_args,&uniform].iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry {binding:i as u32,resource:b.as_entire_binding()}).collect::<Vec<_>>()});
            let dispatch_pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("pose dispatch preparation"),bind_group_layouts:&[Some(&dispatch_layout)],immediate_size:0});
            let dispatch_module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("pose dispatch preparation"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/foliage_pose_dispatch.wgsl").into())});
            let pose_dispatch=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("pose dispatch preparation"),layout:Some(&dispatch_pl),module:&dispatch_module,entry_point:Some("prepare"),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
            let root_poses=scratch("shared root poses",spec.roots.len()as u64*48);
            let mut levels=vec![0u32;(max_depth as usize+1)*64];for d in 0..=max_depth {levels[d as usize*64]=d;}
            let levels=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("immutable pose depth dispatch"),contents:bytemuck::cast_slice(&levels),usage:wgpu::BufferUsages::UNIFORM});
            let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("GPU forest materialization"),entries:&(0..18).map(|binding|wgpu::BindGroupLayoutEntry {
                binding,visibility:wgpu::ShaderStages::COMPUTE,count:None,ty:wgpu::BindingType::Buffer {ty:if binding==0 || binding==16 {wgpu::BufferBindingType::Uniform}else{wgpu::BufferBindingType::Storage {read_only:binding<8 || binding==11 || binding==17}},has_dynamic_offset:binding==16,min_binding_size:None},
            }).collect::<Vec<_>>()});
            let buffers=[&uniform,&selector._instances.buffer,&selector._nodes.buffer,&branches,&reps,&residency.slots,&selector.fallback,&proxies,instances,volumes,&choices,&selector.demand,&root_history,&pose_cache,&pose_work,&root_poses,&levels,&selector.asset_pages];
            let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("GPU forest materialization"),layout:&layout,entries:&buffers.iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry {binding:i as u32,resource:if i==16 {wgpu::BindingResource::Buffer(wgpu::BufferBinding {buffer:b,offset:0,size:std::num::NonZeroU64::new(16)})}else if i==1 {selector._instances.binding()}else if i==2 {selector._nodes.binding()}else{b.as_entire_binding()}}).collect::<Vec<_>>()});
            let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("GPU forest materialization"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
            let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("GPU forest materialization"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/foliage_materialize.wgsl").into())});
            let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("GPU forest materialization"),layout:Some(&pl),module:&module,entry_point:Some("materialize"),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
            let pose_pipelines=["prepare_root_poses","request_poses","evaluate_poses"].map(|entry|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(entry),layout:Some(&pl),module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()}));
            let empty_depth=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("GPU forest conservative depth"),size:wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::R32Float,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]}).create_view(&Default::default());
            let publish_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("forest population publish"),entries:&(0..4).map(|binding|wgpu::BindGroupLayoutEntry {binding,visibility:wgpu::ShaderStages::COMPUTE,count:None,ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Storage {read_only:binding==0},has_dynamic_offset:false,min_binding_size:None}}).collect::<Vec<_>>()});
            let publish_pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("forest population publish"),bind_group_layouts:&[Some(&publish_layout)],immediate_size:0});
            let publish_module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("forest population publish"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/foliage_publish.wgsl").into())});
            let publish_pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("forest population publish"),layout:Some(&publish_pl),module:&publish_module,entry_point:Some("publish"),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
            Self {pose_dispatch,pose_dispatch_bind,pose_args,pose_pipelines,pose_cache,pose_work,pose_depth:max_depth,branch_count:spec.branches.len()as u32,publish_pipeline,publish_layout,source_buffer:instances.clone(),population:std::sync::Mutex::new(std::sync::Arc::new(spec.population.clone())),templates:std::sync::Arc::new(spec.templates.clone()),proxy_owners:std::sync::Arc::new(spec.proxies.iter().map(|v|v.meta[0]).collect()),proxy_buffer:proxies,selector,residency,uniform,bind,pipeline,empty_depth,choices,slots:spec.slots,trees:spec.roots.len()as u32,error:spec.error,raster_budget,source_cluster_budget:spec.source_cluster_budget,wind:Default::default()}
        })
    }
    pub fn set_wind(&self,state:&crate::wind::WindState)->Result<(),String>{*self.wind.lock().unwrap()=state.validated()?;Ok(())}
    pub fn population_seed(&self)->PopulationSeed {
        PopulationSeed {base:self.population.lock().unwrap().clone(),templates:self.templates.clone(),owners:self.proxy_owners.clone()}
    }
    pub(crate) fn commit_population(&self,gpu:&Gpu,prepared:PreparedPopulation,apply:impl FnOnce(&[RootChange])->Result<(),String>)->Result<PopulationStats,String> {
        let mut population=self.population.lock().unwrap();
        if !std::sync::Arc::ptr_eq(&population,&prepared.base){return Err("stale GPU forest population transaction".into());}
        let PreparedPopulation {update,changes,prepare_cpu_ms,..}=prepared;
        let started=web_time::Instant::now();apply(&changes)?;let proxy_cpu_ms=started.elapsed().as_secs_f64()*1000.;
        let started=web_time::Instant::now();
        if !changes.is_empty() {
            let mut encoder=gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {label:Some("forest population publish")});
            // Bound each upload even when every reserved root changes at once.
            for batch in changes.chunks(16384) {
            let upload=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("forest population delta"),contents:bytemuck::cast_slice(batch),usage:wgpu::BufferUsages::STORAGE});
            let buffers=[&upload,&self.selector._instances.buffer,&self.proxy_buffer,&self.source_buffer];
            let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("forest population publish"),layout:&self.publish_layout,entries:&buffers.iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry {binding:i as u32,resource:if i==1 {self.selector._instances.binding()}else{b.as_entire_binding()}}).collect::<Vec<_>>()});
            {let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("forest population publish"),timestamp_writes:None});
                pass.set_pipeline(&self.publish_pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups((batch.len()as u32).div_ceil(64),1,1);}
            }
            gpu.queue.submit(Some(encoder.finish()));
        }
        let stats=PopulationStats {active:update.next.active(),capacity:population.slots.len(),changed:changes.len(),retained:update.retained,
            upload_bytes:changes.len()*std::mem::size_of::<RootChange>(),prepare_cpu_ms,proxy_cpu_ms,root_upload_cpu_ms:started.elapsed().as_secs_f64()*1000.};
        *population=std::sync::Arc::new(update.next);Ok(stats)
    }
    /// Explicit synchronous diagnostic only; never called to select or draw.
    /// Explicit offline attribution of submitted cells to the published GPU cut.
    /// Never used to schedule rendering or choose instances on the CPU.
    pub(crate) fn annotate_raster_profile(&self,gpu:&Gpu,profiles:&mut [serde_json::Value])->Result<(),String> {
        fn read(gpu:&Gpu,source:&wgpu::Buffer)->Result<Vec<u8>,String> {
            let out=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("offline foliage ownership"),size:source.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(source,0,&out,0,source.size());gpu.queue.submit([enc.finish()]);
            let(tx,rx)=std::sync::mpsc::sync_channel(1);out.slice(..).map_async(wgpu::MapMode::Read,move|r|{let _=tx.send(r);});gpu.wait();rx.recv().map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
            let data=out.slice(..).get_mapped_range().map_err(|e|e.to_string())?;Ok(data.to_vec())
        }
        let slot_bytes=read(gpu,&self.residency.slots)?;let node_bytes:Vec<u8>=bytemuck::cast_slice(&self.selector._nodes.read(gpu)).to_vec();let page_bytes=read(gpu,&self.selector.asset_pages)?;
        let root_bytes:Vec<u8>=bytemuck::cast_slice(&self.selector._instances.read(gpu)).to_vec();let publication_bytes=read(gpu,&self.selector.demand)?;let fallback_bytes=read(gpu,&self.selector.fallback)?;
        let slots:Vec<crate::foliage_residency::ResidentSlot>=slot_bytes.chunks_exact(32).map(bytemuck::pod_read_unaligned).collect();
        let nodes:Vec<crate::foliage_select::Node>=node_bytes.chunks_exact(48).map(bytemuck::pod_read_unaligned).collect();
        let pages:Vec<[u32;4]>=page_bytes.chunks_exact(16).map(bytemuck::pod_read_unaligned).collect();
        let roots:Vec<Instance>=root_bytes.chunks_exact(std::mem::size_of::<Instance>()).map(bytemuck::pod_read_unaligned).collect();
        let publications:Vec<[u32;4]>=publication_bytes.chunks_exact(16).map(bytemuck::pod_read_unaligned).collect();
        let fallbacks:Vec<u32>=fallback_bytes.chunks_exact(4).map(bytemuck::pod_read_unaligned).collect();
        #[derive(Clone,Copy)]
        struct Owner {root:u32,node:u32,consumer:u32,generation:u32,history:[u32;4],whole:bool}
        let resolve=|volume:usize|->Result<Owner,String> {
            if volume<self.slots as usize {
                let slot=slots.get(volume).ok_or("offline cut slot out of range")?;
                return Ok(Owner {root:slot.key[0],node:slot.key[1],consumer:256,generation:slot.key[3],history:slot.history,whole:false});
            }
            let (root,consumer)=profile_root_volume(volume,self.slots,self.trees).ok_or("offline root volume out of range")?;
            let instance=roots.get(root as usize).ok_or("offline root instance missing")?;
            let index=root as usize+if consumer==512 {self.trees as usize}else{0};
            let publication=*publications.get(index).ok_or("offline consumer publication missing")?;
            let fault=*fallbacks.get(root as usize).ok_or("offline root fallback missing")?;
            let mut selected=crate::foliage_select::NONE;
            if fault&(consumer|7)==0 && publication[2]!=0 {
                let start=instance.info[0]as usize;let end=nodes.get(start).ok_or("offline recipe root missing")?.links[1]as usize;
                if end>nodes.len() || end<=start {return Err("offline recipe span invalid".into());}
                let representation=publication[2]-1;
                selected=(start..end).find(|&i|nodes[i].links[2]==representation).ok_or("published root representation missing from recipe")? as u32;
            }
            Ok(Owner {root,node:selected,consumer,generation:instance.info[1],history:publication,whole:true})
        };
        let mut tasks=Vec::new();let mut task_indices=std::collections::BTreeMap::new();
        for profile in profiles.iter() {if let Some(cells)=profile["largest_submitted_cells"].as_array(){for cell in cells {
            let owner=resolve(cell["volume"].as_u64().ok_or("missing profile volume")? as usize)?;
            let key=(owner.root,owner.node,owner.consumer);
            if let std::collections::btree_map::Entry::Vacant(entry)=task_indices.entry(key){entry.insert(tasks.len());tasks.push([key.0,key.1,key.2,0]);}
        }}}
        let traced=self.selector.refiner.trace_proposals(gpu,&self.selector,&tasks)?;
        let describe=|id:usize|->Result<serde_json::Value,String>{
            let n=nodes.get(id).ok_or("offline cut node out of range")?;
            let page=pages.get(n.links[2]as usize).ok_or("offline cut page out of range")?;
            Ok(serde_json::json!({"node":id,"sphere":n.sphere,"error_and_splat_cost":n.error,"links":n.links,"page":page}))
        };
        for profile in profiles {if let Some(cells)=profile["largest_submitted_cells"].as_array_mut(){for cell in cells {
            let owner=resolve(cell["volume"].as_u64().ok_or("missing profile volume")? as usize)?;
            let mut children=Vec::new();let mut selected=serde_json::Value::Null;
            if owner.node!=crate::foliage_select::NONE {
                let node=nodes.get(owner.node as usize).ok_or("offline cut owner node out of range")?;
                selected=describe(owner.node as usize)?;let mut child=node.links[0];
                while child!=crate::foliage_select::NONE && child<node.links[1] {
                    children.push(describe(child as usize)?);let next=nodes[child as usize].links[1];if next<=child{return Err("offline cut invalid child span".into());}child=next;
                }
            }
            let trace=traced[task_indices[&(owner.root,owner.node,owner.consumer)]];
            cell["cut_owner"]=serde_json::json!({"kind":if owner.whole {"whole_root"}else{"articulated_slot"},"root":owner.root,
                "consumer":owner.consumer,"generation":owner.generation,"history":if owner.whole {None}else{Some(owner.history)},
                "publication":if owner.whole {Some(owner.history)}else{None},"fallback_flags":fallbacks[owner.root as usize],
                "root_recipe":roots[owner.root as usize].info[0],"selected":selected,"immediate_children":children,
                "final_budget_proposal_replay":{"scope":"actual GPU propose kernel after unary skipping, final-frame budget; not historical admission",
                "old":trace[0],"replacement":trace[1],"control":trace[3]}});
        }}}
        Ok(())
    }
    /// Bytes in one consumer-class choice table; camera then shadow.
    pub(crate) fn choice_table_bytes(&self)->u64 {u64::from(self.slots)*4}
    pub fn diagnostics(&self,gpu:&Gpu)->Result<Diagnostics,String> {
        let inputs=[(&self.selector.counters,0,64u64),(&self.residency.counters,0,32),(&self.selector.fallback,0,self.trees as u64*4),(&self.choices,0,self.choices.size()),(&self.pose_work,0,268),(&self.selector.demand,0,self.trees as u64*32),(&self.selector.refiner.state,crate::foliage_refine::STATE_OFFSET,384)];
        let size=inputs.iter().map(|(_,_,n)|n).sum();
        let output=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("forest diagnostic snapshot"),size,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut enc=gpu.device.create_command_encoder(&Default::default());let mut offset=0;
        self.selector.refiner.audit(gpu,&mut enc,&self.selector);
        for (buffer,from,len) in inputs {enc.copy_buffer_to_buffer(buffer,from,&output,offset,len);offset+=len;}
        let submission=gpu.queue.submit([enc.finish()]);let(tx,rx)=std::sync::mpsc::sync_channel(1);
        output.slice(..).map_async(wgpu::MapMode::Read,move|r|{let _=tx.send(r);});
        gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(30))}).map_err(|e|e.to_string())?;
        rx.recv().map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
        let bytes=output.slice(..).get_mapped_range().map_err(|e|e.to_string())?;
        let words:Vec<u32>=bytes.chunks_exact(4).map(bytemuck::pod_read_unaligned).collect();
        let c=&words[..16];let r=&words[16..24];let fallback=&words[24..24+self.trees as usize];
        let choices=&words[24+self.trees as usize..24+self.trees as usize+2*self.slots as usize];
        let pose_start=24+self.trees as usize+2*self.slots as usize;let poses=&words[pose_start..pose_start+67];
        let demand=&words[pose_start+67..pose_start+67+self.trees as usize*8];
        let refinement=&words[pose_start+67+self.trees as usize*8..];
        let (mut camera_roots,mut shadow_only_roots,mut camera_fallback_roots,mut shadow_only_fallback_roots)=(0,0,0,0);
        let (mut camera_published_branches,mut shadow_only_published_branches)=(0u64,0u64);
        for (i,(d,&fallback)) in demand.chunks_exact(4).zip(fallback).enumerate() {
            if d[1]&256!=0 {camera_roots+=1;camera_published_branches+=d[0]as u64;camera_fallback_roots+=u32::from(fallback&(256|7)!=0);}
            else if d[1]&512!=0 {shadow_only_roots+=1;shadow_only_published_branches+=demand[(self.trees as usize+i)*4]as u64;shadow_only_fallback_roots+=u32::from(fallback&(512|7)!=0);}
        }
        let voxels=choices[..self.slots as usize].iter().filter(|w|**w&3==1).count()as u32;
        let shadow_voxel_slots=choices[self.slots as usize..2*self.slots as usize].iter().filter(|w|**w&3==1).count()as u32;
        let camera_disabled=choices[..self.slots as usize].iter().filter(|w|**w&3==3).count()as u32;
        Ok(Diagnostics {reserved_source_clusters:refinement[69],source_cluster_budget_stops:refinement[72],camera_refinement_limits:CameraRefinementLimits {over_target:refinement[48],branch_slots:refinement[49],camera_splats:refinement[50],shadow_splats:refinement[51],missing_pages:refinement[52],no_children:refinement[53],affordable_resident:refinement[54]},selected_source_consumers:ConsumerCounts::from_words(&refinement[32..36]),selected_voxel_consumers:ConsumerCounts::from_words(&refinement[36..40]),selected_direct_consumers:ConsumerCounts::from_words(&refinement[40..44]),selected_spatial_error_pixels:[f32::from_bits(refinement[19]),f32::from_bits(refinement[20])],selected_error_target_ratio:[f32::from_bits(refinement[21]),f32::from_bits(refinement[22])],selected_over_target:[refinement[23],refinement[24]],error_unknown_proxies:refinement[25],error_unmatched_direct:refinement[26],error_forced_nodes:refinement[27],reserved_raster_splats:[refinement[1],refinement[2]],refinement_budget_stops:refinement[17],refinement_faults:refinement[15],error_cut_scale:[79,78].map(|w|{let v=f32::from_bits(refinement[w]);if v>=1. {v} else {1.}}),source_cut_scale:{let v=f32::from_bits(refinement[85]);if v>=1. {v} else {1.}},error_cut_fill:[f32::from_bits(refinement[74]),f32::from_bits(refinement[77])],error_cut_page_blocked:refinement[73],reused_refinement_proposals:refinement[28],dormant_records:refinement[29],dormant_wakes:refinement[60],dormant_wake_passes:refinement[62],dormant_park_events:refinement[61],dormant_overflows:refinement[63],camera_roots,shadow_only_roots,camera_fallback_roots,shadow_only_fallback_roots,camera_published_branches,shadow_only_published_branches,trees:self.trees,active_roots:self.population.lock().unwrap().active(),branch_capacity:self.slots,selected_records:c[0],tested_nodes:c[4],source_slots:r[6].saturating_sub(voxels+camera_disabled),voxel_slots:voxels,shadow_voxel_slots,direct_roots:c[7],
            fallback_trees:fallback.iter().filter(|f|**f!=0).count()as u32,retained:r[2],assigned:r[3],slot_failures:r[4],hash_failures:r[5],frontier_overflow_requests:c[11],peak_frontier_requests:c[15],pose_hash_failures:poses[64],pose_requests:poses[65],evaluated_shared_poses:poses[66]})
    }
    pub fn encode(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,views:&[crate::part_wind::AnimationView],first_owner:u32)->Result<(),String> {
        let consumers=views.iter().copied().map(crate::foliage_select::ConsumerView::camera).collect::<Vec<_>>();
        self.encode_timed(gpu,enc,&consumers,first_owner,None)
    }
    pub(crate) fn encode_timed(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,views:&[crate::foliage_select::ConsumerView],first_owner:u32,time:Option<(&wgpu::QuerySet,u32)>)->Result<(),String> {
        self.encode_timed_depth(gpu,enc,views,first_owner,time,None)
    }
    pub(crate) fn note_cameras(&self,cameras:&[(Vec3,Vec3,f32)]) {self.selector.refiner.note_cameras(cameras);}
    #[cfg(test)]
    pub(crate) fn test_rounds(&self,full:bool,hint:Option<u32>)->(u32,Option<u32>,u32) {let r=&self.selector.refiner;if full {r.test_full_rounds();}if let Some(h)=hint {r.test_hint(h);}let (e,u)=r.encoded_rounds();(e,u,2*(self.selector.levels+1))}
    pub(crate) fn encode_timed_depth(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,views:&[crate::foliage_select::ConsumerView],first_owner:u32,time:Option<(&wgpu::QuerySet,u32)>,depth:Option<&[crate::foliage_seed_visibility::CameraDepth<'_>]>)->Result<(),String> {
        self.encode_split(gpu,enc,views,first_owner,time,depth,None)
    }
    /// `split`: eight more encoder timestamps (seed classify, refine,
    /// compact/finish, residency) inside "GPU forest selection residency".
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_split(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,views:&[crate::foliage_select::ConsumerView],first_owner:u32,time:Option<(&wgpu::QuerySet,u32)>,depth:Option<&[crate::foliage_seed_visibility::CameraDepth<'_>]>,split:Option<(&wgpu::QuerySet,u32)>)->Result<(),String> {
        let split=split.filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS));
        // All of these are encoder writes: untimed where the GPU lacks them
        // (Apple), or Metal rejects the submit.
        let time=time.filter(|_|gpu.features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS));
        if let Some((q,i))=time {enc.write_timestamp(q,i);}
        // The machine's vegetation tier tightens the cut's budgets (never the
        // buffers): it can change between frames when the app steps it down.
        let (error,slots,splats,source)=crate::vegetation_quality::apply(crate::vegetation_quality::budgets(gpu),self.error,self.slots,self.raster_budget,self.source_cluster_budget);
        self.selector.set_source_cluster_budget(source)?;
        self.selector.union_consumers_marked(gpu,enc,views,error,slots,[splats;2],&self.empty_depth,depth,split)?;
        if let Some((q,i))=split {enc.write_timestamp(q,i+6);}
        self.residency.encode(enc);
        if let Some((q,i))=split {enc.write_timestamp(q,i+7);}
        if let Some((q,i))=time {enc.write_timestamp(q,i+1);enc.write_timestamp(q,i+2);}
        let wind=*self.wind.lock().unwrap();
        let field=[wind.direction.x,wind.direction.y,wind.strength,(wind.time_seconds.rem_euclid(120.)/120.*std::f64::consts::TAU)as f32];
        gpu.queue.write_buffer(&self.uniform,0,bytemuck::cast_slice(&field));
        gpu.queue.write_buffer(&self.uniform,16,bytemuck::cast_slice(&[self.slots,first_owner,self.trees,self.branch_count]));
        enc.clear_buffer(&self.pose_cache,0,None);enc.clear_buffer(&self.pose_work,0,Some(512));
        let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,&self.bind,&[0]);
        pass.set_pipeline(&self.pose_pipelines[0]);pass.dispatch_workgroups(self.trees.div_ceil(64),1,1);
        pass.set_pipeline(&self.pose_pipelines[1]);pass.dispatch_workgroups(self.slots.div_ceil(64),1,1);
        drop(pass);
        let mut pass=enc.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.pose_dispatch);pass.set_bind_group(0,&self.pose_dispatch_bind,&[]);pass.dispatch_workgroups(1,1,1);
        drop(pass);
        let mut pass=enc.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.pose_pipelines[2]);
        for depth in 1..=self.pose_depth {pass.set_bind_group(0,&self.bind,&[depth*256]);pass.dispatch_workgroups_indirect(&self.pose_args,depth as u64*12);}
        drop(pass);
        if let Some((q,i))=time {enc.write_timestamp(q,i+3);enc.write_timestamp(q,i+4);}
        let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,&self.bind,&[0]);pass.set_pipeline(&self.pipeline);
        pass.dispatch_workgroups((self.slots+self.trees).div_ceil(64),1,1);drop(pass);
        if let Some((q,i))=time {enc.write_timestamp(q,i+5);}
        Ok(())
    }
}

#[cfg(test)] mod tests {
    #[test]
    fn raster_work_covers_consumer_mip_ranges_and_root_fallbacks() {
        use super::*;
        let mut branch=Branch::zeroed();branch.volume.meta=[0,1,3,0];branch.info=[0,1,0,0];
        // Non-monotonic occupancy intentionally prevents assuming mip zero is largest.
        let counts=[999,7,13,2];
        assert_eq!(representation_raster_work([0,1,0,0],&branch,&counts).unwrap(),[7,13]);
        assert_eq!(representation_raster_work([0,1,2,1],&branch,&counts).unwrap(),[2,13],"the camera books exactly its selected mip");
        assert_eq!(representation_raster_work([0,1,1,0],&branch,&counts).unwrap(),[13,13]);
        assert_eq!(representation_raster_work([0,0,0,0],&branch,&counts).unwrap(),[0,0]);
        branch.info[0]=NONE;
        assert_eq!(representation_raster_work([0,1,2,0],&branch,&counts).unwrap(),[2,2]);
        // A geometry group may also have parent NONE: its camera draws the selected
        // mip, its shadow still an adaptive bundle.
        assert_eq!(representation_raster_work([0,1,2,1],&branch,&counts).unwrap(),[2,13]);
        branch.info[1]=0;
        assert_eq!(representation_raster_work([0,0,0,0],&branch,&counts).unwrap(),[0,0]);
        assert!(representation_raster_work([0,1,3,0],&branch,&counts).is_err());
        assert!(exact_work((1<<24)+1).is_err());assert_eq!(exact_work(1<<24).unwrap(),16777216.);
    }
    fn basal_reference(mut m:glam::Mat4,direction:glam::Vec2,angle:f32)->glam::Mat4 {
        let height=m.y_axis.truncate();let n=height.normalize();
        let wind=glam::Vec3::new(direction.x,0.,direction.y);
        m.y_axis+=(wind-n*wind.dot(n)).extend(0.)*(angle.sin()*height.length());m
    }
    #[test]
    fn basal_deformation_preserves_feet_optical_volume_and_motion_bounds() {
        use glam::{Vec2,Vec3,Mat4,Quat};
        for up in [Vec3::Y,Vec3::new(0.4,1.,-0.3).normalize()] {
            let rest=Mat4::from_scale_rotation_translation(Vec3::splat(1.3),Quat::from_rotation_arc(Vec3::Y,up)*Quat::from_rotation_y(0.7),Vec3::new(3.,2.,-1.));
            for direction in [Vec2::X,Vec2::Y,Vec2::new(1.,1.).normalize()] {
                for angle in [-0.22f32,-0.1,0.,0.1,0.22] {
                    let moved=basal_reference(rest,direction,angle);
                    assert!((moved.determinant()-rest.determinant()).abs()<1e-5,"shear must not change optical volume");
                    for p in [Vec3::ZERO,Vec3::X*0.7,Vec3::new(-0.3,0.,0.8)] {
                        assert!(moved.transform_point3(p).abs_diff_eq(rest.transform_point3(p),1e-6));
                    }
                    for p in [Vec3::Y,Vec3::new(0.3,0.7,-0.4)] {
                        let displacement=moved.transform_point3(p).distance(rest.transform_point3(p));
                        assert!(displacement<=2.*(0.22f32/2.).sin()*p.length()*1.3+1e-5,"existing rotational envelope must contain shear");
                        assert!(moved.inverse().transform_point3(moved.transform_point3(p)).abs_diff_eq(p,1e-5));
                        assert!(moved.transform_vector3(p).length()<=1.3*(1.+0.22f32.sin())*p.length()+1e-5);
                    }
                    let n=moved.inverse().transpose().transform_vector3(Vec3::Z).normalize();
                    assert!(n.dot(moved.transform_vector3(Vec3::X)).abs()<1e-5);
                    assert!(n.dot(moved.transform_vector3(Vec3::Y)).abs()<1e-5);
                }
            }
        }
    }
    #[test]
    fn nested_basal_deformation_is_rejected_before_scene_mutation() {
        use super::*;
        let mut source=crate::vegetation_asset::tests::aggregate_fixture();source.branches[2].parent=Some(1);
        let config=crate::vegetation_asset::tests::aggregate_config();
        let mut plant=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();plant.cook_proxy(&source,config,3).unwrap();
        let mut scene=Scene::default();let world=[crate::foliage_population::WorldRoot {id:1,placement:RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0}}];
        let err=plant_species_population(&mut scene,&[Species {plant:&plant,category:crate::scene::GeometryCategory::Grass,deformation:Deformation::BasalShear}],&world,2,8,Default::default()).unwrap_err();
        assert!(err.contains("direct child"));assert!(scene.instances.is_empty());assert!(scene.gpu_forest.is_none());assert!(scene.voxel_forest.is_none());
    }

    #[test]
    fn root_detail_pages_are_evictable_and_fallback_pins_are_bounded() {
        use super::*;
        let source=crate::vegetation_asset::tests::aggregate_fixture();
        let config=crate::vegetation_asset::tests::aggregate_config();
        let mut plant=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();plant.cook_proxy(&source,config,3).unwrap();
        let world=[crate::foliage_population::WorldRoot {id:1,placement:RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0}}];
        let mut scene=Scene::default();plant_population(&mut scene,&[plant],&world,4,8,crate::aggregate_lod::Policy {allow_sampled_optics:true,max_coverage_error:1.,max_color_error:1.,..Default::default()}).unwrap();
        let spec=scene.gpu_forest.as_ref().unwrap();let forest=scene.voxel_forest.as_ref().unwrap();
        let (deps,pins)=spec.voxel_page_dependencies(forest.levels.len()).unwrap();
        assert!(pins.iter().filter(|&&v|v).count()<=2,"one root fallback and one proxy fallback per species");
        for proxy in &spec.proxies {assert_eq!(proxy.meta[2],1);assert!(pins[proxy.meta[1]as usize]);}
        let root_rep=spec.nodes[spec.templates[0].node as usize].links[2]as usize;
        assert_eq!(deps[root_rep].len(),1);assert!(pins[deps[root_rep][0]]);
        let mut fine=0;
        for (i,rep) in spec.representations.iter().enumerate() {
            let branch=&spec.branches[if rep[3]==0 {rep[0]}else{rep[3]-1}as usize];
            if rep[1]==1 && rep[3]==0 && branch.info[0]==NONE {
                assert_eq!(deps[i],vec![(branch.volume.meta[1]+rep[2])as usize]);
                if !pins[deps[i][0]] {fine+=1;}
            }else if rep[1]==0 {assert!(deps[i].is_empty(),"source geometry must not depend on voxel pages for shadows");}
            else {
                assert_eq!(deps[i],((branch.volume.meta[1]+rep[2])as usize..(branch.volume.meta[1]+branch.volume.meta[2])as usize).collect::<Vec<_>>(),"a voxel selection waits for its mip and the coarser ones only");
            }
        }
        let work=forest.raster_work_bounds().unwrap();
        for node in &spec.nodes {
            let rep=spec.representations[node.links[2]as usize];
            let branch=&spec.branches[if rep[3]==0 {rep[0]}else{rep[3]-1}as usize];
            let expected=representation_raster_work(rep,branch,&work).unwrap();
            assert_eq!([node.error[2],node.error[3]],expected.map(|v|v as f32));
        }
        for (root,proxy) in spec.roots.iter().zip(&spec.proxies) {
            let expected=work[proxy.meta[1]as usize]as f32;
            assert_eq!([root.scale[2],root.scale[3]],[expected,expected]);
        }
        assert!(fine>0,"detail must not all be pinned");
    }
    #[test]
    fn tilted_cell_initial_and_streamed_placements_match() {
        use super::*;
        let cooked=crate::recipe_forest::cook_meadow_cells(&[(47,0.2)],None).unwrap();
        assert_eq!(cooked[0].plant.stats().placements,1080);
        assert_eq!(cooked[0].plant.stats().branches,37);
        let p=RootPlacement {pos:Vec3::new(3.,2.,-4.),yaw:0.8,scale:1.1,variant:0,up:Vec3::new(-0.3,1.,0.2).normalize()};
        let world=[crate::foliage_population::WorldRoot {id:1,placement:p}];
        let species=[Species {plant:&cooked[0].plant,category:crate::scene::GeometryCategory::Grass,deformation:crate::foliage_runtime::Deformation::BasalShear}];
        let mut scene=Scene::default();plant_species_population(&mut scene,&species,&world,2,64,crate::aggregate_lod::Policy {allow_sampled_optics:true,..Default::default()}).unwrap();
        let spec=scene.gpu_forest.as_ref().unwrap();
        let m=matrix(spec.roots[0].rows);
        assert!(m.transform_vector3(Vec3::Y).normalize().abs_diff_eq(p.up,1e-5));
        let relocated=relocated(spec.templates[0],p,spec.proxies[0].meta[0],spec.roots[0].info[1],false);
        assert!(matrix(relocated.root.rows).abs_diff_eq(m,1e-6));
        assert!(spec.roots[0].scale[2]>0.);
        assert_eq!(relocated.root.scale,spec.roots[0].scale);
        assert!(matrix(relocated.proxy.inverse).abs_diff_eq(matrix(spec.proxies[0].inverse),1e-5));
        for (a,b) in relocated.proxy.bounds_min.into_iter().zip(spec.proxies[0].bounds_min) {assert!((a-b).abs()<1e-4);}
        for (a,b) in relocated.proxy.bounds_max.into_iter().zip(spec.proxies[0].bounds_max) {assert!((a-b).abs()<1e-4);}
    }

    #[test]
    fn mixed_species_share_catalog_and_preserve_categories_on_replacement() {
        use super::*;
        use crate::scene::GeometryCategory::{Trees,Grass};
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut plant=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();plant.cook_proxy(&source,config,3).unwrap();
        let placement=RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0};
        let mut world=vec![crate::foliage_population::WorldRoot {id:10,placement},crate::foliage_population::WorldRoot {id:20,placement:RootPlacement {up:Vec3::Y,pos:Vec3::X,variant:1,..placement}}];
        let mut reference=Scene::default();plant_population(&mut reference,std::slice::from_ref(&plant),&world[..1],4,8,Default::default()).unwrap();
        let mut scene=Scene::default();
        plant_species_population(&mut scene,&[Species {plant:&plant,category:Trees,deformation:Deformation::Rigid},Species {plant:&plant,category:Grass,deformation:Deformation::Rigid}],&world,4,8,Default::default()).unwrap();
        assert_eq!(scene.meshes.len(),reference.meshes.len(),"categories must not duplicate mesh assets");
        let spec=scene.gpu_forest.as_ref().unwrap();assert_eq!(spec.roots.len(),4);assert_eq!(spec.slots,8);
        for (species,category) in [Trees,Grass].into_iter().enumerate() {
            let t=spec.templates[species];assert_eq!(t.source.category(),category);
            for branch in spec.branches.iter().skip(t.branch as usize).take(3) {
                if branch.source.first_cluster==0 {continue;}
                assert_eq!(branch.source.category(),category);
                let first=(branch.source.first_cluster>>12)as usize;let count=(branch.source.first_cluster&4095)as usize;
                for member in &scene.instances[first..first+count] {assert_eq!(member.category(),category,"shared member templates must not inherit another category");}
            }
        }
        let seed=PopulationSeed {base:std::sync::Arc::new(spec.population.clone()),templates:std::sync::Arc::new(spec.templates.clone()),owners:std::sync::Arc::new(spec.proxies.iter().map(|p|p.meta[0]).collect())};
        world[0].placement.variant=1;
        let prepared=seed.prepare(&world).unwrap();assert_eq!(prepared.changes.len(),1);
        let change=prepared.changes[0];assert_eq!(change.source.category(),Grass);
        assert_eq!(change.root.info[0],spec.templates[1].node);
        assert_ne!(change.root.info[1],spec.roots[change.slot as usize].info[1],"species replacement invalidates residency generation");
    }
    #[test]
    fn meadow_ribbons_attach_to_common_gpu_population_contract() {
        use super::*;
        let cooked=crate::recipe_forest::cook_meadow_variants(&[(47,0.2)],None).unwrap();
        let plant=&cooked[0].plant;
        // Three ribbon shapes with three shared palette bindings, not one mesh per blade.
        assert_eq!(plant.stats().primitive_meshes,9);assert_eq!(plant.stats().placements,120);
        let world=[crate::foliage_population::WorldRoot {id:1,placement:RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.4,scale:1.,variant:0}}];
        let mut scene=Scene::default();plant_species_population(&mut scene,&[Species {plant,category:crate::scene::GeometryCategory::Grass,deformation:crate::foliage_runtime::Deformation::BasalShear}],&world,4,16,crate::aggregate_lod::Policy {allow_sampled_optics:true,..Default::default()}).unwrap();
        let spec=scene.gpu_forest.as_ref().unwrap();
        assert!(scene.meadow.is_none());assert!(scene.part_wind.is_empty());
        assert!(spec.branches.iter().any(|b|b.animation[0]>0.));
        assert!(spec.nodes.iter().any(|n|n.links[3]&2!=0),"grass needs the same pinned root fallback");
        assert!(spec.source_page_instances().iter().any(Option::is_some));
        assert!(spec.representations.iter().any(|r|r[1]==1));
        assert_eq!(scene.instances[spec.proxies[0].meta[0]as usize].category(),crate::scene::GeometryCategory::Grass);
    }

    #[test] fn materialize_shader_validates(){for src in [include_str!("shaders/foliage_materialize.wgsl"),include_str!("shaders/foliage_pose_dispatch.wgsl")] {
        let module=naga::front::wgsl::parse_str(src).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));}}

    #[test]
    #[ignore="exclusive GPU root consumer publication oracle"]
    fn gpu_root_ancestors_and_fallback_preserve_consumers() {
        use super::*;
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();cooked.cook_proxy(&source,config,3).unwrap();
        let mut scene=Scene::default();plant(&mut scene,&[cooked],&[RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0}],8,
            crate::aggregate_lod::Policy {allow_sampled_optics:true,max_color_error:1.,max_coverage_error:1.,max_error_pixels:10000.,..Default::default()}).unwrap();
        let spec=scene.gpu_forest.as_ref().unwrap();let gpu=Gpu::new().unwrap();
        let upload=|bytes:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:None,contents:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC});
        let read=|buffer:&wgpu::Buffer| {
            let out=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:buffer.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(buffer,0,&out,0,buffer.size());gpu.queue.submit([enc.finish()]);out.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
            let bytes=out.slice(..).get_mapped_range().unwrap().to_vec();out.unmap();bytes
        };
        let instances=upload(bytemuck::cast_slice(&scene.instances));let volumes=upload(bytemuck::cast_slice(&scene.voxel_forest.as_ref().unwrap().instances));
        let runtime=Runtime::new(&gpu,spec,&instances,&volumes).unwrap();
        let view=crate::part_wind::AnimationView {view_projection:Mat4::perspective_infinite_reverse_rh(1.,1.,0.05)*Mat4::look_at_rh(Vec3::new(0.3,0.4,20.),Vec3::new(0.3,0.4,0.),Vec3::Y),viewport:[128;2]};
        let owner=spec.proxies[0].meta[0]as usize;
        for fallback in [false,true] {
            let mut previous:Option<(u32,u32)>=None;
            for roles in [512u32,768,768,256,768,512] {
                let mut consumers=Vec::new();
                if roles&256!=0 {consumers.push(crate::foliage_select::ConsumerView::camera(view));}
                if roles&512!=0 {consumers.push(crate::foliage_select::ConsumerView::shadow(view));}
                let mut enc=gpu.device.create_command_encoder(&Default::default());runtime.encode_timed(&gpu,&mut enc,&consumers,spec.first_owner,None).unwrap();gpu.queue.submit([enc.finish()]);
                let direct=runtime.selector.direct.read(&gpu);assert_ne!(direct[0],0,"fixture must publish a complete resident ancestor");
                if fallback {
                    // Exercise the actual fallback materializer without changing consumer demand.
                    gpu.queue.write_buffer(&runtime.selector.fallback,0,bytemuck::cast_slice(&[2u32]));
                    gpu.queue.write_buffer(&runtime.selector.demand,0,bytemuck::cast_slice(&[0u32,roles,0,0]));
                    let mut enc=gpu.device.create_command_encoder(&Default::default());
                    {let mut pass=enc.begin_compute_pass(&Default::default());pass.set_pipeline(&runtime.pipeline);pass.set_bind_group(0,&runtime.bind,&[0]);pass.dispatch_workgroups((spec.slots+spec.roots.len()as u32).div_ceil(64),1,1);}
                    gpu.queue.submit([enc.finish()]);
                }
                let raw=read(&volumes);let rendered:&[GpuTree]=bytemuck::cast_slice(&raw);
                assert_eq!(rendered[spec.slots as usize].bounds_min[3],if roles&256!=0 {3.}else{1.});
                assert_eq!(rendered[spec.slots as usize+spec.roots.len()].bounds_min[3],if roles&512!=0 {2.}else{1.});
                let raw=read(&instances);let rendered:&[GpuInstance]=bytemuck::cast_slice(&raw);let flags=rendered[owner].flags;
                assert_eq!(flags&768,0,"consumer bits cannot leak into per-view source hide flags");
                if !fallback {if let Some((old_roles,old_epoch))=previous {
                    assert_eq!(flags&128!=old_epoch,roles!=old_roles,"changing either root publication rejects shared owner history");
                }}
                previous=Some((roles,flags&128));
            }
        }
    }

    #[test]
    #[ignore="exclusive GPU shared ancestry oracle"]
    fn gpu_geometry_groups_use_existing_pose_and_distinct_volume_pages() {
        use super::*;
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut plant=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();plant.cook_proxy(&source,config,3).unwrap();
        let mut scene=Scene::default();
        plant_population(&mut scene,&[plant],&[crate::foliage_population::WorldRoot {id:1,placement:RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.3,scale:1.,variant:0}}],1,16,
            crate::aggregate_lod::Policy {allow_sampled_optics:true,max_coverage_error:1.,max_color_error:1.,..Default::default()}).unwrap();
        let mut spec=scene.gpu_forest.clone().unwrap();
        assert!(spec.representations.iter().any(|r|r[3]!=0));
        assert!(spec.representations.iter().all(|r|r[0]<3),"groups reuse the original three pose indices");
        for n in &mut spec.nodes {
            if n.links[0]==NONE {continue;}
            if spec.representations[n.links[2]as usize][3]!=0 {n.links[3]&=!1;n.error=[0.;4];}
            else {n.links[3]|=1;}
        }
        let gpu=Gpu::new().unwrap();
        let upload=|bytes:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("group oracle"),contents:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC});
        let read=|buffer:&wgpu::Buffer| {
            let target=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("group oracle read"),size:buffer.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            let mut e=gpu.device.create_command_encoder(&Default::default());e.copy_buffer_to_buffer(buffer,0,&target,0,buffer.size());gpu.queue.submit([e.finish()]);target.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
            let bytes=target.slice(..).get_mapped_range().unwrap().to_vec();target.unmap();bytes
        };
        let instances=upload(bytemuck::cast_slice(&scene.instances));let volumes=upload(bytemuck::cast_slice(&scene.voxel_forest.as_ref().unwrap().instances));
        let runtime=Runtime::new(&gpu,&spec,&instances,&volumes).unwrap();
        let view=crate::part_wind::AnimationView {view_projection:Mat4::perspective_infinite_reverse_rh(1.,1.,0.05)*Mat4::look_at_rh(Vec3::new(0.3,0.4,20.),Vec3::new(0.3,0.4,0.),Vec3::Y),viewport:[512;2]};
        for roles in [256u32,512,768] {
        for time in [0.,17.,119.] {
            runtime.set_wind(&crate::wind::WindState {strength:0.7,time_seconds:time,..Default::default()}.validated().unwrap()).unwrap();
            let mut enc=gpu.device.create_command_encoder(&Default::default());let mut consumers=Vec::new();
            if roles&256!=0 {consumers.push(crate::foliage_select::ConsumerView::camera(view));}
            if roles&512!=0 {consumers.push(crate::foliage_select::ConsumerView::shadow(view));}
            runtime.encode_timed(&gpu,&mut enc,&consumers,spec.first_owner,None).unwrap();gpu.queue.submit([enc.finish()]);
            let slot_bytes=read(&runtime.residency.slots);let slots:&[crate::foliage_residency::ResidentSlot]=bytemuck::cast_slice(&slot_bytes);
            let volume_bytes=read(&volumes);let rendered:&[GpuTree]=bytemuck::cast_slice(&volume_bytes);
            let source_bytes=read(&instances);let sources:&[GpuInstance]=bytemuck::cast_slice(&source_bytes);
            let choice_bytes=read(&runtime.choices);let choices:&[u32]=bytemuck::cast_slice(&choice_bytes);assert_eq!(choices.len(),2*spec.slots as usize);
            let mut found=0;
            for (i,slot) in slots.iter().enumerate().filter(|(_,s)|s.history[2]!=0) {
                assert_eq!(slot.history[3]&768,roles);
                assert_eq!(choices[i]&3==3,roles&256==0);assert_eq!(choices[i+spec.slots as usize]&3==3,roles&512==0);
                assert_eq!(choices[i]&1024!=0,roles&256==0);assert_eq!(choices[i]&2048!=0,roles&512==0);
                assert_eq!(sources[spec.first_owner as usize+i].flags&768,0,"consumer roles must not leak into source flags");
                let rep=spec.representations[slot.key[2]as usize];
                if roles&512!=0 {
                    assert_eq!(choices[i+spec.slots as usize]&3,rep[1],"shadow publication must preserve source versus voxel selection");
                    if rep[1]==0 {assert_eq!((choices[i+spec.slots as usize]>>8)&3,0,"source shadows must not suppress source rasterization");}
                }
                if rep[3]==0 {continue;}
                let volume=spec.branches[(rep[3]-1)as usize].volume;
                assert_eq!(rendered[i].meta[1],volume.meta[1]);assert_ne!(volume.meta[1],spec.branches[rep[0]as usize].volume.meta[1]);
                let pose=matrix(sources[spec.first_owner as usize+i].rows);
                assert!(matrix(rendered[i].inverse).abs_diff_eq(pose.inverse(),2e-5));found+=1;
            }
            assert_eq!(found,1,"a mixed source/group cut must actually materialize");
            assert_eq!(runtime.diagnostics(&gpu).unwrap().fallback_trees,0);
        }
        }
    }
    #[test]
    #[ignore="exclusive GPU shared ancestry oracle"]
    fn gpu_shared_poses_match_serial_wind_and_hash_overflow() {
        use super::*;
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();cooked.cook_proxy(&source,config,3).unwrap();
        let trees=[RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.4,scale:1.2,variant:0},RootPlacement {up:Vec3::Y,pos:Vec3::X*0.6,yaw:-0.7,scale:0.8,variant:0}];
        let mut scene=Scene::default();plant(&mut scene,&[cooked],&trees,8,crate::aggregate_lod::Policy {allow_sampled_optics:false,..Default::default()}).unwrap();
        let mut spec=scene.gpu_forest.clone().unwrap();assert_eq!(spec.branches.len(),3);
        spec.branches[2].info[0]=1; // Shared ancestor requested by both selected branches.
        for (i,b) in spec.branches.iter_mut().enumerate(){b.animation=[0.08,2.+i as f32,0.3*i as f32,0.];}
        let gpu=Gpu::new().unwrap();
        let upload=|bytes:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("pose oracle"),contents:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC});
        let read=|buffer:&wgpu::Buffer,size:u64| {
            let target=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("pose oracle read"),size,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(buffer,0,&target,0,size);gpu.queue.submit(Some(enc.finish()));target.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
            let result=target.slice(..).get_mapped_range().unwrap().to_vec();target.unmap();result
        };
        for mode in [Deformation::Rigid,Deformation::BasalShear] {
        spec.branches[2].info[0]=if mode==Deformation::Rigid {1} else {0};
        for (i,b) in spec.branches.iter_mut().enumerate(){b.info[3]=mode as u32;b.animation[0]=if i==0 && mode==Deformation::BasalShear {0.} else {0.08};}
        for capacity in [None,Some(1)] {
            let instances=upload(bytemuck::cast_slice(&scene.instances));let volumes=upload(bytemuck::cast_slice(&scene.voxel_forest.as_ref().unwrap().instances));
            let runtime=Runtime::with_pose_capacity(&gpu,&spec,&instances,&volumes,capacity).unwrap();
            let camera=crate::Camera {pos:Vec3::new(0.3,0.4,1.6),target:Vec3::new(0.3,0.4,0.),fov_y:1.,near:0.05};
            let view=crate::part_wind::AnimationView {view_projection:Mat4::perspective_infinite_reverse_rh(camera.fov_y,1.,camera.near)*Mat4::look_at_rh(camera.pos,camera.target,Vec3::Y),viewport:[512,512]};
            for time in [0.,17.,119.] {
                let wind=crate::wind::WindState {strength:0.7,time_seconds:time,..Default::default()}.validated().unwrap();runtime.set_wind(&wind).unwrap();
                let mut enc=gpu.device.create_command_encoder(&Default::default());runtime.encode(&gpu,&mut enc,&[view],spec.first_owner).unwrap();gpu.queue.submit(Some(enc.finish()));
                let data=read(&instances,scene.instances.len()as u64*std::mem::size_of::<GpuInstance>()as u64);
                let actual:&[GpuInstance]=bytemuck::cast_slice(&data);let data=read(&runtime.residency.slots,spec.slots as u64*32);let slots:&[crate::foliage_residency::ResidentSlot]=bytemuck::cast_slice(&data);
                let mut compared=0;
                for (i,slot) in slots.iter().enumerate().filter(|(_,s)|s.history[2]!=0) {
                    let branch=spec.representations[slot.key[2]as usize][0]as usize;let mut chain=vec![branch];
                    while spec.branches[*chain.last().unwrap()].info[0]!=NONE {chain.push(spec.branches[*chain.last().unwrap()].info[0]as usize);}
                    let mut m=matrix(spec.roots[slot.key[0]as usize].rows);
                    let axis=Vec3::new(-wind.direction.y,0.,wind.direction.x).normalize();let phase=(time/120.*std::f64::consts::TAU)as f32;
                    for &b in chain.iter().rev(){let p=spec.branches[b];m*=matrix(p.rest);let translation=m.w_axis;let angle=p.animation[0]*wind.strength*(phase*p.animation[1]+p.animation[2]).sin();if p.info[3]==1 {m=basal_reference(m,wind.direction,angle);} else {m=Mat4::from_axis_angle(axis,angle)*m;m.w_axis=translation;}}
                    let expected=rows(m);let got=actual[spec.first_owner as usize+i].rows;
                    for (a,b) in got.iter().flatten().zip(expected.iter().flatten()){assert!((a-b).abs()<2e-5,"pose differs {a} vs {b}");}compared+=1;
                }
                assert!(compared>=4,"oracle must exercise shared ancestry");
                let bytes=read(&runtime.pose_args,64*12);let args:&[u32]=bytemuck::cast_slice(&bytes);
                let bytes=read(&runtime.pose_work,64*4);let counts:&[u32]=bytemuck::cast_slice(&bytes);
                for depth in 0..64 {assert_eq!(&args[depth*3..depth*3+3],&[counts[depth].min(spec.slots).div_ceil(64),1,1]);}
                let d=runtime.diagnostics(&gpu).unwrap();assert_eq!(d.pose_requests,d.evaluated_shared_poses);
                if capacity.is_none(){assert_eq!(d.pose_hash_failures,0);assert_eq!(d.evaluated_shared_poses,4);}else{assert!(d.pose_hash_failures>0);}
            }
            let page=spec.representations.iter().position(|r|r[0]==1 && r[1]==0).unwrap()as u32;
            let draw=|| {let mut enc=gpu.device.create_command_encoder(&Default::default());runtime.encode(&gpu,&mut enc,&[view],spec.first_owner).unwrap();gpu.queue.submit(Some(enc.finish()));runtime.diagnostics(&gpu).unwrap()};
            runtime.selector.publish_asset_pages(&gpu,&[(page,false,1)]).unwrap();
            let d=draw();
            let bytes=read(&runtime.pose_args,64*12);let args:&[u32]=bytemuck::cast_slice(&bytes);
            assert!(args.chunks_exact(3).all(|a|a==[0,1,1]),"asset fallback must clear prior pose dispatches");
            assert_eq!(d.fallback_trees,0);assert_eq!(d.direct_roots,2);assert_eq!(d.source_slots+d.voxel_slots,0,"missing child retains a complete resident ancestor");
            let bytes=read(&runtime.selector.asset_requests,runtime.selector.asset_requests.size());let requests:&[u32]=bytemuck::cast_slice(&bytes);
            let entries=requests[4..4+requests[0]as usize*4].chunks_exact(4).collect::<Vec<_>>();
            assert!(entries.iter().any(|r|r[0]==page && r[1]==0));
            assert_eq!(entries.iter().map(|r|r[0]).collect::<std::collections::HashSet<_>>().len(),entries.len());
            let bytes=read(&volumes,scene.voxel_forest.as_ref().unwrap().instances.len()as u64*96);let visible:&[GpuTree]=bytemuck::cast_slice(&bytes);
            assert!(visible[spec.slots as usize..spec.slots as usize+spec.roots.len()].iter().all(|v|v.bounds_min[3]==3.),"camera-only root fallback remains visible without becoming a shadow caster");
            runtime.selector.publish_asset_pages(&gpu,&[(page,true,2)]).unwrap();let d=draw();assert_eq!(d.fallback_trees,0);assert_eq!(d.assigned,6);
            assert!(runtime.selector.publish_asset_pages(&gpu,&[(page,true,1)]).is_err());
            let pinned=spec.nodes.iter().find(|n|n.links[3]&2!=0).unwrap().links[2];
            assert!(runtime.selector.publish_asset_pages(&gpu,&[(page,false,2),(pinned,false,1)]).is_err());assert_eq!(draw().retained,6,"invalid batch must be atomic");
            runtime.selector.publish_asset_pages(&gpu,&[(page,true,3)]).unwrap();let d=draw();assert_eq!(d.assigned,2);assert_eq!(d.retained,4,"asset generations must retire both affected roots' old slots");
        }
        }
    }
    #[test]
    fn large_world_roots_do_not_consume_branch_pack_budget() {
        use super::*;
        let source=crate::vegetation_asset::tests::aggregate_fixture();
        let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();
        cooked.cook_proxy(&source,config,3).unwrap();
        let trees=vec![RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0};20000];
        let mut scene=Scene::default();
        plant(&mut scene,&[cooked],&trees,8,crate::aggregate_lod::Policy::default()).unwrap();
        let spec=scene.gpu_forest.as_ref().unwrap();
        assert_eq!(spec.roots.len(),20000);assert_eq!(spec.branches.len(),3);
        assert_eq!(scene.voxel_forest.as_ref().unwrap().instances.len(),40008);
        assert!(scene.part_wind.is_empty());
    }

    #[test]
    #[ignore="exclusive GPU cold asset streaming draw fixture"]
    fn gpu_foliage_cache_cold_start_recovers_real_draws() {
        use super::*;use crate::{Camera,Lighting,Renderer,ViewRect};
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();cooked.cook_proxy(&source,config,3).unwrap();
        let mut scene=Scene::default();plant(&mut scene,&[cooked],&[RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0}],8,crate::aggregate_lod::Policy {allow_sampled_optics:false,..Default::default()}).unwrap();
        let gpu=Gpu::new().unwrap();let light=Lighting {gi:1.,sun_lux:0.,shadow_strength:0.,..Default::default()};let renderer=Renderer::new(&gpu,&scene,&light);
        let views=[ViewRect {cam:Camera {pos:Vec3::new(0.3,0.4,1.6),target:Vec3::new(0.3,0.4,0.),fov_y:1.,near:0.05},x:0,y:0,w:128,h:128}];
        let mut recovered=false;
        for frame in 0..32 {
            renderer.try_render_views(&gpu,&views,&light,128,128,&[]).unwrap();let d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();
            let cache=renderer.foliage_page_stats().unwrap();assert!(cache.frame_upload_bytes<=8*1024*1024);
            if frame==0 {assert_eq!(d.source_slots,0);assert_eq!(d.direct_roots,1);let source=cache.source.unwrap();assert!(source.resident_pages<source.total_pages,"source details must really cold-start unloaded");}
            assert_eq!(d.fallback_trees,0);
            if d.direct_roots==1 {assert_eq!(d.source_slots+d.voxel_slots,0,"ancestor must suppress every partial branch");}else{assert_eq!(d.source_slots,3,"loaded cut must be complete");}
            if d.source_slots==3 {assert_eq!(d.fallback_trees,0);recovered=true;break;}
        }
        assert!(recovered,"normal async cache must publish loaded geometry without a CPU tree shortlist");
    }
    #[test]
    #[ignore="exclusive GPU and process environment static split oracle"]
    fn gpu_static_split_preserves_forest_sun_depth() {
        use super::*;use crate::{Camera,Lighting,Renderer,ViewRect,foliage_population::WorldRoot};
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {fn drop(&mut self){unsafe {match &self.0 {Some(v)=>std::env::set_var("KERN_STATIC_SUN_CACHE",v),None=>std::env::remove_var("KERN_STATIC_SUN_CACHE")}}}}
        let _restore=Restore(crate::switches::STATIC_SUN_CACHE.os());
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();cooked.cook_proxy(&source,config,3).unwrap();
        let roots=[WorldRoot {id:42,placement:RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0}}];
        let mut scene=Scene::default();plant_population(&mut scene,&[cooked],&roots,2,8,crate::aggregate_lod::Policy {allow_sampled_optics:false,..Default::default()}).unwrap();
        for b in &mut scene.gpu_forest.as_mut().unwrap().branches {b.animation=[0.12,2.,0.3,0.];}
        let gpu=Gpu::new().unwrap();let light=Lighting {gi:1.,near_shadows:0.,sun_dir:Vec3::new(0.3,1.,0.4).normalize(),..Default::default()};
        unsafe {std::env::remove_var("KERN_STATIC_SUN_CACHE");}
        let full=Renderer::new(&gpu,&scene,&light);
        unsafe {std::env::set_var("KERN_STATIC_SUN_CACHE","1");}
        let split=Renderer::new(&gpu,&scene,&light);
        full.test_warm_foliage_assets(&gpu);split.test_warm_foliage_assets(&gpu);
        let views=[ViewRect {cam:Camera {pos:Vec3::new(0.3,0.4,1.6),target:Vec3::new(0.3,0.4,0.),fov_y:1.,near:0.05},x:0,y:0,w:64,h:64}];
        let depth=|renderer:&Renderer| {
            let texture=renderer.debug_sun_depth_textures(None).unwrap().remove(0);let size=texture.size();
            let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("static split raw depth oracle"),size:size.width as u64*size.height as u64*4,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            let mut enc=gpu.device.create_command_encoder(&Default::default());
            enc.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo {texture:&texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::DepthOnly},wgpu::TexelCopyBufferInfo {buffer:&read,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(size.width*4),rows_per_image:Some(size.height)}},size);
            gpu.queue.submit([enc.finish()]);read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
            let result=bytemuck::cast_slice::<u8,u32>(&read.slice(..).get_mapped_range().unwrap()).to_vec();read.unmap();result
        };
        for step in 0..4 {
            for r in [&full,&split] {
                r.set_wind(&gpu,&crate::wind::WindState {strength:if step==0{0.}else{0.7},time_seconds:17.,..Default::default()}).unwrap();
                if step==2 {r.replace_gpu_forest_roots(&gpu,&[]).unwrap();}
                if step==3 {r.replace_gpu_forest_roots(&gpu,&roots).unwrap();}
                r.try_render_views(&gpu,&views,&light,64,64,&[]).unwrap();
            }
            let reference=depth(&full);let cached=depth(&split);
            if step==0 {assert!(reference.iter().any(|&v|v!=1f32.to_bits()),"nonempty caster oracle");}
            assert_eq!(cached.iter().zip(reference).filter(|(a,b)|**a!=*b).count(),0,"static split loses/changes forest depth step{step}");
        }
    }
    #[test]
    #[ignore="exclusive hardware GPU full renderer shadow cache oracle"]
    fn gpu_cached_sun_depth_matches_full_redraw() {
        use super::*;use crate::{Camera,Lighting,Renderer,ViewRect,foliage_population::WorldRoot};
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();cooked.cook_proxy(&source,config,3).unwrap();
        let original=[WorldRoot {id:42,placement:RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0}}];
        let mut scene=Scene::default();plant_population(&mut scene,&[cooked],&original,2,8,crate::aggregate_lod::Policy {allow_sampled_optics:false,..Default::default()}).unwrap();
        for b in &mut scene.gpu_forest.as_mut().unwrap().branches {b.animation=[0.12,2.,0.3,0.];}
        let gpu=Gpu::new().unwrap();let mut light=Lighting {gi:1.,near_shadows:0.,sun_dir:Vec3::new(0.3,1.,0.4).normalize(),..Default::default()};
        let renderer=Renderer::new(&gpu,&scene,&light);renderer.test_warm_foliage_assets(&gpu);
        let mut views=(0..4).map(|i|ViewRect {cam:Camera {pos:Vec3::new(0.3+i as f32*0.1,0.4,1.6),target:Vec3::new(0.3,0.4,0.),fov_y:1.,near:0.05},x:i*64,y:0,w:64,h:64}).collect::<Vec<_>>();
        let depth=|| {
            let texture=renderer.debug_sun_depth_textures(None).unwrap().remove(0);let size=texture.size();
            let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("cached sun raw depth oracle"),size:size.width as u64*size.height as u64*4,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            let mut enc=gpu.device.create_command_encoder(&Default::default());
            enc.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo {texture:&texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::DepthOnly},wgpu::TexelCopyBufferInfo {buffer:&read,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(size.width*4),rows_per_image:Some(size.height)}},size);
            gpu.queue.submit([enc.finish()]);let(tx,rx)=std::sync::mpsc::channel();read.slice(..).map_async(wgpu::MapMode::Read,move|r|{tx.send(r).unwrap();});gpu.wait();rx.recv().unwrap().unwrap();
            let words=bytemuck::cast_slice::<u8,u32>(&read.slice(..).get_mapped_range().unwrap()).to_vec();read.unmap();words
        };
        for step in 0..6 {
            renderer.set_wind(&gpu,&crate::wind::WindState {strength:0.7,time_seconds:if step==0{0.}else{17.},..Default::default()}).unwrap();
            if step==3 {renderer.replace_gpu_forest_roots(&gpu,&[]).unwrap();}
            if step==4 {renderer.replace_gpu_forest_roots(&gpu,&original).unwrap();light.sun_dir=Vec3::new(-0.3,1.,0.4).normalize();}
            if step==5 {views[0].cam.pos.x+=0.4;}
            renderer.try_render_views(&gpu,&views,&light,256,64,&[]).unwrap();let cached=depth();
            if step==0 {assert!(cached.iter().any(|&v|v!=1f32.to_bits()),"oracle requires real caster depth");}
            renderer.invalidate_shadow();renderer.try_render_views(&gpu,&views,&light,256,64,&[]).unwrap();let reference=depth();
            assert_eq!(cached.iter().zip(&reference).filter(|(a,b)|a!=b).count(),0,"cached/full depth differs at step {step}");
        }
    }
    #[test]
    #[ignore="exclusive GPU streamed population fixture"]
    fn gpu_population_updates_retain_slots_and_recover_from_empty() {
        use super::*;use crate::{Camera,Lighting,Renderer,ViewRect,foliage_population::WorldRoot};
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();cooked.cook_proxy(&source,config,3).unwrap();
        let root=|id,x|WorldRoot {id,placement:RootPlacement {up:Vec3::Y,pos:Vec3::X*x,yaw:0.,scale:1.,variant:0}};
        let original=[root(10,0.),root(20,8.)];let policy=crate::aggregate_lod::Policy {allow_sampled_optics:false,..Default::default()};
        let mut scene=Scene::default();plant_population(&mut scene,&[cooked],&original,3,8,policy).unwrap();
        let gpu=Gpu::new().unwrap();let light=Lighting {gi:1.,sun_lux:0.,shadow_strength:0.,..Default::default()};let renderer=Renderer::new(&gpu,&scene,&light);
        let views=(0..2).map(|i|ViewRect {cam:Camera {pos:Vec3::new(i as f32*8.+0.3,0.4,1.6),target:Vec3::new(i as f32*8.+0.3,0.4,0.),fov_y:1.,near:0.05},x:i*128,y:0,w:128,h:128}).collect::<Vec<_>>();
        renderer.test_warm_foliage_assets(&gpu);
        let draw=||{renderer.try_render_views(&gpu,&views,&light,256,128,&[]).unwrap();renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap()};
        assert_eq!(draw().source_slots,6);
        let first=renderer.debug_tree_occlusion(&gpu,0,None).unwrap();
        assert!(first.history_first);assert_eq!(first.phase1_clusters,0);assert!(first.phase2_clusters>0);
        assert_eq!(draw().retained,6);
        let next=renderer.debug_tree_occlusion(&gpu,0,None).unwrap();assert!(next.phase1_clusters>0);
        let reordered=renderer.replace_gpu_forest_roots(&gpu,&[original[1],original[0]]).unwrap();assert_eq!(reordered.changed,0);assert_eq!(reordered.upload_bytes,0);assert_eq!(draw().retained,6);
        let pending=renderer.gpu_forest_population_seed().unwrap();
        let stale=std::thread::spawn(move||pending.prepare(&[]).unwrap()).join().unwrap();
        let replacement=[root(30,0.),original[1]];let stats=renderer.replace_gpu_forest_roots(&gpu,&replacement).unwrap();assert_eq!(stats.changed,1);assert_eq!(stats.retained,1);
        assert!(renderer.commit_gpu_forest_roots(&gpu,stale).unwrap_err().contains("stale"));
        let d=draw();assert_eq!(d.source_slots,6);assert_eq!(d.retained,3);assert_eq!(d.assigned,3);
        let bad=[root(30,0.),root(40,8.),root(50,16.),root(60,24.)];assert!(renderer.replace_gpu_forest_roots(&gpu,&bad).is_err());assert_eq!(draw().active_roots,2);
        let stats=renderer.replace_gpu_forest_roots(&gpu,&[]).unwrap();assert_eq!(stats.changed,2);let d=draw();assert_eq!(d.active_roots,0);assert_eq!(d.source_slots+d.voxel_slots+d.direct_roots,0);
        renderer.replace_gpu_forest_roots(&gpu,&original).unwrap();let d=draw();assert_eq!(d.active_roots,2);assert_eq!(d.source_slots,6);assert_eq!(d.assigned,6);
    }
    // Whatever is rigid and opaque in front of the forest hides it from the
    // camera through the current-frame seed depth, never through a rule about
    // walls or a tag. A cutout material is the control: same pixels, same
    // geometry, but the forest is selected as if nothing were in front.
    #[test]
    #[ignore="exclusive GPU forest occlusion fixture"]
    fn gpu_opaque_geometry_hides_forest_from_camera() {
        use super::*;use crate::{Camera,Lighting,Renderer,ViewRect,scene::{GeometryCategory,Material}};
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();cooked.cook_proxy(&source,config,3).unwrap();
        let gpu=Gpu::new().unwrap();let light=Lighting {gi:0.,sun_lux:0.,shadow_strength:0.,..Default::default()};
        // wall: (centre, size, category, material flags). None means open ground.
        let forest=|trees:&[f32],wall:Option<(Vec3,Vec3,GeometryCategory,u32)>,cam:Camera| {
            let roots:Vec<_>=trees.iter().map(|&x|RootPlacement {up:Vec3::Y,pos:Vec3::new(x,0.,0.),yaw:0.,scale:1.,variant:0}).collect();
            let mut scene=Scene::default();
            plant(&mut scene,std::slice::from_ref(&cooked),&roots,8,crate::aggregate_lod::Policy {allow_sampled_optics:false,..Default::default()}).unwrap();
            if let Some((at,size,category,flags))=wall {
                let paint=scene.add_material(Material {tint:[0.5;3],layer:-1,roughness:0.9,flags,scale:1.,metal:0.});
                let mesh=scene.add_mesh(&crate::primitives::cube(paint).mesh());
                scene.add_classified_instance(mesh,glam::Affine3A::from_scale_rotation_translation(size,glam::Quat::IDENTITY,at),category);
            }
            let renderer=Renderer::new(&gpu,&scene,&light);renderer.test_warm_foliage_assets(&gpu);
            let views=[ViewRect {cam,x:0,y:0,w:128,h:128}];
            let mut last=None;
            for _ in 0..3 {let (image,_)=renderer.try_render_views(&gpu,&views,&light,128,128,&[]).unwrap();last=Some((image,renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap()));}
            let (image,diagnostics)=last.unwrap();
            let splats=renderer.voxel_diagnostics(&gpu).unwrap().unwrap()[0].raster_splats;
            let drawn=splats as usize+diagnostics.source_slots as usize+diagnostics.voxel_slots as usize>0;
            assert_eq!(drawn,diagnostics.camera_roots>0,"forest is drawn exactly while the camera may see it");
            (image,diagnostics)
        };
        let frame=|wall,cam|forest(&[0.],wall,cam);
        // The root's bounding sphere has radius ~2.1 around (0.6,0.6,0.2).
        let front=Camera {pos:Vec3::new(0.6,0.6,12.),target:Vec3::new(0.6,0.6,0.),fov_y:1.,near:0.05};
        // Close enough that the tree's bounds run past every screen edge.
        let close=Camera {pos:Vec3::new(0.6,0.6,5.),target:Vec3::new(0.6,0.6,0.),fov_y:1.,near:0.05};
        let full=|z:f32,category|Some((Vec3::new(0.6,0.6,z),Vec3::new(30.,30.,0.05),category,0));
        // Untextured cutout draws exactly like the opaque wall, but a cutout
        // material may have holes, so it must never hide anything.
        let cutout=|z:f32|Some((Vec3::new(0.6,0.6,z),Vec3::new(30.,30.,0.05),GeometryCategory::Other,crate::scene::ALPHA_TEST));
        let (_,open)=frame(None,front);
        assert_eq!(open.camera_roots,1,"fixture: the tree is in view");assert!(open.source_slots+open.voxel_slots>0,"fixture: the tree draws");
        for (name,cam,z) in [("whole tree on screen",front,5.),("tree bounds cross the screen edge",close,3.5)] {
            let (seen,control)=frame(cutout(z),cam);
            eprintln!("{name}: camera roots {} behind a cutout wall",control.camera_roots);
            assert_eq!(control.camera_roots,1,"{name}: a cutout material must not cull");
            // No tag needed: any rigid opaque geometry hides the forest.
            for category in [GeometryCategory::Other,GeometryCategory::Structures,GeometryCategory::Cars,GeometryCategory::Terrain] {
                let (hidden,culled)=frame(full(z,category),cam);
                assert_eq!(culled.camera_roots,0,"{name}: a tree fully behind an opaque {} wall must not be selected for the camera",category.name());
                let differ=seen.chunks_exact(4).zip(hidden.chunks_exact(4)).filter(|(a,b)|a.iter().zip(b.iter()).any(|(&a,&b)|a.abs_diff(b)>2)).count();
                assert_eq!(differ,0,"{name}: culling must not change a single visible pixel");
            }
            // Foliage never occludes foliage: its cards are full of gaps.
            let (_,leafy)=frame(full(z,GeometryCategory::Trees),cam);
            assert_eq!(leafy.camera_roots,1,"{name}: geometry in the trees category must not cull");
        }
        // A wall over only the left half leaves the tree's right side showing.
        let (_,half)=frame(Some((Vec3::new(-14.4,0.6,5.),Vec3::new(30.,30.,0.05),GeometryCategory::Structures,0)),front);
        assert_eq!(half.camera_roots,1,"a partly covered tree stays selected");
        let (_,behind)=frame(full(-5.,GeometryCategory::Structures),front);
        assert_eq!(behind.camera_roots,1,"a wall behind the tree must not hide it");
        let (_,low)=frame(Some((Vec3::new(0.6,-14.4,5.),Vec3::new(30.,30.,0.05),GeometryCategory::Structures,0)),front);
        assert_eq!(low.camera_roots,1,"a low block must not hide the tree above it");
        let (_,beside)=forest(&[0.,7.],Some((Vec3::new(0.6,0.6,5.),Vec3::new(6.,6.,0.05),GeometryCategory::Structures,0)),front);
        assert_eq!(beside.camera_roots,1,"only the tree behind the wall goes");
    }
    // Selection encodes only the refinement rounds the GPU reported needing,
    // instead of the hierarchy depth bound, without changing what is drawn.
    #[test]
    #[ignore="exclusive GPU forest selection rounds fixture"]
    fn gpu_selection_rounds_follow_reported_need() {
        use super::*;use crate::{Camera,Lighting,Renderer,ViewRect};
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();cooked.cook_proxy(&source,config,3).unwrap();
        let trees=(0..6).map(|i|RootPlacement {up:Vec3::Y,pos:Vec3::new(i as f32*3.-7.5,0.,-(i%3) as f32*4.),yaw:i as f32,scale:1.,variant:0}).collect::<Vec<_>>();
        let mut scene=Scene::default();plant(&mut scene,&[cooked],&trees,24,crate::aggregate_lod::Policy {allow_sampled_optics:false,..Default::default()}).unwrap();
        let gpu=Gpu::new().unwrap();let light=Lighting {gi:0.,sun_lux:0.,shadow_strength:0.,..Default::default()};
        let adaptive=Renderer::new(&gpu,&scene,&light);let reference=Renderer::new(&gpu,&scene,&light);
        adaptive.test_warm_foliage_assets(&gpu);reference.test_warm_foliage_assets(&gpu);reference.test_forest_rounds(true,None);
        let mut views=[ViewRect {cam:Camera {pos:Vec3::new(0.,1.,9.),target:Vec3::new(0.,0.8,0.),fov_y:1.,near:0.05},x:0,y:0,w:128,h:128}];
        for frame in 0..12 {
            // A jump halfway, after pretending the last frames needed almost
            // nothing: the cut must encode the full bound, not the stale hint.
            if frame==8 {views[0].cam.pos=Vec3::new(-7.,0.9,2.5);views[0].cam.target=Vec3::new(-7.5,0.8,0.);adaptive.test_forest_rounds(false,Some(2));}
            let (a,_)=adaptive.try_render_views(&gpu,&views,&light,128,128,&[]).unwrap();
            let (b,_)=reference.try_render_views(&gpu,&views,&light,128,128,&[]).unwrap();
            let (rounds,used,bound)=adaptive.test_forest_rounds(false,None);
            let (da,db)=(adaptive.gpu_forest_diagnostics(&gpu).unwrap().unwrap(),reference.gpu_forest_diagnostics(&gpu).unwrap().unwrap());
            eprintln!("frame {frame}: {rounds} of {bound} rounds encoded, GPU reported {used:?} working, source slots {} vs {}",da.source_slots,db.source_slots);
            if frame==7 {assert!(used.is_some_and(|u|u<bound),"the GPU must report how many rounds did work");}
            if frame==8 {assert_eq!(rounds,bound,"a camera cut encodes the full bound");}
            assert_eq!(a,b,"frame {frame}: fewer rounds must draw the same image");
        }
    }
    #[test]
    #[ignore="exclusive GPU deferred queue overflow fixture"]
    fn gpu_history_queue_overflow_preserves_pixels() {
        use super::*;use crate::{Camera,Lighting,Renderer,ViewRect};
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=crate::vegetation_asset::CookedPlant::cook_aggregate(&source,config).unwrap();cooked.cook_proxy(&source,config,3).unwrap();
        let mut scene=Scene::default();plant(&mut scene,&[cooked],&[RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0}],8,crate::aggregate_lod::Policy {allow_sampled_optics:false,..Default::default()}).unwrap();
        let gpu=Gpu::new().unwrap();let light=Lighting {gi:0.,sun_lux:0.,shadow_strength:0.,..Default::default()};
        let regular=Renderer::new(&gpu,&scene,&light);let mut tiny=Renderer::new(&gpu,&scene,&light);tiny.test_tree_deferred_capacity(&gpu,1);
        regular.test_warm_foliage_assets(&gpu);tiny.test_warm_foliage_assets(&gpu);
        let views=[ViewRect {cam:Camera {pos:Vec3::new(0.3,0.4,1.6),target:Vec3::new(0.3,0.4,0.),fov_y:1.,near:0.05},x:0,y:0,w:128,h:128}];
        for frame in 0..3 {
            let (a,_)=regular.try_render_views(&gpu,&views,&light,128,128,&[]).unwrap();
            let (b,_)=tiny.try_render_views(&gpu,&views,&light,128,128,&[]).unwrap();
            assert_eq!(a,b,"deferred overflow must recover the same source geometry");
            if frame==0 {assert!(tiny.debug_tree_occlusion(&gpu,0,None).unwrap().deferred_overflow);}
        }
    }
    #[test]
    #[ignore="exclusive GPU forest draw fixture"]
    fn gpu_forest_voxels_overflow_split_and_teleport() {
        use super::*;
        use crate::{Camera,Lighting,Renderer,ViewRect,vegetation_asset::{CookedPlant,SceneVegetation,AttachLimits,PlantInstance}};
        let source=crate::vegetation_asset::tests::aggregate_fixture();let config=crate::vegetation_asset::tests::aggregate_config();
        let mut cooked=CookedPlant::cook_aggregate(&source,config).unwrap();cooked.cook_proxy(&source,config,3).unwrap();
        // Keep GI enabled: conservative selection bounds must not inflate canopy density.
        let gpu=Gpu::new().unwrap();let light=Lighting {gi:1.,shadow_strength:0.,..Default::default()};
        let policy=crate::aggregate_lod::Policy {allow_sampled_optics:false,..Default::default()};
        let cam=|x:f32|Camera {pos:Vec3::new(x+0.3,0.4,1.6),target:Vec3::new(x+0.3,0.4,0.),fov_y:1.,near:0.05};
        let mut reference=Scene::default();let mut b=SceneVegetation::new(&mut reference,AttachLimits::default()).unwrap();
        b.append_proxy(&cooked,PlantInstance {id:1,generation:0,root:Mat4::IDENTITY,category:crate::scene::GeometryCategory::Trees}).unwrap();b.finish_aggregate(policy).unwrap();
        let reference=Renderer::new(&gpu,&reference,&light);
        let view=[ViewRect {cam:cam(0.),x:0,y:0,w:128,h:128}];
        let (expected,_)=reference.try_render_views(&gpu,&view,&light,128,128,&[]).unwrap();
        let tree=RootPlacement {up:Vec3::Y,pos:Vec3::ZERO,yaw:0.,scale:1.,variant:0};
        let mut scene=Scene::default();plant(&mut scene,std::slice::from_ref(&cooked),&[tree],1,policy).unwrap();
        let renderer=Renderer::new(&gpu,&scene,&light);
        let (mut actual,_)=renderer.try_render_views(&gpu,&view,&light,128,128,&[]).unwrap();
        let mut d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();
        // Cold pages can retain a direct ancestor before full source demand
        // exists. This test exercises slot overflow, not asynchronous I/O timing.
        for _ in 0..64 {
            if d.refinement_budget_stops>0 {break;}
            actual=renderer.try_render_views(&gpu,&view,&light,128,128,&[]).unwrap().0;
            d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();
        }
        eprintln!("bounded ancestor diagnostics: {d:?}");
        assert_eq!(d.fallback_trees,0);assert_eq!(d.direct_roots,2);assert_eq!(d.source_slots+d.voxel_slots,0);
        assert!(d.refinement_budget_stops>0);assert_eq!(d.refinement_faults,0);
        let raster=renderer.voxel_diagnostics(&gpu).unwrap().unwrap();
        assert!(raster[0].raster_splats>0);assert!(raster[0].raster_splats as u64<=raster[0].raster_capacity);
        let mae=expected.iter().zip(&actual).map(|(&a,&b)|a.abs_diff(b)as u64).sum::<u64>()as f64/expected.len()as f64;
        // The selected complete ancestor is intentionally not the separate emergency
        // proxy. Its optical comparison is reported, not asserted equivalent.
        eprintln!("retained ancestor vs emergency proxy reference MAE={mae}");
        // Force regular whole-subtree voxel selection, distinct from overflow.
        let mut scene=Scene::default();let coarse=crate::aggregate_lod::Policy {allow_sampled_optics:true,max_color_error:1.,max_coverage_error:1.,max_error_pixels:10000.,..policy};
        plant(&mut scene,std::slice::from_ref(&cooked),&[tree],8,coarse).unwrap();
        let renderer=Renderer::new(&gpu,&scene,&light);let distant=[ViewRect {cam:Camera {pos:Vec3::new(0.3,0.4,20.),..cam(0.)},..view[0]}];renderer.try_render_views(&gpu,&distant,&light,128,128,&[]).unwrap();
        let d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();assert_eq!(d.direct_roots,2);assert_eq!(d.voxel_slots,0);assert_eq!(d.source_slots,0);assert_eq!(d.fallback_trees,0);
        // Move between disjoint tree populations, then use four simultaneous
        // cameras. No host assignment, renderer rebuild or root upload occurs.
        let trees=(0..4).map(|i|RootPlacement {up:Vec3::Y,pos:Vec3::X*(i as f32*8.),..tree}).collect::<Vec<_>>();
        let mut scene=Scene::default();plant(&mut scene,&[cooked],&trees,16,policy).unwrap();let renderer=Renderer::new(&gpu,&scene,&light);
        renderer.test_warm_foliage_assets(&gpu);
        let night=Lighting {sun_lux:0.,..light.clone()};
        for index in [0,3,1,2,0] {
            let v=[ViewRect {cam:cam(index as f32*8.),x:0,y:0,w:128,h:128}];
            renderer.try_render_views(&gpu,&v,&night,128,128,&[]).unwrap();let d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();
            assert_eq!(d.fallback_trees,0);assert_eq!(d.source_slots,3);assert_eq!(d.assigned,3);assert_eq!(d.shadow_voxel_slots,0,"no shadow draw may publish when night has no shadow consumers");
        }
        let views=(0..4).map(|i|ViewRect {cam:cam(i as f32*8.),x:(i%2)*128,y:(i/2)*128,w:128,h:128}).collect::<Vec<_>>();
        renderer.try_render_views(&gpu,&views,&night,256,256,&[]).unwrap();let d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();
        assert_eq!(d.source_slots,12);assert_eq!(d.fallback_trees,0);
        renderer.try_render_views(&gpu,&view,&light,128,128,&[]).unwrap();
        let d=renderer.gpu_forest_diagnostics(&gpu).unwrap().unwrap();
        assert_eq!(d.fallback_trees,0);assert_eq!(d.voxel_slots,0);
        let source=d.selected_source_consumers;
        assert_eq!(source.shadow_only+source.both+3*(d.selected_direct_consumers.shadow_only+d.selected_direct_consumers.both),12,"all four trees remain complete casters, including any budget-retained whole root");
        assert!(source.shadow_only+source.both>0 || d.shadow_voxel_slots>0 || d.direct_roots>0,"daylight must restore source or voxel shadow publication");
        assert_eq!(d.refinement_faults,0);
        assert!(d.reserved_raster_splats.iter().all(|&n|n<=scene.gpu_forest.as_ref().unwrap().raster_budget));
        eprintln!("GPU forest source/voxel draws, fallback, teleport, four cameras PASS");
    }
}

#[cfg(test)] mod profile_volume_tests {
    #[test] fn root_consumers_follow_allocation_sized_volume_ranges() {
        for slots in [1u32,4096,32768,65537] {
            let decode=|v|super::profile_root_volume(v,slots,3);
            assert_eq!(decode(slots as usize-1),None);
            for root in 0..3usize {
                assert_eq!(decode(slots as usize+root),Some((root as u32,256)));
                assert_eq!(decode(slots as usize+3+root),Some((root as u32,512)));
            }
            assert_eq!(decode(slots as usize+6),None);
        }
        assert_eq!(super::profile_root_volume(0,0,0),None);
    }
}
