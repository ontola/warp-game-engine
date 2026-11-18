//! GPU complete-cut refinement. All allocation order is GPU stable-prefix work;
//! the CPU submits a cooked depth bound, never a current population shortlist.
use crate::{Gpu,foliage_select::Selector};

/// The selection counters (16 words) and the refine allocation state share
/// one buffer: counters at 0, state from STATE_OFFSET (256-byte aligned).
pub(crate) const STATE_OFFSET:u64=256;
pub(crate) struct Refiner {
    capacity:u32,proposals:wgpu::Buffer,parked:wgpu::Buffer,
    /// Both work queues in one binding (two halves of `capacity` tasks) and,
    /// after them, the prefix blocks; `halves[front]` tells the shader which half
    /// is this round's (x), the next round's (y) and where the blocks start (w).
    queue:wgpu::Buffer,halves:[wgpu::Buffer;2],
    pub state:wgpu::Buffer,args:wgpu::Buffer,layout:wgpu::BindGroupLayout,
    /// Group 1, proposals and parked: only the budgeted admission reads them, so
    /// the error cut's pipelines stay within 8 storage buffers.
    budget_layout:wgpu::BindGroupLayout,budget_bind:wgpu::BindGroup,
    pipelines:Vec<wgpu::ComputePipeline>,budget:Vec<bool>,
    binds:std::sync::Mutex<crate::frame::resource_cache::BindGroups<2>>,
    /// Last frame's working-round count (state[68]), kept for async readback.
    worked:wgpu::Buffer,rounds:std::sync::Mutex<Rounds>,
    /// Error-driven cut (default) instead of budgeted collective admission.
    /// `KERN_FOLIAGE_CUT=budget` selects the old admission for A/B.
    fast:std::sync::atomic::AtomicBool,
}
/// How many refinement rounds to encode. Every round costs a fixed set of
/// dispatches and barriers even when its work list is empty, and the hierarchy
/// depth bound (2*(levels+1)) is far more than a typical frame needs. Rounds
/// that do work form a prefix, so the GPU reports how many it used and later
/// frames encode that plus a margin. A frame that used every encoded round may
/// have been cut short: its drain publishes the current complete cut (coarser,
/// never missing), and the next frames go back to the full bound.
/// Default floor of the camera error-cut scale, so a lone tree turns to
/// voxels sooner than its own tiny budget would. 16 (a full forest's scale at
/// 1600x900) picked coarse per-branch levels that showed as smooth balls on
/// grown pines at 10-30 m. KERN_CAMERA_CUT_FLOOR overrides it (1 disables it).
pub const CAMERA_CUT_FLOOR: f32 = 4.0;
/// How much less error a tree the camera drew in detail last frame keeps
/// that detail down to (foliage_refine.wgsl fast_round), so trees near the
/// cut do not flip each frame. KERN_CUT_HYSTERESIS overrides it (1 disables).
pub const CUT_HYSTERESIS: f32 = 1.25;

pub(crate) fn refine_source() -> String {
    let floor = crate::switches::CAMERA_CUT_FLOOR.parse::<f32>().filter(|v| v.is_finite() && *v >= 1.0).unwrap_or(CAMERA_CUT_FLOOR);
    let hysteresis = crate::switches::CUT_HYSTERESIS.parse::<f32>().filter(|v| v.is_finite() && *v >= 1.0).unwrap_or(CUT_HYSTERESIS);
    let per_tree = !crate::switches::SOURCE_PER_TREE.off();
    let wave = !crate::switches::REFINE_WAVE_PUBLISH.off();
    format!("const CAMERA_CUT_FLOOR: f32 = {floor:?};\nconst CUT_HYSTERESIS: f32 = {hysteresis:?};\nconst SOURCE_PER_TREE: bool = {per_tree};\nconst WAVE_PUBLISH: bool = {wave};\n{}", include_str!("shaders/foliage_refine.wgsl"))
}
struct Rounds {cameras:Vec<(glam::Vec3,glam::Vec3,f32)>,stale:u64,full:bool,used:Option<u32>,hint:Option<u32>,encoded:u32,frame:u64,newest:u64,slots:Vec<RoundSlot>}
struct RoundSlot {buffer:wgpu::Buffer,encoded:u32,frame:u64,pending:Option<std::sync::mpsc::Receiver<Result<(),wgpu::BufferAsyncError>>>}
impl Rounds {
    /// Margin over the observed need, so a camera that moves a little does not
    /// cut refinement short. Shrinks one round per observation.
    const MARGIN:u32=4;
    fn next(&mut self,bound:u32,used:u32,encoded:u32) {
        let want=if used>=encoded {bound} else {(used+Self::MARGIN).min(bound)};
        let shrink=self.hint.unwrap_or(bound).saturating_sub(2);
        self.hint=Some(want.max(shrink).min(bound));
    }
    /// More than half a metre, about 3 degrees of turn, or 5% zoom in a frame.
    fn cut(a:(glam::Vec3,glam::Vec3,f32),b:(glam::Vec3,glam::Vec3,f32))->bool {
        a.0.distance(b.0)>0.5 || a.1.dot(b.1)<0.9986 || (a.2-b.2).abs()>0.05*a.2.max(b.2)
    }
    /// Even, so the final queue is the same buffer as with the full bound.
    fn count(&self,bound:u32)->u32 {if self.full {return bound;}self.hint.map_or(bound,|h|(h+(h&1)).clamp(2,bound))}
}
/// Group 0 (shared) and group 1 (budgeted admission only).
struct Binds {shared:wgpu::BindGroup,budget:wgpu::BindGroup}
impl Refiner {
    /// `selected`: word offsets of the record consumers and direct roots in
    /// the selector's shared record buffer (foliage_select.rs Region::selected),
    /// the first vec4 of the trees in the catalog buffer, and the node count.
    pub fn new(gpu:&Gpu,capacity:u32,selected:[u32;4])->Self {
        let storage=|label,size|gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label:Some(label),size,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST,
            mapped_at_creation:false});
        let layout_of=|label,bindings:&[u32]|{
            let entries=bindings.iter().map(|&binding|wgpu::BindGroupLayoutEntry {binding,visibility:wgpu::ShaderStages::COMPUTE,count:None,
                ty:wgpu::BindingType::Buffer {ty:if binding==0 || binding==6 {wgpu::BufferBindingType::Uniform} else {wgpu::BufferBindingType::Storage {read_only:binding==1||binding==16}},has_dynamic_offset:false,min_binding_size:None}}).collect::<Vec<_>>();
            gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {label:Some(label),entries:&entries})};
        let layout=layout_of("foliage complete-cut refinement",&Self::BINDINGS);let budget_layout=layout_of("foliage budgeted admission",&[4,14]);
        let fast_pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("layout refine"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
        let budget_pl=gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {label:Some("layout refine budget"),bind_group_layouts:&[Some(&layout),Some(&budget_layout)],immediate_size:0});
        let module=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("foliage incremental admission"),source:wgpu::ShaderSource::Wgsl(refine_source().into())});
        let budget:Vec<bool>=Self::ENTRIES.iter().map(|entry|Self::BUDGET.contains(entry)).collect();
        let pipelines=Self::ENTRIES.iter().zip(&budget).map(|(&entry,&budget)|
            gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {label:Some(entry),layout:Some(if budget {&budget_pl} else {&fast_pl}),module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:gpu.pipeline_cache()})).collect();
        let (proposals,parked)=(storage("foliage active proposals",capacity as u64*96),storage("foliage dormant proposals",capacity as u64*112));
        let budget_bind=Self::budget_bind(gpu,&budget_layout,&proposals,&parked);
        // Two task halves, then 32 totals and 32 buckets per 256-task block (vec4s).
        let blocks=2*capacity;
        Self {capacity,proposals,parked,budget_layout,budget_bind,budget,
            queue:storage("foliage complete cut queues and prefixes",(blocks as u64+(capacity.div_ceil(256)as u64+1)*32)*16),
            halves:[[0u32,capacity,capacity,blocks,selected[0],selected[1],selected[2],selected[3]],[capacity,0,capacity,blocks,selected[0],selected[1],selected[2],selected[3]]].map(|h|{use wgpu::util::DeviceExt;gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {label:Some("foliage queue halves"),contents:bytemuck::cast_slice(&h),usage:wgpu::BufferUsages::UNIFORM})}),
            state:storage("foliage selection counters and refinement allocation",STATE_OFFSET+384),
            args:gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("foliage refinement indirect"),size:36,usage:wgpu::BufferUsages::INDIRECT|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false}),
            pipelines,layout,binds:Default::default(),fast:std::sync::atomic::AtomicBool::new(Self::default_fast()),
            worked:gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("foliage refinement rounds used"),size:4,usage:wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false}),
            rounds:std::sync::Mutex::new(Rounds {cameras:Vec::new(),stale:0,full:false,used:None,hint:None,encoded:0,frame:0,newest:0,slots:(0..2).map(|_|RoundSlot {buffer:gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("foliage refinement rounds readback"),size:4,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false}),encoded:0,frame:0,pending:None}).collect()})}
    }
    /// Tests keep the budgeted admission unless they ask for the error cut.
    fn default_fast()->bool {!cfg!(test) && !crate::switches::FOLIAGE_CUT.is("budget")}
    pub(crate) fn set_error_cut(&self,on:bool) {self.fast.store(on,std::sync::atomic::Ordering::Relaxed);}
    pub(crate) fn error_cut(&self)->bool {self.fast.load(std::sync::atomic::Ordering::Relaxed)}
    /// Reads finished round counts without waiting, starts a readback of the
    /// last encoded frame, and returns how many rounds to encode now.
    fn rounds(&self,gpu:&Gpu,bound:u32)->u32 {
        let mut r=self.rounds.lock().unwrap();let r=&mut *r;
        let _=gpu.device.poll(wgpu::PollType::Poll);
        let mut seen=Vec::new();
        for slot in &mut r.slots {
            let Some(Ok(result))=slot.pending.as_ref().map(|rx|rx.try_recv()) else {continue};
            slot.pending=None;
            if result.is_ok() {
                let used=bytemuck::pod_read_unaligned::<u32>(&slot.buffer.slice(..).get_mapped_range().unwrap());
                seen.push((slot.frame,used,slot.encoded));
            }
            slot.buffer.unmap();
        }
        seen.sort_unstable();
        // Readbacks of frames encoded before a camera cut describe the old view.
        for (frame,used,encoded) in seen {if frame>r.newest && frame>=r.stale {r.newest=frame;r.used=Some(used);r.next(bound,used,encoded);}}
        // `worked` holds the previous encode's count; queue order makes this
        // copy see it once that frame's commands ran.
        if r.encoded!=0 {
            let (encoded,frame)=(r.encoded,r.frame);
            if let Some(slot)=r.slots.iter_mut().find(|s|s.pending.is_none()) {
                let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(&self.worked,0,&slot.buffer,0,4);gpu.queue.submit([enc.finish()]);
                let (tx,rx)=std::sync::mpsc::sync_channel(1);slot.buffer.slice(..).map_async(wgpu::MapMode::Read,move|v|{let _=tx.send(v);});
                slot.pending=Some(rx);slot.encoded=encoded;slot.frame=frame;
            }
        }
        // KERN_REFINE_ROUNDS=N (diagnostic): encode exactly N rounds (even,
        // at most the bound), to see what a round costs.
        let fixed=crate::switches::REFINE_ROUNDS.parse::<u32>().map(|n|(n+(n&1)).clamp(2,bound));
        r.frame+=1;r.encoded=fixed.unwrap_or_else(||r.count(bound));r.encoded
    }
    /// A camera that jumps, turns fast or zooms needs refinement the last
    /// readback did not see: go back to the full bound, and ignore readbacks
    /// of frames encoded before the cut.
    pub(crate) fn note_cameras(&self,cameras:&[(glam::Vec3,glam::Vec3,f32)]) {
        let mut r=self.rounds.lock().unwrap();
        let cut=r.cameras.len()!=cameras.len() || r.cameras.iter().zip(cameras).any(|(a,b)|Rounds::cut(*a,*b));
        if cut && !r.cameras.is_empty() {r.hint=None;r.stale=r.frame+1;}
        r.cameras=cameras.to_vec();
    }
    /// Rounds the most recent encode used, for diagnostics and tests.
    /// (rounds the most recent encode used, latest working rounds the GPU reported).
    #[cfg(test)] pub(crate) fn encoded_rounds(&self)->(u32,Option<u32>) {let r=self.rounds.lock().unwrap();(r.encoded,r.used)}
    /// Always encode the depth bound, as before the readback (test reference).
    #[cfg(test)] pub(crate) fn test_full_rounds(&self) {self.rounds.lock().unwrap().full=true;}
    /// Pretend the GPU last needed this few rounds.
    #[cfg(test)] pub(crate) fn test_hint(&self,rounds:u32) {self.rounds.lock().unwrap().hint=Some(rounds);}
    /// Bindings in use: 2 (trees), 11 (direct roots), 13 (state) and 15 (record
    /// consumers) now live inside 1 (catalog), 8 (selected) and 9 (counters);
    /// 7 (prefix blocks) inside 5 (queue). Group 1 holds 4 and 14.
    const BINDINGS:[u32;10]=[0,1,3,5,6,8,9,10,12,16];
    /// Pipeline order: encode() names pipelines by index.
    const ENTRIES:[&str;24]=["seed","start","prepare","propose","priority_scan","priority_totals","decide","output_scan","output_totals","apply","advance","drain","audit","wake_count","wake_prefix","wake_scatter","seed_scatter","fast_begin","fast_round","fast_advance","fast_adapt","fast_mode","fast_round_late","fast_drain"];
    /// Entry points that read proposals or parked (group 1).
    const BUDGET:[&str;9]=["propose","priority_scan","decide","output_scan","output_totals","apply","drain","wake_count","wake_scatter"];
    fn budget_bind(gpu:&Gpu,layout:&wgpu::BindGroupLayout,proposals:&wgpu::Buffer,parked:&wgpu::Buffer)->wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("foliage budgeted admission"),layout,
            entries:&[wgpu::BindGroupEntry {binding:4,resource:proposals.as_entire_binding()},wgpu::BindGroupEntry {binding:14,resource:parked.as_entire_binding()}]})
    }
    /// Bindings 10 (fallback) and 12 (demand) are the selector's per-tree ranges.
    fn entries<'a>(buffers:[&'a wgpu::Buffer;10],s:&'a Selector)->Vec<wgpu::BindGroupEntry<'a>> {
        Self::BINDINGS.iter().zip(buffers).map(|(&binding,b)|wgpu::BindGroupEntry {binding,resource:match binding {10=>s.fallback.binding(),12=>s.demand.binding(),_=>b.as_entire_binding()}}).collect()
    }
    fn bind(&self,gpu:&Gpu,s:&Selector,front:usize)->Binds {
        let entries=Self::entries([&s.uniform,&s._nodes.buffer,&s.asset_pages,&self.queue,&self.halves[front],&s.records.buffer,&s.counters,&s.fallback.buffer,&s.demand.buffer,&s.seed_visibility],s);
        Binds {shared:self.binds.lock().unwrap().entries(gpu,&self.layout,"foliage refinement buffers",&entries),budget:self.budget_bind.clone()}
    }
    fn set(&self,pass:&mut wgpu::ComputePass,bind:&Binds,pipeline:usize) {
        pass.set_bind_group(0,&bind.shared,&[]);if self.budget[pipeline] {pass.set_bind_group(1,&bind.budget,&[]);}
        pass.set_pipeline(&self.pipelines[pipeline]);
    }
    fn dispatch(&self,enc:&mut wgpu::CommandEncoder,bind:&Binds,pipeline:usize,groups:Option<u32>,offset:u64) {
        let mut pass=enc.begin_compute_pass(&Default::default());self.set(&mut pass,bind,pipeline);
        if let Some(n)=groups {pass.dispatch_workgroups(n,1,1);}else{pass.dispatch_workgroups_indirect(&self.args,offset);}
    }
    fn prepare(&self,enc:&mut wgpu::CommandEncoder,bind:&Binds) {
        self.dispatch(enc,bind,2,Some(1),0);
        enc.copy_buffer_to_buffer(&self.state,STATE_OFFSET+64*4,&self.args,24,12);
        {
            let mut pass=enc.begin_compute_pass(&Default::default());
            for (pipeline,direct) in [(13,false),(14,true),(15,false)] {
                self.set(&mut pass,bind,pipeline);
                if direct {pass.dispatch_workgroups(1,1,1);}else {pass.dispatch_workgroups_indirect(&self.args,24);}
            }
        }
        enc.copy_buffer_to_buffer(&self.state,STATE_OFFSET+32,&self.args,0,12);
        enc.copy_buffer_to_buffer(&self.state,STATE_OFFSET+48,&self.args,12,12);
    }
    /// Offline replay of the actual GPU proposal kernel, against final frame
    /// residency/budget. Scratch resources preserve all live queues and counters.
    pub(crate) fn trace_proposals(&self,gpu:&Gpu,s:&Selector,tasks:&[[u32;4]])->Result<Vec<[[u32;4];6]>,String> {
        if tasks.is_empty(){return Ok(Vec::new());}
        if tasks.len()>self.capacity as usize{return Err("too many offline foliage proposals".into());}
        let scratch=|label,size|gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some(label),size,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let state=scratch("offline proposal counters and state",self.state.size());let proposals=scratch("offline proposals",tasks.len()as u64*96);
        let queue=scratch("offline proposal tasks",tasks.len()as u64*16);gpu.queue.write_buffer(&queue,0,bytemuck::cast_slice(tasks));
        let pages=scratch("offline proposal pages",s.asset_pages.size());
        let count=scratch("offline proposal count",4);gpu.queue.write_buffer(&count,0,bytemuck::bytes_of(&(tasks.len()as u32)));
        let entries=Self::entries([&s.uniform,&s._nodes.buffer,&pages,&queue,&self.halves[0],&s.records.buffer,&state,&s.fallback.buffer,&s.demand.buffer,&s.seed_visibility],s);
        let bind=Binds {shared:gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {label:Some("offline proposal replay"),layout:&self.layout,entries:&entries}),
            budget:Self::budget_bind(gpu,&self.budget_layout,&proposals,&self.parked)};
        let out=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("offline proposal readback"),size:proposals.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        let mut enc=gpu.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.state,STATE_OFFSET,&state,STATE_OFFSET,self.state.size()-STATE_OFFSET);enc.copy_buffer_to_buffer(&self.state,STATE_OFFSET,&state,STATE_OFFSET+16,12);enc.copy_buffer_to_buffer(&self.state,STATE_OFFSET+69*4,&state,STATE_OFFSET+70*4,4);
        enc.copy_buffer_to_buffer(&count,0,&state,STATE_OFFSET+12,4);enc.copy_buffer_to_buffer(&s.asset_pages,0,&pages,0,pages.size());
        self.dispatch(&mut enc,&bind,3,Some((tasks.len()as u32).div_ceil(64)),0);
        enc.copy_buffer_to_buffer(&proposals,0,&out,0,proposals.size());gpu.queue.submit([enc.finish()]);
        let(tx,rx)=std::sync::mpsc::sync_channel(1);out.slice(..).map_async(wgpu::MapMode::Read,move|r|{let _=tx.send(r);});gpu.wait();rx.recv().map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
        let data=out.slice(..).get_mapped_range().map_err(|e|e.to_string())?;
        Ok(data.chunks_exact(96).map(bytemuck::pod_read_unaligned).collect())
    }
    pub fn audit(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,s:&Selector) {
        enc.clear_buffer(&self.state,STATE_OFFSET+19*4,Some(9*4));
        enc.clear_buffer(&self.state,STATE_OFFSET+72*4,Some(4));
        enc.clear_buffer(&self.state,STATE_OFFSET+32*4,Some(23*4));
        self.dispatch(enc,&self.bind(gpu,s,0),12,Some(s.count.max(s.dimensions().0).div_ceil(64)),0);
    }
    pub fn encode(&self,gpu:&Gpu,enc:&mut wgpu::CommandEncoder,s:&Selector) {
        // state[88] persists too: the shadow slot allowance from the last fast_adapt.
        // state[78..80] persists across encodes (error-cut scales: 79 camera, 78 shadow; 85 camera source),
        // and 82..85 (splat feedback: stored camera splats, booking ratio, reserved).
        let binds=[self.bind(gpu,s,0),self.bind(gpu,s,1)];enc.clear_buffer(&self.state,STATE_OFFSET+0,Some(78*4));enc.clear_buffer(&self.state,STATE_OFFSET+80*4,Some(2*4));enc.clear_buffer(&self.state,STATE_OFFSET+86*4,Some(2*4));enc.clear_buffer(&self.state,STATE_OFFSET+89*4,None);
        // state[80]: error cut active, so work() books visible splats (set before seed).
        if self.error_cut() {self.dispatch(enc,&binds[0],21,Some(1),0);}
        self.dispatch(enc,&binds[0],0,Some(s.count.div_ceil(256)),0);
        self.dispatch(enc,&binds[0],1,Some(1),0);
        self.dispatch(enc,&binds[0],16,Some(s.count.div_ceil(64)),0);
        // One extra transition admits the initial emergency proxy into the
        // recipe root. A final drain makes depth bounds conservative.
        let rounds=self.rounds(gpu,2*(s.levels+1));
        if self.error_cut() {
            // One dispatch per hierarchy level plus a one-thread advance; the
            // indirect copy is the only reason to leave the pass.
            self.dispatch(enc,&binds[0],17,Some(1),0);
            for round in 0..rounds {
                let bind=&binds[round as usize%2];
                enc.copy_buffer_to_buffer(&self.state,STATE_OFFSET+32,&self.args,0,12);
                let mut pass=enc.begin_compute_pass(&Default::default());
                // Retained shadow budget first, then the rest (fast_round_late).
                for pipeline in [18,22] {self.set(&mut pass,bind,pipeline);pass.dispatch_workgroups_indirect(&self.args,0);}
                self.set(&mut pass,bind,19);pass.dispatch_workgroups(1,1,1);
            }
            // The drain covers the live frontier (fast_advance/fast_begin size
            // it), not the whole 1M-task capacity.
            enc.copy_buffer_to_buffer(&self.state,STATE_OFFSET+89*4,&self.args,0,12);
            let bind=&binds[rounds as usize%2];self.dispatch(enc,bind,23,None,0);
            self.dispatch(enc,bind,20,Some(1),0);
            enc.copy_buffer_to_buffer(&self.state,STATE_OFFSET+68*4,&self.worked,0,4);
            return;
        }
        for round in 0..rounds {
            let bind=&binds[round as usize%2];self.prepare(enc,bind);
            // Compute dispatches are ordered storage-usage scopes. Keep the
            // dependent dispatches in one pass; only indirect argument copies
            // require leaving it. Avoid eight pass/descriptor setups per wave.
            let mut pass=enc.begin_compute_pass(&Default::default());
            for (pipeline,groups,offset) in [(3,None,0),(4,None,12),(5,Some(1),0),(6,None,0),(7,None,12),(8,Some(1),0),(9,None,0),(10,Some(1),0)] {
                self.set(&mut pass,bind,pipeline);
                if let Some(n)=groups {pass.dispatch_workgroups(n,1,1);}else{pass.dispatch_workgroups_indirect(&self.args,offset);}
            }
        }
        let bind=&binds[rounds as usize%2];self.dispatch(enc,bind,11,Some(self.capacity.div_ceil(64)),0);
        enc.copy_buffer_to_buffer(&self.state,STATE_OFFSET+68*4,&self.worked,0,4);
    }
}
#[cfg(test)] mod tests {
    use super::STATE_OFFSET;
    #[test]
    fn rounds_follow_need_and_recover_from_a_short_frame() {
        let mut r=super::Rounds {cameras:Vec::new(),stale:0,full:false,used:None,hint:None,encoded:0,frame:0,newest:0,slots:Vec::new()};
        assert_eq!(r.count(48),48,"nothing observed yet: the depth bound");
        r.next(48,6,48);assert_eq!(r.count(48),46,"shrinks gradually");
        for _ in 0..30 {let n=r.count(48);r.next(48,6,n);}
        assert_eq!(r.count(48),10,"settles at the need plus the margin");
        r.next(48,10,10);assert_eq!(r.count(48),48,"a frame that used every round may be short: back to the bound");
        r.next(48,7,48);assert_eq!(r.count(48),46);
        let mut odd=super::Rounds {cameras:Vec::new(),stale:0,full:false,used:None,hint:Some(5),encoded:0,frame:0,newest:0,slots:Vec::new()};
        assert_eq!(odd.count(48),6,"even, so the final queue matches the full bound");odd.hint=Some(0);assert_eq!(odd.count(48),2);
        odd.full=true;assert_eq!(odd.count(48),48);
        let (at,ahead)=(glam::Vec3::ZERO,glam::Vec3::NEG_Z);
        assert!(!super::Rounds::cut((at,ahead,1.),(glam::Vec3::new(0.3,0.,0.),ahead,1.)),"walking speed is not a cut");
        assert!(super::Rounds::cut((at,ahead,1.),(glam::Vec3::new(3.,0.,0.),ahead,1.)),"a jump is");
        assert!(super::Rounds::cut((at,ahead,1.),(at,glam::Vec3::new(0.1,0.,-1.).normalize(),1.)),"so is a fast turn");
        assert!(super::Rounds::cut((at,ahead,1.),(at,ahead,0.8)),"and a zoom");
    }
    #[test]
    #[ignore="exclusive GPU dormant role scheduling fixture"]
    fn dormant_shadows_wake_after_free_camera_batch() {
        use crate::foliage_select::{Node,Instance,NONE};
        let gpu=crate::Gpu::new().unwrap();
        let node=Node {sphere:[0.,0.,0.5,0.1],error:[0.;4],links:[NONE,1,0,2]};
        let tree=Instance {rows:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.]],scale:[1.,0.,0.,0.],info:[0,1,0,0]};
        let selector=crate::foliage_select::Selector::new(&gpu,&[node],&[tree,tree],2,2).unwrap();
        let r=&selector.refiner;let bind=r.bind(&gpu,&selector,0);
        let mut parked=[0u32;56];parked[..4].copy_from_slice(&[0,0,256,1]);parked[28..32].copy_from_slice(&[1,0,512,1]);
        gpu.queue.write_buffer(&r.parked,0,bytemuck::cast_slice(&parked));
        gpu.queue.write_buffer(&r.state,STATE_OFFSET+29*4,bytemuck::cast_slice(&[2u32]));
        fn read(gpu:&crate::Gpu,b:&wgpu::Buffer)->Vec<u32> {
            let out=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:b.size(),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(b,0,&out,0,b.size());let submission=gpu.queue.submit([enc.finish()]);
            let(tx,rx)=std::sync::mpsc::sync_channel(1);out.slice(..).map_async(wgpu::MapMode::Read,move|v|{let _=tx.send(v);});
            gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(30))}).unwrap();rx.recv().unwrap().unwrap();
            let bytes=out.slice(..).get_mapped_range().unwrap();bytes.chunks_exact(4).map(bytemuck::pod_read_unaligned).collect()
        }
        for (root,role,pending) in [(0,256,1),(1,512,0)] {
            gpu.queue.write_buffer(&r.state,STATE_OFFSET+3*4,bytemuck::cast_slice(&[0u32]));
            let mut enc=gpu.device.create_command_encoder(&Default::default());r.prepare(&mut enc,&bind);gpu.queue.submit([enc.finish()]);
            let state=read(&gpu,&r.state)[(STATE_OFFSET/4) as usize..].to_vec();assert_eq!(state[3],1);assert_eq!(state[30],pending);
            let queue=read(&gpu,&r.queue);assert_eq!(&queue[..4],&[root,0,role,root+2]);
        }
        gpu.queue.write_buffer(&r.state,STATE_OFFSET+3*4,bytemuck::cast_slice(&[0u32]));
        let mut enc=gpu.device.create_command_encoder(&Default::default());r.prepare(&mut enc,&bind);gpu.queue.submit([enc.finish()]);
        let state=read(&gpu,&r.state)[(STATE_OFFSET/4) as usize..].to_vec();assert_eq!(state[3],0);assert_eq!(state[60],2,"no repeated wake without new work or capacity changes");
    }
    #[test]
    #[ignore="exclusive GPU exhausted dormant queue publication oracle"]
    fn full_dormant_queue_publishes_resident_parent() {
        use crate::foliage_select::{Node,Instance,NONE};
        let gpu=crate::Gpu::new().unwrap();
        let node=Node {sphere:[0.,0.,0.5,0.1],error:[0.;4],links:[NONE,1,0,2]};
        let root=Instance {rows:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.]],scale:[1.,0.,0.,0.],info:[0,1,0,0]};
        let selector=crate::foliage_select::Selector::with_work_capacity(&gpu,&[node],&[root],2,2,2).unwrap();let r=&selector.refiner;
        gpu.queue.write_buffer(&r.queue,0,bytemuck::cast_slice(&[0u32,0,256,0]));
        let mut proposal=[0u32;20];proposal[15]=2;gpu.queue.write_buffer(&r.proposals,0,bytemuck::cast_slice(&proposal));
        gpu.queue.write_buffer(&r.state,STATE_OFFSET+3*4,bytemuck::cast_slice(&[1u32]));gpu.queue.write_buffer(&r.state,STATE_OFFSET+31*4,bytemuck::cast_slice(&[2u32]));
        let mut enc=gpu.device.create_command_encoder(&Default::default());r.dispatch(&mut enc,&r.bind(&gpu,&selector,0),9,Some(1),0);
        let out=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:4,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
        enc.copy_buffer_to_buffer(&selector.direct.buffer,selector.direct.offset,&out,0,4);gpu.queue.submit([enc.finish()]);out.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
        assert_eq!(bytemuck::pod_read_unaligned::<u32>(&out.slice(..).get_mapped_range().unwrap()),1,"full parked storage must publish the complete resident ancestor");
    }
    #[test]
    #[ignore="exclusive GPU sparse consumer seed compaction oracle"]
    fn sparse_roots_seed_stable_tasks_and_exact_budgets() {
        use crate::foliage_select::{Node,Instance,NONE,ConsumerView};
        let gpu=crate::Gpu::new().unwrap();let node=Node {sphere:[0.,0.,0.5,0.1],error:[0.;4],links:[NONE,1,0,2]};
        let root=Instance {rows:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.]],scale:[1.,0.,3.,5.],info:[0,1,0,1]};let mut roots=vec![root;513];
        for i in [0,255,511,512] {roots[i].info[3]=0;}roots[255].rows[0][3]=2.;roots[511].rows[0][3]=20.;
        let selector=crate::foliage_select::Selector::new(&gpu,&[node],&roots,8,8).unwrap();let r=&selector.refiner;
        let projection=|x|crate::part_wind::AnimationView {view_projection:glam::Mat4::from_scale(glam::Vec3::new(x,1.,1.)),viewport:[128;2]};
        let empty=gpu.device.create_texture(&wgpu::TextureDescriptor {label:None,size:wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::R32Float,usage:wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]}).create_view(&Default::default());
        for empty_world in [false,true] {
            if empty_world {for root in &mut roots {root.info[3]=1;}selector._instances.write(&gpu,0,bytemuck::cast_slice(&roots));}
            let mut enc=gpu.device.create_command_encoder(&Default::default());selector.union_consumers(&gpu,&mut enc,&[ConsumerView::camera(projection(1.)),ConsumerView::shadow(projection(0.1))],1.,8,&empty).unwrap();
            let bind=r.bind(&gpu,&selector,0);enc.clear_buffer(&r.state,0,None);r.dispatch(&mut enc,&bind,0,Some(3),0);r.dispatch(&mut enc,&bind,1,Some(1),0);r.dispatch(&mut enc,&bind,16,Some(9),0);
            let out=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:96,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            enc.copy_buffer_to_buffer(&r.state,STATE_OFFSET,&out,0,16);enc.copy_buffer_to_buffer(&r.queue,0,&out,16,80);gpu.queue.submit([enc.finish()]);out.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
            let bytes=out.slice(..).get_mapped_range().unwrap();let words:&[u32]=bytemuck::cast_slice(&bytes);
            if empty_world {assert_eq!(&words[..4],&[0,0,0,0]);}else {
                assert_eq!(&words[..4],&[0,6,15,5]);
                assert_eq!(&words[4..],&[0,NONE,256,0,0,NONE,512,0,255,NONE,512,0,512,NONE,256,0,512,NONE,512,0]);
            }
        }
    }
    #[test]
    #[ignore="exclusive GPU free shadow admission oracle"]
    fn free_shadow_ancestor_does_not_wait_for_camera_refinement() {
        use crate::foliage_select::{Node,Instance,NONE};
        let gpu=crate::Gpu::new().unwrap();let node=Node {sphere:[0.,0.,0.5,0.1],error:[0.;4],links:[NONE,1,0,2]};
        let root=Instance {rows:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.]],scale:[1.,0.,0.,0.],info:[0,1,0,0]};
        let selector=crate::foliage_select::Selector::new(&gpu,&[node],&[root],2,2).unwrap();let r=&selector.refiner;
        gpu.queue.write_buffer(&r.queue,0,bytemuck::cast_slice(&[0u32,NONE,512,0]));gpu.queue.write_buffer(&r.state,STATE_OFFSET+3*4,bytemuck::cast_slice(&[1u32]));gpu.queue.write_buffer(&r.state,STATE_OFFSET+18*4,bytemuck::cast_slice(&[1u32]));
        gpu.queue.write_buffer(&selector.uniform,32*80,bytemuck::cast_slice(&[0u32,2,0,2]));
        for (cost,expected) in [([0u32,0,0],1u32),([1,0,0],2),([0,0,1],1)] {
            let mut proposal=[0u32;20];proposal[4..7].copy_from_slice(&cost);proposal[13]=1;gpu.queue.write_buffer(&r.proposals,0,bytemuck::cast_slice(&proposal));
            let out=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:None,size:4,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            let mut enc=gpu.device.create_command_encoder(&Default::default());r.dispatch(&mut enc,&r.bind(&gpu,&selector,0),6,Some(1),0);enc.copy_buffer_to_buffer(&r.proposals,60,&out,0,4);gpu.queue.submit([enc.finish()]);out.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();
            assert_eq!(bytemuck::pod_read_unaligned::<u32>(&out.slice(..).get_mapped_range().unwrap()),expected);
        }
    }
    #[test] fn refinement_shader_validates() {
        let src=&super::refine_source();
        let module=naga::front::wgsl::parse_str(src).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(src)));
    }
}
