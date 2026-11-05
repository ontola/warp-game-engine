//! GPU complete-cut refinement. All allocation order is GPU stable-prefix work;
//! the CPU submits a cooked depth bound, never a current population shortlist.
use crate::{Gpu,foliage_select::Selector};

pub(crate) struct Refiner {
    capacity:u32,proposals:wgpu::Buffer,parked:wgpu::Buffer,queues:[wgpu::Buffer;2],blocks:wgpu::Buffer,
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
        let pipelines=["seed","start","prepare","propose","priority_scan","priority_totals","decide","output_scan","output_totals","apply","advance","drain","audit","wake_count","wake_prefix","wake_scatter"].into_iter().map(|entry|
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(entry),layout:Some(&pl),module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()})).collect();
        Self {capacity,proposals:storage("foliage active proposals",capacity as u64*80),parked:storage("foliage dormant proposals",capacity as u64*96),
            queues:[storage("foliage complete cut A",capacity as u64*16),storage("foliage complete cut B",capacity as u64*16)],
            blocks:storage("foliage refinement prefixes",(capacity.div_ceil(256)as u64+1)*32*16),
            state:storage("foliage refinement allocation",320),
            args:gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("foliage refinement indirect"),size:36,usage:wgpu::BufferUsages::INDIRECT|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false}),
            pipelines,layout,binds:Default::default()}
    }
    fn bind(&self,gpu:&Gpu,s:&Selector,front:usize)->wgpu::BindGroup {
        let buffers=[&s.uniform,&s._nodes,&s._instances,&s.asset_pages,&self.proposals,&self.queues[front],&self.queues[1-front],&self.blocks,&s.records,&s.counters,&s.fallback,&s.direct,&s.demand,&self.state,&self.parked];
        let entries=buffers.iter().enumerate().map(|(binding,b)|wgpu::BindGroupEntry {binding:binding as u32,resource:b.as_entire_binding()}).collect::<Vec<_>>();
        self.binds.lock().unwrap().entries(gpu,&self.layout,"foliage refinement buffers",&entries)
    }
    fn dispatch(&self,enc:&mut wgpu::CommandEncoder,bind:&wgpu::BindGroup,pipeline:usize,groups:Option<u32>,offset:u64) {
        let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,bind,&[]);pass.set_pipeline(&self.pipelines[pipeline]);
        if let Some(n)=groups {pass.dispatch_workgroups(n,1,1);}else{pass.dispatch_workgroups_indirect(&self.args,offset);}
    }
    fn prepare(&self,enc:&mut wgpu::CommandEncoder,bind:&wgpu::BindGroup) {
        self.dispatch(enc,bind,2,Some(1),0);
        enc.copy_buffer_to_buffer(&self.state,64*4,&self.args,24,12);
        {
            let mut pass=enc.begin_compute_pass(&Default::default());pass.set_bind_group(0,bind,&[]);
            for (pipeline,direct) in [(13,false),(14,true),(15,false)] {
                pass.set_pipeline(&self.pipelines[pipeline]);
                if direct {pass.dispatch_workgroups(1,1,1);}else {pass.dispatch_workgroups_indirect(&self.args,24);}
            }
        }
        enc.copy_buffer_to_buffer(&self.state,32,&self.args,0,12);
        enc.copy_buffer_to_buffer(&self.state,48,&self.args,12,12);
    }
    pub fn audit(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,s:&Selector) {
        enc.clear_buffer(&self.state,19*4,Some(9*4));
        enc.clear_buffer(&self.state,32*4,Some(23*4));
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
        let bind=&binds[rounds as usize%2];self.dispatch(enc,bind,11,Some(self.capacity.div_ceil(64)),0);
    }
}
#[cfg(test)] mod tests {
    #[test]
    #[ignore="exclusive GPU dormant role scheduling fixture"]
    fn dormant_shadows_wake_after_free_camera_batch() {
        use crate::foliage_select::{Node,Instance,NONE};
        let gpu=crate::Gpu::new().unwrap();
        let node=Node {sphere:[0.,0.,0.5,0.1],error:[0.;4],links:[NONE,1,0,2]};
        let tree=Instance {rows:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.]],scale:[1.,0.,0.,0.],info:[0,1,0,0]};
        let selector=crate::foliage_select::Selector::new(&gpu,&[node],&[tree,tree],2,2).unwrap();
        let r=&selector.refiner;let bind=r.bind(&gpu,&selector,0);
        let mut parked=[0u32;48];parked[..4].copy_from_slice(&[0,0,256,1]);parked[24..28].copy_from_slice(&[1,0,512,1]);
        gpu.queue.write_buffer(&r.parked,0,bytemuck::cast_slice(&parked));
        gpu.queue.write_buffer(&r.state,29*4,bytemuck::cast_slice(&[2u32]));
        fn read(gpu:&crate::Gpu,b:&wgpu::Buffer)->Vec<u32> {
            let out=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:b.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(b,0,&out,0,b.size());let submission=gpu.queue.submit([enc.finish()]);
            let(tx,rx)=std::sync::mpsc::sync_channel(1);out.slice(..).map_async(wgpu::MapMode::Read,move|v|{let _=tx.send(v);});
            gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(30))}).unwrap();rx.recv().unwrap().unwrap();
            let bytes=out.slice(..).get_mapped_range().unwrap();bytes.chunks_exact(4).map(bytemuck::pod_read_unaligned).collect()
        }
        for (root,role,pending) in [(0,256,1),(1,512,0)] {
            gpu.queue.write_buffer(&r.state,3*4,bytemuck::cast_slice(&[0u32]));
            let mut enc=gpu.device.create_command_encoder(&Default::default());r.prepare(&mut enc,&bind);gpu.queue.submit([enc.finish()]);
            let state=read(&gpu,&r.state);assert_eq!(state[3],1);assert_eq!(state[30],pending);
            let queue=read(&gpu,&r.queues[0]);assert_eq!(&queue[..4],&[root,0,role,root+2]);
        }
        gpu.queue.write_buffer(&r.state,3*4,bytemuck::cast_slice(&[0u32]));
        let mut enc=gpu.device.create_command_encoder(&Default::default());r.prepare(&mut enc,&bind);gpu.queue.submit([enc.finish()]);
        let state=read(&gpu,&r.state);assert_eq!(state[3],0);assert_eq!(state[60],2,"no repeated wake without new work or capacity changes");
    }
    #[test] fn refinement_shader_validates() {
        let src=include_str!("shaders/foliage_refine.wgsl");
        let module=naga::front::wgsl::parse_str(src).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));
    }
}
