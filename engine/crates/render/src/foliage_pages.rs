//! Relocatable voxel pages. Logical level IDs never encode physical addresses.
//! Each page owns its grid/brick directory, sparse cells and splat references.
//! The current renderer uploads all pages; bounded streaming is the next owner.
use crate::foliage_volume_data::{GpuForest,GpuLevel,GpuCell,BRICK_INDIRECTION,cell_index};
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Clone,Debug)]
pub struct VoxelPage {
    level:GpuLevel,
    indices:Vec<u32>,
    cells:Vec<GpuCell>,
    splats:Vec<[u32;2]>,
    directory:usize,
}
impl VoxelPage {
    pub fn cook(forest:&GpuForest,id:usize)->Result<Self,String> {
        let level=*forest.levels.get(id).ok_or("voxel page level outside catalog")?;
        let [dx,dy,dz,offset]=level.dims_offset;
        if dx==0 || dy==0 || dz==0 || dx>1024 || dy>1024 || dz>1024 {return Err("invalid voxel page dimensions".into());}
        let bricked=offset&BRICK_INDIRECTION!=0;
        let count=if bricked {dx.div_ceil(4)as usize*dy.div_ceil(4)as usize*dz.div_ceil(4)as usize}
            else {dx as usize*dy as usize*dz as usize};
        let base=(offset&!BRICK_INDIRECTION)as usize;
        if base.checked_add(count).is_none_or(|end|end>forest.indices.len()) {return Err("invalid voxel page directory".into());}
        let mut page=Self {level,indices:vec![0;count],cells:Vec::new(),splats:Vec::new(),directory:if bricked{count}else{0}};
        let mut remap=BTreeMap::new();
        // Retain the source encoding. Bricked pages do not expand into dense grids.
        if bricked {
            for i in 0..count {
                let address=forest.indices[base+i];if address==0 {continue;}
                let source=forest.indices.get(address as usize-1..address as usize-1+64).ok_or("invalid voxel page brick")?;
                page.indices[i]=page.indices.len()as u32+1;
                for &cell in source {let value=page.local_cell(forest,cell,&mut remap)?;page.indices.push(value);}
            }
        }else {
            for i in 0..count {page.indices[i]=page.local_cell(forest,forest.indices[base+i],&mut remap)?;}
        }
        for z in 0..dz {for y in 0..dy {for x in 0..dx {
            let cell=cell_index(forest,&level,[x,y,z]).ok_or("invalid voxel page cell address")?;
            if cell!=0 {let local=*remap.get(&cell).ok_or("voxel page missing cell")?;page.splats.push([local-1,x|y<<10|z<<20]);}
        }}}
        page.level.dims_offset[3]=if bricked{BRICK_INDIRECTION}else{0};
        Ok(page)
    }
    fn local_cell(&mut self,forest:&GpuForest,cell:u32,remap:&mut BTreeMap<u32,u32>)->Result<u32,String> {
        if cell==0{return Ok(0);}
        if let Some(&local)=remap.get(&cell){return Ok(local);}
        let value=*forest.cells.get(cell as usize-1).ok_or("invalid voxel page sparse cell")?;
        self.cells.push(value);let local=self.cells.len()as u32;remap.insert(cell,local);Ok(local)
    }
    /// Materialize a page at independent physical arena offsets. All address
    /// arithmetic is checked before returning any upload payload.
    pub fn relocate(&self,index_base:u32,cell_base:u32)->Result<(GpuLevel,Vec<u32>,Vec<[u32;2]>),String> {
        if (index_base as u64+self.indices.len()as u64)>=BRICK_INDIRECTION as u64
            || (cell_base as u64+self.cells.len()as u64)>u32::MAX as u64 {return Err("voxel page physical address overflow".into());}
        let mut level=self.level;level.dims_offset[3]|=index_base;
        let mut indices=self.indices.clone();
        for (i,value) in indices.iter_mut().enumerate() {if *value!=0 {*value+=if i<self.directory{index_base}else{cell_base};}}
        let splats=self.splats.iter().map(|s|[s[0]+cell_base,s[1]]).collect();
        Ok((level,indices,splats))
    }
}

/// Physical range arena. A failed allocation has no side effects. Handles are
/// owned by a page publication, not by a tree or camera. Freed spans coalesce.
#[derive(Clone,Debug)]
pub struct Arena {free:BTreeMap<u32,u32>,used:BTreeMap<u32,(u32,u64)>,capacity:u32,next_generation:u64}
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct Allocation {pub range:Range<u32>,pub generation:u64}
impl Arena {
    pub fn new(capacity:u32)->Self {Self {free:if capacity==0{BTreeMap::new()}else{BTreeMap::from([(0,capacity)])},used:BTreeMap::new(),capacity,next_generation:1}}
    pub fn allocate(&mut self,count:u32)->Option<Allocation> {
        if count==0 {return Some(Allocation {range:0..0,generation:0});}
        let next=self.next_generation.checked_add(1)?;
        let (&start,&len)=self.free.iter().find(|(_,len)|**len>=count)?;
        self.free.remove(&start);if len>count {self.free.insert(start+count,len-count);}
        let generation=self.next_generation;self.next_generation=next;
        self.used.insert(start,(count,generation));Some(Allocation {range:start..start+count,generation})
    }
    pub fn release(&mut self,allocation:Allocation)->Result<(),String> {
        let range=allocation.range;
        if range== (0..0) && allocation.generation==0 {return Ok(());}
        if range.start>=range.end || range.end>self.capacity || self.used.get(&range.start)!=Some(&(range.end-range.start,allocation.generation)) {return Err("invalid/stale voxel page allocation".into());}
        self.used.remove(&range.start);let mut start=range.start;let mut end=range.end;
        if let Some((&previous,&len))=self.free.range(..start).next_back() {if previous+len==start {self.free.remove(&previous);start=previous;}}
        if let Some((&next,&len))=self.free.range(end..).next() {if next==end {self.free.remove(&next);end=next+len;}}
        self.free.insert(start,end-start);Ok(())
    }
}

/// Normal renderer upload: logical level order is stable, physical pages are
/// independently allocated (coarse/small pages first). All levels remain resident
/// here; this is address translation, not yet a demand-driven streaming policy.
pub fn pack(forest:&GpuForest)->Result<(GpuForest,Vec<[u32;2]>),String> {
    let pages=(0..forest.levels.len()).map(|i|VoxelPage::cook(forest,i)).collect::<Result<Vec<_>,_>>()?;
    let indices=pages.iter().map(|p|p.indices.len()as u64).sum::<u64>();
    let cells=pages.iter().map(|p|p.cells.len()as u64).sum::<u64>();
    if indices>=BRICK_INDIRECTION as u64 || cells>u32::MAX as u64 {return Err("voxel page arena exceeds addressing budget".into());}
    let mut index_arena=Arena::new(indices as u32);let mut cell_arena=Arena::new(cells as u32);
    let mut result=GpuForest {levels:forest.levels.clone(),indices:vec![0;indices as usize],cells:Vec::with_capacity(cells as usize),instances:forest.instances.clone()};
    let mut splat_list=vec![[0u32;2];pages.len()];
    let mut order=(0..pages.len()).collect::<Vec<_>>();order.sort_by_key(|&i|(pages[i].cells.len(),i));
    for id in order {
        let page=&pages[id];let ir=index_arena.allocate(page.indices.len()as u32).ok_or("voxel index arena exhausted")?;
        let cr=cell_arena.allocate(page.cells.len()as u32).ok_or("voxel cell arena exhausted")?;
        let (level,indices,splats)=page.relocate(ir.range.start,cr.range.start)?;
        splat_list[id]=[splat_list.len()as u32,splats.len()as u32];splat_list.extend(splats);
        result.levels[id]=level;result.indices[ir.range.start as usize..ir.range.end as usize].copy_from_slice(&indices);
        // Initial load has no holes; streaming writes the same span on GPU.
        debug_assert!(page.cells.is_empty() || result.cells.len()==cr.range.start as usize);
        result.cells.extend_from_slice(&page.cells);
    }
    Ok((result,splat_list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    fn fixture()->GpuForest {
        let mut cell=GpuCell::zeroed();cell.albedo_rough=[0.2,0.4,0.7,0.9];
        GpuForest {levels:vec![GpuLevel {origin_cell:[0.,0.,0.,0.5],dims_offset:[5,2,1,0]},GpuLevel {origin_cell:[0.,0.,0.,1.],dims_offset:[1,1,1,10]}],indices:vec![1,0,2,0,1,0,2,0,0,0,2],cells:vec![cell,{let mut other=cell;other.albedo_rough=[0.8,0.1,0.3,0.5];other.normals[0]=[1.,0.,0.,1.];other}],instances:vec![]}
    }
    #[test] fn foliage_pages_roundtrip_dense_bricked_and_splats() {
        for bricked in [false,true] {
            let mut source=fixture();if bricked {crate::foliage_volume_data::pack_bricks(&mut source,128).unwrap();}
            let (packed,packed_splats)=pack(&source).unwrap();
            let oracle=crate::voxel_forest::splat_cells(&packed).unwrap();
            for id in 0..packed.levels.len() {let [a,n]=packed_splats[id];let [b,m]=oracle[id];assert_eq!(n,m);assert_eq!(&packed_splats[a as usize..(a+n)as usize],&oracle[b as usize..(b+m)as usize]);}assert_ne!(source.levels[0].dims_offset[3],packed.levels[0].dims_offset[3]);
            for (id,level) in source.levels.iter().enumerate() {
                let p=VoxelPage::cook(&source,id).unwrap();let (relocated,indices,splats)=p.relocate(17,3).unwrap();
                let mut isolated=GpuForest {levels:vec![relocated],indices:vec![0;17],cells:vec![GpuCell::zeroed();3],instances:vec![]};
                isolated.indices.extend(indices);isolated.cells.extend_from_slice(&p.cells);
                for z in 0..level.dims_offset[2] {for y in 0..level.dims_offset[1] {for x in 0..level.dims_offset[0] {
                    let a=cell_index(&source,level,[x,y,z]).unwrap();
                    for (f,l) in [(&packed,&packed.levels[id]),(&isolated,&isolated.levels[0])] {
                        let b=cell_index(f,l,[x,y,z]).unwrap();assert_eq!(a==0,b==0);
                        if a!=0 {assert_eq!(bytemuck::bytes_of(&source.cells[a as usize-1]),bytemuck::bytes_of(&f.cells[b as usize-1]));}
                    }
                }}}
                for s in splats {let c=[s[1]&1023,(s[1]>>10)&1023,s[1]>>20];assert_eq!(cell_index(&isolated,&relocated,c),Some(s[0]+1));}
                assert!(p.relocate(BRICK_INDIRECTION-1,0).is_err());
            }
        }
    }
    #[test] fn foliage_pages_bounded_arena_eviction_fragmentation_and_invalid_free() {
        let mut a=Arena::new(10);let x=a.allocate(4).unwrap();let y=a.allocate(3).unwrap();let z=a.allocate(3).unwrap();
        assert!(a.allocate(1).is_none());a.release(y.clone()).unwrap();assert!(a.allocate(4).is_none());
        assert!(a.release(Allocation {range:4..6,generation:y.generation}).is_err());let y2=a.allocate(3).unwrap();assert_eq!(y.range,y2.range);assert_ne!(y.generation,y2.generation);assert!(a.release(y).is_err());
        a.release(x).unwrap();a.release(y2).unwrap();a.release(z.clone()).unwrap();assert!(a.release(z).is_err());
        assert_eq!(a.allocate(10).unwrap().range,0..10);assert!(a.allocate(1).is_none());
    }
    #[test] fn foliage_pages_reject_invalid_source_without_partial_output() {
        let mut f=fixture();f.indices[2]=99;assert!(pack(&f).is_err());
        let mut f=fixture();f.levels[0].dims_offset[3]=u32::MAX;assert!(pack(&f).is_err());
    }
}
