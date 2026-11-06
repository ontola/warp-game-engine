//! Bounded voxel asset storage and asynchronous demand service. CPU services
//! asset bytes only; the GPU still chooses every root/branch cut.
use super::{VoxelPage,Arena,Allocation};
use crate::{Gpu,foliage_volume_data::GpuForest,foliage_select::Selector};
use std::{collections::BTreeSet,sync::mpsc};

#[derive(Clone,Copy,Debug)]
pub struct Budget {pub index_words:u32,pub cells:u32,pub splats:u32,pub upload_bytes:u64,pub uploads:u32}
impl Default for Budget {
    // 256 MiB of cells: at 64 MiB the lowland view missed ~600 pages a frame
    // forever and kept 9.8 px cells 14 m from the camera; 256 MiB serves the
    // whole view (2252 pages, 233 MB allocated, RTX 5070 Ti, 2026-11-06).
    fn default()->Self {Self {index_words:16*1024*1024,cells:256*1024*1024/128,splats:2*1024*1024,upload_bytes:8*1024*1024,uploads:8}}
}
#[derive(Clone)]
struct Resident {index:Allocation,cell:Allocation,splat:Allocation,last_used:u64}
struct Pending {id:usize,resident:Resident,level:crate::foliage_volume_data::GpuLevel,indices:Vec<u32>,splats:Vec<[u32;2]>,cells:Vec<crate::foliage_volume_data::PackedCell>,cursor:u64}
struct Feedback {buffer:wgpu::Buffer,sequence:u64,pending:Option<mpsc::Receiver<Result<(),wgpu::BufferAsyncError>>>}
#[derive(Clone,Copy,Debug,Default,serde::Serialize)]
pub struct Stats {pub allocated_bytes:u64,pub resident_pages:u32,pub pending_pages:u32,pub uploads:u64,pub evictions:u64,pub uploaded_bytes:u64,pub frame_upload_bytes:u64,pub misses:u64,pub feedback_skipped:u64,pub source:Option<super::source::Stats>}
pub struct Cache {
    pub source:Option<super::source::Cache>,source_wanted:BTreeSet<usize>,
    pages:Vec<VoxelPage>,deps:Vec<Vec<usize>>,pinned:Vec<bool>,resident:Vec<Option<Resident>>,
    capacities:[u32;3],last_requested:Vec<u64>,
    /// Urgency the GPU last asked for each page with (projected pixel error
    /// of the cut waiting on it, x16; camera x4). Uploads go most urgent first.
    priority:Vec<u32>,index:Arena,cell:Arena,splat:Arena,pending:Option<Pending>,budget:Budget,
    pub buffers:[wgpu::Buffer;4], // levels, indices + brick descriptors, cells, splats (two logical headers + pool)
    publication:Vec<(bool,u32)>,dirty:BTreeSet<usize>,wanted:BTreeSet<usize>,frame:u64,
    feedback:Vec<Feedback>,last_feedback:u64,stats:Stats,
}
impl Cache {
    pub fn for_scene(gpu:&Gpu,scene:&crate::Scene)->Result<Option<Self>,String>{
        let Some(spec)=&scene.gpu_forest else{return Ok(None);};let forest=scene.voxel_forest.as_ref().ok_or("GPU forest missing voxel assets")?;
        let(deps,pinned)=spec.voxel_page_dependencies(forest.levels.len())?;
        let mut cache=Self::new(gpu,forest,deps,pinned,spec.page_budget)?;
        cache.source=Some(super::source::Cache::new(gpu,scene,&spec.source_page_instances(),spec.source_budget)?);
        Ok(Some(cache))
    }

    pub fn new(gpu:&Gpu,forest:&GpuForest,deps:Vec<Vec<usize>>,pinned:Vec<bool>,budget:Budget)->Result<Self,String> {
        if budget.upload_bytes<52 || budget.uploads==0 || pinned.len()!=forest.levels.len()
            || deps.iter().flatten().any(|&i|i>=forest.levels.len()) {return Err("invalid voxel cache contract/budget".into());}
        let pages=(0..forest.levels.len()).map(|i|VoxelPage::cook(forest,i).map(|mut p|{p.group_splats();p})).collect::<Result<Vec<_>,_>>()?;
        let total=|f:fn(&VoxelPage)->usize| pages.iter().map(|p|f(p)as u64).sum::<u64>();
        // Each arena is one binding: shrink to the device's binding limit (WebGPU's
        // default is 128 MiB) rather than fail. Coarser fallbacks cover the rest.
        let limit=gpu.device.limits().max_storage_buffer_binding_size as u64;
        let fit=[limit/4,limit/128,(limit/8).saturating_sub(2*pages.len()as u64)];
        let capacities=[(budget.index_words as u64).min(total(|p|p.index_words())).min(fit[0])as u32,
            (budget.cells as u64).min(total(|p|p.cells.len())).min(fit[1])as u32,(budget.splats as u64).min(total(|p|p.splats.len())).min(fit[2])as u32];
        let pinned_sizes=[pages.iter().enumerate().filter(|(i,_)|pinned[*i]).map(|(_,p)|p.index_words()as u64).sum::<u64>(),
            pages.iter().enumerate().filter(|(i,_)|pinned[*i]).map(|(_,p)|p.cells.len()as u64).sum::<u64>(),
            pages.iter().enumerate().filter(|(i,_)|pinned[*i]).map(|(_,p)|p.splats.len()as u64).sum::<u64>()];
        if (0..3).any(|i|pinned_sizes[i]>capacities[i]as u64) {return Err("pinned voxel ancestors exceed physical cache budget".into());}
        let sizes=[pages.len()as u64*32,capacities[0]as u64*4,capacities[1]as u64*128,(2*pages.len()as u64+capacities[2]as u64)*8];
        if sizes.iter().any(|&s|s>gpu.device.limits().max_storage_buffer_binding_size as u64) {return Err("voxel cache exceeds device buffer limit".into());}
        let buffers=std::array::from_fn(|i|gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("bounded voxel asset arena"),size:sizes[i].max(176),usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false}));
        let last_requested=vec![0;deps.len()];
        let priority=vec![0;deps.len()];
        let mut this=Self {source:None,source_wanted:BTreeSet::from([0]),capacities,last_requested,priority,resident:vec![None;pages.len()],pages,deps,pinned,index:Arena::new(capacities[0]),cell:Arena::new(capacities[1]),splat:Arena::new(capacities[2]),pending:None,budget,buffers,publication:Vec::new(),dirty:BTreeSet::new(),wanted:BTreeSet::new(),frame:0,feedback:Vec::new(),last_feedback:0,stats:Stats {allocated_bytes:sizes.iter().map(|s|(*s).max(176)).sum(),..Default::default()}};
        this.publication=vec![(true,1);this.deps.len()];
        this.dirty.extend(0..this.pages.len());
        // Ancestors are uploaded at construction, before any selector can use
        // the cache. Startup uploads are distinct from per-frame upload budget.
        for id in 0..this.pages.len() {if this.pinned[id] {
            this.begin(id)?;if this.pending.is_none(){return Err("cannot allocate pinned voxel ancestor".into());}
            this.advance(gpu,u64::MAX)?;
        }}
        this.stats.frame_upload_bytes=0;
        Ok(this)
    }
    fn begin(&mut self,id:usize)->Result<(),String> {
        if self.pending.is_some() || self.resident[id].is_some(){return Ok(());}
        let page=&self.pages[id];
        let Some(index)=self.index.allocate(page.index_words()as u32) else{return Ok(());};
        let Some(cell)=self.cell.allocate(page.cells.len()as u32) else{self.index.release(index)?;return Ok(());};
        let Some(splat)=self.splat.allocate(page.splats.len()as u32) else{self.index.release(index)?;self.cell.release(cell)?;return Ok(());};
        let (level,mut indices,splats)=page.relocate(index.range.start,cell.range.start)?;
        let first=2*self.pages.len()as u32+splat.range.start;
        for brick in &page.splat_bricks {indices.extend_from_slice(&[first+brick[0],brick[1],brick[2],0]);}
        self.pending=Some(Pending {id,resident:Resident {index,cell,splat,last_used:self.frame},level,indices,splats,cells:crate::foliage_volume_data::packed_cells(&page.cells),cursor:0});Ok(())
    }
    fn advance(&mut self,gpu:&Gpu,budget:u64)->Result<u64,String> {
        let Some(p)=self.pending.as_mut()else{return Ok(0);};let mut remaining=budget&!3;let initial=remaining;
        let bytes:[&[u8];3]=[bytemuck::cast_slice(&p.indices),bytemuck::cast_slice(&p.cells),bytemuck::cast_slice(&p.splats)];
        let bases=[p.resident.index.range.start as u64*4,p.resident.cell.range.start as u64*128,(2*self.pages.len()as u64+p.resident.splat.range.start as u64)*8];
        let mut begin=0u64;
        for i in 0..3 {
            let end=begin+bytes[i].len()as u64;
            if p.cursor<end && remaining>0 {
                let offset=p.cursor.saturating_sub(begin);let n=(end-p.cursor).min(remaining);
                gpu.queue.write_buffer(&self.buffers[i+1],bases[i]+offset,&bytes[i][offset as usize..(offset+n)as usize]);
                p.cursor+=n;remaining-=n;
            }
            begin=end;
        }
        // Data first, logical headers last. A partially uploaded page is never
        // marked resident and cannot be selected by any camera/shadow consumer.
        if p.cursor==begin && remaining>=48 {
            gpu.queue.write_buffer(&self.buffers[0],p.id as u64*32,bytemuck::bytes_of(&p.level));
            gpu.queue.write_buffer(&self.buffers[3],p.id as u64*8,bytemuck::cast_slice(&[2*self.pages.len()as u32+p.resident.splat.range.start,p.splats.len()as u32]));
            let page=&self.pages[p.id];
            gpu.queue.write_buffer(&self.buffers[3],(self.pages.len()+p.id)as u64*8,bytemuck::cast_slice(&[p.resident.index.range.start+page.indices.len()as u32,page.splat_bricks.len()as u32]));
            remaining-=48;let p=self.pending.take().unwrap();self.dirty.insert(p.id);self.resident[p.id]=Some(p.resident);self.stats.uploads+=1;
        }
        let written=initial-remaining;self.stats.uploaded_bytes+=written;Ok(written)
    }
    fn evict(&mut self,id:usize)->Result<(),String> {
        if self.pinned[id]{return Err("cannot evict pinned voxel ancestor".into());}
        if let Some(r)=self.resident[id].take(){self.index.release(r.index)?;self.cell.release(r.cell)?;self.splat.release(r.splat)?;self.dirty.insert(id);self.stats.evictions+=1;}
        Ok(())
    }
    fn publish(&mut self,gpu:&Gpu,runtime:&Selector)->Result<(),String> {
        if self.dirty.is_empty() && self.source.as_ref().is_none_or(|s|s.dirty.is_empty()){return Ok(());}
        let mut updates=Vec::new();
        for (id,deps) in self.deps.iter().enumerate() {
            let ready=deps.iter().all(|&p|self.resident[p].is_some()) && self.source.as_ref().is_none_or(|s|s.ready(id));let old=self.publication[id];
            if ready!=old.0 || deps.iter().any(|p|self.dirty.contains(p)) || self.source.as_ref().is_some_and(|s|s.changed(id)) {
                updates.push((id as u32,ready,old.1.checked_add(1).ok_or("voxel asset generation exhausted")?));
            }
        }
        runtime.publish_asset_pages(gpu,&updates)?;
        for (id,ready,generation) in updates {self.publication[id as usize]=(ready,generation);}
        self.dirty.clear();if let Some(s)=&mut self.source{s.dirty.clear();}Ok(())
    }
    pub fn initialize(&mut self,gpu:&Gpu,runtime:&Selector)->Result<(),String> {
        self.publish(gpu,runtime)?;
        let size=runtime.asset_requests.size();
        self.feedback=(0..2).map(|_|Feedback {buffer:gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("async voxel asset demand"),size,usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false}),sequence:0,pending:None}).collect();Ok(())
    }
    pub fn service(&mut self,gpu:&Gpu,runtime:&Selector)->Result<(),String> {
        self.frame+=1;
        if self.pending.is_none() && self.resident.iter().all(Option::is_some) && self.source.as_ref().is_none_or(|s|s.complete()) {
            // Immutable catalog fully resident: no pressure can arise from a
            // different camera/root placement. Stop redundant feedback copies.
            self.feedback.clear();self.stats.frame_upload_bytes=0;return Ok(());
        }
        gpu.device.poll(wgpu::PollType::Poll).map_err(|e|e.to_string())?;
        // Pending results may be old. Only matching publication generations
        // affect current demand; no CPU visibility or branch selection occurs.
        let mut observed=None;
        for slot in &mut self.feedback {
            let result=slot.pending.as_ref().and_then(|rx|rx.try_recv().ok());
            if let Some(result)=result {
                slot.pending=None;result.map_err(|e|e.to_string())?;
                let bytes=slot.buffer.slice(..).get_mapped_range().map_err(|e|e.to_string())?;
                let words:&[u32]=bytemuck::cast_slice(&bytes);let count=(words[0]as usize).min(self.deps.len());
                let mut wanted=BTreeSet::new();
                for r in words[4..4+count*4].chunks_exact(4) {let id=r[0]as usize;
                    if id<self.deps.len() && r[2]==self.publication[id].1 {wanted.insert(id);self.priority[id]=r[3];}
                }
                if slot.sequence>self.last_feedback {self.last_feedback=slot.sequence;observed=Some(wanted);}
                drop(bytes);slot.buffer.unmap();
            }
        }
        if let Some(requests)=observed {
            let mut source_wanted=BTreeSet::from([0]);let mut source_used=self.source.as_ref().map_or([0;3],|s|s.cost(0));
            for id in requests {self.last_requested[id]=self.frame;}
            // Admit whole dependency bundles, retaining already-ready bundles
            // first. Short leases bridge async feedback and failed-cut frames.
            let mut requests=(0..self.deps.len()).filter(|&id|self.last_requested[id]!=0 && self.frame-self.last_requested[id]<=8).collect::<Vec<_>>();
            requests.sort_by_key(|&id|(!(self.deps[id].iter().all(|&p|self.resident[p].is_some()) && self.source.as_ref().is_none_or(|s|s.ready(id))),std::cmp::Reverse(self.priority[id]),id));
            let cost=|id:usize|[self.pages[id].index_words()as u64,self.pages[id].cells.len()as u64,self.pages[id].splats.len()as u64];
            let mut wanted=self.pinned.iter().enumerate().filter_map(|(id,&pin)|pin.then_some(id)).collect::<BTreeSet<_>>();
            let mut used=[0u64;3];for &id in &wanted {let c=cost(id);for i in 0..3 {used[i]+=c[i];}}
            for id in requests {
                let mut extra=[0u64;3];for &page in &self.deps[id] {if !wanted.contains(&page){let c=cost(page);for i in 0..3{extra[i]+=c[i];}}}
                let mut source_extra=[0u64;3];if let Some(s)=&self.source{for &p in &s.deps[id]{if !source_wanted.contains(&p){let c=s.cost(p);for i in 0..3{source_extra[i]+=c[i];}}}}
                if (0..3).all(|i|used[i]+extra[i]<=self.capacities[i]as u64) && self.source.as_ref().is_none_or(|s|(0..3).all(|i|source_used[i]+source_extra[i]<=s.capacities[i])) {
                    for i in 0..3{used[i]+=extra[i];}wanted.extend(self.deps[id].iter().copied());
                    if let Some(s)=&self.source{source_wanted.extend(s.deps[id].iter().copied());for i in 0..3{source_used[i]+=source_extra[i];}}
                }else {self.stats.misses+=1;}
            }
            self.wanted=wanted;self.source_wanted=source_wanted;
        }
        for &id in &self.wanted {if let Some(r)=&mut self.resident[id]{r.last_used=self.frame;}}
        let mut remaining=self.budget.upload_bytes&!3;let mut completed=0;
        if let Some(source)=&mut self.source{source.service(gpu,&self.source_wanted,self.frame,&mut remaining,&mut completed,self.budget.uploads)?;}
        if self.pending.is_some(){remaining-=self.advance(gpu,remaining)?;if self.pending.is_none(){completed+=1;}}
        // Most urgent first: a page's urgency is the highest of the requests
        // whose bundles hold it (a bush 12 m away before distant detail).
        let mut urgency=vec![0u32;self.pages.len()];
        for (id,deps) in self.deps.iter().enumerate() {if self.frame-self.last_requested[id]<=8 {for &p in deps {urgency[p]=urgency[p].max(self.priority[id]);}}}
        let mut candidates=self.wanted.iter().copied().filter(|&id|self.resident[id].is_none()).collect::<Vec<_>>();
        candidates.sort_by_key(|&id|(std::cmp::Reverse(urgency[id]),id));
        for id in candidates {
            if self.pending.is_some() || remaining<52 || completed>=self.budget.uploads {break;}
            self.begin(id)?;
            while self.pending.is_none() {
                let victim=self.resident.iter().enumerate().filter(|(i,r)|r.is_some()&&!self.pinned[*i]&&!self.wanted.contains(i))
                    .min_by_key(|(_,r)|r.as_ref().unwrap().last_used).map(|(i,_)|i);
                let Some(victim)=victim else{self.stats.misses+=1;break;};self.evict(victim)?;self.begin(id)?;
            }
            if self.pending.is_some(){remaining-=self.advance(gpu,remaining)?;if self.pending.is_none(){completed+=1;}}
        }
        self.stats.frame_upload_bytes=(self.budget.upload_bytes&!3)-remaining;
        self.publish(gpu,runtime)?;
        // Copy last submitted GPU demand before this frame's selector overwrites
        // it. This submission/readback never waits and uses a bounded two-slot ring.
        if let Some(slot)=self.feedback.iter_mut().find(|s|s.pending.is_none()) {
            let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(&runtime.asset_requests,0,&slot.buffer,0,slot.buffer.size());gpu.queue.submit(Some(enc.finish()));
            let(tx,rx)=mpsc::sync_channel(1);slot.buffer.slice(..).map_async(wgpu::MapMode::Read,move|r|{let _=tx.send(r);});slot.pending=Some(rx);slot.sequence=self.frame;
        }else {self.stats.feedback_skipped+=1;}
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn warm_fixture(&mut self,gpu:&Gpu,selector:&Selector)->Result<(),String>{
        for id in 0..self.pages.len(){if self.resident[id].is_none(){self.begin(id)?;if self.pending.is_none(){return Err("fixture cache too small for warm oracle".into());}self.advance(gpu,u64::MAX)?;}}
        if let Some(source)=&mut self.source{source.warm(gpu)?;}
        self.publish(gpu,selector)
    }
    pub fn stats(&self)->Stats {Stats {source:self.source.as_ref().map(|s|s.stats()),resident_pages:self.resident.iter().filter(|r|r.is_some()).count()as u32,pending_pages:u32::from(self.pending.is_some()),..self.stats}}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{foliage_select::{Node,Instance,NONE},foliage_volume_data::{GpuLevel,GpuCell}};
    use bytemuck::Zeroable;
    #[test]
    #[ignore="exclusive bounded voxel page GPU fixture"]
    fn foliage_pages_gpu_budget_partial_upload_eviction_and_feedback() {
        let gpu=Gpu::new().unwrap();
        let forest=GpuForest {levels:(0..3).map(|i|GpuLevel {origin_cell:[0.,0.,0.,1.],dims_offset:[1,1,1,i]}).collect(),indices:vec![1,2,3],
            cells:(0..3).map(|i|{let mut c=GpuCell::zeroed();c.albedo_rough=[i as f32,0.5,0.2,0.8];c}).collect(),instances:vec![]};
        let nodes=(0..3).map(|i|Node {sphere:[0.,0.,0.,1.],error:[0.;4],links:[NONE,i+1,i,if i==0{2}else{0}]}).collect::<Vec<_>>();
        let roots=[Instance {rows:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.]],scale:[1.,0.,0.,0.],info:[0;4]}];
        let selector=Selector::new(&gpu,&nodes,&roots,8,8).unwrap();
        let budget=Budget {index_words:10,cells:2,splats:2,upload_bytes:52,uploads:1};
        let mut cache=Cache::new(&gpu,&forest,vec![vec![0],vec![1],vec![2]],vec![true,false,false],budget).unwrap();
        cache.initialize(&gpu,&selector).unwrap();assert_eq!(cache.stats().resident_pages,1);assert!(cache.evict(0).is_err());
        let bytes=cache.stats().allocated_bytes;
        let read=|buffer:&wgpu::Buffer| {
            let dst=gpu.device.create_buffer(&wgpu::BufferDescriptor {label:Some("page fixture read"),size:buffer.size(),usage:wgpu::BufferUsages::MAP_READ|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            let mut enc=gpu.device.create_command_encoder(&Default::default());enc.copy_buffer_to_buffer(buffer,0,&dst,0,buffer.size());gpu.queue.submit(Some(enc.finish()));dst.slice(..).map_async(wgpu::MapMode::Read,|_|{});gpu.wait();let bytes=dst.slice(..).get_mapped_range().unwrap().to_vec();dst.unmap();bytes
        };
        for target in [1usize,2,1] {
            for frame in 0..40 {
                gpu.queue.write_buffer(&selector.asset_requests,0,bytemuck::cast_slice(&[1u32,0,0,0,target as u32,0,cache.publication[target].1,0]));
                cache.service(&gpu,&selector).unwrap();gpu.wait();
                assert!(cache.stats().frame_upload_bytes<=52);assert_eq!(cache.stats().allocated_bytes,bytes);
                assert!(cache.resident[0].is_some());assert!(cache.stats().resident_pages<=2);
                if frame<2 {assert!(!cache.publication[target].0,"partial bytes cannot publish a page");}
                if cache.pending.as_ref().is_some_and(|p|p.id==target){assert!(!cache.publication[target].0);}
                if cache.publication[target].0 {break;}
            }
            assert!(cache.publication[target].0,"requested page eventually becomes resident");
            let raw=read(&cache.buffers[0]);let level:GpuLevel=bytemuck::pod_read_unaligned(&raw[target*32..target*32+32]);
            let raw=read(&cache.buffers[1]);let index:u32=bytemuck::pod_read_unaligned(&raw[level.dims_offset[3]as usize*4..][..4]);
            let raw=read(&cache.buffers[2]);let cell:crate::foliage_volume_data::PackedCell=bytemuck::pod_read_unaligned(&raw[(index as usize-1)*128..][..128]);
            assert_eq!(bytemuck::bytes_of(&cell),bytemuck::bytes_of(&crate::foliage_volume_data::PackedCell::from(&forest.cells[target])));
            let raw=read(&cache.buffers[3]);let header:[u32;2]=bytemuck::pod_read_unaligned(&raw[target*8..][..8]);assert_eq!(header[1],1);
            let splat:[u32;2]=bytemuck::pod_read_unaligned(&raw[header[0]as usize*8..][..8]);assert_eq!(splat,[index-1,0]);
            let group:[u32;2]=bytemuck::pod_read_unaligned(&raw[(forest.levels.len()+target)*8..][..8]);assert_eq!(group[1],1);
            let raw=read(&cache.buffers[1]);let descriptor:[u32;4]=bytemuck::pod_read_unaligned(&raw[group[0]as usize*4..][..16]);
            assert_eq!(descriptor,[header[0],1,0,0],"reloaded brick range must address this page's current splat allocation");
        }
        assert!(cache.stats().evictions>=2);assert!(cache.publication[1].1>2);
        assert!(Cache::new(&gpu,&forest,vec![vec![0]],vec![true,true,true],budget).is_err());
        // Voxel readiness alone may not publish a source representation.
        let selector=Selector::new(&gpu,&nodes,&roots,8,8).unwrap();
        let mut combined=Cache::new(&gpu,&forest,vec![vec![0],vec![1],vec![2]],vec![true,true,true],Budget{index_words:15,cells:3,splats:3,upload_bytes:512,uploads:2}).unwrap();
        let(scene,reps)=super::super::source::tests::scene();
        combined.source=Some(super::super::source::Cache::new(&gpu,&scene,&reps,super::super::source::Budget{vertices:195,cluster_vertices:195,triangles:65}).unwrap());
        combined.initialize(&gpu,&selector).unwrap();assert!(combined.publication[0].0);assert!(!combined.publication[1].0);assert!(!combined.publication[2].0);
        for target in [1usize,2,1]{
            for _ in 0..100 {
                gpu.queue.write_buffer(&selector.asset_requests,0,bytemuck::cast_slice(&[1u32,0,0,0,target as u32,0,combined.publication[target].1,0]));combined.service(&gpu,&selector).unwrap();gpu.wait();
                assert!(combined.stats().frame_upload_bytes<=512);
                if combined.publication[target].0{break;}
            }
            assert!(combined.publication[target].0);assert!(!combined.publication[3-target].0);
        }
        assert_eq!(combined.source.as_ref().unwrap().stats().evictions,2);
        println!("bounded voxel cache: partial upload, two-page capacity, repeated eviction/reload, pinned fallback and raw GPU payloads PASS");
    }
}
