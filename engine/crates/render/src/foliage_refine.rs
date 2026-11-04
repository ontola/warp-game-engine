//! GPU complete-cut refinement. All allocation order is GPU stable-prefix work;
//! the CPU submits a cooked depth bound, never a current population shortlist.
use crate::{Gpu,foliage_select::Selector};

pub(crate) struct Refiner {
    proposals:[wgpu::Buffer;2],queues:[wgpu::Buffer;2],blocks:wgpu::Buffer,
    pub state:wgpu::Buffer,args:wgpu::Buffer,layout:wgpu::BindGroupLayout,
    pipelines:Vec<wgpu::ComputePipeline>,
    binds:std::sync::Mutex<crate::frame::resource_cache::BindGroups<2>>,
}
impl Refiner {
    pub fn new(gpu:&Gpu,capacity:u32)->Self {
        let storage=|label,size|gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label:Some(label),size,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST,
            mapped_at_creation:false});
        let entries=(0..15).map(|binding|wgpu::BindGroupLayoutEntry {binding,visibility:wgpu::ShaderStages::COMPUTE,count:None,
            ty:wgpu::BindingType::Buffer {ty:if binding==0 {wgpu::BufferBindingType::Uniform} else {wgpu::BufferBindingType::Storage {read_only:binding==1||binding==2}},has_dynamic_offset:false,min_binding_size:None}}).collect::<Vec<_>>();
        let layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some("foliage complete-cut refinement"),entries:&entries});
        let pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:None,bind_group_layouts:&[Some(&layout)],immediate_size:0});
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("foliage incremental admission"),source:wgpu::ShaderSource::Wgsl(include_str!("shaders/foliage_refine.wgsl").into())});
        let pipelines=["seed","start","prepare","propose","priority_scan","priority_totals","decide","output_scan","output_totals","apply","advance","drain","audit"].into_iter().map(|entry|
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(entry),layout:Some(&pl),module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()})).collect();
        Self {proposals:[storage("foliage refinement proposals A",capacity as u64*80),storage("foliage refinement proposals B",capacity as u64*80)],
            queues:[storage("foliage complete cut A",capacity as u64*16),storage("foliage complete cut B",capacity as u64*16)],
            blocks:storage("foliage refinement prefixes",(capacity.div_ceil(256)as u64+1)*32*16),
            state:storage("foliage refinement allocation",128),
            args:gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("foliage refinement indirect"),size:24,usage:wgpu::BufferUsages::INDIRECT|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false}),
            pipelines,layout,binds:Default::default()}
    }
    fn bind(&self,gpu:&Gpu,s:&Selector,front:usize)->wgpu::BindGroup {
        let buffers=[&s.uniform,&s._nodes,&s._instances,&s.asset_pages,&self.proposals[front],&self.queues[front],&self.queues[1-front],&self.blocks,&s.records,&s.counters,&s.fallback,&s.direct,&s.demand,&self.state,&self.proposals[1-front]];
        let entries=buffers.iter().enumerate().map(|(binding,b)|wgpu::BindGroupEntry {binding:binding as u32,resource:b.as_entire_binding()}).collect::<Vec<_>>();
        self.binds.lock().unwrap().entries(gpu,&self.layout,"foliage refinement buffers",&entries)
    }
    fn dispatch(&self,enc:&mut wgpu::CommandEncoder,bind:&wgpu::BindGroup,pipeline:usize,groups:Option<u32>,offset:u64) {
        let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,bind,&[]);pass.set_pipeline(&self.pipelines[pipeline]);
        if let Some(n)=groups {pass.dispatch_workgroups(n,1,1);}else{pass.dispatch_workgroups_indirect(&self.args,offset);}
    }
    fn prepare(&self,enc:&mut wgpu::CommandEncoder,bind:&wgpu::BindGroup) {
        self.dispatch(enc,bind,2,Some(1),0);
        enc.copy_buffer_to_buffer(&self.state,32,&self.args,0,12);
        enc.copy_buffer_to_buffer(&self.state,48,&self.args,12,12);
    }
    pub fn audit(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,s:&Selector) {
        enc.clear_buffer(&self.state,19*4,Some(9*4));
        self.dispatch(enc,&self.bind(gpu,s,0),12,Some(s.count.max(s.dimensions().0).div_ceil(64)),0);
    }
    pub fn encode(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,s:&Selector) {
        let binds=[self.bind(gpu,s,0),self.bind(gpu,s,1)];enc.clear_buffer(&self.state,0,None);
        self.dispatch(enc,&binds[0],0,Some(s.count.div_ceil(64)),0);
        self.dispatch(enc,&binds[0],1,Some(1),0);
        // One extra transition admits the initial emergency proxy into the
        // recipe root. A final drain makes depth bounds conservative.
        let rounds=2*(s.levels+1);
        for round in 0..rounds {
            let bind=&binds[round as usize%2];self.prepare(enc,bind);
            // Compute dispatches are ordered storage-usage scopes. Keep the
            // dependent dispatches in one pass; only indirect argument copies
            // require leaving it. Avoid eight pass/descriptor setups per wave.
            let mut pass=enc.begin_compute_pass(&Default::default());
            pass.set_bind_group(0,bind,&[]);
            for (pipeline,groups,offset) in [(3,None,0),(4,None,12),(5,Some(1),0),(6,None,0),(7,None,12),(8,Some(1),0),(9,None,0),(10,Some(1),0)] {
                pass.set_pipeline(&self.pipelines[pipeline]);
                if let Some(n)=groups {pass.dispatch_workgroups(n,1,1);}else{pass.dispatch_workgroups_indirect(&self.args,offset);}
            }
        }
        let bind=&binds[rounds as usize%2];self.prepare(enc,bind);self.dispatch(enc,bind,11,None,0);
    }
}
#[cfg(test)] mod tests {
    #[test] fn refinement_shader_validates() {
        let src=include_str!("shaders/foliage_refine.wgsl");
        let module=naga::front::wgsl::parse_str(src).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));
    }
}
