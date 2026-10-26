//! Current-pose receiver demand for existing directional shadow maps.
//!
//! This changes neither shadow projection nor caster LOD. Current instance
//! spheres intersecting any camera frustum request light-space tiles. Terrain
//! owners refine those requests with every transformed resident cluster bound
//! (all LODs/materials, current wind), avoiding scene-wide road spheres. Explicit
//! providers add all non-instance receiver support. Unregistered providers or
//! uncertain bounds request every tile. Camera visibility never limits casters.
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

pub const TILE_SIZE:u32=64;
pub const MAX_MAPS:usize=5;
pub const MAX_CAMERAS:usize=4;
pub const MAX_EXTRA_BOUNDS:usize=8192;
/// GPU-detected invalid bounds or a missing instance/wind slot.
pub const INVALID_BOUNDS:u32=1;
/// The provider supplied more bounds than the bounded upload can hold.
pub const EXTRA_CAPACITY:u32=2;
/// Callers may OR additional, stable provider-specific bits with these flags.
pub const UNREGISTERED_PROVIDER:u32=4;
const HEADER_WORDS:u32=16;
const MAP_WORDS:u32=32;
const MASK_START:u32=HEADER_WORDS+MAX_MAPS as u32*MAP_WORDS;
const MASK_WORDS:u32=5461+4*1365;
const BUFFER_WORDS:u32=MASK_START+MASK_WORDS;

#[derive(Clone,Copy,Debug)]
pub struct ReceiverCamera {pub view_projection:Mat4,pub map_mask:u32}
#[derive(Clone,Copy,Debug)]
pub struct ShadowMap {pub view_projection:Mat4,pub size:u32}
#[derive(Clone,Copy,Debug)]
pub struct ReceiverBounds {pub min:Vec3,pub max:Vec3,pub map_mask:u32}

#[derive(Clone,Debug,Default)]
pub struct MapDiagnostics {
    pub map_index:u32,
    pub size:u32,
    pub tile_size:u32,
    pub tiles_per_axis:u32,
    pub requested_tiles:u32,
    pub total_tiles:u32,
    /// Inclusive base-tile occupancy, X + Y * tiles_per_axis. Explicit capture
    /// only; compare all depth texels in every nonzero tile against reference.
    pub mask:Vec<u32>,
    pub receiver_bounds:u32,
    /// Retained by a published all-tiles map, without projection/OR work.
    /// Not included in caster_clusters_tested.
    pub caster_clusters_full_demand:u32,
    /// Actual projection/OR queries; excludes fail-open and full-map paths.
    pub caster_clusters_tested:u32,
    pub caster_clusters_rejected:u32,
}
#[derive(Clone,Debug,Default)]
pub struct Diagnostics {
    pub enabled:bool,
    pub requested_generation:u32,
    pub published_generation:u32,
    pub fallback_reasons:u32,
    /// All persistent mask, configuration and bounded provider-upload buffers.
    pub shared_bytes:u64,
    pub mask_bytes:u64,
    /// Actual gather work. Completion early-outs make these owner/bound and
    /// cluster counters scheduling-dependent; final tile occupancy is exact.
    pub receiver_owner_tests:u32,
    pub receiver_bounds:u32,
    pub extra_bounds:u32,
    /// Frustum-retained owners requested as one sphere or refined into all
    /// resident cluster bounds. Their sum equals receiver_bounds.
    pub coarse_receiver_owners:u32,
    pub refined_receiver_owners:u32,
    /// Actual GPU cluster iterations/retained bounds; all available LODs and
    /// transparent materials until every possible camera map is full. Counts
    /// vary with scheduling. Not selected or rasterized triangles.
    pub receiver_cluster_tests:u64,
    pub receiver_cluster_bounds:u64,
    pub maps:Vec<MapDiagnostics>,
}

#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct GpuCamera {planes:[[f32;4];6],flags:[u32;4]}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct GpuMap {vp:[f32;16],info:[u32;4],offsets:[u32;8],reserved:[u32;4]}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Config {header:[u32;4],control:[u32;4],cameras:[GpuCamera;4],maps:[GpuMap;5]}
#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct Extra {lower:[f32;3],map_mask:u32,upper:[f32;3],reserved:u32}

fn configuration(generation:u32,owners:u32,cameras:&[ReceiverCamera],maps:&[ShadowMap],
    extra:&[ReceiverBounds],fallback:u32)->Result<(Config,Vec<Extra>),String> {
    if generation==0 || cameras.len()>MAX_CAMERAS || maps.is_empty() || maps.len()>MAX_MAPS {
        return Err("receiver demand needs nonzero generation, 1..5 maps and at most four cameras".into());
    }
    let mut c=Config::zeroed();
    c.header=[generation,maps.len()as u32,cameras.len()as u32,owners];
    c.control[1]=fallback;
    let allowed_mask=(1u32<<maps.len())-1;
    let mut offset=MASK_START;
    for (i,map) in maps.iter().enumerate() {
        let max_size=if i==0 {4096}else{2048};
        let rows=map.view_projection.transpose();
        if !map.view_projection.is_finite() ||
            rows.w_axis!=glam::Vec4::W || !map.size.is_power_of_two() ||
            map.size<TILE_SIZE || map.size>max_size {
            return Err(format!("receiver shadow map {i} needs a finite orthographic matrix and power-of-two size64..{max_size}"));
        }
        let tiles=map.size/TILE_SIZE;let levels=tiles.ilog2()+1;
        let mut desc=GpuMap::zeroed();desc.vp=map.view_projection.to_cols_array();desc.info=[map.size,tiles,levels,0];
        for level in 0..levels {let dim=tiles>>level;desc.offsets[level as usize]=offset;offset+=dim*dim;}
        c.maps[i]=desc;
    }
    assert!(offset<=BUFFER_WORDS);
    for (i,camera) in cameras.iter().enumerate() {
        if !camera.view_projection.is_finite() || camera.map_mask&!allowed_mask!=0 {
            return Err("invalid receiver camera or shadow map mask".into());
        }
        let m=camera.view_projection.transpose();
        let planes=[m.w_axis+m.x_axis,m.w_axis-m.x_axis,m.w_axis+m.y_axis,
            m.w_axis-m.y_axis,m.z_axis,m.w_axis-m.z_axis];
        for (j,plane) in planes.into_iter().enumerate() {
            let length=plane.truncate().length();
            // Infinite-far projections can have one degenerate plane. A zero
            // normal with positive D is permissive, matching view_uniform.
            c.cameras[i].planes[j]=if length>1e-12 {(plane/length).to_array()}else{[0.,0.,0.,1.]};
        }
        c.cameras[i].flags[0]=camera.map_mask;
    }
    let mut uploaded=Vec::with_capacity(extra.len().min(MAX_EXTRA_BOUNDS));
    if extra.len()>MAX_EXTRA_BOUNDS {c.control[1]|=EXTRA_CAPACITY;}
    for b in extra.iter().take(MAX_EXTRA_BOUNDS) {
        if !b.min.is_finite() || !b.max.is_finite() || b.min.abs().max_element()>=1e18 ||
            b.max.abs().max_element()>=1e18 || !b.min.cmple(b.max).all() || b.map_mask&!allowed_mask!=0 {
            c.control[1]|=INVALID_BOUNDS;continue;
        }
        uploaded.push(Extra {lower:b.min.to_array(),upper:b.max.to_array(),map_mask:b.map_mask,reserved:0});
    }
    c.control[0]=uploaded.len()as u32;
    Ok((c,uploaded))
}

pub(crate) fn cull_source()->String {
    format!("{}\n{}",include_str!("shaders/receiver_shadow_math.wgsl"),include_str!("shaders/receiver_shadow_query.wgsl"))
}
fn builder_source()->String {
    format!("{}\n{}",include_str!("shaders/receiver_shadow_math.wgsl"),include_str!("shaders/receiver_shadow_demand.wgsl"))
}
const ENTRIES:[&str;11]=["demand_clear","demand_owners","demand_extras","demand_fallback",
    "demand_reduce_1","demand_reduce_2","demand_reduce_3","demand_reduce_4","demand_reduce_5","demand_reduce_6",
    "demand_publish"];
struct Builder {config:wgpu::Buffer,extra:wgpu::Buffer,bind:wgpu::BindGroup,pipelines:Vec<wgpu::ComputePipeline>}
pub struct Resources {enabled:bool,words:wgpu::Buffer,owner_slots:u32,builder:Option<Builder>}
impl Resources {
    pub fn new(gpu:&Gpu,instances:&wgpu::Buffer,wind:&wgpu::Buffer,clusters:&wgpu::Buffer,owner_slots:u32)->Result<Self,String> {
        let enabled=std::env::var("KERN_RECEIVER_SHADOW_DEMAND").as_deref()==Ok("1");
        let words=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("receiver shadow tile demand"),
            size:if enabled {u64::from(BUFFER_WORDS)*4}else{16},
            usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let builder=if enabled {Some(crate::shaders::checked(gpu,|| {
            let config=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("receiver shadow config"),
                size:std::mem::size_of::<Config>()as u64,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            let extra=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("receiver shadow providers"),
                size:(MAX_EXTRA_BOUNDS*std::mem::size_of::<Extra>())as u64,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            let entries=(0..6).map(|binding|wgpu::BindGroupLayoutEntry {binding,visibility:wgpu::ShaderStages::COMPUTE,
                ty:wgpu::BindingType::Buffer {ty:if binding==0 {wgpu::BufferBindingType::Uniform}else{wgpu::BufferBindingType::Storage {read_only:binding!=4}},
                    has_dynamic_offset:false,min_binding_size:None},count:None}).collect::<Vec<_>>();
            let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("receiver demand"),entries:&entries});
            let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("receiver demand"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
            let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("receiver demand"),source:wgpu::ShaderSource::Wgsl(builder_source().into())});
            let pipelines=ENTRIES.map(|entry|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(entry),layout:Some(&pl),module:&shader,
                entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()})).into_iter().collect();
            let buffers=[&config,instances,wind,&extra,&words,clusters];
            let entries=buffers.iter().enumerate().map(|(i,b)|wgpu::BindGroupEntry {binding:i as u32,resource:b.as_entire_binding()}).collect::<Vec<_>>();
            let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("receiver demand"),layout:&layout,entries:&entries});
            Builder {config,extra,bind,pipelines}
        })?)}else{None};
        Ok(Self {enabled,words,owner_slots,builder})
    }
    pub fn enabled(&self)->bool {self.enabled}
    pub fn buffer(&self)->&wgpu::Buffer {&self.words}
    pub fn encode(&self,gpu:&Gpu,encoder:&mut wgpu::CommandEncoder,generation:u32,cameras:&[ReceiverCamera],maps:&[ShadowMap],
        extra:&[ReceiverBounds],fallback_reasons:u32,time:Option<(&wgpu::QuerySet,u32)>)->Result<(),String> {
        let Some(b)=&self.builder else{return Ok(());};
        let (config,extra)=configuration(generation,self.owner_slots,cameras,maps,extra,fallback_reasons)?;
        gpu.queue.write_buffer(&b.config,0,bytemuck::bytes_of(&config));
        if !extra.is_empty() {gpu.queue.write_buffer(&b.extra,0,bytemuck::cast_slice(&extra));}
        let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("receiver shadow demand"),
            timestamp_writes:time.map(|(query_set,i)|wgpu::ComputePassTimestampWrites {query_set,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:Some(i+1)})});
        pass.set_bind_group(0,&b.bind,&[]);
        pass.set_pipeline(&b.pipelines[0]);pass.dispatch_workgroups(BUFFER_WORDS.div_ceil(64),1,1);
        if self.owner_slots>0 {let n=self.owner_slots.div_ceil(64);pass.set_pipeline(&b.pipelines[1]);pass.dispatch_workgroups(n.min(65535),n.div_ceil(65535),1);}
        if !extra.is_empty() {pass.set_pipeline(&b.pipelines[2]);pass.dispatch_workgroups((extra.len()as u32).div_ceil(64),1,1);}
        pass.set_pipeline(&b.pipelines[3]);pass.dispatch_workgroups(64,maps.len()as u32,1);
        for level in 1..=6 {let dim=64u32>>level;pass.set_pipeline(&b.pipelines[3+level]);pass.dispatch_workgroups((dim*dim).div_ceil(64),maps.len()as u32,1);}
        // Unique-tile counts are maintained during gather/fallback. Publish
        // only after those dispatches and every OR level have completed.
        pass.set_pipeline(&b.pipelines[10]);pass.dispatch_workgroups(1,1,1);
        Ok(())
    }
    /// Blocking explicit capture. Never called from ordinary frame rendering.
    pub fn diagnostics(&self,gpu:&Gpu)->Result<Diagnostics,String> {
        let shared_bytes=self.words.size()+self.builder.as_ref().map_or(0,|b|b.config.size()+b.extra.size());
        if !self.enabled {return Ok(Diagnostics {shared_bytes,mask_bytes:self.words.size(),..Default::default()});}
        let target=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("receiver demand capture"),size:self.words.size(),
            usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut encoder=gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&self.words,0,&target,0,self.words.size());gpu.queue.submit([encoder.finish()]);
        target.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
        let bytes=target.slice(..).get_mapped_range().map_err(|e|e.to_string())?;
        let words=bytes.chunks_exact(4).map(|b|u32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>();
        drop(bytes);target.unmap();decode(&words,shared_bytes)
    }
}
fn decode(words:&[u32],shared_bytes:u64)->Result<Diagnostics,String> {
    if words.len()!=BUFFER_WORDS as usize || words[4]>MAX_MAPS as u32 {return Err("invalid receiver-demand capture layout".into());}
    let wide=|i|u64::from(words[i])|(u64::from(words[i+1])<<32);
    let mut d=Diagnostics {enabled:words[0]!=0,requested_generation:words[1],published_generation:words[2],fallback_reasons:words[3],
        shared_bytes,mask_bytes:words.len()as u64*4,receiver_owner_tests:words[6],receiver_bounds:words[7],extra_bounds:words[8],
        coarse_receiver_owners:words[9],refined_receiver_owners:words[10],receiver_cluster_tests:wide(11),receiver_cluster_bounds:wide(13),maps:Vec::new()};
    if u64::from(d.coarse_receiver_owners)+u64::from(d.refined_receiver_owners)!=u64::from(d.receiver_bounds)
        || d.receiver_bounds>d.receiver_owner_tests || d.receiver_cluster_bounds>d.receiver_cluster_tests {
        return Err("receiver refinement counters violate owner/cluster bounds".into());
    }
    for map in 0..words[4] {
        let base=(HEADER_WORDS+map*MAP_WORDS)as usize;let tiles=words[base+17];
        if tiles==0 || tiles>64 || !tiles.is_power_of_two() {return Err("invalid receiver mask dimensions".into());}
        let total=tiles*tiles;let offset=words[base+20]as usize;
        let mask=words.get(offset..offset+total as usize).ok_or("invalid receiver mask offset")?.to_vec();
        let requested=mask.iter().filter(|v|**v!=0).count()as u32;
        if requested!=words[base+28] || words[base+31]>words[base+30]
            || (words[base+19]!=0 && requested!=total) {return Err("receiver mask counters do not match actual occupancy".into());}
        d.maps.push(MapDiagnostics {map_index:map,size:words[base+16],tile_size:TILE_SIZE,tiles_per_axis:tiles,requested_tiles:requested,
            total_tiles:total,mask,receiver_bounds:words[base+29],caster_clusters_full_demand:words[base+19],
            caster_clusters_tested:words[base+30],caster_clusters_rejected:words[base+31]});
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2,Vec4};

    // f32 counterpart of the shared WGSL rectangle calculation. Tests below
    // compare it with individually projected physical samples, not itself.
    fn rectangle(vp:Mat4,center:Vec3,extent:Vec3,size:u32,guard:f32)->Option<([u32;2],[u32;2])> {
        let rows=vp.transpose();let p=vp*center.extend(1.);
        let radius=Vec2::new(rows.x_axis.truncate().abs().dot(extent),rows.y_axis.truncate().abs().dot(extent));
        let magnitude=Vec2::new(rows.x_axis.abs().dot(center.abs().extend(1.)),rows.y_axis.abs().dot(center.abs().extend(1.)))+radius;
        let uncertainty=(magnitude+Vec2::ONE)*(32.*f32::EPSILON);
        let uv=Vec2::new(p.x*0.5+0.5,-p.y*0.5+0.5);let pad=(radius+uncertainty)*(0.5*size as f32)+Vec2::splat(guard);
        let lo=uv*size as f32-pad;let hi=uv*size as f32+pad;
        if hi.cmplt(Vec2::ZERO).any() || lo.cmpgt(Vec2::splat(size as f32)).any() {return None;}
        let end=Vec2::splat((size-1)as f32);
        Some(((lo.floor().clamp(Vec2::ZERO,end).as_uvec2()/TILE_SIZE).to_array(),
            (hi.ceil().clamp(Vec2::ZERO,end).as_uvec2()/TILE_SIZE).to_array()))
    }
    fn matrix(center:Vec3,extent:f32)->Mat4 {
        Mat4::orthographic_rh(-extent,extent,-extent,extent,1.,2000.)*
            Mat4::look_at_rh(center+Vec3::new(200.,300.,100.),center,Vec3::Y)
    }
    #[test]
    fn receiver_filter_guard_covers_actual_pcf_normal_offset_and_map_edges() {
        for size in [2048,4096] {for extent in [16.,150.,600.] {for center in [Vec3::ZERO,Vec3::new(100000.,80.,-75000.)] {
            let vp=matrix(center,extent);let inv=vp.inverse();let texel=2.*extent/size as f32;
            for x in [-0.999,-0.5,-0.00001,0.,0.5,0.999] {for y in [-0.999,-0.125,0.,0.125,0.999] {
                let receiver=(inv*Vec4::new(x,y,0.4,1.)).truncate();
                let (first,last)=rectangle(vp,receiver,Vec3::ZERO,size,4.).unwrap();
                for normal in [Vec3::X,Vec3::Y,Vec3::Z,Vec3::new(1.,2.,-3.).normalize()] {for sign in [-1.,1.] {
                    // Actual legacy offset plus worst PCF/bilinear reach1.5px.
                    let q=vp*(receiver+normal*(sign*1.5*texel)).extend(1.);
                    for dx in [-1.5,1.5] {for dy in [-1.5,1.5] {
                        let pixel=Vec2::new((q.x*0.5+0.5)*size as f32+dx,(-q.y*0.5+0.5)*size as f32+dy)
                            .floor().clamp(Vec2::ZERO,Vec2::splat((size-1)as f32)).as_uvec2()/TILE_SIZE;
                        assert!(pixel.x>=first[0]&&pixel.y>=first[1]&&pixel.x<=last[0]&&pixel.y<=last[1]);
                    }}
                }}
            }}
        }}}
    }
    #[test]
    fn upstream_off_camera_caster_keeps_requested_depth_without_receiver_z_cull() {
        let vp=matrix(Vec3::ZERO,150.);let inv=vp.inverse();
        let receiver=(inv*Vec4::new(0.13,-0.27,0.8,1.)).truncate();
        let caster=(inv*Vec4::new(0.13,-0.27,0.02,1.)).truncate();
        assert!(receiver.distance(caster)>1000.);
        let (a,b)=rectangle(vp,receiver,Vec3::splat(0.01),4096,4.).unwrap();
        let (c,d)=rectangle(vp,caster,Vec3::splat(0.01),4096,1.).unwrap();
        assert!(c[0]<=b[0]&&c[1]<=b[1]&&d[0]>=a[0]&&d[1]>=a[1]);
        assert!(rectangle(vp,(inv*Vec4::new(-0.7,0.6,0.1,1.)).truncate(),Vec3::ONE,4096,1.).unwrap().1[0]<a[0]);
    }
    fn camera_mask(config:&Config,center:Vec3,radius:f32)->u32 {
        let pad=(center.abs().element_sum()+radius+1.)*(32.*f32::EPSILON);
        config.cameras[..config.header[2]as usize].iter().filter(|camera|
            camera.planes.iter().all(|p|Vec3::from_slice(p).dot(center)+p[3]>=-radius-pad))
            .fold(0,|mask,c|mask|c.flags[0])
    }
    #[test]
    fn cluster_receivers_preserve_transformed_all_lod_support_and_current_wind() {
        use crate::{cluster::GpuCluster,scene::{GeometryCategory,Scene}};
        use glam::{Affine3A,Quat};
        // Multiple real LOD/material descriptors. No source-only or opaque-only
        // filter may remove a coarse/glass bound that contains a visible point.
        for origin in [Vec3::ZERO,Vec3::new(100000.,10.,-75000.)] {
            let cams=[Vec3::Z,Vec3::X,-Vec3::Z,-Vec3::X].map(|axis|ReceiverCamera {
                view_projection:Mat4::perspective_infinite_reverse_rh(1.0,1.4,0.1)*Mat4::look_at_rh(origin+axis*18.,origin,Vec3::Y),map_mask:1});
            let map=matrix(origin,32.);let (config,_)=configuration(1,1,&cams,&[ShadowMap{view_projection:map,size:2048}],&[],0)
                .unwrap();
            let mut mesh=crate::cluster::ClusterMesh::default();mesh.sphere=[0.,0.,0.,8.];
            mesh.clusters=(0..4).map(|i|GpuCluster {cull:[i as f32-1.5,0.,0.,1.],own_error:i as f32,
                parent_error:f32::MAX,material:i,tri_count:1,..GpuCluster::zeroed()}).collect();
            let xf=Affine3A::from_scale_rotation_translation(Vec3::new(1.5,0.7,1.1),Quat::from_rotation_y(0.7),origin);
            let mut scene=Scene::default();let id=scene.add_mesh(&mesh);scene.add_classified_instance(id,xf,GeometryCategory::Terrain);
            let inst=scene.instances[0];let wind=0.65;
            for cluster in &mesh.clusters {
                let local=Vec3::from_slice(&cluster.cull);let center=xf.transform_point3(local);
                let radius=cluster.cull[3]*inst.scale+wind;
                assert_ne!(camera_mask(&config,center,radius),0);
                let (first,last)=rectangle(map,center,Vec3::splat(radius),2048,4.).unwrap();
                for axis in [Vec3::X,-Vec3::X,Vec3::Y,-Vec3::Y,Vec3::Z,-Vec3::Z,Vec3::ONE.normalize()] {
                    let actual=xf.transform_point3(local+axis*cluster.cull[3])+axis*wind;
                    let q=map*actual.extend(1.);let pixel=(Vec2::new(q.x*0.5+0.5,-q.y*0.5+0.5)*2048.).as_uvec2()/TILE_SIZE;
                    assert!(pixel.x>=first[0]&&pixel.y>=first[1]&&pixel.x<=last[0]&&pixel.y<=last[1]);
                }
            }
        }
    }
    #[test]
    fn separated_cluster_requests_do_not_inherit_scene_wide_owner_coverage() {
        let camera=ReceiverCamera {view_projection:Mat4::perspective_infinite_reverse_rh(0.9,1.5,0.1),map_mask:1};
        let map=matrix(Vec3::new(0.,0.,-20.),48.);
        let (config,_)=configuration(1,1,&[camera],&[ShadowMap{view_projection:map,size:2048}],&[],0).unwrap();
        let whole=rectangle(map,Vec3::new(0.,0.,-20.),Vec3::splat(501.),2048,4.).unwrap();
        assert_eq!(whole,([0,0],[31,31]));
        let mut selected=0;let mut mask=vec![false;32*32];
        for x in [-500.,-250.,0.,250.,500.] {
            let center=Vec3::new(x,0.,-20.);let radius=2.;
            if camera_mask(&config,center,radius)==0 {continue;}
            selected+=1;
            if let Some((a,b))=rectangle(map,center,Vec3::splat(radius),2048,4.) {
                for y in a[1]..=b[1] {for x in a[0]..=b[0] {mask[(y*32+x)as usize]=true;}}
            }
        }
        assert_eq!(selected,1);let requested=mask.into_iter().filter(|x|*x).count();
        assert!(requested>0&&requested<128,"cluster refinement must retain support but remove false whole-map demand");
    }
    fn hierarchy(mask:&[u32],tiles:usize)->Vec<Vec<u32>> {
        let mut levels=vec![mask.to_vec()];let mut dim=tiles;
        while dim>1 {let next=dim/2;let mut values=vec![0;next*next];
            for y in 0..next {for x in 0..next {let old=levels.last().unwrap();
                values[y*next+x]=old[(y*2)*dim+x*2]|old[(y*2)*dim+x*2+1]|old[(y*2+1)*dim+x*2]|old[(y*2+1)*dim+x*2+1];}}
            levels.push(values);dim=next;
        }
        levels
    }
    #[test]
    fn four_tap_or_query_never_loses_requested_tiles_for_sparse_rectangles() {
        for tiles in [1usize,2,4,8,16,32,64] {
            let mask=(0..tiles*tiles).map(|i|u32::from((i*17+i/tiles*31)%29==0)).collect::<Vec<_>>();
            let levels=hierarchy(&mask,tiles);
            for i in 0..8192usize {
                let x0=(i*7)%tiles;let x1=(i*53+3)%tiles;let y0=(i*11+1)%tiles;let y1=(i*37+5)%tiles;
                let first=[x0.min(x1),y0.min(y1)];let last=[x0.max(x1),y0.max(y1)];
                let exact=(first[1]..=last[1]).any(|y|(first[0]..=last[0]).any(|x|mask[y*tiles+x]!=0));
                let level=(0..levels.len()).find(|l|(last[0]>>l)-(first[0]>>l)<=1&&(last[1]>>l)-(first[1]>>l)<=1).unwrap();
                let dim=tiles>>level;let mut query=false;let mut taps=0;
                for y in (first[1]>>level)..=(last[1]>>level) {for x in (first[0]>>level)..=(last[0]>>level) {query|=levels[level][y*dim+x]!=0;taps+=1;}}
                assert!(taps<=4);assert!(!exact||query,"lost requested rectangle: {tiles} {first:?} {last:?}");
            }
        }
    }
    #[test]
    fn live_unique_tile_counts_stay_exact_with_duplicates_delayed_counts_and_fallback() {
        // Interleave duplicate atomicOr claims and delayed count increments.
        // Completion may lag, never certify unwritten demand. A subsequent
        // fallback uses the same 0->1 protocol rather than counting twice.
        for tiles in [1usize,4,64] {
            let total=tiles*tiles;let mut mask=vec![vec![false;total];5];let mut reference=mask.clone();
            let mut counts=[0usize;5];let mut unique=[0usize;5];let mut pending=Vec::new();
            let flush=|pending:&mut Vec<usize>,counts:&mut [usize;5],unique:&[usize;5],mask:&[Vec<bool>]| {
                if let Some(map)=pending.pop() {
                    counts[map]+=1;assert!(counts[map]<=unique[map]&&counts[map]<=total);
                    if counts[map]==total {assert!(mask[map].iter().all(|v|*v));}
                }
            };
            for i in 0..total*15 {
                let map=i%5;let tile=(i/5*17+map*7)%total;
                // Some maps deliberately remain partial before fallback.
                if map>=3&&tile%3==0 {continue;}
                reference[map][tile]=true;
                if counts[map]!=total&&!mask[map][tile] {
                    mask[map][tile]=true;unique[map]+=1;pending.push(map);
                }
                if i%2==0 {flush(&mut pending,&mut counts,&unique,&mask);}
                if i%17==0 {while !pending.is_empty() {flush(&mut pending,&mut counts,&unique,&mask);}}
            }
            while !pending.is_empty() {flush(&mut pending,&mut counts,&unique,&mask);}
            assert_eq!(mask,reference);
            for map in 0..5 {assert_eq!(counts[map],mask[map].iter().filter(|v|**v).count());}
            for map in 0..5 {for tile in 0..total {
                if !mask[map][tile] {mask[map][tile]=true;unique[map]+=1;pending.push(map);}
            }}
            while !pending.is_empty() {flush(&mut pending,&mut counts,&unique,&mask);}
            assert_eq!(counts,[total;5]);assert!(mask.iter().flatten().all(|v|*v));
        }
        let source=include_str!("shaders/receiver_shadow_demand.wgsl");
        assert!(source.contains("if atomicOr(&demand_words[tile],1u)==0u {atomicAdd(&demand_words[16u+map*32u+28u],1u);}"));
        assert!(source.contains("demand_mark_tile(gid.y,m.offsets[0].x+gid.x);"));
        assert!(!source.contains("fn demand_count("));
        assert_eq!(&ENTRIES[..4],&["demand_clear","demand_owners","demand_extras","demand_fallback"]);
        assert_eq!(&ENTRIES[4..],&["demand_reduce_1","demand_reduce_2","demand_reduce_3","demand_reduce_4","demand_reduce_5","demand_reduce_6","demand_publish"]);
    }
    #[test]
    fn cluster_completion_guard_includes_every_possible_camera_map() {
        let cameras=[1u32|2,1|4,1|8,1|16];let all_maps=cameras.iter().fold(0,|mask,c|mask|c);
        let total=4u32;
        let complete=|mask:u32,counts:[u32;5]|(0..5).all(|m|mask&(1<<m)==0||counts[m]==total);
        let mut counts=[total,total,0,0,0];
        assert!(complete(cameras[0],counts));
        assert!(!complete(all_maps,counts),"coarse owner mask cannot certify other views");
        for map in 2..5 {counts[map]=total;assert_eq!(complete(all_maps,counts),map==4);}
        let source=include_str!("shaders/receiver_shadow_demand.wgsl");
        assert!(source.contains("mask|=demand_config.cameras[camera].flags.x"));
        assert!(source.contains("let camera_maps=demand_camera_maps();"));
        assert!(source.contains("if demand_maps_complete(camera_maps) {break;}"));
        assert!(!source.contains("demand_maps_complete(mask)"));
    }
    #[test]
    fn bounded_configuration_keeps_four_view_map_identity_and_fails_open_for_providers() {
        let maps=(0..5).map(|i|ShadowMap {view_projection:matrix(Vec3::X*i as f32*100.,if i==0 {600.}else{16.}),size:if i==0 {4096}else{2048}}).collect::<Vec<_>>();
        let cameras=(0..4).map(|i|ReceiverCamera {view_projection:Mat4::perspective_rh(1.,1.5,0.1,2000.),map_mask:1|(1<<(i+1))}).collect::<Vec<_>>();
        let extra=ReceiverBounds {min:Vec3::ZERO,max:Vec3::ONE,map_mask:31};
        let (c,_)=configuration(7,900000,&cameras,&maps,&[extra],0).unwrap();
        assert_eq!(c.header,[7,5,4,900000]);assert_eq!(c.control,[1,0,0,0]);
        for i in 0..4 {assert_eq!(c.cameras[i].flags[0],1|(1<<(i+1)));}
        assert_eq!(c.maps[4].offsets[5]+1,BUFFER_WORDS);
        let (c,_)=configuration(8,0,&cameras,&maps,&vec![extra;MAX_EXTRA_BOUNDS+1],0).unwrap();
        assert_ne!(c.control[1]&EXTRA_CAPACITY,0);
        let (c,valid)=configuration(9,0,&cameras,&maps,&[ReceiverBounds {min:Vec3::splat(f32::NAN),..extra}],UNREGISTERED_PROVIDER).unwrap();
        assert_eq!(c.control[1],INVALID_BOUNDS|UNREGISTERED_PROVIDER);assert!(valid.is_empty());
        assert!(configuration(0,0,&cameras,&maps,&[],0).is_err());
        let perspective=[ShadowMap {view_projection:Mat4::perspective_rh(1.,1.,0.1,100.),size:2048}];
        assert!(configuration(1,0,&[],&perspective,&[],0).is_err());
    }
    #[test]
    fn actual_mask_readback_rejects_counter_corruption() {
        let mut words=vec![0;BUFFER_WORDS as usize];words[..9].copy_from_slice(&[1,7,7,0,1,10,10,1,0]);words[9]=1;
        let b=HEADER_WORDS as usize;words[b+16]=128;words[b+17]=2;words[b+20]=MASK_START;
        words[MASK_START as usize]=1;words[b+28]=1;words[b+30]=5;words[b+31]=3;
        let d=decode(&words,44388).unwrap();assert_eq!(d.maps[0].mask,[1,0,0,0]);assert_eq!(d.maps[0].requested_tiles,1);
        words[b+28]=0;assert!(decode(&words,44388).is_err());words[b+28]=1;words[b+31]=6;assert!(decode(&words,44388).is_err());
        words[b+31]=3;words[9]=0;words[10]=1;words[11]=2;words[12]=1;words[13]=1;
        let d=decode(&words,44388).unwrap();assert_eq!(d.receiver_cluster_tests,(1u64<<32)+2);assert_eq!(d.receiver_cluster_bounds,1);
        words[9]=1;assert!(decode(&words,44388).is_err());words[9]=0;
        words[14]=2;assert!(decode(&words,44388).is_err());
        words[14]=0;words[b+19]=7;assert!(decode(&words,44388).is_err());
        words[MASK_START as usize..MASK_START as usize+4].fill(1);words[b+28]=4;words[b+30]=0;words[b+31]=0;
        let d=decode(&words,44388).unwrap();assert_eq!(d.maps[0].caster_clusters_full_demand,7);
        assert_eq!(d.maps[0].caster_clusters_tested,0);assert_eq!(d.maps[0].caster_clusters_rejected,0);
    }
    #[test]
    fn published_full_map_shader_keeps_generation_and_map_guards_before_fastpath() {
        let source=crate::frame::cull_source();
        let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        assert!(module.functions.iter().any(|(_,f)|f.name.as_deref()==Some("receiver_shadow_full_map")));
        // This checks the compiled shader and safety-gate ordering, not GPU
        // execution. The real-renderer full-demand fixture is the behavior
        // oracle; do not partially interpret Naga's short-circuit local IR.
        let query=include_str!("shaders/receiver_shadow_query.wgsl");
        assert!(query.contains("return tiles>0u && tiles<=64u && requested==tiles*tiles;"));
        let fast=query.find("if receiver_shadow_full_map(").unwrap();
        assert!(fast>query.find("atomicLoad(&receiver_demand[2])!=generation").unwrap());
        assert!(fast>query.find("map>=atomicLoad(&receiver_demand[4])").unwrap());
        assert!(fast>query.find("if size==0u || tiles==0u || levels==0u").unwrap());
        assert!(fast<query.find("bitcast<u32>(view.view_proj[col][row])").unwrap());
        assert!(fast<query.find("let rect=demand_project(").unwrap());
        assert!(query.contains("atomicAdd(&receiver_demand[base+19u],1u);return true;"));
    }
    #[test]
    fn demand_shaders_and_cull_binding_budget_validate() {
        for source in [builder_source(),crate::frame::cull_source()] {
            let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
                .validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            assert!(module.global_variables.iter().filter(|(_,g)|matches!(g.space,naga::AddressSpace::Storage {..})).count()<=16);
            if let Some((_,ty))=module.types.iter().find(|(_,t)|t.name.as_deref()==Some("DemandConfig")) {
                let naga::TypeInner::Struct {span,..}=ty.inner else {panic!()};assert_eq!(span as usize,std::mem::size_of::<Config>());
            }
            if let Some((_,ty))=module.types.iter().find(|(_,t)|t.name.as_deref()==Some("DemandCluster")) {
                let naga::TypeInner::Struct {span,ref members}=ty.inner else {panic!()};
                assert_eq!(span as usize,std::mem::size_of::<crate::cluster::GpuCluster>());
                assert_eq!(members[0].offset,std::mem::offset_of!(crate::cluster::GpuCluster,cull)as u32);
                assert_eq!(members[8].offset,std::mem::offset_of!(crate::cluster::GpuCluster,material)as u32);
            }
        }
        assert_eq!(std::mem::size_of::<Extra>(),32);assert_eq!(std::mem::size_of::<GpuMap>(),128);assert_eq!(std::mem::size_of::<Config>(),1120);
        assert_eq!(BUFFER_WORDS*4,44388);
        let query=include_str!("shaders/receiver_shadow_query.wgsl");
        assert!(query.contains("view.params.z<0.5"));assert!(query.contains("atomicLoad(&receiver_demand[2])!=generation"));
        assert!(query.contains("atomicLoad(&receiver_demand[3])!=0u"));
        // Only the provenance category chooses request precision; all caster,
        // material and camera-depth policies are intentionally absent.
        let builder=include_str!("shaders/receiver_shadow_demand.wgsl").lines()
            .map(|line|line.split("//").next().unwrap()).collect::<Vec<_>>().join("\n");
        assert!(builder.contains("((inst.flags>>24u)&7u)!=1u"));assert!(!builder.contains("tree_hiz"));
        // The only instance-flag read chooses coarse versus refined support;
        // no caster flag (including literal NO_SHADOW bits) may exclude it.
        assert!(!builder.replace("inst.flags>>24u", "category").contains("inst.flags"));
        for predicate in ["parent_error>","own_error>",".material","NO_SHADOW","tri_count=="] {
            assert!(!builder.contains(predicate),"receiver exclusion predicate {predicate}");
        }
    }
    #[test]
    fn shadow_map_selector_cannot_change_alpha_mips_but_camera_ratio_still_does() {
        // Evaluate the actual compiled raster alpha-mip select, not a Rust
        // formula: the original near-map failure changed alpha gradients2..5x
        // even when the demand culler rejected zero near-map clusters.
        let source=crate::grass::raster_source();
        let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        let function=module.functions.iter().find(|(_,f)|f.name.as_deref()==Some("alpha_cut")).unwrap().1;
        let mip_scale=*function.named_expressions.iter().find(|(_,name)|name.as_str()=="mip_scale").unwrap().0;
        fn evaluate(f:&naga::Function,e:naga::Handle<naga::Expression>,extra_z:f32)->f32 {
            use naga::{BinaryOperator as B,Expression as E,Literal};
            match f.expressions[e] {
                E::Literal(Literal::F32(x))=>x,
                E::Load {pointer}=>evaluate(f,pointer,extra_z),
                E::AccessIndex {base,index:2}=>{
                    let E::AccessIndex {base,index:6}=f.expressions[base] else {panic!("expected View.extra.z")};
                    assert!(matches!(f.expressions[base],E::GlobalVariable(_)));extra_z
                }
                E::Binary {op,left,right}=>{
                    let a=evaluate(f,left,extra_z);let b=evaluate(f,right,extra_z);
                    let value=match op {B::Greater=>a>b,_=>panic!("unexpected alpha-mip operator {op:?}")};
                    if value {1.}else{0.}
                }
                E::Select {condition,accept,reject}=>evaluate(f,if evaluate(f,condition,extra_z)!=0. {accept}else{reject},extra_z),
                ref other=>panic!("unexpected alpha-mip expression {other:?}"),
            }
        }
        for map in 0..5 {let encoded=-((map+1)as f32);assert!(encoded<0.);assert_eq!(evaluate(function,mip_scale,encoded),1.);}
        for ratio in [0.25,0.5,0.6667,1.] {assert_eq!(evaluate(function,mip_scale,ratio),ratio);}
        assert_eq!(evaluate(function,mip_scale,0.),1.);
        let query=include_str!("shaders/receiver_shadow_query.wgsl");
        assert!(query.contains("view.params.z<0.5 || !(view.extra.z>=-5.0 && view.extra.z<=-1.0)"));
        assert!(query.contains("let map=u32(-view.extra.z)-1u;"));
    }
}
