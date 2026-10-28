use super::{source_dimensions, MAX_STRENGTH, SOURCE};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[test]
fn spatial_filter_uses_actual_authored_width_and_rejects_legacy() {
    use crate::{foliage_assembly::MeadowBladeBudget, meadow_data::MeadowGeometry};
    let mut source = MeadowGeometry::cook_windswept(73, 0.0, MeadowBladeBudget::Blades120).unwrap();
    let original = source_dimensions(&source).unwrap();
    assert!((original[0] - crate::meadow_windswept::WIDTH).abs() < 1e-7);
    assert!((0.45..=0.9).contains(&original[1]));
    for vertex in &mut source.vertices {
        for axis in 0..3 { vertex.position[axis] = vertex.anchor[axis] + 2.0*(vertex.position[axis]-vertex.anchor[axis]); }
    }
    assert!((source_dimensions(&source).unwrap()[0]-2.0*original[0]).abs() < 1e-7);
    source.vertices[0].anchor[3] = 0.0;
    assert!(source_dimensions(&source).is_none());
    let legacy = MeadowGeometry::cook_with_blade_budget(73, 0.0, MeadowBladeBudget::Blades120).unwrap();
    assert!(source_dimensions(&legacy).is_none());
}

fn compute_source() -> String {
    // The same production pixel function and texture loads. R32Float depth
    // uploads let the bounded fixture supply arbitrary opaque discontinuities;
    // the real depth-texture bindings are separately validated below.
    let source = SOURCE.replace("texture_depth_2d", "texture_2d<f32>")
        .replace("textureLoad(source_depth,pixel,0)", "textureLoad(source_depth,pixel,0).x")
        .replace("textureLoad(opaque_depth,pixel,0)", "textureLoad(opaque_depth,pixel,0).x")
        .replace("textureLoad(source_depth,tap,0)", "textureLoad(source_depth,tap,0).x")
        .replace("textureLoad(opaque_depth,tap,0)", "textureLoad(opaque_depth,tap,0).x");
    source + r#"
@group(0) @binding(5) var<storage,read_write> result:array<vec4<f32>>;
@compute @workgroup_size(8,8) fn verify(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(source_color);
    if any(id.xy>=size) {return;}
    result[id.y*size.x+id.x]=spatial_filter(vec2<i32>(id.xy));
}
"#
}

#[test]
fn spatial_filter_full_shader_and_compute_fixture_validate() {
    for source in [SOURCE.to_owned(), compute_source()] {
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        for (name, stage) in [("vs",naga::ShaderStage::Vertex),("fs",naga::ShaderStage::Fragment)] {
            let mut constants = naga::back::PipelineConstants::default();
            constants.insert("SOURCE_BLADE_WIDTH".into(),0.04);
            constants.insert("SOURCE_BLADE_HEIGHT".into(),0.9);
            naga::back::pipeline_constants::process_overrides(&module,&info,Some((stage,name)),&constants).unwrap();
        }
        let settings = module.types.iter().find(|(_, t)|t.name.as_deref()==Some("SpatialSettings")).unwrap().1;
        assert!(matches!(&settings.inner,naga::TypeInner::Struct {members,span:48} if members[2].offset==32));
        let view = module.types.iter().find(|(_, t)|t.name.as_deref()==Some("SpatialView")).unwrap().1;
        assert!(matches!(&view.inner,naga::TypeInner::Struct {span:320,..}));
    }
    assert_eq!(MAX_STRENGTH,0.65);
}

const SIZE: usize = 17;
const FOCAL: f32 = 512.0;
#[derive(Clone)]
struct Fixture {
    color: Vec<[f32;4]>,
    z: Vec<f32>,
    opaque_z: Vec<f32>,
    growth: f32,
    orthographic: bool,
}
impl Fixture {
    fn new(z:f32) -> Self {
        let color=(0..SIZE*SIZE).map(|i| {
            let f=if ((i/SIZE)+(i%SIZE))%2==0 {0.04}else{0.9};
            [f, f*0.6, f*0.1, 1.0]
        }).collect();
        Self {color,z:vec![z;SIZE*SIZE],opaque_z:vec![z+1.0;SIZE*SIZE],growth:1.0,orthographic:false}
    }
    fn oracle(&self) -> Vec<[f32;4]> {
        fn smooth(a:f64,b:f64,v:f64)->f64 {let t=((v-a)/(b-a)).clamp(0.0,1.0);t*t*(3.0-2.0*t)}
        self.color.iter().enumerate().map(|(i,&center)| {
            if center[3]<=0.0 || self.orthographic || self.z[i]>self.opaque_z[i] {return center;}
            let width=0.5*f64::from(0.04f32*self.growth);
            let footprint=f64::from(self.z[i])/f64::from(FOCAL);
            let strength=0.65*smooth(width,2.0*width,footprint);
            if strength<=0.0 {return center;}
            let limit=f64::from(0.9f32*self.growth).max(4.0*footprint);
            let mut rgb=center[..3].iter().map(|v|4.0*f64::from(*v)).collect::<Vec<_>>();
            let mut coverage=4.0*f64::from(center[3]);
            for dy in -1..=1 {for dx in -1..=1 {
                if dx==0 && dy==0 {continue;}
                let x=(i%SIZE) as i32+dx;let y=(i/SIZE) as i32+dy;
                if x<0 || y<0 || x>=SIZE as i32 || y>=SIZE as i32 {continue;}
                let n=y as usize*SIZE+x as usize;
                let c=self.color[n];
                if c[3]<=0.0 || self.z[n]>self.opaque_z[n] {continue;}
                let kernel=if dx==0 || dy==0 {2.0}else{1.0};
                let weight=kernel*(1.0-smooth(0.5*limit,limit,f64::from(self.z[n]-self.z[i]).abs()));
                for channel in 0..3 {rgb[channel]+=weight*f64::from(c[channel]);}
                coverage+=weight*f64::from(c[3]);
            }}
            let mut result=center;
            for c in 0..3 {result[c]=(f64::from(center[c])*(1.0-strength)+rgb[c]*f64::from(center[3])/coverage*strength) as f32;}
            result
        }).collect()
    }
}

#[test]
fn spatial_filter_oracle_preserves_near_coverage_edges_and_hdr() {
    let near=Fixture::new(5.0);
    assert_eq!(near.oracle(),near.color);
    let far=Fixture::new(100.0);
    let filtered=far.oracle();
    let mean=|pixels:&[[f32;4]]|pixels.iter().map(|p|f64::from(p[0])).sum::<f64>()/pixels.len() as f64;
    let variance=|pixels:&[[f32;4]]|{let m=mean(pixels);pixels.iter().map(|p|(f64::from(p[0])-m).powi(2)).sum::<f64>()/pixels.len() as f64};
    assert!(variance(&filtered)<variance(&far.color)*0.2);
    assert!((mean(&filtered)-mean(&far.color)).abs()<0.002);
    let mut coverage=Fixture::new(100.0);
    for (i,p) in coverage.color.iter_mut().enumerate() {
        let alpha=match i%4 {0=>0.0,1=>0.2,2=>0.6,_=>1.0};
        *p=[2.0*alpha,0.7*alpha,0.1*alpha,alpha];
    }
    for (actual,input) in coverage.oracle().iter().zip(&coverage.color) {
        assert_eq!(actual[3].to_bits(),input[3].to_bits());
        for c in 0..3 {assert!((actual[c]-input[c]).abs()<1e-6);}
    }
    let mut edge=Fixture::new(100.0);
    for y in 0..SIZE {for x in 0..SIZE {
        let i=y*SIZE+x;
        if x<SIZE/2 {edge.color[i]=[0.1,0.2,0.3,1.0];}
        else {edge.color[i]=[0.9,0.8,0.7,1.0];edge.z[i]=105.0;edge.opaque_z[i]=106.0;}
    }}
    assert_eq!(edge.oracle(),edge.color,"depth discontinuity must not mix the two surfaces");
}

#[test]
fn spatial_filter_coarse_triangle_mean_starts_before_the_basal_width_limit() {
    // Midpoint quadrature is exact for the actual coarse linear width W(1-t).
    let mean=(0..128).map(|i|1.0-(f64::from(i)+0.5)/128.0).sum::<f64>()/128.0;
    assert_eq!(mean,0.5);
    let near=Fixture::new(8.0);
    assert_eq!(near.oracle(),near.color);
    let transition=Fixture::new(12.0);
    let changed=transition.oracle();
    // 12/512=.0234375m/pixel is between the new .02/.04 limits, but
    // below the former basal-width onset .04. Keep this distinction covered.
    assert!(12.0/FOCAL>0.5*0.04 && 12.0/FOCAL<0.04);
    assert!(changed[SIZE*SIZE/2][0]>transition.color[SIZE*SIZE/2][0]);
    assert!(changed[SIZE*SIZE/2][0]<0.2,"the transition must stay gradual");
    let mut large=transition.clone();large.growth=1.6;
    assert_eq!(large.oracle(),large.color,"larger live blades remain resolved at this same distance");
}

#[repr(C)]
#[derive(Clone,Copy,Pod,Zeroable)]
struct View {
    vp:[[f32;4];4],inverse:[[f32;4];4],camera:[f32;4],planes:[[f32;4];6],
    params:[f32;4],size:[f32;4],extra:[f32;4],grass:[f32;4],fog:[f32;4],
}

#[test]
#[ignore = "serial GPU fixture; do not run beside a game"]
fn spatial_filter_gpu_preserves_coverage_and_rejects_foreground() {
    let gpu=crate::Gpu::new().unwrap();
    let source=compute_source();
    let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("spatial meadow filter oracle"),source:wgpu::ShaderSource::Wgsl(source.into())});
    let texture=super::super::texture(wgpu::TextureSampleType::Float {filterable:false},wgpu::TextureViewDimension::D2);
    let layout=super::super::layout(&gpu,"filter oracle",&[
        (0,texture),(1,texture),(2,texture),(3,super::super::uniform()),
        (4,super::super::uniform()),(5,super::super::storage(false)),
    ],wgpu::ShaderStages::COMPUTE);
    let pipeline_layout=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label:Some("filter oracle"),bind_group_layouts:&[Some(&layout)],immediate_size:0,
    });
    let pipeline=gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label:Some("spatial meadow filter oracle"),layout:Some(&pipeline_layout),module:&shader,entry_point:Some("verify"),
        compilation_options:Default::default(),cache:gpu.pipeline_cache(),
    });
    let mut cases=vec![Fixture::new(5.0),Fixture::new(100.0),Fixture::new(30.0),Fixture::new(12.0)];
    let mut large=Fixture::new(30.0);large.growth=1.6;cases.push(large);
    let mut ortho=Fixture::new(100.0);ortho.orthographic=true;cases.push(ortho);
    let mut coverage=Fixture::new(100.0);
    for (i,p) in coverage.color.iter_mut().enumerate() {
        let alpha=match i%4 {0=>0.0,1=>0.2,2=>0.6,_=>1.0};
        *p=[2.0*alpha,0.7*alpha,0.1*alpha,alpha];
    }
    cases.push(coverage);
    let mut edges=Fixture::new(100.0);
    for y in 0..SIZE {for x in 0..SIZE {
        let i=y*SIZE+x;
        if x<SIZE/3 {edges.color[i]=[0.0;4];edges.opaque_z[i]=5.0;}
        else if x>SIZE*2/3 {edges.z[i]=105.0;edges.opaque_z[i]=106.0;}
        else if y==SIZE/2 {edges.opaque_z[i]=5.0;} // stale/invalid grass behind foreground
    }}
    cases.push(edges);
    let mut max_error=0.0f32;
    for (case_index,case) in cases.iter().enumerate() {
        let make_texture=|label,format,bytes:&[u8],bpp:u32| {
            let texture=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some(label),
                size:wgpu::Extent3d {width:SIZE as u32,height:SIZE as u32,depth_or_array_layers:1},mip_level_count:1,
                sample_count:1,dimension:wgpu::TextureDimension::D2,format,
                usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
            gpu.queue.write_texture(texture.as_image_copy(),bytes,wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(SIZE as u32*bpp),rows_per_image:Some(SIZE as u32)},texture.size());
            texture.create_view(&Default::default())
        };
        let color=make_texture("filter source",wgpu::TextureFormat::Rgba32Float,bytemuck::cast_slice(&case.color),16);
        let depths:Vec<f32>=case.z.iter().map(|z|0.1/z).collect();
        let opaque_depths:Vec<f32>=case.opaque_z.iter().map(|z|0.1/z).collect();
        let depth=make_texture("filter source depth",wgpu::TextureFormat::R32Float,bytemuck::cast_slice(&depths),4);
        let opaque=make_texture("filter opaque depth",wgpu::TextureFormat::R32Float,bytemuck::cast_slice(&opaque_depths),4);
        let vp=glam::Mat4::perspective_infinite_reverse_rh(2.0*(SIZE as f32/(2.0*FOCAL)).atan(),1.0,0.1);
        let mut view=View::zeroed();view.vp=vp.to_cols_array_2d();view.inverse=vp.inverse().to_cols_array_2d();
        view.camera[3]=FOCAL;view.params[2]=u32::from(case.orthographic) as f32;
        let view=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("filter view"),contents:bytemuck::bytes_of(&view),usage:wgpu::BufferUsages::UNIFORM});
        let mut settings=super::super::Settings::zeroed();settings.shape[0]=case.growth;
        let settings=gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("filter settings"),contents:bytemuck::bytes_of(&settings),usage:wgpu::BufferUsages::UNIFORM});
        let bytes=(SIZE*SIZE*16) as u64;
        let output=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("filter output"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
        let read=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("filter readback"),size:bytes,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let bind=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("filter fixture"),layout:&pipeline.get_bind_group_layout(0),entries:&[
            wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::TextureView(&color)},
            wgpu::BindGroupEntry {binding:1,resource:wgpu::BindingResource::TextureView(&depth)},
            wgpu::BindGroupEntry {binding:2,resource:wgpu::BindingResource::TextureView(&opaque)},
            wgpu::BindGroupEntry {binding:3,resource:view.as_entire_binding()},
            wgpu::BindGroupEntry {binding:4,resource:settings.as_entire_binding()},
            wgpu::BindGroupEntry {binding:5,resource:output.as_entire_binding()},
        ]});
        let mut encoder=gpu.device.create_command_encoder(&Default::default());
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&pipeline);pass.set_bind_group(0,&bind,&[]);pass.dispatch_workgroups((SIZE as u32).div_ceil(8),(SIZE as u32).div_ceil(8),1);}
        encoder.copy_buffer_to_buffer(&output,0,&read,0,bytes);gpu.queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
        let data=read.slice(..).get_mapped_range().unwrap();let actual:&[[f32;4]]=bytemuck::cast_slice(&data);
        for (pixel,((actual,expected),original)) in actual.iter().zip(case.oracle()).zip(&case.color).enumerate() {
            assert_eq!(actual[3].to_bits(),original[3].to_bits(),"case {case_index}, pixel {pixel}: coverage changed");
            for c in 0..3 {let error=(actual[c]-expected[c]).abs();max_error=max_error.max(error);assert!(error<2e-5,"case {case_index}, pixel {pixel}, channel {c}: {actual:?} != {expected:?}");}
            if original[3]==0.0 {assert_eq!(actual,original,"empty foreground/silhouette changed");}
        }
        drop(data);read.unmap();
    }
    eprintln!("meadow spatial filter: {} pixels, {} native/source/depth fixtures; alpha exact; max f64-oracle RGB error {max_error}",cases.len()*SIZE*SIZE,cases.len());
}
