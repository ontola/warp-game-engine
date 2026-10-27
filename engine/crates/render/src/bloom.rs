//! Small display-space highlight bloom. Physical scene HDR and reconstruction
//! history remain untouched. The two quarter-size targets belong to one view.
use crate::Gpu;

pub const DEFAULT_STRENGTH:f32=0.04;
pub const THRESHOLD:f32=2.0;
pub const KNEE:f32=1.0;
pub const MAX_EXPOSED:f32=64.0;

pub fn dimensions(width:u32,height:u32)->[u32;2] {[width.div_ceil(4).max(1),height.div_ceil(4).max(1)]}
pub fn active(enabled:bool,strength:f32)->bool {enabled && strength>0.0}

/// Bind groups for one input texture (TAA alternates two histories).
struct Cached {input:wgpu::Texture,binds:[wgpu::BindGroup;3]}
struct Targets {size:[u32;2],_a:wgpu::Texture,_b:wgpu::Texture,a_view:wgpu::TextureView,b_view:wgpu::TextureView,uniform:wgpu::Buffer,cached:Vec<Cached>}
pub(crate) struct Bloom {
    layout:wgpu::BindGroupLayout,
    pipelines:[wgpu::ComputePipeline;3],
    pub sampler:wgpu::Sampler,
    fallback:wgpu::Texture,
    views:std::sync::Mutex<Vec<Targets>>,
}
fn texture(gpu:&Gpu,size:[u32;2])->wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("quarter-resolution bloom"),
        size:wgpu::Extent3d{width:size[0],height:size[1],depth_or_array_layers:1},
        mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,
        format:wgpu::TextureFormat::Rgba16Float,
        usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::STORAGE_BINDING,view_formats:&[]})
}
impl Bloom {
    pub fn new(gpu:&Gpu)->Self {
        let types=[
            wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},
            wgpu::BindingType::StorageTexture {access:wgpu::StorageTextureAccess::WriteOnly,format:wgpu::TextureFormat::Rgba16Float,view_dimension:wgpu::TextureViewDimension::D2},
            wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:wgpu::BufferSize::new(16)},
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        ];
        let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("bloom"),
            entries:&types.into_iter().enumerate().map(|(binding,ty)|wgpu::BindGroupLayoutEntry {binding:binding as u32,
                visibility:wgpu::ShaderStages::COMPUTE,ty,count:None}).collect::<Vec<_>>()});
        let pipeline_layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("bloom"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("bloom"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/bloom.wgsl").into())});
        let pipelines=["extract","horizontal","vertical"].map(|entry|gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label:Some(entry),layout:Some(&pipeline_layout),module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()}));
        let sampler=gpu.device.create_sampler(&wgpu::SamplerDescriptor {label:Some("view-local bloom filtering"),
            mag_filter:wgpu::FilterMode::Linear,min_filter:wgpu::FilterMode::Linear,
            address_mode_u:wgpu::AddressMode::ClampToEdge,address_mode_v:wgpu::AddressMode::ClampToEdge,..Default::default()});
        Self {layout,pipelines,sampler,fallback:texture(gpu,[1,1]),views:std::sync::Mutex::new(Vec::new())}
    }

    pub fn disabled_view(&self)->wgpu::TextureView {self.fallback.create_view(&Default::default())}
    pub fn set_view_count(&self,count:usize) {self.views.lock().unwrap().truncate(count);}

    pub fn render(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,view:usize,input:&wgpu::Texture,
        exposure:f32,timestamps:Option<(&wgpu::QuerySet,u32)>)->wgpu::TextureView {
        let size=dimensions(input.width(),input.height());
        let mut views=self.views.lock().unwrap();
        let make=|size:[u32;2]| {
            let (a,b)=(texture(gpu,size),texture(gpu,size));
            let (a_view,b_view)=(a.create_view(&Default::default()),b.create_view(&Default::default()));
            // One exposure buffer per view: split views never share a value.
            let uniform=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("bloom exposure"),size:16,
                usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            Targets{size,_a:a,_b:b,a_view,b_view,uniform,cached:Vec::new()}
        };
        while views.len()<=view {views.push(make([1,1]));}
        let target=&mut views[view];
        if target.size!=size {*target=make(size);}
        let exposure=if exposure.is_finite(){exposure.clamp(0.0,65504.0)}else{0.0};
        gpu.queue.write_buffer(&target.uniform,0,bytemuck::cast_slice(&[exposure,THRESHOLD,KNEE,MAX_EXPOSED]));
        if !target.cached.iter().any(|c|c.input==*input) {
            let source=input.create_view(&Default::default());
            let binds=[(&source,&target.a_view),(&target.a_view,&target.b_view),(&target.b_view,&target.a_view)].map(|(src,dst)|
                gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("bloom view"),layout:&self.layout,entries:&[
                    wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::TextureView(src)},
                    wgpu::BindGroupEntry {binding:1,resource:wgpu::BindingResource::TextureView(dst)},
                    wgpu::BindGroupEntry {binding:2,resource:target.uniform.as_entire_binding()},
                    wgpu::BindGroupEntry {binding:3,resource:wgpu::BindingResource::Sampler(&self.sampler)},
                ]}));
            if target.cached.len()>=2 {target.cached.remove(0);}
            target.cached.push(Cached{input:input.clone(),binds});
        }
        let cached=target.cached.iter().find(|c|c.input==*input).unwrap();
        for i in 0..3 {
            let bind=&cached.binds[i];
            let writes=if i==1 {None}else{timestamps.map(|(query_set,start)|wgpu::ComputePassTimestampWrites {query_set,
                beginning_of_pass_write_index:(i==0).then_some(start),end_of_pass_write_index:(i==2).then_some(start+1)})};
            let mut pass=enc.begin_compute_pass(&wgpu::ComputePassDescriptor {label:Some(["bloom highlight extraction","bloom horizontal filter","bloom vertical filter"][i]),timestamp_writes:writes});
            pass.set_pipeline(&self.pipelines[i]);pass.set_bind_group(0,bind,&[]);pass.dispatch_workgroups(size[0].div_ceil(8),size[1].div_ceil(8),1);
        }
        target.a_view.clone()
    }
}

#[cfg(test)]mod tests {
    use super::*;
    #[test]fn bounded_view_dimensions_and_disabled_mode() {
        for (input,expected) in [([1,1],[1,1]),([639,359],[160,90]),([1920,1080],[480,270]),([3840,2160],[960,540])] {
            assert_eq!(dimensions(input[0],input[1]),expected);
            assert!(expected[0]*expected[1]*16<=input[0]*input[1]+16*(input[0]+input[1]+1));
        }
        assert!(!active(false,DEFAULT_STRENGTH));assert!(!active(true,0.0));assert!(active(true,DEFAULT_STRENGTH));
        // Four independently allocated views have no sampling coordinates in
        // another view, including odd split sizes and after a layout change.
        assert_eq!(dimensions(639,359),[160,90]);
    }
    #[test]fn exposure_soft_knee_preserves_dark_ground_and_tiny_highlights() {
        let extract=|radiance:f64,exposure:f64| {let b=(radiance.max(0.0)*exposure).min(MAX_EXPOSED as f64);
            let soft=(b-THRESHOLD as f64+KNEE as f64).clamp(0.0,2.0*KNEE as f64);
            (b-THRESHOLD as f64).max(soft*soft/(4.0*KNEE as f64)).max(0.0)};
        for b in [0.0,0.18,0.5,1.0] {assert_eq!(extract(b,1.0),0.0);}
        assert_eq!(extract(8.0,0.25),extract(2.0,1.0));
        assert!(extract(8.0,1.0)/16.0>0.0,"threshold before downsample retains a small bright emitter");
        assert_eq!(extract(8.0/16.0,1.0),0.0,"threshold after downsample loses that emitter");
        let mut previous=0.0;for i in 0..10000 {let value=extract(i as f64/100.0,1.0);assert!(value>=previous&&value.is_finite());previous=value;}
        assert_eq!(extract(f64::INFINITY,1.0),62.0);
    }
    #[test]fn bloom_shader_validates_and_all_filters_preserve_constant_energy() {
        let source=include_str!("shaders/bloom.wgsl");let module=naga::front::wgsl::parse_str(source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap();
        assert_eq!(module.entry_points.len(),3);
        assert!((0.227027027_f64+2.0*0.316216216+2.0*0.070270270-1.0).abs()<1e-8);
        assert!(source.contains("if all(p<size)"));assert!(source.contains("max(count,1.0)"));
    }
}
