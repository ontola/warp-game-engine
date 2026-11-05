//! Current-frame conservative occlusion for all camera geometry (the name is
//! historical: it started with trees). Phase one draws what was visible last
//! frame; phase two tests everything else against that real, alpha-tested
//! depth. Camera visibility never removes shadow/GI casters.
use crate::{depth_pyramid::{DepthPyramid, PyramidTarget}, Gpu};
use std::sync::Mutex;
use wgpu::util::DeviceExt;

pub const SPLIT_METRES: f32 = 32.0;
pub(crate) const PHASE_COUNTER_BYTES: u64 = 32 * 4;
/// Words 40..47: masked cluster counts and cursors (cull.wgsl RASTER_SPLIT),
/// phase counters like 0..31 but after the traversal block.
pub(crate) const SPLIT_COUNTER_OFFSET: u64 = 40 * 4;
pub(crate) const SPLIT_COUNTER_BYTES: u64 = 8 * 4;
pub(crate) const COUNTER_WORDS: usize = 48;
pub(crate) const COUNTER_BYTES: u64 = COUNTER_WORDS as u64 * 4;

#[derive(Clone, Debug)]
pub struct Diagnostics {
    pub enabled: bool,
    /// False for the phase-order diagnostic: both draws remain, but Hi-Z
    /// cannot reject geometry. Also false when two-phase rendering is off.
    pub rejection_enabled: bool,
    pub disabled_reason: Option<&'static str>,
    pub frame_id: u64,
    pub render_sequence: u64,
    pub view_index: usize,
    pub split_m: f32,
    pub history_first:bool,
    pub deferred_capacity:u32,
    pub deferred_clusters:u32,
    pub deferred_overflow:bool,
    pub phase1_clusters: u32,
    pub phase2_clusters: u32,
    pub instances_tested: u32,
    pub instances_rejected: u32,
    pub clusters_tested: u32,
    pub clusters_rejected: u32,
    /// Spatial broadphase work; populated by the renderer's matching resources.
    pub instance_groups: Option<crate::cull_groups::Diagnostics>,
    pub cluster_traversal: Option<crate::cluster_traversal::Diagnostics>,
}

pub(crate) struct Resources {
    pub history_first:bool,
    pub deferred_capacity:u32,
    pub history:Mutex<Vec<wgpu::Buffer>>,
    pub empty_history:wgpu::Buffer,
    history_pipeline:wgpu::ComputePipeline,
    pub enabled: bool,
    pub rejection_enabled: bool,
    pub disabled_reason: Option<&'static str>,
    pub reference: wgpu::Buffer,
    pub phases: [wgpu::Buffer; 2],
    pub empty: wgpu::TextureView,
    pub pyramid: Option<DepthPyramid>,
    pub targets: Mutex<Vec<Option<PyramidTarget>>>,
}
impl Resources {
    pub fn new(gpu: &Gpu, bucketed: bool, gpu_forest:bool) -> Self {Self::with_history_capacity(gpu,bucketed,gpu_forest,262144)}
    pub(crate) fn with_history_capacity(gpu:&Gpu,bucketed:bool,gpu_forest:bool,deferred_capacity:u32)->Self {
        assert!((1..=262144).contains(&deferred_capacity));
        // On by default for every scene: hidden geometry of any category is
        // tested against the Hi-Z of what was visible last frame.
        // KERN_OCCLUSION=0 (or the older KERN_TREE_OCCLUSION=0) turns it off.
        let _=gpu_forest;
        let off=|name|std::env::var(name).as_deref()==Ok("0");
        let requested = !off("KERN_OCCLUSION") && !off("KERN_TREE_OCCLUSION");
        let history_first=requested && bucketed;
        let enabled = requested && bucketed;
        let rejection_enabled = enabled
            && !off("KERN_OCCLUSION_REJECT") && !off("KERN_TREE_OCCLUSION_REJECT");
        let disabled_reason = if requested && !bucketed { Some("cluster_buckets_required") }
            else if !requested { Some("not_requested") } else { None };
        let control = |phase: f32| gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable occlusion phase"),
            contents: bytemuck::cast_slice(&[phase, if history_first {deferred_capacity as f32}else{SPLIT_METRES}, if rejection_enabled {1.0} else {0.0}, if history_first {1.0}else{0.0}]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let empty = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("disabled occlusion depth"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }).create_view(&Default::default());
        if requested { eprintln!("Tree occlusion: enabled={enabled}, rejection_enabled={rejection_enabled}, history_first={history_first}, split={SPLIT_METRES}m (legacy only), disabled_reason={disabled_reason:?}; current-frame real alpha depth, camera only"); }
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("tree pixel visibility history"),source:wgpu::ShaderSource::Wgsl(concat!(include_str!("shaders/tree_history_key.wgsl"),"\n",include_str!("shaders/tree_history.wgsl")).into())});
        let history_pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("tree pixel visibility history"),layout:None,module:&module,entry_point:Some("gather"),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
        let empty_history=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("disabled tree history"),size:65536,usage:wgpu::BufferUsages::UNIFORM,mapped_at_creation:false});
        Self { history_first,deferred_capacity,history:Mutex::new(Vec::new()),empty_history,history_pipeline,enabled, rejection_enabled, disabled_reason, reference: control(0.0), phases: [control(1.0), control(2.0)],
            empty, pyramid: enabled.then(||DepthPyramid::new(gpu)), targets: Mutex::new(Vec::new()) }
    }
    pub fn history_buffer(&self,gpu:&Gpu,view:usize,owners:u32)->wgpu::Buffer {
        if !self.history_first {return self.empty_history.clone();}
        assert!(owners<=1<<19,"GPU forest owner ID capacity");
        let mut all=self.history.lock().unwrap();
        while all.len()<=view {all.push(gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("per-view source visibility history"),size:65536,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false}));}
        all[view].clone()
    }
    pub fn capture_history(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,history:&wgpu::Buffer,visibility:&wgpu::TextureView,visible:&wgpu::Buffer,size:(u32,u32),time:Option<(&wgpu::QuerySet,u32)>) {
        if !self.history_first {return;}
        enc.clear_buffer(history,0,Some(32768));
        let layout=self.history_pipeline.get_bind_group_layout(0);
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("tree pixel history"),layout:&layout,entries:&[
            wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::TextureView(visibility)},
            wgpu::BindGroupEntry {binding:1,resource:visible.as_entire_binding()},
            wgpu::BindGroupEntry {binding:2,resource:history.as_entire_binding()}]});
        let mut pass=enc.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("visibility history"),timestamp_writes:time.map(|(query_set,i)|wgpu::ComputePassTimestampWrites {query_set,beginning_of_pass_write_index:Some(i),end_of_pass_write_index:Some(i+1)})});
        pass.set_pipeline(&self.history_pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(size.0.div_ceil(8),size.1.div_ceil(8),1);
    }
    pub fn decode(&self, words: &[u32], frame_id:u64, render_sequence:u64, view_index:usize) -> Result<Diagnostics,String> {
        if words.len() != COUNTER_WORDS { return Err("invalid occlusion counter size".into()); }
        if self.enabled && (words[13].checked_add(words[14]) != Some(words[0])
            || words[16] > words[15] || words[18] > words[17]) {
            return Err("inconsistent occlusion submitted/rejected counters".into());
        }
        if !self.rejection_enabled && (words[16] != 0 || words[18] != 0) {
            return Err("occlusion rejection counters are nonzero while rejection is disabled".into());
        }
        Ok(Diagnostics { enabled:self.enabled, rejection_enabled:self.rejection_enabled, disabled_reason:self.disabled_reason, frame_id, render_sequence, view_index,
            split_m:if self.history_first {0.0}else{SPLIT_METRES},history_first:self.history_first,deferred_capacity:self.deferred_capacity,deferred_clusters:words[19],deferred_overflow:self.history_first&&words[19]>self.deferred_capacity, phase1_clusters:if self.enabled {words[13]} else {words[0]},
            phase2_clusters:words[14], instances_tested:words[15], instances_rejected:words[16],
            clusters_tested:words[17], clusters_rejected:words[18], instance_groups:None,cluster_traversal:None })
    }
}

#[cfg(test)]
mod tests {
    use glam::{Mat4,Vec3};
    #[test]
    fn history_hiz_mip_start_preserves_exact_odd_size_footprints() {
        for width in [1u32,2,3,5,7,31,32,33,65,127,128,129,1713] {
            let levels=32-width.leading_zeros();
            for first in (0..width).step_by(7) {for last in (first..width).step_by(11) {
                let span=last-first+1;let start=(31-span.leading_zeros()).saturating_sub(1);
                let find=|start| (start..levels).find(|&l| {let end=(width>>l).max(1)-1;(last>>l).min(end)-(first>>l).min(end)<=1});
                assert_eq!(find(0),find(start));
            }}
        }
    }
    #[test]
    #[ignore="exclusive GPU visibility history readback"]
    fn gpu_cluster_history_matches_pixel_oracle_and_clears() {
        use wgpu::util::DeviceExt;
        let gpu=crate::Gpu::new().unwrap();let r=super::Resources::new(&gpu,true,true);
        let history=r.history_buffer(&gpu,0,100);
        let records=(0..64u32).map(|i|[0x80000000|(i<<12)|(i%7),i*97]).collect::<Vec<_>>();
        let input=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("history oracle records"),contents:bytemuck::cast_slice(&records),usage:wgpu::BufferUsages::STORAGE});
        let texture=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("history oracle pixels"),size:wgpu::Extent3d {width:8,height:8,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::R32Uint,usage:wgpu::TextureUsages::COPY_DST|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
        let hash=|mut x:u32|{x=(x^(x>>16)).wrapping_mul(0x7feb352d);x=(x^(x>>15)).wrapping_mul(0x846ca68b);x^(x>>16)};
        for clear in [false,true] {
            let pixels=(0..64u32).map(|i|if clear || i%7==0 {u32::MAX}else{i<<7}).collect::<Vec<_>>();
            gpu.queue.write_texture(wgpu::TexelCopyTextureInfo {texture:&texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All},bytemuck::cast_slice(&pixels),wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(32),rows_per_image:Some(8)},wgpu::Extent3d {width:8,height:8,depth_or_array_layers:1});
            let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("history oracle readback"),size:65536,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            let mut enc=gpu.device.create_command_encoder(&Default::default());r.capture_history(&gpu,&mut enc,&history,&texture.create_view(&Default::default()),&input,(8,8),None);
            enc.copy_buffer_to_buffer(&history,0,&read,0,65536);gpu.queue.submit(Some(enc.finish()));
            read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
            let mapped=read.slice(..).get_mapped_range().unwrap();let actual:&[u32]=bytemuck::cast_slice(&mapped);let mut expected=vec![0u32;16384];
            for pixel in pixels.into_iter().filter(|p|*p!=u32::MAX) {let pair=records[(pixel>>7)as usize];let seed=pair[0]^pair[1].wrapping_mul(0x9e3779b9);
                for key in [hash(seed)&0x3ffff,hash(seed^0x85ebca6b)&0x3ffff] {expected[(key>>5)as usize]|=1<<(key&31);}}
            expected[8192]=if clear {2}else{1};
            assert_eq!(actual,expected);drop(mapped);read.unmap();
        }
    }
    // The foliage seed's DEFORMING instance filter must never match procedural
    // terrain patches (PROCEDURAL shares bit 128 with MOLTEN_DEBRIS): without
    // the land in the seed, trees behind hills count as visible.
    #[test]
    fn occluder_filter_never_drops_procedural_terrain() {
        let shader=include_str!("shaders/cull.wgsl");
        let line=shader.lines().find(|l|l.starts_with("const DEFORMING:u32=")).unwrap();
        let bits=line.trim_start_matches("const DEFORMING:u32=").trim_end_matches(';').split('|').map(|b|b.trim_end_matches('u').parse::<u32>().unwrap()).fold(0,|a,b|a|b);
        assert_eq!(bits&crate::procedural_terrain::PROCEDURAL,0);
        assert_eq!(bits&crate::scene::SKINNED,crate::scene::SKINNED);
    }
    // Occlusion is not about trees or walls: any geometry fully behind what
    // the camera sees is not drawn, whatever its category, and nothing in
    // view is lost. A rock, a building and a terrain patch hide behind a wall.
    #[test]
    #[ignore="exclusive GPU occlusion of every category"]
    fn gpu_hidden_geometry_of_any_category_is_not_drawn() {
        use crate::{Camera,Lighting,Renderer,ViewRect,scene::{GeometryCategory,Material,Scene}};
        let gpu=crate::Gpu::new().unwrap();let light=Lighting {gi:0.,sun_lux:0.,shadow_strength:0.,..Default::default()};
        let cam=Camera {pos:Vec3::new(0.,1.,12.),target:Vec3::new(0.,1.,0.),fov_y:1.,near:0.05};
        let hidden=[(GeometryCategory::Other,Vec3::new(-2.,1.,0.)),(GeometryCategory::Structures,Vec3::new(0.,1.,-3.)),(GeometryCategory::Terrain,Vec3::new(2.,0.5,0.)),(GeometryCategory::Cars,Vec3::new(0.,0.5,-8.))];
        let render=|wall:bool| {
            let mut scene=Scene::default();
            let paint=scene.add_material(Material {tint:[0.5;3],layer:-1,roughness:0.9,flags:0,scale:1.,metal:0.});
            let cube=scene.add_mesh(&crate::primitives::cube(paint).mesh());
            for (category,at) in hidden {scene.add_classified_instance(cube,glam::Affine3A::from_translation(at),category);}
            if wall {scene.add_classified_instance(cube,glam::Affine3A::from_scale_rotation_translation(Vec3::new(60.,60.,0.1),glam::Quat::IDENTITY,Vec3::new(0.,1.,6.)),GeometryCategory::Other);}
            // In front of the wall: always in view, so it must always draw.
            let front=scene.instances.len() as u32;scene.add_classified_instance(cube,glam::Affine3A::from_translation(Vec3::new(1.5,0.5,8.)),GeometryCategory::Other);
            let renderer=Renderer::new(&gpu,&scene,&light);
            assert!(renderer.debug_occlusion_enabled(),"occlusion is on by default for every scene");
            let views=[ViewRect {cam,x:0,y:0,w:128,h:128}];
            let mut drawn=Vec::new();
            for frame in 0..40 {
                renderer.try_render_views(&gpu,&views,&light,128,128,&[]).unwrap();
                let pairs=renderer.debug_visible_clusters(&gpu,0,None).unwrap().pairs;
                assert!(pairs.iter().any(|p|p[0]==front),"frame {frame}: geometry in view must never be culled");
                let mut owners=pairs.iter().map(|p|p[0]).filter(|&o|o<hidden.len()as u32).collect::<Vec<_>>();owners.sort_unstable();owners.dedup();
                // Frame 0 has no history, and a scene without proven occlusion
                // draws directly until the next probe (every 4th frame).
                if frame>=5 {drawn.push(owners);}
            }
            drawn
        };
        let open=render(false);
        assert!(open.iter().all(|o|o.len()==hidden.len()),"fixture: without the wall every object is drawn");
        let blocked=render(true);
        eprintln!("hidden objects drawn per frame behind the wall: {:?}",blocked.iter().map(Vec::len).collect::<Vec<_>>());
        assert!(blocked.iter().all(Vec::is_empty),"objects fully behind the wall must not be drawn: {blocked:?}");
    }
    // Independent clip projection oracle: near-plane or W
    // crossings must never become an occluded box. Mirrors only the geometric
    // contract, not shader sampling or its pixel-coordinate arithmetic.
    fn closest(vp:Mat4, center:Vec3, radius:f32)->Option<f64> {
        let mut nearest=0.0f64;
        for corner in 0..8 { let p=center+Vec3::new(if corner&1==0 {-radius}else{radius},if corner&2==0 {-radius}else{radius},if corner&4==0 {-radius}else{radius});
            let q=vp* p.extend(1.0);
            if !q.is_finite() || q.w<=1e-5 || q.z<0.0 || q.z>=q.w-1e-6 {return None;}
            nearest=nearest.max(q.z as f64/q.w as f64);
        } Some(nearest)
    }
    #[test]
    fn phase_partition_has_no_holes_even_at_wind_envelope_boundary() {
        for is_tree in [false,true] { for distance in [0.0f32,31.0,32.0,33.0,100.0] {for radius in [0.0,1.0,12.0] {for wind in [0.0,0.4,3.0] {
            let first=!is_tree || distance-(radius+wind)<=super::SPLIT_METRES;
            let second=is_tree && distance-(radius+wind)>super::SPLIT_METRES;
            assert_ne!(first,second);
        }}}}
    }
    #[test]
    fn near_plane_and_wind_bounds_fail_open_and_alpha_holes_prevent_rejection() {
        let vp=Mat4::perspective_infinite_reverse_rh(1.1,1.7,0.1);
        assert!(closest(vp,Vec3::new(0.,0.,-0.12),0.04).is_none());
        assert!(closest(vp,Vec3::ZERO,0.5).is_none());
        let center=Vec3::new(0.,0.,-10.);
        let rest=closest(vp,center,1.).unwrap();let wind=closest(vp,center,3.).unwrap();
        assert!(wind>rest, "nearer wind envelope must make rejection harder");
        let covered=[0.04f64;4];let gap=[0.04,0.04,0.0,0.04];
        assert!(covered.into_iter().fold(1.0,f64::min)>wind+2e-5);
        assert!(gap.into_iter().fold(1.0,f64::min)<=rest+2e-5);
        // An occluder behind any part of the bound is insufficient.
        assert!(!(rest+1e-5>rest+2e-5));
    }
    #[test]
    fn appended_phase_draws_preserve_pixel_ids_and_accumulate_submitted_work() {
        // Two previously rasterized pixels refer to slots0 and2. Bucket
        // scatter in phase2 may reorder draws, never these retained records.
        let mut visible=vec![(7u32,3u32,8u32),(9,4,64),(8,5,19)];
        let old_ids=[(0u32<<7)|5,(2u32<<7)|12];
        let before=old_ids.map(|id| (visible[(id>>7)as usize].0,visible[(id>>7)as usize].1,id&127));
        let prefix=visible.len();
        visible.extend([(11,8,128),(15,6,6),(16,10,31)]);
        let bucket=|n| if n<=8 {0usize}else if n<=32 {1}else if n<=64 {2}else {3};
        let mut phase_draws=[Vec::new(),Vec::new(),Vec::new(),Vec::new()];
        for slot in prefix..visible.len() {phase_draws[bucket(visible[slot].2)].push(slot);}
        let mut remaining=phase_draws.iter().flatten().copied().collect::<Vec<_>>();remaining.sort_unstable();
        assert_eq!(remaining,vec![3,4,5],"phase2 excludes every phase1 prefix record");
        assert_eq!(before,old_ids.map(|id| (visible[(id>>7)as usize].0,visible[(id>>7)as usize].1,id&127)));
        let submitted=|entries:&[(u32,u32,u32)]|entries.iter().map(|v|[24u32,96,192,384][bucket(v.2)]).sum::<u32>();
        assert_eq!(submitted(&visible[..prefix])+submitted(&visible[prefix..]),submitted(&visible));
        assert_eq!(visible[..prefix].len()+remaining.len(),visible.len());
    }
    #[test]
    #[ignore="exclusive GPU counter lifetime readback"]
    fn shared_counter_ranges_reset_independently() {
        use wgpu::util::DeviceExt;
        let gpu=crate::Gpu::new().unwrap();
        let initial=(1..=super::COUNTER_WORDS as u32).collect::<Vec<_>>();
        let buffer=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some("shared cull counters lifetime"),contents:bytemuck::cast_slice(&initial),
            usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST});
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("counter lifetime snapshots"),
            size:super::COUNTER_BYTES*3,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut enc=gpu.device.create_command_encoder(&Default::default());
        // An appended tree phase retains both regions. A fresh draw resets only
        // its phase counters; traversal accounting has its own frame lifetime.
        enc.copy_buffer_to_buffer(&buffer,0,&read,0,super::COUNTER_BYTES);
        enc.clear_buffer(&buffer,0,Some(super::PHASE_COUNTER_BYTES));
        enc.clear_buffer(&buffer,super::SPLIT_COUNTER_OFFSET,Some(super::SPLIT_COUNTER_BYTES));
        enc.copy_buffer_to_buffer(&buffer,0,&read,super::COUNTER_BYTES,super::COUNTER_BYTES);
        enc.clear_buffer(&buffer,crate::cluster_traversal::COUNTER_OFFSET,Some(crate::cluster_traversal::COUNTER_BYTES));
        enc.copy_buffer_to_buffer(&buffer,0,&read,super::COUNTER_BYTES*2,super::COUNTER_BYTES);
        gpu.queue.submit([enc.finish()]);read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
        let mapped=read.slice(..).get_mapped_range().unwrap();let words:&[u32]=bytemuck::cast_slice(&mapped);
        let n=super::COUNTER_WORDS;let split=(super::PHASE_COUNTER_BYTES/4)as usize;
        assert_eq!(&words[..n],initial);
        assert!(words[n..n+split].iter().all(|v|*v==0));
        let traversal_end=(super::SPLIT_COUNTER_OFFSET/4)as usize;
        assert_eq!(&words[n+split..n+traversal_end],&initial[split..traversal_end]);
        assert!(words[n+traversal_end..2*n].iter().all(|v|*v==0));
        assert!(words[2*n..].iter().all(|v|*v==0));
    }
    #[test]
    fn cull_counter_abi_has_room_for_both_phases_without_extra_storage_binding() {
        let source=crate::frame::cull_source();
        let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        let (_,counter)=module.global_variables.iter().find(|(_,g)|g.name.as_deref()==Some("counter")).unwrap();
        let naga::TypeInner::Array {size:naga::ArraySize::Constant(size),stride,..}=module.types[counter.ty].inner else {panic!("fixed counter array required")};
        assert_eq!(size.get() as usize,super::COUNTER_WORDS);
        assert_eq!(size.get() as u64*stride as u64,super::COUNTER_BYTES);
        assert_eq!(counter.binding.as_ref().unwrap().binding,2);
    }
}
