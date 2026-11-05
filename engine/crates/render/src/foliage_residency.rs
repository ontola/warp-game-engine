//! GPU-owned branch slots for a completed foliage selection cut.
//!
//! Slots persist by (tree, node, generation), never by CPU camera distance.
//! Scratch is O(selected record capacity); persistent state is O(slot budget),
//! not O(all trees * all branches). No readback participates in allocation.
//! This is not a draw consumer: consumers must check published AND history
//! validity, and render resident roots for the selector's fallback flags.
use crate::{Gpu, foliage_select::Selector};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ResidentSlot {
    /// Stable tree, node, representation payload, generation.
    pub key: [u32; 4],
    /// Resident asset generation (zero = free), previous-frame-valid, published-this-frame, epoch bit (128) plus published consumer bits (camera256/shadow512).
    /// A newly assigned or previously unpublished slot has no motion history.
    pub history: [u32; 4],
}

pub struct Residency {
    pub slots: wgpu::Buffer,
    /// Selected record -> slot, or NONE for fallback / duplicate records.
    pub request_slots: wgpu::Buffer,
    /// Free slots, attempts, retained, assigned, capacity failures, hash
    /// failures, published, reserved. Diagnostic readback only.
    pub counters: wgpu::Buffer,
    consumer_union: wgpu::Buffer,table: wgpu::Buffer,
    _free: wgpu::Buffer,
    _uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    pipelines: [wgpu::ComputePipeline; 5],
    indirect: wgpu::Buffer,
    capacity: u32,
}

impl Residency {
    /// One allocation domain per selector/view. Do not re-use a domain for a
    /// different camera or shadow cut in the same frame. Immutable recipes and
    /// root placement buffers may be shared; these mutable outputs may not.
    pub fn new(gpu: &Gpu, selector: &Selector, capacity: u32) -> Result<Self, String> {
        let (records, trees) = selector.dimensions();
        let table_size = records.checked_mul(2).and_then(u32::checked_next_power_of_two)
            .ok_or("foliage residency hash size overflow")?;
        let limit = gpu.device.limits();
        if capacity == 0 || capacity.div_ceil(64) > limit.max_compute_workgroups_per_dimension
            || records.div_ceil(64) > limit.max_compute_workgroups_per_dimension
            || limit.max_storage_buffers_per_shader_stage < 11 {
            return Err("foliage residency exceeds device dispatch/binding limits".into());
        }
        for bytes in [capacity as u64 * 32, capacity as u64 * 4, records as u64 * 4, table_size as u64 * 4] {
            if bytes > limit.max_storage_buffer_binding_size as u64 || bytes > limit.max_buffer_size {
                return Err("foliage residency exceeds device buffer limit".into());
            }
        }
        crate::shaders::checked(gpu, || {
            let buffer = |label, size| gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label), size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let slots = buffer("GPU resident branch slots", capacity as u64 * 32);
            let request_slots = buffer("GPU selected branch slots", records as u64 * 4);
            let counters = buffer("GPU branch residency counters", 32);
            let consumer_union=buffer("GPU branch consumer union",records as u64*4);
            let table = buffer("GPU branch request hash", table_size as u64 * 4);
            let free = buffer("GPU branch free slots", capacity as u64 * 4);
            let uniform = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("GPU branch residency bounds"),
                contents: bytemuck::cast_slice(&[records, capacity, table_size, trees, 64u32.min(table_size), 0, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let entries = (0..12).map(|binding| wgpu::BindGroupLayoutEntry {
                binding, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                ty: wgpu::BindingType::Buffer {
                    ty: if binding == 0 { wgpu::BufferBindingType::Uniform }
                        else { wgpu::BufferBindingType::Storage { read_only: binding <= 2 || binding == 9 || binding == 10 } },
                    has_dynamic_offset: false, min_binding_size: None,
                },
            }).collect::<Vec<_>>();
            let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("GPU branch residency"), entries: &entries,
            });
            let buffers = [&uniform, &selector.records, &selector.counters, &selector.fallback,
                &slots, &table, &request_slots, &free, &counters, &selector.asset_pages,&selector.record_consumers,&consumer_union];
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("GPU branch residency"), layout: &layout,
                entries: &buffers.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32, resource: b.as_entire_binding(),
                }).collect::<Vec<_>>(),
            });
            let layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("GPU branch residency"), bind_group_layouts: &[Some(&layout)], immediate_size: 0,
            });
            let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("GPU branch residency"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/foliage_residency.wgsl").into()),
            });
            let pipelines = ["index_requests", "retain_slots", "allocate_slots", "publish_slots", "resolve_requests"]
                .map(|entry| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry), layout: Some(&layout), module: &module, entry_point: Some(entry),
                    compilation_options: Default::default(), cache: gpu.pipeline_cache(),
                }));
            Self { slots, request_slots, counters, consumer_union, table, _free: free, _uniform: uniform, bind,
                pipelines, indirect: selector.arguments.clone(), capacity }
        })
    }

    /// Encode after BOTH selection phases, before any final source/voxel draws.
    /// The selector resets fallback at its first phase; this appends allocation
    /// failure flags. Previous slot history is retained solely on the GPU.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.clear_buffer(&self.table, 0, None);
        encoder.clear_buffer(&self.consumer_union,0,None);
        encoder.clear_buffer(&self.counters, 0, None);
        for (index, pipeline) in self.pipelines.iter().enumerate() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("GPU branch residency"), ..Default::default()
            });
            pass.set_pipeline(pipeline); pass.set_bind_group(0, &self.bind, &[]);
            if index == 1 || index == 3 { pass.dispatch_workgroups(self.capacity.div_ceil(64), 1, 1); }
            else { pass.dispatch_workgroups_indirect(&self.indirect, 12); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foliage_select::{Instance, Node, NONE};
    fn read(gpu:&Gpu, source:&wgpu::Buffer)->Vec<u32> {
        let dst=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("residency test readback"),size:source.size(),
            usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(source,0,&dst,0,source.size());
        let submission=gpu.queue.submit([enc.finish()]);let(tx,rx)=std::sync::mpsc::sync_channel(1);
        dst.slice(..).map_async(wgpu::MapMode::Read,move|r|{let _=tx.send(r);});
        gpu.device.poll(wgpu::PollType::Wait {submission_index:Some(submission),timeout:Some(std::time::Duration::from_secs(30))}).unwrap();
        rx.recv().unwrap().unwrap();let bytes=dst.slice(..).get_mapped_range().unwrap();
        bytes.chunks_exact(4).map(bytemuck::pod_read_unaligned).collect()
    }
    fn selector(gpu:&Gpu)->Selector {
        // The synthetic request stream below uses representations 10..=12.
        // Give it a real published asset catalog, as production selection does.
        let nodes=(0..13).map(|rep| Node {sphere:[0.,0.,0.5,0.1],error:[0.;4],links:[NONE,rep+1,rep,0]}).collect::<Vec<_>>();
        let instance=Instance {rows:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.]],scale:[1.,0.,0.,0.],info:[0,0,0,0]};
        Selector::new(gpu,&nodes,&vec![instance;2400],8192,8192).unwrap()
    }
    fn run(gpu:&Gpu, select:&Selector, resident:&Residency, requests:&[[u32;4]])->(Vec<u32>,Vec<u32>,Vec<u32>) {
        // Fixture only: stand in for the selector's completed record stream.
        // Production has no CPU record upload or allocation decision.
        gpu.queue.write_buffer(&select.records,0,bytemuck::cast_slice(requests));
        gpu.queue.write_buffer(&select.counters,0,bytemuck::cast_slice(&[requests.len()as u32]));
        gpu.queue.write_buffer(&select.arguments,12,bytemuck::cast_slice(&[(requests.len()as u32).div_ceil(64),1,1]));
        let mut enc=gpu.device.create_command_encoder(&Default::default());enc.clear_buffer(&select.fallback,0,None);
        resident.encode(&mut enc);gpu.queue.submit([enc.finish()]);
        (read(gpu,&resident.slots),read(gpu,&resident.request_slots),read(gpu,&select.fallback))
    }
    #[test]
    #[ignore="exclusive GPU residency fixture"]
    fn gpu_residency_survives_reordering_reuse_generation_and_overflow() {
        let gpu=Gpu::new().unwrap();let select=selector(&gpu);let resident=Residency::new(&gpu,&select,4096).unwrap();
        let records=(0..1200).flat_map(|tree|(0..3).map(move|node|[tree,node,node+10,4])).collect::<Vec<_>>();
        let (slots,map,fallback)=run(&gpu,&select,&resident,&records);
        assert!(fallback.iter().all(|f|*f==0));let mut old=std::collections::HashMap::new();
        for (r,&slot) in records.iter().zip(&map) {
            assert!(slot<4096);let state=&slots[slot as usize*8..][..8];
            assert_eq!(&state[..4],r);assert_eq!(&state[4..7],&[1,0,1]);old.insert(*r,(slot,state[7]));
        }
        let mut reversed=records.clone();reversed.reverse();
        let (slots,map,fallback)=run(&gpu,&select,&resident,&reversed);
        assert!(fallback.iter().all(|f|*f==0));
        for (r,&slot) in reversed.iter().zip(&map) {
            assert_eq!(slot,old[r].0);assert_eq!(&slots[slot as usize*8+4..][..3],&[1,1,1]);
        }
        // Keep half, replace half with different stable tree IDs; same budget,
        // no CPU slot map or preselected full-tree limit participates.
        let mut changed=records.clone();for r in &mut changed[1800..] {r[0]+=1200;}
        let (slots,map,fallback)=run(&gpu,&select,&resident,&changed);
        assert!(fallback.iter().all(|f|*f==0));
        for (i,(r,&slot)) in changed.iter().zip(&map).enumerate() {
            let state=&slots[slot as usize*8..][..8];assert_eq!(&state[..4],r);
            assert_eq!(state[5],u32::from(i<1800));if i<1800 {assert_eq!(slot,old[r].0);}
        }
        for r in &mut changed {r[3]+=1;}
        let (slots,map,fallback)=run(&gpu,&select,&resident,&changed);
        assert!(fallback.iter().all(|f|*f==0));
        for &slot in map.iter().take(changed.len()) {assert_eq!(slots[slot as usize*8+5],0);}
        // Partial allocation must never publish half a tree: every failed
        // instance becomes exactly one root at the draw consumer.
        let small=Residency::new(&gpu,&select,5).unwrap();
        let (slots,map,fallback)=run(&gpu,&select,&small,&records);
        assert!(fallback[..1200].iter().any(|f|*f!=0));
        for (r,&slot) in records.iter().zip(&map) {
            if fallback[r[0]as usize]!=0 {assert_eq!(slot,NONE);}
            else {assert!(slot<5);assert_eq!(slots[slot as usize*8+6],1);}
        }
        for slot in slots.chunks_exact(8).filter(|s|s[6]!=0) {assert_eq!(fallback[slot[0]as usize],0);}
        let hidden=slots.chunks_exact(8).find(|s|s[4]!=0 && s[6]==0).unwrap();
        let key:[u32;4]=hidden[..4].try_into().unwrap();
        let (slots,map,fallback)=run(&gpu,&select,&small,&[key]);
        assert_eq!(fallback[key[0]as usize],0);assert_ne!(map[0],NONE);
        assert_eq!(slots[map[0]as usize*8+5],0,"a fallback frame cannot supply source motion history");
        // Duplicate demands emit only once and share no mutable map reads.
        let duplicates=vec![[1,2,12,0];512];
        let (slots,map,fallback)=run(&gpu,&select,&small,&duplicates);
        assert_eq!(fallback[1],0);assert_eq!(map[..512].iter().filter(|s|**s!=NONE).count(),1);
        assert_eq!(slots.chunks_exact(8).filter(|s|s[6]!=0).count(),1);
        let (slots,_,_)=run(&gpu,&select,&small,&[]);
        assert!(slots.chunks_exact(8).all(|s|s[4]==0 && s[5]==0 && s[6]==0));
        println!("GPU branch residency: 3600 branches / 1200 trees; reorder, reuse, generation, overflow and duplicate requests PASS");
    }
    #[test]
    #[ignore="exclusive GPU shared consumer publication fixture"]
    fn gpu_consumer_union_and_new_consumer_history() {
        let gpu=Gpu::new().unwrap();let select=selector(&gpu);let resident=Residency::new(&gpu,&select,8).unwrap();
        let key=[1,2,12,4];
        gpu.queue.write_buffer(&select.record_consumers,0,bytemuck::cast_slice(&[256u32,512,256]));
        let (slots,map,_)=run(&gpu,&select,&resident,&[key,key,key]);
        let index=*map[..3].iter().find(|v|**v!=NONE).unwrap()as usize;
        assert_eq!(map[..3].iter().filter(|v|**v!=NONE).count(),1);
        assert_eq!(slots[index*8+7]&768,768,"duplicate camera/shadow demands share one slot");
        let epoch=slots[index*8+7]&128;
        gpu.queue.write_buffer(&select.record_consumers,0,bytemuck::cast_slice(&[512u32]));
        let (slots,map,_)=run(&gpu,&select,&resident,&[key]);assert_eq!(map[0]as usize,index);
        assert_eq!(slots[index*8+7]&768,512);assert_eq!(slots[index*8+5],1);assert_eq!(slots[index*8+7]&128,epoch);
        gpu.queue.write_buffer(&select.record_consumers,0,bytemuck::cast_slice(&[256u32]));
        let (slots,map,_)=run(&gpu,&select,&resident,&[key]);assert_eq!(map[0]as usize,index);
        assert_eq!(slots[index*8+7]&768,256);assert_eq!(slots[index*8+5],0,"newly visible camera cannot reuse shadow-only motion history");
        assert_ne!(slots[index*8+7]&128,epoch);
        let (slots,_,_)=run(&gpu,&select,&resident,&[key]);assert_eq!(slots[index*8+5],1);
        gpu.queue.write_buffer(&select.record_consumers,0,bytemuck::cast_slice(&[512u32,256]));
        let (slots,map,_)=run(&gpu,&select,&resident,&[key,key]);
        let index=*map[..2].iter().find(|v|**v!=NONE).unwrap()as usize;
        assert_eq!(slots[index*8+7]&768,768);assert_eq!(slots[index*8+5],0);
    }
    #[test]
    fn residency_shader_validates() {
        let src = include_str!("shaders/foliage_residency.wgsl");
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        assert_eq!(std::mem::size_of::<super::ResidentSlot>(), 32);
    }
}
