//! GPU selection of a branch aggregate and its shared fine source members.
//! The cooker measures both candidate representations against the same fine
//! optical reference. Sampled optical evidence is explicit, not an exact bound.
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub max_error_pixels: f32,
    pub max_coverage_error: f32,
    pub max_color_error: f32,
    pub allow_sampled_optics: bool,
    /// Source groups keep the measured world-error triangle cut the
    /// comparison fixtures need. Off, they use the renderer's ordinary
    /// screen-error cluster LOD and traversal, as games want.
    pub measured_source_cuts: bool,
    /// Whole-plant proxies (SceneVegetation::append_proxy) use the
    /// coarsest level whose cell stays under this many pixels.
    pub proxy_pixels: f32,
    /// Past this camera distance (m) a proxy's pixel error grows with
    /// sqrt(distance / proxy_pixels_from), so far forest costs far fewer
    /// splats. Zero keeps proxy_pixels at every distance.
    pub proxy_pixels_from: f32,
}
impl Default for Policy {
    fn default() -> Self {
        Self { max_error_pixels: 1.0, max_coverage_error: 0.08,
            max_color_error: 0.08, allow_sampled_optics: false, measured_source_cuts: true, proxy_pixels: 4.0, proxy_pixels_from: 50.0 }
    }
}
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    pub source_only: bool,
    pub level: u32,
    pub spatial_error: f32,
    pub voxel_coverage_error: f32,
    pub voxel_color_error: f32,
    pub triangle_coverage_error: f32,
    pub triangle_color_error: f32,
    /// The exact assembly-local triangle CLOD cut used by the optical oracle.
    pub triangle_error: f32,
}
#[derive(Clone, Debug)]
pub struct Group {
    pub volume: u32,
    pub center: Vec3,
    pub radius: f32,
    pub scale: f32,
    /// Optional containing whole-plant aggregate, in this selection's group order.
    pub parent: Option<u32>,
    pub candidates: Vec<Candidate>,
}
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub groups: Vec<Group>,
    pub policy: Policy,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Descriptor { sphere: [f32;4], info: [u32;4], scale: [f32;4] }
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Level { errors: [f32;4], triangle: [f32;4], info: [u32;4] }
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform { vp: [f32;16], output: [f32;4], policy: [f32;4], info: [u32;4] }

fn pack(selection: &Selection) -> Result<(Vec<Descriptor>,Vec<Level>),String> {
    let p=selection.policy;
    if selection.groups.is_empty() || selection.groups.len()>crate::voxel_forest::MAX_VIEW_GROUPS
        || !p.max_error_pixels.is_finite() || p.max_error_pixels<=0.
        || !p.max_coverage_error.is_finite() || !(0.0..=1.0).contains(&p.max_coverage_error)
        || !p.max_color_error.is_finite() || !(0.0..=1.0).contains(&p.max_color_error) {
        return Err("invalid aggregate selection capacity/error policy".into());
    }
    let mut groups=Vec::new();let mut levels=Vec::new();let mut ids=std::collections::BTreeSet::new();
    for (group_index,g) in selection.groups.iter().enumerate() {
        if !ids.insert(g.volume) || g.volume as usize>=crate::voxel_forest::MAX_VIEW_GROUPS
            || !g.center.is_finite() || !g.radius.is_finite() || g.radius<=0.
            || !g.scale.is_finite() || g.scale<=0. || g.candidates.is_empty()
            || g.parent.is_some_and(|p|p as usize>=group_index || selection.groups[p as usize].parent.is_some()) {
            return Err("invalid aggregate group bounds/identity".into());
        }
        let first=levels.len()as u32;let mut previous=None;let mut previous_source=0.0;
        for c in &g.candidates {
            if [c.spatial_error,c.voxel_coverage_error,c.voxel_color_error,
                c.triangle_coverage_error,c.triangle_color_error,c.triangle_error].iter().any(|x|!x.is_finite()||*x<0.)
                || (!c.source_only && previous.is_some_and(|i|c.level<=i))
                || (c.source_only && (previous.is_some() || c.triangle_error<=previous_source)) {
                return Err("invalid aggregate candidate error/order".into());
            }
            if c.source_only {previous_source=c.triangle_error;} else {previous=Some(c.level);}
            levels.push(Level { errors:[c.spatial_error,c.voxel_coverage_error,c.voxel_color_error,0.],
                triangle:[c.triangle_coverage_error,c.triangle_color_error,c.triangle_error,0.],info:[c.level,u32::from(c.source_only),0,0] });
        }
        groups.push(Descriptor {sphere:g.center.extend(g.radius).to_array(),
            info:[g.volume,first,g.candidates.len()as u32,g.parent.unwrap_or(u32::MAX)],scale:[g.scale,0.,0.,0.]});
    }
    Ok((groups,levels))
}

pub(crate) fn validate(selection:&Selection)->Result<(),String> { pack(selection).map(|_|()) }

pub(crate) fn validate_scene(scene:&crate::Scene)->Result<(),String> {
    let Some(selection)=&scene.aggregate_lod else {return Ok(());};
    validate(selection)?;
    let forest=scene.voxel_forest.as_ref().ok_or("aggregate selection requires its voxel provider")?;
    // Whole-plant proxies follow the groups and have no selection.
    if selection.groups.len()+scene.voxel_proxies as usize!=forest.instances.len() {
        return Err("aggregate selector must cover every volume exactly once".into());
    }
    for group in &selection.groups {
        let tree=forest.instances.get(group.volume as usize).ok_or("aggregate volume out of bounds")?;
        if group.candidates.iter().any(|c|c.level>=tree.meta[2]) {
            return Err("aggregate candidate level outside packed asset".into());
        }
        if !scene.foliage_representations.iter().any(|b|b.volume==group.volume&&b.owner==tree.meta[0]) {
            return Err("aggregate group has no matching source representation".into());
        }
    }
    Ok(())
}

pub(crate) struct Selector {
    groups:wgpu::Buffer, levels:wgpu::Buffer, layout:wgpu::BindGroupLayout,
    pipeline:wgpu::ComputePipeline, count:u32, policy:Policy,
}
impl Selector {
    pub(crate) fn new(gpu:&Gpu,selection:&Selection)->Result<Self,String> {
        let (groups,levels)=pack(selection)?;
        let upload=|label,data:&[u8]|gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:Some(label),contents:data,usage:wgpu::BufferUsages::STORAGE });
        crate::shaders::checked(gpu,|| {
            let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label:Some("aggregate error selection"),entries:&(0..4).map(|binding|wgpu::BindGroupLayoutEntry {
                    binding,visibility:wgpu::ShaderStages::COMPUTE,count:None,
                    ty:wgpu::BindingType::Buffer {ty:if binding==0 {wgpu::BufferBindingType::Uniform}
                        else{wgpu::BufferBindingType::Storage {read_only:binding!=3}},has_dynamic_offset:false,min_binding_size:None}
                }).collect::<Vec<_>>() });
            let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("aggregate error selection"),
                bind_group_layouts:&[Some(&layout)],immediate_size:0});
            let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("aggregate error selection"),
                source:wgpu::ShaderSource::Wgsl(include_str!("shaders/aggregate_lod.wgsl").into())});
            let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some("aggregate error selection"),
                layout:Some(&pl),module:&shader,entry_point:Some("select_aggregate"),compilation_options:Default::default(),cache:gpu.pipeline_cache()});
            Self { groups:upload("aggregate hierarchy groups",bytemuck::cast_slice(&groups)),
                levels:upload("aggregate measured candidate errors",bytemuck::cast_slice(&levels)),
                layout,pipeline,count:groups.len()as u32,policy:selection.policy }
        })
    }
    pub(crate) fn encode(&self,gpu:&Gpu,encoder:&mut wgpu::CommandEncoder,selection:&wgpu::Buffer,
        vp:Mat4,size:[u32;2],enabled:bool,time:Option<(&wgpu::QuerySet,u32)>)->Result<(),String> {
        if !vp.is_finite()||size.contains(&0){return Err("invalid aggregate selection view".into());}
        let u=Uniform {vp:vp.to_cols_array(),output:[size[0]as f32,size[1]as f32,self.policy.max_error_pixels,0.],
            policy:[self.policy.max_coverage_error,self.policy.max_color_error,if self.policy.measured_source_cuts {1.} else {0.},0.],
            info:[self.count,u32::from(enabled&&self.policy.allow_sampled_optics),0,0]};
        let uniform=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("aggregate selection view"),
            contents:bytemuck::bytes_of(&u),usage:wgpu::BufferUsages::UNIFORM});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("aggregate selection"),layout:&self.layout,
            entries:&[&uniform,&self.groups,&self.levels,selection].into_iter().enumerate().map(|(binding,b)|wgpu::BindGroupEntry {
                binding:binding as u32,resource:b.as_entire_binding()}).collect::<Vec<_>>()});
        let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some("aggregate error selection"),
            timestamp_writes:time.map(|(query_set,i)|wgpu::ComputePassTimestampWrites {query_set,
                beginning_of_pass_write_index:Some(i),end_of_pass_write_index:Some(i+1)})});
        pass.set_pipeline(&self.pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups(self.count.div_ceil(64),1,1);Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->Selection {
        Selection {groups:vec![Group {volume:0,center:Vec3::ZERO,radius:1.,scale:1.,parent:None,
            candidates:vec![Candidate {source_only:false,level:0,spatial_error:0.1,voxel_coverage_error:0.01,
                voxel_color_error:0.01,triangle_coverage_error:0.5,triangle_color_error:0.5,triangle_error:0.1}]}],
            policy:Policy::default()}
    }
    #[test] fn rejects_invalid_hierarchy_identity_and_error_evidence() {
        let good=fixture();assert!(validate(&good).is_ok());
        let mut bad=good.clone();bad.groups[0].parent=Some(0);assert!(validate(&bad).is_err());
        let mut bad=good.clone();bad.groups.push(bad.groups[0].clone());assert!(validate(&bad).is_err());
        let mut bad=good.clone();bad.groups[0].candidates[0].voxel_coverage_error=f32::NAN;assert!(validate(&bad).is_err());
        let mut bad=good.clone();let duplicate=bad.groups[0].candidates[0];bad.groups[0].candidates.push(duplicate);assert!(validate(&bad).is_err());
        let mut bad=good;bad.policy.max_error_pixels=0.;assert!(validate(&bad).is_err());
    }

    #[test]
    fn source_refinement_prefix_keeps_distinct_shader_flags_and_monotonic_cuts() {
        let mut selection=fixture();let voxel=selection.groups[0].candidates[0];
        selection.groups[0].candidates=[0.001,0.004,0.016].into_iter().map(|error|Candidate {
            source_only:true,level:0,spatial_error:error,triangle_error:error,
            triangle_coverage_error:0.01,triangle_color_error:0.01,..voxel
        }).chain([voxel]).collect();
        let (groups,levels)=pack(&selection).unwrap();
        assert_eq!(groups[0].info[1..3],[0,4]);
        assert_eq!(levels.iter().map(|l|l.info[1]).collect::<Vec<_>>(),[1,1,1,0]);
        assert_eq!(levels.iter().map(|l|l.info[0]).collect::<Vec<_>>(),[0,0,0,0]);
        for (packed,source) in levels.iter().zip(&selection.groups[0].candidates) {
            assert_eq!(packed.triangle[2].to_bits(),source.triangle_error.to_bits());
            assert_eq!(packed.errors[0].to_bits(),source.spatial_error.to_bits());
        }
        let mut bad=selection.clone();bad.groups[0].candidates.swap(0,1);
        assert!(validate(&bad).is_err());
        let mut bad=selection.clone();bad.groups[0].candidates[1].triangle_error=0.001;
        assert!(validate(&bad).is_err());
        let mut bad=selection.clone();bad.groups[0].candidates.swap(2,3);
        assert!(validate(&bad).is_err());
        let mut bad=selection;bad.groups[0].candidates[0].triangle_error=0.0;
        assert!(validate(&bad).is_err());
    }
    #[test] fn shader_validates() {
        let source=include_str!("shaders/aggregate_lod.wgsl");
        let m=naga::front::wgsl::parse_str(source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
            .validate(&m).unwrap_or_else(|e|panic!("{}",e.emit_to_string(source)));
    }

    #[test]
    #[ignore = "requires an exclusive GPU fixture slot"]
    fn gpu_selection_preserves_hierarchy_exact_triangle_cuts_and_single_scale() {
        fn decisions(gpu:&Gpu,selector:&Selector,slots:usize,vp:Mat4,enabled:bool)->Vec<[u32;4]> {
            // Sparse volume IDs deliberately differ from hierarchy indices.
            // Untouched slots expose accidental writes using the latter.
            let initial=vec![[99u32;4];slots];
            let output=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label:Some("aggregate selector oracle decisions"),contents:bytemuck::cast_slice(&initial),
                usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC });
            let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label:Some("aggregate selector oracle readback"),size:output.size(),
                usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false });
            let mut encoder=gpu.device.create_command_encoder(&Default::default());
            selector.encode(gpu,&mut encoder,&output,vp,[128;2],enabled,None).unwrap();
            encoder.copy_buffer_to_buffer(&output,0,&read,0,output.size());
            let submission=gpu.queue.submit([encoder.finish()]);
            let (send,receive)=std::sync::mpsc::sync_channel(1);
            read.slice(..).map_async(wgpu::MapMode::Read,move |result|{let _=send.send(result);});
            gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),
                timeout:Some(std::time::Duration::from_secs(30))}).unwrap();
            receive.try_recv().unwrap().unwrap();
            let bytes=read.slice(..).get_mapped_range().unwrap();
            bytes.chunks_exact(16).map(bytemuck::pod_read_unaligned).collect()
        }
        let gpu=Gpu::new().unwrap();
        let voxel=Candidate {source_only:false,level:3,spatial_error:0.1,voxel_coverage_error:0.01,
            voxel_color_error:0.01,triangle_coverage_error:0.5,triangle_color_error:0.5,triangle_error:0.075};
        let mut selection=Selection {
            groups:vec![
                Group {volume:5,center:Vec3::ZERO,radius:1.0,scale:1.0,parent:None,candidates:vec![voxel]},
                Group {volume:2,center:Vec3::X*0.2,radius:0.3,scale:0.5,parent:Some(0),candidates:vec![voxel]},
                Group {volume:7,center:-Vec3::X*0.2,radius:0.3,scale:2.0,parent:Some(0),candidates:vec![voxel]},
                Group {volume:4,center:Vec3::X*1000.0,radius:1.0,scale:1.0,parent:None,candidates:vec![voxel]},
            ],policy:Policy {allow_sampled_optics:true,..Default::default()},
        };
        let camera=|distance:f32|Mat4::perspective_infinite_reverse_rh(1.0,1.0,0.1)
            *Mat4::look_at_rh(Vec3::Z*distance,Vec3::ZERO,Vec3::Y);
        let selector=Selector::new(&gpu,&selection).unwrap();
        let near=decisions(&gpu,&selector,8,camera(3.0),true);
        for id in [5,2,7] {assert_eq!(near[id],[0,0,0,1],"near volume {id} must retain finest source");}
        assert_eq!(near[4],[2,0,0,1],"a fully outside envelope is culled");
        let far=decisions(&gpu,&selector,8,camera(100.0),true);
        assert_eq!(far[5],[1,3,0,1],"the lower optical error voxel wins far away");
        for id in [2,7] {assert_eq!(far[id],[2,0,0,1],"the parent replaces every child exactly once");}
        for id in [0,1,3,6] {assert_eq!(far[id],[99;4],"selection must address volume IDs");}
        let disabled=decisions(&gpu,&selector,8,camera(100.0),false);
        for id in [5,2,7,4] {assert_eq!(disabled[id],[0,0,0,1],"disabled selects explicit finest source even outside the view");}

        // A measured parent triangle cut owns all children. Their independent
        // voxel preferences and different scales cannot modify that exact cut.
        selection.groups[0].scale=2.0;
        selection.groups[0].candidates[0]=Candidate {voxel_coverage_error:0.04,voxel_color_error:0.04,
            triangle_coverage_error:0.01,triangle_color_error:0.01,..voxel};
        let triangle_selector=Selector::new(&gpu,&selection).unwrap();
        let triangle=decisions(&gpu,&triangle_selector,8,camera(100.0),true);
        let inherited=[0,0,(voxel.triangle_error*2.0).to_bits(),1];
        for id in [5,2,7] {assert_eq!(triangle[id],inherited,"parent's world-space triangle cut must propagate unchanged");}

        // Orthographic projection removes distance and perspective variation.
        // Its screen derivative is sqrt(8^2+8^2) pixels/metre. Each radius is
        // already world-space; candidate error and triangle cut scale once.
        let ortho=Mat4::orthographic_rh(-8.0,8.0,-8.0,8.0,0.1,100.0)
            *Mat4::look_at_rh(Vec3::Z*10.0,Vec3::ZERO,Vec3::Y);
        let ppm=8.0f32*2.0f32.sqrt();
        let scaled=Selection {groups:vec![
            Group {volume:0,center:Vec3::ZERO,radius:0.5,scale:0.5,parent:None,
                candidates:vec![Candidate {spatial_error:1.25/(ppm*0.5),..voxel}]},
            Group {volume:1,center:Vec3::ZERO,radius:2.0,scale:2.0,parent:None,
                candidates:vec![Candidate {spatial_error:0.8/(ppm*2.0),..voxel}]},
        ],policy:selection.policy};
        let scaled_selector=Selector::new(&gpu,&scaled).unwrap();
        let choices=decisions(&gpu,&scaled_selector,2,ortho,true);
        assert_eq!(choices[0],[0,0,0,1],"0.5 scale must reject 1.25px; squaring scale would incorrectly accept 0.625px");
        assert_eq!(choices[1],[1,3,0,1],"2.0 scale must accept 0.8px; squaring scale would incorrectly reject 1.6px");
        let mut cuts=scaled;
        for group in &mut cuts.groups {group.candidates[0]=Candidate {spatial_error:0.01,
            voxel_coverage_error:0.04,voxel_color_error:0.04,triangle_coverage_error:0.01,triangle_color_error:0.01,..voxel};}
        let cuts_selector=Selector::new(&gpu,&cuts).unwrap();
        let choices=decisions(&gpu,&cuts_selector,2,ortho,true);
        for (id,scale) in [0.5,2.0].into_iter().enumerate() {
            assert_eq!(choices[id],[0,0,(voxel.triangle_error*scale).to_bits(),1]);
        }

        // A source-only candidate has no voxel alternative, even when its
        // unused voxel diagnostics are lower and the optical budget is broad.
        // The later voxel level is too large on screen; it must not erase the
        // already accepted finer triangle cut.
        let refinement=Candidate {source_only:true,level:0,spatial_error:0.01,triangle_error:0.005,
            voxel_coverage_error:0.0,voxel_color_error:0.0,
            triangle_coverage_error:0.02,triangle_color_error:0.02,..voxel};
        for group in &mut cuts.groups {
            group.candidates=vec![refinement,Candidate {spatial_error:1.0,..voxel}];
        }
        cuts.policy.max_coverage_error=1.0;cuts.policy.max_color_error=1.0;
        let fine_selector=Selector::new(&gpu,&cuts).unwrap();
        let choices=decisions(&gpu,&fine_selector,2,ortho,true);
        for (id,scale) in [0.5,2.0].into_iter().enumerate() {
            assert_eq!(choices[id],[0,0,(refinement.triangle_error*scale).to_bits(),1],
                "source-only refinement must never select a voxel");
        }
        cuts.policy.max_coverage_error=0.01;cuts.policy.max_color_error=0.01;
        let rejected_selector=Selector::new(&gpu,&cuts).unwrap();
        let choices=decisions(&gpu,&rejected_selector,2,ortho,true);
        assert!(choices.iter().all(|choice|*choice==[0,0,0,1]),
            "failed triangle optics must retain finest source, never use source-only voxel fields");
    }
}
